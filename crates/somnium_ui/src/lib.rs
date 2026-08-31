pub mod a11y;
pub mod color;
pub mod commands;
pub mod data_table;
pub mod debug;
pub mod dock;
pub mod drag_drop;
pub mod draw;
pub mod editor;
pub mod editor_event;
pub mod font;
pub mod graph;
pub mod icon_svg;
pub mod icons;
pub mod layout_persist;
pub mod log;
pub mod message;
pub mod metaphor;
pub mod motion;
pub mod node;
pub mod outliner_filter;
pub mod pass;
pub mod path;
pub mod pool;
pub mod primitive;
pub mod runtime;
pub mod shaped;
pub mod somui;
pub mod somui_editor;
pub mod style;
pub mod text;
pub mod theme;
pub mod thumbnail;
pub mod timeline;
pub mod types;
pub mod typography;
pub mod ui;
pub mod viewport_layout;
pub mod virtual_list;
pub mod widget;
pub mod widgets;
pub mod workspace;

use crate::editor::inspector_gen::GeneratedComponentPanel;
use crate::editor::property_editors::PropertyEditorKind;
use crate::editor::{
    content::{build_content_drawer, build_create_popup},
    help::{build_help_overlay, fill_help_body},
    inspector::{build_generated_details, build_inspector},
    shell::build_editor_layout,
};
pub use drag_drop::{DragPayload, DropAcceptance, DropEffect, DropRequest, DropTarget};
pub use editor_event::{
    CreateKind, EditorEvent, FoliageBrushField, GestureId, OutlinerRow, ScriptAttachmentRow,
    ScriptFieldKind, ScriptFieldRow, ScriptInspectorState, SelectionMode, TerrainToolField,
};
pub use node::CursorKind;
pub use runtime::{GameUi, GameUiFrame, UiCanvas};
// MORROWIND-H. The runtime motion vocabulary, at the top level beside the
// canvas that drives it: a game reaching for a spring should not have to know
// which module Phase 27 happened to put the animator in.
pub use motion::{CurveId, Easing, Motion, Spring, Transition, TransitionStep};
// MORROWIND-I. The accessibility vocabulary at the top level, because a game
// setting a role on its own widget should not have to find the module.
pub use a11y::{A11ySettings, A11yTree, Announcement, Politeness, Role, Toggled};

pub use typography::{FontRole, TextRole};
pub use workspace::{BottomPanel, Workspace, WorkspaceLayout};

use crate::{
    message::{MessageDirection, Modifiers, NodeHandle, TextMessage, UiMessage, WidgetMessage},
    pass::UiPass,
    types::Thickness,
    ui::UserInterface,
    widget::WidgetBuilder,
    widgets::{
        button::{ButtonBuilder, ButtonMessage},
        check_box::CheckBoxMessage,
        color_picker::{ColorPickerMessage, ColorSwatchMessage},
        combo_box::ComboBoxMessage,
        command_palette::{CommandPaletteMessage, PaletteItem},
        context_menu::{ContextMenuMessage, MenuItem},
        curve_editor::CurveEditorMessage,
        gradient_editor::GradientEditorMessage,
        grid::GridMessage,
        image::ImageBuilder,
        menu::MenuMessage,
        numeric_field::NumericFieldMessage,
        popup::PopupMessage,
        property_row::PropertyRowMessage,
        search_box::{BreadcrumbMessage, SearchBoxMessage},
        slider::SliderMessage,
        splitter::SplitterMessage,
        stack_panel::{Orientation, StackPanelBuilder},
        text::TextBuilder,
        text_box::{TextBoxBuilder, TextBoxMessage},
        toast::ToastMessage,
        tree_view::{TreeItem, TreeViewMessage},
    },
};
use glam::Vec2;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tracing::{info, warn};
use winit::event::{ElementState, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window};

/// What confirming the name prompt will do.
///
/// The prompt is one widget serving three flows, so it carries the flow
/// with it rather than the editor keeping a mode flag beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NamePrompt {
    /// Create a folder in this content-relative directory.
    NewFolder { parent: String },
    /// Create a script in this content-relative directory.
    NewScript { parent: String },
    /// Create a material in this content-relative directory.
    NewMaterial { parent: String },
    /// Rename an entity. CONTROL-F's `F2`: the same modal the three creating
    /// flows already share, because a rename is a name prompt and building a
    /// second one would only give the two a chance to disagree.
    RenameEntity { entity: u32 },
}

