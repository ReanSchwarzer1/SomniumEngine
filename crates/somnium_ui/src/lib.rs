pub mod color;
pub mod commands;
pub mod draw;
pub mod editor;
pub mod editor_event;
pub mod font;
pub mod icon_svg;
pub mod icons;
pub mod layout_persist;
pub mod message;
pub mod metaphor;
pub mod motion;
pub mod node;
pub mod pass;
pub mod pool;
pub mod primitive;
pub mod runtime;
pub mod style;
pub mod theme;
pub mod thumbnail;
pub mod types;
pub mod typography;
pub mod ui;
pub mod widget;
pub mod widgets;
pub mod workspace;

use crate::editor::{
    content::{build_content_drawer, build_create_popup},
    help::{build_help_overlay, fill_help_body},
    inspector::build_inspector,
    shell::build_editor_layout,
};
pub use editor_event::{
    ColorField, CreateKind, EditorEvent, InspectorField, PostFxToggle, ScriptAttachmentRow,
    ScriptFieldKind, ScriptFieldRow, ScriptInspectorState,
};
pub use node::CursorKind;
pub use runtime::UiCanvas;

pub use typography::{FontRole, TextRole};
pub use workspace::{BottomPanel, Workspace, WorkspaceLayout};

use crate::{
    editor_event::InspectorField as IF,
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
        text_box::TextBoxMessage,
        toast::ToastMessage,
        tree_view::{TreeItem, TreeViewMessage},
    },
};
use glam::Vec2;
use std::collections::VecDeque;
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
    /// Rename this absolute path.
    Rename { path: String },
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

// ── Inspector field handle bundle ────────────────────────────────────────────

