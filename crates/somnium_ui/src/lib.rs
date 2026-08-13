pub mod color;
pub mod draw;
pub mod editor_event;
pub mod font;
pub mod icons;
pub mod layout_persist;
pub mod message;
pub mod metaphor;
pub mod node;
pub mod pass;
pub mod pool;
pub mod runtime;
pub mod theme;
pub mod types;
pub mod ui;
pub mod widget;
pub mod widgets;

pub use editor_event::{ColorField, CreateKind, EditorEvent, InspectorField, PostFxToggle};
pub use node::CursorKind;
pub use runtime::UiCanvas;

use crate::{
    editor_event::InspectorField as IF,
    icons::IconId,
    message::{MessageDirection, NodeHandle, TextMessage, UiMessage},
    pass::UiPass,
    types::{HorizontalAlignment, Thickness, VerticalAlignment},
    ui::UserInterface,
    widget::WidgetBuilder,
    widgets::{
        border::BorderBuilder,
        button::{ButtonBuilder, ButtonMessage},
        check_box::{CheckBoxBuilder, CheckBoxMessage},
        color_picker::{
            ColorPickerBuilder, ColorPickerMessage, ColorSwatchBuilder, ColorSwatchMessage,
        },
        combo_box::{ComboBoxBuilder, ComboBoxMessage, ComboDropdownBuilder},
        command_palette::{CommandPaletteBuilder, CommandPaletteMessage, PaletteItem},
        context_menu::{ContextMenuBuilder, MenuItem},
        grid::{Column, GridBuilder, GridMessage, Row},
        image::ImageBuilder,
        menu::{MenuBuilder, MenuMessage},
        numeric_field::{NumericFieldBuilder, NumericFieldMessage},
        popup::{PopupBuilder, PopupMessage, PopupPlacement},
        scroll_viewer::ScrollViewerBuilder,
        search_box::{
            BreadcrumbBuilder, BreadcrumbMessage, SearchBoxBuilder, SearchBoxMessage,
            TooltipBuilder, build_property_row,
        },
        slider::{SliderBuilder, SliderMessage},
        splitter::{SplitterBuilder, SplitterMessage, SplitterOrientation},
        stack_panel::{Orientation, StackPanelBuilder},
        text::TextBuilder,
        toast::{ToastHostBuilder, ToastMessage},
        tree_view::{TreeItem, TreeViewBuilder, TreeViewMessage},
        wrap_panel::WrapPanelBuilder,
    },
};
use glam::Vec2;
use std::collections::VecDeque;
use std::sync::Arc;
use tracing::{info, warn};
use winit::event::{ElementState, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window};

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

/// Short paint-palette labels (Phase XV-I / XV-Zeta). Indices match the renderer roster.
const TERRAIN_LAYER_SHORT: [&str; 32] = [
    "Grass", "Forest", "Rock", "Snow", "Meadow", "Mud", "Coast", "Gravel", "DrySd", "DampSd",
    "Earth", "Clay", "Sparse", "Moss", "Cliff", "Talus", "Lawn", "Duff", "GrayRk", "Slate",
    "MossC", "Lime", "Loam", "Pine", "Wild", "Peat", "Gran", "Dune", "Lichen", "Autumn", "Path",
    "Crust",
];

const TERRAIN_BRUSH_NAMES: [&str; 6] = ["Raise", "Lower", "Smooth", "Flatten", "Noise", "Paint"];

pub type LightInspectorValues = [f32; 11];

// ── Layout build result ───────────────────────────────────────────────────────

struct EditorLayout {
    outliner_scroll: NodeHandle,
    outliner_stack: NodeHandle,
    inspector_stack: NodeHandle,
    log_stack: NodeHandle,
    create_button: NodeHandle,
    create_popup: NodeHandle,
    create_popup_items: Vec<(NodeHandle, CreateKind)>,
    file_button: NodeHandle,
    file_popup: NodeHandle,
    file_import_item: NodeHandle,
    file_new_item: NodeHandle,
    file_save_item: NodeHandle,
    camera_speed_slider: NodeHandle,
    camera_speed_label: NodeHandle,
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
    /// Phase 29 profiler overlay.
    profiler_panel: NodeHandle,
    profiler_toggle: NodeHandle,
    profiler_toggle_lbl: NodeHandle,
    profiler_names: Vec<NodeHandle>,
    profiler_values: Vec<NodeHandle>,
    outer_grid: NodeHandle,
    menu_bar_h: NodeHandle,
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
    edit_undo: NodeHandle,
    edit_redo: NodeHandle,
    edit_delete: NodeHandle,
    edit_dup: NodeHandle,
    view_profiler: NodeHandle,
    view_content: NodeHandle,
    window_dock_content: NodeHandle,
    help_open_item: NodeHandle,
    help_shortcuts: NodeHandle,
    help_about: NodeHandle,
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
    save_button: NodeHandle,
    palette_popup: NodeHandle,
    palette_widget: NodeHandle,
    toast_host: NodeHandle,
    unsaved_popup: NodeHandle,
    unsaved_save: NodeHandle,
    unsaved_discard: NodeHandle,
    unsaved_cancel: NodeHandle,
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
    window_size: (u32, u32),
    native_ui: UserInterface,
    ui_pass: UiPass,
    font_id: u8,
    // Live-update widget handles
    #[allow(dead_code)]
    outliner_scroll: NodeHandle,
    #[allow(dead_code)]
    outliner_stack: NodeHandle,
    #[allow(dead_code)]
    inspector_stack: NodeHandle,
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
    file_import_item: NodeHandle,
    file_new_item: NodeHandle,
    file_save_item: NodeHandle,
    // Viewport toolbar (Phase 20B): camera speed
    camera_speed_slider: NodeHandle,
    camera_speed_label: NodeHandle,
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
    edit_undo: NodeHandle,
    edit_redo: NodeHandle,
    edit_delete: NodeHandle,
    edit_dup: NodeHandle,
    view_profiler: NodeHandle,
    view_content: NodeHandle,
    window_dock_content: NodeHandle,
    help_open_item: NodeHandle,
    help_shortcuts: NodeHandle,
    help_about: NodeHandle,
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
    save_button: NodeHandle,
    palette_popup: NodeHandle,
    palette_widget: NodeHandle,
    toast_host: NodeHandle,
    unsaved_popup: NodeHandle,
    unsaved_save: NodeHandle,
    unsaved_discard: NodeHandle,
    unsaved_cancel: NodeHandle,
    color_popup: NodeHandle,
    color_picker: NodeHandle,
    help_open: bool,
    drawer_open: bool,
    #[allow(dead_code)]
    drawer_docked: bool,
    show_engine_content: bool,
    ctrl_held: bool,
    inspector_filter: String,
    content_filter: String,
    content_path: String,
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
    scene_dirty: bool,
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
        let (sw, sh) = (size.width as f32, size.height as f32);
        let mut native_ui = UserInterface::new(sw, sh);

        let font_bytes = include_bytes!("../assets/fonts/Inter-Regular.ttf");
        let font_id: u8 = match native_ui.add_font(font_bytes) {
            Ok(id) => {
                info!("Native UI: bundled Inter loaded (id={})", id);
                id
            }
            Err(e) => {
                warn!("Native UI: bundled Inter failed ({e}); trying system fonts");
                let fallback = std::fs::read("C:\\Windows\\Fonts\\segoeui.ttf")
                    .or_else(|_| std::fs::read("C:\\Windows\\Fonts\\arial.ttf"))
                    .or_else(|_| {
                        std::fs::read(
                            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
                        )
                    })
                    .ok();
                fallback
                    .and_then(|b| native_ui.add_font(&b).ok())
                    .unwrap_or(0)
            }
        };

        let mut layout_sizes = crate::layout_persist::load();
        if layout_sizes.tools < 120.0 {
            layout_sizes.tools = 128.0;
        }
        let layout = build_editor_layout(&mut native_ui, font_id, layout_sizes);
        let ui_pass = UiPass::new(device, queue, output_format);

        // Tell the UserInterface which handle is the viewport so mouse events pass through.
        native_ui.set_viewport_handle(layout.viewport_handle);

        let mut this = Self {
            window: Arc::clone(&window),
            window_size: (size.width, size.height),
            native_ui,
            ui_pass,
            font_id,
            outliner_scroll: layout.outliner_scroll,
            outliner_stack: layout.outliner_stack,
            inspector_stack: layout.inspector_stack,
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
            file_import_item: layout.file_import_item,
            file_new_item: layout.file_new_item,
            file_save_item: layout.file_save_item,
            camera_speed_slider: layout.camera_speed_slider,
            camera_speed_label: layout.camera_speed_label,
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
            edit_undo: layout.edit_undo,
            edit_redo: layout.edit_redo,
            edit_delete: layout.edit_delete,
            edit_dup: layout.edit_dup,
            view_profiler: layout.view_profiler,
            view_content: layout.view_content,
            window_dock_content: layout.window_dock_content,
            help_open_item: layout.help_open_item,
            help_shortcuts: layout.help_shortcuts,
            help_about: layout.help_about,
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
            save_button: layout.save_button,
            palette_popup: layout.palette_popup,
            palette_widget: layout.palette_widget,
            toast_host: layout.toast_host,
            unsaved_popup: layout.unsaved_popup,
            unsaved_save: layout.unsaved_save,
            unsaved_discard: layout.unsaved_discard,
            unsaved_cancel: layout.unsaved_cancel,
            color_popup: layout.color_popup,
            color_picker: layout.color_picker,
            help_open: false,
            drawer_open: true,
            drawer_docked: true,
            show_engine_content: false,
            ctrl_held: false,
            inspector_filter: String::new(),
            content_filter: String::new(),
            content_path: String::new(),
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
            scene_dirty: false,
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
        this.refresh_content_list();
        this
    }

    // ── Window integration ────────────────────────────────────────────────────