/// What one generated widget in the Scripts section does.
///
/// Carried beside the handle rather than encoded in a field enum, because
/// the rows are built from a script's declaration and there is no fixed
/// set of them to enumerate.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ScriptWidgetAction {
    /// The attachment's enable checkbox.
    Enable(usize),
    /// Move it earlier or later in execution order.
    Reorder(usize, i32),
    /// Remove it.
    Detach(usize),
    /// A declared numeric property.
    Number(usize, String),
    /// A declared boolean property.
    Bool(usize, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorTarget {
    Reflected {
        component: somnium_ecs::reflect::StableId,
        field: somnium_ecs::reflect::FieldId,
        vec4: bool,
    },
    /// CONTROL-K. One stop of an authored gradient. The picker path is shared
    /// with every other swatch; only the write-back differs, because the value
    /// that travels is the whole gradient with one stop replaced.
    GradientStop {
        component: somnium_ecs::reflect::StableId,
        field: somnium_ecs::reflect::FieldId,
        index: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GeneratedEdit {
    Whole,
    Lane(u8),
    Euler(u8),
    /// One lane of one element of an `Array` field.
    ///
    /// A spline's control points are the first array anyone actually edits,
    /// and before this they arrived in Details as `Array([Vec3([...]), ...])`
    /// printed as a caption: visible, accurate, and completely unusable. An
    /// element edit addresses the same schema field as any other — the write
    /// path rebuilds the whole array and sends it, so undo, multi-select and
    /// serialization need to know nothing about collections.
    Element {
        index: u16,
        lane: u8,
    },
}

/// One lane of one array element, as a number, when it is one.
fn element_lane(items: &[somnium_ecs::reflect::ReflectValue], index: u16, lane: u8) -> Option<f32> {
    use somnium_ecs::reflect::ReflectValue as RV;
    #[allow(clippy::cast_possible_truncation)]
    match items.get(index as usize)? {
        RV::F64(v) => Some(*v as f32),
        RV::I64(v) => Some(*v as f32),
        RV::Vec2(v) => v.get(lane as usize).copied(),
        RV::Vec3(v) => v.get(lane as usize).copied(),
        RV::Vec4(v) => v.get(lane as usize).copied(),
        _ => None,
    }
}

/// How many numeric boxes one element of an array needs.
fn element_lane_count(value: &somnium_ecs::reflect::ReflectValue) -> usize {
    use somnium_ecs::reflect::ReflectValue as RV;
    match value {
        RV::Vec2(_) => 2,
        RV::Vec3(_) => 3,
        RV::Vec4(_) => 4,
        RV::F64(_) | RV::I64(_) => 1,
        _ => 0,
    }
}

/// What an array row's buttons do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CollectionAction {
    /// Append a copy of the last element, or a zero if there is none.
    Append,
    /// Drop element `index`.
    Remove(u16),
    /// Insert a copy of element `index` directly after it.
    Duplicate(u16),
}

#[derive(Debug, Clone)]
struct GeneratedBinding {
    component: somnium_ecs::reflect::StableId,
    field: somnium_ecs::reflect::FieldId,
    value: somnium_ecs::reflect::ReflectValue,
    default: somnium_ecs::reflect::ReflectValue,
    edit: GeneratedEdit,
    asset_kind_mask: u64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum AssetPickerAction {
    Edit,
    Locate,
    MakeUnique,
    /// Take whatever is selected in the Content Drawer and put it here.
    ///
    /// Unreal's left-arrow, and the reason it exists there too: a drag is a
    /// gesture with a dozen ways to not quite happen — a threshold not
    /// crossed, a pointer a few pixels off the row, a window that lost focus
    /// mid-drag — and every one of them looks identical to a broken feature.
    /// This is the same assignment as one click that cannot miss.
    UseDrawerSelection,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ContentToolbarAction {
    Back,
    Forward,
    Up,
    Kind(crate::metaphor::ContentFilterKind),
    Sort,
    Density,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GeneratedAssetPicker {
    pub combo: NodeHandle,
    pub list: NodeHandle,
    pub kind_mask: u64,
}

// ── Inspector field handle bundle ────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Default)]
struct ToolHandles {
    post_section: NodeHandle,
    post_census_toggle: NodeHandle,
    post_bins_toggle: NodeHandle,

    terrain_section: NodeHandle,
    terrain_paint_toggle: NodeHandle,
    terrain_hex_toggle: NodeHandle,
    terrain_parallax_toggle: NodeHandle,
    terrain_clipmap_toggle: NodeHandle,
    terrain_aerial_toggle: NodeHandle,
    terrain_aerial_dist: NodeHandle,
    terrain_aerial_hero_toggle: NodeHandle,
    terrain_morph_toggle: NodeHandle,
    terrain_morph_start: NodeHandle,
    terrain_brush_items: Vec<(NodeHandle, NodeHandle, u8)>,
    terrain_layer: NodeHandle,
    terrain_palette: [NodeHandle; 32],
    terrain_palette_labels: [NodeHandle; 32],
    terrain_tile: NodeHandle,
    terrain_relief: NodeHandle,
    terrain_wetness: NodeHandle,
    terrain_macro: NodeHandle,
    terrain_debug: NodeHandle,

    foliage_section: NodeHandle,
    foliage_toggle: NodeHandle,
    foliage_paint_toggle: NodeHandle,
    foliage_erase_toggle: NodeHandle,
    foliage_single_toggle: NodeHandle,
    foliage_kind_button: NodeHandle,
    foliage_density: NodeHandle,
    foliage_seed: NodeHandle,
    foliage_slope: NodeHandle,
    foliage_layer: NodeHandle,
    foliage_smin: NodeHandle,
    foliage_smax: NodeHandle,

    script_section: NodeHandle,
    script_add: NodeHandle,
    script_list: NodeHandle,
}

/// Everything the Post FX inspector section displays.
#[derive(Debug, Clone, Copy)]
pub struct PostInspectorState {
    /// `[ev100, exposure_compensation, vignette, chromatic_aberration, ibl]`.
    pub values: [f32; 5],
    pub vignette: bool,
    pub chromatic: bool,
    pub fxaa: bool,
    pub cel_shading: bool,
    pub taa: bool,
    pub gtao: bool,
    pub restir: bool,
    /// Phase 24L: ray-traced indirect diffuse.
    pub restir_gi: bool,
    /// Phase VV: ray-traced water reflections.
    pub rt_reflect: bool,
    /// Phase VV+1: ray-traced water refraction. Default off.
    pub rt_refract: bool,
    /// Percentage-closer soft shadows. Default on.
    pub pcss: bool,
    /// Screen-space contact shadows. Default on.
    pub contact_shadows: bool,
    /// Phase 24AC.
    pub cas: bool,
    /// Phase 24Z.
    pub motion_blur: bool,
    pub bloom: bool,
    pub dof: bool,
    /// Phases 24U/25I.
    pub volumetrics: bool,
    pub shafts: bool,
    /// Exposure comes from aperture/shutter/ISO rather than the EV row.
    pub physical_camera: bool,
    pub world_cache: bool,
    pub specular_gi: bool,
    pub path_tracer: bool,
    pub mesh_sdf: bool,
    pub probes: bool,
    pub analytic_grad: bool,
    pub fsr: bool,
    pub fsr_sharpness: f32,
    pub cache_intensity: f32,
    pub cache_cell: f32,
    pub spec_rough: f32,
    pub path_bounces: f32,
    pub probe_intensity: f32,
    pub shaft_intensity: f32,
    /// `[bloom_intensity, focus_distance, temperature, contrast, saturation,
    /// grain, fog_density, fog_height, fog_asymmetry, tint, lift, gamma, gain,
    /// aperture_f_stops, shutter_denominator, iso, ao_radius, ao_intensity]`.
    pub extras: [f32; 22],
    pub auto_exposure: bool,
    pub tonemapper: &'static str,
}

/// Terrain inspector payload (Phase 17C / XV-Zeta).
#[derive(Debug, Clone, Copy)]
pub struct TerrainInspectorState {
    pub paint_layer: f32,
    pub tile: f32,
    pub relief: f32,
    pub wetness: f32,
    pub debug_view: f32,
    pub macro_strength: f32,
    /// `BrushMode` index 0..=5.
    pub brush: u8,
    pub terrain_edit: bool,
    /// True when terrain edit is on and the brush is Paint.
    pub terrain_paint: bool,
    pub foliage_paint: bool,
    pub hex_tiling: bool,
    pub parallax: bool,
    pub clipmap: bool,
    pub lod_morph: bool,
    pub morph_start: f32,
}

/// One line of the profiler overlay (Phase 29).
///
/// Split into label and value rather than one pre-formatted string because the
/// editor font is proportional: padding a name to a fixed character count lines
/// the numbers up in a log and not on screen. Two columns lay out properly.
#[derive(Clone, Debug)]
pub struct ProfilerRow {
    pub label: String,
    pub value: String,
    /// Nesting depth, drawn as indentation.
    pub depth: u8,
}

/// Engine-neutral status-bar projection of an active background job.
#[derive(Debug, Clone)]
pub struct UiJobStatus {
    pub id: u64,
    pub name: &'static str,
    pub progress: f32,
}

/// Rows the overlay can show before it starts dropping them.
pub const PROFILER_ROWS: usize = 40;

/// Space between content-drawer tiles, in both axes. This was the wrap panel's
/// gap; now it is the difference between a tile and the cell it is placed in.
const CONTENT_GAP: f32 = 10.0;

/// Names shown in the foliage picker (Phase 17F).
///
/// Mirrors `somnium_core::FOLIAGE_PALETTE` by index. The UI crate deliberately
/// does not depend on the engine, so the picker sends back an index and the
/// engine decides what that means — which is also what lets a content drawer
/// replace the list later without touching this code.
pub const FOLIAGE_KIND_NAMES: [&str; 4] = [
    "Grass Medium",
    "Grass Bermuda",
    "Fir Sapling",
    "Island Tree",
];

/// The table entry closest to a value, so a settings file holding `0.3` shows
/// the nearest offered step rather than an empty combo.
fn nearest_index(table: &[f32], value: f32) -> usize {
    table
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (*a - value).abs().total_cmp(&(*b - value).abs()))
        .map_or(0, |(index, _)| index)
}

/// CONTROL-G's translate grid steps. The values the phase names, plus "Off",
/// because a grid you cannot leave is worse than no grid.
pub(crate) const SNAP_GRID_NAMES: [&str; 6] = ["Off", "0.1 m", "0.25 m", "0.5 m", "1 m", "5 m"];
/// The metres each entry means. Index 0 is off.
pub(crate) const SNAP_GRID_VALUES: [f32; 6] = [0.0, 0.1, 0.25, 0.5, 1.0, 5.0];
/// CONTROL-G's rotate increments.
pub(crate) const SNAP_ANGLE_NAMES: [&str; 5] =
    ["Off", "1\u{b0}", "5\u{b0}", "15\u{b0}", "45\u{b0}"];
/// The degrees each entry means. Index 0 is off.
pub(crate) const SNAP_ANGLE_VALUES: [f32; 5] = [0.0, 1.0, 5.0, 15.0, 45.0];

const VIEWPORT_RESOLUTION_NAMES: [&str; 5] =
    ["Native", "2560×1440", "1920×1080", "1600×900", "1280×720"];

/// Short paint-palette labels (Phase XV-I / XV-Zeta). Indices match the renderer roster.
const TERRAIN_LAYER_SHORT: [&str; 32] = [
    "Grass", "Forest", "Rock", "Snow", "Meadow", "Mud", "Coast", "Gravel", "DrySd", "DampSd",
    "Earth", "Clay", "Sparse", "Moss", "Cliff", "Talus", "Lawn", "Duff", "GrayRk", "Slate",
    "MossC", "Lime", "Loam", "Pine", "Wild", "Peat", "Gran", "Dune", "Lichen", "Autumn", "Path",
    "Crust",
];

const TERRAIN_BRUSH_NAMES: [&str; 6] = ["Raise", "Lower", "Smooth", "Flatten", "Noise", "Paint"];

pub type LightInspectorValues = [f32; 11];

/// Light Details rows. Visibility depends on the selected fixture, not a single
/// "is rect" flag — disc hides width/height, tube shows Half W as half-length.
pub struct LightInspectorState {
    pub values: LightInspectorValues,
    pub kelvin: f32,
    pub directional: bool,
    pub show_cone: bool,
    pub show_width: bool,
    pub show_height: bool,
}

// ── Layout build result ───────────────────────────────────────────────────────

struct EditorLayout {
    outliner_scroll: NodeHandle,
    outliner_empty: NodeHandle,
    outliner_stack: NodeHandle,
    inspector_stack: NodeHandle,
    /// Shown when nothing is selected; hidden the moment something is.
    details_empty: NodeHandle,
    log_stack: NodeHandle,
    log_severity_chips: Vec<(NodeHandle, crate::log::LogSeverity)>,
    log_search: NodeHandle,
    log_pin_only: NodeHandle,
    log_copy: NodeHandle,
    log_clear: NodeHandle,
    log_jobs_toggle: NodeHandle,
    log_history_toggle: NodeHandle,
    log_empty: NodeHandle,
    create_button: NodeHandle,
    create_popup: NodeHandle,
    create_popup_items: Vec<(NodeHandle, &'static str)>,
    file_button: NodeHandle,
    file_popup: NodeHandle,
    file_menu_stack: NodeHandle,
    menu_command_items: Vec<(NodeHandle, &'static str)>,
    camera_speed_slider: NodeHandle,
    camera_speed_label: NodeHandle,
    viewport_res_combo: NodeHandle,
    play_button: NodeHandle,
    play_label: NodeHandle,
    immersive_button: NodeHandle,
    pause_button: NodeHandle,
    /// MORROWIND-N: advance one fixed step while paused.
    step_button: NodeHandle,
    pause_label: NodeHandle,
    stop_button: NodeHandle,
    stop_label: NodeHandle,
    select_button: NodeHandle,
    landscape_button: NodeHandle,
    foliage_toolbar_button: NodeHandle,
    terrain_tool_items: Vec<(NodeHandle, NodeHandle, u8)>,
    inspector_handles: ToolHandles,
    viewport_handle: NodeHandle,
    /// Hidden composite shown by the Animation workspace.
    animation_workspace: NodeHandle,
    /// Production animation graph/state-machine surface inside that composite.
    animation_graph_editor: NodeHandle,
    /// Production track timeline and its retained CONTROL-K child.
    animation_timeline: crate::timeline::TimelineEditorHandles,
    /// Last authored timeline emitted by the retained control.
    animation_timeline_document: crate::timeline::TimelineDocument,
    /// Selection readout floating over the render, bottom-left.
    vp_overlay: NodeHandle,
    vp_overlay_text: NodeHandle,
    /// Phase 29 profiler overlay.
    profiler_panel: NodeHandle,
    profiler_toggle: NodeHandle,
    profiler_toggle_lbl: NodeHandle,
    profiler_names: Vec<NodeHandle>,
    profiler_values: Vec<NodeHandle>,
    outer_grid: NodeHandle,
    menu_bar_h: NodeHandle,
    status_dirty: NodeHandle,
    status_selection: NodeHandle,
    status_stats: NodeHandle,
    status_stats_button: NodeHandle,
    /// Floating viewport-context scope; a child of the viewport, not a grid row.
    /// Held for the layout regression test that pins the 68 px scene budget.
    #[allow(dead_code)]
    vp_bar_h: NodeHandle,
    snap_cluster: NodeHandle,
    snap_grid_combo: NodeHandle,
    snap_angle_combo: NodeHandle,
    snap_surface_toggle: NodeHandle,
    gizmo_space_toggle: NodeHandle,
    gizmo_space_label: NodeHandle,
    select_only_toggle: NodeHandle,
    snap_overflow: NodeHandle,
    /// CONTROL-L: the day-cycle scrub cluster, hidden when the scene has none.
    time_cluster: NodeHandle,
    time_label: NodeHandle,
    time_slider: NodeHandle,
    /// Mode-scope command labels. Collapse to icon-only under 1400 px.
    mode_labels: [NodeHandle; 4],
    inner_h: NodeHandle,
    content_split_h: NodeHandle,
    right_split_h: NodeHandle,
    toolbar_h: NodeHandle,
    right_h: NodeHandle,
    bottom_h: NodeHandle,
    fps_text: NodeHandle,
    help_button: NodeHandle,
    help_overlay: NodeHandle,
    help_body: NodeHandle,
    tooltip: NodeHandle,
    edit_button: NodeHandle,
    view_button: NodeHandle,
    window_button: NodeHandle,
    help_menu_button: NodeHandle,
    edit_popup: NodeHandle,
    view_popup: NodeHandle,
    window_popup: NodeHandle,
    help_menu_popup: NodeHandle,
    status_text: NodeHandle,
    status_cancel: NodeHandle,
    drawer_button: NodeHandle,
    log_button: NodeHandle,
    content_drawer: NodeHandle,
    content_search: NodeHandle,
    content_breadcrumb: NodeHandle,
    content_engine_toggle: NodeHandle,
    content_scroll: NodeHandle,
    content_list: NodeHandle,
    content_toolbar_actions: Vec<(NodeHandle, ContentToolbarAction)>,
    outliner_tree: NodeHandle,
    outliner_search: NodeHandle,
    inspector_search: NodeHandle,
    foliage_kind_combo: NodeHandle,
    foliage_kind_popup: NodeHandle,
    viewport_res_popup: NodeHandle,
    /// CONTROL-G's snap dropdowns. Held so `combo_entries` can close them with
    /// the others — a dropdown nothing knows about stays open behind the next
    /// one.
    snap_grid_popup: NodeHandle,
    snap_angle_popup: NodeHandle,
    save_button: NodeHandle,
    palette_button: NodeHandle,
    palette_popup: NodeHandle,
    palette_widget: NodeHandle,
    toast_host: NodeHandle,
    unsaved_popup: NodeHandle,
    unsaved_save: NodeHandle,
    unsaved_discard: NodeHandle,
    unsaved_cancel: NodeHandle,
    content_menu_popup: NodeHandle,
    content_menu: NodeHandle,
    outliner_menu_popup: NodeHandle,
    outliner_menu: NodeHandle,
    preferences: crate::editor::preferences::PreferencesHandles,
    name_popup: NodeHandle,
    name_title: NodeHandle,
    name_input: NodeHandle,
    name_ok: NodeHandle,
    name_cancel: NodeHandle,
    color_popup: NodeHandle,
    color_picker: NodeHandle,
    title_drag: NodeHandle,
    title_label: NodeHandle,
    win_min: NodeHandle,
    win_max: NodeHandle,
    win_close: NodeHandle,
    help_toc: Vec<(NodeHandle, u8)>,
    help_close: NodeHandle,
    log_panel: NodeHandle,
    references_panel: NodeHandle,
    references_button: NodeHandle,
    references_title: NodeHandle,
    references_list: NodeHandle,
    locale_panel: NodeHandle,
    locale_button: NodeHandle,
    locale_search: NodeHandle,
    locale_incomplete: NodeHandle,
    locale_grid: NodeHandle,
    locale_actions: Vec<(NodeHandle, LocaleAction)>,
}

/// A verb in the Localisation panel's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocaleAction {
    /// Write the catalogue back to disk, one file per locale.
    Save,
    /// Hand the table to a translator as a CSV.
    ExportCsv,
}

// ── UiManager ────────────────────────────────────────────────────────────────

/// Combined UI manager — wraps the native wgpu widget tree rendered by UiPass.
/// Which list the bottom panel is showing.
///
/// One enum rather than two booleans: the three are mutually exclusive and a
/// pair of flags would make "jobs and history at once" a representable state
/// that nothing implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogView {
    /// The Output Log.
    Log,
    /// Background jobs, CONTROL-C's, including failed and cancelled ones.
    Jobs,
    /// The undo history, CONTROL-J's.
    History,
}

pub struct UiManager {
    window: Arc<Window>,
    /// Window size in **logical units** — what the widget tree, the collapse
    /// rules and the workspace presets all measure against.
    window_size: (u32, u32),
    /// Window size in device pixels, for the scissor rect.
    physical_size: (u32, u32),
    /// Device pixels per logical unit.
    ui_scale: f32,
    /// Wall clock of the previous `end_frame`, for the motion delta. `None`
    /// until the first frame, which therefore advances nothing.
    last_frame_at: Option<std::time::Instant>,
    native_ui: UserInterface,
    ui_pass: UiPass,
    font_id: u8,
    // Live-update widget handles
    #[allow(dead_code)]
    outliner_scroll: NodeHandle,
    #[allow(dead_code)]
    outliner_stack: NodeHandle,
    inspector_stack: NodeHandle,
    /// Shown when nothing is selected; hidden the moment something is.
    details_empty: NodeHandle,
    /// Shown when the scene has no entities.
    outliner_empty: NodeHandle,
    /// Shown when nothing has been logged yet.
    log_empty: NodeHandle,
    /// Selection readout floating over the render.
    vp_overlay: NodeHandle,
    vp_overlay_text: NodeHandle,
    log_stack: NodeHandle,
    log_severity_chips: Vec<(NodeHandle, crate::log::LogSeverity)>,
    log_search: NodeHandle,
    log_pin_only: NodeHandle,
    log_copy: NodeHandle,
    log_clear: NodeHandle,
    log_jobs_toggle: NodeHandle,
    log_history_toggle: NodeHandle,
    // Create menu
    create_button: NodeHandle,
    create_popup: NodeHandle,
    create_popup_open: bool,
    create_popup_items: Vec<(NodeHandle, &'static str)>,
    /// Palette entry currently shown on the picker button, so a click can
    /// advance to the next one.
    foliage_kind_shown: u8,
    // File menu (Phase 19B): Import
    file_button: NodeHandle,
    file_popup: NodeHandle,
    file_menu_stack: NodeHandle,
    file_popup_open: bool,
    open_combo_popup: NodeHandle,
    /// Application menu handles mapped back to stable registry ids.
    menu_command_items: Vec<(NodeHandle, &'static str)>,
    // Viewport toolbar (Phase 20B): camera speed
    camera_speed_slider: NodeHandle,
    camera_speed_label: NodeHandle,
    viewport_res_combo: NodeHandle,
    play_button: NodeHandle,
    play_label: NodeHandle,
    immersive_button: NodeHandle,
    pause_button: NodeHandle,
    /// MORROWIND-N: advance one fixed step while paused.
    step_button: NodeHandle,
    #[allow(dead_code)]
    pause_label: NodeHandle,
    stop_button: NodeHandle,
    #[allow(dead_code)]
    stop_label: NodeHandle,
    select_button: NodeHandle,
    landscape_button: NodeHandle,
    foliage_toolbar_button: NodeHandle,
    // Terrain tool buttons (Phase 14F / XV-Zeta): (button, label, BrushMode index)
    terrain_tool_items: Vec<(NodeHandle, NodeHandle, u8)>,
    // Outliner row mapping: (button_handle, entity_index)
    outliner_rows: Vec<(NodeHandle, u32)>,
    /// Entities the palette can jump to, mirrored from the Outliner.
    palette_entities: Vec<(u32, String)>,
    /// Stable dynamic palette ids mapped to their current targets.
    palette_targets: std::collections::HashMap<String, PaletteTarget>,
    /// Persisted palette learning state.
    command_history: crate::commands::CommandHistory,
    // Inspector field handles
    inspector_handles: ToolHandles,
    // Editor event queue drained by app.rs each frame
    editor_events: VecDeque<EditorEvent>,
    selected_entity: Option<somnium_ecs::Entity>,
    next_property_gesture: u64,
    // Viewport area handle — mouse events here pass through to the game
    #[allow(dead_code)]
    viewport_handle: NodeHandle,
    animation_workspace: NodeHandle,
    animation_graph_editor: NodeHandle,
    animation_timeline: crate::timeline::TimelineEditorHandles,
    animation_timeline_document: crate::timeline::TimelineDocument,
    profiler_panel: NodeHandle,
    profiler_toggle: NodeHandle,
    profiler_toggle_lbl: NodeHandle,
    profiler_names: Vec<NodeHandle>,
    profiler_values: Vec<NodeHandle>,
    last_outliner_state: Option<(Vec<OutlinerRow>, Option<u32>)>,
    outer_grid: NodeHandle,
    menu_bar_h: NodeHandle,
    /// Mode-scope command labels. Held so a future rule can address them; the
    /// 1400 px breakpoint deliberately does not.
    #[allow(dead_code)]
    mode_labels: [NodeHandle; 4],
    /// Last width the collapse rules were evaluated at, so a resize that does
    /// not cross a breakpoint costs nothing.
    collapsed_at: Option<u32>,
    status_dirty: NodeHandle,
    status_selection: NodeHandle,
    status_stats: NodeHandle,
    status_stats_button: NodeHandle,
    /// Value each inspector field held at the last baseline reset — a scene
    /// load, a save, or a change of selection. The modified dot lights when the
    /// live value differs from this, and reverting writes it back.
    ///
    /// This is deliberately *not* "differs from the component default": the UI
    /// layer does not know component defaults, and inventing them would make
    /// the dot lie. "Unsaved edit to this property" is the honest reading, and
    /// it is the one that pairs with the status bar's save state.
    generated_root: NodeHandle,
    generated_entity: Option<somnium_ecs::Entity>,
    generated_signature: Vec<(
        somnium_ecs::reflect::StableId,
        somnium_ecs::reflect::FieldId,
        GeneratedEdit,
    )>,
    generated_bindings: HashMap<NodeHandle, GeneratedBinding>,
    generated_collection_actions: HashMap<
        NodeHandle,
        (
            somnium_ecs::reflect::StableId,
            somnium_ecs::reflect::FieldId,
            CollectionAction,
        ),
    >,
    /// What the last drop probe resolved to, in words, for the message a
    /// failed drop shows. Diagnostic state, not authoring state.
    drop_probe: String,
    generated_rows: HashMap<NodeHandle, GeneratedBinding>,
    generated_gestures: HashMap<NodeHandle, GestureId>,
    generated_asset_choices: HashMap<NodeHandle, Vec<Option<somnium_ecs::reflect::AssetRef>>>,
    generated_asset_searches: HashMap<NodeHandle, GeneratedAssetPicker>,
    generated_asset_actions: HashMap<NodeHandle, (NodeHandle, AssetPickerAction)>,
    inner_h: NodeHandle,
    content_split_h: NodeHandle,
    right_split_h: NodeHandle,
    toolbar_h: NodeHandle,
    right_h: NodeHandle,
    bottom_h: NodeHandle,
    fps_text: NodeHandle,
    help_button: NodeHandle,
    help_overlay: NodeHandle,
    help_body: NodeHandle,
    tooltip: NodeHandle,
    edit_button: NodeHandle,
    view_button: NodeHandle,
    window_button: NodeHandle,
    help_menu_button: NodeHandle,
    edit_popup: NodeHandle,
    view_popup: NodeHandle,
    window_popup: NodeHandle,
    help_menu_popup: NodeHandle,
    /// Workspace the shell is currently arranged for; Reset returns to its
    /// shipped preset rather than to a global default.
    active_workspace: crate::workspace::Workspace,
    /// Height of the shared bottom row when it is open. Set by the active
    /// workspace preset and persisted with it.
    drawer_height: f32,
    status_text: NodeHandle,
    status_cancel: NodeHandle,
    status_cancel_job: Option<u64>,
    drawer_button: NodeHandle,
    log_button: NodeHandle,
    content_drawer: NodeHandle,
    content_search: NodeHandle,
    content_breadcrumb: NodeHandle,
    content_engine_toggle: NodeHandle,
    content_scroll: NodeHandle,
    content_list: NodeHandle,
    content_toolbar_actions: Vec<(NodeHandle, ContentToolbarAction)>,
    outliner_tree: NodeHandle,
    outliner_search: NodeHandle,
    inspector_search: NodeHandle,
    foliage_kind_combo: NodeHandle,
    foliage_kind_popup: NodeHandle,
    viewport_res_popup: NodeHandle,
    /// CONTROL-G's snap dropdowns. Held so `combo_entries` can close them with
    /// the others — a dropdown nothing knows about stays open behind the next
    /// one.
    snap_grid_popup: NodeHandle,
    snap_angle_popup: NodeHandle,
    save_button: NodeHandle,
    palette_button: NodeHandle,
    palette_popup: NodeHandle,
    palette_widget: NodeHandle,
    toast_host: NodeHandle,
    unsaved_popup: NodeHandle,
    unsaved_save: NodeHandle,
    unsaved_discard: NodeHandle,
    unsaved_cancel: NodeHandle,
    content_menu_popup: NodeHandle,
    content_menu: NodeHandle,
    outliner_menu_popup: NodeHandle,
    outliner_menu: NodeHandle,
    preferences: crate::editor::preferences::PreferencesHandles,
    snap_cluster: NodeHandle,
    snap_grid_combo: NodeHandle,
    snap_angle_combo: NodeHandle,
    snap_surface_toggle: NodeHandle,
    gizmo_space_toggle: NodeHandle,
    gizmo_space_label: NodeHandle,
    select_only_toggle: NodeHandle,
    snap_overflow: NodeHandle,
    time_cluster: NodeHandle,
    time_label: NodeHandle,
    time_slider: NodeHandle,
    /// CONTROL-L: the hour the context bar last displayed, so the label is
    /// rewritten only when it changes rather than sixty times a second.
    time_shown: Option<f32>,
    /// True while a play session exists — playing or paused. Set from
    /// [`Self::update_simulation_controls`], the one place every transport
    /// transition passes through.
    play_session_active: bool,
    name_popup: NodeHandle,
    name_title: NodeHandle,
    name_input: NodeHandle,
    name_ok: NodeHandle,
    name_cancel: NodeHandle,
    color_popup: NodeHandle,
    color_picker: NodeHandle,
    help_open: bool,
    drawer_open: bool,
    #[allow(dead_code)]
    drawer_docked: bool,
    show_engine_content: bool,
    inspector_filter: String,
    content_filter: String,
    content_path: String,
    /// Phase 27-G browser workflows.
    content_history: crate::metaphor::ContentHistory,
    content_kind: crate::metaphor::ContentFilterKind,
    content_density: crate::metaphor::ContentDensity,
    /// Published immutable asset inventory; drawer queries never walk disk.
    asset_db: somnium_asset::database::AssetDbSnapshot,
    content_sort: somnium_asset::database::AssetSort,
    content_sort_descending: bool,
    /// Paths selected in the drawer. Multi-select is a set rather than a range
    /// because the tiles wrap, so "everything between A and B" has no stable
    /// meaning once the panel is resized.
    content_selection: std::collections::HashSet<std::path::PathBuf>,
    outliner_expanded: std::collections::HashSet<u32>,
    /// The structured log. Replaced the flat `VecDeque<String>` in CONTROL-I:
    /// a severity chip, a category filter and a clickable `file:line` all need
    /// facts a string does not carry. The policy lives in `crate::log` so it
    /// is testable without a GPU device; this holds only the widget side.
    log: crate::log::OutputLog,
    /// `(row, entry id)` for the rows currently mounted.
    log_rows: Vec<(NodeHandle, u64)>,
    /// Which of the three lists the bottom panel is showing.
    log_view: LogView,
    /// How many error toasts are waiting to be dismissed. Mirrored here
    /// because the host owns them and `Esc` needs the answer synchronously.
    sticky_toasts: usize,
    /// Seconds since the editor started, for timestamps.
    log_clock: std::time::Instant,
    /// Background jobs, as the status surface last reported them.
    job_rows: Vec<(u64, String, f32, bool)>,
    /// The scene the title bar names.
    scene_name: Option<String>,
    /// Undo history entry names, oldest first.
    history_rows: Vec<String>,
    /// How many history entries have been applied.
    history_position: usize,
    edit_popup_open: bool,
    view_popup_open: bool,
    window_popup_open: bool,
    help_menu_open: bool,
    tooltip_since: Option<(NodeHandle, std::time::Instant)>,
    help_page: u8,
    /// The tiles that currently exist, in the order they were built.
    content_entries: Vec<(NodeHandle, crate::metaphor::ContentEntry)>,
    /// Every entry the last query returned, whether or not it has a tile.
    ///
    /// MORROWIND-M. This is the list the drawer *shows*; `content_entries` is
    /// the screenful of it that was built. Keeping both is what lets the
    /// window move without asking the asset database anything.
    content_all: Vec<crate::metaphor::ContentEntry>,
    /// The window `content_entries` was built for, or `None` if it is stale.
    content_window: Option<crate::virtual_list::GridWindow>,
    /// MORROWIND-M item 3. Built with the asset inventory, on its job, so the
    /// graph and the drawer always describe the same disk.
    dependency_index: somnium_asset::depend::DependencyIndex,
    /// The asset the References panel is answering about.
    references_subject: Option<somnium_asset::database::AssetId>,
    /// One row per listed asset, so a click can name what it landed on.
    references_rows: Vec<(NodeHandle, somnium_asset::database::AssetId)>,
    references_open: bool,
    outliner_entity_handles: HashMap<u32, somnium_ecs::Entity>,
    outliner_selection: Vec<u32>,
    clipboard_filled: bool,
    /// User keybinding overrides. Dispatch reads this, not the declarations.
    keybindings: crate::commands::KeyBindings,
    /// The command whose binding is being captured, if the Preferences window
    /// is waiting for a keystroke.
    rebinding: Option<&'static str>,
    preferences_open: bool,
    /// Which Preferences tab is showing.
    preferences_bindings_tab: bool,
    preferences_query: String,
    preferences_modified_only: bool,
    /// Generated settings rows, mounted in the Preferences window. A separate
    /// map from Details' because the two panels are alive at the same time and
    /// address different objects.
    settings_bindings: HashMap<NodeHandle, GeneratedBinding>,
    settings_rows: HashMap<NodeHandle, GeneratedBinding>,
    settings_root: NodeHandle,
    settings_signature: Vec<(
        somnium_ecs::reflect::StableId,
        somnium_ecs::reflect::FieldId,
        GeneratedEdit,
    )>,
    settings_panels: Vec<crate::editor::inspector_gen::GeneratedComponentPanel>,
    /// `(command id, capture button, reset button)` for the keyboard tab.
    binding_rows: Vec<(&'static str, NodeHandle, NodeHandle)>,
    recent_scenes: Vec<(String, bool)>,
    /// `(button, path)` for the File menu's recent tail. The separator carries
    /// an empty path and is never clickable.
    recent_menu_items: Vec<(NodeHandle, String)>,
    active_debug_view: &'static str,
    render_toggles: crate::debug::DebugToggles,
    /// `(entity index, name)` for the open piercing menu.
    piercing_rows: Vec<(u32, String)>,
    tooltip_delay_ms: f32,
    /// The value a badge-column drag is painting, so the run stays uniform.
    badge_drag_value: Option<bool>,
    /// Mirror of core's live rubber-band, so `Esc` can cancel it before the
    /// overlay stack gets a look in.
    marquee_active: bool,
    viewport_drop_probe: (Option<somnium_ecs::Entity>, Option<[f32; 3]>),
    external_drag_files: Vec<std::path::PathBuf>,
    outliner_filter: String,
    palette_open: bool,
    unsaved_open: bool,
    pending_scene_action: Option<EditorEvent>,
    color_open: bool,
    color_target: Option<ColorTarget>,
    color_gesture: Option<GestureId>,
    color_original: [f32; 4],
    color_live: [f32; 4],
    /// What the Content Drawer's right-click menu is currently about.
    ///
    /// Set when the menu opens and read when an item is chosen, because
    /// the menu itself only reports an id — the *subject* is the editor's
    /// to remember.
    content_menu_target: Option<crate::metaphor::ContentEntry>,
    /// Which folder a right-click happened in, so a "New Folder" from
    /// inside `scripts/` lands in `scripts/` and not at the root.
    content_menu_folder: String,
    content_inline_rename: Option<(NodeHandle, std::path::PathBuf)>,
    /// What confirming the name prompt will do.
    name_prompt: Option<NamePrompt>,
    /// What is currently typed in the name prompt.
    ///
    /// Mirrored here rather than read back from the widget on confirm:
    /// the box reports every keystroke, and asking a widget for its state
    /// is the pattern this UI does not have.
    name_text: String,
    /// Phase 16-D: the Scripts section as it was last built.
    ///
    /// Rebuilding a widget tree per frame would be wasteful and would eat
    /// the focus of any field being typed into, so the section is rebuilt
    /// only when this differs from what the engine offers.
    script_state: ScriptInspectorState,
    /// What each generated widget in the Scripts section does. Rebuilt
    /// alongside the widgets, so a handle can never outlive its meaning.
    script_widgets: Vec<(NodeHandle, ScriptWidgetAction)>,
    scene_dirty: bool,
    /// Phase 16-D: blocking script diagnostics since the last clear, shown
    /// in the status cluster. A count rather than a light, because "three
    /// scripts are broken" and "one script is broken" are different
    /// situations and the log is one click away either way.
    script_errors: usize,
    chrome_layout: crate::layout_persist::ChromeLayout,
    log_open: bool,
    references_panel: NodeHandle,
    references_button: NodeHandle,
    references_title: NodeHandle,
    references_list: NodeHandle,
    locale_panel: NodeHandle,
    locale_button: NodeHandle,
    locale_search: NodeHandle,
    locale_incomplete: NodeHandle,
    locale_grid: NodeHandle,
    locale_actions: Vec<(NodeHandle, LocaleAction)>,
    locale_open: bool,
    /// MORROWIND-J step 3. How the viewport region is divided.
    viewport_layout: crate::viewport_layout::ViewportLayout,
    /// Whether the panel is filtered to rows with an untranslated cell.
    locale_only_incomplete: bool,
    /// The last committed state of the localisation table.
    ///
    /// The grid owns the live one — it is mid-edit most of the time it is
    /// looked at — and publishes a copy on every commit. This is what a save
    /// writes, so a save can never catch a half-typed cell.
    locale_table: Option<crate::data_table::DataTable>,
    title_drag: NodeHandle,
    title_label: NodeHandle,
    win_min: NodeHandle,
    win_max: NodeHandle,
    win_close: NodeHandle,
    help_toc: Vec<(NodeHandle, u8)>,
    help_close: NodeHandle,
    log_panel: NodeHandle,
    title_last_click: Option<std::time::Instant>,
    immersive: bool,
    immersive_restore_fullscreen: Option<Fullscreen>,
    immersive_restore_maximized: bool,
}

/// Load the bundled Nocturne faces and publish the [`FontRole`] table.
///
/// Phase 26-Zeta-D. Order is fixed so a capture at 100 % and one at 200 %
/// resolve the same ids. Any cut that fails to load is aliased onto one that
/// did rather than left pointing at an unloaded id — a missing SemiBold should
/// cost the header its weight, not its glyphs. Returns the UI Regular id, which
/// is what the existing `font_id` parameter threaded through `build_*` means.
fn load_fonts(ui: &mut UserInterface) -> u8 {
    use typography::{FontRegistry, FontRole};

    const FACES: [(FontRole, &[u8], &str); 5] = [
        (
            FontRole::UiRegular,
            include_bytes!("../assets/fonts/Inter-Regular.ttf"),
            "Inter Regular",
        ),
        (
            FontRole::UiMedium,
            include_bytes!("../assets/fonts/Inter-Medium.ttf"),
            "Inter Medium",
        ),
        (
            FontRole::UiSemiBold,
            include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
            "Inter SemiBold",
        ),
        (
            FontRole::Mono,
            include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
            "JetBrains Mono Regular",
        ),
        (
            FontRole::MonoMedium,
            include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf"),
            "JetBrains Mono Medium",
        ),
    ];

    let mut registry = FontRegistry::uniform(0);
    let mut base: Option<u8> = None;
    let mut missing: Vec<FontRole> = Vec::new();

    for (role, bytes, name) in FACES {
        match ui.add_font(bytes) {
            Ok(id) => {
                if base.is_none() {
                    base = Some(id);
                }
                registry.set(role, id);
            }
            Err(e) => {
                warn!("Native UI: bundled {name} failed to load ({e})");
                missing.push(role);
            }
        }
    }

    let base = match base {
        Some(id) => id,
        None => {
            // Every bundled cut failed — fall back to a system face so the
            // editor still has glyphs. Weight hierarchy is lost; the shell
            // stays usable.
            warn!("Native UI: no bundled face loaded; trying system fonts");
            let fallback = std::fs::read("C:\\Windows\\Fonts\\segoeui.ttf")
                .or_else(|_| std::fs::read("C:\\Windows\\Fonts\\arial.ttf"))
                .or_else(|_| {
                    std::fs::read("/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf")
                })
                .ok();
            let id = fallback
                .and_then(|b| ui.add_font(&b).ok())
                .unwrap_or_default();
            registry = FontRegistry::uniform(id);
            id
        }
    };

    for role in missing {
        registry.set(role, base);
    }

    info!(
        "Native UI: Nocturne faces loaded — ui {}/{}/{}, mono {}/{}",
        registry.id(FontRole::UiRegular),
        registry.id(FontRole::UiMedium),
        registry.id(FontRole::UiSemiBold),
        registry.id(FontRole::Mono),
        registry.id(FontRole::MonoMedium),
    );
    typography::install_fonts(registry);
    registry.id(FontRole::UiRegular)
}

/// Path separators the content tree may use on either platform.
///
/// Named because a `char` array literal containing a backslash is easy to
/// mangle when this file is edited programmatically.
const SEP: [char; 2] = ['/', '\\'];

/// What a dynamic palette row resolves to. Registered commands use stable ids
/// directly; entities/assets use namespaced ids in this map.
#[derive(Clone, Debug, PartialEq)]
enum PaletteTarget {
    Entity(u32),
    Asset(std::path::PathBuf),
    Help(u8),
    Drawer,
    Log,
}

impl UiManager {
    fn allocate_property_gesture(&mut self) -> GestureId {
        let gesture = GestureId(self.next_property_gesture);
        self.next_property_gesture = self.next_property_gesture.wrapping_add(1).max(1);
        gesture
    }

    fn queue_generated_binding(
        &mut self,
        binding: &GeneratedBinding,
        value: somnium_ecs::reflect::ReflectValue,
        gesture: GestureId,
        live: bool,
    ) {
        self.queue_generated_address(binding.component, binding.field, value, gesture, live);
    }

    fn queue_generated_address(
        &mut self,
        component: somnium_ecs::reflect::StableId,
        field: somnium_ecs::reflect::FieldId,
        value: somnium_ecs::reflect::ReflectValue,
        gesture: GestureId,
        live: bool,
    ) {
        let Some(entity) = self.selected_entity else {
            return;
        };
        self.editor_events
            .push_back(EditorEvent::SetComponentField {
                entity,
                component,
                field,
                value,
                gesture,
                live,
            });
    }

    /// The gesture id a widget's edit belongs to.
    ///
    /// A live edit reuses (or opens) one id for the whole drag; a commit
    /// consumes it, so the coalescing window closes exactly when the gesture
    /// does. Extracted in CONTROL-K because three widgets now need it.
    fn gesture_for(&mut self, handle: NodeHandle, live: bool) -> GestureId {
        if live {
            if let Some(gesture) = self.generated_gestures.get(&handle).copied() {
                return gesture;
            }
            let gesture = self.allocate_property_gesture();
            self.generated_gestures.insert(handle, gesture);
            return gesture;
        }
        self.generated_gestures
            .remove(&handle)
            .unwrap_or_else(|| self.allocate_property_gesture())
    }

    /// Write a colour the picker produced back to whatever it was opened for.
    ///
    /// One place rather than three identical `match`es, and the reason it is
    /// worth extracting: CONTROL-K added a fourth destination shape, and three
    /// copies is where the fourth one gets missed.
    fn write_color_target(
        &mut self,
        target: ColorTarget,
        rgba: [f32; 4],
        gesture: GestureId,
        live: bool,
    ) {
        match target {
            ColorTarget::Reflected {
                component,
                field,
                vec4,
            } => {
                let value = if vec4 {
                    somnium_ecs::reflect::ReflectValue::Vec4(rgba)
                } else {
                    somnium_ecs::reflect::ReflectValue::Vec3([rgba[0], rgba[1], rgba[2]])
                };
                self.queue_generated_address(component, field, value, gesture, live);
            }
            ColorTarget::GradientStop {
                component,
                field,
                index,
            } => {
                // The value that travels is the whole gradient. A stop is not
                // separately addressable by `(StableId, FieldId)`, and
                // inventing a sub-field address for it would be a second
                // property vocabulary — exactly what Seam 1 forbids.
                let Some(binding) = self
                    .generated_bindings
                    .values()
                    .find(|b| b.component == component && b.field == field)
                    .cloned()
                else {
                    return;
                };
                let somnium_ecs::reflect::ReflectValue::Gradient(gradient) = &binding.value else {
                    return;
                };
                let mut gradient = gradient.clone();
                let Some(stop) = gradient.stop_mut(index) else {
                    return;
                };
                stop.color = rgba;
                self.queue_generated_address(
                    component,
                    field,
                    somnium_ecs::reflect::ReflectValue::Gradient(gradient),
                    gesture,
                    live,
                );
            }
        }
    }

    fn collection_result(
        value: &somnium_ecs::reflect::ReflectValue,
        action: CollectionAction,
    ) -> Option<somnium_ecs::reflect::ReflectValue> {
        use somnium_ecs::reflect::ReflectValue as RV;
        let RV::Array(items) = value else {
            return None;
        };
        let mut items = items.clone();
        match action {
            CollectionAction::Append => {
                // A copy of the last element, not a zero: appending to a
                // shoreline should extend it near where it already is, and a
                // new point at the world origin is a point the author has to
                // go and find before they can use it.
                let next = items.last().cloned().unwrap_or(RV::Vec3([0.0; 3]));
                items.push(next);
            }
            CollectionAction::Remove(index) => {
                let index = index as usize;
                if index >= items.len() {
                    return None;
                }
                items.remove(index);
            }
            CollectionAction::Duplicate(index) => {
                let index = index as usize;
                let copy = items.get(index)?.clone();
                items.insert(index + 1, copy);
            }
        }
        Some(RV::Array(items))
    }

    fn numeric_reflect_value(
        binding: &GeneratedBinding,
        edited: f32,
    ) -> Option<somnium_ecs::reflect::ReflectValue> {
        use somnium_ecs::reflect::ReflectValue as RV;
        Some(match (&binding.value, binding.edit) {
            (RV::I64(_), GeneratedEdit::Whole) => RV::I64(edited.round() as i64),
            (RV::F64(_), GeneratedEdit::Whole) => RV::F64(f64::from(edited)),
            (RV::Vec2(current), GeneratedEdit::Lane(lane)) => {
                let mut v = *current;
                v[lane as usize] = edited;
                RV::Vec2(v)
            }
            (RV::Vec3(current), GeneratedEdit::Lane(lane)) => {
                let mut v = *current;
                v[lane as usize] = edited;
                RV::Vec3(v)
            }
            (RV::Vec4(current), GeneratedEdit::Lane(lane)) => {
                let mut v = *current;
                v[lane as usize] = edited;
                RV::Vec4(v)
            }
            (RV::Array(items), GeneratedEdit::Element { index, lane }) => {
                let mut items = items.clone();
                let slot = items.get_mut(index as usize)?;
                match slot {
                    RV::F64(v) => *v = f64::from(edited),
                    RV::I64(v) => *v = edited.round() as i64,
                    RV::Vec2(v) => *v.get_mut(lane as usize)? = edited,
                    RV::Vec3(v) => *v.get_mut(lane as usize)? = edited,
                    RV::Vec4(v) => *v.get_mut(lane as usize)? = edited,
                    _ => return None,
                }
                RV::Array(items)
            }
            (RV::Quat(current), GeneratedEdit::Euler(lane)) => {
                let (x, y, z) = glam::Quat::from_array(*current).to_euler(glam::EulerRot::XYZ);
                let mut euler = [x, y, z];
                euler[lane as usize] = edited.to_radians();
                RV::Quat(
                    glam::Quat::from_euler(glam::EulerRot::XYZ, euler[0], euler[1], euler[2])
                        .to_array(),
                )
            }
            _ => return None,
        })
    }

    pub fn new(
        device: &wgpu::Device,
        output_format: wgpu::TextureFormat,
        _msaa_samples: u32,
        queue: &wgpu::Queue,
        window: Arc<Window>,
    ) -> Self {
        info!("Initializing native UI…");

        let size = window.inner_size();
        // The tree lays out in logical units so every density token keeps its
        // apparent size at any DPI. `inner_size()` is device pixels.
        let ui_scale = window.scale_factor() as f32;
        let (sw, sh) = (size.width as f32 / ui_scale, size.height as f32 / ui_scale);
        let mut native_ui = UserInterface::new(sw, sh);
        native_ui.set_ui_scale(ui_scale);
        // Now that one layout unit is one *logical* pixel, the atlas can finally
        // be told the real device ratio: glyphs rasterize at
        // `px * ui_scale * SUPER_SAMPLE` and land in a quad that is exactly
        // `px * ui_scale` device pixels wide.
        native_ui.draw_ctx.font_atlas.set_render_scale(ui_scale);
        native_ui.draw_ctx.icon_atlas.set_render_scale(ui_scale);

        let font_id = load_fonts(&mut native_ui);

        let layout_sizes = crate::layout_persist::load().resolved(sw, sh);
        let layout = build_editor_layout(&mut native_ui, font_id, layout_sizes);
        let ui_pass = UiPass::new(device, queue, output_format);

        // Tell the UserInterface which handle is the viewport so mouse events pass through.
        native_ui.set_viewport_handle(layout.viewport_handle);

        let mut this = Self {
            window: Arc::clone(&window),
            window_size: (sw.round() as u32, sh.round() as u32),
            physical_size: (size.width, size.height),
            ui_scale,
            last_frame_at: None,
            native_ui,
            ui_pass,
            font_id,
            outliner_scroll: layout.outliner_scroll,
            outliner_stack: layout.outliner_stack,
            inspector_stack: layout.inspector_stack,
            details_empty: layout.details_empty,
            outliner_empty: layout.outliner_empty,
            log_empty: layout.log_empty,
            vp_overlay: layout.vp_overlay,
            vp_overlay_text: layout.vp_overlay_text,
            log_stack: layout.log_stack,
            log_severity_chips: layout.log_severity_chips,
            log_search: layout.log_search,
            log_pin_only: layout.log_pin_only,
            log_copy: layout.log_copy,
            log_clear: layout.log_clear,
            log_jobs_toggle: layout.log_jobs_toggle,
            log_history_toggle: layout.log_history_toggle,
            create_button: layout.create_button,
            create_popup: layout.create_popup,
            create_popup_open: false,
            create_popup_items: layout.create_popup_items,
            foliage_kind_shown: 0,
            file_button: layout.file_button,
            file_popup: layout.file_popup,
            file_menu_stack: layout.file_menu_stack,
            file_popup_open: false,
            open_combo_popup: NodeHandle::NONE,
            menu_command_items: layout.menu_command_items,
            camera_speed_slider: layout.camera_speed_slider,
            camera_speed_label: layout.camera_speed_label,
            viewport_res_combo: layout.viewport_res_combo,
            play_button: layout.play_button,
            play_label: layout.play_label,
            immersive_button: layout.immersive_button,
            pause_button: layout.pause_button,
            step_button: layout.step_button,
            pause_label: layout.pause_label,
            stop_button: layout.stop_button,
            stop_label: layout.stop_label,
            select_button: layout.select_button,
            landscape_button: layout.landscape_button,
            foliage_toolbar_button: layout.foliage_toolbar_button,
            terrain_tool_items: layout.terrain_tool_items,
            outliner_rows: Vec::new(),
            palette_entities: Vec::new(),
            palette_targets: std::collections::HashMap::new(),
            command_history: crate::commands::CommandHistory::load(),
            inspector_handles: layout.inspector_handles,
            editor_events: VecDeque::new(),
            selected_entity: None,
            next_property_gesture: 1,
            viewport_handle: layout.viewport_handle,
            animation_workspace: layout.animation_workspace,
            animation_graph_editor: layout.animation_graph_editor,
            animation_timeline: layout.animation_timeline,
            animation_timeline_document: layout.animation_timeline_document,
            profiler_panel: layout.profiler_panel,
            profiler_toggle: layout.profiler_toggle,
            profiler_toggle_lbl: layout.profiler_toggle_lbl,
            profiler_names: layout.profiler_names,
            profiler_values: layout.profiler_values,
            last_outliner_state: None,
            outer_grid: layout.outer_grid,
            menu_bar_h: layout.menu_bar_h,
            mode_labels: layout.mode_labels,
            collapsed_at: None,
            status_dirty: layout.status_dirty,
            status_selection: layout.status_selection,
            status_stats: layout.status_stats,
            status_stats_button: layout.status_stats_button,
            generated_root: NodeHandle::NONE,
            generated_entity: None,
            generated_signature: Vec::new(),
            generated_bindings: HashMap::new(),
            generated_collection_actions: HashMap::new(),
            drop_probe: String::new(),
            generated_rows: HashMap::new(),
            generated_gestures: HashMap::new(),
            generated_asset_choices: HashMap::new(),
            generated_asset_searches: HashMap::new(),
            generated_asset_actions: HashMap::new(),
            inner_h: layout.inner_h,
            content_split_h: layout.content_split_h,
            right_split_h: layout.right_split_h,
            toolbar_h: layout.toolbar_h,
            right_h: layout.right_h,
            bottom_h: layout.bottom_h,
            fps_text: layout.fps_text,
            help_button: layout.help_button,
            help_overlay: layout.help_overlay,
            help_body: layout.help_body,
            tooltip: layout.tooltip,
            edit_button: layout.edit_button,
            view_button: layout.view_button,
            window_button: layout.window_button,
            help_menu_button: layout.help_menu_button,
            edit_popup: layout.edit_popup,
            view_popup: layout.view_popup,
            window_popup: layout.window_popup,
            help_menu_popup: layout.help_menu_popup,
            active_workspace: crate::workspace::Workspace::Layout,
            drawer_height: theme::BOTTOM_DRAWER_HEIGHT,
            status_text: layout.status_text,
            status_cancel: layout.status_cancel,
            status_cancel_job: None,
            drawer_button: layout.drawer_button,
            log_button: layout.log_button,
            content_drawer: layout.content_drawer,
            content_search: layout.content_search,
            content_breadcrumb: layout.content_breadcrumb,
            content_engine_toggle: layout.content_engine_toggle,
            content_scroll: layout.content_scroll,
            content_list: layout.content_list,
            content_toolbar_actions: layout.content_toolbar_actions,
            outliner_tree: layout.outliner_tree,
            outliner_search: layout.outliner_search,
            inspector_search: layout.inspector_search,
            foliage_kind_combo: layout.foliage_kind_combo,
            foliage_kind_popup: layout.foliage_kind_popup,
            viewport_res_popup: layout.viewport_res_popup,
            snap_grid_popup: layout.snap_grid_popup,
            snap_angle_popup: layout.snap_angle_popup,
            save_button: layout.save_button,
            palette_button: layout.palette_button,
            palette_popup: layout.palette_popup,
            palette_widget: layout.palette_widget,
            toast_host: layout.toast_host,
            unsaved_popup: layout.unsaved_popup,
            unsaved_save: layout.unsaved_save,
            unsaved_discard: layout.unsaved_discard,
            unsaved_cancel: layout.unsaved_cancel,
            content_menu_popup: layout.content_menu_popup,
            content_menu: layout.content_menu,
            outliner_menu_popup: layout.outliner_menu_popup,
            outliner_menu: layout.outliner_menu,
            preferences: layout.preferences,
            snap_cluster: layout.snap_cluster,
            snap_grid_combo: layout.snap_grid_combo,
            snap_angle_combo: layout.snap_angle_combo,
            snap_surface_toggle: layout.snap_surface_toggle,
            gizmo_space_toggle: layout.gizmo_space_toggle,
            gizmo_space_label: layout.gizmo_space_label,
            select_only_toggle: layout.select_only_toggle,
            snap_overflow: layout.snap_overflow,
            time_cluster: layout.time_cluster,
            time_label: layout.time_label,
            time_slider: layout.time_slider,
            time_shown: None,
            play_session_active: false,
            name_popup: layout.name_popup,
            name_title: layout.name_title,
            name_input: layout.name_input,
            name_ok: layout.name_ok,
            name_cancel: layout.name_cancel,
            color_popup: layout.color_popup,
            color_picker: layout.color_picker,
            help_open: false,
            drawer_open: true,
            drawer_docked: true,
            show_engine_content: false,
            inspector_filter: String::new(),
            content_filter: String::new(),
            content_path: String::new(),
            content_history: crate::metaphor::ContentHistory::new(String::new()),
            content_kind: crate::metaphor::ContentFilterKind::All,
            content_density: crate::metaphor::ContentDensity::Comfortable,
            asset_db: somnium_asset::database::AssetDbSnapshot::default(),
            content_sort: somnium_asset::database::AssetSort::Name,
            content_sort_descending: false,
            content_selection: std::collections::HashSet::new(),
            outliner_expanded: std::collections::HashSet::new(),
            log: crate::log::OutputLog::default(),
            log_rows: Vec::new(),
            log_view: LogView::Log,
            sticky_toasts: 0,
            log_clock: std::time::Instant::now(),
            job_rows: Vec::new(),
            scene_name: None,
            history_rows: Vec::new(),
            history_position: 0,
            edit_popup_open: false,
            view_popup_open: false,
            window_popup_open: false,
            help_menu_open: false,
            tooltip_since: None,
            help_page: 0,
            content_entries: Vec::new(),
            content_all: Vec::new(),
            content_window: None,
            dependency_index: somnium_asset::depend::DependencyIndex::default(),
            references_subject: None,
            references_rows: Vec::new(),
            references_open: false,
            outliner_entity_handles: HashMap::new(),
            outliner_selection: Vec::new(),
            clipboard_filled: false,
            keybindings: crate::commands::KeyBindings::load(),
            rebinding: None,
            preferences_open: false,
            preferences_bindings_tab: false,
            preferences_query: String::new(),
            preferences_modified_only: false,
            settings_bindings: HashMap::new(),
            settings_rows: HashMap::new(),
            settings_root: NodeHandle::NONE,
            settings_signature: Vec::new(),
            settings_panels: Vec::new(),
            binding_rows: Vec::new(),
            recent_scenes: Vec::new(),
            recent_menu_items: Vec::new(),
            active_debug_view: "lit",
            render_toggles: crate::debug::DebugToggles::default(),
            piercing_rows: Vec::new(),
            tooltip_delay_ms: 500.0,
            badge_drag_value: None,
            marquee_active: false,
            viewport_drop_probe: (None, None),
            external_drag_files: Vec::new(),
            outliner_filter: String::new(),
            palette_open: false,
            unsaved_open: false,
            pending_scene_action: None,
            color_open: false,
            color_target: None,
            color_gesture: None,
            color_original: [1.0, 1.0, 1.0, 1.0],
            color_live: [1.0, 1.0, 1.0, 1.0],
            content_menu_target: None,
            content_menu_folder: String::new(),
            content_inline_rename: None,
            name_prompt: None,
            name_text: String::new(),
            script_state: ScriptInspectorState::default(),
            script_widgets: Vec::new(),
            scene_dirty: false,
            script_errors: 0,
            chrome_layout: layout_sizes,
            log_open: false,
            title_drag: layout.title_drag,
            title_label: layout.title_label,
            win_min: layout.win_min,
            win_max: layout.win_max,
            win_close: layout.win_close,
            help_toc: layout.help_toc,
            help_close: layout.help_close,
            log_panel: layout.log_panel,
            references_panel: layout.references_panel,
            references_button: layout.references_button,
            references_title: layout.references_title,
            references_list: layout.references_list,
            locale_panel: layout.locale_panel,
            locale_button: layout.locale_button,
            locale_search: layout.locale_search,
            locale_incomplete: layout.locale_incomplete,
            locale_grid: layout.locale_grid,
            locale_actions: layout.locale_actions,
            locale_open: false,
            viewport_layout: crate::viewport_layout::ViewportLayout::from_env().unwrap_or_default(),
            locale_only_incomplete: false,
            locale_table: None,
            title_last_click: None,
            immersive: false,
            immersive_restore_fullscreen: None,
            immersive_restore_maximized: false,
        };
        // CONTROL-A evidence harness. A relative drawer path lets a timing run
        // queue the real `assets/terrain/` workload before frame one, without
        // synthesising mouse input. Parent/root/prefix components are refused:
        // this diagnostic may inspect the assets tree and nothing outside it.
        if let Ok(raw) = std::env::var("SOMNIUM_AUDIT_CONTENT_PATH") {
            let relative = std::path::Path::new(raw.trim());
            let safe = !relative.as_os_str().is_empty()
                && relative
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_)));
            let root = std::env::current_dir().unwrap_or_default().join("assets");
            if safe && root.join(relative).is_dir() {
                let path = relative.to_string_lossy().replace('\\', "/");
                this.content_path.clone_from(&path);
                this.content_history = crate::metaphor::ContentHistory::new(path);
            } else {
                warn!(
                    "SOMNIUM_AUDIT_CONTENT_PATH ignored: expected an existing relative assets folder"
                );
            }
        }
        this.refresh_content_list();
        // Nothing is selected at startup and `update_inspector` has not run
        // yet, so the two Details states would otherwise both be visible and
        // overlap. Seed the empty one.
        this.native_ui.set_visibility(this.inspector_stack, false);
        this.native_ui.set_visibility(this.details_empty, true);
        // The Outliner is repopulated on the first frame and flips itself; the
        // log has nothing until something logs.
        this.native_ui.set_visibility(this.log_stack, false);
        this.native_ui.set_visibility(this.log_empty, true);
        // A window that opens narrow must start collapsed, not collapse on its
        // first resize.
        this.apply_collapse_rules(this.window_size.0);
        this.apply_audit_startup_state();
        this
    }

    /// Put one existing editor surface into a deterministic startup state for
    /// CONTROL-A captures. This is inert unless the audit variable is present;
    /// it calls the same paths as clicks/shortcuts and owns no parallel UI.
    fn apply_audit_startup_state(&mut self) {
        let Ok(raw) = std::env::var("SOMNIUM_AUDIT_UI_STATE") else {
            return;
        };
        match raw.trim().to_ascii_lowercase().as_str() {
            "shell" => {}
            "menu-file" => self.open_menu(0),
            "menu-create" => self.open_menu(1),
            "menu-edit" => self.open_menu(2),
            "menu-view" => self.open_menu(3),
            "menu-window" => self.open_menu(4),
            "menu-help" => self.open_menu(5),
            "palette" => self.toggle_palette(),
            "help" => self.toggle_help(Some(0)),
            "log" => self.toggle_log_panel(),
            // Goes through the command/event boundary, exactly like the
            // viewport button. The host applies it on the first frame.
            "profiler" => {
                self.run_command_id("editor.view.profiler");
            }
            "modal-unsaved" => {
                self.scene_dirty = true;
                self.prompt_unsaved_new();
            }
            other => warn!("SOMNIUM_AUDIT_UI_STATE={other} ignored: unknown audit surface"),
        }
    }

    // ── Window integration ────────────────────────────────────────────────────

    pub fn reposition_panels(&mut self, window: &Window) {
        let size = window.inner_size();
        let ui_scale = window.scale_factor() as f32;
        let logical_w = size.width as f32 / ui_scale;
        let logical_h = size.height as f32 / ui_scale;

        self.ui_scale = ui_scale;
        self.physical_size = (size.width, size.height);
        self.window_size = (logical_w.round() as u32, logical_h.round() as u32);

        self.native_ui.set_ui_scale(ui_scale);
        // Dragging a window between monitors changes the scale factor, which
        // invalidates every cached glyph. `set_render_scale` is a no-op when the
        // ratio is unchanged, so this costs nothing on an ordinary resize.
        self.native_ui
            .draw_ctx
            .font_atlas
            .set_render_scale(ui_scale);
        self.native_ui
            .draw_ctx
            .icon_atlas
            .set_render_scale(ui_scale);
        self.native_ui.resize(logical_w, logical_h);
        self.apply_collapse_rules(self.window_size.0);
    }

    /// Switch to a named workspace and lay the shell out accordingly.
    ///
    /// Phase 26-Zeta-F. The preset resolves against the current window size,
    /// so a workspace authored for 1080p does not become a broken arrangement
    /// on an ultrawide or a laptop. The result is persisted like any manual
    /// splitter drag, so the choice survives a restart.
    pub fn set_workspace(&mut self, workspace: crate::workspace::Workspace) {
        self.active_workspace = workspace;
        self.native_ui.set_visibility(
            self.animation_workspace,
            workspace == crate::workspace::Workspace::Animation,
        );
        let (w, h) = (self.window_size.0 as f32, self.window_size.1 as f32);
        let preset = workspace.preset(w, h);
        self.apply_workspace_layout(preset, w);
        self.push_toast(&format!("{} workspace", workspace.label()));
    }

    /// Open an authored animation state machine on the shipped graph surface.
    ///
    /// The graph control becomes the document owner; the shell does not mirror
    /// the graph or transition overlay into a second editor-side model.
    pub fn edit_animation_state_machine(
        &mut self,
        document: crate::graph::AnimationStateMachineDocument,
    ) {
        self.native_ui.send(
            crate::graph::GraphEditorMessage::set_state_machine_document(
                self.animation_graph_editor,
                document,
            ),
        );
        self.set_workspace(crate::workspace::Workspace::Animation);
    }

    /// Open a track document on MORROWIND-L's shared timeline in the shipped
    /// Animation workspace.
    pub fn edit_animation_timeline(&mut self, document: crate::timeline::TimelineDocument) {
        self.animation_timeline_document = document.clone();
        self.native_ui
            .send(crate::timeline::TimelineEditorMessage::set_document(
                self.animation_timeline.editor,
                document,
            ));
        self.set_workspace(crate::workspace::Workspace::Animation);
    }

    /// Latest authored timeline, including live edits from the embedded curve
    /// editor. The animation asset owner reads this when saving.
    #[must_use]
    pub fn animation_timeline_document(&self) -> &crate::timeline::TimelineDocument {
        &self.animation_timeline_document
    }

    /// Return the current workspace to its shipped arrangement.
    pub fn reset_workspace(&mut self) {
        let workspace = self.active_workspace;
        self.set_workspace(workspace);
    }

    pub fn active_workspace(&self) -> crate::workspace::Workspace {
        self.active_workspace
    }

    fn apply_workspace_layout(&mut self, preset: crate::workspace::WorkspaceLayout, window_w: f32) {
        use crate::workspace::BottomPanel;

        let set_split = |ui: &mut UserInterface, handle: NodeHandle, size: f32| {
            ui.send(UiMessage::new(
                handle,
                MessageDirection::ToWidget,
                SplitterMessage::SetFirstSize(size),
            ));
        };
        set_split(&mut self.native_ui, self.inner_h, preset.tools);
        // The content splitter stores the *viewport* width, so the Details
        // column is what is left over.
        set_split(
            &mut self.native_ui,
            self.content_split_h,
            (window_w - preset.tools - preset.details).max(200.0),
        );
        set_split(&mut self.native_ui, self.right_split_h, preset.outliner);

        self.drawer_height = preset.drawer_height;
        match preset.bottom {
            BottomPanel::None => {
                self.drawer_open = false;
                self.log_open = false;
            }
            BottomPanel::Content => {
                self.drawer_open = true;
                self.log_open = false;
            }
            BottomPanel::Log => {
                self.drawer_open = false;
                self.log_open = true;
            }
        }
        // Neither References nor Localisation is ever a preset's answer: one
        // is scoped to an asset a preset does not know, and the other is a
        // document you open on purpose.
        self.references_open = false;
        self.locale_open = false;
        self.apply_bottom_panel();

        self.chrome_layout = crate::layout_persist::ChromeLayout {
            tools: preset.tools,
            viewport: (window_w - preset.tools - preset.details).max(200.0),
            details: preset.details,
            outliner: preset.outliner,
        };
        crate::layout_persist::save(self.chrome_layout);
        self.native_ui.invalidate_ancestors(self.outer_grid);
    }

    /// Apply the redline §06 collapse rules for a window width.
    ///
    /// Phase 26-Zeta-F. The rules are stated as logical widths, and each one
    /// removes the least load-bearing thing at that width rather than shrinking
    /// everything: nothing becomes unreachable, it only becomes terser. Called
    /// on resize and once at startup.
    pub fn apply_collapse_rules(&mut self, width: u32) {
        if self.collapsed_at == Some(width) {
            return;
        }
        self.collapsed_at = Some(width);
        let rules = CollapseRules::for_width(width as f32);

        // Mode scope: the play-state word goes; the transport glyphs and the
        // named mode commands stay.
        self.native_ui
            .set_visibility(self.play_label, rules.transport_label);
        // Application scope: the 320 px search field is the widest optional
        // thing in the band. Ctrl+P still opens the palette when it is hidden,
        // and the Help menu still lists it.
        self.native_ui
            .set_visibility(self.palette_button, rules.search_field);
        self.native_ui.invalidate_ancestors(self.outer_grid);
    }

    /// Debug layout dump helper
    pub fn debug_dump_layout(&self) {
        info!("=== UI LAYOUT DEBUG DUMP ===");
        let print_widget = |name: &str, handle: NodeHandle| {
            if let Some(node) = self.native_ui.nodes.try_borrow(handle.transmute()).ok() {
                info!(
                    "{}: pos={:?}, size={:?}, desired={:?}, clip={:?}, vis={}, g_vis={}",
                    name,
                    node.widget.actual_local_position,
                    node.widget.actual_local_size,
                    node.widget.desired_size,
                    node.widget.clip_bounds,
                    node.widget.visibility,
                    node.widget.global_visibility
                );
            } else {
                warn!("{}: NOT FOUND", name);
            }
        };
        print_widget("Outer Grid", self.outer_grid);
        print_widget("Menu Bar", self.menu_bar_h);
        print_widget("Inner Grid", self.inner_h);
        print_widget("Toolbar", self.toolbar_h);
        print_widget("Right Panel", self.right_h);
        print_widget("Bottom Panel", self.bottom_h);
    }

    /// No-op stub — existing game code calls compile without changes.
    pub fn send_message<T>(&self, _msg_type: &str, _data: T) {}

    pub fn begin_frame(&mut self, _window: &Window) {}

    /// Layout, draw, GPU upload, and render the native UI overlay.
    pub fn end_frame(
        &mut self,
        window: &Window,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
    ) {
        // Scale and logical size are owned by `reposition_panels`, which winit
        // calls for both Resized and ScaleFactorChanged. Nothing to do here.

        // Phase 27-C. Advance motion before layout so a track that finished
        // this frame settles into the layout it produced. The first frame has
        // no previous timestamp and therefore advances nothing, and a tick with
        // no live tracks returns false without touching the tree — which is what
        // keeps an idle shell's draw list byte-identical.
        // Phase 27-G. Decode a bounded number of previews per frame, so
        // opening a folder of textures costs a predictable slice rather than an
        // unpredictable stall. Anything the engine must render is handed to the
        // host through `take_thumbnail_requests`.
        self.native_ui.draw_ctx.thumbnails.pump();

        let now = std::time::Instant::now();
        if let Some(previous) = self.last_frame_at {
            let dt_ms = now.duration_since(previous).as_secs_f32() * 1000.0;
            // A stalled frame (breakpoint, minimised window) must not teleport
            // every track to its end state.
            self.native_ui.draw_ctx.motion.tick(dt_ms.min(100.0));
        }
        self.last_frame_at = Some(now);

        // Flush all queued widget messages; convert outgoing to EditorEvents.
        let outgoing = self.native_ui.update();
        self.process_outgoing(outgoing);
        // Apply layout messages sent from those handlers (drawer row height, etc.)
        // before measure/draw, so a just-opened pane is never laid out at 0px.
        let extra = self.native_ui.update();
        self.process_outgoing(extra);

        let (logical_w, logical_h) = (self.window_size.0 as f32, self.window_size.1 as f32);
        let (phys_w, phys_h) = self.physical_size;
        self.native_ui.perform_layout();
        // MORROWIND-M. Between the two layout passes on purpose: the first
        // gives the drawer's canvas the bounds this reads, and the second
        // arranges whatever tiles the new window built, so a scroll costs one
        // frame's rebuild rather than a frame of empty drawer.
        self.sync_content_tiles();
        self.request_visible_thumbnails();
        self.reanchor_open_popups();
        self.update_tooltip();
        self.native_ui.perform_layout();
        self.native_ui.draw();
        window.set_cursor(self.native_ui.cursor_kind().to_winit());
        self.ui_pass.prepare(
            device,
            queue,
            &mut self.native_ui.draw_ctx,
            crate::pass::UiSurface::new((logical_w, logical_h), (phys_w, phys_h)),
        );
        self.ui_pass.render(encoder, view);
    }

    // ── OS event routing ─────────────────────────────────────────────────────

    /// Route a winit event into the widget tree.  Returns true if consumed.
    pub fn process_os_event(&mut self, event: &WindowEvent) -> bool {
        if self.immersive {
            if let WindowEvent::KeyboardInput { event: key_ev, .. } = event {
                let pressed = key_ev.state == ElementState::Pressed;
                if pressed {
                    if let PhysicalKey::Code(KeyCode::Escape) = key_ev.physical_key {
                        self.editor_events
                            .push_back(EditorEvent::ToggleImmersiveViewport);
                        return true;
                    }
                }
            }
            return false;
        }
        if let WindowEvent::ModifiersChanged(m) = event {
            let state = m.state();
            self.native_ui.set_modifiers(Modifiers {
                ctrl: state.control_key(),
                shift: state.shift_key(),
                alt: state.alt_key(),
                logo: state.super_key(),
            });
        }
        if let WindowEvent::CursorMoved { position, .. } = event {
            self.native_ui.cursor_pos = self.native_ui.to_logical(position.x, position.y);
        }
        if let WindowEvent::HoveredFile(path) = event {
            if !self.external_drag_files.contains(path) {
                self.external_drag_files.push(path.clone());
            }
            self.native_ui
                .begin_external_drag(crate::drag_drop::DragPayload::ExternalFiles(
                    self.external_drag_files.clone(),
                ));
            self.refresh_drop_acceptance();
            return true;
        }
        if matches!(event, WindowEvent::HoveredFileCancelled) {
            self.external_drag_files.clear();
            self.native_ui.cancel_active_gesture();
            return true;
        }
        if let WindowEvent::DroppedFile(path) = event {
            if !self.external_drag_files.contains(path) {
                self.external_drag_files.push(path.clone());
            }
            self.native_ui
                .begin_external_drag(crate::drag_drop::DragPayload::ExternalFiles(
                    self.external_drag_files.clone(),
                ));
            self.refresh_drop_acceptance();
            self.complete_drop();
            self.external_drag_files.clear();
            return true;
        }
        // Phase 16-D: the Content Drawer's right-click menu.
        //
        // Intercepted here rather than as a widget message because the
        // menu is about the *drawer*, not about whichever button happens
        // to be under the cursor — right-clicking the gap between two
        // items has to work, and no widget owns the gap.
        if let WindowEvent::MouseInput {
            state: ElementState::Pressed,
            button: winit::event::MouseButton::Right,
            ..
        } = event
        {
            if self.native_ui.cancel_active_gesture() {
                return true;
            }
            if self.native_ui.modifiers().command()
                && self.native_ui.is_under(
                    self.native_ui.hit_test(self.native_ui.cursor_pos),
                    self.viewport_handle,
                )
            {
                self.editor_events.push_back(EditorEvent::OpenPiercingMenu);
                return true;
            }
            if self.open_content_menu(self.native_ui.cursor_pos) {
                return true;
            }
            if self.open_outliner_menu(self.native_ui.cursor_pos) {
                return true;
            }
        }
        if let WindowEvent::KeyboardInput { event: key_ev, .. } = event {
            let pressed = key_ev.state == ElementState::Pressed;
            if let PhysicalKey::Code(code) = key_ev.physical_key {
                // Two guards, and the second one was missing.
                //
                // `has_text_focus` keeps a shortcut out of a text field. The
                // fly-cam needs the same protection and did not have it: `S`
                // is bound to the Scale tool *and* is "move backward", so
                // holding right-mouse and pressing `S` ran the command and the
                // press never reached the game. It appeared to work a couple of
                // seconds later only because OS key-repeat sets `repeat`, which
                // skips this branch — so the camera started moving exactly when
                // the keyboard began auto-repeating.
                //
                // The game layer already guards its own `W`/`E` gizmo
                // shortcuts on `!is_rmb_down`; that guard was dead for any key
                // the registry claimed first, because the registry intercepts a
                // layer above it. This is the same rule applied where the
                // interception actually happens.
                if pressed && !key_ev.repeat && !self.native_ui.has_text_focus() {
                    let chord = crate::commands::Chord::from_winit(
                        code,
                        self.native_ui.modifiers().command(),
                        self.native_ui.modifiers().shift,
                        self.native_ui.modifiers().alt,
                        false,
                    )
                    // Same rule as the engine's dispatcher, and it has to be
                    // here too: the *rebindable* lookup lives on this side, so
                    // a user-rebound bare key would otherwise still be eaten
                    // while the game has the keyboard.
                    .filter(|chord| !self.game_owns_keyboard() || chord.has_modifier());
                    if let Some(chord) = chord {
                        // CONTROL-H: a rebinding editor is only real if the
                        // override is what dispatch consults. The registry's
                        // declared chord is the fallback, not the authority.
                        if self.rebinding.is_some() {
                            self.capture_rebind(chord);
                            return true;
                        }
                        if let Some(id) = self.keybindings.command_for(chord)
                            && self.run_command_id(id)
                        {
                            return true;
                        }
                    }
                }
                match code {
                    KeyCode::ControlLeft | KeyCode::ControlRight => {
                        let mut modifiers = self.native_ui.modifiers();
                        modifiers.ctrl = pressed;
                        self.native_ui.set_modifiers(modifiers);
                    }
                    KeyCode::ShiftLeft | KeyCode::ShiftRight => {
                        let mut modifiers = self.native_ui.modifiers();
                        modifiers.shift = pressed;
                        self.native_ui.set_modifiers(modifiers);
                    }
                    KeyCode::AltLeft | KeyCode::AltRight => {
                        let mut modifiers = self.native_ui.modifiers();
                        modifiers.alt = pressed;
                        self.native_ui.set_modifiers(modifiers);
                    }
                    KeyCode::SuperLeft | KeyCode::SuperRight => {
                        let mut modifiers = self.native_ui.modifiers();
                        modifiers.logo = pressed;
                        self.native_ui.set_modifiers(modifiers);
                    }
                    KeyCode::Escape if pressed => {
                        // `Esc` runs outermost-first, which is the precedence
                        // CONTROL-A1 established and every sub-phase since has
                        // extended rather than rearranged:
                        //
                        //   a pending rebind · a persistent error toast ·
                        //   a viewport rubber-band · a drag or control gesture ·
                        //   the overlay stack.
                        //
                        // A rebind is first because it is *listening* — every
                        // other key would be swallowed by it. An error toast is
                        // next because it is the one overlay that outlives the
                        // action that raised it, so nothing else will remove it.
                        if self.rebinding.take().is_some() {
                            return true;
                        }
                        if self.dismiss_toast() {
                            return true;
                        }
                        if self.marquee_active {
                            self.marquee_active = false;
                            self.editor_events.push_back(EditorEvent::CancelMarquee);
                            return true;
                        }
                        if self.native_ui.cancel_active_gesture() {
                            return true;
                        }
                        if self.close_top_overlay() {
                            return true;
                        }
                    }
                    // Tab moves between the shell's major regions. It is
                    // deliberately region-level rather than control-level: the
                    // design's focus order gives the viewport, the Outliner and
                    // each Details section *one* stop each and expects arrow
                    // keys to traverse inside them, so tabbing through 120
                    // property fields would be the wrong shape.
                    //
                    // A focused text field keeps Tab for itself, so typing in
                    // the search box is not interrupted.
                    KeyCode::Tab if pressed && self.native_ui.modal_root().is_some() => {
                        self.advance_focus(if self.native_ui.modifiers().shift {
                            -1
                        } else {
                            1
                        });
                        return true;
                    }
                    KeyCode::Tab if pressed && !self.native_ui.has_text_focus() => {
                        self.advance_focus(if self.native_ui.modifiers().shift {
                            -1
                        } else {
                            1
                        });
                        return true;
                    }
                    KeyCode::ArrowUp
                    | KeyCode::ArrowDown
                    | KeyCode::ArrowLeft
                    | KeyCode::ArrowRight
                    | KeyCode::Home
                    | KeyCode::End
                        if pressed && self.native_ui.focused() == self.outliner_tree =>
                    {
                        self.native_ui.send(UiMessage::new(
                            self.outliner_tree,
                            MessageDirection::ToWidget,
                            WidgetMessage::KeyDown(code, self.native_ui.modifiers()),
                        ));
                        return true;
                    }
                    KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::Home | KeyCode::End
                        if pressed
                            && (self.native_ui.focused() == self.inspector_search
                                || self
                                    .native_ui
                                    .is_under(self.native_ui.focused(), self.inspector_stack)) =>
                    {
                        self.native_ui.traverse_region(self.inspector_stack, code);
                        return true;
                    }
                    _ => {}
                }
            }
        }
        if let WindowEvent::MouseInput {
            state: ElementState::Pressed,
            button: winit::event::MouseButton::Left,
            ..
        } = event
        {
            if let Some((axis, positive)) =
                self.native_ui.axis_widget_hit(self.native_ui.cursor_pos)
            {
                // The widget's ends map onto the view presets: clicking +Y is
                // Top, and the negative ends are the same presets from the
                // other side, which is why the preset carries the sign.
                let preset = match (axis, positive) {
                    (1, _) => 0,
                    (2, _) => 1,
                    _ => 2,
                };
                self.editor_events
                    .push_back(EditorEvent::ViewPreset(preset));
                return true;
            }
            self.arm_internal_drag();
            let hit = self.native_ui.hit_test(self.native_ui.cursor_pos);
            if self.is_title_chrome_hit(hit) {
                if hit == self.win_min || hit == self.win_max || hit == self.win_close {
                    // Fall through so the button Click is delivered.
                } else {
                    let now = std::time::Instant::now();
                    let double = self
                        .title_last_click
                        .map(|t| now.duration_since(t).as_millis() < 400)
                        .unwrap_or(false);
                    self.title_last_click = Some(now);
                    if double {
                        self.window.set_maximized(!self.window.is_maximized());
                    } else {
                        let _ = self.window.drag_window();
                    }
                    return true;
                }
            }
            if self.transient_overlay_open() {
                if self.hit_is_inside_transient_content(hit) {
                    // Keep the overlay; let the widget handle the click.
                } else if self.menu_button_for(hit).is_some() {
                    let opener = self.menu_button_for(hit);
                    let current_opener = self.open_menu_button();
                    if opener != current_opener {
                        self.close_all_menus();
                    }
                    if self.help_open {
                        self.help_open = false;
                        self.native_ui.send(UiMessage::new(
                            self.help_overlay,
                            MessageDirection::ToWidget,
                            PopupMessage::Close,
                        ));
                    }
                    // Fall through so the clicked menu can toggle.
                } else {
                    self.close_top_overlay();
                    return true;
                }
            }
        }
        let consumed = self.native_ui.process_os_event(event);
        if matches!(event, WindowEvent::CursorMoved { .. }) && self.native_ui.is_dragging() {
            self.refresh_drop_acceptance();
            return true;
        }
        if matches!(
            event,
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: winit::event::MouseButton::Left,
                ..
            }
        ) {
            self.badge_drag_value = None;
            self.complete_drop();
        }
        consumed
    }

    fn arm_internal_drag(&mut self) {
        let hit = self.native_ui.hit_test(self.native_ui.cursor_pos);
        if let Some(entry) = self
            .content_entries
            .iter()
            .find_map(|(handle, entry)| self.native_ui.is_under(hit, *handle).then_some(entry))
        {
            let mut ids: Vec<_> = if self.content_selection.contains(&entry.path) {
                self.content_entries
                    .iter()
                    .filter(|(_, e)| self.content_selection.contains(&e.path))
                    .filter_map(|(_, e)| e.asset_id)
                    .collect()
            } else {
                entry.asset_id.into_iter().collect()
            };
            ids.sort_by_key(|id| id.raw());
            ids.dedup();
            if ids.is_empty() {
                // The tile is on screen but the asset index has no id for it.
                // Silence here reads as "drag and drop is broken"; naming it
                // reads as "wait for the scan", which is what it means.
                tracing::warn!(
                    path = %entry.path.display(),
                    "drag: this item is not in the asset index yet, so it cannot be dragged"
                );
                self.push_toast("Not indexed yet — refresh the Content Drawer and try again");
                return;
            }
            tracing::info!(count = ids.len(), "drag: armed from the Content Drawer");
            self.native_ui
                .arm_drag(crate::drag_drop::DragPayload::Assets(ids));
            return;
        }
        if self.native_ui.is_under(hit, self.outliner_tree) {
            let bounds = self.native_ui.screen_bounds(self.outliner_tree);
            let row = ((self.native_ui.cursor_pos.y - bounds.y) / crate::theme::TREE_ROW_HEIGHT)
                .floor() as usize;
            if let Some((_, id)) = self.outliner_rows.get(row)
                && let Some(entity) = self.outliner_entity_handles.get(id).copied()
            {
                // Dragging a row that is part of the selection drags the whole
                // selection; dragging an unselected row drags just that row,
                // which is the convention every file manager shares.
                let entities = if self.outliner_selection.contains(id) {
                    self.outliner_selection
                        .iter()
                        .filter_map(|id| self.outliner_entity_handles.get(id).copied())
                        .collect()
                } else {
                    vec![entity]
                };
                self.native_ui
                    .arm_drag(crate::drag_drop::DragPayload::Entities(entities));
            }
        }
    }

    fn refresh_drop_acceptance(&mut self) {
        let Some(payload) = self.native_ui.drag_payload().cloned() else {
            return;
        };
        let hit = self.native_ui.hit_test(self.native_ui.cursor_pos);
        // Resolution order is the ancestor walk in practice: the innermost
        // registered surface under the pointer wins, and each branch reports
        // the exact rectangle it will highlight so the adorner cannot claim a
        // target the release would not use.
        let resolved = if self.native_ui.is_under(hit, self.viewport_handle)
            || hit == self.viewport_handle
        {
            Some((
                crate::drag_drop::DropTarget::Viewport {
                    entity: self.viewport_drop_probe.0,
                    terrain_hit: self.viewport_drop_probe.1,
                },
                self.native_ui.screen_bounds(self.viewport_handle),
            ))
        } else if let Some((row, binding)) = self.generated_rows.iter().find(|(row, binding)| {
            self.native_ui.is_under(hit, **row)
                && matches!(binding.value, somnium_ecs::reflect::ReflectValue::Asset(_))
        }) {
            self.generated_entity.map(|entity| {
                (
                    crate::drag_drop::DropTarget::AssetField {
                        entity,
                        component: binding.component,
                        field: binding.field,
                        kind_mask: binding.asset_kind_mask,
                    },
                    self.native_ui.screen_bounds(*row),
                )
            })
        } else if self.native_ui.is_under(hit, self.outliner_tree) {
            let bounds = self.native_ui.screen_bounds(self.outliner_tree);
            let row = ((self.native_ui.cursor_pos.y - bounds.y) / crate::theme::TREE_ROW_HEIGHT)
                .floor() as usize;
            let entity = self
                .outliner_rows
                .get(row)
                .and_then(|(_, id)| self.outliner_entity_handles.get(id))
                .copied();
            let highlight = if entity.is_some() {
                types::Rect::new(
                    bounds.x,
                    bounds.y + row as f32 * crate::theme::TREE_ROW_HEIGHT,
                    bounds.w,
                    crate::theme::TREE_ROW_HEIGHT,
                )
            } else {
                bounds
            };
            Some((crate::drag_drop::DropTarget::Outliner(entity), highlight))
        } else if self.native_ui.is_under(hit, self.content_drawer) {
            Some((
                crate::drag_drop::DropTarget::DrawerFolder(std::path::PathBuf::from(
                    &self.content_path,
                )),
                self.native_ui.screen_bounds(self.content_drawer),
            ))
        } else {
            None
        };
        let Some((target, highlight)) = resolved else {
            // Nothing under the pointer registered as a target. This is the
            // case that used to end in complete silence: no acceptance means
            // no rejection reason either, so the release had nothing to
            // report and the drag simply evaporated.
            self.drop_probe = if hit.is_none() {
                "the viewport".to_string()
            } else {
                "something that is not a drop target".to_string()
            };
            self.native_ui.set_drop_acceptance(None);
            self.native_ui.set_drop_highlight(None);
            return;
        };
        self.drop_probe = match &target {
            crate::drag_drop::DropTarget::AssetField { .. } => "an asset field".to_string(),
            crate::drag_drop::DropTarget::Outliner(_) => "the Outliner".to_string(),
            crate::drag_drop::DropTarget::Viewport { .. } => "the viewport".to_string(),
            crate::drag_drop::DropTarget::DrawerFolder(_) => "the Content Drawer".to_string(),
        };
        tracing::debug!(target = %self.drop_probe, "drag: pointer is over a target");
        let candidate = crate::drag_drop::acceptance_for(&self.asset_db, &payload, target.clone());
        let acceptance = match crate::drag_drop::semantic_request(
            &self.asset_db,
            &payload,
            &candidate,
            self.native_ui.modifiers(),
        ) {
            Ok(_) => candidate,
            Err(reason) => crate::drag_drop::DropAcceptance::rejected(target, reason),
        };
        self.native_ui.set_drop_highlight(Some(highlight));
        self.native_ui.set_drop_acceptance(Some(acceptance));
    }

    /// What "open" means for one content item.
    ///
    /// One function because the double-click and the context menu's `Open`
    /// have to agree, and because "double-click does nothing for this kind"
    /// is exactly the sort of gap that appears when two call sites each
    /// handle their own subset.
    fn open_content_entry(&mut self, entry: &crate::metaphor::ContentEntry) {
        if entry.is_dir {
            self.navigate_content(entry.path.to_string_lossy().into_owned());
            return;
        }
        if entry.is_engine {
            self.push_toast("Built-in Engine assets have nothing to open");
            return;
        }
        let extension = entry
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match extension.as_str() {
            "somnium" => self.prompt_unsaved_action(EditorEvent::LoadScene(
                entry.path.to_string_lossy().into_owned(),
            )),
            // Everything else goes to whatever the OS or the preference says
            // opens it. Silently doing nothing is not one of the options.
            _ => self.editor_events.push_back(EditorEvent::EditContentAsset(
                entry.path.to_string_lossy().into_owned(),
            )),
        }
    }

    fn complete_drop(&mut self) {
        self.native_ui.set_drop_highlight(None);
        // Read both before taking the drop, because taking it clears the state
        // that knows why.
        let was_dragging = self.native_ui.is_dragging();
        let refused = self.native_ui.drop_rejection_reason();
        let Some(drop) = self.native_ui.take_completed_drop() else {
            // Every failed drop says something now. A drag that ends in
            // nothing at all is the one outcome an author cannot act on: it
            // looks identical whether the field refused the asset, the pointer
            // was a few pixels off the row, or the whole feature is broken.
            if was_dragging {
                let message = refused.unwrap_or_else(|| {
                    format!("Released over {} — nothing was assigned", self.drop_probe)
                });
                tracing::warn!(probe = %self.drop_probe, "drag: {message}");
                self.push_toast(&message);
            } else {
                // Nothing was ever dragging. Either the press never armed, or
                // the pointer never travelled the four logical pixels that
                // promote an armed drag into a live one. Both are invisible
                // from the outside and both look exactly like a dead feature.
                tracing::debug!("drag: release with no drag in progress");
            }
            return;
        };
        match crate::drag_drop::semantic_request(
            &self.asset_db,
            &drop.payload,
            &drop.acceptance,
            self.native_ui.modifiers(),
        ) {
            Ok(request @ crate::drag_drop::DropRequest::LoadScene { .. }) => {
                self.prompt_unsaved_action(EditorEvent::CompleteDrop(request));
            }
            Ok(request) => self
                .editor_events
                .push_back(EditorEvent::CompleteDrop(request)),
            Err(reason) => self.push_toast(&reason),
        }
    }

    /// Translate the live modifier state into a selection combinator. Reading
    /// it here rather than inside `TreeView` keeps the widget ignorant of what
    /// a selection *is*, which is the same reason Details never learns about
    /// multi-selection.
    fn selection_mode(&self) -> SelectionMode {
        let modifiers = self.native_ui.modifiers();
        if modifiers.shift {
            SelectionMode::Range
        } else if modifiers.command() {
            SelectionMode::Toggle
        } else {
            SelectionMode::Replace
        }
    }

    /// Publish the whole selection to the Outliner. The primary still arrives
    /// through `update_outliner_tree`; this is the rest of the set, and it is
    /// sent every frame because it is cheap and because a stale multi-select
    /// highlight is worse than a redundant message.
    pub fn set_outliner_selection(&mut self, ids: Vec<u32>) {
        if self.outliner_selection == ids {
            return;
        }
        self.outliner_selection = ids.clone();
        self.native_ui.send(UiMessage::new(
            self.outliner_tree,
            MessageDirection::ToWidget,
            crate::widgets::tree_view::TreeViewMessage::SetSelectedSet(ids),
        ));
    }

    /// Publish the live viewport rubber-band, in logical pixels, for painting.
    pub fn set_marquee(&mut self, rect: Option<(f32, f32, f32, f32)>) {
        self.marquee_active = rect.is_some();
        self.native_ui
            .set_marquee(rect.map(|(x, y, w, h)| types::Rect::new(x, y, w, h)));
    }

    /// Whether the platform's primary shortcut modifier is held. Core reads
    /// it for the additive marquee; the UI already owns the modifier state.
    #[must_use]
    pub fn command_modifier_held(&self) -> bool {
        self.native_ui.modifiers().command()
    }

    /// Everything currently selected, in selection order.
    #[must_use]
    pub fn outliner_selection(&self) -> &[u32] {
        &self.outliner_selection
    }

    pub fn set_outliner_entity_handles(
        &mut self,
        entities: impl IntoIterator<Item = somnium_ecs::Entity>,
    ) {
        self.outliner_entity_handles = entities
            .into_iter()
            .map(|entity| (entity.index(), entity))
            .collect();
    }

    pub fn set_viewport_drop_probe(
        &mut self,
        entity: Option<somnium_ecs::Entity>,
        terrain_hit: Option<[f32; 3]>,
    ) {
        self.viewport_drop_probe = (entity, terrain_hit);
    }

    /// Raise a transient toast, and mirror it into the status text.
    ///
    /// A toast whose text reads as a failure becomes a *sticky* one, which is
    /// CONTROL-I's rule: errors persist until dismissed. Inferring it from the
    /// text rather than adding a severity argument keeps the seventy-odd
    /// existing call sites correct without touching one of them, and uses the
    /// same `LogSeverity::infer` the Output Log already trusts.
    pub fn push_toast(&mut self, text: &str) {
        let error = crate::log::LogSeverity::infer(text) == crate::log::LogSeverity::Error;
        self.native_ui.send(UiMessage::new(
            self.toast_host,
            MessageDirection::ToWidget,
            if error {
                self.sticky_toasts += 1;
                ToastMessage::PushError(text.to_string())
            } else {
                ToastMessage::Push(text.to_string())
            },
        ));
        self.native_ui
            .send(TextMessage::set_text(self.status_text, text.to_string()));
    }

    /// Dismiss the oldest sticky toast, reporting whether there was one.
    ///
    /// The boolean matters: `Esc` must fall through to the rest of the chain
    /// when there is no error to dismiss, or it would stop closing popups.
    pub fn dismiss_toast(&mut self) -> bool {
        if self.sticky_toasts == 0 {
            return false;
        }
        self.sticky_toasts -= 1;
        self.native_ui.send(UiMessage::new(
            self.toast_host,
            MessageDirection::ToWidget,
            ToastMessage::DismissOldest,
        ));
        true
    }

    /// Show the highest-priority active job and expose cancellation.
    ///
    /// Idempotent: called every frame, and does nothing at all when the job
    /// has not changed. `set_visibility` invalidates its ancestors
    /// unconditionally, so re-sending the same value re-measured the whole
    /// status bar sixty times a second.
    pub fn update_jobs(&mut self, jobs: &[UiJobStatus]) {
        let job = jobs.first();
        let next = job.map(|job| job.id);
        let changed = next != self.status_cancel_job;
        self.status_cancel_job = next;
        if changed {
            self.native_ui
                .set_visibility(self.status_cancel, job.is_some());
        }
        match job {
            Some(job) => {
                self.native_ui.send(TextMessage::set_text(
                    self.status_text,
                    format!("{} — {:.0}%", job.name, job.progress * 100.0),
                ));
            }
            // The status line borrowed by a job has to be given back, or it
            // keeps reporting work that finished.
            None if changed => {
                self.native_ui
                    .send(TextMessage::set_text(self.status_text, "Ready"));
            }
            None => {}
        }
    }

    pub fn set_scene_dirty(&mut self, dirty: bool) {
        self.scene_dirty = dirty;
        self.native_ui.send(TextMessage::set_text(
            self.status_dirty,
            if dirty { "Unsaved changes" } else { "Saved" },
        ));
        self.refresh_title();
    }

    /// The scene the title bar names. `None` is an unsaved scene.
    pub fn set_scene_name(&mut self, name: Option<String>) {
        if self.scene_name == name {
            return;
        }
        self.scene_name = name;
        self.refresh_title();
    }

    /// `Somnium Engine — scene.somnium •` — CONTROL-J's accurate title bar.
    ///
    /// The bullet is the convention every editor uses for unsaved work, and
    /// the title bar is the one place that state is visible without looking
    /// down at the status bar. Composed here rather than at the two call
    /// sites so the facts it combines cannot get out of step.
    fn refresh_title(&mut self) {
        let mut title = "Somnium Engine".to_string();
        if let Some(name) = &self.scene_name {
            title.push_str(" \u{2014} ");
            title.push_str(name);
        }
        if self.scene_dirty {
            title.push_str(" \u{2022}");
        }
        self.native_ui
            .send(TextMessage::set_text(self.title_label, title.clone()));
        self.window.set_title(&title);
    }

    /// Name of the current selection, shown in the status bar. `None` clears
    /// it to "No selection" rather than to an empty gap, so the slot does not
    /// silently vanish.
    pub fn set_status_selection(&mut self, name: Option<&str>) {
        self.native_ui.send(TextMessage::set_text(
            self.status_selection,
            name.unwrap_or("No selection"),
        ));

        // Phase 27-G. The same fact, shown where the eye already is. Hidden
        // entirely when nothing is selected rather than reading "No selection"
        // twice — an overlay that is always on stops being an overlay.
        match name {
            Some(n) => {
                self.native_ui
                    .send(TextMessage::set_text(self.vp_overlay_text, n));
                self.native_ui.set_visibility(self.vp_overlay, true);
            }
            None => self.native_ui.set_visibility(self.vp_overlay, false),
        }
    }

    /// Right-hand statistics cluster. Object count is what the editor can
    /// state honestly today; triangles and memory join it when the renderer
    /// reports them per frame (Zeta-G).
    pub fn set_status_stats(&mut self, objects: usize, fps: f64) {
        // Redline §06: items drop right to left as width runs out, and FPS
        // never drops.
        let rules = CollapseRules::for_width(self.window_size.0 as f32);
        let mut text = if rules.status_objects {
            format!("{objects} objects · {fps:.0} fps")
        } else {
            format!("{fps:.0} fps")
        };
        // Phase 16-D. Prepended, not appended: it is the item that must
        // survive the narrowest window, because a broken script is the one
        // thing in this cluster that needs acting on.
        if self.script_errors > 0 {
            let plural = if self.script_errors == 1 { "" } else { "s" };
            text = format!("{} script error{plural} · {text}", self.script_errors);
        }
        self.native_ui
            .send(TextMessage::set_text(self.status_stats, text));
    }

    /// Re-read the content folder. Called after the editor writes a file
    /// into it — a new script that does not appear in the drawer until the
    /// next unrelated refresh looks like the create failed.
    pub fn refresh_content(&mut self) {
        self.refresh_content_list();
    }

    fn begin_inline_content_rename(&mut self, entry: crate::metaphor::ContentEntry) {
        if let Some((handle, _)) = self.content_inline_rename.take() {
            self.native_ui.remove_node(handle);
        }
        let Some(tile) = self
            .content_entries
            .iter()
            .find_map(|(handle, candidate)| (candidate.path == entry.path).then_some(*handle))
        else {
            return;
        };
        let field = TextBoxBuilder::new(
            WidgetBuilder::new()
                .with_height(theme::ROW_HEIGHT)
                .with_background(theme::BG_INPUT),
        )
        .with_text(entry.name)
        .with_font_id(self.font_id)
        .build();
        let field = self.native_ui.add_node(field, tile);
        self.native_ui.set_focus(field);
        self.content_inline_rename = Some((field, entry.path));
    }

    // ── Content Drawer right-click ───────────────────────────────────────

    /// Open the drawer's context menu at `pos`, if `pos` is over it.
    ///
    /// Returns whether the click was ours. The item list depends on what
    /// was under the cursor: a file or folder gets Rename and Show in
    /// Folder, empty space does not.
    ///
    /// **Delete is deliberately absent.** Removing a file from a
    /// right-click, with no undo and no confirmation, is not a mistake
    /// anyone recovers from; the OS file browser is one menu item away
    /// and has a recycle bin.
    /// The Outliner's right-click menu, built from the registry.
    ///
    /// Right-clicking a row that is *not* in the selection selects it first,
    /// which is what every file manager does and what stops "Delete" from
    /// meaning something other than the row under the cursor.
    fn open_outliner_menu(&mut self, pos: Vec2) -> bool {
        let hit = self.native_ui.hit_test(pos);
        if !self.native_ui.is_under(hit, self.outliner_tree) {
            return false;
        }
        let bounds = self.native_ui.screen_bounds(self.outliner_tree);
        let row = ((pos.y - bounds.y) / crate::theme::TREE_ROW_HEIGHT).floor() as usize;
        if let Some((_, id)) = self.outliner_rows.get(row).copied()
            && !self.outliner_selection.contains(&id)
        {
            self.editor_events.push_back(EditorEvent::ModifySelection {
                id,
                mode: SelectionMode::Replace,
            });
        }

        let ctx = self.command_context();
        let items: Vec<MenuItem> = crate::commands::registry()
            .surface(crate::commands::CommandSurface::OutlinerContext)
            .into_iter()
            .map(|command| MenuItem {
                id: command.id.to_string(),
                label: command.menu_label(),
                enabled: command.enabled(&ctx).is_enabled(),
            })
            .collect();
        if items.is_empty() {
            return false;
        }

        let height = items.len() as f32 * theme::ROW_HEIGHT + 4.0;
        let (window_w, window_h) = (self.window_size.0 as f32, self.window_size.1 as f32);
        let mut placement = pos;
        if placement.y + height > window_h {
            placement.y = (pos.y - height).max(0.0);
        }
        const ASSUMED_WIDTH: f32 = 180.0;
        if placement.x + ASSUMED_WIDTH > window_w {
            placement.x = (window_w - ASSUMED_WIDTH).max(0.0);
        }

        self.native_ui.send(UiMessage::new(
            self.outliner_menu,
            MessageDirection::ToWidget,
            ContextMenuMessage::SetItems(items),
        ));
        self.native_ui
            .set_desired_position(self.outliner_menu, placement);
        self.native_ui.send(UiMessage::new(
            self.outliner_menu_popup,
            MessageDirection::ToWidget,
            PopupMessage::Open,
        ));
        self.native_ui
            .invalidate_ancestors(self.outliner_menu_popup);
        true
    }

    fn close_outliner_menu(&mut self) {
        self.native_ui.send(UiMessage::new(
            self.outliner_menu_popup,
            MessageDirection::ToWidget,
            PopupMessage::Close,
        ));
    }

    fn open_content_menu(&mut self, pos: Vec2) -> bool {
        if !self.drawer_open {
            return false;
        }
        let hit = self.native_ui.hit_test(pos);
        if !self.native_ui.is_under(hit, self.content_drawer) {
            return false;
        }

        // Which entry, if any, is under the cursor.
        self.content_menu_target = self
            .content_entries
            .iter()
            .find(|(handle, _)| self.native_ui.is_under(hit, *handle))
            .map(|(_, entry)| entry.clone())
            .filter(|entry| !entry.is_engine);

        // A right-click inside a folder creates in that folder. The
        // drawer's current path is the folder being *shown*, which is
        // what an author means by "here".
        self.content_menu_folder = self.content_path.clone();

        let ctx = self.command_context();
        let items: Vec<MenuItem> = crate::commands::registry()
            .surface(crate::commands::CommandSurface::ContentContext)
            .into_iter()
            .filter(|command| {
                self.content_menu_target.is_some()
                    || !matches!(
                        command.action,
                        crate::commands::CommandAction::ContentRename
                            | crate::commands::CommandAction::ContentShowInFolder
                    )
            })
            .map(|command| MenuItem {
                id: command.id.to_string(),
                label: command.menu_label(),
                enabled: command.enabled(&ctx).is_enabled(),
            })
            .collect();

        // Keep it on screen. The drawer sits at the bottom of the window,
        // which is precisely where a menu that opened downwards would
        // fall off — so a menu near the bottom flips to open upwards, the
        // way every OS menu does.
        let height = items.len() as f32 * theme::ROW_HEIGHT + 4.0;
        let (window_w, window_h) = (self.window_size.0 as f32, self.window_size.1 as f32);
        let mut placement = pos;
        if placement.y + height > window_h {
            placement.y = (pos.y - height).max(0.0);
        }
        // A conservative width: the menu measures itself from its longest
        // label, and over-reserving here only moves it left a little
        // sooner than it strictly had to.
        const ASSUMED_WIDTH: f32 = 180.0;
        if placement.x + ASSUMED_WIDTH > window_w {
            placement.x = (window_w - ASSUMED_WIDTH).max(0.0);
        }

        self.native_ui.send(UiMessage::new(
            self.content_menu,
            MessageDirection::ToWidget,
            ContextMenuMessage::SetItems(items),
        ));
        // `AnchorBelow` with no anchor honours the child's desired
        // position, which is how the menu lands under the cursor.
        self.native_ui
            .set_desired_position(self.content_menu, placement);
        self.native_ui.send(UiMessage::new(
            self.content_menu_popup,
            MessageDirection::ToWidget,
            PopupMessage::Open,
        ));
        self.native_ui.invalidate_ancestors(self.content_menu_popup);
        true
    }

    fn close_content_menu(&mut self) {
        self.native_ui.send(UiMessage::new(
            self.content_menu_popup,
            MessageDirection::ToWidget,
            PopupMessage::Close,
        ));
    }

    /// Show the name prompt for one of the three flows.
    fn open_name_prompt(&mut self, prompt: NamePrompt, title: &str, initial: &str) {
        self.name_prompt = Some(prompt);
        self.name_text = initial.to_string();
        self.native_ui
            .send(TextMessage::set_text(self.name_title, title.to_string()));
        self.native_ui
            .send(TextMessage::set_text(self.name_input, initial.to_string()));
        self.native_ui.send(UiMessage::new(
            self.name_popup,
            MessageDirection::ToWidget,
            PopupMessage::Open,
        ));
        self.native_ui.invalidate_ancestors(self.name_popup);
        // So the author can start typing without clicking the box first.
        self.enter_modal_focus(self.name_popup, self.name_input);
    }

    fn close_name_prompt(&mut self) {
        self.name_prompt = None;
        self.exit_modal_focus(self.name_popup);
        self.native_ui.send(UiMessage::new(
            self.name_popup,
            MessageDirection::ToWidget,
            PopupMessage::Close,
        ));
    }

    /// Turn a confirmed name prompt into an editor event.
    ///
    /// A blank name is treated as a cancel rather than an error: it is
    /// what someone who changed their mind does, and a modal that refuses
    /// to close is worse than one that quietly gives up.
    /// Start an in-place rename of the primary selection.
    fn begin_outliner_rename(&mut self) {
        let Some((_, Some(entity))) = self
            .last_outliner_state
            .as_ref()
            .map(|(rows, selected)| (rows, *selected))
        else {
            return;
        };
        let current = self
            .last_outliner_state
            .as_ref()
            .and_then(|(rows, _)| rows.iter().find(|row| row.id == entity))
            .map(|row| row.name.clone())
            .unwrap_or_default();
        self.open_name_prompt(NamePrompt::RenameEntity { entity }, "Rename", &current);
    }

    fn confirm_name_prompt(&mut self) {
        let Some(prompt) = self.name_prompt.take() else {
            return;
        };
        let name = self.name_text.trim().to_string();
        self.close_name_prompt();
        if name.is_empty() {
            return;
        }
        let event = match prompt {
            NamePrompt::NewFolder { parent } => EditorEvent::CreateContentFolder { parent, name },
            NamePrompt::NewScript { parent } => EditorEvent::CreateContentScript { parent, name },
            NamePrompt::NewMaterial { parent } => {
                EditorEvent::CreateContentMaterial { parent, name }
            }
            NamePrompt::RenameEntity { entity } => EditorEvent::RenameEntity { entity, name },
        };
        self.editor_events.push_back(event);
    }

    /// Act on a chosen menu item.
    fn activate_content_menu(&mut self, id: &str) {
        // The context-menu row disappears on close. Restore the underlying
        // tile (or the drawer search after a gap click) first, so a command
        // which opens the name modal has a durable return-focus target.
        let return_focus = self
            .content_menu_target
            .as_ref()
            .and_then(|entry| {
                self.content_entries
                    .iter()
                    .find(|(_, candidate)| candidate.path == entry.path)
                    .map(|(handle, _)| *handle)
            })
            .unwrap_or(self.content_search);
        self.native_ui.set_focus(return_focus);
        self.native_ui.send(UiMessage::new(
            return_focus,
            MessageDirection::ToWidget,
            WidgetMessage::Focus,
        ));
        self.close_content_menu();
        let _ = self.run_command_id(id);
    }

    /// Phase 16-D: how many blocking script diagnostics are outstanding.
    ///
    /// Accumulates rather than replaces, because diagnostics arrive a
    /// batch at a time and the status area is reporting the session, not
    /// the last batch. [`Self::clear_script_errors`] is the reset.
    pub fn set_script_error_count(&mut self, errors: usize) {
        self.script_errors += errors;
    }

    /// Reset the script error count — on a successful reload, or when the
    /// author clears the Output Log.
    pub fn clear_script_errors(&mut self) {
        self.script_errors = 0;
    }

    pub fn prompt_unsaved_new(&mut self) {
        self.prompt_unsaved_action(EditorEvent::NewScene);
    }

    fn prompt_unsaved_action(&mut self, action: EditorEvent) {
        if !self.scene_dirty {
            self.editor_events.push_back(action);
            return;
        }
        self.pending_scene_action = Some(action);
        self.unsaved_open = true;
        self.native_ui.send(UiMessage::new(
            self.unsaved_popup,
            MessageDirection::ToWidget,
            PopupMessage::Open,
        ));
        self.native_ui.invalidate_ancestors(self.unsaved_popup);
        self.enter_modal_focus(self.unsaved_popup, self.unsaved_save);
    }

    pub fn set_fps(&mut self, fps: f64) {
        self.native_ui.send(TextMessage::set_text(
            self.fps_text,
            format!("{fps:.0} fps"),
        ));
    }

    pub fn set_play_overlays_hidden(&mut self, hidden: bool) {
        if hidden {
            if self.drawer_open || self.log_open || self.references_open || self.locale_open {
                self.drawer_open = false;
                self.log_open = false;
                self.references_open = false;
                self.locale_open = false;
                self.apply_bottom_panel();
            }
            if self.help_open {
                self.help_open = false;
                self.native_ui.send(UiMessage::new(
                    self.help_overlay,
                    MessageDirection::ToWidget,
                    PopupMessage::Close,
                ));
            }
            self.close_all_menus();
        }
    }

    fn toggle_help(&mut self, page: Option<u8>) {
        if let Some(p) = page {
            self.help_page = p;
            self.help_open = true;
        } else {
            self.help_open = !self.help_open;
        }
        self.native_ui.send(UiMessage::new(
            self.help_overlay,
            MessageDirection::ToWidget,
            if self.help_open {
                PopupMessage::Open
            } else {
                PopupMessage::Close
            },
        ));
        if self.help_open {
            self.set_help_page(self.help_page);
        }
        self.native_ui.invalidate_ancestors(self.help_overlay);
    }

    fn set_help_page(&mut self, page: u8) {
        self.help_page = page;
        fill_help_body(&mut self.native_ui, self.help_body, self.font_id, page);
        let toc = self.help_toc.clone();
        for (handle, id) in toc {
            self.native_ui
                .send(ButtonMessage::set_selected(handle, id == page));
        }
        self.native_ui.invalidate_ancestors(self.help_body);
    }

    fn toggle_drawer(&mut self) {
        if self.drawer_open {
            self.drawer_open = false;
        } else {
            self.drawer_open = true;
            self.log_open = false;
            self.references_open = false;
            self.locale_open = false;
        }
        self.apply_bottom_panel();
    }

    fn toggle_log_panel(&mut self) {
        if self.log_open {
            self.log_open = false;
        } else {
            self.log_open = true;
            self.drawer_open = false;
            self.references_open = false;
            self.locale_open = false;
        }
        self.apply_bottom_panel();
    }

    /// MORROWIND-M item 3. Opens on whatever the panel was last pointed at,
    /// which for a first press is nothing and says so.
    fn toggle_references_panel(&mut self) {
        if self.references_open {
            self.references_open = false;
        } else {
            self.references_open = true;
            self.drawer_open = false;
            self.log_open = false;
            self.locale_open = false;
            self.refresh_references_panel();
        }
        self.apply_bottom_panel();
    }

    /// MORROWIND-M item 2. The localisation table, which is empty until a
    /// project with a `locale/` directory has been loaded.
    fn toggle_locale_panel(&mut self) {
        if self.locale_open {
            self.locale_open = false;
        } else {
            self.locale_open = true;
            self.drawer_open = false;
            self.log_open = false;
            self.references_open = false;
        }
        self.apply_bottom_panel();
    }

    fn apply_bottom_panel(&mut self) {
        let show = self.drawer_open || self.log_open || self.references_open || self.locale_open;
        self.native_ui
            .set_visibility(self.content_drawer, self.drawer_open);
        self.native_ui.set_visibility(self.log_panel, self.log_open);
        self.native_ui
            .set_visibility(self.references_panel, self.references_open);
        self.native_ui
            .set_visibility(self.locale_panel, self.locale_open);
        // A hidden widget must not keep the keyboard. The grid holds it while a
        // cell is chosen, which is right while you can see the cell and is the
        // fly-cam silently not responding once the panel is closed over it.
        if !self.locale_open && self.native_ui.focused() == self.locale_grid {
            self.native_ui.release_keyboard();
        }
        self.native_ui.send(UiMessage::new(
            self.outer_grid,
            MessageDirection::ToWidget,
            GridMessage::SetRowSize(5, if show { self.drawer_height } else { 0.0 }),
        ));
        // Naming the open panel next to the button that opened it is a
        // duplicate, so the slot only speaks when the drawer row is closed.
        self.native_ui.send(TextMessage::set_text(
            self.status_text,
            if show { "" } else { "Ready" },
        ));
        self.native_ui.invalidate_ancestors(self.outer_grid);
    }

    fn combo_entries(&self) -> [(NodeHandle, NodeHandle); 4] {
        [
            (self.foliage_kind_combo, self.foliage_kind_popup),
            (self.viewport_res_combo, self.viewport_res_popup),
            (self.snap_grid_combo, self.snap_grid_popup),
            (self.snap_angle_combo, self.snap_angle_popup),
        ]
    }

    fn close_combo_dropdowns(&mut self) {
        if self.open_combo_popup.is_none() {
            return;
        }
        for (combo, popup) in self.combo_entries() {
            self.native_ui.send(UiMessage::new(
                popup,
                MessageDirection::ToWidget,
                PopupMessage::Close,
            ));
            self.native_ui.send(ComboBoxMessage::close(combo));
            self.native_ui.invalidate_ancestors(popup);
        }
        self.open_combo_popup = NodeHandle::NONE;
    }

    fn combo_popup_for(&self, combo: NodeHandle) -> Option<NodeHandle> {
        self.combo_entries()
            .into_iter()
            .find(|(c, _)| *c == combo)
            .map(|(_, p)| p)
    }

    fn combo_for_popup(&self, popup: NodeHandle) -> Option<NodeHandle> {
        self.combo_entries()
            .into_iter()
            .find(|(_, p)| *p == popup)
            .map(|(c, _)| c)
    }

    /// Close exactly one layer.
    ///
    /// Phase 26-Zeta-H. The order is the design package's:
    /// **modal → palette → popup → drawer → filter → selection.** One Esc must
    /// never dismiss two things, and the modal goes first because it is the
    /// only layer that traps focus — leaving it open while something below it
    /// closes would strand the keyboard.
    ///
    /// Returns whether anything was closed, so the caller can let the key fall
    /// through to the viewport when the editor has nothing to dismiss.
    fn close_top_overlay(&mut self) -> bool {
        // Preferences is modal and sits above the name prompt, because it can
        // open one (the project picker) but never the other way round.
        if self.preferences_open {
            self.toggle_preferences();
            return true;
        }
        // Modal name prompts share the same focus trap as the unsaved prompt.
        if self.name_prompt.is_some() {
            self.close_name_prompt();
            return true;
        }
        // 6 — modal
        if self.unsaved_open {
            self.close_unsaved();
            return true;
        }
        // 5 — command palette
        if self.palette_open {
            self.close_palette();
            return true;
        }
        // 3 — popups: colour, combo lists, menus, then the Help overlay
        if self.color_open {
            self.close_color_picker(false);
            return true;
        }
        if self.open_combo_popup.is_some() {
            self.close_combo_dropdowns();
            return true;
        }
        if self.file_popup_open
            || self.create_popup_open
            || self.edit_popup_open
            || self.view_popup_open
            || self.window_popup_open
            || self.help_menu_open
        {
            self.close_all_menus();
            return true;
        }
        if self.help_open {
            self.help_open = false;
            self.native_ui.send(UiMessage::new(
                self.help_overlay,
                MessageDirection::ToWidget,
                PopupMessage::Close,
            ));
            return true;
        }
        // 1 — drawer. Only an *undocked* drawer dismisses on Esc; a docked one
        // is layout, and phase_26 §13.2 fixed that it does not close on
        // click-away either.
        if !self.drawer_docked && (self.drawer_open || self.log_open) {
            self.drawer_open = false;
            self.log_open = false;
            self.apply_bottom_panel();
            return true;
        }
        // filter — a live search is a mode, and Esc is how you leave a mode
        if self.clear_active_filter() {
            return true;
        }
        // selection — the last thing Esc gives back
        if self
            .last_outliner_state
            .as_ref()
            .is_some_and(|(_, selected)| selected.is_some())
        {
            self.editor_events
                .push_back(EditorEvent::SelectEntity(None));
            return true;
        }
        false
    }

    /// Clear whichever search field currently has text. Returns whether one
    /// did.
    fn clear_active_filter(&mut self) -> bool {
        for (handle, filter) in [
            (self.inspector_search, &mut self.inspector_filter),
            (self.outliner_search, &mut self.outliner_filter),
        ] {
            if !filter.is_empty() {
                filter.clear();
                self.native_ui.send(UiMessage::new(
                    handle,
                    MessageDirection::ToWidget,
                    SearchBoxMessage::SetText(String::new()),
                ));
                return true;
            }
        }
        false
    }

    /// The shell's Tab stops, in the order the design annotation specifies:
    /// application scope → mode scope → viewport context → left rail →
    /// viewport → Outliner → Details → drawer → status.
    fn focus_stops(&self) -> Vec<NodeHandle> {
        let mut stops = if self.name_prompt.is_some() {
            vec![self.name_input, self.name_ok, self.name_cancel]
        } else if self.unsaved_open {
            vec![self.unsaved_save, self.unsaved_discard, self.unsaved_cancel]
        } else {
            vec![
                self.palette_button,
                self.save_button,
                self.select_button,
                self.landscape_button,
                self.foliage_toolbar_button,
                self.camera_speed_slider,
                self.outliner_search,
                self.outliner_tree,
                self.inspector_search,
            ]
        };
        if self.native_ui.modal_root().is_none() && self.drawer_open {
            stops.push(self.content_search);
        }
        if self.native_ui.modal_root().is_none() {
            stops.push(self.drawer_button);
        }
        stops.retain(|h| !h.is_none() && self.native_ui.is_globally_visible(*h));
        stops
    }

    /// Move keyboard focus one stop forward (`+1`) or back (`-1`), wrapping.
    fn advance_focus(&mut self, delta: i32) {
        let stops = self.focus_stops();
        if stops.is_empty() {
            return;
        }
        let current = self.native_ui.focused();
        let next = match stops.iter().position(|h| *h == current) {
            Some(i) => {
                let n = stops.len() as i32;
                (((i as i32 + delta) % n + n) % n) as usize
            }
            // Nothing in the ring has focus — enter at the front or the back
            // depending on direction, so Shift+Tab from cold lands on the last
            // stop rather than the first.
            None if delta >= 0 => 0,
            None => stops.len() - 1,
        };
        let target = stops[next];
        let previous = self.native_ui.focused();
        if !previous.is_none() {
            self.native_ui.send(UiMessage::new(
                previous,
                MessageDirection::ToWidget,
                WidgetMessage::Unfocus,
            ));
        }
        self.native_ui.set_focus(target);
        self.native_ui.send(UiMessage::new(
            target,
            MessageDirection::ToWidget,
            WidgetMessage::Focus,
        ));
    }

    fn close_all_menus(&mut self) {
        for (flag, handle) in [
            (&mut self.file_popup_open, self.file_popup),
            (&mut self.create_popup_open, self.create_popup),
            (&mut self.edit_popup_open, self.edit_popup),
            (&mut self.view_popup_open, self.view_popup),
            (&mut self.window_popup_open, self.window_popup),
            (&mut self.help_menu_open, self.help_menu_popup),
        ] {
            *flag = false;
            self.native_ui.send(UiMessage::new(
                handle,
                MessageDirection::ToWidget,
                PopupMessage::Close,
            ));
        }
    }

    fn is_title_chrome_hit(&self, hit: NodeHandle) -> bool {
        hit == self.title_drag || self.native_ui.is_under(hit, self.title_drag)
    }

    fn transient_overlay_open(&self) -> bool {
        self.file_popup_open
            || self.create_popup_open
            || self.edit_popup_open
            || self.view_popup_open
            || self.window_popup_open
            || self.help_menu_open
            || self.help_open
            || self.palette_open
            || self.color_open
            || self.unsaved_open
            || self.name_prompt.is_some()
            || self.open_combo_popup.is_some()
    }

    fn open_transient_popup(&self) -> Option<NodeHandle> {
        if self.file_popup_open {
            Some(self.file_popup)
        } else if self.create_popup_open {
            Some(self.create_popup)
        } else if self.edit_popup_open {
            Some(self.edit_popup)
        } else if self.view_popup_open {
            Some(self.view_popup)
        } else if self.window_popup_open {
            Some(self.window_popup)
        } else if self.help_menu_open {
            Some(self.help_menu_popup)
        } else if self.help_open {
            Some(self.help_overlay)
        } else if self.palette_open {
            Some(self.palette_popup)
        } else if self.color_open {
            Some(self.color_popup)
        } else if self.name_prompt.is_some() {
            Some(self.name_popup)
        } else if self.unsaved_open {
            Some(self.unsaved_popup)
        } else if self.open_combo_popup.is_some() {
            Some(self.open_combo_popup)
        } else {
            None
        }
    }

    fn hit_is_inside_transient_content(&self, hit: NodeHandle) -> bool {
        let Some(popup) = self.open_transient_popup() else {
            return false;
        };
        let content = self.native_ui.first_child(popup);
        self.native_ui.is_under(hit, content) || hit == content
    }

    fn menu_button_for(&self, hit: NodeHandle) -> Option<NodeHandle> {
        for btn in [
            self.file_button,
            self.create_button,
            self.edit_button,
            self.view_button,
            self.window_button,
            self.help_menu_button,
        ] {
            if hit == btn || self.native_ui.is_under(hit, btn) {
                return Some(btn);
            }
        }
        None
    }

    fn open_menu_button(&self) -> Option<NodeHandle> {
        if self.file_popup_open {
            Some(self.file_button)
        } else if self.create_popup_open {
            Some(self.create_button)
        } else if self.edit_popup_open {
            Some(self.edit_button)
        } else if self.view_popup_open {
            Some(self.view_button)
        } else if self.window_popup_open {
            Some(self.window_button)
        } else if self.help_menu_open {
            Some(self.help_menu_button)
        } else {
            None
        }
    }

    fn toggle_palette(&mut self) {
        if self.palette_open {
            self.close_palette();
            return;
        }
        self.close_all_menus();
        self.palette_open = true;
        // Entities and assets change while the palette is closed, so the set is
        // rebuilt on open rather than kept in sync from a dozen call sites.
        self.refresh_palette_items();
        self.native_ui.send(UiMessage::new(
            self.palette_widget,
            MessageDirection::ToWidget,
            CommandPaletteMessage::SetQuery(String::new()),
        ));
        self.native_ui.send(UiMessage::new(
            self.palette_popup,
            MessageDirection::ToWidget,
            PopupMessage::Open,
        ));
        self.enter_modal_focus(self.palette_popup, self.palette_widget);
        self.native_ui.invalidate_ancestors(self.palette_popup);
    }

    fn close_palette(&mut self) {
        self.palette_open = false;
        self.exit_modal_focus(self.palette_popup);
        self.native_ui.send(UiMessage::new(
            self.palette_popup,
            MessageDirection::ToWidget,
            PopupMessage::Close,
        ));
        self.native_ui.invalidate_ancestors(self.palette_popup);
    }

    /// Rebuild the searchable set from the command registry and current
    /// dynamic editor objects. Every row has a stable id.
    fn refresh_palette_items(&mut self) {
        use crate::widgets::command_palette::PaletteCategory as Cat;

        let ctx = self.command_context();
        let mut items: Vec<PaletteItem> = crate::commands::registry()
            .commands()
            .iter()
            .map(|command| {
                let enablement = command.enabled(&ctx);
                PaletteItem::command(
                    command.id,
                    command.label,
                    command
                        .default_binding
                        .map(|binding| binding.to_string())
                        .unwrap_or_default(),
                )
                .with_enablement(enablement.is_enabled(), enablement.reason())
                .with_recency(self.command_history.recency(command.id))
            })
            .collect();
        let mut targets = std::collections::HashMap::new();

        // Entities, from whatever the Outliner is currently showing.
        for (id, name) in &self.palette_entities {
            let key = format!("entity:{id}");
            items.push(
                PaletteItem::new(&key, name.clone(), "Select", Cat::Entity)
                    .with_recency(self.command_history.recency(&key)),
            );
            targets.insert(key, PaletteTarget::Entity(*id));
        }

        // Assets in the current Content Drawer folder.
        for (_, entry) in &self.content_entries {
            if entry.is_dir {
                continue;
            }
            let key = format!("asset:{}", entry.path.to_string_lossy());
            items.push(
                PaletteItem::new(&key, entry.name.clone(), "Open", Cat::Asset)
                    .with_recency(self.command_history.recency(&key)),
            );
            targets.insert(key, PaletteTarget::Asset(entry.path.clone()));
        }

        // Help pages, so a question is answerable from the same surface.
        for (i, title) in crate::metaphor::help_titles().iter().enumerate() {
            let key = format!("help:page:{i}");
            items.push(
                PaletteItem::new(&key, (*title).to_string(), "F1", Cat::Help)
                    .with_recency(self.command_history.recency(&key)),
            );
            targets.insert(key, PaletteTarget::Help(i as u8));
        }

        // Panels, so the palette can also be used to reach a surface.
        for (key, label, target) in [
            ("panel:content", "Content Drawer", PaletteTarget::Drawer),
            ("panel:log", "Output Log", PaletteTarget::Log),
        ] {
            items.push(
                PaletteItem::new(key, label, "", Cat::Panel)
                    .with_recency(self.command_history.recency(key)),
            );
            targets.insert(key.to_string(), target);
        }

        self.palette_targets = targets;
        self.native_ui.send(UiMessage::new(
            self.palette_widget,
            MessageDirection::ToWidget,
            CommandPaletteMessage::SetItems(items),
        ));
    }

    /// Open or close the Preferences window.
    pub fn toggle_preferences(&mut self) {
        self.preferences_open = !self.preferences_open;
        self.rebinding = None;
        self.native_ui.send(UiMessage::new(
            self.preferences.overlay,
            MessageDirection::ToWidget,
            if self.preferences_open {
                PopupMessage::Open
            } else {
                PopupMessage::Close
            },
        ));
        if self.preferences_open {
            self.enter_modal_focus(self.preferences.overlay, self.preferences.search);
            self.rebuild_binding_rows();
        } else {
            self.exit_modal_focus(self.preferences.overlay);
        }
        self.native_ui
            .invalidate_ancestors(self.preferences.overlay);
    }

    /// Whether the Preferences window is showing.
    #[must_use]
    pub fn preferences_open(&self) -> bool {
        self.preferences_open
    }

    /// The settings panels core should publish, filtered by the search box.
    ///
    /// Filtering here rather than in core keeps the query where the box is,
    /// and reuses the generated search that already covers label, field name
    /// and doc comment — Unity's generated keyword index, not a maintained one.
    pub fn update_settings_panels(
        &mut self,
        panels: Vec<crate::editor::inspector_gen::GeneratedComponentPanel>,
        overridden: &[(
            somnium_ecs::reflect::StableId,
            somnium_ecs::reflect::FieldId,
            String,
        )],
    ) {
        let filtered: Vec<_> = panels
            .into_iter()
            .map(|mut panel| {
                if !self.preferences_query.trim().is_empty() {
                    let kept: Vec<_> = crate::editor::inspector_gen::search_rows(
                        &panel.rows,
                        &self.preferences_query,
                    )
                    .into_iter()
                    .cloned()
                    .collect();
                    panel.rows = kept;
                }
                if self.preferences_modified_only {
                    panel.rows.retain(|row| row.modified);
                }
                // An overridden setting is shown, disabled, with the variable
                // named — craft defect C8. Hiding it would be worse: the value
                // on screen would then be unexplained.
                for row in &mut panel.rows {
                    if overridden.iter().any(|(component, field, _)| {
                        *component == row.component && *field == row.field
                    }) {
                        row.read_only = true;
                    }
                }
                panel
            })
            .filter(|panel| !panel.rows.is_empty())
            .collect();

        let signature: Vec<_> = filtered
            .iter()
            .flat_map(|panel| panel.rows.iter())
            .map(|row| (row.component, row.field, GeneratedEdit::Whole))
            .collect();
        if signature != self.settings_signature {
            if self.settings_root.is_some() {
                self.native_ui.remove_node(self.settings_root);
            }
            self.settings_bindings.clear();
            self.settings_rows.clear();
            let (root, bindings, rows, _, _, _, _) = build_generated_details(
                &mut self.native_ui,
                self.preferences.settings_body,
                self.font_id,
                &filtered,
                &self.asset_db,
            );
            self.settings_root = root;
            self.settings_bindings = bindings;
            self.settings_rows = rows;
            self.settings_signature = signature;
            self.native_ui
                .invalidate_ancestors(self.preferences.settings_body);
        }
        self.settings_panels = filtered;
    }

    /// Rebuild the keyboard tab from the live bindings.
    fn rebuild_binding_rows(&mut self) {
        for (_, capture, _) in std::mem::take(&mut self.binding_rows) {
            let row = self.native_ui.parent(capture);
            if row.is_some() {
                self.native_ui.remove_node(row);
            }
        }
        let query = self.preferences_query.to_ascii_lowercase();
        let rows = self.keybindings.rows();
        for row in rows {
            let Some(command) = crate::commands::registry().get(row.command).copied() else {
                continue;
            };
            if !query.is_empty()
                && !command.label.to_ascii_lowercase().contains(&query)
                && !command.id.contains(&query)
            {
                continue;
            }
            if self.preferences_modified_only && !row.customised {
                continue;
            }
            let chord = row
                .chord
                .map_or_else(|| "—".to_string(), |chord| chord.to_string());
            let (capture, reset) = crate::editor::preferences::build_binding_row(
                &mut self.native_ui,
                self.preferences.bindings_body,
                self.font_id,
                command.label,
                &chord,
                row.conflicted,
                row.customised,
            );
            self.binding_rows.push((row.command, capture, reset));
        }
        self.native_ui
            .invalidate_ancestors(self.preferences.bindings_body);
    }

    /// A keystroke arrived while a binding row was waiting for one.
    ///
    /// A conflict is reported and the rebind still applies: the person just
    /// asked for it, and silently refusing would leave them pressing a key
    /// that does nothing with no explanation. What is *not* done is unbinding
    /// the other command behind their back — the conflict stays visible in the
    /// list until they resolve it.
    fn capture_rebind(&mut self, chord: crate::commands::Chord) {
        let Some(id) = self.rebinding.take() else {
            return;
        };
        let conflicts = self.keybindings.conflicts_for(chord, id);
        self.keybindings.bind(id, Some(chord));
        self.keybindings.save();
        if let Some(other) = conflicts
            .first()
            .and_then(|other| crate::commands::registry().get(other).map(|c| c.label))
        {
            self.push_toast(&format!("{chord} also runs {other}"));
        }
        self.rebuild_binding_rows();
    }

    /// Recently opened scenes, newest first, each with whether it still
    /// exists. A missing entry is greyed rather than dropped — craft defect
    /// C11: silently forgetting looks the same as never having opened it.
    pub fn set_recent_scenes(&mut self, scenes: Vec<(String, bool)>) {
        if self.recent_scenes == scenes {
            return;
        }
        self.recent_scenes = scenes;
        self.rebuild_file_menu();
    }

    /// Rebuild the recent-scenes tail of the File menu.
    ///
    /// The registry half of the menu is built once and never changes; this
    /// appends and re-appends only the recent entries, so a command added to
    /// `Menu::File` still arrives without touching this code.
    fn rebuild_file_menu(&mut self) {
        for (handle, _) in std::mem::take(&mut self.recent_menu_items) {
            self.native_ui.remove_node(handle);
        }
        if self.recent_scenes.is_empty() {
            return;
        }
        let separator =
            crate::editor::parts::scope_separator(&mut self.native_ui, self.file_menu_stack);
        self.recent_menu_items.push((separator, String::new()));
        let recents = self.recent_scenes.clone();
        for (path, exists) in recents {
            let label = std::path::Path::new(&path)
                .file_name()
                .map_or_else(|| path.clone(), |name| name.to_string_lossy().into_owned());
            let handle = crate::editor::parts::menu_entry(
                &mut self.native_ui,
                self.file_menu_stack,
                self.font_id,
                &label,
                &path,
                exists,
            );
            self.recent_menu_items.push((handle, path));
        }
        self.native_ui.invalidate_ancestors(self.file_menu_stack);
    }

    /// How long the pointer rests before a tooltip appears.
    pub fn set_tooltip_delay_ms(&mut self, ms: f32) {
        self.tooltip_delay_ms = ms.max(0.0);
    }

    /// Publish the viewport statistics overlay. `None` hides it, which is what
    /// the `show_statistics` preference being off means.
    pub fn set_viewport_statistics(&mut self, stats: Option<crate::debug::ViewportStats>) {
        let area = stats.map(|stats| {
            (
                self.native_ui.screen_bounds(self.viewport_handle),
                stats.lines(),
            )
        });
        self.native_ui.set_statistics(area);
    }

    /// Publish the snapping and gizmo state to the viewport context bar.
    ///
    /// Also applies Unreal 5.6's overflow rule: below the collapse width the
    /// whole cluster is hidden behind a chevron rather than clipped, because a
    /// half-drawn control is worse than one you have to open.
    pub fn set_snap_state(
        &mut self,
        translate_m: f32,
        rotate_deg: f32,
        snap_to_surface: bool,
        local_space: bool,
        select_only: bool,
    ) {
        let rules = CollapseRules::for_width(self.window_size.0 as f32);
        self.native_ui
            .set_visibility(self.snap_cluster, rules.context_bar_snap_inline);
        self.native_ui
            .set_visibility(self.snap_overflow, !rules.context_bar_snap_inline);

        let grid = nearest_index(&SNAP_GRID_VALUES, translate_m);
        let angle = nearest_index(&SNAP_ANGLE_VALUES, rotate_deg);
        self.native_ui
            .send(ComboBoxMessage::set_selected(self.snap_grid_combo, grid));
        self.native_ui
            .send(ComboBoxMessage::set_selected(self.snap_angle_combo, angle));
        self.native_ui.send(ButtonMessage::set_selected(
            self.snap_surface_toggle,
            snap_to_surface,
        ));
        self.native_ui.send(ButtonMessage::set_selected(
            self.gizmo_space_toggle,
            local_space,
        ));
        self.native_ui.send(TextMessage::set_text(
            self.gizmo_space_label,
            if local_space { "Local" } else { "World" }.to_string(),
        ));
        self.native_ui.send(ButtonMessage::set_selected(
            self.select_only_toggle,
            select_only,
        ));
    }

    /// Publish the corner axis widget. `axes` are the world X/Y/Z directions
    /// projected into screen space, y already flipped.
    pub fn set_axis_widget(&mut self, axes: [(f32, f32); 3]) {
        let viewport = self.native_ui.screen_bounds(self.viewport_handle);
        self.native_ui
            .set_axis_widget(viewport, axes.map(|(x, y)| Vec2::new(x, y)));
    }

    /// Which debug visualisation is active, so the View menu can mark it and
    /// the status bar can say so. `"lit"` means the ordinary image.
    pub fn set_active_debug_view(&mut self, id: &'static str) {
        self.active_debug_view = id;
    }

    /// The active debug visualisation.
    #[must_use]
    pub fn active_debug_view(&self) -> &'static str {
        self.active_debug_view
    }

    /// Mirror of the renderer's pipeline switches, so the View menu can show
    /// each one checked, and disabled with its variable named when the
    /// environment owns it.
    pub fn set_render_toggles(&mut self, toggles: crate::debug::DebugToggles) {
        self.render_toggles = toggles;
    }

    /// The live pipeline switches.
    #[must_use]
    pub fn render_toggles(&self) -> &crate::debug::DebugToggles {
        &self.render_toggles
    }

    /// Show every selectable entity under the cursor — Unity 6's piercing
    /// menu, and craft defect C9.
    ///
    /// Reuses the Outliner's context-menu popup: the rows are entity names
    /// and the ids are entity indices, so nothing new has to learn how a
    /// menu is placed or dismissed.
    pub fn open_piercing_menu(&mut self, rows: Vec<(u32, String)>) {
        self.piercing_rows = rows;
        if self.piercing_rows.is_empty() {
            self.push_toast("Nothing under the cursor");
            return;
        }
        let items: Vec<MenuItem> = self
            .piercing_rows
            .iter()
            .map(|(index, name)| MenuItem {
                id: format!("pierce:{index}"),
                label: name.clone(),
                enabled: true,
            })
            .collect();
        let pos = self.native_ui.cursor_pos;
        self.native_ui.send(UiMessage::new(
            self.outliner_menu,
            MessageDirection::ToWidget,
            ContextMenuMessage::SetItems(items),
        ));
        self.native_ui.set_desired_position(self.outliner_menu, pos);
        self.native_ui.send(UiMessage::new(
            self.outliner_menu_popup,
            MessageDirection::ToWidget,
            PopupMessage::Open,
        ));
        self.native_ui
            .invalidate_ancestors(self.outliner_menu_popup);
    }

    /// The chord in force for a command, for menus and tooltips.
    #[must_use]
    pub fn chord_for(&self, id: &str) -> Option<crate::commands::Chord> {
        let default = crate::commands::registry()
            .get(id)
            .and_then(|command| command.default_binding);
        self.keybindings.chord_for(id, default)
    }

    /// Everything the Preferences window does with a message.
    ///
    /// One function rather than four arms scattered through the main loop,
    /// because every one of them is answered the same way — publish a
    /// `SetSetting` and let core decide whether the environment has taken the
    /// field away — and because the window is either open or it is not.
    /// The viewport context bar's snap cluster.
    ///
    /// Every one of these writes a Seam-4 setting rather than an ad-hoc field,
    /// which is what makes them survive a restart and appear in Preferences
    /// without a second implementation.
    fn handle_snap_message(&mut self, msg: &UiMessage) -> bool {
        if let Some(ComboBoxMessage::SelectionChanged(index)) = msg.data::<ComboBoxMessage>() {
            if msg.destination == self.snap_grid_combo {
                self.push_setting(
                    "somnium.EditorSettings",
                    "snap_translate_m",
                    somnium_ecs::reflect::ReflectValue::F64(f64::from(
                        SNAP_GRID_VALUES.get(*index).copied().unwrap_or(0.0),
                    )),
                );
                return true;
            }
            if msg.destination == self.snap_angle_combo {
                self.push_setting(
                    "somnium.EditorSettings",
                    "snap_rotate_deg",
                    somnium_ecs::reflect::ReflectValue::F64(f64::from(
                        SNAP_ANGLE_VALUES.get(*index).copied().unwrap_or(0.0),
                    )),
                );
                return true;
            }
        }
        if !matches!(msg.data::<ButtonMessage>(), Some(ButtonMessage::Click)) {
            return false;
        }
        if msg.destination == self.snap_overflow {
            // The overflow opens Preferences at the Settings tab rather than
            // duplicating the cluster in a popup: one place these live.
            if !self.preferences_open {
                self.toggle_preferences();
            }
            self.preferences_query = "snap".into();
            // The query has to reach the box too, or Preferences opens
            // filtered with an empty-looking search field and reads as broken.
            self.native_ui.send(UiMessage::new(
                self.preferences.search,
                MessageDirection::ToWidget,
                crate::widgets::search_box::SearchBoxMessage::SetText("snap".into()),
            ));
            self.settings_signature.clear();
            return true;
        }
        for (handle, component, field) in [
            (
                self.snap_surface_toggle,
                "somnium.EditorSettings",
                "snap_to_surface",
            ),
            (
                self.gizmo_space_toggle,
                "somnium.EditorSettings",
                "gizmo_local_space",
            ),
            (
                self.select_only_toggle,
                "somnium.EditorSettings",
                "select_only",
            ),
        ] {
            if msg.destination == handle {
                self.editor_events.push_back(EditorEvent::ToggleSetting {
                    component: somnium_ecs::reflect::StableId::new(component),
                    field_name: field,
                });
                return true;
            }
        }
        false
    }

    fn push_setting(
        &mut self,
        component: &'static str,
        field_name: &'static str,
        value: somnium_ecs::reflect::ReflectValue,
    ) {
        self.editor_events.push_back(EditorEvent::SetSettingByName {
            component: somnium_ecs::reflect::StableId::new(component),
            field_name,
            value,
        });
    }

    fn handle_preferences_message(&mut self, msg: &UiMessage) -> bool {
        if matches!(msg.data::<ButtonMessage>(), Some(ButtonMessage::Click)) {
            if msg.destination == self.preferences.close {
                self.toggle_preferences();
                return true;
            }
            if msg.destination == self.preferences.tab_settings
                || msg.destination == self.preferences.tab_bindings
            {
                self.preferences_bindings_tab = msg.destination == self.preferences.tab_bindings;
                self.native_ui.set_visibility(
                    self.preferences.settings_body,
                    !self.preferences_bindings_tab,
                );
                self.native_ui.set_visibility(
                    self.preferences.bindings_body,
                    self.preferences_bindings_tab,
                );
                self.rebuild_binding_rows();
                return true;
            }
            if msg.destination == self.preferences.reset_all {
                self.keybindings.reset_all();
                self.keybindings.save();
                self.editor_events.push_back(EditorEvent::ResetAllSettings);
                self.rebuild_binding_rows();
                return true;
            }
            if let Some((id, _, _)) = self
                .binding_rows
                .iter()
                .find(|(_, capture, _)| *capture == msg.destination)
            {
                self.rebinding = Some(id);
                self.push_toast("Press a shortcut, or Esc to cancel");
                return true;
            }
            if let Some((id, _, _)) = self
                .binding_rows
                .iter()
                .find(|(_, _, reset)| *reset == msg.destination)
                .map(|entry| (entry.0, entry.1, entry.2))
            {
                self.keybindings.reset(id);
                self.keybindings.save();
                self.rebuild_binding_rows();
                return true;
            }
        }
        if msg.destination == self.preferences.search
            && let Some(SearchBoxMessage::Query(query)) = msg.data::<SearchBoxMessage>()
        {
            self.preferences_query = query.clone();
            self.settings_signature.clear();
            if self.preferences_bindings_tab {
                self.rebuild_binding_rows();
            }
            return true;
        }
        if msg.destination == self.preferences.modified_only
            && let Some(CheckBoxMessage::Check(value)) = msg.data::<CheckBoxMessage>()
        {
            self.preferences_modified_only = *value;
            self.settings_signature.clear();
            if self.preferences_bindings_tab {
                self.rebuild_binding_rows();
            }
            return true;
        }

        // A settings row edited. The value shape follows the widget, exactly
        // as it does for entity properties; the difference is only where the
        // write lands.
        let Some(binding) = self.settings_bindings.get(&msg.destination).cloned() else {
            return false;
        };
        let value = if let Some(NumericFieldMessage::ValueChanged(v)) =
            msg.data::<NumericFieldMessage>()
        {
            Self::numeric_reflect_value(&binding, *v)
        } else if let Some(CheckBoxMessage::Check(v)) = msg.data::<CheckBoxMessage>() {
            Some(somnium_ecs::reflect::ReflectValue::Bool(*v))
        } else if let Some(TextBoxMessage::TextCommit(v)) = msg.data::<TextBoxMessage>() {
            Some(somnium_ecs::reflect::ReflectValue::Str(v.clone()))
        } else if let Some(ComboBoxMessage::SelectionChanged(index)) = msg.data::<ComboBoxMessage>()
        {
            Some(somnium_ecs::reflect::ReflectValue::I64(*index as i64))
        } else {
            None
        };
        let Some(value) = value else {
            // A live scrub inside the Preferences window is deliberately
            // ignored: settings are written straight to disk, and a drag would
            // otherwise write the file once per pixel.
            return msg.data::<NumericFieldMessage>().is_some();
        };
        self.editor_events.push_back(EditorEvent::SetSetting {
            component: binding.component,
            field: binding.field,
            value,
        });
        true
    }

    fn command_context(&self) -> crate::commands::EditorCtx {
        crate::commands::EditorCtx {
            has_selection: self
                .last_outliner_state
                .as_ref()
                .is_some_and(|(_, selected)| selected.is_some()),
            // UiManager does not own the undo cursor; core remains authoritative.
            // Keep existing reachability until CONTROL-H exposes that snapshot.
            can_undo: true,
            can_redo: true,
            has_content_target: self.content_menu_target.is_some(),
            has_clipboard: self.clipboard_filled,
        }
    }

    /// Core owns the entity clipboard; this is the enablement mirror, pushed
    /// every frame so `Paste` greys out honestly instead of always offering.
    pub fn set_clipboard_filled(&mut self, filled: bool) {
        self.clipboard_filled = filled;
    }

    fn run_command_id(&mut self, id: &str) -> bool {
        if let Some(command) = crate::commands::registry().get(id).copied() {
            if !command.enabled(&self.command_context()).is_enabled() {
                return false;
            }
            self.run_command_action(command.action);
        } else {
            match self.palette_targets.get(id).cloned() {
                Some(PaletteTarget::Entity(entity)) => self
                    .editor_events
                    .push_back(EditorEvent::SelectEntity(Some(entity))),
                Some(PaletteTarget::Asset(path)) => {
                    self.editor_events
                        .push_back(EditorEvent::ShowContentItemInFolder(
                            path.to_string_lossy().into_owned(),
                        ))
                }
                Some(PaletteTarget::Help(page)) => self.toggle_help(Some(page)),
                Some(PaletteTarget::Drawer) => self.toggle_drawer(),
                Some(PaletteTarget::Log) => self.toggle_log_panel(),
                None => return false,
            }
        }
        self.command_history.record(id);
        self.command_history.save();
        true
    }

    fn run_command_action(&mut self, action: crate::commands::CommandAction) {
        use crate::commands::CommandAction as A;
        match action {
            A::NewScene => self.prompt_unsaved_new(),
            A::SaveScene => self.editor_events.push_back(EditorEvent::SaveScene),
            A::ImportModel => self.editor_events.push_back(EditorEvent::ImportModel),
            A::Undo => self.editor_events.push_back(EditorEvent::Undo),
            A::Redo => self.editor_events.push_back(EditorEvent::Redo),
            A::DeleteSelected => self.editor_events.push_back(EditorEvent::DeleteSelected),
            A::DuplicateSelected => self.editor_events.push_back(EditorEvent::DuplicateSelected),
            A::SetDebugView(id) => self.editor_events.push_back(EditorEvent::SetDebugView(id)),
            A::ToggleRenderSwitch(id) => self
                .editor_events
                .push_back(EditorEvent::ToggleRenderSwitch(id)),
            A::ViewPreset(index) => self.editor_events.push_back(EditorEvent::ViewPreset(index)),
            A::SetBookmark(slot) => self
                .editor_events
                .push_back(EditorEvent::SetCameraBookmark(slot)),
            A::RecallBookmark(slot) => self
                .editor_events
                .push_back(EditorEvent::RecallCameraBookmark(slot)),
            A::ToggleOrbitSelection => self
                .editor_events
                .push_back(EditorEvent::ToggleOrbitSelection),
            // CONTROL-L. The hour comes from the registry's own table, so a
            // preset row and the hour it means cannot disagree.
            A::SetSkyPreset(id) => self.editor_events.push_back(EditorEvent::SetSkyPreset(id)),
            A::SetWeatherPreset(id) => self
                .editor_events
                .push_back(EditorEvent::SetWeatherPreset(id)),
            A::SetTimeOfDay(id) => {
                if let Some((_, _, hour)) = crate::commands::TIME_PRESETS
                    .iter()
                    .find(|(preset, _, _)| *preset == id)
                {
                    self.editor_events.push_back(EditorEvent::SetTimeOfDayHour {
                        hour: *hour,
                        live: false,
                    });
                }
            }
            A::OpenPreferences => self.toggle_preferences(),
            A::OpenProjectPicker => self.editor_events.push_back(EditorEvent::OpenProjectPicker),
            A::CopySelected => self.editor_events.push_back(EditorEvent::CopySelected),
            A::PasteClipboard => self.editor_events.push_back(EditorEvent::PasteClipboard),
            A::SelectAll => self.editor_events.push_back(EditorEvent::SelectAll),
            A::FocusSelection => self.editor_events.push_back(EditorEvent::FocusSelection),
            A::RenameSelected => self.begin_outliner_rename(),
            A::Play => self.editor_events.push_back(EditorEvent::PlaySimulation),
            A::Pause => self.editor_events.push_back(EditorEvent::PauseSimulation),
            A::Stop => self.editor_events.push_back(EditorEvent::StopSimulation),
            A::Step => self.editor_events.push_back(EditorEvent::StepSimulation),
            A::ToggleProfiler => self.editor_events.push_back(EditorEvent::ToggleProfiler),
            A::ToggleDrawer => self.toggle_drawer(),
            A::TogglePalette => self.toggle_palette(),
            A::OpenHelp(page) => self.toggle_help(Some(page)),
            A::ReloadScripts => self.editor_events.push_back(EditorEvent::ReloadScripts),
            A::ToggleShadingMode => self.editor_events.push_back(EditorEvent::ToggleShadingMode),
            A::SetGizmoMode(mode) => self
                .editor_events
                .push_back(EditorEvent::SetGizmoMode(mode)),
            A::ToggleTerrainEdit => self.editor_events.push_back(EditorEvent::ToggleTerrainEdit),
            A::ToggleFoliage => self.editor_events.push_back(EditorEvent::ToggleFoliage),
            A::ToggleImmersiveViewport => self
                .editor_events
                .push_back(EditorEvent::ToggleImmersiveViewport),
            A::OpenOutputLog => self.toggle_log_panel(),
            A::OpenReferences => self.toggle_references_panel(),
            A::OpenLocalisation => self.toggle_locale_panel(),
            A::SetViewportLayout(layout) => self.set_viewport_layout(layout),
            A::ContentShowReferences => {
                // Only an asset has references. A folder is a place, and the
                // command is disabled for one because `content_target` gates
                // on there being an item at all — but a virtual engine
                // primitive has no id either, and that is not a failure worth
                // a message.
                if let Some(asset) = self
                    .content_menu_target
                    .as_ref()
                    .and_then(|entry| entry.asset_id)
                {
                    self.show_references_for(asset);
                }
            }
            A::CreateEntity(kind) => self
                .editor_events
                .push_back(EditorEvent::CreateEntity(kind)),
            A::DockContentDrawer => {
                self.drawer_docked = true;
                if !self.drawer_open {
                    self.toggle_drawer();
                }
            }
            A::SetWorkspace(workspace) => self.set_workspace(workspace),
            A::ResetWorkspace => self.reset_workspace(),
            A::ContentNewFolder => self.open_name_prompt(
                NamePrompt::NewFolder {
                    parent: self.content_menu_folder.clone(),
                },
                "New folder name",
                "NewFolder",
            ),
            A::ContentNewScript => self.open_name_prompt(
                NamePrompt::NewScript {
                    parent: self.content_menu_folder.clone(),
                },
                "New script name",
                "NewScript.luau",
            ),
            A::ContentNewMaterial => self.open_name_prompt(
                NamePrompt::NewMaterial {
                    parent: self.content_menu_folder.clone(),
                },
                "New material name",
                "NewMaterial.sommat",
            ),
            A::ContentRename => {
                if let Some(entry) = self.content_menu_target.clone() {
                    self.begin_inline_content_rename(entry);
                }
            }
            A::ContentShowInFolder => {
                if let Some(entry) = self.content_menu_target.clone() {
                    self.editor_events
                        .push_back(EditorEvent::ShowContentItemInFolder(
                            entry.path.to_string_lossy().into_owned(),
                        ));
                }
            }
            A::ContentRefresh => self.refresh_content_list(),
            A::OpenScene => self.prompt_unsaved_action(EditorEvent::OpenScenePicker),
            A::ContentOpen => {
                if let Some(entry) = self.content_menu_target.clone() {
                    self.open_content_entry(&entry);
                }
            }
            A::ContentAssignToSelection => {
                if let Some(entry) = self.content_menu_target.clone() {
                    if entry.is_dir {
                        self.push_toast("A folder cannot be assigned to a field");
                    } else {
                        self.editor_events
                            .push_back(EditorEvent::AssignAssetToSelection(
                                entry.path.to_string_lossy().into_owned(),
                            ));
                    }
                }
            }
        }
    }

    fn enter_modal_focus(&mut self, root: NodeHandle, target: NodeHandle) {
        let previous = self.native_ui.focused();
        if previous.is_some() && previous != target {
            self.native_ui.send(UiMessage::new(
                previous,
                MessageDirection::ToWidget,
                WidgetMessage::Unfocus,
            ));
        }
        self.native_ui.enter_modal(root, target);
        self.native_ui.send(UiMessage::new(
            target,
            MessageDirection::ToWidget,
            WidgetMessage::Focus,
        ));
    }

    fn exit_modal_focus(&mut self, root: NodeHandle) {
        let previous = self.native_ui.focused();
        if previous.is_some() {
            self.native_ui.send(UiMessage::new(
                previous,
                MessageDirection::ToWidget,
                WidgetMessage::Unfocus,
            ));
        }
        let target = self.native_ui.exit_modal(root);
        if target.is_some() {
            self.native_ui.send(UiMessage::new(
                target,
                MessageDirection::ToWidget,
                WidgetMessage::Focus,
            ));
        }
    }

    fn close_unsaved(&mut self) {
        self.unsaved_open = false;
        self.exit_modal_focus(self.unsaved_popup);
        self.native_ui.send(UiMessage::new(
            self.unsaved_popup,
            MessageDirection::ToWidget,
            PopupMessage::Close,
        ));
        self.native_ui.invalidate_ancestors(self.unsaved_popup);
    }

    fn dismiss_color_ui(&mut self) {
        self.color_open = false;
        self.color_target = None;
        self.color_gesture = None;
        self.native_ui.send(UiMessage::new(
            self.color_popup,
            MessageDirection::ToWidget,
            PopupMessage::Close,
        ));
        self.native_ui.invalidate_ancestors(self.color_popup);
    }

    fn close_color_picker(&mut self, commit: bool) {
        if let (Some(target), Some(gesture)) = (self.color_target, self.color_gesture) {
            let rgba = if commit {
                self.color_live
            } else {
                self.color_original
            };
            self.write_color_target(target, rgba, gesture, false);
        }
        self.dismiss_color_ui();
    }

    fn open_menu(&mut self, which: u8) {
        self.close_all_menus();
        self.close_combo_dropdowns();
        let (flag, popup, anchor) = match which {
            0 => (&mut self.file_popup_open, self.file_popup, self.file_button),
            1 => (
                &mut self.create_popup_open,
                self.create_popup,
                self.create_button,
            ),
            2 => (&mut self.edit_popup_open, self.edit_popup, self.edit_button),
            3 => (&mut self.view_popup_open, self.view_popup, self.view_button),
            4 => (
                &mut self.window_popup_open,
                self.window_popup,
                self.window_button,
            ),
            _ => (
                &mut self.help_menu_open,
                self.help_menu_popup,
                self.help_menu_button,
            ),
        };
        *flag = true;
        self.native_ui.send(UiMessage::new(
            popup,
            MessageDirection::ToWidget,
            PopupMessage::SetAnchor(anchor),
        ));
        self.native_ui.send(UiMessage::new(
            popup,
            MessageDirection::ToWidget,
            PopupMessage::Open,
        ));
        self.native_ui.invalidate_ancestors(popup);
    }

    fn reanchor_open_popups(&mut self) {
        let open = [
            (self.file_popup_open, self.file_popup, self.file_button),
            (
                self.create_popup_open,
                self.create_popup,
                self.create_button,
            ),
            (self.edit_popup_open, self.edit_popup, self.edit_button),
            (self.view_popup_open, self.view_popup, self.view_button),
            (
                self.window_popup_open,
                self.window_popup,
                self.window_button,
            ),
            (
                self.help_menu_open,
                self.help_menu_popup,
                self.help_menu_button,
            ),
        ];
        for (is_open, popup, anchor) in open {
            if is_open {
                self.native_ui.send(UiMessage::new(
                    popup,
                    MessageDirection::ToWidget,
                    PopupMessage::SetAnchor(anchor),
                ));
            }
        }
        if self.open_combo_popup.is_some() {
            for (anchor, popup) in self.combo_entries() {
                if self.open_combo_popup == popup {
                    self.native_ui.send(UiMessage::new(
                        popup,
                        MessageDirection::ToWidget,
                        PopupMessage::SetAnchor(anchor),
                    ));
                }
            }
        }
        let outgoing = self.native_ui.update();
        let _ = outgoing;
    }

    fn update_tooltip(&mut self) {
        let pos = self.native_ui.cursor_pos;
        let text = self.native_ui.tooltip_at(pos);
        if text.is_empty() {
            self.tooltip_since = None;
            self.native_ui.set_visibility(self.tooltip, false);
            return;
        }
        let now = std::time::Instant::now();
        match self.tooltip_since {
            Some((_, since))
                if now.duration_since(since).as_millis() >= self.tooltip_delay_ms as u128 =>
            {
                let size = crate::widgets::tooltip_size(
                    self.native_ui
                        .draw_ctx
                        .font_atlas
                        .measure_text(&text, 11.0, self.font_id),
                );
                let window = self.native_ui.screen_size;
                self.native_ui
                    .send(TextMessage::set_text(self.tooltip, text));
                self.native_ui.set_desired_position(
                    self.tooltip,
                    crate::widgets::place_tooltip(pos, size, window),
                );
                self.native_ui.set_visibility(self.tooltip, true);
            }
            Some(_) => {}
            None => {
                self.tooltip_since = Some((NodeHandle::NONE, now));
                self.native_ui.set_visibility(self.tooltip, false);
            }
        }
    }

    /// Navigate the drawer, recording the move in history.
    ///
    /// Every entry point that changes folder goes through here so back/forward
    /// cannot drift out of step with what is on screen.
    pub fn navigate_content(&mut self, path: String) {
        self.content_history.push(path.clone());
        self.content_path = path;
        self.content_selection.clear();
        self.refresh_content_list();
    }

    /// Step back through the drawer's folder history. Returns false when there
    /// is nowhere to go, so a toolbar button can disable itself honestly.
    pub fn content_back(&mut self) -> bool {
        match self.content_history.back() {
            Some(path) => {
                self.content_path = path.to_string();
                self.content_selection.clear();
                self.refresh_content_list();
                true
            }
            None => false,
        }
    }

    pub fn content_forward(&mut self) -> bool {
        match self.content_history.forward() {
            Some(path) => {
                self.content_path = path.to_string();
                self.content_selection.clear();
                self.refresh_content_list();
                true
            }
            None => false,
        }
    }

    pub fn can_content_back(&self) -> bool {
        self.content_history.can_go_back()
    }

    pub fn can_content_forward(&self) -> bool {
        self.content_history.can_go_forward()
    }

    /// Restrict the drawer to one asset kind.
    pub fn set_content_kind(&mut self, kind: crate::metaphor::ContentFilterKind) {
        if self.content_kind != kind {
            self.content_kind = kind;
            self.content_selection.clear();
            self.refresh_content_list();
        }
    }

    pub fn content_kind(&self) -> crate::metaphor::ContentFilterKind {
        self.content_kind
    }

    /// Cycle tile size. Selection survives: the same assets are on screen.
    pub fn cycle_content_density(&mut self) -> crate::metaphor::ContentDensity {
        self.content_density = self.content_density.next();
        self.refresh_content_list();
        self.content_density
    }

    pub fn content_density(&self) -> crate::metaphor::ContentDensity {
        self.content_density
    }

    /// Select an asset. `additive` is the Ctrl-click path: it toggles, so a
    /// second Ctrl-click on the same tile removes it from the set.
    pub fn select_content(&mut self, path: std::path::PathBuf, additive: bool) {
        if additive {
            if !self.content_selection.remove(&path) {
                self.content_selection.insert(path);
            }
        } else {
            self.content_selection.clear();
            self.content_selection.insert(path);
        }
        self.refresh_content_list();
    }

    pub fn content_selection(&self) -> &std::collections::HashSet<std::path::PathBuf> {
        &self.content_selection
    }

    /// Previews the engine must render, drained for the host to fulfil.
    ///
    /// The UI owns no filesystem decoder or renderer. The host answers with
    /// [`Self::deliver_thumbnail`]
    /// or [`Self::fail_thumbnail`]; an unanswered request leaves the tile on
    /// its type icon, which is a correct resting state rather than a bug.
    pub fn take_thumbnail_requests(&mut self) -> Vec<crate::thumbnail::ThumbnailRequest> {
        self.native_ui.draw_ctx.thumbnails.take_requests(8)
    }

    /// Supply a rendered preview: `CELL * CELL` RGBA8.
    pub fn deliver_thumbnail(&mut self, path: &std::path::Path, rgba: &[u8]) -> bool {
        self.native_ui.draw_ctx.thumbnails.deliver(path, rgba)
    }

    /// Copy prepared previews into the atlas within an actual wall-clock
    /// budget. Unfinished results remain queued for the next frame.
    pub fn deliver_thumbnails_budgeted(
        &mut self,
        ready: &mut VecDeque<(std::path::PathBuf, Vec<u8>)>,
        budget: std::time::Duration,
    ) -> usize {
        self.native_ui
            .draw_ctx
            .thumbnails
            .deliver_budgeted(ready, budget)
    }

    /// Record that a preview could not be produced, so it is not retried.
    pub fn fail_thumbnail(&mut self, path: &std::path::Path) {
        self.native_ui.draw_ctx.thumbnails.mark_failed(path);
    }

    /// Invalidate one thumbnail after its authored asset changed.
    pub fn invalidate_thumbnail(&mut self, path: &std::path::Path) {
        self.native_ui.draw_ctx.thumbnails.invalidate(path);
        self.refresh_content_list();
    }

    /// Atomically replace the drawer's inventory with a worker-built snapshot.
    /// Resolve an asset reference back to its record.
    ///
    /// An `AssetId` is a hash of a path, so a component that stores one cannot
    /// recover the path on its own — it needs the database that hashed it. The
    /// shell already holds the snapshot; a game reading an authored asset field
    /// should not have to keep a second copy of it in step.
    #[must_use]
    pub fn asset_record(
        &self,
        id: somnium_asset::database::AssetId,
    ) -> Option<&somnium_asset::database::AssetRecord> {
        self.asset_db.get(id)
    }

    pub fn set_asset_snapshot(&mut self, snapshot: somnium_asset::database::AssetDbSnapshot) {
        self.asset_db = snapshot;
        self.refresh_content_list();
    }

    /// How the viewport region is divided this frame (MORROWIND-J step 3).
    #[must_use]
    pub fn viewport_layout(&self) -> crate::viewport_layout::ViewportLayout {
        self.viewport_layout
    }

    /// Choose how the viewport region is divided.
    pub fn set_viewport_layout(&mut self, layout: crate::viewport_layout::ViewportLayout) {
        if self.viewport_layout == layout {
            return;
        }
        self.viewport_layout = layout;
        self.push_toast(layout.label());
    }

    /// The viewport's rectangle in **physical** pixels.
    ///
    /// Physical, because it is handed to the renderer, and the renderer draws
    /// into a swapchain measured in physical pixels. Handing it logical ones on
    /// a 150% display puts three quarters of the scene in the top-left corner —
    /// the classic DPI bug, and one that looks like a layout error.
    #[must_use]
    pub fn viewport_physical_rect(&self, scale: f32) -> (u32, u32, u32, u32) {
        let b = self.native_ui.screen_bounds(self.viewport_handle);
        let scale = if scale > 0.0 { scale } else { 1.0 };
        (
            (b.x * scale).max(0.0) as u32,
            (b.y * scale).max(0.0) as u32,
            (b.w * scale).max(0.0) as u32,
            (b.h * scale).max(0.0) as u32,
        )
    }

    /// Hand the editor a localisation catalogue, already projected as a table.
    ///
    /// A `DataTable` and not a `Catalog`: `somnium_ui` does not know what a
    /// catalogue is and must not learn — the projection lives in
    /// `somnium_core::i18n`, which is the one place that knows both
    /// vocabularies. See [`Self::localisation_table`] for the way back.
    pub fn set_localisation_table(&mut self, table: crate::data_table::DataTable) {
        self.locale_table = Some(table.clone());
        self.native_ui.send(UiMessage::new(
            self.locale_grid,
            MessageDirection::ToWidget,
            crate::widgets::data_grid::DataGridMessage::SetTable(Box::new(table)),
        ));
    }

    /// The last committed state of the table, for a host about to save it.
    #[must_use]
    pub fn localisation_table(&self) -> Option<&crate::data_table::DataTable> {
        self.locale_table.as_ref()
    }

    /// Open the Localisation panel on the table already loaded.
    pub fn show_localisation(&mut self) {
        if !self.locale_open {
            self.toggle_locale_panel();
        }
    }

    /// The project's reference graph, rebuilt with the asset inventory.
    ///
    /// MORROWIND-M item 3. It arrives from the same job as the snapshot, so
    /// the two always describe the same disk — a graph a scan behind the
    /// drawer would name assets the drawer cannot show.
    pub fn set_dependency_index(&mut self, index: somnium_asset::depend::DependencyIndex) {
        self.dependency_index = index;
        if self.references_open {
            self.refresh_references_panel();
        }
    }

    /// Point the References panel at an asset and bring it forward.
    pub fn show_references_for(&mut self, asset: somnium_asset::database::AssetId) {
        self.references_subject = Some(asset);
        if !self.references_open {
            self.references_open = true;
            self.drawer_open = false;
            self.log_open = false;
        }
        self.refresh_references_panel();
        self.apply_bottom_panel();
    }

    /// The three questions, in the order they get asked.
    ///
    /// Outgoing first — it is the one with an answer even for an asset nothing
    /// uses — then incoming, then the transitive breakage, which comes last
    /// because it is a superset of incoming and reads as an alarm rather than
    /// as information.
    fn refresh_references_panel(&mut self) {
        self.native_ui.clear_children(self.references_list);
        self.references_rows.clear();

        let Some(subject) = self.references_subject else {
            self.native_ui
                .send(TextMessage::set_text(self.references_title, "References"));
            self.reference_note(
                "Right-click an asset in the Content Drawer and choose Show References.",
            );
            return;
        };

        let name = self
            .asset_db
            .get(subject)
            .map(|record| record.relative_path.clone())
            .unwrap_or_else(|| format!("{subject}"));
        self.native_ui.send(TextMessage::set_text(
            self.references_title,
            &format!("References - {name}"),
        ));

        // A folder is a place, not an asset: it references nothing and
        // nothing references it, and the three lists would all be empty in a
        // way that reads as "safe to delete" when its contents may not be.
        if self
            .asset_db
            .get(subject)
            .is_some_and(|record| record.kind == somnium_asset::database::AssetKind::Folder)
        {
            self.reference_note(
                "A folder has no references of its own. Ask about the assets inside it.",
            );
            return;
        }

        let uses = self.dependency_index.references(subject);
        let used_by = self.dependency_index.referenced_by(subject);
        let breakage = self.dependency_index.breakage(subject);
        let dangling = self.dependency_index.dangling(subject);
        let scannable = self
            .asset_db
            .get(subject)
            .is_some_and(|record| somnium_asset::depend::is_scannable(record.kind));

        if scannable {
            self.reference_section(&format!("Uses ({})", uses.len()), &uses);
        } else {
            // A `.glb` names its own textures, a script names assets by path,
            // and the index reads neither. Reporting that as "uses nothing" is
            // the lie that would make a dependency view worse than none.
            self.reference_note(
                "References are read out of scenes, prefabs, materials and UI documents. This kind is not one of them, so what it uses is not listed here.",
            );
        }
        self.reference_section(&format!("Used by ({})", used_by.len()), &used_by);

        // Only when it differs from the direct dependents: repeating the same
        // three rows under a scarier heading teaches people to ignore it.
        if breakage.len() > used_by.len() {
            self.reference_section(
                &format!("Breaks if deleted ({})", breakage.len()),
                &breakage,
            );
        }
        if !dangling.is_empty() {
            self.reference_section(&format!("Missing ({})", dangling.len()), &dangling);
        }
        if uses.is_empty() && used_by.is_empty() && dangling.is_empty() && scannable {
            self.reference_note(
                "Nothing in the project references this, and it references nothing.",
            );
        }
    }

    /// A heading, then one clickable row per asset.
    fn reference_section(&mut self, heading: &str, assets: &[somnium_asset::database::AssetId]) {
        if assets.is_empty() {
            return;
        }
        let t = theme::active();
        let head = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
            left: 8.0,
            top: 8.0,
            right: 0.0,
            bottom: 2.0,
        }))
        .with_role(TextRole::SectionCaps)
        .with_text(heading)
        .with_font_id(self.font_id)
        .build();
        self.native_ui.add_node(head, self.references_list);

        for asset in assets {
            let record = self.asset_db.get(*asset).cloned();
            // An id with no record is a reference to something that is not in
            // the project. The raw id is the only honest label, and it is what
            // you paste into a search.
            let label = record
                .as_ref()
                .map(|record| record.relative_path.clone())
                .unwrap_or_else(|| format!("{asset}"));
            let icon = record
                .as_ref()
                .map(|record| crate::metaphor::icon_for_path(&record.absolute_path, false))
                .unwrap_or(crate::icons::IconId::Warn);
            let tint = if record.is_some() {
                theme::TEXT_PRIMARY
            } else {
                t.semantic.status.warning.bytes()
            };
            let row = ButtonBuilder::new(
                WidgetBuilder::new()
                    .with_height(theme::ROW_HEIGHT)
                    .with_background(theme::TRANSPARENT),
            )
            .build();
            let row = self.native_ui.add_node(row, self.references_list);
            let line =
                StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
                    .with_orientation(Orientation::Horizontal)
                    .build();
            let line = self.native_ui.add_node(line, row);
            let glyph = ImageBuilder::new(
                WidgetBuilder::new()
                    .with_width(14.0)
                    .with_height(14.0)
                    .with_margin(Thickness::axes(8.0, 4.0)),
            )
            .with_icon(icon)
            .with_size(14.0)
            .with_tint(tint)
            .build();
            self.native_ui.add_node(glyph, line);
            let text =
                TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(0.0, 4.0)))
                    .with_text(&label)
                    .with_font_size(11.0)
                    .with_font_id(self.font_id)
                    .with_color(tint)
                    .build();
            self.native_ui.add_node(text, line);
            self.references_rows.push((row, *asset));
        }
    }

    /// A line of prose in the panel, for the states a list cannot express.
    fn reference_note(&mut self, text: &str) {
        let note = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::uniform(8.0)))
            .with_text(text)
            .with_font_size(11.0)
            .with_font_id(self.font_id)
            .with_color(theme::active().semantic.text.secondary.bytes())
            .with_wrap(true)
            .build();
        self.native_ui.add_node(note, self.references_list);
    }

    /// Select sort order for the current database query.
    pub fn set_content_sort(&mut self, sort: somnium_asset::database::AssetSort, descending: bool) {
        self.content_sort = sort;
        self.content_sort_descending = descending;
        self.refresh_content_list();
    }

    /// Promote only tiles intersecting the scroll viewport. This runs after
    /// layout so scrolling changes priority without rebuilding the drawer.
    fn request_visible_thumbnails(&mut self) {
        let viewport = self.native_ui.screen_bounds(self.content_scroll);
        let visible: Vec<_> = self
            .content_entries
            .iter()
            .filter(|(_, entry)| !entry.is_dir && !entry.is_engine)
            .filter_map(|(handle, entry)| {
                let bounds = self.native_ui.screen_bounds(*handle);
                let intersection = viewport.intersect(&bounds);
                (intersection.w > 0.0 && intersection.h > 0.0).then(|| entry.path.clone())
            })
            .collect();
        for path in visible {
            self.native_ui.draw_ctx.thumbnails.request(&path, true);
        }
    }

    fn refresh_content_list(&mut self) {
        let kind_mask = match self.content_kind {
            crate::metaphor::ContentFilterKind::All => u64::MAX,
            crate::metaphor::ContentFilterKind::Folders => {
                somnium_asset::database::AssetKind::Folder.bit()
            }
            crate::metaphor::ContentFilterKind::Models => {
                somnium_asset::database::AssetKind::Mesh.bit()
            }
            crate::metaphor::ContentFilterKind::Textures => {
                somnium_asset::database::AssetKind::Texture.bit()
            }
            crate::metaphor::ContentFilterKind::Scripts => {
                somnium_asset::database::AssetKind::Script.bit()
            }
            crate::metaphor::ContentFilterKind::Scenes => {
                somnium_asset::database::AssetKind::Scene.bit()
            }
            crate::metaphor::ContentFilterKind::Audio => {
                somnium_asset::database::AssetKind::Audio.bit()
            }
        };
        let mut entries: Vec<crate::metaphor::ContentEntry> = self
            .asset_db
            .query(&somnium_asset::database::AssetQuery {
                parent: self.content_path.clone(),
                text: self.content_filter.clone(),
                kind_mask,
                sort: self.content_sort,
                descending: self.content_sort_descending,
            })
            .into_iter()
            .map(Into::into)
            .collect();
        if self.show_engine_content && self.content_path.is_empty() {
            entries.extend(
                crate::metaphor::virtual_engine_content(&self.content_filter)
                    .into_iter()
                    .filter(|entry| entry.is_engine && self.content_kind.accepts(entry)),
            );
        }
        self.content_all = entries;
        // Forget the window rather than the tiles: the folder changed, so the
        // tiles the old window named are about to describe different assets.
        self.content_window = None;
        self.sync_content_tiles();
        self.refresh_content_breadcrumb();
    }

    /// Which tiles the drawer can show, given the layout it last had.
    ///
    /// The seam is the outliner's — a clip rectangle and a content origin —
    /// but the answer is put to a different use: here it decides which tiles
    /// *exist*, not which of the existing ones paint.
    fn content_grid_window(&self) -> crate::virtual_list::GridWindow {
        let (tile_w, tile_h, _) = self.content_density.metrics();
        let canvas = self.native_ui.screen_bounds(self.content_list);
        // Pitch, not size: the gap belongs to the cell. The last column has no
        // gap after it, so the width offered is one gap wider than the panel —
        // without that, a panel that fits four tiles is told it fits three.
        crate::virtual_list::GridWindow::new(
            canvas.y,
            (tile_w + CONTENT_GAP, tile_h + CONTENT_GAP),
            canvas.w + CONTENT_GAP,
            self.content_all.len(),
            self.native_ui.screen_bounds(self.content_scroll),
        )
    }

    /// Build exactly the tiles the window names, and nothing else.
    ///
    /// MORROWIND-M. The outliner virtualises its *draw*, because a `TreeView`
    /// is one widget that paints rows itself. The drawer cannot: a tile is a
    /// real button with a real icon and a real label, and it is a drop target
    /// and a drag source by being one. So here the window decides which
    /// widgets are built, and the canvas is left as tall as all of them.
    ///
    /// Cheap to call every frame: it compares the window it would build
    /// against the one it did, and a drawer nobody scrolled does nothing.
    fn sync_content_tiles(&mut self) {
        let window = self.content_grid_window();
        if self.content_window == Some(window) {
            return;
        }
        // An inline rename is a text box parented to a tile. Rebuilding under
        // it would delete the field being typed into, so a scroll during a
        // rename leaves the window stale until the rename lands. Never on the
        // first build, or the rename would be parented to nothing.
        if self.content_inline_rename.is_some() && self.content_window.is_some() {
            return;
        }

        let (tile_w, tile_h, icon_px) = self.content_density.metrics();
        let pitch = (tile_w + CONTENT_GAP, tile_h + CONTENT_GAP);
        let font_id = self.font_id;
        let parent = self.content_list;
        self.native_ui.clear_children(parent);
        self.content_entries.clear();
        self.content_window = Some(window);

        // Phase 27-G. A drawer with nothing in it used to be a blank grey
        // rectangle, which reads as broken rather than as empty. A filtered
        // miss and a genuinely empty folder are different situations and get
        // different copy — offering "import a model" to someone who mistyped a
        // search would be the wrong advice.
        if self.content_all.is_empty() {
            let state = if self.content_filter.is_empty() {
                crate::metaphor::empty::CONTENT
            } else {
                crate::metaphor::empty::CONTENT_FILTERED
            };
            // The height still gets set, to the nought rows an empty folder
            // is: the canvas does not clip, so the empty state is visible
            // regardless, and the scrollbar correctly says there is nowhere to
            // scroll to.
            self.native_ui.set_height(parent, 0.0);
            crate::editor::parts::build_empty_state(&mut self.native_ui, parent, font_id, state);
            return;
        }

        // As tall as the whole folder, though only a screenful was built.
        self.native_ui
            .set_height(parent, window.content_height(pitch.1));

        let visible = self.content_all[window.range()].to_vec();
        for (offset, entry) in visible.into_iter().enumerate() {
            let index = window.first + offset;
            // A selected tile keeps the raised fill but gains the selection
            // wash, so selection reads the same way here as in the Outliner.
            let selected = self.content_selection.contains(&entry.path);
            let tile_bg = if selected {
                theme::active().semantic.accent.selected_bg.bytes()
            } else {
                theme::BG_RAISED
            };
            let btn = ButtonBuilder::new(
                WidgetBuilder::new()
                    .with_width(tile_w)
                    .with_height(tile_h)
                    .with_background(tile_bg),
            )
            .build();
            let bh = self.native_ui.add_node(btn, parent);
            let col =
                StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
                    .with_orientation(Orientation::Vertical)
                    .build();
            let col_h = self.native_ui.add_node(col, bh);
            let icon = ImageBuilder::new(
                WidgetBuilder::new()
                    .with_width(tile_w)
                    .with_height(icon_px)
                    .with_margin(Thickness {
                        left: 0.0,
                        top: 8.0,
                        right: 0.0,
                        bottom: 0.0,
                    }),
            )
            .with_icon(entry.icon)
            .with_asset(entry.path.clone())
            .with_size(icon_px)
            .with_tint(if entry.is_dir {
                theme::FOLDER_SAND
            } else {
                theme::TEXT_PRIMARY
            })
            .build();
            self.native_ui.add_node(icon, col_h);
            let lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
                left: 4.0,
                top: 6.0,
                right: 4.0,
                bottom: 0.0,
            }))
            .with_text(&entry.name)
            .with_font_size(11.0)
            .with_font_id(font_id)
            .with_color(theme::TEXT_PRIMARY)
            .with_wrap(true)
            .build();
            self.native_ui.add_node(lbl, col_h);

            // Phase 27-G. A type badge, so a tile says what it *is* without the
            // user having to parse the extension out of a wrapped filename or
            // recognise the icon. Folders are self-evident and get none; the
            // engine's virtual primitives are labelled as such because they do
            // not exist on disk and cannot be revealed in a file browser.
            let badge_text = if entry.is_dir {
                String::new()
            } else if entry.is_engine {
                "ENGINE".to_string()
            } else {
                entry
                    .path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_uppercase())
                    .unwrap_or_default()
            };
            if !badge_text.is_empty() {
                let t = theme::active();
                // Ember is the warm content half of the palette (§4.2); it marks
                // asset identity and never competes with indigo for a state cue.
                let badge = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
                    left: 4.0,
                    top: 2.0,
                    right: 4.0,
                    bottom: 0.0,
                }))
                .with_text(&badge_text)
                .with_font_size(t.typography.caption)
                .with_font_id(font_id)
                .with_color(if entry.is_engine {
                    t.semantic.text.muted.bytes()
                } else {
                    t.ember.bytes()
                })
                .build();
                self.native_ui.add_node(badge, col_h);
            }

            // Absolute placement is what the window rests on: a tile sits
            // where its index *in the whole folder* puts it, not where its
            // position among the built widgets would.
            let cell = window.tile_rect(index, pitch);
            self.native_ui
                .place_node(bh, crate::types::Rect::new(cell.x, cell.y, tile_w, tile_h));
            self.content_entries.push((bh, entry));
        }
    }

    fn refresh_content_breadcrumb(&mut self) {
        let mut parts = vec!["Game".to_string()];
        if !self.content_path.is_empty() {
            for p in self.content_path.split(['/', '\\']) {
                if !p.is_empty() {
                    parts.push(p.to_string());
                }
            }
        }
        self.native_ui.send(UiMessage::new(
            self.content_breadcrumb,
            MessageDirection::ToWidget,
            BreadcrumbMessage::SetParts(parts),
        ));
    }

    fn apply_inspector_filter(&mut self) {
        let q = self.inspector_filter.to_ascii_lowercase();
        let h = &self.inspector_handles;
        let pairs = [
            (h.post_section, "renderer diagnostics census shade bins"),
            (
                h.terrain_section,
                "terrain paint layer hex aerial lod distance",
            ),
            (h.foliage_section, "foliage brush grass tree"),
        ];
        if q.is_empty() {
            return;
        }
        for (section, names) in pairs {
            let hit = names.split_whitespace().any(|w| w.contains(&q));
            if !hit {
                self.native_ui.set_visibility(section, false);
            }
        }
    }

    /// Schema defaults are the durable baseline; rebuilding Details does not
    /// manufacture a second save-relative notion of "modified".
    pub fn reset_inspector_baseline(&mut self) {}

    pub fn refresh_modified_dots(&mut self) {
        for (row, binding) in &self.generated_rows {
            self.native_ui.send(PropertyRowMessage::set_modified(
                *row,
                binding.value != binding.default,
            ));
        }
    }

    /// Re-apply Details search after the per-frame inspector refresh.
    pub fn refresh_inspector_filter(&mut self) {
        self.apply_inspector_filter();
    }

    /// Returns true if a text-input widget (TextBox or NumericField) has keyboard focus.
    pub fn has_text_focus(&self) -> bool {
        self.native_ui.has_text_focus()
    }

    /// Whether a right-drag over the viewport is currently flying the camera.
    ///
    /// The engine's own shortcut dispatcher runs *before* the UI is consulted,
    /// so it has to ask.
    #[must_use]
    pub fn viewport_camera_active(&self) -> bool {
        self.native_ui.viewport_camera_active()
    }

    /// Whether a play session is running — playing or paused.
    #[must_use]
    pub fn play_session_active(&self) -> bool {
        self.play_session_active
    }

    /// Whether the game currently owns the keyboard.
    ///
    /// True while the fly-cam is driving *or* a play session is running. In
    /// both cases an unmodified single-key editor shortcut must stand down:
    /// bare `S` is the Scale tool and is also "move backward", and whichever
    /// dispatcher claims it first stops the key reaching the game at all.
    #[must_use]
    pub fn game_owns_keyboard(&self) -> bool {
        self.viewport_camera_active() || self.play_session_active
    }

    // ── Editor event queue ────────────────────────────────────────────────────

    /// Drain one EditorEvent per call; returns None when queue is empty.
    pub fn poll_editor_event(&mut self) -> Option<EditorEvent> {
        self.editor_events.pop_front()
    }

    // ── Live UI updates ───────────────────────────────────────────────────────

    /// CONTROL-L: publish the scene's clock to the viewport context bar.
    ///
    /// `None` means the scene has no day cycle, and the whole cluster
    /// disappears — a control that cannot do anything is worse than no control,
    /// which is craft defect C8's rule applied to presence rather than to
    /// enablement.
    pub fn update_time_of_day(&mut self, hour: Option<f32>) {
        let visible = hour.is_some();
        self.native_ui.set_visibility(self.time_cluster, visible);
        let Some(hour) = hour else {
            self.time_shown = None;
            return;
        };
        // A tenth of a minute: below that the label reads the same and the
        // handle moves less than a pixel, so rewriting either is pure churn.
        if self
            .time_shown
            .is_some_and(|shown| (shown - hour).abs() < 0.002)
        {
            return;
        }
        self.time_shown = Some(hour);
        self.native_ui
            .send(SliderMessage::set_value(self.time_slider, hour / 24.0));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (h, m) = (hour.floor() as u32, ((hour.fract()) * 60.0).round() as u32);
        let text = if m >= 60 {
            format!("{:02}:00", (h + 1) % 24)
        } else {
            format!("{h:02}:{m:02}")
        };
        self.native_ui
            .send(TextMessage::set_text(self.time_label, text));
    }

    /// Update the camera-speed slider and its readout (Phase 20B).
    ///
    /// `normalized` is the slider position in `0..=1`; `speed` is the resulting
    /// world speed, shown as text. Called when the scroll wheel changes the
    /// speed so the widget stays in sync with the camera.
    pub fn update_camera_speed(&mut self, normalized: f32, speed: f32) {
        self.native_ui.send(SliderMessage::set_value(
            self.camera_speed_slider,
            normalized,
        ));
        self.native_ui.send(TextMessage::set_text(
            self.camera_speed_label,
            format!("{speed:.1} m/s"),
        ));
    }

    /// Keep the UE-style transport controls visually synchronized with the
    /// engine-owned simulation state.
    pub fn update_simulation_controls(&mut self, state: u8) {
        // Every transport transition comes through here, which makes it the
        // one place that knows a play session is running. `0` is stopped; `1`
        // playing and `2` paused are both "a session exists", and during one
        // the keyboard belongs to the game.
        self.play_session_active = state != 0;
        let play = match state {
            1 => "Playing",
            2 => "Paused",
            _ => "Stopped",
        };
        self.native_ui
            .send(TextMessage::set_text(self.play_label, play.to_string()));
        self.native_ui
            .send(ButtonMessage::set_selected(self.play_button, state == 1));
        self.native_ui
            .send(ButtonMessage::set_selected(self.pause_button, state == 2));
        self.native_ui
            .send(ButtonMessage::set_selected(self.stop_button, state == 0));
    }

    /// The editor shell's accessibility tree. MORROWIND-I.
    ///
    /// Built on demand rather than maintained: the shell rebuilds widgets
    /// freely, and a cached tree would be one more thing that can be stale in a
    /// system whose whole failure mode is staleness nobody sighted can see.
    pub fn a11y_tree(&self) -> crate::a11y::A11yTree {
        self.native_ui.a11y_tree()
    }

    /// Apply accessibility preferences to the shell. MORROWIND-I.
    pub fn set_a11y_settings(&mut self, settings: crate::a11y::A11ySettings) {
        self.native_ui.set_a11y_settings(settings);
    }

    /// The accessibility preferences in force.
    pub fn a11y_settings(&self) -> crate::a11y::A11ySettings {
        self.native_ui.a11y_settings()
    }

    pub fn is_immersive(&self) -> bool {
        self.immersive
    }

    /// Borderless fullscreen and hide chrome. Esc (or a second toggle) restores.
    pub fn set_immersive(&mut self, on: bool) {
        if self.immersive == on {
            return;
        }
        self.immersive = on;
        self.native_ui
            .send(ButtonMessage::set_selected(self.immersive_button, on));
        if on {
            self.set_play_overlays_hidden(true);
            self.immersive_restore_maximized = self.window.is_maximized();
            self.immersive_restore_fullscreen = self.window.fullscreen();
            self.window
                .set_fullscreen(Some(Fullscreen::Borderless(None)));
        } else {
            self.window
                .set_fullscreen(self.immersive_restore_fullscreen.take());
            if self.immersive_restore_maximized {
                self.window.set_maximized(true);
            }
        }
    }

    /// Rebuild the outliner entity list.  `entities` is (entity_index, display_name).
    pub fn update_outliner(&mut self, entities: &[(u32, String)], selected: Option<u32>) {
        let rows: Vec<OutlinerRow> = entities
            .iter()
            .map(|(id, name)| OutlinerRow {
                id: *id,
                name: name.clone(),
                depth: 0,
                has_children: false,
                hidden: false,
                locked: false,
                script_error: false,
                tags: Vec::new(),
            })
            .collect();
        self.update_outliner_tree(&rows, selected);
    }

    /// Hierarchical outliner (Phase 26-E), one [`OutlinerRow`] per visible row.
    pub fn update_outliner_tree(&mut self, entities: &[OutlinerRow], selected: Option<u32>) {
        let new_state = (entities.to_vec(), selected);
        if let Some(ref old_state) = self.last_outliner_state {
            if *old_state == new_state {
                return;
            }
        }
        self.last_outliner_state = Some(new_state);

        let filter = crate::outliner_filter::OutlinerFilter::parse(&self.outliner_filter);
        let mut items = Vec::new();
        for row in entities {
            if !filter.matches(row) {
                continue;
            }
            let expanded = self.outliner_expanded.contains(&row.id) || !row.has_children;
            items.push(TreeItem {
                id: row.id,
                label: row.name.clone(),
                depth: row.depth,
                icon: crate::metaphor::icon_for_entity_name(&row.name),
                has_children: row.has_children,
                expanded: expanded || self.outliner_expanded.contains(&row.id),
                hidden: row.hidden,
                locked: row.locked,
                script_error: row.script_error,
            });
        }
        // Phase 27-G: an empty scene says so rather than showing a blank panel.
        let has_entities = !items.is_empty();
        self.native_ui
            .set_visibility(self.outliner_tree, has_entities);
        self.native_ui
            .set_visibility(self.outliner_empty, !has_entities);

        self.outliner_rows = items.iter().map(|i| (self.outliner_tree, i.id)).collect();
        // Mirror the rows so the palette can offer them without reaching into
        // the widget tree.
        self.palette_entities = entities
            .iter()
            .map(|row| (row.id, row.name.clone()))
            .collect();
        self.native_ui
            .send(TreeViewMessage::set_items(self.outliner_tree, items));
        if selected.is_some() {
            self.native_ui.send(UiMessage::new(
                self.outliner_tree,
                MessageDirection::ToWidget,
                TreeViewMessage::SetSelected(selected),
            ));
        }
    }

    /// Phase 16-D: rebuild the Details panel's Scripts section.
    ///
    /// `None` hides it — the selection carries no `ScriptSet`. An empty
    /// state shows the section with just its "New Script" button, which is
    /// how an entity that *could* have scripts differs from one that
    /// cannot.
    ///
    /// The rows are built from what each script declared. Nothing in this
    /// function names a property; adding a field to a script is a one-line
    /// edit in the `.luau` file and nothing here changes.
    pub fn update_script_inspector(&mut self, state: Option<ScriptInspectorState>) {
        let section = self.inspector_handles.script_section;
        let Some(state) = state else {
            self.native_ui.set_visibility(section, false);
            if !self.script_state.attachments.is_empty() {
                self.script_state = ScriptInspectorState::default();
                self.script_widgets.clear();
                let list = self.inspector_handles.script_list;
                self.native_ui.clear_children(list);
            }
            return;
        };
        self.native_ui.set_visibility(section, true);
        if state == self.script_state && !self.script_widgets.is_empty() {
            return;
        }
        if state == self.script_state && state.attachments.is_empty() {
            return;
        }
        self.script_state = state;
        self.rebuild_script_rows();
    }

    /// Build one widget per declared property, plus the per-attachment
    /// controls.
    fn rebuild_script_rows(&mut self) {
        let list = self.inspector_handles.script_list;
        self.native_ui.clear_children(list);
        self.script_widgets.clear();
        let font_id = self.font_id;

        // Cloned because the builders below borrow `self.native_ui`
        // mutably, and the state is small — a handful of rows per entity.
        let attachments = self.script_state.attachments.clone();
        for (index, attachment) in attachments.iter().enumerate() {
            let card = crate::widgets::border::BorderBuilder::new(
                WidgetBuilder::new()
                    .with_margin(Thickness::axes(4.0, 3.0))
                    .with_background(theme::BG_PANEL)
                    .with_foreground(theme::BORDER_DARK),
            )
            .with_stroke_thickness(Thickness::uniform(1.0))
            .build();
            let card = self.native_ui.add_node(card, list);
            let column =
                StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
                    .with_orientation(Orientation::Vertical)
                    .build();
            let column = self.native_ui.add_node(column, card);

            // Header: name, state, and the three structural controls.
            let header =
                StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
                    .with_orientation(Orientation::Horizontal)
                    .build();
            let header = self.native_ui.add_node(header, column);

            let enable = crate::widgets::check_box::CheckBoxBuilder::new(
                WidgetBuilder::new()
                    .with_height(theme::ROW_HEIGHT)
                    .with_margin(Thickness::axes(4.0, 0.0)),
            )
            .with_label(&attachment.asset_name)
            .with_checked(attachment.enabled)
            .with_font_id(font_id)
            .with_font_size(12.0)
            .build();
            let enable = self.native_ui.add_node(enable, header);
            self.script_widgets
                .push((enable, ScriptWidgetAction::Enable(index)));

            for (glyph, action) in [
                ("↑", ScriptWidgetAction::Reorder(index, -1)),
                ("↓", ScriptWidgetAction::Reorder(index, 1)),
                ("✕", ScriptWidgetAction::Detach(index)),
            ] {
                let button = ButtonBuilder::new(
                    WidgetBuilder::new()
                        .with_width(22.0)
                        .with_height(theme::ROW_HEIGHT)
                        .with_background(theme::TRANSPARENT),
                )
                .build();
                let button = self.native_ui.add_node(button, header);
                let label = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
                    left: 6.0,
                    top: 3.0,
                    right: 0.0,
                    bottom: 0.0,
                }))
                .with_text(glyph)
                .with_font_size(12.0)
                .with_font_id(font_id)
                .with_color(theme::TEXT_SECONDARY)
                .build();
                self.native_ui.add_node(label, button);
                self.script_widgets.push((button, action));
            }

            // Status. Quarantine reads differently from an authored
            // disable, so it says which one it is.
            let status = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
                left: 10.0,
                top: 0.0,
                right: 0.0,
                bottom: 2.0,
            }))
            .with_role(TextRole::Caption)
            .with_text(&attachment.status)
            .with_color(if attachment.quarantined {
                theme::STATUS_WARN
            } else {
                theme::TEXT_SECONDARY
            })
            .build();
            self.native_ui.add_node(status, column);

            // Declared properties.
            for field in &attachment.fields {
                match &field.kind {
                    ScriptFieldKind::Number { value, .. } => {
                        let row = crate::widgets::property_row::PropertyRowBuilder::new(
                            WidgetBuilder::new()
                                .with_clip_to_bounds(false)
                                .with_background(theme::TRANSPARENT),
                        )
                        .with_label(&field.name)
                        .build();
                        let row = self.native_ui.add_node(row, column);
                        let numeric = crate::widgets::numeric_field::NumericFieldBuilder::new(
                            WidgetBuilder::new().with_margin(Thickness::axes(0.0, 1.0)),
                        )
                        .with_drag_step(0.05)
                        .build();
                        let numeric = self.native_ui.add_node(numeric, row);
                        self.native_ui
                            .send(NumericFieldMessage::set_value(numeric, *value));
                        self.script_widgets.push((
                            numeric,
                            ScriptWidgetAction::Number(index, field.name.clone()),
                        ));
                    }
                    ScriptFieldKind::Bool(on) => {
                        let check = crate::widgets::check_box::CheckBoxBuilder::new(
                            WidgetBuilder::new()
                                .with_height(theme::ROW_HEIGHT)
                                .with_margin(Thickness::axes(10.0, 0.0)),
                        )
                        .with_label(&field.name)
                        .with_checked(*on)
                        .with_font_id(font_id)
                        .with_font_size(12.0)
                        .build();
                        let check = self.native_ui.add_node(check, column);
                        self.script_widgets
                            .push((check, ScriptWidgetAction::Bool(index, field.name.clone())));
                    }
                    ScriptFieldKind::Text(text) => {
                        // Shown, not editable. A property kind the editor
                        // cannot author yet is better visible than absent:
                        // absent looks like the script failed to declare it.
                        let label = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
                            left: 10.0,
                            top: 2.0,
                            right: 0.0,
                            bottom: 2.0,
                        }))
                        .with_role(TextRole::Caption)
                        .with_text(format!("{}: {text}", field.name))
                        .build();
                        self.native_ui.add_node(label, column);
                    }
                }
            }
        }
        self.native_ui
            .invalidate_ancestors(self.inspector_handles.script_list);
    }

    /// Refresh the live schema-generated Details tree for one selection.
    pub fn update_generated_details(
        &mut self,
        entity: Option<somnium_ecs::Entity>,
        panels: Vec<GeneratedComponentPanel>,
    ) {
        let signature: Vec<_> = panels
            .iter()
            .flat_map(|panel| panel.rows.iter())
            .flat_map(|row| {
                let edits: Vec<_> = match row.editor {
                    PropertyEditorKind::Vec2 => (0..2).map(GeneratedEdit::Lane).collect(),
                    PropertyEditorKind::Vec3 | PropertyEditorKind::Vec4 => {
                        let n = if row.editor == PropertyEditorKind::Vec3 {
                            3
                        } else {
                            4
                        };
                        (0..n).map(GeneratedEdit::Lane).collect()
                    }
                    PropertyEditorKind::Euler => (0..3).map(GeneratedEdit::Euler).collect(),
                    // The element count is part of this editor's *shape*, not
                    // of its values: adding a spline point has to rebuild the
                    // rows, and a signature that ignored the length would
                    // refresh three boxes and never draw the fourth.
                    PropertyEditorKind::Collection => {
                        let len = match &row.value {
                            somnium_ecs::reflect::ReflectValue::Array(items) => items.len(),
                            _ => 0,
                        };
                        (0..=len)
                            .map(|index| GeneratedEdit::Element {
                                index: index as u16,
                                lane: 0,
                            })
                            .collect()
                    }
                    _ => vec![GeneratedEdit::Whole],
                };
                edits
                    .into_iter()
                    .map(move |edit| (row.component, row.field, edit))
            })
            .collect();
        let rebuild = self.generated_entity != entity || self.generated_signature != signature;
        if rebuild {
            if self.generated_root.is_some() {
                self.native_ui.remove_node(self.generated_root);
            }
            self.generated_bindings.clear();
            self.generated_rows.clear();
            self.generated_gestures.clear();
            self.generated_asset_choices.clear();
            self.generated_asset_searches.clear();
            self.generated_asset_actions.clear();
            self.generated_collection_actions.clear();
            let (
                root,
                bindings,
                rows,
                asset_choices,
                asset_searches,
                asset_actions,
                collection_actions,
            ) = build_generated_details(
                &mut self.native_ui,
                self.inspector_stack,
                self.font_id,
                &panels,
                &self.asset_db,
            );
            self.generated_root = root;
            self.generated_bindings = bindings;
            self.generated_rows = rows;
            self.generated_asset_choices = asset_choices;
            self.generated_asset_searches = asset_searches;
            self.generated_asset_actions = asset_actions;
            self.generated_collection_actions = collection_actions;
            self.generated_entity = entity;
            self.generated_signature = signature;
            self.native_ui.invalidate_ancestors(self.inspector_stack);
        }

        let values: HashMap<_, _> = panels
            .iter()
            .flat_map(|panel| panel.rows.iter())
            .map(|row| {
                (
                    (row.component, row.field),
                    (row.value.clone(), row.default.clone()),
                )
            })
            .collect();
        for (handle, binding) in &mut self.generated_bindings {
            let Some((value, default)) = values.get(&(binding.component, binding.field)) else {
                continue;
            };
            binding.value = value.clone();
            binding.default = default.clone();
            match (&binding.value, binding.edit) {
                (somnium_ecs::reflect::ReflectValue::Bool(value), GeneratedEdit::Whole) => self
                    .native_ui
                    .send(CheckBoxMessage::set_checked(*handle, *value)),
                (somnium_ecs::reflect::ReflectValue::I64(value), GeneratedEdit::Whole) => self
                    .native_ui
                    .send(NumericFieldMessage::set_value(*handle, *value as f32)),
                (somnium_ecs::reflect::ReflectValue::F64(value), GeneratedEdit::Whole) => self
                    .native_ui
                    .send(NumericFieldMessage::set_value(*handle, *value as f32)),
                (somnium_ecs::reflect::ReflectValue::Vec2(value), GeneratedEdit::Lane(lane)) => {
                    self.native_ui.send(NumericFieldMessage::set_value(
                        *handle,
                        value[lane as usize],
                    ))
                }
                (somnium_ecs::reflect::ReflectValue::Vec3(value), GeneratedEdit::Lane(lane)) => {
                    self.native_ui.send(NumericFieldMessage::set_value(
                        *handle,
                        value[lane as usize],
                    ))
                }
                (somnium_ecs::reflect::ReflectValue::Vec4(value), GeneratedEdit::Lane(lane)) => {
                    self.native_ui.send(NumericFieldMessage::set_value(
                        *handle,
                        value[lane as usize],
                    ))
                }
                (
                    somnium_ecs::reflect::ReflectValue::Array(items),
                    GeneratedEdit::Element { index, lane },
                ) => {
                    if let Some(value) = element_lane(items, index, lane) {
                        self.native_ui
                            .send(NumericFieldMessage::set_value(*handle, value));
                    }
                }
                (somnium_ecs::reflect::ReflectValue::Curve(curve), GeneratedEdit::Whole) => self
                    .native_ui
                    .send(CurveEditorMessage::set_value(*handle, curve.clone())),
                (somnium_ecs::reflect::ReflectValue::Gradient(gradient), GeneratedEdit::Whole) => {
                    self.native_ui
                        .send(GradientEditorMessage::set_value(*handle, gradient.clone()));
                }
                (somnium_ecs::reflect::ReflectValue::Quat(value), GeneratedEdit::Euler(lane)) => {
                    let e = glam::Quat::from_array(*value).to_euler(glam::EulerRot::XYZ);
                    self.native_ui.send(NumericFieldMessage::set_value(
                        *handle,
                        [e.0, e.1, e.2][lane as usize].to_degrees(),
                    ));
                }
                _ => {}
            }
        }
        for binding in self.generated_rows.values_mut() {
            if let Some((value, default)) = values.get(&(binding.component, binding.field)) {
                binding.value = value.clone();
                binding.default = default.clone();
            }
        }
    }

    /// Update selection visibility. Reflected fields are populated separately.
    /// `transform` is (translation, euler_degrees, scale)`.
    pub fn update_inspector(
        &mut self,
        entity: Option<somnium_ecs::Entity>,
        pos: Option<[f32; 3]>,
        rot_deg: Option<[f32; 3]>,
        scale: Option<[f32; 3]>,
    ) {
        let _ = (pos, rot_deg, scale);
        // Phase 27-G. An empty Details panel now says so. Before this it
        // rendered POSITION / ROTATION / SCALE at 0.000 next to a status bar
        // reading "No selection", which says "the selection sits at the origin"
        // rather than "there is no selection".
        self.selected_entity = entity;
        let has_selection = entity.is_some();
        self.native_ui
            .set_visibility(self.inspector_stack, has_selection);
        self.native_ui
            .set_visibility(self.details_empty, !has_selection);
    }

    pub fn update_profiler(&mut self, rows: Option<&[ProfilerRow]>) {
        let panel = self.profiler_panel;
        let Some(rows) = rows else {
            self.native_ui.set_visibility(panel, false);
            self.native_ui.send(TextMessage::set_text(
                self.profiler_toggle_lbl,
                "Profiler".to_string(),
            ));
            return;
        };
        self.native_ui.set_visibility(panel, true);
        self.native_ui.send(TextMessage::set_text(
            self.profiler_toggle_lbl,
            "Profiler".to_string(),
        ));
        let names = self.profiler_names.clone();
        let values = self.profiler_values.clone();
        for i in 0..names.len() {
            let (label, value) = match rows.get(i) {
                Some(row) => (
                    format!("{}{}", "   ".repeat(row.depth as usize), row.label),
                    row.value.clone(),
                ),
                None => (String::new(), String::new()),
            };
            self.native_ui.send(TextMessage::set_text(names[i], label));
            self.native_ui.send(TextMessage::set_text(values[i], value));
        }
    }

    /// Show or hide the Terrain section and refresh it (Phase 17C / XV-I / XV-Zeta).
    pub fn update_terrain_inspector(&mut self, values: Option<TerrainInspectorState>) {
        let h = &self.inspector_handles;
        let (section, layer, tile) = (h.terrain_section, h.terrain_layer, h.terrain_tile);
        let relief = h.terrain_relief;
        let wetness = h.terrain_wetness;
        let macro_s = h.terrain_macro;
        let debug = h.terrain_debug;
        match values {
            Some(v) => {
                self.native_ui.set_visibility(section, true);
                self.native_ui
                    .send(NumericFieldMessage::set_value(layer, v.paint_layer));
                self.native_ui
                    .send(NumericFieldMessage::set_value(tile, v.tile));
                self.native_ui
                    .send(NumericFieldMessage::set_value(relief, v.relief));
                self.native_ui
                    .send(NumericFieldMessage::set_value(wetness, v.wetness));
                self.native_ui
                    .send(NumericFieldMessage::set_value(macro_s, v.macro_strength));
                self.native_ui
                    .send(NumericFieldMessage::set_value(debug, v.debug_view));
                self.native_ui.send(NumericFieldMessage::set_value(
                    h.terrain_morph_start,
                    v.morph_start,
                ));
                let paint =
                    (v.paint_layer.round().max(0.0) as usize).min(TERRAIN_LAYER_SHORT.len() - 1);
                let brush = (v.brush as usize).min(TERRAIN_BRUSH_NAMES.len() - 1);
                self.native_ui.send(CheckBoxMessage::set_checked(
                    h.terrain_paint_toggle,
                    v.terrain_paint && !v.foliage_paint,
                ));
                self.native_ui.send(CheckBoxMessage::set_checked(
                    h.terrain_hex_toggle,
                    v.hex_tiling,
                ));
                self.native_ui.send(CheckBoxMessage::set_checked(
                    h.terrain_parallax_toggle,
                    v.parallax,
                ));
                self.native_ui.send(CheckBoxMessage::set_checked(
                    h.terrain_clipmap_toggle,
                    v.clipmap,
                ));
                self.native_ui.send(CheckBoxMessage::set_checked(
                    h.terrain_morph_toggle,
                    v.lod_morph,
                ));
                for (i, label) in h.terrain_palette_labels.iter().enumerate() {
                    self.native_ui.send(TextMessage::set_text(
                        *label,
                        TERRAIN_LAYER_SHORT[i].to_string(),
                    ));
                }
                for (i, &btn) in h.terrain_palette.iter().enumerate() {
                    self.native_ui
                        .send(ButtonMessage::set_selected(btn, i == paint));
                }
                let active_brush = if v.terrain_edit { Some(brush) } else { None };
                for &(btn, lbl, tool) in &h.terrain_brush_items {
                    let name = TERRAIN_BRUSH_NAMES[tool as usize];
                    self.native_ui
                        .send(TextMessage::set_text(lbl, name.to_string()));
                    self.native_ui.send(ButtonMessage::set_selected(
                        btn,
                        active_brush == Some(tool as usize),
                    ));
                }
                for &(btn, lbl, tool) in &self.terrain_tool_items {
                    let name = TERRAIN_BRUSH_NAMES[tool as usize];
                    self.native_ui
                        .send(TextMessage::set_text(lbl, name.to_string()));
                    self.native_ui.send(ButtonMessage::set_selected(
                        btn,
                        active_brush == Some(tool as usize),
                    ));
                }
            }
            None => self.native_ui.set_visibility(section, false),
        }
    }

    /// Show the stable authoring subset of a first-class water body.

    /// Show or hide the Foliage section and refresh it (Phase 17C).
    ///
    /// `values` is `[density, seed, max_slope_deg, layer, scale_min, scale_max]`
    /// plus the enable flag.
    pub fn update_foliage_inspector(&mut self, values: Option<([f32; 10], [bool; 4])>) {
        let h = &self.inspector_handles;
        let section = h.foliage_section;
        let fields = [
            h.foliage_density,
            h.foliage_seed,
            h.foliage_slope,
            h.foliage_layer,
            h.foliage_smin,
            h.foliage_smax,
        ];
        match values {
            Some((v, flags)) => {
                self.native_ui.set_visibility(section, true);
                for (f, val) in fields.iter().zip(v[..6].iter()) {
                    self.native_ui
                        .send(NumericFieldMessage::set_value(*f, *val));
                }
                let toggles = [
                    h.foliage_toggle,
                    h.foliage_paint_toggle,
                    h.foliage_erase_toggle,
                    h.foliage_single_toggle,
                ];
                for (handle, on) in toggles.iter().zip(flags.iter()) {
                    self.native_ui
                        .send(CheckBoxMessage::set_checked(*handle, *on));
                }
                let kind = (v[3].round().max(0.0) as usize).min(FOLIAGE_KIND_NAMES.len() - 1);
                self.foliage_kind_shown = kind as u8;
                self.native_ui
                    .send(ComboBoxMessage::set_selected(h.foliage_kind_button, kind));
            }
            None => self.native_ui.set_visibility(section, false),
        }
    }

    /// Append a line to the Output Log.
    ///
    /// The ring buffer is still 200 entries, but pinned lines are exempt: a
    /// line you deliberately kept should not be evicted by two hundred lines
    /// of import chatter arriving behind it.
    pub fn append_log(&mut self, text: &str) {
        self.log
            .append(self.log_clock.elapsed().as_secs_f64(), text);
        self.rebuild_log_rows();
    }

    /// The Output Log's state.
    #[must_use]
    pub fn log(&self) -> &crate::log::OutputLog {
        &self.log
    }

    /// The undo history, for the History view: the entry names in order, and
    /// how many of them have been applied.
    pub fn set_history(&mut self, entries: Vec<String>, position: usize) {
        if self.history_rows == entries && self.history_position == position {
            return;
        }
        self.history_rows = entries;
        self.history_position = position;
        if self.log_view == LogView::History {
            self.rebuild_log_rows();
        }
    }

    /// Background jobs, for the Jobs view. `(id, label, progress, failed)`.
    pub fn set_job_rows(&mut self, rows: Vec<(u64, String, f32, bool)>) {
        if self.job_rows == rows {
            return;
        }
        self.job_rows = rows;
        if self.log_view == LogView::Jobs {
            self.rebuild_log_rows();
        }
    }

    /// Show the first error, opening the panel and filtering to errors.
    ///
    /// This is what the status bar's "N script errors" clicks through to. It
    /// filters rather than merely scrolling, because a lone error thirty lines
    /// up in a busy log is not findable by scrolling to it.
    pub fn reveal_first_error(&mut self) {
        self.log_view = LogView::Log;
        self.log.reveal_errors();
        if !self.log_open {
            self.toggle_log_panel();
        }
        self.rebuild_log_rows();
    }

    /// Rebuild the panel from the model.
    ///
    /// Wholesale rather than incrementally: the list is bounded at 200 rows,
    /// a filter change invalidates all of them anyway, and an incremental path
    /// would be a second place for the view and the model to disagree.
    fn rebuild_log_rows(&mut self) {
        for (row, _) in std::mem::take(&mut self.log_rows) {
            self.native_ui.remove_node(row);
        }
        let font_id = self.font_id;
        let stack = self.log_stack;
        let mut rows = Vec::new();

        if self.log_view == LogView::History {
            // Row 0 is the state before anything happened, so the list is one
            // longer than the entry list. Marking the current position rather
            // than only the last row is what makes this a *history* instead of
            // a list of things that already happened.
            let entries = self.history_rows.clone();
            let position = self.history_position;
            for index in 0..=entries.len() {
                let label = if index == 0 {
                    "\u{2014} before any change \u{2014}".to_string()
                } else {
                    format!("{index}. {}", entries[index - 1])
                };
                let current = index == position;
                let button = ButtonBuilder::new(
                    WidgetBuilder::new()
                        .with_height(15.0)
                        .with_background(theme::TRANSPARENT),
                )
                .build();
                let button = self.native_ui.add_node(button, stack);
                let marker = if current { "\u{25b8} " } else { "  " };
                let node =
                    TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(8.0, 1.0)))
                        .with_text(&format!("{marker}{label}"))
                        .with_font_size(11.0)
                        .with_font_id(font_id)
                        .with_color(if current {
                            theme::active().semantic.accent.default.bytes()
                        } else if index > position {
                            // Everything past the marker is redo: still there, but not
                            // part of the current state.
                            theme::TEXT_DISABLED
                        } else {
                            theme::TEXT_PRIMARY
                        })
                        .build();
                self.native_ui.add_node(node, button);
                rows.push((button, index as u64));
            }
        } else if self.log_view == LogView::Jobs {
            for (id, label, progress, failed) in self.job_rows.clone() {
                let text = if failed {
                    format!("{label} — failed")
                } else if progress >= 1.0 {
                    format!("{label} — done")
                } else {
                    format!("{label} — {:.0}%", progress * 100.0)
                };
                let node =
                    TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(8.0, 1.0)))
                        .with_text(&text)
                        .with_font_size(11.0)
                        .with_font_id(font_id)
                        .with_color(if failed {
                            theme::active().semantic.status.error.bytes()
                        } else {
                            theme::TEXT_SECONDARY
                        })
                        .build();
                rows.push((self.native_ui.add_node(node, stack), id));
            }
        } else {
            let visible: Vec<_> = self.log.visible().into_iter().cloned().collect();
            for entry in visible {
                // A line carrying a source reference is a button, so it can be
                // clicked; a line without one is text, so it cannot pretend to
                // be. Craft: a link that does nothing teaches people not to
                // click links.
                let colour = match entry.severity {
                    crate::log::LogSeverity::Error => theme::active().semantic.status.error.bytes(),
                    crate::log::LogSeverity::Warn => {
                        theme::active().semantic.status.warning.bytes()
                    }
                    crate::log::LogSeverity::Debug => theme::TEXT_DISABLED,
                    crate::log::LogSeverity::Info => theme::TEXT_PRIMARY,
                };
                let label = format!("{:>7.2}s  {}", entry.timestamp, entry.text);
                let row = if entry.sources.is_empty() {
                    let node = TextBuilder::new(
                        WidgetBuilder::new().with_margin(Thickness::axes(8.0, 1.0)),
                    )
                    .with_text(&label)
                    .with_font_size(11.0)
                    .with_font_id(font_id)
                    .with_color(colour)
                    .build();
                    self.native_ui.add_node(node, stack)
                } else {
                    let button = ButtonBuilder::new(
                        WidgetBuilder::new()
                            .with_height(15.0)
                            .with_tooltip(&format!(
                                "Open {}:{}",
                                entry.sources[0].file, entry.sources[0].line
                            ))
                            .with_background(theme::TRANSPARENT),
                    )
                    .build();
                    let button = self.native_ui.add_node(button, stack);
                    let node = TextBuilder::new(
                        WidgetBuilder::new().with_margin(Thickness::axes(8.0, 1.0)),
                    )
                    .with_text(&label)
                    .with_font_size(11.0)
                    .with_font_id(font_id)
                    .with_color(theme::active().semantic.accent.default.bytes())
                    .build();
                    self.native_ui.add_node(node, button);
                    button
                };
                rows.push((row, entry.id));
            }
        }

        let empty = rows.is_empty();
        self.log_rows = rows;
        self.native_ui.set_visibility(self.log_empty, empty);
        self.native_ui.set_visibility(self.log_stack, !empty);
        for (chip, severity) in self.log_severity_chips.clone() {
            self.native_ui.send(ButtonMessage::set_selected(
                chip,
                self.log.filter.severities.contains(&severity),
            ));
        }
        self.native_ui.send(ButtonMessage::set_selected(
            self.log_pin_only,
            self.log.filter.pinned_only,
        ));
        self.native_ui.send(ButtonMessage::set_selected(
            self.log_jobs_toggle,
            self.log_view == LogView::Jobs,
        ));
        self.native_ui.send(ButtonMessage::set_selected(
            self.log_history_toggle,
            self.log_view == LogView::History,
        ));
        self.native_ui.invalidate_ancestors(self.log_stack);
    }

    /// Everything the Output Log's toolbar and rows do with a message.
    fn handle_log_message(&mut self, msg: &UiMessage) -> bool {
        if msg.destination == self.log_search
            && let Some(SearchBoxMessage::Query(query)) = msg.data::<SearchBoxMessage>()
        {
            self.log.filter.search = query.clone();
            self.rebuild_log_rows();
            return true;
        }
        if !matches!(msg.data::<ButtonMessage>(), Some(ButtonMessage::Click)) {
            return false;
        }
        if let Some((_, severity)) = self
            .log_severity_chips
            .iter()
            .find(|(chip, _)| *chip == msg.destination)
            .copied()
        {
            self.log.filter.toggle(severity);
            self.rebuild_log_rows();
            return true;
        }
        if msg.destination == self.status_stats_button {
            self.reveal_first_error();
            return true;
        }
        if msg.destination == self.log_pin_only {
            self.log.filter.pinned_only = !self.log.filter.pinned_only;
            self.rebuild_log_rows();
            return true;
        }
        if msg.destination == self.log_jobs_toggle {
            self.log_view = if self.log_view == LogView::Jobs {
                LogView::Log
            } else {
                LogView::Jobs
            };
            self.rebuild_log_rows();
            return true;
        }
        if msg.destination == self.log_history_toggle {
            self.log_view = if self.log_view == LogView::History {
                LogView::Log
            } else {
                LogView::History
            };
            self.rebuild_log_rows();
            return true;
        }
        if self.log_view == LogView::History
            && let Some(index) = self
                .log_rows
                .iter()
                .position(|(row, _)| *row == msg.destination)
        {
            // Row 0 is "before anything happened", so the click target is the
            // position *after* the entry named on the row — which is what
            // "take me back to there" means when you click the row itself.
            self.editor_events
                .push_back(EditorEvent::JumpToHistory(index));
            return true;
        }
        if msg.destination == self.log_clear {
            // Pinned lines survive Clear. That is the whole point of a pin.
            self.log.clear();
            self.rebuild_log_rows();
            return true;
        }
        if msg.destination == self.log_copy {
            let text = self.log.copy_text();
            self.editor_events.push_back(EditorEvent::CopyText(text));
            self.push_toast("Copied the visible log");
            return true;
        }
        // A row with a source reference. `command()`-click pins instead of
        // opening, so the two useful things a log line can do are one gesture
        // apart rather than behind a menu.
        if let Some((_, id)) = self
            .log_rows
            .iter()
            .find(|(row, _)| *row == msg.destination)
            .copied()
        {
            if self.native_ui.modifiers().command() {
                self.log.toggle_pin(id);
                self.rebuild_log_rows();
                return true;
            }
            if let Some(source) = self.log.source_of(id).cloned() {
                self.editor_events.push_back(EditorEvent::OpenSource {
                    file: source.file,
                    line: source.line,
                    column: source.column.unwrap_or(1),
                });
            }
            return true;
        }
        false
    }

    // ── Internal ─────────────────────────────────────────────────────────────

    fn process_outgoing(&mut self, msgs: Vec<UiMessage>) {
        let h = &self.inspector_handles;
        let terrain_numeric = [
            (h.terrain_aerial_dist, TerrainToolField::AerialDistance),
            (h.terrain_layer, TerrainToolField::PaintLayer),
            (h.terrain_tile, TerrainToolField::TileScale),
            (h.terrain_relief, TerrainToolField::Relief),
            (h.terrain_wetness, TerrainToolField::Wetness),
            (h.terrain_macro, TerrainToolField::MacroStrength),
            (h.terrain_debug, TerrainToolField::DebugView),
            (h.terrain_morph_start, TerrainToolField::MorphStart),
        ];
        let foliage_numeric = [
            (h.foliage_density, FoliageBrushField::Density),
            (h.foliage_seed, FoliageBrushField::Radius),
            (h.foliage_slope, FoliageBrushField::MaxSlope),
            (h.foliage_layer, FoliageBrushField::Kind),
            (h.foliage_smin, FoliageBrushField::ScaleMin),
            (h.foliage_smax, FoliageBrushField::ScaleMax),
        ];
        for msg in msgs {
            if msg.destination == self.animation_timeline.editor
                && let Some(crate::timeline::TimelineEditorMessage::Changed(document)) =
                    msg.data::<crate::timeline::TimelineEditorMessage>()
            {
                self.animation_timeline_document = document.clone();
                self.set_scene_dirty(true);
                continue;
            }
            if matches!(
                msg.data::<crate::graph::GraphEditorMessage>(),
                Some(
                    crate::graph::GraphEditorMessage::Changed(_)
                        | crate::graph::GraphEditorMessage::StateMachineChanged(_)
                )
            ) {
                self.set_scene_dirty(true);
                continue;
            }
            if self.handle_preferences_message(&msg) {
                continue;
            }
            if self.handle_snap_message(&msg) {
                continue;
            }
            if self.handle_log_message(&msg) {
                continue;
            }
            if matches!(msg.data::<ButtonMessage>(), Some(ButtonMessage::Click))
                && let Some((_, path)) = self
                    .recent_menu_items
                    .iter()
                    .find(|(handle, path)| *handle == msg.destination && !path.is_empty())
                    .cloned()
            {
                self.close_all_menus();
                self.prompt_unsaved_action(EditorEvent::LoadScene(path));
                continue;
            }
            if let Some(ColorSwatchMessage::Clicked(rgba)) = msg.data::<ColorSwatchMessage>() {
                let generated_target =
                    self.generated_bindings
                        .get(&msg.destination)
                        .and_then(|binding| match binding.value {
                            somnium_ecs::reflect::ReflectValue::Vec3(_) => {
                                Some(ColorTarget::Reflected {
                                    component: binding.component,
                                    field: binding.field,
                                    vec4: false,
                                })
                            }
                            somnium_ecs::reflect::ReflectValue::Vec4(_) => {
                                Some(ColorTarget::Reflected {
                                    component: binding.component,
                                    field: binding.field,
                                    vec4: true,
                                })
                            }
                            _ => None,
                        });
                if let Some(field) = generated_target {
                    self.color_target = Some(field);
                    self.color_gesture = Some(self.allocate_property_gesture());
                    self.color_open = true;
                    self.color_original = *rgba;
                    self.color_live = *rgba;
                    self.native_ui.send(UiMessage::new(
                        self.color_picker,
                        MessageDirection::ToWidget,
                        ColorPickerMessage::SetColor(*rgba),
                    ));
                    self.native_ui.send(UiMessage::new(
                        self.color_popup,
                        MessageDirection::ToWidget,
                        PopupMessage::SetAnchor(msg.destination),
                    ));
                    self.native_ui.send(UiMessage::new(
                        self.color_popup,
                        MessageDirection::ToWidget,
                        PopupMessage::Open,
                    ));
                    self.native_ui.invalidate_ancestors(self.color_popup);
                }
                continue;
            }
            if let Some(cmsg) = msg.data::<ColorPickerMessage>() {
                if let (Some(target), Some(gesture)) = (self.color_target, self.color_gesture) {
                    match cmsg {
                        ColorPickerMessage::Changing(rgba) => {
                            self.color_live = *rgba;
                            self.write_color_target(target, *rgba, gesture, true);
                        }
                        ColorPickerMessage::Changed(rgba) => {
                            self.color_live = *rgba;
                            self.write_color_target(target, *rgba, gesture, false);
                            self.dismiss_color_ui();
                        }
                        ColorPickerMessage::Cancelled(rgba) => {
                            self.color_original = *rgba;
                            self.write_color_target(target, *rgba, gesture, false);
                            self.dismiss_color_ui();
                        }
                        ColorPickerMessage::SetColor(_) => {}
                    }
                }
                continue;
            }
            if let Some(CommandPaletteMessage::Run(id)) = msg.data::<CommandPaletteMessage>() {
                self.close_palette();
                // Close (and restore focus from) the palette before running a
                // command which may open another modal, such as New Scene's
                // unsaved-changes prompt. Modal focus scopes never overlap.
                self.run_command_id(id);
                continue;
            }
            if let Some(SplitterMessage::Changed(size)) = msg.data::<SplitterMessage>() {
                if msg.destination == self.inner_h {
                    self.chrome_layout.tools = *size;
                } else if msg.destination == self.content_split_h {
                    // The splitter reports the viewport pane; persist the
                    // *column* it implies, because that is what survives a
                    // change of window size.
                    self.chrome_layout.viewport = *size;
                    let available = self.window_size.0 as f32 - self.chrome_layout.tools - 12.0;
                    self.chrome_layout.details = (available - *size).max(0.0);
                } else if msg.destination == self.right_split_h {
                    self.chrome_layout.outliner = *size;
                }
                crate::layout_persist::save(self.chrome_layout);
                continue;
            }
            if let Some(ButtonMessage::DoubleClick) = msg.data::<ButtonMessage>() {
                // The tile's children — icon, label, badge — are separate
                // nodes, and a double-click that lands on one of them arrives
                // addressed to *it*, not to the tile. Matching only the tile
                // handle meant a double-click on the icon, which is most of
                // the tile's area, did nothing at all. `is_under` is the same
                // ancestor walk the drag arm and the context menu already use.
                if let Some((_, entry)) = self
                    .content_entries
                    .iter()
                    .find(|(bh, _)| {
                        *bh == msg.destination || self.native_ui.is_under(msg.destination, *bh)
                    })
                    .cloned()
                {
                    self.open_content_entry(&entry);
                }
                continue;
            }
            if let Some(ButtonMessage::Click) = msg.data::<ButtonMessage>() {
                if msg.destination == self.status_cancel {
                    if let Some(id) = self.status_cancel_job {
                        self.editor_events.push_back(EditorEvent::CancelJob(id));
                    }
                    continue;
                }
                if let Some((_, action)) = self
                    .content_toolbar_actions
                    .iter()
                    .find(|(handle, _)| *handle == msg.destination)
                    .copied()
                {
                    match action {
                        ContentToolbarAction::Back => {
                            self.content_back();
                        }
                        ContentToolbarAction::Forward => {
                            self.content_forward();
                        }
                        ContentToolbarAction::Up => {
                            let up = std::path::Path::new(&self.content_path)
                                .parent()
                                .map(|path| path.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            self.navigate_content(up);
                        }
                        ContentToolbarAction::Kind(kind) => self.set_content_kind(kind),
                        ContentToolbarAction::Sort => {
                            self.content_sort = match self.content_sort {
                                somnium_asset::database::AssetSort::Name => {
                                    somnium_asset::database::AssetSort::Kind
                                }
                                somnium_asset::database::AssetSort::Kind => {
                                    somnium_asset::database::AssetSort::Size
                                }
                                somnium_asset::database::AssetSort::Size => {
                                    somnium_asset::database::AssetSort::Modified
                                }
                                somnium_asset::database::AssetSort::Modified => {
                                    somnium_asset::database::AssetSort::Name
                                }
                            };
                            self.refresh_content_list();
                        }
                        ContentToolbarAction::Density => {
                            self.cycle_content_density();
                        }
                    }
                    continue;
                }
                if let Some((component, field, action)) = self
                    .generated_collection_actions
                    .get(&msg.destination)
                    .copied()
                {
                    // Add, duplicate and remove all rebuild the whole array
                    // and send it down the ordinary field-write path, so each
                    // is one undo step and the serializer, the multi-select
                    // fan-out and the script boundary need to know nothing
                    // about collections.
                    let current = self
                        .generated_rows
                        .values()
                        .find(|binding| binding.component == component && binding.field == field)
                        .map(|binding| binding.value.clone());
                    if let Some(current) = current
                        && let Some(next) = Self::collection_result(&current, action)
                        && let Some(entity) = self.selected_entity
                    {
                        let gesture = self.allocate_property_gesture();
                        self.editor_events
                            .push_back(EditorEvent::SetComponentField {
                                entity,
                                component,
                                field,
                                value: next,
                                gesture,
                                live: false,
                            });
                    }
                    continue;
                }
                if let Some((combo, action)) =
                    self.generated_asset_actions.get(&msg.destination).copied()
                {
                    let binding = self.generated_bindings.get(&combo).cloned();
                    let record = binding.as_ref().and_then(|binding| match binding.value {
                        somnium_ecs::reflect::ReflectValue::Asset(Some(asset)) => self
                            .asset_db
                            .get(somnium_asset::database::AssetId::from_raw(asset.raw()))
                            .cloned(),
                        _ => None,
                    });
                    match (action, binding, record) {
                        (AssetPickerAction::UseDrawerSelection, Some(binding), _) => {
                            let chosen = self
                                .content_entries
                                .iter()
                                .map(|(_, entry)| entry)
                                .find(|entry| {
                                    !entry.is_dir && self.content_selection.contains(&entry.path)
                                })
                                .and_then(|entry| entry.asset_id);
                            match (chosen, self.selected_entity) {
                                (Some(asset), Some(entity)) => {
                                    let accepted = self.asset_db.get(asset).is_some_and(|record| {
                                        record.kind.bit() & binding.asset_kind_mask != 0
                                    });
                                    if accepted {
                                        let gesture = self.allocate_property_gesture();
                                        self.editor_events.push_back(
                                            EditorEvent::SetComponentField {
                                                entity,
                                                component: binding.component,
                                                field: binding.field,
                                                value: somnium_ecs::reflect::ReflectValue::Asset(
                                                    Some(somnium_ecs::reflect::AssetRef::from_raw(
                                                        asset.raw(),
                                                    )),
                                                ),
                                                gesture,
                                                live: false,
                                            },
                                        );
                                    } else {
                                        self.push_toast(
                                            "This field does not accept that kind of asset",
                                        );
                                    }
                                }
                                (None, _) => {
                                    self.push_toast("Select a file in the Content Drawer first")
                                }
                                (_, None) => self.push_toast("Select an entity first"),
                            }
                        }
                        (AssetPickerAction::Locate, _, Some(record)) => {
                            self.navigate_content(record.parent);
                        }
                        (AssetPickerAction::Edit, _, Some(record)) => {
                            self.editor_events.push_back(EditorEvent::EditContentAsset(
                                record.absolute_path.to_string_lossy().into_owned(),
                            ));
                        }
                        (AssetPickerAction::MakeUnique, Some(binding), Some(record)) => {
                            if let Some(entity) = self.selected_entity {
                                self.editor_events.push_back(EditorEvent::MakeAssetUnique {
                                    source: record.absolute_path.to_string_lossy().into_owned(),
                                    entity,
                                    component: binding.component,
                                    field: binding.field,
                                });
                            }
                        }
                        _ => self.push_toast("Choose an asset first"),
                    }
                    continue;
                }
                if msg.destination == self.unsaved_save {
                    self.close_unsaved();
                    self.editor_events.push_back(EditorEvent::SaveScene);
                    if let Some(action) = self.pending_scene_action.take() {
                        self.editor_events.push_back(action);
                    }
                    continue;
                }
                if msg.destination == self.unsaved_discard {
                    self.close_unsaved();
                    if let Some(action) = self.pending_scene_action.take() {
                        self.editor_events.push_back(action);
                    }
                    continue;
                }
                if msg.destination == self.unsaved_cancel {
                    self.close_unsaved();
                    self.pending_scene_action = None;
                    continue;
                }
                if msg.destination == self.name_ok {
                    self.confirm_name_prompt();
                    continue;
                }
                if msg.destination == self.name_cancel {
                    self.close_name_prompt();
                    continue;
                }
            }
            // Phase 16-D: the drawer's right-click menu, and the name
            // prompt the three creating flows share.
            if let Some(ContextMenuMessage::Activate(id)) = msg.data::<ContextMenuMessage>() {
                if msg.destination == self.content_menu {
                    self.activate_content_menu(id);
                    continue;
                }
                if msg.destination == self.outliner_menu {
                    self.close_outliner_menu();
                    if let Some(index) = id.strip_prefix("pierce:")
                        && let Ok(index) = index.parse::<u32>()
                    {
                        self.piercing_rows.clear();
                        self.editor_events
                            .push_back(EditorEvent::PickPierced(index));
                        continue;
                    }
                    self.run_command_id(id);
                    continue;
                }
            }
            if let Some(text) = msg.data::<TextBoxMessage>() {
                if msg.destination == self.name_input {
                    match text {
                        TextBoxMessage::TextChanged(value) => self.name_text = value.clone(),
                        // Enter in the box is the same as pressing Create:
                        // typing a name and hitting return is what anyone
                        // does, and making them reach for the mouse after
                        // would be a small daily annoyance.
                        TextBoxMessage::TextCommit(value) => {
                            self.name_text = value.clone();
                            if self.name_prompt.is_some() {
                                self.confirm_name_prompt();
                            }
                        }
                    }
                    continue;
                }
            }
            if let Some(PopupMessage::Close) = msg.data::<PopupMessage>() {
                if msg.destination == self.color_popup && self.color_open {
                    // Click-away commits (Iris: OK = click-outside).
                    self.close_color_picker(true);
                }
                if msg.destination == self.palette_popup {
                    self.close_palette();
                }
                if msg.destination == self.unsaved_popup {
                    self.close_unsaved();
                }
                if msg.destination == self.name_popup {
                    // Clicking away from the prompt abandons it, the same
                    // as Cancel.
                    self.close_name_prompt();
                }
            }

            // A committed cell edit in the Localisation grid. Outside the
            // button block on purpose: a `DataGridMessage` is not a
            // `ButtonMessage`, and a handler nested in there would never run.
            if let Some(crate::widgets::data_grid::DataGridMessage::Edited { table, .. }) =
                msg.data::<crate::widgets::data_grid::DataGridMessage>()
            {
                // A commit, not a keystroke: this is the state a save writes.
                self.locale_table = Some((**table).clone());
                continue;
            }

            if let Some(ButtonMessage::Click) = msg.data::<ButtonMessage>() {
                if let Some((_, id)) = self
                    .menu_command_items
                    .iter()
                    .find(|(handle, _)| *handle == msg.destination)
                    .copied()
                {
                    if let Some(opener) = self.open_menu_button() {
                        let previous = self.native_ui.focused();
                        if previous.is_some() && previous != opener {
                            self.native_ui.send(UiMessage::new(
                                previous,
                                MessageDirection::ToWidget,
                                WidgetMessage::Unfocus,
                            ));
                        }
                        self.native_ui.set_focus(opener);
                        self.native_ui.send(UiMessage::new(
                            opener,
                            MessageDirection::ToWidget,
                            WidgetMessage::Focus,
                        ));
                    }
                    self.close_all_menus();
                    self.run_command_id(id);
                    continue;
                }
                // Outliner row
                if msg.destination == self.palette_button {
                    self.toggle_palette();
                    continue;
                }
                if let Some(&(_, eidx)) = self
                    .outliner_rows
                    .iter()
                    .find(|(bh, _)| *bh == msg.destination)
                {
                    self.editor_events.push_back(EditorEvent::ModifySelection {
                        id: eidx,
                        mode: self.selection_mode(),
                    });
                    continue;
                }
                if let Some(layer) = self
                    .inspector_handles
                    .terrain_palette
                    .iter()
                    .position(|&h| h == msg.destination)
                {
                    self.editor_events
                        .push_back(EditorEvent::SetTerrainPaintLayer(layer as u8));
                    continue;
                }
                // Post-process controls are checkboxes now. Handling their
                // underlying button-click messages as well as Check messages
                // delivered two events for one click and flipped the setting
                // straight back to its starting value.
                if msg.destination == self.profiler_toggle {
                    self.run_command_id("editor.view.profiler");
                    continue;
                }
                if msg.destination == self.play_button {
                    self.run_command_id("editor.simulation.play");
                    continue;
                }
                if msg.destination == self.select_button {
                    self.run_command_id("editor.gizmo.translate");
                    continue;
                }
                if msg.destination == self.landscape_button {
                    self.run_command_id("editor.terrain.edit");
                    continue;
                }
                if msg.destination == self.foliage_toolbar_button {
                    self.run_command_id("editor.foliage.edit");
                    continue;
                }
                if msg.destination == self.immersive_button {
                    self.run_command_id("editor.viewport.immersive");
                    continue;
                }
                if msg.destination == self.step_button {
                    self.run_command_id("editor.simulation.step");
                    continue;
                }
                if msg.destination == self.pause_button {
                    self.run_command_id("editor.simulation.pause");
                    continue;
                }
                if msg.destination == self.stop_button {
                    self.run_command_id("editor.simulation.stop");
                    continue;
                }
                if msg.destination == self.inspector_handles.foliage_toggle {
                    self.editor_events.push_back(EditorEvent::ToggleFoliage);
                    continue;
                }
                if msg.destination == self.inspector_handles.foliage_paint_toggle {
                    self.editor_events
                        .push_back(EditorEvent::ToggleFoliagePaint);
                    continue;
                }
                if msg.destination == self.inspector_handles.terrain_paint_toggle {
                    self.editor_events
                        .push_back(EditorEvent::ToggleTerrainPaint);
                    continue;
                }
                if msg.destination == self.inspector_handles.terrain_hex_toggle {
                    self.editor_events.push_back(EditorEvent::ToggleTerrainHex);
                    continue;
                }
                if msg.destination == self.inspector_handles.terrain_parallax_toggle {
                    self.editor_events
                        .push_back(EditorEvent::ToggleTerrainParallax);
                    continue;
                }
                if msg.destination == self.inspector_handles.terrain_clipmap_toggle {
                    self.editor_events
                        .push_back(EditorEvent::ToggleTerrainClipmap);
                    continue;
                }
                if msg.destination == self.inspector_handles.terrain_morph_toggle {
                    self.editor_events
                        .push_back(EditorEvent::ToggleTerrainMorph);
                    continue;
                }
                if msg.destination == self.inspector_handles.foliage_erase_toggle {
                    self.editor_events
                        .push_back(EditorEvent::ToggleFoliageErase);
                    continue;
                }
                if msg.destination == self.inspector_handles.foliage_single_toggle {
                    self.editor_events
                        .push_back(EditorEvent::ToggleFoliageSingle);
                    continue;
                }
                if let Some(&(_, _, tool)) = self
                    .inspector_handles
                    .terrain_brush_items
                    .iter()
                    .find(|(bh, _, _)| *bh == msg.destination)
                {
                    self.editor_events
                        .push_back(EditorEvent::SetTerrainTool(tool));
                    continue;
                }
                // Terrain tool button (Phase 14F)
                if let Some(&(_, _, tool)) = self
                    .terrain_tool_items
                    .iter()
                    .find(|(bh, _, _)| *bh == msg.destination)
                {
                    self.editor_events
                        .push_back(EditorEvent::SetTerrainTool(tool));
                    continue;
                }
                if msg.destination == self.help_button {
                    self.close_all_menus();
                    self.run_command_id("editor.help.index");
                    continue;
                }
                if msg.destination == self.log_button {
                    self.toggle_log_panel();
                    continue;
                }
                if msg.destination == self.references_button {
                    self.toggle_references_panel();
                    continue;
                }
                if msg.destination == self.locale_button {
                    self.toggle_locale_panel();
                    continue;
                }
                if let Some(action) = self
                    .locale_actions
                    .iter()
                    .find(|(handle, _)| *handle == msg.destination)
                    .map(|(_, action)| *action)
                {
                    match action {
                        LocaleAction::Save => {
                            self.editor_events.push_back(EditorEvent::SaveLocalisation);
                        }
                        LocaleAction::ExportCsv => {
                            self.editor_events
                                .push_back(EditorEvent::ExportLocalisationCsv);
                        }
                    }
                    continue;
                }
                // Walking the graph is the point of the panel: a row is a
                // link, so clicking one asks the same three questions about
                // what it names. A row for an asset that is not in the project
                // is not a link to anywhere.
                if let Some((_, asset)) = self
                    .references_rows
                    .iter()
                    .find(|(row, _)| *row == msg.destination)
                    .copied()
                {
                    if self.asset_db.get(asset).is_some() {
                        self.show_references_for(asset);
                    }
                    continue;
                }
                if msg.destination == self.drawer_button {
                    self.toggle_drawer();
                    continue;
                }
                if msg.destination == self.win_min {
                    self.window.set_minimized(true);
                    continue;
                }
                if msg.destination == self.win_max {
                    self.window.set_maximized(!self.window.is_maximized());
                    continue;
                }
                if msg.destination == self.win_close {
                    self.editor_events.push_back(EditorEvent::CloseWindow);
                    continue;
                }
                if let Some(&(_, page)) = self.help_toc.iter().find(|(h, _)| *h == msg.destination)
                {
                    self.set_help_page(page);
                    continue;
                }
                if msg.destination == self.help_close {
                    self.toggle_help(None);
                    continue;
                }
                if msg.destination == self.save_button {
                    self.run_command_id("editor.scene.save");
                    continue;
                }
                if let Some((_, entry)) = self
                    .content_entries
                    .iter()
                    .find(|(bh, _)| *bh == msg.destination)
                    .cloned()
                {
                    if entry.is_dir {
                        let root = self.asset_db.root().to_path_buf();
                        // Through `navigate_content` so the move lands in
                        // history. Setting `content_path` directly is what would
                        // leave Back pointing at a folder the user never left.
                        match entry.path.strip_prefix(&root) {
                            Ok(rel) => {
                                let rel = rel.to_string_lossy().into_owned();
                                self.navigate_content(rel);
                            }
                            Err(_) => self.refresh_content_list(),
                        }
                    } else if entry.is_engine {
                        if let Some(kind) = match entry.name.as_str() {
                            "Cube" => Some(CreateKind::Cube),
                            "Sphere" => Some(CreateKind::Sphere),
                            "Plane" => Some(CreateKind::Plane),
                            "Cylinder" => Some(CreateKind::Cylinder),
                            _ => None,
                        } {
                            self.editor_events
                                .push_back(EditorEvent::CreateEntity(kind));
                        }
                    } else {
                        let additive = self.native_ui.modifiers().ctrl;
                        self.select_content(entry.path.clone(), additive);
                        if entry
                            .path
                            .extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|e| e.eq_ignore_ascii_case("luau"))
                        {
                            self.editor_events.push_back(EditorEvent::AttachScript(
                                entry.path.to_string_lossy().into_owned(),
                            ));
                        }
                    }
                    continue;
                }
                // Phase 16-D: the Scripts section's generated controls.
                if let Some((_, action)) = self
                    .script_widgets
                    .iter()
                    .find(|(handle, _)| *handle == msg.destination)
                    .cloned()
                {
                    match action {
                        ScriptWidgetAction::Reorder(index, delta) => {
                            self.editor_events
                                .push_back(EditorEvent::ReorderScript { index, delta });
                        }
                        ScriptWidgetAction::Detach(index) => {
                            self.editor_events
                                .push_back(EditorEvent::DetachScript(index));
                        }
                        // Enable and the property widgets are not buttons;
                        // they arrive as CheckBox and NumericField messages.
                        ScriptWidgetAction::Enable(_)
                        | ScriptWidgetAction::Number(_, _)
                        | ScriptWidgetAction::Bool(_, _) => {}
                    }
                    continue;
                }
                if msg.destination == self.inspector_handles.script_add {
                    self.editor_events.push_back(EditorEvent::CreateScript);
                    continue;
                }
                // Create popup item
                if let Some(&(_, id)) = self
                    .create_popup_items
                    .iter()
                    .find(|(bh, _)| *bh == msg.destination)
                {
                    self.run_command_id(id);
                    self.create_popup_open = false;
                    self.native_ui.send(UiMessage::new(
                        self.create_popup,
                        MessageDirection::ToWidget,
                        PopupMessage::Close,
                    ));
                    self.native_ui.invalidate_ancestors(self.create_popup);
                    continue;
                }
            } else if let Some(MenuMessage::Click) = msg.data::<MenuMessage>() {
                if msg.destination == self.file_button {
                    if self.file_popup_open {
                        self.close_all_menus();
                    } else {
                        self.open_menu(0);
                    }
                    continue;
                }
                if msg.destination == self.create_button {
                    if self.create_popup_open {
                        self.close_all_menus();
                    } else {
                        self.open_menu(1);
                    }
                    continue;
                }
                if msg.destination == self.edit_button {
                    if self.edit_popup_open {
                        self.close_all_menus();
                    } else {
                        self.open_menu(2);
                    }
                    continue;
                }
                if msg.destination == self.view_button {
                    if self.view_popup_open {
                        self.close_all_menus();
                    } else {
                        self.open_menu(3);
                    }
                    continue;
                }
                if msg.destination == self.window_button {
                    if self.window_popup_open {
                        self.close_all_menus();
                    } else {
                        self.open_menu(4);
                    }
                    continue;
                }
                if msg.destination == self.help_menu_button {
                    if self.help_menu_open {
                        self.close_all_menus();
                    } else {
                        self.open_menu(5);
                    }
                    continue;
                }
            } else if let Some(PopupMessage::Close) = msg.data::<PopupMessage>() {
                if msg.destination == self.file_popup {
                    self.file_popup_open = false;
                    self.native_ui.invalidate_ancestors(self.file_popup);
                }
                if msg.destination == self.create_popup {
                    self.create_popup_open = false;
                    self.native_ui.invalidate_ancestors(self.create_popup);
                }
                if msg.destination == self.edit_popup {
                    self.edit_popup_open = false;
                }
                if msg.destination == self.view_popup {
                    self.view_popup_open = false;
                }
                if msg.destination == self.window_popup {
                    self.window_popup_open = false;
                }
                if msg.destination == self.help_menu_popup {
                    self.help_menu_open = false;
                }
                if msg.destination == self.help_overlay {
                    self.help_open = false;
                }
                if let Some(combo) = self.combo_for_popup(msg.destination) {
                    self.native_ui.send(ComboBoxMessage::close(combo));
                    self.open_combo_popup = NodeHandle::NONE;
                    self.native_ui.invalidate_ancestors(msg.destination);
                }
            } else if let Some(CheckBoxMessage::Check(on)) = msg.data::<CheckBoxMessage>() {
                // ToWidget messages are state synchronization from the engine,
                // not user intent. Treating them as clicks caused every
                // inspector refresh to mutate the component again.
                if msg.direction != MessageDirection::FromWidget {
                    continue;
                }
                if msg.destination == self.content_engine_toggle {
                    self.show_engine_content = !self.show_engine_content;
                    self.refresh_content_list();
                    continue;
                }
                if msg.destination == self.locale_incomplete {
                    self.locale_only_incomplete = !self.locale_only_incomplete;
                    self.native_ui.send(UiMessage::new(
                        self.locale_grid,
                        MessageDirection::ToWidget,
                        crate::widgets::data_grid::DataGridMessage::SetOnlyIncomplete(
                            self.locale_only_incomplete,
                        ),
                    ));
                    continue;
                }
                // Phase 16-D: an attachment's enable box, or one of its
                // declared boolean properties.
                if let Some((_, action)) = self
                    .script_widgets
                    .iter()
                    .find(|(handle, _)| *handle == msg.destination)
                    .cloned()
                {
                    match action {
                        ScriptWidgetAction::Enable(index) => {
                            self.editor_events.push_back(EditorEvent::SetScriptEnabled {
                                index,
                                enabled: *on,
                            });
                        }
                        ScriptWidgetAction::Bool(index, field) => {
                            self.editor_events.push_back(EditorEvent::SetScriptBool {
                                index,
                                field,
                                value: *on,
                            });
                        }
                        _ => {}
                    }
                    continue;
                }
                if msg.destination == self.inspector_handles.foliage_toggle {
                    self.editor_events.push_back(EditorEvent::ToggleFoliage);
                    continue;
                }
                if msg.destination == self.inspector_handles.foliage_paint_toggle {
                    self.editor_events
                        .push_back(EditorEvent::ToggleFoliagePaint);
                    continue;
                }
                if msg.destination == self.inspector_handles.terrain_paint_toggle {
                    self.editor_events
                        .push_back(EditorEvent::ToggleTerrainPaint);
                    continue;
                }
                if msg.destination == self.inspector_handles.terrain_hex_toggle {
                    self.editor_events.push_back(EditorEvent::ToggleTerrainHex);
                    continue;
                }
                if msg.destination == self.inspector_handles.terrain_parallax_toggle {
                    self.editor_events
                        .push_back(EditorEvent::ToggleTerrainParallax);
                    continue;
                }
                if msg.destination == self.inspector_handles.terrain_clipmap_toggle {
                    self.editor_events
                        .push_back(EditorEvent::ToggleTerrainClipmap);
                    continue;
                }
                if msg.destination == self.inspector_handles.terrain_aerial_toggle {
                    if msg.direction == MessageDirection::FromWidget {
                        self.editor_events
                            .push_back(EditorEvent::SetTerrainAerial(*on));
                    }
                    continue;
                }
                if msg.destination == self.inspector_handles.terrain_aerial_hero_toggle {
                    if msg.direction == MessageDirection::FromWidget {
                        self.editor_events
                            .push_back(EditorEvent::SetTerrainAerialHeroBank(*on));
                    }
                    continue;
                }
                if msg.destination == self.inspector_handles.post_census_toggle {
                    if msg.direction == MessageDirection::FromWidget {
                        self.editor_events
                            .push_back(EditorEvent::SetPixelCensus(*on));
                    }
                    continue;
                }
                if msg.destination == self.inspector_handles.post_bins_toggle {
                    if msg.direction == MessageDirection::FromWidget {
                        self.editor_events.push_back(EditorEvent::SetShadeBins(*on));
                    }
                    continue;
                }
                if msg.destination == self.inspector_handles.terrain_morph_toggle {
                    self.editor_events
                        .push_back(EditorEvent::ToggleTerrainMorph);
                    continue;
                }
                if msg.destination == self.inspector_handles.foliage_erase_toggle {
                    self.editor_events
                        .push_back(EditorEvent::ToggleFoliageErase);
                    continue;
                }
                if msg.destination == self.inspector_handles.foliage_single_toggle {
                    self.editor_events
                        .push_back(EditorEvent::ToggleFoliageSingle);
                    continue;
                }
            } else if let Some(ComboBoxMessage::SelectionChanged(i)) = msg.data::<ComboBoxMessage>()
            {
                if msg.destination == self.inspector_handles.foliage_kind_button
                    || msg.destination == self.foliage_kind_combo
                {
                    self.editor_events
                        .push_back(EditorEvent::SelectFoliageKind(*i as u8));
                    continue;
                }
                if msg.destination == self.viewport_res_combo {
                    self.editor_events
                        .push_back(EditorEvent::SetViewportResolution(*i as u8));
                    continue;
                }
            } else if let Some(ComboBoxMessage::Open) = msg.data::<ComboBoxMessage>() {
                self.close_all_menus();
                if let Some(popup) = self.combo_popup_for(msg.destination) {
                    if self.open_combo_popup.is_some() && self.open_combo_popup != popup {
                        let other = self.open_combo_popup;
                        for (combo, p) in self.combo_entries() {
                            if p == other {
                                self.native_ui.send(ComboBoxMessage::close(combo));
                                self.native_ui.send(UiMessage::new(
                                    p,
                                    MessageDirection::ToWidget,
                                    PopupMessage::Close,
                                ));
                            }
                        }
                    }
                    self.open_combo_popup = popup;
                    self.native_ui.invalidate_ancestors(popup);
                }
                continue;
            } else if let Some(ComboBoxMessage::Close) = msg.data::<ComboBoxMessage>() {
                if self.combo_popup_for(msg.destination) == Some(self.open_combo_popup)
                    || msg.destination == self.open_combo_popup
                {
                    self.open_combo_popup = NodeHandle::NONE;
                }
                continue;
            } else if let Some(TreeViewMessage::Select(id)) = msg.data::<TreeViewMessage>() {
                if msg.destination == self.outliner_tree {
                    self.editor_events.push_back(EditorEvent::ModifySelection {
                        id: *id,
                        mode: self.selection_mode(),
                    });
                }
            } else if let Some(TreeViewMessage::ToggleBadge { id, lock }) =
                msg.data::<TreeViewMessage>()
            {
                if msg.destination == self.outliner_tree {
                    // A drag down the column sets every row it crosses to the
                    // value the first click produced, rather than toggling each
                    // one — otherwise dragging over an already-hidden row would
                    // un-hide it and the gesture would read as noise.
                    let value = self.badge_drag_value.get_or_insert_with(|| {
                        self.last_outliner_state
                            .as_ref()
                            .and_then(|(rows, _)| rows.iter().find(|row| row.id == *id))
                            .map(|row| !if *lock { row.locked } else { row.hidden })
                            .unwrap_or(true)
                    });
                    self.editor_events.push_back(EditorEvent::ToggleEntityFlag {
                        entity: *id,
                        lock: *lock,
                        value: Some(*value),
                    });
                }
            } else if let Some(TreeViewMessage::ToggleExpand(id)) = msg.data::<TreeViewMessage>() {
                if self.outliner_expanded.contains(id) {
                    self.outliner_expanded.remove(id);
                } else {
                    self.outliner_expanded.insert(*id);
                }
                self.last_outliner_state = None;
            } else if let Some(SearchBoxMessage::Query(q)) = msg.data::<SearchBoxMessage>() {
                if let Some(picker) = self.generated_asset_searches.get(&msg.destination).copied() {
                    use crate::editor::property_editors::AssetEditorContext;
                    let candidates = AssetEditorContext::query(&self.asset_db, q, picker.kind_mask);
                    let mut labels = vec!["None".to_string()];
                    labels.extend(candidates.iter().map(|candidate| candidate.label.clone()));
                    let mut choices = vec![None];
                    choices.extend(candidates.iter().map(|candidate| Some(candidate.id)));
                    let mut paths = vec![None];
                    paths.extend(candidates.iter().map(|candidate| {
                        self.asset_db
                            .get(somnium_asset::database::AssetId::from_raw(
                                candidate.id.raw(),
                            ))
                            .map(|record| record.absolute_path.clone())
                    }));
                    self.generated_asset_choices.insert(picker.combo, choices);
                    self.native_ui.send(UiMessage::new(
                        picker.combo,
                        MessageDirection::ToWidget,
                        ComboBoxMessage::SetItems(labels.clone()),
                    ));
                    self.native_ui.send(UiMessage::new(
                        picker.list,
                        MessageDirection::ToWidget,
                        ComboBoxMessage::SetItems(labels),
                    ));
                    self.native_ui.send(UiMessage::new(
                        picker.list,
                        MessageDirection::ToWidget,
                        ComboBoxMessage::SetAssetPaths(paths.clone()),
                    ));
                    for path in paths.into_iter().flatten().take(8) {
                        self.native_ui.draw_ctx.thumbnails.request(&path, true);
                    }
                    continue;
                }
                if msg.destination == self.content_search {
                    self.content_filter = q.clone();
                    self.refresh_content_list();
                }
                if msg.destination == self.locale_search {
                    // Straight through to the grid: the filter is the view's,
                    // and the view belongs to the widget that draws it.
                    self.native_ui.send(UiMessage::new(
                        self.locale_grid,
                        MessageDirection::ToWidget,
                        crate::widgets::data_grid::DataGridMessage::SetFilter(q.clone()),
                    ));
                }
                if msg.destination == self.outliner_search {
                    self.outliner_filter = q.clone();
                    self.last_outliner_state = None;
                }
                if msg.destination == self.inspector_search {
                    self.inspector_filter = q.clone();
                    self.apply_inspector_filter();
                }
            } else if let Some(BreadcrumbMessage::Navigate(i)) = msg.data::<BreadcrumbMessage>() {
                if msg.destination == self.content_breadcrumb {
                    let target = if *i == 0 {
                        String::new()
                    } else {
                        let parts: Vec<&str> = self.content_path.split(SEP).collect();
                        parts.iter().take(*i).copied().collect::<Vec<_>>().join("/")
                    };
                    // A crumb click is navigation too, so it belongs in history.
                    self.navigate_content(target);
                }
            }

            // — Camera speed slider (Phase 20B), day scrub (CONTROL-L) ————
            if let Some(smsg) = msg.data::<SliderMessage>() {
                if msg.destination == self.camera_speed_slider {
                    if let SliderMessage::Value(v) = smsg {
                        self.editor_events
                            .push_back(EditorEvent::SetCameraSpeed(*v));
                    }
                } else if msg.destination == self.time_slider {
                    // `Value` while dragging, `Committed` once at the end:
                    // every intermediate hour is applied so the light moves
                    // under the cursor, and exactly one undo entry is recorded.
                    match smsg {
                        SliderMessage::Value(v) => {
                            self.editor_events.push_back(EditorEvent::SetTimeOfDayHour {
                                hour: v * 24.0,
                                live: true,
                            });
                        }
                        SliderMessage::Committed(v) => {
                            self.editor_events.push_back(EditorEvent::SetTimeOfDayHour {
                                hour: v * 24.0,
                                live: false,
                            });
                        }
                        SliderMessage::SetValue(_) => {}
                    }
                }
            }

            // — Gutter dot: revert one property ————————
            //
            // The row emits the request; the editor answers it by writing the
            // baseline back through the ordinary value path. That means revert
            // costs exactly one `ValueChanged`, which is one undo step, and it
            // needs no new `EditorEvent` variant.
            if matches!(
                msg.data::<PropertyRowMessage>(),
                Some(PropertyRowMessage::RevertRequested)
            ) {
                if let Some(binding) = self.generated_rows.get(&msg.destination).cloned() {
                    let gesture = self.allocate_property_gesture();
                    self.queue_generated_binding(&binding, binding.default.clone(), gesture, false);
                }
            }

            // — NumericField value changes ————————
            let numeric = match msg.data::<NumericFieldMessage>() {
                Some(NumericFieldMessage::ValueChanged(v)) => Some((*v, false)),
                Some(NumericFieldMessage::ValueChanging(v)) => Some((*v, true)),
                _ => None,
            };
            if let Some((v, live)) = numeric {
                if let Some(binding) = self.generated_bindings.get(&msg.destination).cloned() {
                    let gesture = if live {
                        if let Some(gesture) =
                            self.generated_gestures.get(&msg.destination).copied()
                        {
                            gesture
                        } else {
                            let gesture = self.allocate_property_gesture();
                            self.generated_gestures.insert(msg.destination, gesture);
                            gesture
                        }
                    } else {
                        self.generated_gestures
                            .remove(&msg.destination)
                            .unwrap_or_else(|| self.allocate_property_gesture())
                    };
                    if let Some(value) = Self::numeric_reflect_value(&binding, v) {
                        self.queue_generated_binding(&binding, value, gesture, live);
                    }
                    continue;
                }
                if let Some((_, field)) = terrain_numeric
                    .iter()
                    .find(|(handle, _)| *handle == msg.destination)
                {
                    self.editor_events
                        .push_back(EditorEvent::SetTerrainToolValue {
                            field: *field,
                            value: v,
                            live,
                        });
                    continue;
                }
                if let Some((_, field)) = foliage_numeric
                    .iter()
                    .find(|(handle, _)| *handle == msg.destination)
                {
                    self.editor_events
                        .push_back(EditorEvent::SetFoliageBrushValue {
                            field: *field,
                            value: v,
                            live,
                        });
                    continue;
                }
                // Phase 16-D: a script's declared numeric property. Same
                // live/commit convention as every other inspector field —
                // a drag is applied and not recorded, and the gesture's
                // final value is one undo step.
                if let Some((_, ScriptWidgetAction::Number(index, field))) = self
                    .script_widgets
                    .iter()
                    .find(|(handle, _)| *handle == msg.destination)
                    .cloned()
                {
                    self.editor_events.push_back(EditorEvent::SetScriptNumber {
                        index,
                        field,
                        value: v,
                        live,
                    });
                }
            }

            // — CONTROL-K: curve and gradient edits ————————
            //
            // Both follow the drag-scrub convention exactly as `NumericField`
            // does above: a live gesture reuses one `GestureId` so the whole
            // drag coalesces into a single undo entry, and the commit consumes
            // it. Nothing here knows what a keyframe is; the value that
            // travels is the whole `ReflectValue::Curve`.
            if let Some(CurveEditorMessage::Value { curve, live }) =
                msg.data::<CurveEditorMessage>()
            {
                if let Some(binding) = self.generated_bindings.get(&msg.destination).cloned() {
                    let gesture = self.gesture_for(msg.destination, *live);
                    self.queue_generated_binding(
                        &binding,
                        somnium_ecs::reflect::ReflectValue::Curve(curve.clone()),
                        gesture,
                        *live,
                    );
                }
                continue;
            }
            if let Some(gmsg) = msg.data::<GradientEditorMessage>() {
                match gmsg {
                    GradientEditorMessage::Value { gradient, live } => {
                        if let Some(binding) =
                            self.generated_bindings.get(&msg.destination).cloned()
                        {
                            let gesture = self.gesture_for(msg.destination, *live);
                            self.queue_generated_binding(
                                &binding,
                                somnium_ecs::reflect::ReflectValue::Gradient(gradient.clone()),
                                gesture,
                                *live,
                            );
                        }
                    }
                    GradientEditorMessage::StopActivated { index, color } => {
                        if let Some(binding) =
                            self.generated_bindings.get(&msg.destination).cloned()
                        {
                            self.color_target = Some(ColorTarget::GradientStop {
                                component: binding.component,
                                field: binding.field,
                                index: *index,
                            });
                            self.color_gesture = Some(self.allocate_property_gesture());
                            self.color_open = true;
                            self.color_original = *color;
                            self.color_live = *color;
                            self.native_ui.send(UiMessage::new(
                                self.color_picker,
                                MessageDirection::ToWidget,
                                ColorPickerMessage::SetColor(*color),
                            ));
                            self.native_ui.send(UiMessage::new(
                                self.color_popup,
                                MessageDirection::ToWidget,
                                PopupMessage::SetAnchor(msg.destination),
                            ));
                            self.native_ui.send(UiMessage::new(
                                self.color_popup,
                                MessageDirection::ToWidget,
                                PopupMessage::Open,
                            ));
                            self.native_ui.invalidate_ancestors(self.color_popup);
                        }
                    }
                    GradientEditorMessage::SetValue(_) => {}
                }
                continue;
            }
            if let Some(CheckBoxMessage::Check(value)) = msg.data::<CheckBoxMessage>() {
                if let Some(binding) = self.generated_bindings.get(&msg.destination).cloned() {
                    let gesture = self.allocate_property_gesture();
                    self.queue_generated_binding(
                        &binding,
                        somnium_ecs::reflect::ReflectValue::Bool(*value),
                        gesture,
                        false,
                    );
                    continue;
                }
            }
            if let Some(TextBoxMessage::TextCommit(value)) = msg.data::<TextBoxMessage>() {
                if self
                    .content_inline_rename
                    .as_ref()
                    .is_some_and(|(handle, _)| *handle == msg.destination)
                {
                    let (_, path) = self.content_inline_rename.take().expect("checked above");
                    self.editor_events
                        .push_back(EditorEvent::RenameContentItem {
                            path: path.to_string_lossy().into_owned(),
                            name: value.clone(),
                        });
                    self.native_ui.remove_node(msg.destination);
                    continue;
                }
                if let Some(binding) = self.generated_bindings.get(&msg.destination).cloned() {
                    let gesture = self.allocate_property_gesture();
                    self.queue_generated_binding(
                        &binding,
                        somnium_ecs::reflect::ReflectValue::Str(value.clone()),
                        gesture,
                        false,
                    );
                    continue;
                }
            }
            if let Some(ComboBoxMessage::SelectionChanged(index)) = msg.data::<ComboBoxMessage>() {
                if let Some(binding) = self.generated_bindings.get(&msg.destination).cloned() {
                    let gesture = self.allocate_property_gesture();
                    if let Some(choice) = self
                        .generated_asset_choices
                        .get(&msg.destination)
                        .and_then(|choices| choices.get(*index))
                        .copied()
                    {
                        self.queue_generated_binding(
                            &binding,
                            somnium_ecs::reflect::ReflectValue::Asset(choice),
                            gesture,
                            false,
                        );
                        continue;
                    }
                    self.queue_generated_binding(
                        &binding,
                        somnium_ecs::reflect::ReflectValue::I64(*index as i64),
                        gesture,
                        false,
                    );
                    continue;
                }
            }
        }
    }
}