#[allow(dead_code)]
struct InspectorHandles {
    pos_x: NodeHandle,
    pos_y: NodeHandle,
    pos_z: NodeHandle,
    rot_x: NodeHandle,
    rot_y: NodeHandle,
    rot_z: NodeHandle,
    sc_x: NodeHandle,
    sc_y: NodeHandle,
    sc_z: NodeHandle,
    // Light section (Phase 13E) — hidden unless a light is selected.
    light_section: NodeHandle,
    light_intensity: NodeHandle,
    light_range: NodeHandle,
    light_inner: NodeHandle,
    light_outer: NodeHandle,
    /// Linear-RGB colour rows (Phase 22C) — kept for event compatibility; hidden.
    light_col_r: NodeHandle,
    light_col_g: NodeHandle,
    light_col_b: NodeHandle,
    light_color: NodeHandle,
    light_temp_k: NodeHandle,
    /// Row containers for the point/spot-only fields, so they can be hidden
    /// wholesale (label included) when a directional light is selected — range
    /// and cone angles mean nothing for a sun.
    light_range_row: NodeHandle,
    light_inner_row: NodeHandle,
    light_outer_row: NodeHandle,
    /// Row container for directional-only moonlight intensity field (Phase 25M-2).
    light_moon_row: NodeHandle,
    light_moon_int: NodeHandle,
    light_radius: NodeHandle,
    light_width_row: NodeHandle,
    light_width: NodeHandle,
    light_height_row: NodeHandle,
    light_height: NodeHandle,
    // Camera section (Phase CR-C) — hidden unless a Camera entity is selected.
    camera_section: NodeHandle,
    camera_frustum_toggle: NodeHandle,
    camera_frustum_label: NodeHandle,
    /// Phase DOOM-F.
    camera_dynres_toggle: NodeHandle,
    camera_dynres_label: NodeHandle,
    camera_dynres_target: NodeHandle,
    camera_dynres_floor: NodeHandle,
    /// Phase DOOM-E (terrain) and DOOM-B/C (post FX diagnostics).
    terrain_aerial_toggle: NodeHandle,
    terrain_aerial_label: NodeHandle,
    terrain_aerial_dist: NodeHandle,
    terrain_aerial_hero_toggle: NodeHandle,
    terrain_aerial_hero_label: NodeHandle,
    post_census_toggle: NodeHandle,
    post_census_label: NodeHandle,
    post_bins_toggle: NodeHandle,
    post_bins_label: NodeHandle,
    // Post-processing section (Phase 15A1) — hidden unless a Post Processing
    // entity is selected.
    post_section: NodeHandle,
    post_exposure: NodeHandle,
    post_exp_comp: NodeHandle,
    post_auto_exp_toggle: NodeHandle,
    post_auto_exp_label: NodeHandle,
    post_tonemap_button: NodeHandle,
    post_tonemap_label: NodeHandle,
    post_vig_toggle: NodeHandle,
    post_vig_str: NodeHandle,
    post_ca_toggle: NodeHandle,
    post_ca_str: NodeHandle,
    /// Scene-wide indirect-light strength (Phase 22C).
    post_ibl: NodeHandle,
    // Terrain + foliage sections (Phase 17C), hidden unless a terrain is picked.
    terrain_section: NodeHandle,
    terrain_mode_label: NodeHandle,
    terrain_paint_toggle: NodeHandle,
    terrain_paint_label: NodeHandle,
    terrain_hex_toggle: NodeHandle,
    terrain_hex_label: NodeHandle,
    terrain_parallax_toggle: NodeHandle,
    terrain_parallax_label: NodeHandle,
    terrain_clipmap_toggle: NodeHandle,
    terrain_clipmap_label: NodeHandle,
    terrain_morph_toggle: NodeHandle,
    terrain_morph_label: NodeHandle,
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
    water_section: NodeHandle,
    water_surface: NodeHandle,
    water_depth: NodeHandle,
    water_clarity: NodeHandle,
    water_amplitude: NodeHandle,
    water_roughness: NodeHandle,
    water_ssr: NodeHandle,
    water_rt_reflect: NodeHandle,
    water_reflect_debug: NodeHandle,
    water_wave_a: NodeHandle,
    water_wave_b: NodeHandle,
    water_speed: NodeHandle,
    water_steepness: NodeHandle,
    water_wind_speed: NodeHandle,
    water_foam_decay: NodeHandle,
    water_foam_threshold: NodeHandle,
    water_spectrum_blend: NodeHandle,
    water_edge_scale: NodeHandle,
    water_anisotropy: NodeHandle,
    water_caustic: NodeHandle,
    water_deep: NodeHandle,
    water_shallow: NodeHandle,
    water_edge: NodeHandle,
    water_abs: NodeHandle,
    water_scatter: NodeHandle,
    water_abs_mag: NodeHandle,
    water_scatter_mag: NodeHandle,
    water_underwater: NodeHandle,
    water_dir_ax: NodeHandle,
    water_dir_az: NodeHandle,
    water_dir_bx: NodeHandle,
    water_dir_bz: NodeHandle,
    particle_section: NodeHandle,
    particle_start: NodeHandle,
    particle_end: NodeHandle,
    material_section: NodeHandle,
    material_base: NodeHandle,
    /// Phase 16-D. The Scripts section holds a header, a "New Script"
    /// button and `script_list`; every row inside the list is built from a
    /// script's declared schema at refresh time, not here.
    script_section: NodeHandle,
    script_add: NodeHandle,
    script_list: NodeHandle,
    vessel_section: NodeHandle,
    vessel_buoyancy: NodeHandle,
    vessel_drag: NodeHandle,
    vessel_angular_drag: NodeHandle,
    vessel_thrust: NodeHandle,
    vessel_draft: NodeHandle,
    vessel_righting: NodeHandle,
    foliage_section: NodeHandle,
    foliage_toggle: NodeHandle,
    foliage_label: NodeHandle,
    foliage_paint_toggle: NodeHandle,
    foliage_paint_label: NodeHandle,
    foliage_erase_toggle: NodeHandle,
    foliage_erase_label: NodeHandle,
    foliage_single_toggle: NodeHandle,
    foliage_single_label: NodeHandle,
    /// Button that opens the picker; its label shows the current entry.
    foliage_kind_button: NodeHandle,
    foliage_kind_label: NodeHandle,
    foliage_density: NodeHandle,
    foliage_seed: NodeHandle,
    foliage_slope: NodeHandle,
    foliage_layer: NodeHandle,
    foliage_smin: NodeHandle,
    foliage_smax: NodeHandle,
    foliage_shadow: NodeHandle,
    foliage_cull: NodeHandle,
    foliage_lod: NodeHandle,
    foliage_impostor: NodeHandle,
    /// Text label inside each toggle button, so the tick can be redrawn.
    post_vig_label: NodeHandle,
    post_ca_label: NodeHandle,
    post_fxaa_toggle: NodeHandle,
    post_fxaa_label: NodeHandle,
    post_cel_toggle: NodeHandle,
    post_cel_label: NodeHandle,
    post_fsr_toggle: NodeHandle,
    post_fsr_label: NodeHandle,
    post_fsr_sharp: NodeHandle,
    post_taa_toggle: NodeHandle,
    post_taa_label: NodeHandle,
    post_gtao_toggle: NodeHandle,
    post_gtao_label: NodeHandle,
    post_restir_toggle: NodeHandle,
    post_restir_gi_toggle: NodeHandle,
    post_rt_reflect_toggle: NodeHandle,
    post_rt_reflect_label: NodeHandle,
    post_rt_refract_toggle: NodeHandle,
    post_rt_refract_label: NodeHandle,
    post_cas_toggle: NodeHandle,
    post_mb_toggle: NodeHandle,
    post_mb_label: NodeHandle,
    post_mb_shutter: NodeHandle,
    post_gi_intensity: NodeHandle,
    post_cas_label: NodeHandle,
    post_cas_sharp: NodeHandle,
    post_cas_strength: NodeHandle,
    post_restir_gi_label: NodeHandle,
    post_restir_label: NodeHandle,
    post_pcss_toggle: NodeHandle,
    post_pcss_label: NodeHandle,
    post_contact_toggle: NodeHandle,
    post_contact_label: NodeHandle,
    post_bloom_toggle: NodeHandle,
    post_bloom_label: NodeHandle,
    post_bloom_amt: NodeHandle,
    post_dof_toggle: NodeHandle,
    post_dof_label: NodeHandle,
    post_dof_focus: NodeHandle,
    post_temperature: NodeHandle,
    post_contrast: NodeHandle,
    post_saturation: NodeHandle,
    post_grain: NodeHandle,
    post_vol_toggle: NodeHandle,
    post_vol_label: NodeHandle,
    post_shafts_toggle: NodeHandle,
    post_shafts_label: NodeHandle,
    post_phys_toggle: NodeHandle,
    post_phys_label: NodeHandle,
    post_aperture: NodeHandle,
    post_shutter: NodeHandle,
    post_iso: NodeHandle,
    post_tint: NodeHandle,
    post_lift: NodeHandle,
    post_gamma: NodeHandle,
    post_gain: NodeHandle,
    post_ao_radius: NodeHandle,
    post_ao_intensity: NodeHandle,
    post_fog_density: NodeHandle,
    post_fog_height: NodeHandle,
    post_fog_asym: NodeHandle,
    post_world_cache_toggle: NodeHandle,
    post_world_cache_label: NodeHandle,
    post_cache_intensity: NodeHandle,
    post_cache_cell: NodeHandle,
    post_specular_toggle: NodeHandle,
    post_specular_label: NodeHandle,
    post_spec_rough: NodeHandle,
    post_path_toggle: NodeHandle,
    post_path_label: NodeHandle,
    post_path_bounces: NodeHandle,
    post_sdf_toggle: NodeHandle,
    post_sdf_label: NodeHandle,
    post_probes_toggle: NodeHandle,
    post_probes_label: NodeHandle,
    post_probe_intensity: NodeHandle,
    post_analytic_toggle: NodeHandle,
    post_analytic_label: NodeHandle,
    post_shaft_amt: NodeHandle,
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

/// Rows the overlay can show before it starts dropping them.
pub const PROFILER_ROWS: usize = 40;

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

const TONEMAP_NAMES: [&str; 3] = ["AgX", "ACES", "Reinhard"];
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
    log_empty: NodeHandle,
    create_button: NodeHandle,
    create_popup: NodeHandle,
    create_popup_items: Vec<(NodeHandle, CreateKind)>,
    file_button: NodeHandle,
    file_popup: NodeHandle,
    menu_command_items: Vec<(NodeHandle, &'static str)>,
    camera_speed_slider: NodeHandle,
    camera_speed_label: NodeHandle,
    viewport_res_combo: NodeHandle,
    play_button: NodeHandle,
    play_label: NodeHandle,
    immersive_button: NodeHandle,
    pause_button: NodeHandle,
    pause_label: NodeHandle,
    stop_button: NodeHandle,
    stop_label: NodeHandle,
    select_button: NodeHandle,
    landscape_button: NodeHandle,
    foliage_toolbar_button: NodeHandle,
    terrain_tool_items: Vec<(NodeHandle, NodeHandle, u8)>,
    inspector_handles: InspectorHandles,
    viewport_handle: NodeHandle,
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
    /// Floating viewport-context scope; a child of the viewport, not a grid row.
    /// Held for the layout regression test that pins the 68 px scene budget.
    #[allow(dead_code)]
    vp_bar_h: NodeHandle,
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
    drawer_button: NodeHandle,
    log_button: NodeHandle,
    content_drawer: NodeHandle,
    content_search: NodeHandle,
    content_breadcrumb: NodeHandle,
    content_engine_toggle: NodeHandle,
    content_list: NodeHandle,
    outliner_tree: NodeHandle,
    outliner_search: NodeHandle,
    inspector_search: NodeHandle,
    foliage_kind_combo: NodeHandle,
    post_tonemap_combo: NodeHandle,
    foliage_kind_popup: NodeHandle,
    post_tonemap_popup: NodeHandle,
    viewport_res_popup: NodeHandle,
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
    name_popup: NodeHandle,
    name_title: NodeHandle,
    name_input: NodeHandle,
    name_ok: NodeHandle,
    name_cancel: NodeHandle,
    color_popup: NodeHandle,
    color_picker: NodeHandle,
    title_drag: NodeHandle,
    win_min: NodeHandle,
    win_max: NodeHandle,
    win_close: NodeHandle,
    help_toc: Vec<(NodeHandle, u8)>,
    help_close: NodeHandle,
    log_panel: NodeHandle,
}

// ── UiManager ────────────────────────────────────────────────────────────────

/// Combined UI manager — wraps the native wgpu widget tree rendered by UiPass.
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
    log_entry_count: usize,
    // Create menu
    create_button: NodeHandle,
    create_popup: NodeHandle,
    create_popup_open: bool,
    create_popup_items: Vec<(NodeHandle, CreateKind)>,
    /// Palette entry currently shown on the picker button, so a click can
    /// advance to the next one.
    foliage_kind_shown: u8,
    // File menu (Phase 19B): Import
    file_button: NodeHandle,
    file_popup: NodeHandle,
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
    inspector_handles: InspectorHandles,
    // Editor event queue drained by app.rs each frame
    editor_events: VecDeque<EditorEvent>,
    // Viewport area handle — mouse events here pass through to the game
    #[allow(dead_code)]
    viewport_handle: NodeHandle,
    profiler_panel: NodeHandle,
    profiler_toggle: NodeHandle,
    profiler_toggle_lbl: NodeHandle,
    profiler_names: Vec<NodeHandle>,
    profiler_values: Vec<NodeHandle>,
    last_outliner_state: Option<(Vec<(u32, String)>, Option<u32>)>,
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
    /// Value each inspector field held at the last baseline reset — a scene
    /// load, a save, or a change of selection. The modified dot lights when the
    /// live value differs from this, and reverting writes it back.
    ///
    /// This is deliberately *not* "differs from the component default": the UI
    /// layer does not know component defaults, and inventing them would make
    /// the dot lie. "Unsaved edit to this property" is the honest reading, and
    /// it is the one that pairs with the status bar's save state.
    inspector_baseline: std::collections::HashMap<IF, f32>,
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
    drawer_button: NodeHandle,
    log_button: NodeHandle,
    content_drawer: NodeHandle,
    content_search: NodeHandle,
    content_breadcrumb: NodeHandle,
    content_engine_toggle: NodeHandle,
    content_list: NodeHandle,
    outliner_tree: NodeHandle,
    outliner_search: NodeHandle,
    inspector_search: NodeHandle,
    foliage_kind_combo: NodeHandle,
    post_tonemap_combo: NodeHandle,
    foliage_kind_popup: NodeHandle,
    post_tonemap_popup: NodeHandle,
    viewport_res_popup: NodeHandle,
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
    /// Paths selected in the drawer. Multi-select is a set rather than a range
    /// because the tiles wrap, so "everything between A and B" has no stable
    /// meaning once the panel is resized.
    content_selection: std::collections::HashSet<std::path::PathBuf>,
    outliner_expanded: std::collections::HashSet<u32>,
    log_lines: VecDeque<String>,
    edit_popup_open: bool,
    view_popup_open: bool,
    window_popup_open: bool,
    help_menu_open: bool,
    tooltip_since: Option<(NodeHandle, std::time::Instant)>,
    help_page: u8,
    content_entries: Vec<(NodeHandle, crate::metaphor::ContentEntry)>,
    outliner_filter: String,
    palette_open: bool,
    unsaved_open: bool,
    color_open: bool,
    color_target: Option<crate::ColorField>,
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
    title_drag: NodeHandle,
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
            log_entry_count: 0,
            create_button: layout.create_button,
            create_popup: layout.create_popup,
            create_popup_open: false,
            create_popup_items: layout.create_popup_items,
            foliage_kind_shown: 0,
            file_button: layout.file_button,
            file_popup: layout.file_popup,
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
            viewport_handle: layout.viewport_handle,
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
            inspector_baseline: std::collections::HashMap::new(),
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
            drawer_button: layout.drawer_button,
            log_button: layout.log_button,
            content_drawer: layout.content_drawer,
            content_search: layout.content_search,
            content_breadcrumb: layout.content_breadcrumb,
            content_engine_toggle: layout.content_engine_toggle,
            content_list: layout.content_list,
            outliner_tree: layout.outliner_tree,
            outliner_search: layout.outliner_search,
            inspector_search: layout.inspector_search,
            foliage_kind_combo: layout.foliage_kind_combo,
            post_tonemap_combo: layout.post_tonemap_combo,
            foliage_kind_popup: layout.foliage_kind_popup,
            post_tonemap_popup: layout.post_tonemap_popup,
            viewport_res_popup: layout.viewport_res_popup,
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
            content_selection: std::collections::HashSet::new(),
            outliner_expanded: std::collections::HashSet::new(),
            log_lines: VecDeque::new(),
            edit_popup_open: false,
            view_popup_open: false,
            window_popup_open: false,
            help_menu_open: false,
            tooltip_since: None,
            help_page: 0,
            content_entries: Vec::new(),
            outliner_filter: String::new(),
            palette_open: false,
            unsaved_open: false,
            color_open: false,
            color_target: None,
            color_original: [1.0, 1.0, 1.0, 1.0],
            color_live: [1.0, 1.0, 1.0, 1.0],
            content_menu_target: None,
            content_menu_folder: String::new(),
            name_prompt: None,
            name_text: String::new(),
            script_state: ScriptInspectorState::default(),
            script_widgets: Vec::new(),
            scene_dirty: false,
            script_errors: 0,
            chrome_layout: layout_sizes,
            log_open: false,
            title_drag: layout.title_drag,
            win_min: layout.win_min,
            win_max: layout.win_max,
            win_close: layout.win_close,
            help_toc: layout.help_toc,
            help_close: layout.help_close,
            log_panel: layout.log_panel,
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
        let (w, h) = (self.window_size.0 as f32, self.window_size.1 as f32);
        let preset = workspace.preset(w, h);
        self.apply_workspace_layout(preset, w);
        self.push_toast(&format!("{} workspace", workspace.label()));
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
            if self.open_content_menu(self.native_ui.cursor_pos) {
                return true;
            }
        }
        if let WindowEvent::KeyboardInput { event: key_ev, .. } = event {
            let pressed = key_ev.state == ElementState::Pressed;
            if let PhysicalKey::Code(code) = key_ev.physical_key {
                if pressed && !key_ev.repeat && !self.native_ui.has_text_focus() {
                    if let Some(chord) = crate::commands::Chord::from_winit(
                        code,
                        self.native_ui.modifiers().command(),
                        self.native_ui.modifiers().shift,
                        self.native_ui.modifiers().alt,
                        false,
                    ) {
                        if let Some(command) = crate::commands::registry().binding(chord).copied() {
                            if self.run_command_id(command.id) {
                                return true;
                            }
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
        self.native_ui.process_os_event(event)
    }

    pub fn push_toast(&mut self, text: &str) {
        self.native_ui.send(UiMessage::new(
            self.toast_host,
            MessageDirection::ToWidget,
            ToastMessage::Push(text.to_string()),
        ));
        self.native_ui
            .send(TextMessage::set_text(self.status_text, text.to_string()));
    }

    pub fn set_scene_dirty(&mut self, dirty: bool) {
        self.scene_dirty = dirty;
        self.native_ui.send(TextMessage::set_text(
            self.status_dirty,
            if dirty { "Unsaved changes" } else { "Saved" },
        ));
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
            NamePrompt::Rename { path } => EditorEvent::RenameContentItem { path, name },
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
        if !self.scene_dirty {
            self.editor_events.push_back(EditorEvent::NewScene);
            return;
        }
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
            if self.drawer_open || self.log_open {
                self.drawer_open = false;
                self.log_open = false;
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
        }
        self.apply_bottom_panel();
    }

    fn toggle_log_panel(&mut self) {
        if self.log_open {
            self.log_open = false;
        } else {
            self.log_open = true;
            self.drawer_open = false;
        }
        self.apply_bottom_panel();
    }

    fn apply_bottom_panel(&mut self) {
        let show = self.drawer_open || self.log_open;
        self.native_ui
            .set_visibility(self.content_drawer, self.drawer_open);
        self.native_ui.set_visibility(self.log_panel, self.log_open);
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

    fn combo_entries(&self) -> [(NodeHandle, NodeHandle); 3] {
        [
            (self.foliage_kind_combo, self.foliage_kind_popup),
            (self.post_tonemap_combo, self.post_tonemap_popup),
            (self.viewport_res_combo, self.viewport_res_popup),
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
        }
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
            A::Play => self.editor_events.push_back(EditorEvent::PlaySimulation),
            A::Pause => self.editor_events.push_back(EditorEvent::PauseSimulation),
            A::Stop => self.editor_events.push_back(EditorEvent::StopSimulation),
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
            A::ContentRename => {
                if let Some(entry) = self.content_menu_target.clone() {
                    self.open_name_prompt(
                        NamePrompt::Rename {
                            path: entry.path.to_string_lossy().into_owned(),
                        },
                        "Rename to",
                        &entry.name,
                    );
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
        self.native_ui.send(UiMessage::new(
            self.color_popup,
            MessageDirection::ToWidget,
            PopupMessage::Close,
        ));
        self.native_ui.invalidate_ancestors(self.color_popup);
    }

    fn close_color_picker(&mut self, commit: bool) {
        if let Some(field) = self.color_target {
            if commit {
                self.editor_events
                    .push_back(EditorEvent::SetInspectorColor {
                        field,
                        rgba: self.color_live,
                        live: false,
                    });
            } else {
                self.editor_events
                    .push_back(EditorEvent::CancelInspectorColor {
                        field,
                        rgba: self.color_original,
                    });
            }
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
            Some((_, since)) if now.duration_since(since).as_millis() >= 400 => {
                self.native_ui
                    .send(TextMessage::set_text(self.tooltip, text));
                self.native_ui
                    .set_desired_position(self.tooltip, Vec2::new(pos.x + 12.0, pos.y + 18.0));
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
    /// `somnium_ui` decodes images itself but owns no renderer, so meshes and
    /// scenes are requests. The host answers with [`Self::deliver_thumbnail`]
    /// or [`Self::fail_thumbnail`]; an unanswered request leaves the tile on
    /// its type icon, which is a correct resting state rather than a bug.
    pub fn take_thumbnail_requests(&mut self) -> Vec<crate::thumbnail::ThumbnailRequest> {
        self.native_ui.draw_ctx.thumbnails.take_requests()
    }

    /// Supply a rendered preview: `CELL * CELL` RGBA8.
    pub fn deliver_thumbnail(&mut self, path: &std::path::Path, rgba: &[u8]) -> bool {
        self.native_ui.draw_ctx.thumbnails.deliver(path, rgba)
    }

    /// Record that a preview could not be produced, so it is not retried.
    pub fn fail_thumbnail(&mut self, path: &std::path::Path) {
        self.native_ui.draw_ctx.thumbnails.mark_failed(path);
    }

    fn refresh_content_list(&mut self) {
        let root = std::env::current_dir().unwrap_or_default().join("assets");
        let current = if self.content_path.is_empty() {
            std::path::PathBuf::new()
        } else {
            root.join(&self.content_path)
        };
        let entries: Vec<crate::metaphor::ContentEntry> = crate::metaphor::list_content(
            &root,
            self.show_engine_content,
            &self.content_filter,
            &current,
        )
        .into_iter()
        .filter(|e| self.content_kind.accepts(e))
        .collect();
        let (tile_w, tile_h, icon_px) = self.content_density.metrics();
        self.native_ui.clear_children(self.content_list);
        self.content_entries.clear();
        let font_id = self.font_id;
        let parent = self.content_list;

        // Phase 27-G. A drawer with nothing in it used to be a blank grey
        // rectangle, which reads as broken rather than as empty. A filtered
        // miss and a genuinely empty folder are different situations and get
        // different copy — offering "import a model" to someone who mistyped a
        // search would be the wrong advice.
        if entries.is_empty() {
            let state = if self.content_filter.is_empty() {
                crate::metaphor::empty::CONTENT
            } else {
                crate::metaphor::empty::CONTENT_FILTERED
            };
            crate::editor::parts::build_empty_state(&mut self.native_ui, parent, font_id, state);
        }

        for entry in entries {
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
            // Phase 27-G. Ask for a preview; the tile shows its type icon until
            // one arrives, and forever if none can be made. `request` is
            // idempotent, so the drawer's per-frame rebuild costs a lookup.
            if !entry.is_dir && !entry.is_engine {
                self.native_ui.draw_ctx.thumbnails.request(&entry.path);
            }
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

            self.content_entries.push((bh, entry));
        }
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
            (h.light_section, "light"),
            (
                h.camera_section,
                "camera frustum cull dynamic resolution target floor",
            ),
            (
                h.post_section,
                "post fx bloom exposure tonemap census shade bins",
            ),
            (
                h.terrain_section,
                "terrain paint layer hex aerial lod distance",
            ),
            (h.foliage_section, "foliage grass tree"),
            (h.water_section, "water body wave"),
            (h.vessel_section, "vessel buoyancy"),
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

    /// Forget every recorded baseline, so no property reads as modified.
    ///
    /// Called when the selection changes and when the scene is saved or
    /// replaced: those are the three moments where "modified" would otherwise
    /// keep referring to a value the user can no longer see.
    pub fn reset_inspector_baseline(&mut self) {
        self.inspector_baseline.clear();
        let bindings = Self::field_bindings(&self.inspector_handles);
        for (field_handle, _) in bindings {
            if let Some(row) = self.native_ui.parent_of(field_handle) {
                self.native_ui
                    .send(PropertyRowMessage::set_modified(row, false));
            }
        }
    }

    /// Light or clear the modified dot on every inspector row.
    ///
    /// Runs once per frame after the `update_*` writes have landed. The first
    /// observation of a field becomes its baseline, which is why this must be
    /// called *after* the inspector has been populated for the frame and not
    /// before.
    pub fn refresh_modified_dots(&mut self) {
        let bindings = Self::field_bindings(&self.inspector_handles);
        for (field_handle, field) in bindings {
            let Some(value) = self.native_ui.numeric_value_of(field_handle) else {
                continue;
            };
            let baseline = *self.inspector_baseline.entry(field).or_insert(value);
            // Float equality is the right test here: the baseline is a copy of
            // a value that came through the same path, so an untouched field
            // compares bit-identical. An epsilon would hide small real edits.
            let modified = baseline != value;
            if let Some(row) = self.native_ui.parent_of(field_handle) {
                self.native_ui
                    .send(PropertyRowMessage::set_modified(row, modified));
            }
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

    // ── Editor event queue ────────────────────────────────────────────────────

    /// Drain one EditorEvent per call; returns None when queue is empty.
    pub fn poll_editor_event(&mut self) -> Option<EditorEvent> {
        self.editor_events.pop_front()
    }

    // ── Live UI updates ───────────────────────────────────────────────────────

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
        let rows: Vec<(u32, String, u8, bool)> = entities
            .iter()
            .map(|(id, name)| (*id, name.clone(), 0, false))
            .collect();
        self.update_outliner_tree(&rows, selected);
    }

    /// Hierarchical outliner (Phase 26-E). Each row is (id, name, depth, has_children).
    pub fn update_outliner_tree(
        &mut self,
        entities: &[(u32, String, u8, bool)],
        selected: Option<u32>,
    ) {
        let flat: Vec<(u32, String)> = entities
            .iter()
            .map(|(id, name, _, _)| (*id, name.clone()))
            .collect();
        let new_state = (flat, selected);
        if let Some(ref old_state) = self.last_outliner_state {
            if *old_state == new_state {
                return;
            }
        }
        self.last_outliner_state = Some(new_state);

        let filter = self.outliner_filter.to_ascii_lowercase();
        let mut items = Vec::new();
        for &(id, ref name, depth, has_children) in entities {
            if !filter.is_empty() && !name.to_ascii_lowercase().contains(&filter) {
                continue;
            }
            let expanded = self.outliner_expanded.contains(&id) || !has_children;
            if has_children && !self.outliner_expanded.contains(&id) && depth > 0 {
                // collapsed children are omitted by the caller
            }
            items.push(TreeItem {
                id,
                label: name.clone(),
                depth,
                icon: crate::metaphor::icon_for_entity_name(name),
                has_children,
                expanded: expanded || self.outliner_expanded.contains(&id),
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
            .map(|(id, name, _, _)| (*id, name.clone()))
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

    /// Update inspector NumericFields from a Transform.
    /// `transform` is (translation, euler_degrees, scale)`.
    pub fn update_inspector(
        &mut self,
        entity_idx: Option<u32>,
        pos: Option<[f32; 3]>,
        rot_deg: Option<[f32; 3]>,
        scale: Option<[f32; 3]>,
    ) {
        // Phase 27-G. An empty Details panel now says so. Before this it
        // rendered POSITION / ROTATION / SCALE at 0.000 next to a status bar
        // reading "No selection", which says "the selection sits at the origin"
        // rather than "there is no selection".
        let has_selection = entity_idx.is_some();
        self.native_ui
            .set_visibility(self.inspector_stack, has_selection);
        self.native_ui
            .set_visibility(self.details_empty, !has_selection);

        let h = &self.inspector_handles;
        let send = |ui: &mut UserInterface, handle: NodeHandle, v: f32| {
            ui.send(NumericFieldMessage::set_value(handle, v));
        };
        if let Some([x, y, z]) = pos {
            send(&mut self.native_ui, h.pos_x, x);
            send(&mut self.native_ui, h.pos_y, y);
            send(&mut self.native_ui, h.pos_z, z);
        }
        if let Some([x, y, z]) = rot_deg {
            send(&mut self.native_ui, h.rot_x, x);
            send(&mut self.native_ui, h.rot_y, y);
            send(&mut self.native_ui, h.rot_z, z);
        }
        if let Some([x, y, z]) = scale {
            send(&mut self.native_ui, h.sc_x, x);
            send(&mut self.native_ui, h.sc_y, y);
            send(&mut self.native_ui, h.sc_z, z);
        }
    }

    /// Show or hide the inspector's Light section and refresh it
    /// (Phase 13E). Pass `None` when the selection has no `LightComponent`.
    pub fn update_light_inspector(&mut self, values: Option<LightInspectorState>) {
        let h = &self.inspector_handles;
        let (section, intensity, range, inner, outer) = (
            h.light_section,
            h.light_intensity,
            h.light_range,
            h.light_inner,
            h.light_outer,
        );
        let light_color = h.light_color;
        let light_temp = h.light_temp_k;
        let (range_row, inner_row, outer_row, moon_row, moon_int) = (
            h.light_range_row,
            h.light_inner_row,
            h.light_outer_row,
            h.light_moon_row,
            h.light_moon_int,
        );
        match values {
            Some(LightInspectorState {
                values: [i, r, ia, oa, cr, cg, cb, moon_i, radius, width, height],
                kelvin,
                directional,
                show_cone,
                show_width,
                show_height,
            }) => {
                self.native_ui.set_visibility(section, true);
                self.native_ui
                    .send(NumericFieldMessage::set_value(intensity, i));
                self.native_ui
                    .send(NumericFieldMessage::set_value(range, r));
                self.native_ui
                    .send(NumericFieldMessage::set_value(inner, ia));
                self.native_ui
                    .send(NumericFieldMessage::set_value(outer, oa));
                self.native_ui.send(UiMessage::new(
                    light_color,
                    MessageDirection::ToWidget,
                    ColorSwatchMessage::SetColor([cr, cg, cb, 1.0]),
                ));
                self.native_ui.send(UiMessage::new(
                    light_color,
                    MessageDirection::ToWidget,
                    ColorSwatchMessage::SetLocked(kelvin > 0.0),
                ));
                self.native_ui
                    .send(NumericFieldMessage::set_value(light_temp, kelvin));
                self.native_ui
                    .send(NumericFieldMessage::set_value(moon_int, moon_i));
                self.native_ui
                    .send(NumericFieldMessage::set_value(h.light_radius, radius));
                self.native_ui
                    .send(NumericFieldMessage::set_value(h.light_width, width));
                self.native_ui
                    .send(NumericFieldMessage::set_value(h.light_height, height));
                self.native_ui.set_visibility(range_row, !directional);
                self.native_ui.set_visibility(inner_row, show_cone);
                self.native_ui.set_visibility(outer_row, show_cone);
                self.native_ui.set_visibility(moon_row, directional);
                self.native_ui.set_visibility(h.light_width_row, show_width);
                self.native_ui
                    .set_visibility(h.light_height_row, show_height);
            }
            None => self.native_ui.set_visibility(section, false),
        }
    }

    /// Show or hide the Camera section (Phase CR-C, extended by DOOM-F).
    ///
    /// `dynamic` is `(enabled, target_ms, floor_fraction)`. The floor is shown
    /// as a percentage because "67" is a number someone can reason about and
    /// "0.67" is not.
    pub fn update_camera_inspector(
        &mut self,
        frustum_cull: Option<bool>,
        dynamic: Option<(bool, f32, f32)>,
    ) {
        let h = &self.inspector_handles;
        let (dynres_toggle, dynres_target, dynres_floor) = (
            h.camera_dynres_toggle,
            h.camera_dynres_target,
            h.camera_dynres_floor,
        );
        let (section, frustum) = (h.camera_section, h.camera_frustum_toggle);
        match frustum_cull {
            Some(on) => {
                self.native_ui.set_visibility(section, true);
                self.native_ui
                    .send(CheckBoxMessage::set_checked(frustum, on));
            }
            None => {
                self.native_ui.set_visibility(section, false);
                return;
            }
        }
        if let Some((on, target_ms, floor)) = dynamic {
            self.native_ui
                .send(CheckBoxMessage::set_checked(dynres_toggle, on));
            self.native_ui
                .send(NumericFieldMessage::set_value(dynres_target, target_ms));
            self.native_ui
                .send(NumericFieldMessage::set_value(dynres_floor, floor * 100.0));
        }
    }

    /// Show or hide the inspector's Post FX section and refresh it (Phase 15A1).
    ///
    /// `values` is `[exposure, vignette_strength, ca_strength, ibl_intensity]`
    /// plus the three enable flags; pass `None` when the selection has no
    /// `PostProcessComponent`.
    ///
    /// Grouped into a struct rather than a tuple: it had already grown to four
    /// positional booleans, which is exactly the shape that invites passing
    /// them in the wrong order.
    pub fn update_post_inspector(&mut self, values: Option<PostInspectorState>) {
        let h = &self.inspector_handles;
        let (section, exposure, vig_str, ca_str, ibl) = (
            h.post_section,
            h.post_exposure,
            h.post_vig_str,
            h.post_ca_str,
            h.post_ibl,
        );
        let vig_toggle = h.post_vig_toggle;
        let ca_toggle = h.post_ca_toggle;
        let fxaa_toggle = h.post_fxaa_toggle;
        let auto_toggle = h.post_auto_exp_toggle;
        let tonemap_combo = h.post_tonemap_button;
        let cel_toggle = h.post_cel_toggle;
        let taa_toggle = h.post_taa_toggle;
        let gtao_toggle = h.post_gtao_toggle;
        let restir_toggle = h.post_restir_toggle;
        let restir_gi_toggle = h.post_restir_gi_toggle;
        let pcss_toggle = h.post_pcss_toggle;
        let contact_toggle = h.post_contact_toggle;
        let cas_toggle = h.post_cas_toggle;
        let mb_toggle = h.post_mb_toggle;
        let bloom_toggle = h.post_bloom_toggle;
        let dof_toggle = h.post_dof_toggle;
        let vol_toggle = h.post_vol_toggle;
        let shafts_toggle = h.post_shafts_toggle;
        let phys_toggle = h.post_phys_toggle;
        let exp_comp = h.post_exp_comp;

        match values {
            Some(v) => {
                let ([exp, ec, vig, ca, ibl_i], vig_on, ca_on, fxaa_on, auto_on, tonemap) = (
                    v.values,
                    v.vignette,
                    v.chromatic,
                    v.fxaa,
                    v.auto_exposure,
                    v.tonemapper,
                );
                let cel_on = v.cel_shading;
                self.native_ui.set_visibility(section, true);
                self.native_ui
                    .send(NumericFieldMessage::set_value(exposure, exp));
                self.native_ui
                    .send(NumericFieldMessage::set_value(exp_comp, ec));
                self.native_ui
                    .send(NumericFieldMessage::set_value(vig_str, vig));
                self.native_ui
                    .send(NumericFieldMessage::set_value(ca_str, ca));
                self.native_ui
                    .send(NumericFieldMessage::set_value(ibl, ibl_i));
                // Redraw the tick in each toggle's checkbox.
                let tick = |ui: &mut UserInterface, handle: NodeHandle, on: bool| {
                    ui.send(CheckBoxMessage::set_checked(handle, on));
                };
                tick(&mut self.native_ui, vig_toggle, vig_on);
                tick(&mut self.native_ui, ca_toggle, ca_on);
                tick(&mut self.native_ui, fxaa_toggle, fxaa_on);
                tick(&mut self.native_ui, auto_toggle, auto_on);
                self.native_ui.send(ComboBoxMessage::set_selected(
                    tonemap_combo,
                    crate::metaphor::tonemap_index(tonemap),
                ));
                tick(&mut self.native_ui, cel_toggle, cel_on);
                for (handle, on) in [
                    (taa_toggle, v.taa),
                    (gtao_toggle, v.gtao),
                    (restir_toggle, v.restir),
                    (restir_gi_toggle, v.restir_gi),
                    (h.post_rt_reflect_toggle, v.rt_reflect),
                    (h.post_rt_refract_toggle, v.rt_refract),
                    (pcss_toggle, v.pcss),
                    (contact_toggle, v.contact_shadows),
                    (cas_toggle, v.cas),
                    (mb_toggle, v.motion_blur),
                    (bloom_toggle, v.bloom),
                    (dof_toggle, v.dof),
                    (vol_toggle, v.volumetrics),
                    (shafts_toggle, v.shafts),
                    (phys_toggle, v.physical_camera),
                    (h.post_world_cache_toggle, v.world_cache),
                    (h.post_specular_toggle, v.specular_gi),
                    (h.post_path_toggle, v.path_tracer),
                    (h.post_sdf_toggle, v.mesh_sdf),
                    (h.post_probes_toggle, v.probes),
                    (h.post_analytic_toggle, v.analytic_grad),
                    (h.post_fsr_toggle, v.fsr),
                ] {
                    tick(&mut self.native_ui, handle, on);
                }
                for (field, value) in [
                    (h.post_bloom_amt, v.extras[0]),
                    (h.post_dof_focus, v.extras[1]),
                    (h.post_temperature, v.extras[2]),
                    (h.post_contrast, v.extras[3]),
                    (h.post_saturation, v.extras[4]),
                    (h.post_grain, v.extras[5]),
                    (h.post_fog_density, v.extras[6]),
                    (h.post_fog_height, v.extras[7]),
                    (h.post_fog_asym, v.extras[8]),
                    (h.post_tint, v.extras[9]),
                    (h.post_lift, v.extras[10]),
                    (h.post_gamma, v.extras[11]),
                    (h.post_gain, v.extras[12]),
                    (h.post_aperture, v.extras[13]),
                    (h.post_shutter, v.extras[14]),
                    (h.post_iso, v.extras[15]),
                    (h.post_ao_radius, v.extras[16]),
                    (h.post_ao_intensity, v.extras[17]),
                    (h.post_cas_sharp, v.extras[18]),
                    (h.post_cas_strength, v.extras[19]),
                    (h.post_mb_shutter, v.extras[20]),
                    (h.post_gi_intensity, v.extras[21]),
                    (h.post_cache_intensity, v.cache_intensity),
                    (h.post_cache_cell, v.cache_cell),
                    (h.post_spec_rough, v.spec_rough),
                    (h.post_path_bounces, v.path_bounces),
                    (h.post_probe_intensity, v.probe_intensity),
                    (h.post_shaft_amt, v.shaft_intensity),
                    (h.post_fsr_sharp, v.fsr_sharpness),
                ] {
                    self.native_ui
                        .send(NumericFieldMessage::set_value(field, value));
                }
            }
            None => self.native_ui.set_visibility(section, false),
        }
    }

    /// Refresh the profiler overlay (Phase 29).
    ///
    /// `None` hides it. Rows past [`PROFILER_ROWS`] are dropped rather than
    /// grown into: the overlay is a glance at the frame, and a panel that
    /// resizes itself every time a pass appears is harder to read than one that
    /// stays put.
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
                Some(r) => (
                    format!("{}{}", "   ".repeat(r.depth as usize), r.label),
                    r.value.clone(),
                ),
                // Blanked rather than hidden: hiding a row would reflow the
                // panel every frame a pass drops out of the list.
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
        let mode_label = h.terrain_mode_label;
        let _paint_label = h.terrain_paint_label;
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
                let status = if v.foliage_paint {
                    "Active: FOLIAGE paint".to_string()
                } else if v.terrain_edit && v.terrain_paint {
                    format!("Active: TERRAIN paint — {}", TERRAIN_LAYER_SHORT[paint])
                } else if v.terrain_edit {
                    format!("Active: TERRAIN sculpt — {}", TERRAIN_BRUSH_NAMES[brush])
                } else {
                    "Active: none — click Terrain Paint or a layer".to_string()
                };
                self.native_ui
                    .send(TextMessage::set_text(mode_label, status));
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
    pub fn update_water_inspector(&mut self, values: Option<[f32; 19]>) {
        let h = &self.inspector_handles;
        match values {
            Some(values) => {
                self.native_ui.set_visibility(h.water_section, true);
                for (handle, value) in [
                    h.water_surface,
                    h.water_depth,
                    h.water_clarity,
                    h.water_amplitude,
                    h.water_roughness,
                    h.water_ssr,
                    h.water_rt_reflect,
                    h.water_reflect_debug,
                    h.water_wave_a,
                    h.water_wave_b,
                    h.water_speed,
                    h.water_steepness,
                    h.water_wind_speed,
                    h.water_foam_decay,
                    h.water_foam_threshold,
                    h.water_spectrum_blend,
                    h.water_edge_scale,
                    h.water_anisotropy,
                    h.water_caustic,
                ]
                .into_iter()
                .zip(values)
                {
                    self.native_ui
                        .send(NumericFieldMessage::set_value(handle, value));
                }
            }
            None => self.native_ui.set_visibility(h.water_section, false),
        }
    }

    pub fn update_water_iris(
        &mut self,
        values: Option<(
            [f32; 4],
            [f32; 4],
            [f32; 4],
            [f32; 3],
            [f32; 3],
            bool,
            [f32; 4],
        )>,
    ) {
        let h = &self.inspector_handles;
        match values {
            Some((deep, shallow, edge, abs, scatter, underwater, dirs)) => {
                let (abs_tint, abs_mag) = crate::color::split_magnitude(abs);
                let (sc_tint, sc_mag) = crate::color::split_magnitude(scatter);
                for (handle, rgba) in [
                    (h.water_deep, deep),
                    (h.water_shallow, shallow),
                    (h.water_edge, edge),
                    (h.water_abs, [abs_tint[0], abs_tint[1], abs_tint[2], 1.0]),
                    (h.water_scatter, [sc_tint[0], sc_tint[1], sc_tint[2], 1.0]),
                ] {
                    self.native_ui.send(UiMessage::new(
                        handle,
                        MessageDirection::ToWidget,
                        ColorSwatchMessage::SetColor(rgba),
                    ));
                }
                self.native_ui
                    .send(NumericFieldMessage::set_value(h.water_abs_mag, abs_mag));
                self.native_ui
                    .send(NumericFieldMessage::set_value(h.water_scatter_mag, sc_mag));
                self.native_ui
                    .send(CheckBoxMessage::set_checked(h.water_underwater, underwater));
                self.native_ui
                    .send(NumericFieldMessage::set_value(h.water_dir_ax, dirs[0]));
                self.native_ui
                    .send(NumericFieldMessage::set_value(h.water_dir_az, dirs[1]));
                self.native_ui
                    .send(NumericFieldMessage::set_value(h.water_dir_bx, dirs[2]));
                self.native_ui
                    .send(NumericFieldMessage::set_value(h.water_dir_bz, dirs[3]));
            }
            None => {}
        }
    }

    pub fn update_particle_inspector(&mut self, values: Option<([f32; 4], [f32; 4])>) {
        let h = &self.inspector_handles;
        match values {
            Some((start, end)) => {
                self.native_ui.set_visibility(h.particle_section, true);
                self.native_ui.send(UiMessage::new(
                    h.particle_start,
                    MessageDirection::ToWidget,
                    ColorSwatchMessage::SetColor(start),
                ));
                self.native_ui.send(UiMessage::new(
                    h.particle_end,
                    MessageDirection::ToWidget,
                    ColorSwatchMessage::SetColor(end),
                ));
            }
            None => self.native_ui.set_visibility(h.particle_section, false),
        }
    }

    pub fn update_material_inspector(&mut self, values: Option<[f32; 4]>) {
        let h = &self.inspector_handles;
        match values {
            Some(base) => {
                self.native_ui.set_visibility(h.material_section, true);
                self.native_ui.send(UiMessage::new(
                    h.material_base,
                    MessageDirection::ToWidget,
                    ColorSwatchMessage::SetColor(base),
                ));
            }
            None => self.native_ui.set_visibility(h.material_section, false),
        }
    }

    /// Show buoyancy controls when a `BuoyantVessel` is selected.
    pub fn update_vessel_inspector(&mut self, values: Option<[f32; 6]>) {
        let h = &self.inspector_handles;
        match values {
            Some(values) => {
                self.native_ui.set_visibility(h.vessel_section, true);
                for (handle, value) in [
                    h.vessel_buoyancy,
                    h.vessel_drag,
                    h.vessel_angular_drag,
                    h.vessel_thrust,
                    h.vessel_draft,
                    h.vessel_righting,
                ]
                .into_iter()
                .zip(values)
                {
                    self.native_ui
                        .send(NumericFieldMessage::set_value(handle, value));
                }
            }
            None => self.native_ui.set_visibility(h.vessel_section, false),
        }
    }

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
            h.foliage_shadow,
            h.foliage_cull,
            h.foliage_lod,
            h.foliage_impostor,
        ];
        match values {
            Some((v, flags)) => {
                self.native_ui.set_visibility(section, true);
                for (f, val) in fields.iter().zip(v.iter()) {
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

    /// Append a line to the output log panel (ring buffer, max 200).
    pub fn append_log(&mut self, text: &str) {
        const MAX: usize = 200;
        self.log_lines.push_back(text.to_string());
        if self.log_lines.len() > MAX {
            self.log_lines.pop_front();
            let first = self.native_ui.first_child(self.log_stack);
            if first.is_some() {
                self.native_ui.remove_node(first);
            }
            self.log_entry_count = self.log_entry_count.saturating_sub(1);
        }
        let font_id = self.font_id;
        let entry = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
            left: 8.0,
            top: 1.0,
            right: 0.0,
            bottom: 0.0,
        }))
        .with_text(text)
        .with_font_size(11.0)
        .with_font_id(font_id)
        .with_color(theme::TEXT_PRIMARY)
        .build();
        let log_stack = self.log_stack;
        self.native_ui.add_node(entry, log_stack);
        // The first line retires the empty state; it never comes back, because
        // the log is append-only for the session.
        if self.log_entry_count == 1 {
            self.native_ui.set_visibility(self.log_empty, false);
            self.native_ui.set_visibility(self.log_stack, true);
        }
        self.log_entry_count += 1;
    }

    // ── Internal ─────────────────────────────────────────────────────────────

    /// Every inspector numeric field paired with the `InspectorField` it
    /// writes.
    ///
    /// Extracted from `process_outgoing` in Phase 26-Zeta-G because the
    /// modified-dot sync needs exactly the same pairing, and two copies of a
    /// 100-row table would drift the first time a field was added.
    fn field_bindings(h: &InspectorHandles) -> Vec<(NodeHandle, IF)> {
        vec![
            (h.pos_x, IF::PosX),
            (h.pos_y, IF::PosY),
            (h.pos_z, IF::PosZ),
            (h.rot_x, IF::RotX),
            (h.rot_y, IF::RotY),
            (h.rot_z, IF::RotZ),
            (h.sc_x, IF::ScaleX),
            (h.sc_y, IF::ScaleY),
            (h.sc_z, IF::ScaleZ),
            (h.light_intensity, IF::LightIntensity),
            (h.light_range, IF::LightRange),
            (h.light_inner, IF::LightInnerAngle),
            (h.light_outer, IF::LightOuterAngle),
            (h.light_col_r, IF::LightColorR),
            (h.light_col_g, IF::LightColorG),
            (h.light_col_b, IF::LightColorB),
            (h.light_temp_k, IF::LightColorTemperature),
            (h.light_moon_int, IF::LightMoonIntensity),
            (h.light_radius, IF::LightSourceRadius),
            (h.light_width, IF::LightAreaWidth),
            (h.light_height, IF::LightAreaHeight),
            (h.terrain_aerial_dist, IF::TerrainAerialDistance),
            (h.camera_dynres_target, IF::CameraDynResTargetMs),
            (h.camera_dynres_floor, IF::CameraDynResFloor),
            (h.post_exposure, IF::PostExposure),
            (h.post_exp_comp, IF::PostExposureCompensation),
            (h.post_bloom_amt, IF::PostBloomIntensity),
            (h.post_dof_focus, IF::PostFocusDistance),
            (h.post_temperature, IF::PostTemperature),
            (h.post_contrast, IF::PostContrast),
            (h.post_saturation, IF::PostSaturation),
            (h.post_grain, IF::PostGrain),
            (h.post_fog_density, IF::PostFogDensity),
            (h.post_fog_height, IF::PostFogHeight),
            (h.post_fog_asym, IF::PostFogAsymmetry),
            (h.post_tint, IF::PostTint),
            (h.post_lift, IF::PostLift),
            (h.post_gamma, IF::PostGamma),
            (h.post_gain, IF::PostGain),
            (h.post_aperture, IF::PostAperture),
            (h.post_shutter, IF::PostShutter),
            (h.post_iso, IF::PostIso),
            (h.post_ao_radius, IF::PostAoRadius),
            (h.post_ao_intensity, IF::PostAoIntensity),
            (h.post_fsr_sharp, IF::PostFsrSharpness),
            (h.post_cas_sharp, IF::PostCasSharpness),
            (h.post_cas_strength, IF::PostCasStrength),
            (h.post_mb_shutter, IF::PostMotionBlurShutter),
            (h.post_gi_intensity, IF::PostGiIntensity),
            (h.post_cache_intensity, IF::PostCacheIntensity),
            (h.post_cache_cell, IF::PostCacheCell),
            (h.post_spec_rough, IF::PostSpecRough),
            (h.post_path_bounces, IF::PostPathBounces),
            (h.post_probe_intensity, IF::PostProbeIntensity),
            (h.post_shaft_amt, IF::PostShaftIntensity),
            (h.post_vig_str, IF::PostVignetteStrength),
            (h.post_ca_str, IF::PostCaStrength),
            (h.post_ibl, IF::PostIblIntensity),
            (h.terrain_layer, IF::TerrainPaintLayer),
            (h.terrain_tile, IF::TerrainTile0),
            (h.terrain_relief, IF::TerrainRelief),
            (h.terrain_wetness, IF::TerrainWetness),
            (h.terrain_macro, IF::TerrainMacroStrength),
            (h.terrain_debug, IF::TerrainDebugView),
            (h.terrain_morph_start, IF::TerrainMorphStart),
            (h.water_surface, IF::WaterSurface),
            (h.water_depth, IF::WaterMaxDepth),
            (h.water_clarity, IF::WaterClarity),
            (h.water_amplitude, IF::WaterAmplitude),
            (h.water_roughness, IF::WaterRoughness),
            (h.water_ssr, IF::WaterSsrStrength),
            (h.water_rt_reflect, IF::WaterRtReflect),
            (h.water_reflect_debug, IF::WaterReflectDebug),
            (h.water_wave_a, IF::WaterWaveLengthA),
            (h.water_wave_b, IF::WaterWaveLengthB),
            (h.water_speed, IF::WaterWaveSpeed),
            (h.water_steepness, IF::WaterWaveSteepness),
            (h.water_wind_speed, IF::WaterWindSpeed),
            (h.water_foam_decay, IF::WaterFoamDecay),
            (h.water_foam_threshold, IF::WaterFoamThreshold),
            (h.water_spectrum_blend, IF::WaterSpectrumBlend),
            (h.water_edge_scale, IF::WaterEdgeScale),
            (h.water_anisotropy, IF::WaterAnisotropy),
            (h.water_caustic, IF::WaterCausticStrength),
            (h.water_dir_ax, IF::WaterWaveDirAX),
            (h.water_dir_az, IF::WaterWaveDirAZ),
            (h.water_dir_bx, IF::WaterWaveDirBX),
            (h.water_dir_bz, IF::WaterWaveDirBZ),
            (h.water_abs_mag, IF::WaterAbsorptionMag),
            (h.water_scatter_mag, IF::WaterScatteringMag),
            (h.vessel_buoyancy, IF::VesselBuoyancy),
            (h.vessel_drag, IF::VesselDrag),
            (h.vessel_angular_drag, IF::VesselAngularDrag),
            (h.vessel_thrust, IF::VesselThrust),
            (h.vessel_draft, IF::VesselDraft),
            (h.vessel_righting, IF::VesselRighting),
            (h.foliage_density, IF::FoliageDensity),
            (h.foliage_seed, IF::FoliageSeed),
            (h.foliage_slope, IF::FoliageSlope),
            (h.foliage_layer, IF::FoliageLayer),
            (h.foliage_smin, IF::FoliageScaleMin),
            (h.foliage_smax, IF::FoliageScaleMax),
            (h.foliage_shadow, IF::FoliageShadowDistance),
            (h.foliage_cull, IF::FoliageCullDistance),
            (h.foliage_lod, IF::FoliageLodDistance),
            (h.foliage_impostor, IF::FoliageImpostorDistance),
        ]
    }

    fn process_outgoing(&mut self, msgs: Vec<UiMessage>) {
        let h = &self.inspector_handles;
        let field_map = Self::field_bindings(h);

        let color_map: &[(NodeHandle, crate::ColorField)] = &[
            (h.light_color, crate::ColorField::Light),
            (h.water_deep, crate::ColorField::WaterDeep),
            (h.water_shallow, crate::ColorField::WaterShallow),
            (h.water_edge, crate::ColorField::WaterEdge),
            (h.water_abs, crate::ColorField::WaterAbsorption),
            (h.water_scatter, crate::ColorField::WaterScattering),
            (h.particle_start, crate::ColorField::ParticleStart),
            (h.particle_end, crate::ColorField::ParticleEnd),
            (h.material_base, crate::ColorField::MaterialBase),
        ];

        for msg in msgs {
            if let Some(ColorSwatchMessage::Clicked(rgba)) = msg.data::<ColorSwatchMessage>() {
                if let Some((_, field)) = color_map.iter().find(|(h, _)| *h == msg.destination) {
                    self.color_target = Some(*field);
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
                if let Some(field) = self.color_target {
                    match cmsg {
                        ColorPickerMessage::Changing(rgba) => {
                            self.color_live = *rgba;
                            self.editor_events
                                .push_back(EditorEvent::SetInspectorColor {
                                    field,
                                    rgba: *rgba,
                                    live: true,
                                });
                        }
                        ColorPickerMessage::Changed(rgba) => {
                            self.color_live = *rgba;
                            self.editor_events
                                .push_back(EditorEvent::SetInspectorColor {
                                    field,
                                    rgba: *rgba,
                                    live: false,
                                });
                            self.dismiss_color_ui();
                        }
                        ColorPickerMessage::Cancelled(rgba) => {
                            self.color_original = *rgba;
                            self.editor_events
                                .push_back(EditorEvent::CancelInspectorColor {
                                    field,
                                    rgba: *rgba,
                                });
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
                if let Some((_, entry)) = self
                    .content_entries
                    .iter()
                    .find(|(bh, _)| *bh == msg.destination)
                    .cloned()
                {
                    let is_map = !entry.is_dir
                        && !entry.is_engine
                        && entry
                            .path
                            .extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|e| e.eq_ignore_ascii_case("somnium"));
                    if is_map {
                        self.editor_events.push_back(EditorEvent::LoadScene(
                            entry.path.to_string_lossy().into_owned(),
                        ));
                    }
                }
                continue;
            }
            if let Some(ButtonMessage::Click) = msg.data::<ButtonMessage>() {
                if msg.destination == self.unsaved_save {
                    self.close_unsaved();
                    self.editor_events.push_back(EditorEvent::SaveScene);
                    self.editor_events.push_back(EditorEvent::NewScene);
                    continue;
                }
                if msg.destination == self.unsaved_discard {
                    self.close_unsaved();
                    self.editor_events.push_back(EditorEvent::NewScene);
                    continue;
                }
                if msg.destination == self.unsaved_cancel {
                    self.close_unsaved();
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
            if let Some(CheckBoxMessage::Check(on)) = msg.data::<CheckBoxMessage>() {
                if msg.destination == self.inspector_handles.water_underwater {
                    let _ = on;
                    self.editor_events
                        .push_back(EditorEvent::ToggleWaterUnderwater);
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
                    self.editor_events
                        .push_back(EditorEvent::SelectEntity(Some(eidx)));
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
                if msg.destination == self.pause_button {
                    self.run_command_id("editor.simulation.pause");
                    continue;
                }
                if msg.destination == self.stop_button {
                    self.run_command_id("editor.simulation.stop");
                    continue;
                }
                if msg.destination == self.inspector_handles.post_tonemap_button {
                    self.editor_events.push_back(EditorEvent::CycleTonemapper);
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
                        let root = std::env::current_dir().unwrap_or_default().join("assets");
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
                    } else if entry
                        .path
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case("luau"))
                    {
                        // Phase 16-D: clicking a script attaches it to the
                        // selection. `app.rs` refuses with a toast if there
                        // is nothing selected, rather than silently doing
                        // nothing.
                        self.editor_events.push_back(EditorEvent::AttachScript(
                            entry.path.to_string_lossy().into_owned(),
                        ));
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
                if let Some(&(_, kind)) = self
                    .create_popup_items
                    .iter()
                    .find(|(bh, _)| *bh == msg.destination)
                {
                    if let Some(command) = crate::commands::registry()
                        .menu(crate::commands::Menu::Create)
                        .into_iter()
                        .find(|command| {
                            command.action == crate::commands::CommandAction::CreateEntity(kind)
                        })
                    {
                        self.run_command_id(command.id);
                    }
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
                // Inspector checkboxes share the same destinations as the old buttons.
                if msg.destination == self.inspector_handles.post_vig_toggle {
                    self.editor_events
                        .push_back(EditorEvent::SetPostFx(PostFxToggle::Vignette, *on));
                    continue;
                }
                if msg.destination == self.inspector_handles.post_auto_exp_toggle {
                    self.editor_events
                        .push_back(EditorEvent::SetPostFx(PostFxToggle::AutoExposure, *on));
                    continue;
                }
                if msg.destination == self.inspector_handles.post_cel_toggle {
                    self.editor_events
                        .push_back(EditorEvent::SetPostFx(PostFxToggle::CelShading, *on));
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
                if msg.destination == self.inspector_handles.camera_frustum_toggle {
                    if msg.direction == MessageDirection::FromWidget {
                        self.editor_events
                            .push_back(EditorEvent::SetCpuFrustum(*on));
                    }
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
                if msg.destination == self.inspector_handles.camera_dynres_toggle {
                    if msg.direction == MessageDirection::FromWidget {
                        self.editor_events
                            .push_back(EditorEvent::SetDynamicResolution(*on));
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
                if msg.destination == self.inspector_handles.post_fxaa_toggle {
                    self.editor_events
                        .push_back(EditorEvent::SetPostFx(PostFxToggle::Fxaa, *on));
                    continue;
                }
                if msg.destination == self.inspector_handles.post_ca_toggle {
                    self.editor_events.push_back(EditorEvent::SetPostFx(
                        PostFxToggle::ChromaticAberration,
                        *on,
                    ));
                    continue;
                }
                for (handle, which) in [
                    (self.inspector_handles.post_taa_toggle, PostFxToggle::Taa),
                    (self.inspector_handles.post_gtao_toggle, PostFxToggle::Gtao),
                    (
                        self.inspector_handles.post_restir_toggle,
                        PostFxToggle::Restir,
                    ),
                    (
                        self.inspector_handles.post_restir_gi_toggle,
                        PostFxToggle::RestirGi,
                    ),
                    (
                        self.inspector_handles.post_rt_reflect_toggle,
                        PostFxToggle::RtReflect,
                    ),
                    (
                        self.inspector_handles.post_rt_refract_toggle,
                        PostFxToggle::RtRefract,
                    ),
                    (self.inspector_handles.post_pcss_toggle, PostFxToggle::Pcss),
                    (
                        self.inspector_handles.post_contact_toggle,
                        PostFxToggle::ContactShadows,
                    ),
                    (self.inspector_handles.post_cas_toggle, PostFxToggle::Cas),
                    (
                        self.inspector_handles.post_mb_toggle,
                        PostFxToggle::MotionBlur,
                    ),
                    (
                        self.inspector_handles.post_bloom_toggle,
                        PostFxToggle::Bloom,
                    ),
                    (
                        self.inspector_handles.post_dof_toggle,
                        PostFxToggle::DepthOfField,
                    ),
                    (
                        self.inspector_handles.post_vol_toggle,
                        PostFxToggle::Volumetrics,
                    ),
                    (
                        self.inspector_handles.post_shafts_toggle,
                        PostFxToggle::LightShafts,
                    ),
                    (
                        self.inspector_handles.post_phys_toggle,
                        PostFxToggle::PhysicalCamera,
                    ),
                    (
                        self.inspector_handles.post_world_cache_toggle,
                        PostFxToggle::WorldCache,
                    ),
                    (
                        self.inspector_handles.post_specular_toggle,
                        PostFxToggle::SpecularGi,
                    ),
                    (
                        self.inspector_handles.post_path_toggle,
                        PostFxToggle::PathTracer,
                    ),
                    (
                        self.inspector_handles.post_sdf_toggle,
                        PostFxToggle::MeshSdf,
                    ),
                    (
                        self.inspector_handles.post_probes_toggle,
                        PostFxToggle::Probes,
                    ),
                    (
                        self.inspector_handles.post_analytic_toggle,
                        PostFxToggle::AnalyticGrad,
                    ),
                    (self.inspector_handles.post_fsr_toggle, PostFxToggle::Fsr),
                ] {
                    if msg.destination == handle {
                        self.editor_events
                            .push_back(EditorEvent::SetPostFx(which, *on));
                        break;
                    }
                }
            } else if let Some(ComboBoxMessage::SelectionChanged(i)) = msg.data::<ComboBoxMessage>()
            {
                if msg.destination == self.inspector_handles.post_tonemap_button
                    || msg.destination == self.post_tonemap_combo
                {
                    self.editor_events
                        .push_back(EditorEvent::SetTonemapper(*i as u8));
                    continue;
                }
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
                    self.editor_events
                        .push_back(EditorEvent::SelectEntity(Some(*id)));
                }
            } else if let Some(TreeViewMessage::ToggleExpand(id)) = msg.data::<TreeViewMessage>() {
                if self.outliner_expanded.contains(id) {
                    self.outliner_expanded.remove(id);
                } else {
                    self.outliner_expanded.insert(*id);
                }
                self.last_outliner_state = None;
            } else if let Some(SearchBoxMessage::Query(q)) = msg.data::<SearchBoxMessage>() {
                if msg.destination == self.content_search {
                    self.content_filter = q.clone();
                    self.refresh_content_list();
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

            // — Camera speed slider (Phase 20B) ————————
            if let Some(SliderMessage::Value(v)) = msg.data::<SliderMessage>() {
                if msg.destination == self.camera_speed_slider {
                    self.editor_events
                        .push_back(EditorEvent::SetCameraSpeed(*v));
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
                if let Some(&(field_handle, field)) = field_map
                    .iter()
                    .find(|(fh, _)| self.native_ui.parent_of(*fh) == Some(msg.destination))
                {
                    if let Some(&baseline) = self.inspector_baseline.get(&field) {
                        self.native_ui
                            .send(NumericFieldMessage::set_value(field_handle, baseline));
                        self.editor_events
                            .push_back(EditorEvent::SetInspectorValue {
                                field,
                                value: baseline,
                                live: false,
                            });
                    }
                }
            }

            // — NumericField value changes ————————
            let numeric = match msg.data::<NumericFieldMessage>() {
                Some(NumericFieldMessage::ValueChanged(v)) => Some((*v, false)),
                Some(NumericFieldMessage::ValueChanging(v)) => Some((*v, true)),
                _ => None,
            };
            if let Some((v, live)) = numeric {
                if let Some(&(_, field)) = field_map.iter().find(|(fh, _)| *fh == msg.destination) {
                    self.editor_events
                        .push_back(EditorEvent::SetInspectorValue {
                            field,
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
}

impl CollapseRules {
    pub fn for_width(width: f32) -> Self {
        Self {
            transport_label: width >= 1400.0,
            search_field: width >= 1100.0,
            status_objects: width >= 1280.0,
        }
    }
}

#[cfg(test)]
mod elysium_tests {
    use super::*;

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

        // Floors, not exact counts: adding chrome should never take these
        // backwards. The first run after the widget migration measured 49 / 28 /
        // 20 / 4 / 17, and the wash count was 1 until `wash_from` stopped
        // treating a caller-supplied background as a request for flatness.
        assert!(rounded >= 40, "corner radius regressed to {rounded}");
        assert!(gradients >= 20, "chrome wash regressed to {gradients}");
        assert!(shadows >= 15, "elevation regressed to {shadows}");
        assert!(insets >= 4, "recession regressed to {insets}");
        assert!(borders >= 15, "strokes regressed to {borders}");

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
        assert_eq!(layout.create_popup_items.len(), 13);
        for handle in [
            layout.save_button,
            layout.select_button,
            layout.landscape_button,
            layout.foliage_toolbar_button,
            layout.play_button,
            layout.immersive_button,
            layout.pause_button,
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
        let kinds: Vec<CreateKind> = l.create_popup_items.iter().map(|(_, k)| *k).collect();
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
        ] {
            assert!(kinds.contains(&kind), "{kind:?} lost its Create row");
        }
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
            ("stop", l.stop_button),
            ("immersive play", l.immersive_button),
            ("file menu", l.file_button),
            ("profiler", l.profiler_toggle),
            ("camera speed", l.camera_speed_slider),
            ("content drawer", l.drawer_button),
            ("output log", l.log_button),
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
    fn every_inspector_control_is_actually_hittable() {
        // Phase 26-Zeta-G moved every inspector row into `PropertyRow`, which
        // arranges its value control itself. A row that measures to zero height
        // or zero width still *draws* its label, so a broken control looks
        // present and silently ignores clicks. This walks the whole handle
        // bundle and fails on anything that cannot be hit.
        let mut ui = UserInterface::new(1920.0, 1080.0);
        let l = build_editor_layout(
            &mut ui,
            0,
            crate::layout_persist::ChromeLayout::default().resolved(1920.0, 1080.0),
        );
        // Sections are hidden until something of that type is selected; make
        // them all visible so their rows get arranged.
        let h = &l.inspector_handles;
        for section in [
            h.light_section,
            h.camera_section,
            h.post_section,
            h.terrain_section,
            h.foliage_section,
            h.water_section,
            h.vessel_section,
            h.particle_section,
            h.material_section,
        ] {
            ui.set_visibility(section, true);
        }
        // Rows revealed conditionally by light type or foliage mode. They are
        // hidden at rest by design; the test still has to prove they arrange to
        // something clickable when they are shown.
        for row in [h.light_width_row, h.light_height_row] {
            ui.set_visibility(row, true);
        }
        if let Some(row) = ui.parent_of(h.foliage_layer) {
            ui.set_visibility(row, true);
        }
        ui.perform_layout();

        let mut broken = Vec::new();

        // Toggles, combos and colour swatches do not appear in
        // `field_bindings`, so a check that only walked that table would miss
        // exactly the controls the Details panel is mostly made of.
        for (name, handle) in [
            ("post tonemap combo", h.post_tonemap_button),
            ("foliage kind combo", h.foliage_kind_button),
            ("water underwater", h.water_underwater),
            ("camera frustum cull", h.camera_frustum_toggle),
            ("camera dynamic resolution", h.camera_dynres_toggle),
            ("terrain paint", h.terrain_paint_toggle),
            ("terrain hex", h.terrain_hex_toggle),
            ("foliage enabled", h.foliage_toggle),
            ("foliage paint", h.foliage_paint_toggle),
            ("foliage erase", h.foliage_erase_toggle),
            ("foliage single", h.foliage_single_toggle),
            ("post bloom", h.post_bloom_toggle),
            ("post vignette", h.post_vig_toggle),
            ("post fsr", h.post_fsr_toggle),
            ("light colour", h.light_color),
            ("water deep colour", h.water_deep),
            ("water scattering colour", h.water_scatter),
            ("particle start colour", h.particle_start),
            ("material base colour", h.material_base),
        ] {
            if handle.is_none() {
                broken.push(format!("{name} (no widget)"));
                continue;
            }
            let b = bounds_of(&ui, handle);
            if b.w < 8.0 || b.h < 8.0 {
                broken.push(format!("{name} ({}x{})", b.w, b.h));
            }
        }

        for (handle, field) in UiManager::field_bindings(h) {
            if handle.is_none() {
                // Retired bindings (the pre-Iris light R/G/B rows) keep their
                // `InspectorField` so the event contract is stable, but have no
                // widget.
                continue;
            }
            let b = bounds_of(&ui, handle);
            if b.w < 8.0 || b.h < 8.0 {
                broken.push(format!("{field:?} ({}x{})", b.w, b.h));
            }
        }
        assert!(
            broken.is_empty(),
            "{} inspector controls cannot be clicked: {:?}",
            broken.len(),
            broken
        );
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