    pub fn reposition_panels(&mut self, window: &Window) {
        let size = window.inner_size();
        self.window_size = (size.width, size.height);
        self.native_ui.resize(size.width as f32, size.height as f32);
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
        let scale = window.scale_factor() as f32;
        self.native_ui.draw_ctx.font_atlas.set_dpi_scale(scale);

        // Flush all queued widget messages; convert outgoing to EditorEvents.
        let outgoing = self.native_ui.update();
        self.process_outgoing(outgoing);
        // Apply layout messages sent from those handlers (drawer row height, etc.)
        // before measure/draw, so a just-opened pane is never laid out at 0px.
        let extra = self.native_ui.update();
        self.process_outgoing(extra);

        let (w, h) = self.window_size;
        self.native_ui.perform_layout();
        self.reanchor_open_popups();
        self.update_tooltip();
        self.native_ui.perform_layout();
        self.native_ui.draw();
        window.set_cursor(self.native_ui.cursor_kind().to_winit());
        self.ui_pass
            .prepare(device, queue, &mut self.native_ui.draw_ctx, w, h);
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
            self.ctrl_held = m.state().control_key();
        }
        if let WindowEvent::CursorMoved { position, .. } = event {
            self.native_ui.cursor_pos = Vec2::new(position.x as f32, position.y as f32);
        }
        if let WindowEvent::KeyboardInput { event: key_ev, .. } = event {
            let pressed = key_ev.state == ElementState::Pressed;
            if let PhysicalKey::Code(code) = key_ev.physical_key {
                match code {
                    KeyCode::ControlLeft | KeyCode::ControlRight => {
                        self.ctrl_held = pressed;
                    }
                    KeyCode::F1 if pressed => {
                        self.toggle_help(None);
                        return true;
                    }
                    KeyCode::Space if pressed && self.ctrl_held => {
                        self.toggle_drawer();
                        return true;
                    }
                    KeyCode::KeyP if pressed && self.ctrl_held => {
                        self.toggle_palette();
                        return true;
                    }
                    KeyCode::Escape if pressed => {
                        if self.close_top_overlay() {
                            return true;
                        }
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
            GridMessage::SetRowSize(
                5,
                if show {
                    theme::BOTTOM_DRAWER_HEIGHT
                } else {
                    0.0
                },
            ),
        ));
        self.native_ui.send(TextMessage::set_text(
            self.status_text,
            if self.drawer_open {
                "Content Drawer".to_string()
            } else if self.log_open {
                "Output Log".to_string()
            } else {
                "Ready".to_string()
            },
        ));
        self.native_ui.invalidate_ancestors(self.outer_grid);
    }

    fn close_combo_dropdowns(&mut self) {
        if self.open_combo_popup.is_none() {
            return;
        }
        for (combo, popup) in [
            (self.foliage_kind_combo, self.foliage_kind_popup),
            (self.post_tonemap_combo, self.post_tonemap_popup),
        ] {
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
        if combo == self.foliage_kind_combo || combo == self.inspector_handles.foliage_kind_button {
            Some(self.foliage_kind_popup)
        } else if combo == self.post_tonemap_combo
            || combo == self.inspector_handles.post_tonemap_button
        {
            Some(self.post_tonemap_popup)
        } else {
            None
        }
    }

    fn close_top_overlay(&mut self) -> bool {
        if self.palette_open {
            self.close_palette();
            return true;
        }
        if self.color_open {
            self.close_color_picker(false);
            return true;
        }
        if self.unsaved_open {
            self.close_unsaved();
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
        false
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
        self.native_ui.set_focus(self.palette_widget);
        self.native_ui.invalidate_ancestors(self.palette_popup);
    }

    fn close_palette(&mut self) {
        self.palette_open = false;
        self.native_ui.set_focus(NodeHandle::NONE);
        self.native_ui.send(UiMessage::new(
            self.palette_popup,
            MessageDirection::ToWidget,
            PopupMessage::Close,
        ));
        self.native_ui.invalidate_ancestors(self.palette_popup);
    }

    fn run_palette_command(&mut self, idx: usize) {
        match idx {
            0 => self.prompt_unsaved_new(),
            1 => self.editor_events.push_back(EditorEvent::SaveScene),
            2 => self.editor_events.push_back(EditorEvent::ImportModel),
            3 => self.editor_events.push_back(EditorEvent::Undo),
            4 => self.editor_events.push_back(EditorEvent::Redo),
            5 => self.editor_events.push_back(EditorEvent::DeleteSelected),
            6 => self.editor_events.push_back(EditorEvent::DuplicateSelected),
            7 => self.editor_events.push_back(EditorEvent::PlaySimulation),
            8 => self.editor_events.push_back(EditorEvent::PauseSimulation),
            9 => self.editor_events.push_back(EditorEvent::StopSimulation),
            10 => self.editor_events.push_back(EditorEvent::ToggleProfiler),
            11 => self.toggle_drawer(),
            12 => self.toggle_help(Some(0)),
            13 => self
                .editor_events
                .push_back(EditorEvent::CreateEntity(CreateKind::Cube)),
            14 => self
                .editor_events
                .push_back(EditorEvent::CreateEntity(CreateKind::DirectionalLight)),
            _ => {}
        }
    }

    fn close_unsaved(&mut self) {
        self.unsaved_open = false;
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
            let (popup, anchor) = if self.open_combo_popup == self.foliage_kind_popup {
                (self.foliage_kind_popup, self.foliage_kind_combo)
            } else {
                (self.post_tonemap_popup, self.post_tonemap_combo)
            };
            self.native_ui.send(UiMessage::new(
                popup,
                MessageDirection::ToWidget,
                PopupMessage::SetAnchor(anchor),
            ));
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

    fn refresh_content_list(&mut self) {
        let root = std::env::current_dir().unwrap_or_default().join("assets");
        let current = if self.content_path.is_empty() {
            std::path::PathBuf::new()
        } else {
            root.join(&self.content_path)
        };
        let entries = crate::metaphor::list_content(
            &root,
            self.show_engine_content,
            &self.content_filter,
            &current,
        );
        self.native_ui.clear_children(self.content_list);
        self.content_entries.clear();
        let font_id = self.font_id;
        let parent = self.content_list;
        for entry in entries {
            let btn = ButtonBuilder::new(
                WidgetBuilder::new()
                    .with_width(112.0)
                    .with_height(120.0)
                    .with_background(theme::BG_RAISED),
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
                    .with_width(112.0)
                    .with_height(theme::ICON_DRAWER)
                    .with_margin(Thickness {
                        left: 0.0,
                        top: 8.0,
                        right: 0.0,
                        bottom: 0.0,
                    }),
            )
            .with_icon(entry.icon)
            .with_size(theme::ICON_DRAWER)
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
            (h.post_section, "post fx bloom exposure tonemap"),
            (h.terrain_section, "terrain paint layer hex"),
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
        self.outliner_rows = items.iter().map(|i| (self.outliner_tree, i.id)).collect();
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

    /// Update inspector NumericFields from a Transform.
    /// `transform` is (translation, euler_degrees, scale)`.
    pub fn update_inspector(
        &mut self,
        entity_idx: Option<u32>,
        pos: Option<[f32; 3]>,
        rot_deg: Option<[f32; 3]>,
        scale: Option<[f32; 3]>,
    ) {
        let _ = entity_idx;
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
    ///
    /// `values` is `[intensity, range, inner_deg, outer_deg, r, g, b, moon_intensity]`, paired
    /// with whether the light is directional.
    pub fn update_light_inspector(
        &mut self,
        values: Option<(LightInspectorValues, bool, f32, bool)>,
    ) {
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
            Some((
                [i, r, ia, oa, cr, cg, cb, moon_i, radius, width, height],
                directional,
                kelvin,
                rect,
            )) => {
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
                self.native_ui
                    .set_visibility(inner_row, !directional && !rect);
                self.native_ui
                    .set_visibility(outer_row, !directional && !rect);
                self.native_ui.set_visibility(moon_row, directional);
                self.native_ui.set_visibility(h.light_width_row, rect);
                self.native_ui.set_visibility(h.light_height_row, rect);
            }
            None => self.native_ui.set_visibility(section, false),
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
        self.log_entry_count += 1;
    }

    // ── Internal ─────────────────────────────────────────────────────────────

    fn process_outgoing(&mut self, msgs: Vec<UiMessage>) {
        let h = &self.inspector_handles;
        let field_map: &[(NodeHandle, IF)] = &[
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
        ];

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
            if let Some(CommandPaletteMessage::Run(idx)) = msg.data::<CommandPaletteMessage>() {
                self.run_palette_command(*idx);
                self.close_palette();
                continue;
            }
            if let Some(SplitterMessage::Changed(size)) = msg.data::<SplitterMessage>() {
                if msg.destination == self.inner_h {
                    self.chrome_layout.tools = *size;
                } else if msg.destination == self.content_split_h {
                    self.chrome_layout.viewport = *size;
                } else if msg.destination == self.right_split_h {
                    self.chrome_layout.outliner = *size;
                }
                crate::layout_persist::save(self.chrome_layout);
                continue;
            }
            if let Some(ButtonMessage::Click) = msg.data::<ButtonMessage>() {
                if msg.destination == self.file_new_item {
                    self.close_all_menus();
                    self.prompt_unsaved_new();
                    continue;
                }
                if msg.destination == self.file_save_item {
                    self.close_all_menus();
                    self.editor_events.push_back(EditorEvent::SaveScene);
                    continue;
                }
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
                    self.palette_open = false;
                }
                if msg.destination == self.unsaved_popup {
                    self.unsaved_open = false;
                }
            }

            if let Some(ButtonMessage::Click) = msg.data::<ButtonMessage>() {
                // Outliner row
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
                // File > Import Model (Phase 19B)
                if msg.destination == self.file_import_item {
                    self.editor_events.push_back(EditorEvent::ImportModel);
                    self.file_popup_open = false;
                    self.native_ui.send(UiMessage::new(
                        self.file_popup,
                        MessageDirection::ToWidget,
                        PopupMessage::Close,
                    ));
                    self.native_ui.invalidate_ancestors(self.file_popup);
                    continue;
                }
                // Post FX toggles (Phase 15A1)
                if msg.destination == self.inspector_handles.post_vig_toggle {
                    self.editor_events
                        .push_back(EditorEvent::TogglePostFx(PostFxToggle::Vignette));
                    continue;
                }
                // Phase 24A/24B exposure controls.
                if msg.destination == self.inspector_handles.post_auto_exp_toggle {
                    self.editor_events
                        .push_back(EditorEvent::TogglePostFx(PostFxToggle::AutoExposure));
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
                ] {
                    if msg.destination == handle {
                        self.editor_events
                            .push_back(EditorEvent::TogglePostFx(which));
                        break;
                    }
                }
                if msg.destination == self.profiler_toggle {
                    self.editor_events.push_back(EditorEvent::ToggleProfiler);
                    continue;
                }
                if msg.destination == self.play_button {
                    self.editor_events.push_back(EditorEvent::PlaySimulation);
                    continue;
                }
                if msg.destination == self.select_button {
                    self.editor_events.push_back(EditorEvent::SetGizmoMode(0));
                    continue;
                }
                if msg.destination == self.landscape_button {
                    self.editor_events.push_back(EditorEvent::ToggleTerrainEdit);
                    continue;
                }
                if msg.destination == self.foliage_toolbar_button {
                    self.editor_events.push_back(EditorEvent::ToggleFoliage);
                    continue;
                }
                if msg.destination == self.immersive_button {
                    self.editor_events
                        .push_back(EditorEvent::ToggleImmersiveViewport);
                    continue;
                }
                if msg.destination == self.pause_button {
                    self.editor_events.push_back(EditorEvent::PauseSimulation);
                    continue;
                }
                if msg.destination == self.stop_button {
                    self.editor_events.push_back(EditorEvent::StopSimulation);
                    continue;
                }
                if msg.destination == self.inspector_handles.post_cel_toggle {
                    self.editor_events
                        .push_back(EditorEvent::TogglePostFx(PostFxToggle::CelShading));
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
                        .push_back(EditorEvent::TogglePostFx(PostFxToggle::Fxaa));
                    continue;
                }
                if msg.destination == self.inspector_handles.post_ca_toggle {
                    self.editor_events
                        .push_back(EditorEvent::TogglePostFx(PostFxToggle::ChromaticAberration));
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
                if msg.destination == self.edit_undo {
                    self.editor_events.push_back(EditorEvent::Undo);
                    self.close_all_menus();
                    continue;
                }
                if msg.destination == self.edit_redo {
                    self.editor_events.push_back(EditorEvent::Redo);
                    self.close_all_menus();
                    continue;
                }
                if msg.destination == self.edit_delete {
                    self.editor_events.push_back(EditorEvent::DeleteSelected);
                    self.close_all_menus();
                    continue;
                }
                if msg.destination == self.edit_dup {
                    self.editor_events.push_back(EditorEvent::DuplicateSelected);
                    self.close_all_menus();
                    continue;
                }
                if msg.destination == self.view_profiler {
                    self.editor_events.push_back(EditorEvent::ToggleProfiler);
                    self.close_all_menus();
                    continue;
                }
                if msg.destination == self.view_content {
                    self.close_all_menus();
                    self.toggle_drawer();
                    continue;
                }
                if msg.destination == self.window_dock_content {
                    self.drawer_docked = true;
                    if !self.drawer_open {
                        self.toggle_drawer();
                    }
                    self.close_all_menus();
                    continue;
                }
                if msg.destination == self.help_open_item || msg.destination == self.help_button {
                    self.close_all_menus();
                    self.toggle_help(Some(0));
                    continue;
                }
                if msg.destination == self.help_shortcuts {
                    self.close_all_menus();
                    self.toggle_help(Some(2));
                    continue;
                }
                if msg.destination == self.help_about {
                    self.close_all_menus();
                    self.toggle_help(Some(4));
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
                    self.editor_events.push_back(EditorEvent::SaveScene);
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
                        if let Ok(rel) = entry.path.strip_prefix(&root) {
                            self.content_path = rel.to_string_lossy().into_owned();
                        }
                        self.refresh_content_list();
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
                    }
                    continue;
                }
                // Create popup item
                if let Some(&(_, kind)) = self
                    .create_popup_items
                    .iter()
                    .find(|(bh, _)| *bh == msg.destination)
                {
                    self.editor_events
                        .push_back(EditorEvent::CreateEntity(kind));
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
                if msg.destination == self.foliage_kind_popup
                    || msg.destination == self.post_tonemap_popup
                {
                    if let Some(combo) = if msg.destination == self.foliage_kind_popup {
                        Some(self.foliage_kind_combo)
                    } else {
                        Some(self.post_tonemap_combo)
                    } {
                        self.native_ui.send(ComboBoxMessage::close(combo));
                    }
                    self.open_combo_popup = NodeHandle::NONE;
                    self.native_ui.invalidate_ancestors(msg.destination);
                }
            } else if let Some(CheckBoxMessage::Check(_)) = msg.data::<CheckBoxMessage>() {
                if msg.destination == self.content_engine_toggle {
                    self.show_engine_content = !self.show_engine_content;
                    self.refresh_content_list();
                    continue;
                }
                // Inspector checkboxes share the same destinations as the old buttons.
                if msg.destination == self.inspector_handles.post_vig_toggle {
                    self.editor_events
                        .push_back(EditorEvent::TogglePostFx(PostFxToggle::Vignette));
                    continue;
                }
                if msg.destination == self.inspector_handles.post_auto_exp_toggle {
                    self.editor_events
                        .push_back(EditorEvent::TogglePostFx(PostFxToggle::AutoExposure));
                    continue;
                }
                if msg.destination == self.inspector_handles.post_cel_toggle {
                    self.editor_events
                        .push_back(EditorEvent::TogglePostFx(PostFxToggle::CelShading));
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
                        .push_back(EditorEvent::TogglePostFx(PostFxToggle::Fxaa));
                    continue;
                }
                if msg.destination == self.inspector_handles.post_ca_toggle {
                    self.editor_events
                        .push_back(EditorEvent::TogglePostFx(PostFxToggle::ChromaticAberration));
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
                ] {
                    if msg.destination == handle {
                        self.editor_events
                            .push_back(EditorEvent::TogglePostFx(which));
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
            } else if let Some(ComboBoxMessage::Open) = msg.data::<ComboBoxMessage>() {
                self.close_all_menus();
                if let Some(popup) = self.combo_popup_for(msg.destination) {
                    if self.open_combo_popup.is_some() && self.open_combo_popup != popup {
                        let other = self.open_combo_popup;
                        if other == self.foliage_kind_popup {
                            self.native_ui
                                .send(ComboBoxMessage::close(self.foliage_kind_combo));
                            self.native_ui.send(UiMessage::new(
                                self.foliage_kind_popup,
                                MessageDirection::ToWidget,
                                PopupMessage::Close,
                            ));
                        } else if other == self.post_tonemap_popup {
                            self.native_ui
                                .send(ComboBoxMessage::close(self.post_tonemap_combo));
                            self.native_ui.send(UiMessage::new(
                                self.post_tonemap_popup,
                                MessageDirection::ToWidget,
                                PopupMessage::Close,
                            ));
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
                    if *i == 0 {
                        self.content_path.clear();
                    } else {
                        let parts: Vec<&str> = self.content_path.split(['/', '\\']).collect();
                        self.content_path =
                            parts.iter().take(*i).copied().collect::<Vec<_>>().join("/");
                    }
                    self.refresh_content_list();
                }
            }

            // — Camera speed slider (Phase 20B) ————————
            if let Some(SliderMessage::Value(v)) = msg.data::<SliderMessage>() {
                if msg.destination == self.camera_speed_slider {
                    self.editor_events
                        .push_back(EditorEvent::SetCameraSpeed(*v));
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
                }
            }
        }
    }
}

// ── Editor layout builder ─────────────────────────────────────────────────────

fn build_editor_layout(
    ui: &mut UserInterface,
    font_id: u8,
    layout: crate::layout_persist::ChromeLayout,
) -> EditorLayout {
    let root = ui.root();

    // ── Outer grid: title | menu | toolbar | viewport bar | main | drawer | status
    let outer_grid = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::strict(theme::TITLEBAR_HEIGHT))
        .add_row(Row::strict(theme::MENU_HEIGHT))
        .add_row(Row::strict(theme::TOOLBAR_HEIGHT))
        .add_row(Row::strict(26.0))
        .add_row(Row::stretch())
        .add_row(Row::strict(theme::BOTTOM_DRAWER_HEIGHT))
        .add_row(Row::strict(theme::STATUS_HEIGHT))
        .add_column(Column::stretch())
        .build();
    let outer_h = ui.add_node(outer_grid, root);

    // ── Row 0: custom title bar ──────────────────────────────────────────────
    let title_bar = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(0)
            .with_background(theme::BG_VOID)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 0.0,
        top: 0.0,
        bottom: 1.0,
    })
    .build();
    let title_bar_h = ui.add_node(title_bar, outer_h);
    let title_grid = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::stretch())
        .add_column(Column::stretch())
        .add_column(Column::auto())
        .build();
    let title_grid_h = ui.add_node(title_grid, title_bar_h);

    let title_drag = BorderBuilder::new(
        WidgetBuilder::new()
            .with_column(0)
            .with_background(theme::TRANSPARENT)
            .with_foreground(theme::TRANSPARENT),
    )
    .with_stroke_thickness(Thickness::ZERO)
    .build();
    let title_drag = ui.add_node(title_drag, title_grid_h);
    let title_left =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Horizontal)
            .build();
    let title_left_h = ui.add_node(title_left, title_drag);
    let mark = ImageBuilder::new(
        WidgetBuilder::new()
            .with_width(36.0)
            .with_height(theme::TITLEBAR_HEIGHT)
            .with_margin(Thickness {
                left: 10.0,
                top: 4.0,
                right: 4.0,
                bottom: 0.0,
            }),
    )
    .with_icon(IconId::EngineMark)
    .with_size(theme::ICON_MARK)
    .with_tint(theme::ACCENT)
    .build();
    ui.add_node(mark, title_left_h);
    let title_lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
        left: 4.0,
        top: 10.0,
        right: 12.0,
        bottom: 0.0,
    }))
    .with_text("Somnium Engine")
    .with_font_size(13.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_PRIMARY)
    .build();
    ui.add_node(title_lbl, title_left_h);

    let title_right = StackPanelBuilder::new(
        WidgetBuilder::new()
            .with_column(1)
            .with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Horizontal)
    .build();
    let title_right_h = ui.add_node(title_right, title_grid_h);
    let fps_node = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(8.0, 10.0)))
        .with_text("— fps")
        .with_font_size(11.0)
        .with_font_id(font_id)
        .with_color(theme::TEXT_SECONDARY)
        .build();
    let fps_text = ui.add_node(fps_node, title_right_h);
    let win_min = window_chrome_button(ui, title_right_h, IconId::Minimize, "Minimize");
    let win_max = window_chrome_button(ui, title_right_h, IconId::Maximize, "Maximize");
    let win_close = window_chrome_button(ui, title_right_h, IconId::Close, "Close");