/// Which optional chrome survives at a given window width.
///
/// Kept as one pure function so the breakpoints are testable without a window
/// and so a reader can see the whole responsive policy at once, rather than
/// finding it spread across the widgets that obey it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollapseRules {
    /// The transport's play-state word ("Stopped"/"Playing") is shown beside
    /// the ▶ ❚❚ ■ triple. Redline §06: this is what drops at 1400, *not* the
    /// mode command names — Save / Select / Landscape / Foliage keep their
    /// labels all the way down to the 1280 compact case, because "icon-only
    /// controls" is a forbidden motif and the compact screen in the design
    /// package still shows them spelled out.
    pub transport_label: bool,
    /// The application-scope command-search field is shown.
    pub search_field: bool,
    /// The status bar shows the object count beside the frame rate.
    pub status_objects: bool,
    /// CONTROL-G, following Unreal 5.6: the floating viewport context bar
    /// keeps its snap cluster inline. Below the threshold the cluster moves
    /// into an overflow menu rather than clipping, because Zeta's 68 px
    /// pre-scene budget makes the bar genuinely too short at 1280 and a
    /// control that is half-drawn is worse than one behind a chevron.
    pub context_bar_snap_inline: bool,
}

impl CollapseRules {
    pub fn for_width(width: f32) -> Self {
        Self {
            transport_label: width >= 1400.0,
            search_field: width >= 1100.0,
            status_objects: width >= 1280.0,
            context_bar_snap_inline: width >= 1600.0,
        }
    }
}

#[cfg(test)]
mod elysium_tests {
    use super::*;

    #[test]
    fn shipped_animation_workspace_contains_both_retained_editors() {
        let mut ui = UserInterface::new(1280.0, 720.0);
        let layout =
            build_editor_layout(&mut ui, 0, crate::layout_persist::ChromeLayout::default());
        {
            let workspace = ui
                .nodes
                .try_borrow(layout.animation_workspace.transmute())
                .unwrap();
            assert_eq!(workspace.widget.parent, layout.viewport_handle);
            assert!(
                !workspace.widget.visibility,
                "Layout starts on the 3D viewport"
            );
        }

        {
            let graph = ui
                .nodes
                .try_borrow(layout.animation_graph_editor.transmute())
                .unwrap();
            assert_eq!(graph.widget.parent, layout.animation_workspace);
            assert!(
                graph.widget.visibility,
                "the composite parent exclusively owns workspace visibility"
            );
        }

        assert_eq!(
            ui.parent_of(layout.animation_timeline.editor),
            Some(layout.animation_workspace)
        );
        assert_eq!(
            ui.parent_of(layout.animation_timeline.curve_editor),
            Some(layout.animation_timeline.editor)
        );
        assert!(
            ui.nodes
                .try_borrow(layout.animation_timeline.editor.transmute())
                .unwrap()
                .widget
                .visibility
        );

        ui.set_visibility(layout.animation_workspace, true);
        ui.perform_layout();
        assert!(ui.is_globally_visible(layout.animation_graph_editor));
        assert!(ui.is_globally_visible(layout.animation_timeline.editor));
        assert!(ui.is_globally_visible(layout.animation_timeline.curve_editor));
        assert!(
            crate::commands::registry()
                .get("editor.workspace.animation")
                .is_some(),
            "the Window menu/palette can reveal the production graph control"
        );
    }