    // ── Row 1: menu bar ───────────────────────────────────────────────────────
    let menu_bar = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(1)
            .with_column(0)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 0.0,
        top: 0.0,
        bottom: 1.0,
    })
    .build();
    let menu_bar_h = ui.add_node(menu_bar, outer_h);

    // Menu bar grid: [stretch col for menu items | auto col for FPS counter]
    let menu_grid = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::stretch())
        .add_column(Column::stretch()) // col 0 — menu items
        .add_column(Column::auto()) // col 1 — FPS (right-aligned)
        .build();
    let menu_grid_h = ui.add_node(menu_grid, menu_bar_h);

    // Horizontal stack for menu items (col 0)
    let menu_stack = StackPanelBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(0)
            .with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Horizontal)
    .build();
    let menu_stack_h = ui.add_node(menu_stack, menu_grid_h);

    // "File" — Menu so clicks are captured (holds Import).
    let file_btn_node =
        MenuBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT)).build();
    let file_button = ui.add_node(file_btn_node, menu_stack_h);
    let file_lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(8.0, 6.0)))
        .with_text("File")
        .with_font_size(13.0)
        .with_font_id(font_id)
        .with_color(theme::TEXT_PRIMARY)
        .build();
    ui.add_node(file_lbl, file_button);

    let edit_button = menu_button(ui, menu_stack_h, "Edit", font_id);

    // "Create" — Menu so clicks are captured
    let create_btn_node = MenuBuilder::new(
        WidgetBuilder::new()
            .with_margin(Thickness {
                left: 0.0,
                right: 0.0,
                top: 0.0,
                bottom: 0.0,
            })
            .with_background(theme::TRANSPARENT),
    )
    .build();
    let create_button = ui.add_node(create_btn_node, menu_stack_h);
    let create_lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(8.0, 6.0)))
        .with_text("Create")
        .with_font_size(13.0)
        .with_font_id(font_id)
        .with_color(theme::TEXT_PRIMARY)
        .build();
    ui.add_node(create_lbl, create_button);

    let view_button = menu_button(ui, menu_stack_h, "View", font_id);
    let window_button = menu_button(ui, menu_stack_h, "Window", font_id);
    let help_menu_button = menu_button(ui, menu_stack_h, "Help", font_id);

    let fps_col = StackPanelBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(1)
            .with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Horizontal)
    .build();
    let fps_col_h = ui.add_node(fps_col, menu_grid_h);
    let help_button = icon_tool_button(ui, fps_col_h, IconId::HelpCircle, "Help (F1)");

    // ── Row 2: main toolbar ──────────────────────────────────────────────────
    let main_tb = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(2)
            .with_column(0)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 0.0,
        top: 0.0,
        bottom: 1.0,
    })
    .build();
    let main_tb_h = ui.add_node(main_tb, outer_h);
    let main_tb_stack =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Horizontal)
            .build();
    let main_tb_stack_h = ui.add_node(main_tb_stack, main_tb_h);
    let save_button = icon_tool_button(ui, main_tb_stack_h, IconId::Save, "Save (Ctrl+S)");
    let select_button = icon_tool_button(ui, main_tb_stack_h, IconId::Select, "Select (T)");
    let landscape_button =
        icon_tool_button(ui, main_tb_stack_h, IconId::Landscape, "Landscape (F6)");
    let foliage_toolbar_button =
        icon_tool_button(ui, main_tb_stack_h, IconId::Foliage, "Foliage (F8)");
    let play_button = icon_tool_button(ui, main_tb_stack_h, IconId::Play, "");
    let immersive_button = icon_tool_button(
        ui,
        main_tb_stack_h,
        IconId::ImmersivePlay,
        "Immersive play (Esc to exit)",
    );
    let pause_button = icon_tool_button(ui, main_tb_stack_h, IconId::Pause, "");
    let stop_button = icon_tool_button(ui, main_tb_stack_h, IconId::Stop, "");
    let play_label_n =
        TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(6.0, 5.0)))
            .with_text("Stopped")
            .with_font_size(11.0)
            .with_font_id(font_id)
            .with_color(theme::TEXT_SECONDARY)
            .build();
    let play_label = ui.add_node(play_label_n, main_tb_stack_h);
    let pause_label = play_label;
    let stop_label = play_label;

    // ── Row 3: viewport toolbar — camera speed (Phase 20B) ───────────────────
    let vp_bar = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(3)
            .with_column(0)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 0.0,
        top: 0.0,
        bottom: 1.0,
    })
    .build();
    let vp_bar_h = ui.add_node(vp_bar, outer_h);

    let vp_stack = StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .with_orientation(Orientation::Horizontal)
        .build();
    let vp_stack_h = ui.add_node(vp_stack, vp_bar_h);

    let cam_lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
        left: 10.0,
        top: 6.0,
        right: 6.0,
        bottom: 0.0,
    }))
    .with_text("Camera Speed")
    .with_font_size(11.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_SECONDARY)
    .build();
    ui.add_node(cam_lbl, vp_stack_h);

    let cam_slider_node = SliderBuilder::new(WidgetBuilder::new().with_width(140.0).with_margin(
        Thickness {
            left: 0.0,
            top: 4.0,
            right: 8.0,
            bottom: 0.0,
        },
    ))
    .with_value(0.5)
    .build();
    let camera_speed_slider = ui.add_node(cam_slider_node, vp_stack_h);

    // Numeric readout, updated as the slider (or RMB+wheel) changes speed.
    let cam_val = TextBuilder::new(
        WidgetBuilder::new()
            .with_width(84.0)
            .with_margin(Thickness {
                left: 0.0,
                top: 6.0,
                right: 0.0,
                bottom: 0.0,
            }),
    )
    .with_text("5.0 m/s")
    .with_font_size(11.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_PRIMARY)
    .build();
    let camera_speed_label = ui.add_node(cam_val, vp_stack_h);

    // Play/Pause/Stop live on the main toolbar (Phase 26-C).

    // Phase 29: the profiler switch lives on the viewport toolbar rather than
    // in a menu, because it is a thing you flick on and off while looking at
    // the scene — the same reason UE5 puts its stat toggles there.
    let (profiler_toggle, profiler_toggle_lbl) = labeled_icon_button(
        ui,
        vp_stack_h,
        IconId::Profiler,
        "Profiler",
        "Toggle GPU profiler overlay",
        font_id,
        22.0,
    );

    let hint = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
        left: 4.0,
        top: 6.0,
        right: 0.0,
        bottom: 0.0,
    }))
    .with_text("(RMB + scroll wheel)")
    .with_font_size(10.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_SECONDARY)
    .build();
    ui.add_node(hint, vp_stack_h);

    // ── Row 4: resizable columns — tools | viewport | details ────────────────
    let tools_split = SplitterBuilder::new(
        WidgetBuilder::new()
            .with_row(4)
            .with_column(0)
            .with_background(theme::TRANSPARENT),
    )
    .with_orientation(SplitterOrientation::Horizontal)
    .with_first_size(layout.tools)
    .with_min_first(120.0)
    .with_min_second(240.0)
    .build();
    let inner_h = ui.add_node(tools_split, outer_h);

    // Left toolbar strip
    let toolbar = BorderBuilder::new(
        WidgetBuilder::new()
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 1.0,
        top: 0.0,
        bottom: 0.0,
    })
    .build();
    let toolbar_h = ui.add_node(toolbar, inner_h);

    let content_split =
        SplitterBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(SplitterOrientation::Horizontal)
            .with_first_size(layout.viewport)
            .with_min_first(200.0)
            .with_min_second(180.0)
            .build();
    let content_split_h = ui.add_node(content_split, inner_h);

    // Terrain tool palette (Phase 14F): label + 6 brush mode buttons.
    // Active only while a terrain entity is selected (F6 toggles edit mode).
    let tool_stack =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let tool_stack_h = ui.add_node(tool_stack, toolbar_h);

    let ter_lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
        left: 8.0,
        top: 8.0,
        right: 0.0,
        bottom: 4.0,
    }))
    .with_text("Sculpt")
    .with_font_size(11.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_SECONDARY)
    .build();
    ui.add_node(ter_lbl, tool_stack_h);

    const TERRAIN_TOOLS: &[(IconId, &str, u8)] = &[
        (IconId::Landscape, "Raise", 0),
        (IconId::Landscape, "Lower", 1),
        (IconId::Landscape, "Smooth", 2),
        (IconId::Landscape, "Flatten", 3),
        (IconId::Landscape, "Noise", 4),
        (IconId::Texture, "Paint", 5),
    ];
    let mut terrain_tool_items = Vec::with_capacity(TERRAIN_TOOLS.len());
    for &(icon, label, tool) in TERRAIN_TOOLS {
        let btn = ButtonBuilder::new(
            WidgetBuilder::new()
                .with_height(28.0)
                .with_margin(Thickness {
                    left: 4.0,
                    top: 2.0,
                    right: 4.0,
                    bottom: 0.0,
                })
                .with_background(theme::BG_RAISED),
        )
        .build();
        let btn_h = ui.add_node(btn, tool_stack_h);
        let row = StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Horizontal)
            .build();
        let row_h = ui.add_node(row, btn_h);
        let img = ImageBuilder::new(WidgetBuilder::new().with_margin(Thickness {
            left: 4.0,
            top: 4.0,
            right: 2.0,
            bottom: 0.0,
        }))
        .with_icon(icon)
        .with_size(16.0)
        .with_tint(theme::TEXT_PRIMARY)
        .build();
        ui.add_node(img, row_h);
        let lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
            left: 2.0,
            top: 5.0,
            right: 4.0,
            bottom: 0.0,
        }))
        .with_text(label)
        .with_font_size(11.0)
        .with_font_id(font_id)
        .with_color(theme::TEXT_PRIMARY)
        .build();
        let lbl_h = ui.add_node(lbl, row_h);
        terrain_tool_items.push((btn_h, lbl_h, tool));
    }

    // Viewport area (col 1) — transparent, no hit-test. Mouse events in this region
    // will hit-test to this handle, which the UI knows to NOT consume.
    let viewport_border = BorderBuilder::new(
        WidgetBuilder::new()
            .with_background(theme::TRANSPARENT)
            .with_foreground(theme::TRANSPARENT),
    )
    .with_stroke_thickness(Thickness::ZERO)
    .build();
    let viewport_handle = ui.add_node(viewport_border, content_split_h);

    // ── Profiler overlay (Phase 29) ──────────────────────────────────────────
    // A child of the viewport, pinned top-left, so it floats over the render
    // instead of stealing layout from it. Rows are built once and rewritten
    // each frame: allocating twenty text nodes per frame to display a frame
    // timing would be its own entry in the table.
    let prof_panel = BorderBuilder::new(
        WidgetBuilder::new()
            .with_width(300.0)
            .with_margin(Thickness {
                left: 10.0,
                top: 10.0,
                right: 0.0,
                bottom: 0.0,
            })
            .with_horizontal_alignment(HorizontalAlignment::Left)
            .with_vertical_alignment(VerticalAlignment::Top)
            .with_background(theme::BG_DARK)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness::uniform(1.0))
    .build();
    let profiler_panel = ui.add_node(prof_panel, viewport_handle);

    let prof_stack =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let prof_stack_h = ui.add_node(prof_stack, profiler_panel);

    let prof_hdr = TextBuilder::new(WidgetBuilder::new().with_height(18.0).with_margin(
        Thickness {
            left: 8.0,
            top: 4.0,
            right: 0.0,
            bottom: 2.0,
        },
    ))
    .with_text("GPU PROFILER")
    .with_font_size(11.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_SECONDARY)
    .build();
    ui.add_node(prof_hdr, prof_stack_h);

    let mut profiler_names = Vec::with_capacity(PROFILER_ROWS);
    let mut profiler_values = Vec::with_capacity(PROFILER_ROWS);
    for _ in 0..PROFILER_ROWS {
        let row = StackPanelBuilder::new(
            WidgetBuilder::new()
                .with_height(15.0)
                .with_background(theme::TRANSPARENT),
        )
        .with_orientation(Orientation::Horizontal)
        .build();
        let row_h = ui.add_node(row, prof_stack_h);

        let name = TextBuilder::new(WidgetBuilder::new().with_width(190.0).with_margin(
            Thickness {
                left: 8.0,
                top: 1.0,
                right: 0.0,
                bottom: 0.0,
            },
        ))
        .with_text("")
        .with_font_size(11.0)
        .with_font_id(font_id)
        .with_color(theme::TEXT_SECONDARY)
        .build();
        profiler_names.push(ui.add_node(name, row_h));

        let value = TextBuilder::new(WidgetBuilder::new().with_width(92.0).with_margin(
            Thickness {
                left: 0.0,
                top: 1.0,
                right: 0.0,
                bottom: 0.0,
            },
        ))
        .with_text("")
        .with_font_size(11.0)
        .with_font_id(font_id)
        .with_color(theme::TEXT_PRIMARY)
        .build();
        profiler_values.push(ui.add_node(value, row_h));
    }
    ui.set_visibility(profiler_panel, false);

    // Right panel: two sections (outliner top, inspector bottom)
    let right_border = BorderBuilder::new(
        WidgetBuilder::new()
            .with_background(theme::BG_DARK)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 1.0,
        right: 0.0,
        top: 0.0,
        bottom: 0.0,
    })
    .build();
    let right_h = ui.add_node(right_border, content_split_h);

    let right_split =
        SplitterBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(SplitterOrientation::Vertical)
            .with_first_size(layout.outliner)
            .with_min_first(80.0)
            .with_min_second(120.0)
            .build();
    let right_split_h = ui.add_node(right_split, right_h);

    let out_grid = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::strict(24.0))
        .add_row(Row::strict(22.0))
        .add_row(Row::stretch())
        .add_column(Column::stretch())
        .build();
    let out_grid_h = ui.add_node(out_grid, right_split_h);

    let ins_grid = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::strict(24.0))
        .add_row(Row::strict(22.0))
        .add_row(Row::stretch())
        .add_column(Column::stretch())
        .build();
    let ins_grid_h = ui.add_node(ins_grid, right_split_h);

    // Outliner header
    let out_hdr = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(0)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 0.0,
        top: 0.0,
        bottom: 1.0,
    })
    .build();
    let out_hdr_h = ui.add_node(out_hdr, out_grid_h);
    let out_hdr_txt = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
        left: 8.0,
        top: 5.0,
        right: 0.0,
        bottom: 0.0,
    }))
    .with_text("OUTLINER")
    .with_font_size(11.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_SECONDARY)
    .build();
    ui.add_node(out_hdr_txt, out_hdr_h);

    let outliner_search = {
        let n = SearchBoxBuilder::new(
            WidgetBuilder::new()
                .with_row(1)
                .with_column(0)
                .with_background(theme::BG_INPUT),
        )
        .with_font_id(font_id)
        .build();
        ui.add_node(n, out_grid_h)
    };

    let out_scroll = ScrollViewerBuilder::new(
        WidgetBuilder::new()
            .with_row(2)
            .with_column(0)
            .with_background(theme::BG_DARK),
    )
    .build();
    let outliner_scroll = ui.add_node(out_scroll, out_grid_h);

    let outliner_tree = {
        let t = TreeViewBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_font_id(font_id)
            .build();
        ui.add_node(t, outliner_scroll)
    };
    let outliner_stack = outliner_tree;

    // Inspector header
    let ins_hdr = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(0)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 0.0,
        top: 1.0,
        bottom: 1.0,
    })
    .build();
    let ins_hdr_h = ui.add_node(ins_hdr, ins_grid_h);
    let ins_hdr_txt = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
        left: 8.0,
        top: 5.0,
        right: 0.0,
        bottom: 0.0,
    }))
    .with_text("DETAILS")
    .with_font_size(11.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_SECONDARY)
    .build();
    ui.add_node(ins_hdr_txt, ins_hdr_h);

    let inspector_search = {
        let n = SearchBoxBuilder::new(
            WidgetBuilder::new()
                .with_row(1)
                .with_column(0)
                .with_background(theme::BG_INPUT),
        )
        .with_font_id(font_id)
        .build();
        ui.add_node(n, ins_grid_h)
    };

    let ins_content = ScrollViewerBuilder::new(
        WidgetBuilder::new()
            .with_row(2)
            .with_column(0)
            .with_background(theme::BG_DARK),
    )
    .build();
    let ins_content_h = ui.add_node(ins_content, ins_grid_h);

    let inspector_stack =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let inspector_stack = ui.add_node(inspector_stack, ins_content_h);

    let inspector_handles = build_inspector(ui, inspector_stack, font_id);

    // ── Row 5: docked Content Drawer / Output Log ────────────────────────────
    let bottom = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(5)
            .with_column(0)
            .with_background(theme::BG_DARK)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 0.0,
        top: 1.0,
        bottom: 0.0,
    })
    .build();
    let bottom_h = ui.add_node(bottom, outer_h);

    let bottom_swap = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::stretch())
        .add_column(Column::stretch())
        .build();
    let bottom_swap_h = ui.add_node(bottom_swap, bottom_h);

    let (content_drawer, content_search, content_breadcrumb, content_engine_toggle, content_list) =
        build_content_drawer(ui, bottom_swap_h, font_id);

    let log_panel = GridBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(0)
            .with_background(theme::TRANSPARENT)
            .with_visibility(false),
    )
    .add_row(Row::strict(22.0))
    .add_row(Row::stretch())
    .add_column(Column::stretch())
    .build();
    let log_panel = ui.add_node(log_panel, bottom_swap_h);

    let log_hdr_border = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(0)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 0.0,
        top: 0.0,
        bottom: 1.0,
    })
    .build();
    let log_hdr_h = ui.add_node(log_hdr_border, log_panel);

    let log_header = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
        left: 8.0,
        top: 4.0,
        right: 0.0,
        bottom: 0.0,
    }))
    .with_text("Output Log")
    .with_font_size(11.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_SECONDARY)
    .build();
    ui.add_node(log_header, log_hdr_h);

    let log_scroll = ScrollViewerBuilder::new(
        WidgetBuilder::new()
            .with_row(1)
            .with_column(0)
            .with_background(theme::BG_DARK),
    )
    .build();
    let log_scroll_h = ui.add_node(log_scroll, log_panel);

    let log_stack_node =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let log_stack = ui.add_node(log_stack_node, log_scroll_h);

    // ── Row 6: status bar ────────────────────────────────────────────────────
    let status_bar = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(6)
            .with_column(0)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 0.0,
        top: 1.0,
        bottom: 0.0,
    })
    .build();
    let status_h = ui.add_node(status_bar, outer_h);
    let status_stack =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Horizontal)
            .build();
    let status_stack_h = ui.add_node(status_stack, status_h);
    let (drawer_button, _) = labeled_icon_button(
        ui,
        status_stack_h,
        IconId::ContentDrawer,
        "Content Drawer",
        "Show or hide the Content Drawer (Ctrl+Space)",
        font_id,
        theme::STATUS_HEIGHT,
    );
    let (log_button, _) = labeled_icon_button(
        ui,
        status_stack_h,
        IconId::OutputLog,
        "Output Log",
        "Show or hide the Output Log",
        font_id,
        theme::STATUS_HEIGHT,
    );
    let status_lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(8.0, 4.0)))
        .with_text("Content Drawer")
        .with_font_size(11.0)
        .with_font_id(font_id)
        .with_color(theme::TEXT_SECONDARY)
        .build();
    let status_text = ui.add_node(status_lbl, status_stack_h);

    // ── Popup overlays (children of root, drawn on top) ───────────────────────
    let (create_popup, create_popup_items) = build_create_popup(ui, root, font_id);
    let (file_popup, file_items) = popup_items(
        ui,
        root,
        font_id,
        &["New Scene", "Save Scene", "Import Model..."],
    );
    let file_new_item = file_items[0];
    let file_save_item = file_items[1];
    let file_import_item = file_items[2];
    let (edit_popup, edit_items) =
        popup_items(ui, root, font_id, &["Undo", "Redo", "Delete", "Duplicate"]);
    let edit_undo = edit_items[0];
    let edit_redo = edit_items[1];
    let edit_delete = edit_items[2];
    let edit_dup = edit_items[3];
    let (view_popup, view_items) = popup_items(ui, root, font_id, &["Profiler", "Content Drawer"]);
    let view_profiler = view_items[0];
    let view_content = view_items[1];
    let (window_popup, window_items) = popup_items(ui, root, font_id, &["Show Content Drawer"]);
    let window_dock_content = window_items[0];
    let (help_menu_popup, help_items) = popup_items(
        ui,
        root,
        font_id,
        &["Help Overlay (F1)", "Shortcuts", "About"],
    );
    let help_open_item = help_items[0];
    let help_shortcuts = help_items[1];
    let help_about = help_items[2];

    let (help_overlay, help_body, help_toc, help_close) = build_help_overlay(ui, root, font_id);

    let tooltip_node = TooltipBuilder::new(
        WidgetBuilder::new()
            .with_visibility(false)
            .with_desired_position(Vec2::new(0.0, 0.0)),
    )
    .with_font_id(font_id)
    .build();
    let tooltip = ui.add_node(tooltip_node, root);

    let _ctx = ContextMenuBuilder::new(
        WidgetBuilder::new()
            .with_visibility(false)
            .with_desired_position(Vec2::new(0.0, 0.0)),
    )
    .with_items(vec![
        MenuItem {
            id: 1,
            label: "Duplicate".into(),
            enabled: true,
        },
        MenuItem {
            id: 2,
            label: "Delete".into(),
            enabled: true,
        },
    ])
    .with_font_id(font_id)
    .build();
    let _ = ui.add_node(_ctx, root);

    let palette_items = vec![
        PaletteItem {
            label: "New Scene".into(),
            hint: "Ctrl+N".into(),
        },
        PaletteItem {
            label: "Save Scene".into(),
            hint: "Ctrl+S".into(),
        },
        PaletteItem {
            label: "Import Model…".into(),
            hint: String::new(),
        },
        PaletteItem {
            label: "Undo".into(),
            hint: "Ctrl+Z".into(),
        },
        PaletteItem {
            label: "Redo".into(),
            hint: "Ctrl+Y".into(),
        },
        PaletteItem {
            label: "Delete".into(),
            hint: "Del".into(),
        },
        PaletteItem {
            label: "Duplicate".into(),
            hint: "Ctrl+D".into(),
        },
        PaletteItem {
            label: "Play".into(),
            hint: String::new(),
        },
        PaletteItem {
            label: "Pause".into(),
            hint: String::new(),
        },
        PaletteItem {
            label: "Stop".into(),
            hint: String::new(),
        },
        PaletteItem {
            label: "Toggle Profiler".into(),
            hint: String::new(),
        },
        PaletteItem {
            label: "Content Drawer".into(),
            hint: "Ctrl+Space".into(),
        },
        PaletteItem {
            label: "Help".into(),
            hint: "F1".into(),
        },
        PaletteItem {
            label: "Create Cube".into(),
            hint: String::new(),
        },
        PaletteItem {
            label: "Create Directional Light".into(),
            hint: String::new(),
        },
    ];
    let palette_popup_node = PopupBuilder::new(WidgetBuilder::new().with_background([0, 0, 0, 80]))
        .with_placement(PopupPlacement::Center)
        .build();
    let palette_popup = ui.add_node(palette_popup_node, root);
    let palette_widget_node = CommandPaletteBuilder::new(
        WidgetBuilder::new()
            .with_background(theme::BG_HEADER)
            .with_horizontal_alignment(HorizontalAlignment::Center)
            .with_vertical_alignment(VerticalAlignment::Center),
    )
    .with_font_id(font_id)
    .with_items(palette_items)
    .build();
    let palette_widget = ui.add_node(palette_widget_node, palette_popup);

    let toast_node = ToastHostBuilder::new(WidgetBuilder::new())
        .with_font_id(font_id)
        .build();
    let toast_host = ui.add_node(toast_node, root);

    let unsaved_popup_node =
        PopupBuilder::new(WidgetBuilder::new().with_background([0, 0, 0, 100]))
            .with_placement(PopupPlacement::Center)
            .build();
    let unsaved_popup = ui.add_node(unsaved_popup_node, root);
    let unsaved_border = BorderBuilder::new(
        WidgetBuilder::new()
            .with_width(340.0)
            .with_horizontal_alignment(HorizontalAlignment::Center)
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness::uniform(1.0))
    .build();
    let unsaved_border_h = ui.add_node(unsaved_border, unsaved_popup);
    let unsaved_stack =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let unsaved_stack_h = ui.add_node(unsaved_stack, unsaved_border_h);
    let unsaved_lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::uniform(12.0)))
        .with_text("Save changes to the current scene?")
        .with_font_id(font_id)
        .with_font_size(13.0)
        .with_color(theme::TEXT_PRIMARY)
        .build();
    ui.add_node(unsaved_lbl, unsaved_stack_h);
    let unsaved_row =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Horizontal)
            .build();
    let unsaved_row_h = ui.add_node(unsaved_row, unsaved_stack_h);
    let mk_modal_btn = |ui: &mut UserInterface, label: &str, parent: NodeHandle| {
        let b = ButtonBuilder::new(
            WidgetBuilder::new()
                .with_width(100.0)
                .with_height(24.0)
                .with_margin(Thickness::axes(8.0, 8.0))
                .with_background(theme::BG_RAISED),
        )
        .build();
        let h = ui.add_node(b, parent);
        let t = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(8.0, 4.0)))
            .with_text(label)
            .with_font_id(font_id)
            .with_font_size(12.0)
            .with_color(theme::TEXT_PRIMARY)
            .build();
        ui.add_node(t, h);
        h
    };
    let unsaved_save = mk_modal_btn(ui, "Save", unsaved_row_h);
    let unsaved_discard = mk_modal_btn(ui, "Don't Save", unsaved_row_h);
    let unsaved_cancel = mk_modal_btn(ui, "Cancel", unsaved_row_h);

    let color_popup_node =
        PopupBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_placement(PopupPlacement::AnchorBelow)
            .build();
    let color_popup = ui.add_node(color_popup_node, root);
    let color_picker_node = ColorPickerBuilder::new(
        WidgetBuilder::new()
            .with_background(theme::BG_HEADER)
            .with_horizontal_alignment(HorizontalAlignment::Left)
            .with_vertical_alignment(VerticalAlignment::Top),
    )
    .with_font_id(font_id)
    .build();
    let color_picker = ui.add_node(color_picker_node, color_popup);

    let foliage_kind_combo = inspector_handles.foliage_kind_button;
    let post_tonemap_combo = inspector_handles.post_tonemap_button;
    let foliage_kind_popup =
        attach_combo_popup(ui, foliage_kind_combo, &FOLIAGE_KIND_NAMES, font_id);
    let post_tonemap_popup = attach_combo_popup(ui, post_tonemap_combo, &TONEMAP_NAMES, font_id);

    EditorLayout {
        outliner_scroll,
        outliner_stack,
        inspector_stack,
        log_stack,
        create_button,
        create_popup,
        create_popup_items,
        file_button,
        file_popup,
        file_import_item,
        file_new_item,
        file_save_item,
        camera_speed_slider,
        camera_speed_label,
        play_button,
        play_label,
        immersive_button,
        pause_button,
        pause_label,
        stop_button,
        stop_label,
        select_button,
        landscape_button,
        foliage_toolbar_button,
        terrain_tool_items,
        inspector_handles,
        viewport_handle,
        profiler_panel,
        profiler_toggle,
        profiler_toggle_lbl,
        profiler_names,
        profiler_values,
        outer_grid: outer_h,
        menu_bar_h,
        inner_h,
        content_split_h,
        right_split_h,
        toolbar_h,
        right_h,
        bottom_h,
        fps_text,
        help_button,
        help_overlay,
        help_body,
        tooltip,
        edit_button,
        view_button,
        window_button,
        help_menu_button,
        edit_popup,
        view_popup,
        window_popup,
        help_menu_popup,
        edit_undo,
        edit_redo,
        edit_delete,
        edit_dup,
        view_profiler,
        view_content,
        window_dock_content,
        help_open_item,
        help_shortcuts,
        help_about,
        status_text,
        drawer_button,
        log_button,
        content_drawer,
        content_search,
        content_breadcrumb,
        content_engine_toggle,
        content_list,
        outliner_tree,
        outliner_search,
        inspector_search,
        foliage_kind_combo,
        post_tonemap_combo,
        foliage_kind_popup,
        post_tonemap_popup,
        save_button,
        palette_popup,
        palette_widget,
        toast_host,
        unsaved_popup,
        unsaved_save,
        unsaved_discard,
        unsaved_cancel,
        color_popup,
        color_picker,
        title_drag,
        win_min,
        win_max,
        win_close,
        help_toc,
        help_close,
        log_panel,
    }
}