    #[test]
    fn details_shows_exactly_one_state_at_a_time() {
        // The screenshot defect: POSITION / ROTATION / SCALE rendered at 0.000
        // beside a status bar reading "No selection". Exactly one of the two
        // must be visible, at startup and after every selection change.
        let mut ui = UserInterface::new(1920.0, 1080.0);
        let font_id = load_fonts(&mut ui);
        let layout = build_editor_layout(
            &mut ui,
            font_id,
            crate::layout_persist::ChromeLayout::default(),
        );

        // The builder leaves both visible; `UiManager::new` seeds the empty
        // one, and `update_inspector` flips them from then on. Assert the
        // handles are distinct so a flip can never be a no-op.
        assert_ne!(
            layout.inspector_stack, layout.details_empty,
            "the two Details states must be separate nodes"
        );
        assert!(layout.details_empty.is_some(), "the empty state must exist");
    }

    #[test]
    fn every_registered_command_is_palette_and_help_discoverable() {
        let registry = crate::commands::registry();
        let palette_ids: std::collections::HashSet<_> = registry
            .commands()
            .iter()
            .map(|command| command.id)
            .collect();
        let help_ids: std::collections::HashSet<_> =
            registry.help_index().iter().map(|(id, _, _)| *id).collect();
        assert_eq!(palette_ids, help_ids);
        assert!(
            palette_ids.len() > 15,
            "CONTROL-A2 must replace the old shortlist"
        );
    }