/// Build the 9 NumericFields for the inspector TRS section.
/// Returns the inspector handle bundle.
fn build_inspector(ui: &mut UserInterface, parent: NodeHandle, font_id: u8) -> InspectorHandles {
    // `label_w` widens the gutter for the light section's longer labels.
    // Returns `(row, field)`. The row handle is what a caller needs to hide a
    // whole line, label included.
    let make_row_rw = |ui: &mut UserInterface,
                       label: &str,
                       label_w: f32,
                       font_id: u8,
                       parent: NodeHandle,
                       drag_step: f32| {
        let row = StackPanelBuilder::new(
            WidgetBuilder::new()
                .with_height(theme::ROW_HEIGHT)
                .with_background(theme::TRANSPARENT),
        )
        .with_orientation(Orientation::Horizontal)
        .build();
        let row_h = ui.add_node(row, parent);

        let lbl = TextBuilder::new(WidgetBuilder::new().with_width(label_w).with_margin(
            Thickness {
                left: 6.0,
                top: 4.0,
                right: 4.0,
                bottom: 0.0,
            },
        ))
        .with_text(label)
        .with_font_size(11.0)
        .with_font_id(font_id)
        .with_color(theme::TEXT_SECONDARY)
        .build();
        ui.add_node(lbl, row_h);

        let field = NumericFieldBuilder::new(WidgetBuilder::new().with_margin(Thickness {
            left: 0.0,
            top: 2.0,
            right: 4.0,
            bottom: 0.0,
        }))
        .with_font_size(12.0)
        .with_font_id(font_id)
        .with_drag_step(drag_step)
        .build();
        (row_h, ui.add_node(field, row_h))
    };
    let make_color =
        |ui: &mut UserInterface, label: &str, label_w: f32, font_id: u8, parent: NodeHandle| {
            let row = StackPanelBuilder::new(
                WidgetBuilder::new()
                    .with_height(theme::ROW_HEIGHT)
                    .with_background(theme::TRANSPARENT),
            )
            .with_orientation(Orientation::Horizontal)
            .build();
            let row_h = ui.add_node(row, parent);
            let lbl = TextBuilder::new(WidgetBuilder::new().with_width(label_w).with_margin(
                Thickness {
                    left: 6.0,
                    top: 4.0,
                    right: 4.0,
                    bottom: 0.0,
                },
            ))
            .with_text(label)
            .with_font_size(11.0)
            .with_font_id(font_id)
            .with_color(theme::TEXT_SECONDARY)
            .build();
            ui.add_node(lbl, row_h);
            let swatch = ColorSwatchBuilder::new(WidgetBuilder::new()).build();
            ui.add_node(swatch, row_h)
        };
    // 0.05 per pixel: a 100-pixel drag moves a position by 5 units, which is
    // the right feel for metres and degrees alike.
    let make_row_w =
        |ui: &mut UserInterface, label: &str, label_w: f32, font_id: u8, parent: NodeHandle| {
            make_row_rw(ui, label, label_w, font_id, parent, 0.05).1
        };
    let make_row_step =
        |ui: &mut UserInterface,
         label: &str,
         label_w: f32,
         font_id: u8,
         parent: NodeHandle,
         step: f32| { make_row_rw(ui, label, label_w, font_id, parent, step).1 };
    let make_row = |ui: &mut UserInterface, label: &str, font_id: u8, parent: NodeHandle| {
        make_row_w(ui, label, 20.0, font_id, parent)
    };

    let sec_label = |ui: &mut UserInterface, text: &str, font_id: u8, parent: NodeHandle| {
        let lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
            left: 6.0,
            top: 6.0,
            right: 0.0,
            bottom: 2.0,
        }))
        .with_text(text)
        .with_font_size(11.0)
        .with_font_id(font_id)
        .with_color(theme::TEXT_SECONDARY)
        .build();
        ui.add_node(lbl, parent);
    };

    sec_label(ui, "Position", font_id, parent);
    let pos_x = make_row(ui, "X", font_id, parent);
    let pos_y = make_row(ui, "Y", font_id, parent);
    let pos_z = make_row(ui, "Z", font_id, parent);

    sec_label(ui, "Rotation", font_id, parent);
    let rot_x = make_row(ui, "X", font_id, parent);
    let rot_y = make_row(ui, "Y", font_id, parent);
    let rot_z = make_row(ui, "Z", font_id, parent);

    sec_label(ui, "Scale", font_id, parent);
    let sc_x = make_row(ui, "X", font_id, parent);
    let sc_y = make_row(ui, "Y", font_id, parent);
    let sc_z = make_row(ui, "Z", font_id, parent);

    // ── Light section (Phase 13E) ────────────────────────────────────────────
    // Lives in its own panel so it can be hidden when the selection isn't a
    // light. Angles are shown in degrees; range/angles only apply to
    // point/spot lights (a directional light ignores them).
    let light_panel =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let light_section = ui.add_node(light_panel, parent);

    sec_label(ui, "Light", font_id, light_section);
    let light_intensity = make_row_w(ui, "Int", 34.0, font_id, light_section);
    let light_color = make_color(ui, "Color", 34.0, font_id, light_section);
    let light_col_r = NodeHandle::NONE;
    let light_col_g = NodeHandle::NONE;
    let light_col_b = NodeHandle::NONE;
    let light_temp_k = make_row_step(ui, "Kelvin", 34.0, font_id, light_section, 5.0);
    let (light_range_row, light_range) = make_row_rw(ui, "Rng", 34.0, font_id, light_section, 0.1);
    let (light_inner_row, light_inner) = make_row_rw(ui, "In°", 34.0, font_id, light_section, 0.2);
    let (light_outer_row, light_outer) = make_row_rw(ui, "Out°", 34.0, font_id, light_section, 0.2);
    let (light_moon_row, light_moon_int) =
        make_row_rw(ui, "Moon", 34.0, font_id, light_section, 0.005);
    let light_radius = make_row_step(ui, "Radius", 34.0, font_id, light_section, 0.01);
    let (light_width_row, light_width) =
        make_row_rw(ui, "Half W", 34.0, font_id, light_section, 0.05);
    let (light_height_row, light_height) =
        make_row_rw(ui, "Half H", 34.0, font_id, light_section, 0.05);
    ui.set_visibility(light_width_row, false);
    ui.set_visibility(light_height_row, false);
    ui.set_visibility(light_section, false);

    // ── Post-processing section (Phase 15A1) ─────────────────────────────────
    // The engine has no checkbox widget, so a Button whose label carries the
    // tick doubles as one: clicking flips the state and the label is rewritten.
    let post_panel =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let post_section = ui.add_node(post_panel, parent);

    sec_label(ui, "Post FX", font_id, post_section);
    // Phase 24A/24B exposure controls. "EV" is the manual exposure value;
    // "EC" is compensation in stops, which is what you reach for when the meter
    // is right but the shot wants to be a stop darker.
    let (post_auto_exp_toggle, post_auto_exp_label) =
        make_toggle(ui, "Auto Exposure", font_id, post_section);
    let post_exposure = make_row_step(ui, "EV", 34.0, font_id, post_section, 0.05);
    let post_exp_comp = make_row_step(ui, "EC", 34.0, font_id, post_section, 0.05);
    // Phase 24A. With this on, EV above is computed from the three rows under
    // it, so a scene is lit by picking a real exposure triangle instead of a
    // number with no units. Aperture drives the DoF blur either way.
    let (post_phys_toggle, post_phys_label) =
        make_toggle(ui, "Physical Camera", font_id, post_section);
    let post_aperture = make_row_step(ui, "f/", 34.0, font_id, post_section, 0.05);
    // Shown as the denominator a photographer reads — 100 means 1/100 s —
    // because the seconds themselves are a third of a hundredth and no scrub
    // rate makes that row usable.
    let post_shutter = make_row_step(ui, "1/s", 34.0, font_id, post_section, 1.0);
    let post_iso = make_row_step(ui, "ISO", 34.0, font_id, post_section, 2.0);
    let (post_tonemap_button, post_tonemap_label) = {
        let tonemap_combo = ComboBoxBuilder::new(WidgetBuilder::new())
            .with_items(TONEMAP_NAMES)
            .with_font_id(font_id)
            .build();
        let (_, h) = build_property_row(ui, post_section, "Tonemap", font_id, tonemap_combo);
        (h, h)
    };
    // Indirect-light strength. Low values give contrasty shadows, high values a
    // flatter, brighter scene — see `PostProcessComponent::ibl_intensity`.
    let post_ibl = make_row_step(ui, "IBL", 34.0, font_id, post_section, 0.005);
    let (post_vig_toggle, post_vig_label) = make_toggle(ui, "Vignette", font_id, post_section);
    let post_vig_str = make_row_step(ui, "Amt", 34.0, font_id, post_section, 0.01);
    let (post_ca_toggle, post_ca_label) = make_toggle(ui, "Chromatic Ab.", font_id, post_section);
    let post_ca_str = make_row_step(ui, "Amt", 34.0, font_id, post_section, 0.0002);
    let (post_fxaa_toggle, post_fxaa_label) = make_toggle(ui, "FXAA", font_id, post_section);
    // Phase 24AC. Next to FXAA because they are the two filters that run on the
    // finished LDR image, and because the pair is what a reader is comparing.
    let (post_cas_toggle, post_cas_label) = make_toggle(ui, "Sharpen (CAS)", font_id, post_section);
    let post_cas_sharp = make_row_step(ui, "Sharp", 34.0, font_id, post_section, 0.01);
    let post_cas_strength = make_row_step(ui, "Amount", 34.0, font_id, post_section, 0.01);
    // Phase 24Z. Below the two AA/sharpen filters because it is the other
    // camera-motion effect, and its shutter is a photographic quantity like the
    // exposure rows at the top.
    let (post_mb_toggle, post_mb_label) = make_toggle(ui, "Motion Blur", font_id, post_section);
    let post_mb_shutter = make_row_step(ui, "Shutter", 34.0, font_id, post_section, 0.01);
    let (post_cel_toggle, post_cel_label) = make_toggle(ui, "Cel Shading", font_id, post_section);

    // Phase 24F/24I/24K/24T/24Z. Ordered roughly the way the frame runs, so the
    // list reads as a pipeline rather than an unsorted pile of switches.
    let (post_taa_toggle, post_taa_label) = make_toggle(ui, "TAA", font_id, post_section);
    let (post_gtao_toggle, post_gtao_label) = make_toggle(ui, "GTAO", font_id, post_section);
    // Radius is in metres and is the control that decides whether AO reads as
    // contact darkening under an object or as a broad smear across a hillside.
    let post_ao_radius = make_row_step(ui, "AO Rad", 34.0, font_id, post_section, 0.02);
    let post_ao_intensity = make_row_step(ui, "AO Amt", 34.0, font_id, post_section, 0.02);
    let (post_restir_toggle, post_restir_label) =
        make_toggle(ui, "RT Direct Light", font_id, post_section);
    // Phase 24L. Directly under the direct-light switch, because it is the
    // other half of the same traced solution and they read as a pair.
    let (post_restir_gi_toggle, post_restir_gi_label) =
        make_toggle(ui, "RT Indirect (GI)", font_id, post_section);
    let (post_rt_reflect_toggle, post_rt_reflect_label) =
        make_toggle(ui, "RT Reflections", font_id, post_section);
    let (post_rt_refract_toggle, post_rt_refract_label) =
        make_toggle(ui, "RT Refraction", font_id, post_section);
    // Phase 24L. Directly under its toggle, matching every other effect that
    // pairs a switch with an amount.
    let post_gi_intensity = make_row_step(ui, "GI Amt", 34.0, font_id, post_section, 0.01);
    let (post_pcss_toggle, post_pcss_label) =
        make_toggle(ui, "Soft Shadows", font_id, post_section);
    let (post_contact_toggle, post_contact_label) =
        make_toggle(ui, "Contact Shadows", font_id, post_section);
    let (post_bloom_toggle, post_bloom_label) = make_toggle(ui, "Bloom", font_id, post_section);
    let post_bloom_amt = make_row_step(ui, "Amt", 34.0, font_id, post_section, 0.002);
    let (post_dof_toggle, post_dof_label) =
        make_toggle(ui, "Depth of Field", font_id, post_section);
    let post_dof_focus = make_row_step(ui, "Focus", 34.0, font_id, post_section, 0.1);
    let post_temperature = make_row_step(ui, "Temp", 34.0, font_id, post_section, 0.01);
    // The other white-balance axis; without it "Temp" can only slide a scene
    // between orange and blue and never correct a green cast.
    let post_tint = make_row_step(ui, "Tint", 34.0, font_id, post_section, 0.01);
    let post_contrast = make_row_step(ui, "Contr", 34.0, font_id, post_section, 0.01);
    let post_saturation = make_row_step(ui, "Sat", 34.0, font_id, post_section, 0.01);
    // Phase 24Y lift/gamma/gain: shadows, midtones, highlights.
    let post_lift = make_row_step(ui, "Lift", 34.0, font_id, post_section, 0.005);
    let post_gamma = make_row_step(ui, "Gamma", 34.0, font_id, post_section, 0.01);
    let post_gain = make_row_step(ui, "Gain", 34.0, font_id, post_section, 0.01);
    let post_grain = make_row_step(ui, "Grain", 34.0, font_id, post_section, 0.002);

    // Phases 24U/25I. Aerial perspective is always on with the volume — the
    // atmosphere is not optional — so the toggle covers the whole volume and
    // the density below controls only the fog medium on top of it.
    let (post_vol_toggle, post_vol_label) = make_toggle(ui, "Volumetrics", font_id, post_section);
    let (post_shafts_toggle, post_shafts_label) =
        make_toggle(ui, "Light Shafts", font_id, post_section);
    let post_shaft_amt = make_row_step(ui, "Shaft Amt", 34.0, font_id, post_section, 0.05);
    // Fog density is tiny — a visible haze is ~1e-3 per metre — so the scrub
    // rate has to be far finer than the other rows or one pixel of drag takes
    // the scene from clear to opaque.
    let post_fog_density = make_row_step(ui, "Fog", 34.0, font_id, post_section, 0.00005);
    let post_fog_height = make_row_step(ui, "FogH", 34.0, font_id, post_section, 1.0);
    let post_fog_asym = make_row_step(ui, "FogG", 34.0, font_id, post_section, 0.01);
    let (post_world_cache_toggle, post_world_cache_label) =
        make_toggle(ui, "World Cache", font_id, post_section);
    let post_cache_intensity = make_row_step(ui, "Cache Amt", 34.0, font_id, post_section, 0.02);
    let post_cache_cell = make_row_step(ui, "Cell m", 34.0, font_id, post_section, 0.05);
    let (post_specular_toggle, post_specular_label) =
        make_toggle(ui, "RT Specular", font_id, post_section);
    let post_spec_rough = make_row_step(ui, "Spec Rgh", 34.0, font_id, post_section, 0.01);
    let (post_path_toggle, post_path_label) = make_toggle(ui, "Path Tracer", font_id, post_section);
    let post_path_bounces = make_row_step(ui, "Bounces", 34.0, font_id, post_section, 1.0);
    let (post_sdf_toggle, post_sdf_label) = make_toggle(ui, "Mesh SDF", font_id, post_section);
    let (post_probes_toggle, post_probes_label) = make_toggle(ui, "Probes", font_id, post_section);
    let post_probe_intensity = make_row_step(ui, "Probe Amt", 34.0, font_id, post_section, 0.02);
    let (post_analytic_toggle, post_analytic_label) =
        make_toggle(ui, "Analytic Mips", font_id, post_section);
    ui.set_visibility(post_section, false);

    // ── Foliage (Phase 17C) ──────────────────────────────────────────────────
    let foliage_panel =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let foliage_section = ui.add_node(foliage_panel, parent);
    sec_label(ui, "Foliage", font_id, foliage_section);
    let (foliage_toggle, foliage_label) = make_toggle(ui, "Enabled", font_id, foliage_section);
    // Phase 17F: the brush. Paint arms it, Erase flips the stroke, Single
    // places one instance at the cursor — which is how trees get placed.
    let (foliage_paint_toggle, foliage_paint_label) =
        make_toggle(ui, "Foliage Paint", font_id, foliage_section);
    let (foliage_erase_toggle, foliage_erase_label) =
        make_toggle(ui, "Erase", font_id, foliage_section);
    let (foliage_single_toggle, foliage_single_label) =
        make_toggle(ui, "Single", font_id, foliage_section);
    let kind_combo = ComboBoxBuilder::new(WidgetBuilder::new())
        .with_items(FOLIAGE_KIND_NAMES)
        .with_font_id(font_id)
        .build();
    let (_, foliage_kind_button) =
        build_property_row(ui, foliage_section, "Type", font_id, kind_combo);
    let foliage_kind_label = foliage_kind_button;
    // Density is per square metre and lives well under 1, so it needs a far
    // finer drag rate than a position.
    let foliage_density = make_row_step(ui, "Dens", 34.0, font_id, foliage_section, 0.02);
    let foliage_seed = make_row_step(ui, "Size", 34.0, font_id, foliage_section, 0.05);
    let foliage_slope = make_row_step(ui, "Slp\u{b0}", 34.0, font_id, foliage_section, 0.2);
    // Kept only so the engine's field routing still has a handle. Hiding the
    // whole row matters, not just the field — the label lives in the row, and
    // hiding the field alone leaves a stray "Type" caption behind.
    let (foliage_layer_row, foliage_layer) =
        make_row_rw(ui, "Type", 34.0, font_id, foliage_section, 0.02);
    ui.set_visibility(foliage_layer_row, false);
    let foliage_smin = make_row_step(ui, "Sc Mn", 34.0, font_id, foliage_section, 0.01);
    let foliage_smax = make_row_step(ui, "Sc Mx", 34.0, font_id, foliage_section, 0.01);
    // Phase 24AE. Metres, so a whole-number step: this is the dial that decides
    // how much of the shadow pass a grass field is allowed to cost, and the
    // profiler's `shadow casters` row is the readout for it.
    let foliage_shadow = make_row_step(ui, "Sh Dst", 34.0, font_id, foliage_section, 1.0);
    let foliage_cull = make_row_step(ui, "Cull", 34.0, font_id, foliage_section, 1.0);
    let foliage_lod = make_row_step(ui, "LOD", 34.0, font_id, foliage_section, 1.0);
    let foliage_impostor = make_row_step(ui, "Impostor", 34.0, font_id, foliage_section, 1.0);
    ui.set_visibility(foliage_section, false);

    // ── Terrain layers (Phase 17C) ───────────────────────────────────────────
    let terrain_panel =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let terrain_section = ui.add_node(terrain_panel, parent);
    sec_label(ui, "Terrain", font_id, terrain_section);
    let terrain_mode_label = TextBuilder::new(WidgetBuilder::new().with_height(32.0).with_margin(
        Thickness {
            left: 6.0,
            top: 2.0,
            right: 6.0,
            bottom: 2.0,
        },
    ))
    .with_text("Active: none")
    .with_font_size(11.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_PRIMARY)
    .build();
    let terrain_mode_label = ui.add_node(terrain_mode_label, terrain_section);
    let (terrain_paint_toggle, terrain_paint_label) =
        make_toggle(ui, "Terrain Paint", font_id, terrain_section);
    let (terrain_hex_toggle, terrain_hex_label) =
        make_toggle(ui, "Hex Tiling", font_id, terrain_section);
    let (terrain_morph_toggle, terrain_morph_label) =
        make_toggle(ui, "LOD Morph", font_id, terrain_section);
    let terrain_morph_start = make_row_step(ui, "Morph", 34.0, font_id, terrain_section, 0.02);
    let mut terrain_brush_items = Vec::with_capacity(6);
    for row in 0..2 {
        let row_panel =
            StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
                .with_orientation(Orientation::Horizontal)
                .build();
        let row_h = ui.add_node(row_panel, terrain_section);
        for col in 0..3 {
            let tool = (row * 3 + col) as u8;
            let (btn, lbl) =
                make_palette_button(ui, TERRAIN_BRUSH_NAMES[tool as usize], font_id, row_h);
            terrain_brush_items.push((btn, lbl, tool));
        }
    }
    // Whole steps: the paint layer is an index, and a fractional drag would be
    // meaningless.
    let terrain_layer = make_row_step(ui, "Paint", 34.0, font_id, terrain_section, 0.02);
    let mut terrain_palette = [NodeHandle::NONE; 32];
    let mut terrain_palette_labels = [NodeHandle::NONE; 32];
    for row in 0..8 {
        let row_panel =
            StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
                .with_orientation(Orientation::Horizontal)
                .build();
        let row_h = ui.add_node(row_panel, terrain_section);
        for col in 0..4 {
            let i = row * 4 + col;
            let (btn, lbl) = make_palette_button(ui, TERRAIN_LAYER_SHORT[i], font_id, row_h);
            terrain_palette[i] = btn;
            terrain_palette_labels[i] = lbl;
        }
    }
    let terrain_tile = make_row_step(ui, "Tile", 34.0, font_id, terrain_section, 0.01);
    // Phase 25H: multiplies the relief depth every layer authors for itself, so
    // one dial covers the whole terrain without flattening the differences
    // between gravel and mud. 0 switches parallax off.
    let terrain_relief = make_row_step(ui, "Relief", 34.0, font_id, terrain_section, 0.05);
    let terrain_wetness = make_row_step(ui, "Wet", 34.0, font_id, terrain_section, 0.02);
    let terrain_macro = make_row_step(ui, "Macro", 34.0, font_id, terrain_section, 0.02);
    let terrain_debug = make_row_step(ui, "Dbg", 34.0, font_id, terrain_section, 1.0);
    ui.set_visibility(terrain_section, false);

    let water_panel =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let water_section = ui.add_node(water_panel, parent);
    sec_label(ui, "Water Body", font_id, water_section);
    let water_surface = make_row_step(ui, "Level", 34.0, font_id, water_section, 0.05);
    let water_depth = make_row_step(ui, "Depth", 34.0, font_id, water_section, 0.05);
    let water_clarity = make_row_step(ui, "Clear", 34.0, font_id, water_section, 0.01);
    let water_amplitude = make_row_step(ui, "Waves", 34.0, font_id, water_section, 0.01);
    let water_roughness = make_row_step(ui, "Rough", 34.0, font_id, water_section, 0.01);
    let water_ssr = make_row_step(ui, "SSR", 34.0, font_id, water_section, 0.01);
    let water_rt_reflect = make_row_step(ui, "RT Reflect", 34.0, font_id, water_section, 0.01);
    let water_reflect_debug = make_row_step(ui, "Reflect Debug", 34.0, font_id, water_section, 1.0);
    let water_wave_a = make_row_step(ui, "Wave A", 34.0, font_id, water_section, 0.25);
    let water_wave_b = make_row_step(ui, "Wave B", 34.0, font_id, water_section, 0.25);
    let water_speed = make_row_step(ui, "Speed", 34.0, font_id, water_section, 0.05);
    let water_steepness = make_row_step(ui, "Steep", 34.0, font_id, water_section, 0.01);
    let water_wind_speed = make_row_step(ui, "Wind", 34.0, font_id, water_section, 0.5);
    let water_foam_decay = make_row_step(ui, "Foam", 34.0, font_id, water_section, 0.05);
    let water_foam_threshold = make_row_step(ui, "Whitecap", 34.0, font_id, water_section, 0.01);
    let water_spectrum_blend = make_row_step(ui, "Spect", 34.0, font_id, water_section, 0.01);
    let water_edge_scale = make_row_step(ui, "Edge", 34.0, font_id, water_section, 0.05);
    let water_anisotropy = make_row_step(ui, "Aniso", 34.0, font_id, water_section, 0.01);
    let water_caustic = make_row_step(ui, "Caustic", 34.0, font_id, water_section, 0.05);
    let water_deep = make_color(ui, "Deep", 34.0, font_id, water_section);
    let water_shallow = make_color(ui, "Shallow", 34.0, font_id, water_section);
    let water_edge = make_color(ui, "Edge", 34.0, font_id, water_section);
    let water_abs = make_color(ui, "Abs", 34.0, font_id, water_section);
    let water_abs_mag = make_row_step(ui, "Abs Mag", 34.0, font_id, water_section, 0.005);
    let water_scatter = make_color(ui, "Scatter", 34.0, font_id, water_section);
    let water_scatter_mag = make_row_step(ui, "Sc Mag", 34.0, font_id, water_section, 0.005);
    let water_dir_ax = make_row_step(ui, "DirAX", 34.0, font_id, water_section, 0.01);
    let water_dir_az = make_row_step(ui, "DirAZ", 34.0, font_id, water_section, 0.01);
    let water_dir_bx = make_row_step(ui, "DirBX", 34.0, font_id, water_section, 0.01);
    let water_dir_bz = make_row_step(ui, "DirBZ", 34.0, font_id, water_section, 0.01);
    let (water_underwater, _) = {
        let cb = CheckBoxBuilder::new(WidgetBuilder::new().with_height(theme::ROW_HEIGHT))
            .with_label("Underwater")
            .with_font_id(font_id)
            .build();
        (ui.add_node(cb, water_section), NodeHandle::NONE)
    };
    ui.set_visibility(water_section, false);

    let particle_panel =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let particle_section = ui.add_node(particle_panel, parent);
    sec_label(ui, "Particles", font_id, particle_section);
    let particle_start = make_color(ui, "Start", 34.0, font_id, particle_section);
    let particle_end = make_color(ui, "End", 34.0, font_id, particle_section);
    ui.set_visibility(particle_section, false);

    let material_panel =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let material_section = ui.add_node(material_panel, parent);
    sec_label(ui, "Material", font_id, material_section);
    let material_base = make_color(ui, "Base", 34.0, font_id, material_section);
    ui.set_visibility(material_section, false);

    let vessel_panel =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let vessel_section = ui.add_node(vessel_panel, parent);
    sec_label(ui, "Vessel", font_id, vessel_section);
    let vessel_buoyancy = make_row_step(ui, "Buoy", 34.0, font_id, vessel_section, 250.0);
    let vessel_drag = make_row_step(ui, "Drag", 34.0, font_id, vessel_section, 50.0);
    let vessel_angular_drag = make_row_step(ui, "YawD", 34.0, font_id, vessel_section, 50.0);
    let vessel_thrust = make_row_step(ui, "Thrust", 34.0, font_id, vessel_section, 250.0);
    let vessel_draft = make_row_step(ui, "Draft", 34.0, font_id, vessel_section, 0.05);
    let vessel_righting = make_row_step(ui, "Right", 34.0, font_id, vessel_section, 250.0);
    ui.set_visibility(vessel_section, false);

    InspectorHandles {
        pos_x,
        pos_y,
        pos_z,
        rot_x,
        rot_y,
        rot_z,
        sc_x,
        sc_y,
        sc_z,
        light_section,
        light_intensity,
        light_range,
        light_inner,
        light_outer,
        light_col_r,
        light_col_g,
        light_col_b,
        light_color,
        light_temp_k,
        light_range_row,
        light_inner_row,
        light_outer_row,
        light_moon_row,
        light_moon_int,
        light_radius,
        light_width_row,
        light_width,
        light_height_row,
        light_height,
        terrain_section,
        terrain_mode_label,
        terrain_paint_toggle,
        terrain_paint_label,
        terrain_hex_toggle,
        terrain_hex_label,
        terrain_morph_toggle,
        terrain_morph_label,
        terrain_morph_start,
        terrain_brush_items,
        terrain_layer,
        terrain_palette,
        terrain_palette_labels,
        terrain_tile,
        terrain_relief,
        terrain_wetness,
        terrain_macro,
        terrain_debug,
        water_section,
        water_surface,
        water_depth,
        water_clarity,
        water_amplitude,
        water_roughness,
        water_ssr,
        water_rt_reflect,
        water_reflect_debug,
        water_wave_a,
        water_wave_b,
        water_speed,
        water_steepness,
        water_wind_speed,
        water_foam_decay,
        water_foam_threshold,
        water_spectrum_blend,
        water_edge_scale,
        water_anisotropy,
        water_caustic,
        water_deep,
        water_shallow,
        water_edge,
        water_abs,
        water_scatter,
        water_abs_mag,
        water_scatter_mag,
        water_underwater,
        water_dir_ax,
        water_dir_az,
        water_dir_bx,
        water_dir_bz,
        particle_section,
        particle_start,
        particle_end,
        material_section,
        material_base,
        vessel_section,
        vessel_buoyancy,
        vessel_drag,
        vessel_angular_drag,
        vessel_thrust,
        vessel_draft,
        vessel_righting,
        foliage_section,
        foliage_toggle,
        foliage_label,
        foliage_paint_toggle,
        foliage_paint_label,
        foliage_erase_toggle,
        foliage_erase_label,
        foliage_single_toggle,
        foliage_single_label,
        foliage_kind_button,
        foliage_kind_label,
        foliage_density,
        foliage_seed,
        foliage_slope,
        foliage_layer,
        foliage_smin,
        foliage_smax,
        foliage_shadow,
        foliage_cull,
        foliage_lod,
        foliage_impostor,
        post_section,
        post_exposure,
        post_exp_comp,
        post_auto_exp_toggle,
        post_auto_exp_label,
        post_tonemap_button,
        post_tonemap_label,
        post_ibl,
        post_vig_toggle,
        post_vig_str,
        post_ca_toggle,
        post_ca_str,
        post_vig_label,
        post_ca_label,
        post_cel_toggle,
        post_cel_label,
        post_taa_toggle,
        post_taa_label,
        post_gtao_toggle,
        post_gtao_label,
        post_restir_toggle,
        post_restir_label,
        post_bloom_toggle,
        post_bloom_label,
        post_restir_gi_toggle,
        post_rt_reflect_toggle,
        post_rt_reflect_label,
        post_rt_refract_toggle,
        post_rt_refract_label,
        post_restir_gi_label,
        post_pcss_toggle,
        post_pcss_label,
        post_contact_toggle,
        post_contact_label,
        post_gi_intensity,
        post_bloom_amt,
        post_dof_toggle,
        post_dof_label,
        post_dof_focus,
        post_temperature,
        post_contrast,
        post_saturation,
        post_grain,
        post_vol_toggle,
        post_vol_label,
        post_shafts_toggle,
        post_shafts_label,
        post_fog_density,
        post_fog_height,
        post_fog_asym,
        post_world_cache_toggle,
        post_world_cache_label,
        post_cache_intensity,
        post_cache_cell,
        post_specular_toggle,
        post_specular_label,
        post_spec_rough,
        post_path_toggle,
        post_path_label,
        post_path_bounces,
        post_sdf_toggle,
        post_sdf_label,
        post_probes_toggle,
        post_probes_label,
        post_probe_intensity,
        post_analytic_toggle,
        post_analytic_label,
        post_shaft_amt,
        post_phys_toggle,
        post_phys_label,
        post_aperture,
        post_shutter,
        post_iso,
        post_tint,
        post_lift,
        post_gamma,
        post_gain,
        post_ao_radius,
        post_ao_intensity,
        post_fxaa_toggle,
        post_fxaa_label,
        post_cas_toggle,
        post_cas_label,
        post_cas_sharp,
        post_cas_strength,
        post_mb_toggle,
        post_mb_label,
        post_mb_shutter,
    }
}