    #[test]
    fn empty_state_copy_follows_plain_speech() {
        // phase_27 §13: sentence case bodies ending in a period, no exclamation
        // marks, no "Please", no "Oops", and an action phrased as an action.
        use crate::metaphor::empty;
        for state in [
            empty::OUTLINER,
            empty::DETAILS,
            empty::CONTENT,
            empty::CONTENT_FILTERED,
            empty::LOG,
        ] {
            let head = state.headline;
            let body = state.body;
            let action = state.action;
            assert!(!head.is_empty() && !body.is_empty() && !action.is_empty());
            assert!(body.ends_with('.'), "body must be a sentence: {body}");
            assert!(!head.ends_with('.'), "headline is a label: {head}");
            assert!(!action.ends_with('.'), "action is a label: {action}");
            for text in [head, body, action] {
                assert!(!text.contains('!'), "no exclamation marks: {text}");
                assert!(!text.contains("Please"), "no Please: {text}");
                assert!(!text.contains("Oops"), "no Oops: {text}");
            }
        }
    }

    #[test]
    fn an_empty_folder_and_a_filtered_miss_give_different_advice() {
        // Offering "import a model" to someone who mistyped a search would be
        // the wrong instruction.
        use crate::metaphor::empty;
        assert_ne!(empty::CONTENT.headline, empty::CONTENT_FILTERED.headline);
        assert_ne!(empty::CONTENT.action, empty::CONTENT_FILTERED.action);
    }