fn make_palette_button(
    ui: &mut UserInterface,
    text: &str,
    font_id: u8,
    parent: NodeHandle,
) -> (NodeHandle, NodeHandle) {
    let btn = ButtonBuilder::new(
        WidgetBuilder::new()
            .with_height(20.0)
            .with_width(52.0)
            .with_margin(Thickness {
                left: 2.0,
                top: 1.0,
                right: 2.0,
                bottom: 1.0,
            })
            .with_background(theme::BG_DARK),
    )
    .build();
    let btn_h = ui.add_node(btn, parent);
    let lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
        left: 3.0,
        top: 3.0,
        right: 0.0,
        bottom: 0.0,
    }))
    .with_text(&format!(" {text}"))
    .with_font_size(10.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_PRIMARY)
    .build();
    let lbl_h = ui.add_node(lbl, btn_h);
    (btn_h, lbl_h)
}

/// Build a checkbox toggle row (Phase 26-B). Both handles are the CheckBox
/// so existing inspector fields that stored a separate label still compile.
fn make_toggle(
    ui: &mut UserInterface,
    text: &str,
    font_id: u8,
    parent: NodeHandle,
) -> (NodeHandle, NodeHandle) {
    let cb = CheckBoxBuilder::new(
        WidgetBuilder::new()
            .with_height(22.0)
            .with_margin(Thickness {
                left: 6.0,
                top: 2.0,
                right: 6.0,
                bottom: 0.0,
            })
            .with_background(theme::TRANSPARENT),
    )
    .with_label(text)
    .with_font_id(font_id)
    .with_font_size(11.0)
    .build();
    let h = ui.add_node(cb, parent);
    (h, h)
}