    #[test]
    fn the_content_drawer_shows_an_empty_state_when_it_has_nothing() {
        // Built against a directory that cannot contain assets, so the drawer
        // is genuinely empty rather than incidentally so.
        let mut ui = UserInterface::new(1920.0, 1080.0);
        let font_id = load_fonts(&mut ui);
        let parent = ui.root();
        assert!(
            ui.first_child(parent).is_none(),
            "the fixture root should start empty"
        );

        let column = crate::editor::parts::build_empty_state(
            &mut ui,
            parent,
            font_id,
            crate::metaphor::empty::CONTENT,
        );
        assert!(column.is_some(), "the empty state must build a container");
        assert_eq!(
            ui.first_child(parent),
            column,
            "and attach it to the panel it was asked to fill"
        );

        // Mark, headline, body and action: four children, none optional.
        ui.perform_layout();
        ui.draw();
        assert!(
            ui.draw_ctx.instance_count() > 0,
            "an empty state that draws nothing is the bug it exists to prevent"
        );
    }

    #[test]
    fn the_drawer_canvas_carries_the_whole_folders_height_and_scrolls_by_it() {
        // MORROWIND-M. The two facts the windowed drawer rests on, neither of
        // which is visible from `GridWindow`'s own tests:
        //
        //   1. a canvas given an explicit height makes the scroll viewer above
        //      it scrollable to that height, even with a screenful of children;
        //   2. the canvas's screen `y` is the content origin the window reads,
        //      and it moves by exactly the scroll.
        //
        // Get either wrong and the drawer builds the right number of tiles for
        // the wrong part of the folder.
        let mut ui = UserInterface::new(1600.0, 900.0);
        let font_id = load_fonts(&mut ui);
        let layout = build_editor_layout(
            &mut ui,
            font_id,
            crate::layout_persist::ChromeLayout::default(),
        );
        ui.perform_layout();

        // 40,000 assets at the comfortable density, four rows of which exist.
        let (tile_w, tile_h, _) = crate::metaphor::ContentDensity::Comfortable.metrics();
        let pitch = (tile_w + CONTENT_GAP, tile_h + CONTENT_GAP);
        let canvas_before = ui.screen_bounds(layout.content_list);
        let window = crate::virtual_list::GridWindow::new(
            canvas_before.y,
            pitch,
            canvas_before.w + CONTENT_GAP,
            40_000,
            ui.screen_bounds(layout.content_scroll),
        );
        ui.set_height(layout.content_list, window.content_height(pitch.1));
        let mut tiles = Vec::new();
        for index in window.range() {
            let cell = window.tile_rect(index, pitch);
            let tile = ui.add_node(
                ButtonBuilder::new(WidgetBuilder::new()).build(),
                layout.content_list,
            );
            ui.place_node(
                tile,
                crate::types::Rect::new(cell.x, cell.y, tile_w, tile_h),
            );
            tiles.push((index, tile));
        }
        ui.perform_layout();

        assert!(
            !tiles.is_empty() && tiles.len() < 200,
            "a screenful, not a folder: {}",
            tiles.len()
        );
        let first = ui.screen_bounds(tiles[0].1);
        assert!(
            (first.y - canvas_before.y).abs() < 0.5,
            "the first tile should sit at the top of the canvas, not {first:?}"
        );

        // Scroll a hundred rows down. The canvas rises by exactly that much,
        // which is what tells the next window which rows are now in view.
        let scroll = 100.0 * pitch.1;
        ui.send(UiMessage::new(
            layout.content_scroll,
            MessageDirection::ToWidget,
            WidgetMessage::MouseWheel {
                pos: {
                    let b = ui.screen_bounds(layout.content_scroll);
                    glam::Vec2::new(b.x + b.w * 0.5, b.y + b.h * 0.5)
                },
                // Negative is downward: the viewer subtracts the delta.
                delta: -scroll,
                mods: Modifiers::default(),
            },
        ));
        let _ = ui.update();
        ui.perform_layout();

        let canvas_after = ui.screen_bounds(layout.content_list);
        assert!(
            (canvas_before.y - canvas_after.y - scroll).abs() < 1.0,
            "the canvas moved by {} rather than {scroll}",
            canvas_before.y - canvas_after.y
        );
        let moved = crate::virtual_list::GridWindow::new(
            canvas_after.y,
            pitch,
            canvas_after.w + CONTENT_GAP,
            40_000,
            ui.screen_bounds(layout.content_scroll),
        );
        // Row 100 is the one the scroll landed on. The window must never
        // start *below* it — that is a band of empty drawer along the top —
        // and must not start far above it either, or the saving is spent on
        // tiles nobody can see. The slack is the overscan row plus the one the
        // canvas's own margin straddles.
        let first_row = moved.first / moved.columns;
        assert!(
            first_row <= 100 && first_row + 2 >= 100,
            "a hundred rows of scroll built from row {first_row}"
        );
    }

    #[test]
    fn an_empty_folder_does_not_crop_its_own_empty_state() {
        // MORROWIND-M, and the trap under it. The canvas is given an explicit
        // height so the scroll viewer knows how deep the folder is — but an
        // empty folder is nought rows deep. A canvas that cropped to its own
        // bounds would build the "this folder is empty" panel perfectly and
        // then crop it out of existence, which reads to the user exactly like
        // the blank grey rectangle the empty state exists to replace.
        let mut ui = UserInterface::new(1600.0, 900.0);
        let font_id = load_fonts(&mut ui);
        let layout = build_editor_layout(
            &mut ui,
            font_id,
            crate::layout_persist::ChromeLayout::default(),
        );
        ui.set_height(layout.content_list, 0.0);
        let column = crate::editor::parts::build_empty_state(
            &mut ui,
            layout.content_list,
            font_id,
            crate::metaphor::empty::CONTENT,
        );
        assert!(column.is_some(), "the empty state must build a container");
        ui.perform_layout();

        assert_eq!(
            ui.screen_bounds(layout.content_list).h,
            0.0,
            "fixture: an empty folder is nought rows tall"
        );
        let clip = ui.clip_bounds(column);
        assert!(
            clip.h > 0.0 && clip.w > 0.0,
            "the empty state was clipped away by its own container: {clip:?}"
        );
        // It is still clipped by the drawer, which is the clip that matters.
        let drawer = ui.screen_bounds(layout.content_scroll);
        assert!(clip.h <= drawer.h + 0.5, "{clip:?} escaped {drawer:?}");
    }

    #[test]
    fn setting_the_height_a_node_already_has_does_not_invalidate_layout() {
        // `sync_content_tiles` runs every frame. If the same height counted as
        // a change, an idle drawer would re-lay-out the whole shell forever.
        let mut ui = UserInterface::new(1600.0, 900.0);
        let font_id = load_fonts(&mut ui);
        let layout = build_editor_layout(
            &mut ui,
            font_id,
            crate::layout_persist::ChromeLayout::default(),
        );
        ui.set_height(layout.content_list, 4_000.0);
        ui.perform_layout();
        ui.set_height(layout.content_list, 4_000.0);
        assert!(ui.is_layout_valid(layout.content_list));
        // NaN is never equal to itself, so the same-value check has to say so.
        ui.set_height(layout.content_list, f32::NAN);
        ui.perform_layout();
        ui.set_height(layout.content_list, f32::NAN);
        assert!(ui.is_layout_valid(layout.content_list));
    }
}

#[cfg(test)]
mod dpi_tests {
    use super::*;

    fn shell(logical_w: f32, logical_h: f32, scale: f32) -> UserInterface {
        let mut ui = UserInterface::new(logical_w, logical_h);
        ui.set_ui_scale(scale);
        let font_id = load_fonts(&mut ui);
        let _ = build_editor_layout(
            &mut ui,
            font_id,
            crate::layout_persist::ChromeLayout::default(),
        );
        ui.perform_layout();
        ui
    }

    fn bounds(ui: &UserInterface, handle: NodeHandle) -> crate::types::Rect {
        ui.nodes
            .try_borrow(handle.transmute())
            .expect("layout handle should remain valid")
            .widget
            .screen_bounds()
    }

    #[test]
    fn pointer_positions_convert_from_device_pixels_to_layout_units() {
        let mut ui = UserInterface::new(1280.0, 720.0);
        ui.set_ui_scale(2.0);
        assert_eq!(ui.to_logical(200.0, 100.0), glam::Vec2::new(100.0, 50.0));
        ui.set_ui_scale(1.0);
        assert_eq!(ui.to_logical(200.0, 100.0), glam::Vec2::new(200.0, 100.0));
    }

    #[test]
    fn layout_is_identical_at_every_scale_for_the_same_logical_size() {
        // The whole point of the fix: a density token means the same apparent
        // size at 100 %, 150 % and 200 %. Before Phase 27 the tree was fed
        // device pixels, so a 36 unit title bar shrank to a third of its
        // intended height at 300 %.
        let a = shell(1280.0, 720.0, 1.0);
        let b = shell(1280.0, 720.0, 2.0);
        let c = shell(1280.0, 720.0, 1.5);

        for (h_a, h_b, h_c) in [(a.root(), b.root(), c.root())] {
            assert_eq!(bounds(&a, h_a), bounds(&b, h_b));
            assert_eq!(bounds(&a, h_a), bounds(&c, h_c));
        }
        assert_eq!(a.screen_size, b.screen_size);
        assert_eq!(a.screen_size, c.screen_size);
    }

    #[test]
    fn the_pre_scene_budget_is_measured_in_logical_units() {
        // phase_26_Zeta redline: application 36 + mode 32 = 68 before the
        // viewport, and that number must now hold at any DPI.
        for scale in [1.0f32, 1.25, 1.5, 2.0] {
            let mut ui = UserInterface::new(1920.0, 1080.0);
            ui.set_ui_scale(scale);
            let font_id = load_fonts(&mut ui);
            let layout = build_editor_layout(
                &mut ui,
                font_id,
                crate::layout_persist::ChromeLayout::default(),
            );
            ui.perform_layout();
            let viewport = bounds(&ui, layout.viewport_handle);
            assert!(
                (viewport.y - 68.0).abs() < 0.1,
                "scale {scale}: viewport starts at {} not 68",
                viewport.y
            );
        }
    }

    #[test]
    fn a_click_at_a_device_pixel_hits_the_widget_drawn_there() {
        // The round trip that actually matters: the OS reports a pointer in
        // device pixels, the tree hit-tests in logical units, and the two must
        // agree or every control is offset at HiDPI.
        let mut ui = UserInterface::new(1920.0, 1080.0);
        ui.set_ui_scale(2.0);
        let font_id = load_fonts(&mut ui);
        let layout = build_editor_layout(
            &mut ui,
            font_id,
            crate::layout_persist::ChromeLayout::default(),
        );
        ui.perform_layout();

        let save = bounds(&ui, layout.save_button);
        assert!(save.w > 0.0 && save.h > 0.0, "save button must be laid out");

        // Centre of the Save button, expressed the way winit would report it.
        let device_x = (save.x + save.w * 0.5) as f64 * 2.0;
        let device_y = (save.y + save.h * 0.5) as f64 * 2.0;
        let logical = ui.to_logical(device_x, device_y);

        assert!(
            logical.x >= save.x && logical.x <= save.x + save.w,
            "converted x {} outside {:?}",
            logical.x,
            save
        );
        assert!(
            ui.hit_test(logical).is_some(),
            "hit test must land on a widget"
        );
    }

    #[test]
    fn font_render_scale_follows_the_device_ratio() {
        // With layout in logical units the atlas ratio is finally meaningful:
        // a `px` tall glyph occupies `px * scale` device pixels, so it must
        // rasterize at `px * scale * SUPER_SAMPLE`.
        let mut ui = UserInterface::new(1280.0, 720.0);
        ui.draw_ctx.font_atlas.set_render_scale(2.0);
        assert_eq!(ui.draw_ctx.font_atlas.render_scale, 2.0);
    }
}

#[cfg(test)]
mod styx_budget_tests {
    use super::*;

    /// Builds the real Nocturne shell with the real bundled faces and returns
    /// the frame's draw list, so the Phase 27 §10.6 budget is measured against
    /// the actual editor rather than against a synthetic scene.
    fn shell_frame(w: f32, h: f32) -> UserInterface {
        let mut ui = UserInterface::new(w, h);
        let font_id = load_fonts(&mut ui);
        let _ = build_editor_layout(
            &mut ui,
            font_id,
            crate::layout_persist::ChromeLayout::default(),
        );
        ui.perform_layout();
        ui.draw();
        ui
    }

    /// PORTAL-0-D: what a frame of the real editor shell costs on the CPU.
    ///
    /// There was no such number. `.somtime`'s `UI` row is the GPU pass, and the
    /// two whole-tree CPU traversals — `update_global_visibility` and
    /// `draw_node` — ran inside the renderer's frame with no zone around them,
    /// so the editor's per-frame layout and paint cost had never been measured
    /// at all.
    ///
    /// The shape follows `somnium_script_luau/tests/budgets.rs`: report always,
    /// assert only in release, and quote a p95 rather than a mean, because the
    /// question a budget answers is about the bad frames.
    ///
    /// The ceiling is deliberately loose. It is a tripwire against something
    /// going quadratic in the widget count, not a target — a tight bound taken
    /// from one machine would fail on a slower one and teach whoever hits it to
    /// delete the test.
    #[test]
    fn measured_cpu_cost_of_a_shell_frame() {
        let mut ui = shell_frame(1920.0, 1080.0);
        let nodes = ui.nodes.alive_count();

        let mut samples = Vec::new();
        for _ in 0..60 {
            // Steady state on purpose: `perform_layout` short-circuits on
            // nodes whose measure is still valid, which is what a real
            // unchanged frame does, and `draw` walks the whole tree every time
            // regardless. That pair is exactly the two traversals this
            // sub-phase touched.
            let t0 = std::time::Instant::now();
            ui.perform_layout();
            ui.draw();
            samples.push(t0.elapsed());
        }
        samples.sort_unstable();
        let p95 = samples[(samples.len() * 95 / 100).min(samples.len() - 1)];
        let median = samples[samples.len() / 2];

        println!(
            "shell frame: {nodes} nodes, median {:.3} ms, p95 {:.3} ms{}",
            median.as_secs_f64() * 1000.0,
            p95.as_secs_f64() * 1000.0,
            if cfg!(debug_assertions) {
                "  (debug: not enforced)"
            } else {
                ""
            }
        );

        // Both traversals allocated one `Vec<NodeHandle>` per node per frame
        // before PORTAL-0-D — two per node, so `2 * nodes` heap allocations a
        // frame that now do not happen. Recorded here because it is the part of
        // the change that is exact, where the timing is not.
        assert!(nodes > 100, "the shell should be a real tree, got {nodes}");

        if !cfg!(debug_assertions) {
            assert!(
                p95 < std::time::Duration::from_millis(8),
                "shell frame p95 {:.3} ms — layout+draw should not approach a frame budget",
                p95.as_secs_f64() * 1000.0
            );
        }
    }

    #[test]
    fn measured_instance_budget_for_the_real_shell() {
        for (w, h) in [(1920.0, 1080.0), (2560.0, 1440.0)] {
            let ui = shell_frame(w, h);
            let ctx = &ui.draw_ctx;
            let instances = ctx.instance_count();
            let bytes = instances * std::mem::size_of::<crate::primitive::Primitive>();
            let batches = ctx.commands.iter().filter(|c| c.instance_count > 0).count();

            println!(
                "{w}x{h}: {instances} instances, {} KiB, {batches} batches",
                bytes / 1024
            );

            // phase_27 §10.6, restated from measurement rather than estimate.
            //
            // Bytes: the pre-Styx list spent 4 vertices (20 B each) plus 6
            // indices (4 B each) per quad = 104 B, and a border cost four quads
            // while a shadow cost six. Styx spends one 100 B instance for each,
            // and the real shell measures ~61 KiB. 256 KiB leaves 4x headroom.
            assert!(
                bytes <= 256 * 1024,
                "{w}x{h}: instance buffer {bytes} B exceeds the 256 KiB budget"
            );

            // Batches: the plan guessed 8. That was wrong by more than an order
            // of magnitude and is corrected here against the real shell.
            //
            // `UserInterface::draw_node` pushes a clip rect for every visible
            // node, so a batch break is a genuine clip transition, not waste —
            // folding the atlases into one bind group (which is why
            // `DrawCommand` no longer carries a texture) took this from 164 to
            // 146, and the remainder is the widget tree's clipping structure.
            // Collapsing it to a single draw means clipping per instance in the
            // fragment shader instead of by scissor. That work is real and is
            // scheduled where it is actually needed — 27-D wants rounded clips
            // for scroll regions and thumbnails — rather than done here to chase
            // an invented number, because 146 scissor-plus-draw pairs cost
            // microseconds of command recording.
            assert!(
                batches <= 192,
                "{w}x{h}: {batches} batches exceeds the measured budget"
            );
        }
    }

    #[test]
    fn the_shell_actually_uses_the_new_paint_capabilities() {
        // The honest check on "does the editor look different yet". Counting
        // capability use in the real draw list is the only answer that is not a
        // claim: after 27-A/B alone every one of these was zero.
        use crate::primitive::{FLAG_GRADIENT, FLAG_INSET, FLAG_SHADOW};
        let ui = shell_frame(1920.0, 1080.0);
        let inst = &ui.draw_ctx.instances;

        let rounded = inst.iter().filter(|p| p.radii[0] > 0.0).count();
        let gradients = inst.iter().filter(|p| p.flags & FLAG_GRADIENT != 0).count();
        let shadows = inst
            .iter()
            .filter(|p| p.flags & FLAG_SHADOW != 0 && p.flags & FLAG_INSET == 0)
            .count();
        let insets = inst.iter().filter(|p| p.flags & FLAG_INSET != 0).count();
        let borders = inst.iter().filter(|p| p.border_width > 0.0).count();

        println!(
            "shell paint: {} instances | {rounded} rounded | {gradients} washed |              {shadows} lifted | {insets} recessed | {borders} stroked",
            inst.len()
        );

        // CONTROL-B no longer constructs 100+ hidden component rows at shell
        // startup; those rows exist only when a selected entity supplies a
        // schema. Keep a floor for real idle-shell strokes without fabricating
        // invisible legacy chrome merely to preserve the old count.
        assert!(rounded >= 40, "corner radius regressed to {rounded}");
        assert!(gradients >= 20, "chrome wash regressed to {gradients}");
        assert!(shadows >= 15, "elevation regressed to {shadows}");
        assert!(insets >= 4, "recession regressed to {insets}");
        assert!(borders >= 10, "strokes regressed to {borders}");

        // Content grounds must stay flat: a wash on every surface is exactly the
        // "lit like a toy" failure §5.2 forbids.
        let ground = crate::theme::active().semantic.surface.canvas.bytes();
        assert!(
            !inst
                .iter()
                .any(|p| p.fill_a == ground && p.flags & FLAG_GRADIENT != 0),
            "the canvas ground must never be washed"
        );
    }

    #[test]
    fn every_shell_region_still_paints_something_visible() {
        // The regression guard for the widget migration. 18 `draw()` methods
        // changed; the failure mode that matters is a surface that quietly
        // stopped painting — a transparent fill or a zero-alpha colour looks
        // like "the panel is gone", and no layout test would catch it because
        // the bounds are still correct.
        let ui = shell_frame(1920.0, 1080.0);
        let inst = &ui.draw_ctx.instances;

        let visible_in = |r: crate::types::Rect| {
            inst.iter().any(|p| {
                let x = p.rect[0] + p.rect[2] * 0.5;
                let y = p.rect[1] + p.rect[3] * 0.5;
                let opaque = p.fill_a[3] > 0 || p.border_color[3] > 0 || p.shadow_color[3] > 0;
                opaque && x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h
            })
        };

        let w = 1920.0;
        let h = 1080.0;
        for (name, region) in [
            (
                "application bar",
                crate::types::Rect::new(0.0, 0.0, w, 36.0),
            ),
            ("mode toolbar", crate::types::Rect::new(0.0, 36.0, w, 32.0)),
            (
                "left rail",
                crate::types::Rect::new(0.0, 68.0, 120.0, 400.0),
            ),
            (
                "right column",
                crate::types::Rect::new(w - 300.0, 68.0, 300.0, 600.0),
            ),
            (
                "status bar",
                crate::types::Rect::new(0.0, h - 26.0, w, 26.0),
            ),
        ] {
            assert!(visible_in(region), "{name} paints nothing visible");
        }
    }

    #[test]
    fn no_migrated_surface_became_fully_transparent() {
        // A `Paint` whose background, border and shadow are all zero-alpha
        // renders nothing at all. Some are legitimately invisible (a ghost icon
        // button at rest), so this checks the *proportion* rather than banning
        // them outright: a migration slip would push it far past this.
        let ui = shell_frame(1920.0, 1080.0);
        let inst = &ui.draw_ctx.instances;
        let invisible = inst
            .iter()
            .filter(|p| p.fill_a[3] == 0 && p.border_color[3] == 0 && p.shadow_color[3] == 0)
            .count();
        let ratio = invisible as f32 / inst.len() as f32;
        println!(
            "invisible instances: {invisible}/{} ({:.1}%)",
            inst.len(),
            ratio * 100.0
        );
        assert!(
            ratio < 0.25,
            "{:.0}% of the draw list paints nothing — a surface was likely lost",
            ratio * 100.0
        );
    }

    #[test]
    fn an_idle_shell_rebuilds_a_byte_identical_draw_list() {
        // phase_27 §10.3 / §5.6: nothing may churn the draw list between two
        // frames with no input. This is the guard that keeps the coming
        // animation driver (27-C) from quietly costing a redraw every frame.
        let a = shell_frame(1920.0, 1080.0);
        let b = shell_frame(1920.0, 1080.0);
        assert_eq!(a.draw_ctx.instance_count(), b.draw_ctx.instance_count());
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&a.draw_ctx.instances),
            bytemuck::cast_slice::<_, u8>(&b.draw_ctx.instances),
        );
        assert_eq!(a.draw_ctx.commands, b.draw_ctx.commands);
    }

    #[test]
    fn the_shell_never_exhausts_the_font_atlas() {
        // A full atlas makes text silently vanish (`FontAtlas::get_or_rasterize`
        // returns None and the draw path advances past a blank). The shell must
        // sit well clear of that.
        let ui = shell_frame(1920.0, 1080.0);
        let atlas = &ui.draw_ctx.font_atlas;
        println!(
            "shell atlas: {} glyphs, {:.1}% used",
            atlas.cached_glyph_count(),
            atlas.utilization() * 100.0
        );
        assert!(!atlas.is_full());
        assert!(atlas.utilization() < 0.75);
    }
}

#[cfg(test)]
mod zeta_layout_tests {
    use super::*;

    fn bounds(ui: &UserInterface, handle: NodeHandle) -> crate::types::Rect {
        ui.nodes
            .try_borrow(handle.transmute())
            .expect("layout handle should remain valid")
            .widget
            .screen_bounds()
    }

    #[test]
    fn application_and_mode_scopes_cost_68px_and_the_context_bar_floats() {
        let mut ui = UserInterface::new(1920.0, 1080.0);
        let layout =
            build_editor_layout(&mut ui, 0, crate::layout_persist::ChromeLayout::default());
        ui.perform_layout();

        let menu = bounds(&ui, layout.menu_bar_h);
        let search = bounds(&ui, layout.palette_button);
        let viewport = bounds(&ui, layout.viewport_handle);
        let context = bounds(&ui, layout.vp_bar_h);

        assert!(menu.y < theme::TITLEBAR_HEIGHT);
        assert!(menu.y + menu.h <= theme::TITLEBAR_HEIGHT + 0.1);
        assert!(search.w > 0.0 && search.y + search.h <= theme::TITLEBAR_HEIGHT + 0.1);

        // Redline §06: only two scopes take layout space — application 36 and
        // mode 32. The scene starts at 68 px, not the 122 px of the four-band
        // shell or the 100 px of the docked-context intermediate.
        assert!(
            (viewport.y - 68.0).abs() < 0.1,
            "viewport should begin after the 36 + 32 scope budget, got {}",
            viewport.y
        );

        // The third scope floats *inside* the viewport at a 12 px inset.
        assert!(
            context.y >= viewport.y + 11.9 && context.y <= viewport.y + 12.1,
            "context bar should be inset 12 px into the viewport, got {} vs {}",
            context.y,
            viewport.y
        );
        assert!((context.h - theme::NOCTURNE.density.toolbar).abs() < 0.1);
        assert!(context.x >= viewport.x + 11.9);
        assert!(context.x + context.w <= viewport.x + viewport.w - 11.9);
    }

    #[test]
    fn collapse_rules_shed_chrome_in_the_redline_order() {
        // 1920: everything present.
        let wide = CollapseRules::for_width(1920.0);
        assert!(wide.transport_label && wide.search_field && wide.status_objects);

        // 1366 laptop: the play-state word goes first; search, statistics and
        // — crucially — the named mode commands stay.
        let laptop = CollapseRules::for_width(1366.0);
        assert!(!laptop.transport_label);
        assert!(laptop.search_field && laptop.status_objects);

        // 1280: the status bar starts shedding right to left.
        assert!(!CollapseRules::for_width(1279.0).status_objects);

        // 1024: the search field goes too. Ctrl+P still opens the palette.
        let small = CollapseRules::for_width(1024.0);
        assert!(!small.search_field);

        // The order is monotone — nothing reappears as the window narrows.
        let mut prev = CollapseRules::for_width(2560.0);
        for w in (600..2560).step_by(17).rev() {
            let now = CollapseRules::for_width(w as f32);
            assert!(!(now.transport_label && !prev.transport_label), "width {w}");
            assert!(!(now.search_field && !prev.search_field), "width {w}");
            assert!(!(now.status_objects && !prev.status_objects), "width {w}");
            prev = now;
        }
    }

    #[test]
    fn shell_rehosts_every_create_action_and_primary_transport_control() {
        let mut ui = UserInterface::new(1280.0, 720.0);
        let layout =
            build_editor_layout(&mut ui, 0, crate::layout_persist::ChromeLayout::default());
        let expected_create_ids: Vec<_> = crate::commands::registry()
            .menu(crate::commands::Menu::Create)
            .into_iter()
            .map(|command| command.id)
            .collect();
        let shell_create_ids: Vec<_> = layout
            .create_popup_items
            .iter()
            .map(|(_, command_id)| *command_id)
            .collect();
        assert_eq!(shell_create_ids, expected_create_ids);
        for handle in [
            layout.save_button,
            layout.select_button,
            layout.landscape_button,
            layout.foliage_toolbar_button,
            layout.play_button,
            layout.immersive_button,
            layout.pause_button,
            layout.step_button,
            layout.stop_button,
            layout.palette_button,
        ] {
            assert!(!handle.is_none());
        }
    }
}

/// Phase 26-Zeta-J — automated coverage of the `phase_26.md` §14
/// must-not-break inventory.
///
/// These tests cannot press keys or click a viewport; what they *can* do is
/// assert that every control the inventory names still exists, is reachable,
/// and is wired to the same `EditorEvent`. That turns "I re-read the list" into
/// something CI can repeat. The items that genuinely need a human at the
/// keyboard — fly-cam feel, gizmo drag, terrain sculpting against real
/// heightmaps — are enumerated in `MANUAL_ONLY` below rather than quietly
/// omitted.
#[cfg(test)]
mod must_not_break {
    use super::*;

    /// §14 items that no headless test can stand in for. Listed so the gap is
    /// countable rather than invisible.
    pub const MANUAL_ONLY: &[&str] = &[
        "1. viewport RMB fly-cam WASD/QE + Shift",
        "2. LMB pick entity; gizmo drag T/R/S",
        "9. terrain sculpt against a real heightmap",
        "10. foliage paint/erase density on a surface",
        "18. UI does not eat viewport input over the 3D hole",
    ];

    fn layout(w: f32, h: f32) -> (UserInterface, EditorLayout) {
        let mut ui = UserInterface::new(w, h);
        let l = build_editor_layout(
            &mut ui,
            0,
            crate::layout_persist::ChromeLayout::default().resolved(w, h),
        );
        ui.perform_layout();
        (ui, l)
    }

    #[test]
    fn every_create_kind_still_has_a_menu_row() {
        // §14.4. The Create popup is built from `CreateKind`, so a new variant
        // that nobody added a row for shows up here.
        let (_, l) = layout(1920.0, 1080.0);
        let ids: Vec<&str> = l.create_popup_items.iter().map(|(_, id)| *id).collect();
        for kind in [
            CreateKind::Cube,
            CreateKind::Sphere,
            CreateKind::Plane,
            CreateKind::Cylinder,
            CreateKind::DirectionalLight,
            CreateKind::PointLight,
            CreateKind::SpotLight,
            CreateKind::Particle,
            CreateKind::Terrain,
            CreateKind::VoxelTerrain,
            CreateKind::UiCanvas,
        ] {
            let command = crate::commands::registry()
                .menu(crate::commands::Menu::Create)
                .into_iter()
                .find(|command| {
                    command.action == crate::commands::CommandAction::CreateEntity(kind)
                })
                .unwrap();
            assert!(ids.contains(&command.id), "{kind:?} lost its Create row");
        }
        assert!(ids.contains(&"editor.asset.new_material"));
    }

    #[test]
    fn transport_scene_and_mode_commands_survive_the_shell_rebuild() {
        // §14.4–8: save, new/import via the File menu, the three editing modes,
        // the transport triple and immersive play.
        let (ui, l) = layout(1920.0, 1080.0);
        for (name, handle) in [
            ("save", l.save_button),
            ("select mode", l.select_button),
            ("landscape mode", l.landscape_button),
            ("foliage mode", l.foliage_toolbar_button),
            ("play", l.play_button),
            ("pause", l.pause_button),
            ("step", l.step_button),
            ("stop", l.stop_button),
            ("immersive play", l.immersive_button),
            ("file menu", l.file_button),
            ("profiler", l.profiler_toggle),
            ("camera speed", l.camera_speed_slider),
            ("content drawer", l.drawer_button),
            ("output log", l.log_button),
            ("references", l.references_button),
            ("localisation", l.locale_button),
            ("help", l.help_button),
        ] {
            assert!(!handle.is_none(), "{name} is missing from the shell");
            assert!(
                bounds_of(&ui, handle).w > 0.0,
                "{name} exists but has no width — it cannot be clicked"
            );
        }

        // Menu rows are registry-derived and map back to stable ids.
        for id in [
            "editor.scene.new",
            "editor.scene.save",
            "editor.asset.import_model",
            "editor.edit.undo",
            "editor.edit.redo",
            "editor.edit.delete",
            "editor.edit.duplicate",
        ] {
            let handle = l
                .menu_command_items
                .iter()
                .find(|(_, command_id)| *command_id == id)
                .map(|(handle, _)| *handle);
            assert!(
                handle.is_some_and(|handle| !handle.is_none()),
                "{id} is missing from its menu"
            );
        }
    }

    #[test]
    fn the_terrain_brush_palette_still_arms_all_six_tools() {
        // §14.9. Tool indices are what `SetTerrainTool` carries, so an
        // accidental reorder of the palette is a behaviour change.
        let (_, l) = layout(1920.0, 1080.0);
        let tools: Vec<u8> = l.terrain_tool_items.iter().map(|(_, _, t)| *t).collect();
        assert_eq!(tools, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn the_viewport_stays_the_largest_region_at_every_target_size() {
        // §10.2. Checked at the compact case and the ultrawide one, where a
        // fixed-pixel panel would break it first.
        for (w, h) in [
            (1280.0, 720.0),
            (1920.0, 1080.0),
            (2560.0, 1440.0),
            (3440.0, 1440.0),
        ] {
            let (ui, l) = layout(w, h);
            let vp = bounds_of(&ui, l.viewport_handle);
            assert!(vp.w > w * 0.4, "viewport only {} wide at {w}×{h}", vp.w);
            assert!(vp.h > 0.0);
        }
    }

    #[test]
    fn the_shell_survives_a_1280x720_window_without_losing_a_control() {
        // §10.2 compact case. Collapse may hide *labels*; it must not remove a
        // command, so the handles stay valid and the buttons keep their size.
        let (ui, l) = layout(1280.0, 720.0);
        for handle in [l.save_button, l.play_button, l.stop_button, l.drawer_button] {
            assert!(bounds_of(&ui, handle).w > 0.0);
        }
    }

    fn bounds_of(ui: &UserInterface, handle: NodeHandle) -> crate::types::Rect {
        ui.nodes
            .try_borrow(handle.transmute())
            .expect("handle should remain valid after layout")
            .widget
            .screen_bounds()
    }

    #[test]
    fn every_tool_only_inspector_control_is_hittable() {
        let mut ui = UserInterface::new(1920.0, 1080.0);
        let layout = build_editor_layout(
            &mut ui,
            0,
            crate::layout_persist::ChromeLayout::default().resolved(1920.0, 1080.0),
        );
        let handles = &layout.inspector_handles;
        for section in [
            handles.post_section,
            handles.terrain_section,
            handles.foliage_section,
        ] {
            ui.set_visibility(section, true);
        }
        ui.perform_layout();
        for handle in [
            handles.post_census_toggle,
            handles.post_bins_toggle,
            handles.terrain_paint_toggle,
            handles.terrain_aerial_dist,
            handles.foliage_kind_button,
            handles.foliage_density,
        ] {
            let bounds = bounds_of(&ui, handle);
            assert!(
                bounds.w >= 8.0 && bounds.h >= 8.0,
                "{handle} is not hittable"
            );
        }
    }

    #[test]
    fn the_manual_only_list_is_short_and_explicit() {
        // Guard against the list quietly growing to cover for missing tests.
        assert!(
            MANUAL_ONLY.len() <= 6,
            "too much is being deferred to a human"
        );
    }
}

#[cfg(test)]
mod collection_editor_tests {
    use super::{CollectionAction, GeneratedEdit, UiManager, element_lane, element_lane_count};
    use somnium_ecs::reflect::ReflectValue as RV;

    fn points() -> RV {
        RV::Array(vec![
            RV::Vec3([0.0, 1.0, 2.0]),
            RV::Vec3([3.0, 4.0, 5.0]),
            RV::Vec3([6.0, 7.0, 8.0]),
        ])
    }

    /// Appending copies the last element rather than inserting a zero.
    /// A new spline point at the world origin is a point the author has to go
    /// and find before they can use it; one beside the end of the path is one
    /// they can drag straight away.
    #[test]
    fn adding_a_point_extends_the_path_where_it_already_is() {
        let RV::Array(items) = UiManager::collection_result(&points(), CollectionAction::Append)
            .expect("append always succeeds on an array")
        else {
            panic!("append must return an array");
        };
        assert_eq!(items.len(), 4);
        assert_eq!(items[3], RV::Vec3([6.0, 7.0, 8.0]));
    }

    /// An empty collection still has to accept a first point, or a freshly
    /// created spline can never be given one.
    #[test]
    fn the_first_point_can_be_added_to_an_empty_collection() {
        let RV::Array(items) =
            UiManager::collection_result(&RV::Array(Vec::new()), CollectionAction::Append).unwrap()
        else {
            panic!()
        };
        assert_eq!(items, vec![RV::Vec3([0.0; 3])]);
    }

    #[test]
    fn removing_takes_the_addressed_element_and_nothing_else() {
        let RV::Array(items) =
            UiManager::collection_result(&points(), CollectionAction::Remove(1)).unwrap()
        else {
            panic!()
        };
        assert_eq!(
            items,
            vec![RV::Vec3([0.0, 1.0, 2.0]), RV::Vec3([6.0, 7.0, 8.0])]
        );
    }

    /// Duplicating inserts *after* the original, which is what makes it a way
    /// to add detail to the middle of a path rather than a way to append.
    #[test]
    fn duplicating_inserts_directly_after_the_original() {
        let RV::Array(items) =
            UiManager::collection_result(&points(), CollectionAction::Duplicate(0)).unwrap()
        else {
            panic!()
        };
        assert_eq!(items.len(), 4);
        assert_eq!(items[0], items[1]);
        assert_eq!(items[2], RV::Vec3([3.0, 4.0, 5.0]));
    }

    /// An index that is not there is a refusal, not a panic and not a silent
    /// no-op on the wrong element. Stale indices are ordinary here: the rows
    /// are rebuilt a frame after a removal.
    #[test]
    fn an_out_of_range_index_changes_nothing() {
        assert!(UiManager::collection_result(&points(), CollectionAction::Remove(9)).is_none());
        assert!(UiManager::collection_result(&points(), CollectionAction::Duplicate(9)).is_none());
        assert!(
            UiManager::collection_result(&RV::Vec3([0.0; 3]), CollectionAction::Append).is_none(),
            "and a non-array is not a collection"
        );
    }

    /// Editing one lane of one point leaves every other number alone. The
    /// write path rebuilds the whole array, so this is the test that it
    /// rebuilds it *faithfully*.
    #[test]
    fn editing_one_lane_touches_exactly_that_lane() {
        let binding = super::GeneratedBinding {
            component: somnium_ecs::reflect::StableId::new("somnium.Spline"),
            field: somnium_ecs::reflect::FieldId(0),
            value: points(),
            default: RV::Nil,
            edit: GeneratedEdit::Element { index: 1, lane: 2 },
            asset_kind_mask: u64::MAX,
        };
        let RV::Array(items) = UiManager::numeric_reflect_value(&binding, -42.0).unwrap() else {
            panic!()
        };
        assert_eq!(items[0], RV::Vec3([0.0, 1.0, 2.0]), "untouched");
        assert_eq!(items[1], RV::Vec3([3.0, 4.0, -42.0]), "one lane changed");
        assert_eq!(items[2], RV::Vec3([6.0, 7.0, 8.0]), "untouched");
    }

    #[test]
    fn lanes_are_read_back_by_index_and_refuse_what_they_cannot_show() {
        let RV::Array(items) = points() else { panic!() };
        assert_eq!(element_lane(&items, 2, 1), Some(7.0));
        assert_eq!(element_lane(&items, 9, 0), None, "past the end");
        assert_eq!(element_lane(&items, 0, 7), None, "past the lanes");

        assert_eq!(element_lane_count(&RV::Vec3([0.0; 3])), 3);
        assert_eq!(element_lane_count(&RV::F64(1.0)), 1);
        assert_eq!(
            element_lane_count(&RV::Str(String::new())),
            0,
            "a type with no numeric lanes draws no row rather than a broken one"
        );
    }
}