fn attach_combo_popup(
    ui: &mut UserInterface,
    combo: NodeHandle,
    items: &[&str],
    font_id: u8,
) -> NodeHandle {
    let popup = PopupBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .with_anchor(combo)
        .with_placement(PopupPlacement::AnchorBelow)
        .build();
    let popup_h = ui.add_node(popup, ui.root());
    let list = ComboDropdownBuilder::new(WidgetBuilder::new())
        .with_items(items.iter().copied())
        .with_combo(combo)
        .with_popup(popup_h)
        .with_font_id(font_id)
        .build();
    let list_h = ui.add_node(list, popup_h);
    ui.send(ComboBoxMessage::bind_popup(combo, popup_h, list_h));
    popup_h
}

fn menu_button(ui: &mut UserInterface, parent: NodeHandle, label: &str, font_id: u8) -> NodeHandle {
    let node = MenuBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT)).build();
    let h = ui.add_node(node, parent);
    let lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(8.0, 6.0)))
        .with_text(label)
        .with_font_size(13.0)
        .with_font_id(font_id)
        .with_color(theme::TEXT_PRIMARY)
        .build();
    ui.add_node(lbl, h);
    h
}

fn popup_items(
    ui: &mut UserInterface,
    root: NodeHandle,
    font_id: u8,
    items: &[&str],
) -> (NodeHandle, Vec<NodeHandle>) {
    let popup = PopupBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT)).build();
    let popup_h = ui.add_node(popup, root);
    let border = BorderBuilder::new(
        WidgetBuilder::new()
            .with_width(200.0)
            .with_horizontal_alignment(HorizontalAlignment::Left)
            .with_vertical_alignment(VerticalAlignment::Top)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness::uniform(1.0))
    .build();
    let border_h = ui.add_node(border, popup_h);
    let stack = StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .with_orientation(Orientation::Vertical)
        .build();
    let stack_h = ui.add_node(stack, border_h);
    let mut handles = Vec::with_capacity(items.len());
    for item in items {
        let btn = ButtonBuilder::new(
            WidgetBuilder::new()
                .with_height(22.0)
                .with_background(theme::TRANSPARENT),
        )
        .build();
        let bh = ui.add_node(btn, stack_h);
        let lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(8.0, 4.0)))
            .with_text(*item)
            .with_font_size(12.0)
            .with_font_id(font_id)
            .with_color(theme::TEXT_PRIMARY)
            .build();
        ui.add_node(lbl, bh);
        handles.push(bh);
    }
    (popup_h, handles)
}

fn icon_tool_button(
    ui: &mut UserInterface,
    parent: NodeHandle,
    icon: IconId,
    tooltip: &str,
) -> NodeHandle {
    let mut wb = WidgetBuilder::new()
        .with_width(36.0)
        .with_height(theme::TOOLBAR_HEIGHT)
        .with_margin(Thickness::axes(2.0, 2.0))
        .with_background(theme::BG_RAISED);
    if !tooltip.is_empty() {
        wb = wb.with_tooltip(tooltip);
    }
    let btn = ButtonBuilder::new(wb).build();
    let h = ui.add_node(btn, parent);
    let img = ImageBuilder::new(WidgetBuilder::new())
        .with_icon(icon)
        .with_size(theme::ICON_TOOL)
        .with_tint(theme::TEXT_PRIMARY)
        .build();
    ui.add_node(img, h);
    h
}

fn fill_help_body(ui: &mut UserInterface, parent: NodeHandle, font_id: u8, page: u8) {
    ui.clear_children(parent);
    for block in crate::metaphor::help_blocks(page) {
        match block {
            crate::metaphor::HelpBlock::Heading(text) => {
                let n = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
                    left: 0.0,
                    top: 10.0,
                    right: 8.0,
                    bottom: 6.0,
                }))
                .with_text(text)
                .with_font_size(16.0)
                .with_font_id(font_id)
                .with_color(theme::ACCENT)
                .with_wrap(true)
                .build();
                ui.add_node(n, parent);
            }
            crate::metaphor::HelpBlock::Paragraph(text) => {
                let n = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
                    left: 0.0,
                    top: 0.0,
                    right: 8.0,
                    bottom: 8.0,
                }))
                .with_text(text)
                .with_font_size(13.0)
                .with_font_id(font_id)
                .with_color(theme::TEXT_PRIMARY)
                .with_wrap(true)
                .build();
                ui.add_node(n, parent);
            }
            crate::metaphor::HelpBlock::Bullet(text) => {
                let n = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
                    left: 12.0,
                    top: 0.0,
                    right: 8.0,
                    bottom: 4.0,
                }))
                .with_text(format!("• {text}"))
                .with_font_size(13.0)
                .with_font_id(font_id)
                .with_color(theme::TEXT_PRIMARY)
                .with_wrap(true)
                .build();
                ui.add_node(n, parent);
            }
        }
    }
}

fn window_chrome_button(
    ui: &mut UserInterface,
    parent: NodeHandle,
    icon: IconId,
    tooltip: &str,
) -> NodeHandle {
    let btn = ButtonBuilder::new(
        WidgetBuilder::new()
            .with_width(46.0)
            .with_height(theme::TITLEBAR_HEIGHT)
            .with_tooltip(tooltip)
            .with_background(theme::TRANSPARENT),
    )
    .build();
    let h = ui.add_node(btn, parent);
    let img = ImageBuilder::new(WidgetBuilder::new())
        .with_icon(icon)
        .with_size(16.0)
        .with_tint(theme::TEXT_PRIMARY)
        .build();
    ui.add_node(img, h);
    h
}

fn labeled_icon_button(
    ui: &mut UserInterface,
    parent: NodeHandle,
    icon: IconId,
    label: &str,
    tooltip: &str,
    font_id: u8,
    height: f32,
) -> (NodeHandle, NodeHandle) {
    let btn = ButtonBuilder::new(
        WidgetBuilder::new()
            .with_height(height)
            .with_margin(Thickness::axes(2.0, 1.0))
            .with_tooltip(tooltip)
            .with_background(theme::BG_RAISED),
    )
    .build();
    let h = ui.add_node(btn, parent);
    let row = StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .with_orientation(Orientation::Horizontal)
        .build();
    let row_h = ui.add_node(row, h);
    let img = ImageBuilder::new(WidgetBuilder::new().with_margin(Thickness {
        left: 6.0,
        top: ((height - 16.0) * 0.5).max(2.0),
        right: 4.0,
        bottom: 0.0,
    }))
    .with_icon(icon)
    .with_size(16.0)
    .with_tint(theme::TEXT_PRIMARY)
    .build();
    ui.add_node(img, row_h);
    let lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
        left: 0.0,
        top: ((height - 14.0) * 0.5).max(2.0),
        right: 8.0,
        bottom: 0.0,
    }))
    .with_text(label)
    .with_font_size(12.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_PRIMARY)
    .build();
    let lbl_h = ui.add_node(lbl, row_h);
    (h, lbl_h)
}

fn build_help_overlay(
    ui: &mut UserInterface,
    root: NodeHandle,
    font_id: u8,
) -> (NodeHandle, NodeHandle, Vec<(NodeHandle, u8)>, NodeHandle) {
    let overlay = PopupBuilder::new(WidgetBuilder::new().with_background([0x0E, 0x10, 0x14, 0xE0]))
        .with_placement(PopupPlacement::Center)
        .build();
    let overlay_h = ui.add_node(overlay, root);

    let card = BorderBuilder::new(
        WidgetBuilder::new()
            .with_width(860.0)
            .with_height(540.0)
            .with_horizontal_alignment(HorizontalAlignment::Center)
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_background(theme::BG_PANEL)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness::uniform(1.0))
    .build();
    let card_h = ui.add_node(card, overlay_h);

    let grid = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::strict(36.0))
        .add_row(Row::stretch())
        .add_column(Column::stretch())
        .build();
    let grid_h = ui.add_node(grid, card_h);

    let header = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(0)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 0.0,
        top: 0.0,
        bottom: 1.0,
    })
    .build();
    let header_h = ui.add_node(header, grid_h);
    let header_grid = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::stretch())
        .add_column(Column::stretch())
        .add_column(Column::auto())
        .build();
    let header_grid_h = ui.add_node(header_grid, header_h);
    let title = TextBuilder::new(WidgetBuilder::new().with_column(0).with_margin(Thickness {
        left: 12.0,
        top: 10.0,
        right: 0.0,
        bottom: 0.0,
    }))
    .with_text("Editor Help")
    .with_font_size(14.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_PRIMARY)
    .build();
    ui.add_node(title, header_grid_h);
    let close_col = StackPanelBuilder::new(
        WidgetBuilder::new()
            .with_column(1)
            .with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Horizontal)
    .build();
    let close_col_h = ui.add_node(close_col, header_grid_h);
    let help_close = window_chrome_button(ui, close_col_h, IconId::Close, "Close Help");

    let body_grid = GridBuilder::new(
        WidgetBuilder::new()
            .with_row(1)
            .with_column(0)
            .with_background(theme::TRANSPARENT),
    )
    .add_row(Row::stretch())
    .add_column(Column::strict(168.0))
    .add_column(Column::stretch())
    .build();
    let body_grid_h = ui.add_node(body_grid, grid_h);

    let toc_border = BorderBuilder::new(
        WidgetBuilder::new()
            .with_column(0)
            .with_row(0)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        right: 1.0,
        top: 0.0,
        bottom: 0.0,
    })
    .build();
    let toc_border_h = ui.add_node(toc_border, body_grid_h);
    let toc_stack =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let toc_stack_h = ui.add_node(toc_stack, toc_border_h);
    let mut help_toc = Vec::new();
    for (i, title) in crate::metaphor::help_titles().iter().enumerate() {
        let btn = ButtonBuilder::new(
            WidgetBuilder::new()
                .with_height(28.0)
                .with_margin(Thickness::axes(6.0, 2.0))
                .with_background(theme::BG_RAISED),
        )
        .build();
        let bh = ui.add_node(btn, toc_stack_h);
        let lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(10.0, 6.0)))
            .with_text(*title)
            .with_font_size(12.0)
            .with_font_id(font_id)
            .with_color(theme::TEXT_PRIMARY)
            .build();
        ui.add_node(lbl, bh);
        help_toc.push((bh, i as u8));
    }

    let scroll = ScrollViewerBuilder::new(
        WidgetBuilder::new()
            .with_column(1)
            .with_row(0)
            .with_background(theme::BG_PANEL),
    )
    .build();
    let scroll_h = ui.add_node(scroll, body_grid_h);
    let body = StackPanelBuilder::new(
        WidgetBuilder::new()
            .with_margin(Thickness::uniform(16.0))
            .with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Vertical)
    .build();
    let body_h = ui.add_node(body, scroll_h);
    fill_help_body(ui, body_h, font_id, 0);
    (overlay_h, body_h, help_toc, help_close)
}

fn build_content_drawer(
    ui: &mut UserInterface,
    parent: NodeHandle,
    font_id: u8,
) -> (NodeHandle, NodeHandle, NodeHandle, NodeHandle, NodeHandle) {
    let panel = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(0)
            .with_background(theme::BG_PANEL)
            .with_foreground(theme::TRANSPARENT),
    )
    .with_stroke_thickness(Thickness::ZERO)
    .build();
    let panel_h = ui.add_node(panel, parent);

    let grid = GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .add_row(Row::strict(26.0))
        .add_row(Row::strict(22.0))
        .add_row(Row::stretch())
        .add_column(Column::stretch())
        .add_column(Column::auto())
        .build();
    let grid_h = ui.add_node(grid, panel_h);

    let search = SearchBoxBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(0)
            .with_background(theme::BG_INPUT),
    )
    .with_font_id(font_id)
    .build();
    let search_h = ui.add_node(search, grid_h);

    let engine = CheckBoxBuilder::new(
        WidgetBuilder::new()
            .with_row(0)
            .with_column(1)
            .with_margin(Thickness::axes(8.0, 2.0)),
    )
    .with_label("Show Engine Content")
    .with_font_id(font_id)
    .with_font_size(11.0)
    .build();
    let engine_h = ui.add_node(engine, grid_h);

    let crumb = BreadcrumbBuilder::new(
        WidgetBuilder::new()
            .with_row(1)
            .with_column(0)
            .with_background(theme::TRANSPARENT),
    )
    .with_parts(["Game"])
    .with_font_id(font_id)
    .build();
    let crumb_h = ui.add_node(crumb, grid_h);

    let list_scroll = ScrollViewerBuilder::new(
        WidgetBuilder::new()
            .with_row(2)
            .with_column(0)
            .with_background(theme::BG_CONTENT),
    )
    .build();
    let list_scroll_h = ui.add_node(list_scroll, grid_h);
    let list = WrapPanelBuilder::new(
        WidgetBuilder::new()
            .with_margin(Thickness::uniform(8.0))
            .with_background(theme::TRANSPARENT),
    )
    .with_gap(10.0, 10.0)
    .build();
    let list_h = ui.add_node(list, list_scroll_h);

    (panel_h, search_h, crumb_h, engine_h, list_h)
}

/// Build the Create dropdown popup (initially hidden, child of root).
fn build_create_popup(
    ui: &mut UserInterface,
    root: NodeHandle,
    font_id: u8,
) -> (NodeHandle, Vec<(NodeHandle, CreateKind)>) {
    let popup_backdrop =
        PopupBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT)).build();
    let popup_h = ui.add_node(popup_backdrop, root);

    let popup_border = BorderBuilder::new(
        WidgetBuilder::new()
            .with_desired_position(Vec2::new(148.0, 28.0))
            .with_width(160.0)
            .with_horizontal_alignment(HorizontalAlignment::Left)
            .with_vertical_alignment(VerticalAlignment::Top)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness::uniform(1.0))
    .build();
    let popup_border_h = ui.add_node(popup_border, popup_h);

    let popup_stack =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let popup_stack_h = ui.add_node(popup_stack, popup_border_h);

    const KINDS: &[CreateKind] = &[
        CreateKind::Cube,
        CreateKind::Sphere,
        CreateKind::Plane,
        CreateKind::Cylinder,
        CreateKind::DirectionalLight,
        CreateKind::PointLight,
        CreateKind::SpotLight,
        CreateKind::RectLight,
        CreateKind::Particle,
        CreateKind::Terrain,
        CreateKind::VoxelTerrain,
    ];

    let mut items = Vec::with_capacity(KINDS.len());
    for &kind in KINDS {
        let btn = ButtonBuilder::new(
            WidgetBuilder::new()
                .with_height(22.0)
                .with_background(theme::TRANSPARENT),
        )
        .build();
        let btn_h = ui.add_node(btn, popup_stack_h);

        let lbl = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness {
            left: 8.0,
            top: 4.0,
            right: 0.0,
            bottom: 0.0,
        }))
        .with_text(kind.label())
        .with_font_size(12.0)
        .with_font_id(font_id)
        .with_color(theme::TEXT_PRIMARY)
        .build();
        ui.add_node(lbl, btn_h);
        items.push((btn_h, kind));
    }

    (popup_h, items)
}
