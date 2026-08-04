pub mod draw;
pub mod editor_event;
pub mod font;
pub mod message;
pub mod node;
pub mod pass;
pub mod pool;
pub mod theme;
pub mod types;
pub mod ui;
pub mod widget;
pub mod widgets;

pub use editor_event::{CreateKind, EditorEvent, InspectorField};

use crate::{
    editor_event::InspectorField as IF,
    message::{MessageDirection, NodeHandle, UiMessage},
    pass::UiPass,
    types::Thickness,
    ui::UserInterface,
    widget::WidgetBuilder,
    widgets::{
        border::BorderBuilder,
        button::{ButtonBuilder, ButtonMessage},
        grid::{Column, GridBuilder, Row},
        numeric_field::{NumericFieldBuilder, NumericFieldMessage},
        scroll_viewer::ScrollViewerBuilder,
        stack_panel::{Orientation, StackPanelBuilder},
        text::TextBuilder,
        popup::{PopupBuilder, PopupMessage},
        menu::{MenuBuilder, MenuMessage},
    },
};
use glam::Vec2;
use std::collections::VecDeque;
use std::sync::Arc;
use winit::event::WindowEvent;
use winit::window::Window;
use tracing::{info, warn};

// ── Inspector field handle bundle ────────────────────────────────────────────

struct InspectorHandles {
    pos_x: NodeHandle, pos_y: NodeHandle, pos_z: NodeHandle,
    rot_x: NodeHandle, rot_y: NodeHandle, rot_z: NodeHandle,
    sc_x:  NodeHandle, sc_y:  NodeHandle, sc_z:  NodeHandle,
    // Light section (Phase 13E) — hidden unless a light is selected.
    light_section:   NodeHandle,
    light_intensity: NodeHandle,
    light_range:     NodeHandle,
    light_inner:     NodeHandle,
    light_outer:     NodeHandle,
}

/// Light values shown in the inspector: intensity, range, inner°, outer°.
pub type LightInspectorValues = [f32; 4];

// ── Layout build result ───────────────────────────────────────────────────────

struct EditorLayout {
    outliner_scroll:    NodeHandle,
    outliner_stack:     NodeHandle,
    inspector_stack:    NodeHandle,
    log_stack:          NodeHandle,
    create_button:      NodeHandle,
    create_popup:       NodeHandle,
    create_popup_items: Vec<(NodeHandle, CreateKind)>,
    terrain_tool_items: Vec<(NodeHandle, u8)>,
    inspector_handles:  InspectorHandles,
    viewport_handle:    NodeHandle,
    outer_grid:         NodeHandle,
    menu_bar_h:         NodeHandle,
    inner_h:            NodeHandle,
    toolbar_h:          NodeHandle,
    right_h:            NodeHandle,
    bottom_h:           NodeHandle,
}

// ── UiManager ────────────────────────────────────────────────────────────────

/// Combined UI manager — wraps the native wgpu widget tree rendered by UiPass.
pub struct UiManager {
    window_size:         (u32, u32),
    native_ui:           UserInterface,
    ui_pass:             UiPass,
    font_id:             u8,
    // Live-update widget handles
    outliner_scroll:     NodeHandle,
    outliner_stack:      NodeHandle,
    #[allow(dead_code)]
    inspector_stack:     NodeHandle,
    log_stack:           NodeHandle,
    log_entry_count:     usize,
    // Create menu
    create_button:       NodeHandle,
    create_popup:        NodeHandle,
    create_popup_open:   bool,
    create_popup_items:  Vec<(NodeHandle, CreateKind)>,
    // Terrain tool buttons (Phase 14F): (button_handle, BrushMode index)
    terrain_tool_items:  Vec<(NodeHandle, u8)>,
    // Outliner row mapping: (button_handle, entity_index)
    outliner_rows:       Vec<(NodeHandle, u32)>,
    // Inspector field handles
    inspector_handles:   InspectorHandles,
    // Editor event queue drained by app.rs each frame
    editor_events:       VecDeque<EditorEvent>,
    // Viewport area handle — mouse events here pass through to the game
    #[allow(dead_code)]
    viewport_handle:     NodeHandle,
    last_outliner_state: Option<(Vec<(u32, String)>, Option<u32>)>,
    outer_grid:          NodeHandle,
    menu_bar_h:          NodeHandle,
    inner_h:             NodeHandle,
    toolbar_h:           NodeHandle,
    right_h:             NodeHandle,
    bottom_h:            NodeHandle,
}

impl UiManager {
    pub fn new(
        device:        &wgpu::Device,
        output_format: wgpu::TextureFormat,
        _msaa_samples: u32,
        queue:         &wgpu::Queue,
        window:        Arc<Window>,
    ) -> Self {
        info!("Initializing native UI…");

        let size = window.inner_size();
        let (sw, sh) = (size.width as f32, size.height as f32);
        let mut native_ui = UserInterface::new(sw, sh);

        let font_bytes = std::fs::read("C:\\Windows\\Fonts\\segoeui.ttf")
            .or_else(|_| std::fs::read("C:\\Windows\\Fonts\\arial.ttf"))
            .or_else(|_| std::fs::read("/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf"))
            .ok();
        let font_id: u8 = if let Some(bytes) = font_bytes {
            match native_ui.add_font(&bytes) {
                Ok(id) => { info!("Native UI: font loaded (id={})", id); id }
                Err(e) => { warn!("Native UI: font load failed — {}", e); 0 }
            }
        } else {
            warn!("Native UI: no system font found — text will not render");
            0
        };

        let layout  = build_editor_layout(&mut native_ui, font_id);
        let ui_pass = UiPass::new(device, queue, output_format);

        // Tell the UserInterface which handle is the viewport so mouse events pass through.
        native_ui.set_viewport_handle(layout.viewport_handle);

        Self {
            window_size:        (size.width, size.height),
            native_ui,
            ui_pass,
            font_id,
            outliner_scroll:    layout.outliner_scroll,
            outliner_stack:     layout.outliner_stack,
            inspector_stack:    layout.inspector_stack,
            log_stack:          layout.log_stack,
            log_entry_count:    0,
            create_button:      layout.create_button,
            create_popup:       layout.create_popup,
            create_popup_open:  false,
            create_popup_items: layout.create_popup_items,
            terrain_tool_items: layout.terrain_tool_items,
            outliner_rows:      Vec::new(),
            inspector_handles:  layout.inspector_handles,
            editor_events:      VecDeque::new(),
            viewport_handle:    layout.viewport_handle,
            last_outliner_state: None,
            outer_grid:         layout.outer_grid,
            menu_bar_h:         layout.menu_bar_h,
            inner_h:            layout.inner_h,
            toolbar_h:          layout.toolbar_h,
            right_h:            layout.right_h,
            bottom_h:           layout.bottom_h,
        }
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
                info!("{}: pos={:?}, size={:?}, desired={:?}, clip={:?}, vis={}, g_vis={}",
                    name, node.widget.actual_local_position, node.widget.actual_local_size, node.widget.desired_size, node.widget.clip_bounds, node.widget.visibility, node.widget.global_visibility);
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
        _window: &Window,
        device:  &wgpu::Device,
        queue:   &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view:    &wgpu::TextureView,
    ) {
        // Flush all queued widget messages; convert outgoing to EditorEvents.
        let outgoing = self.native_ui.update();
        self.process_outgoing(outgoing);

        let (w, h) = self.window_size;
        self.native_ui.perform_layout();
        self.native_ui.draw();
        self.ui_pass.prepare(device, queue, &mut self.native_ui.draw_ctx, w, h);
        self.ui_pass.render(encoder, view);
    }

    // ── OS event routing ─────────────────────────────────────────────────────

    /// Route a winit event into the widget tree.  Returns true if consumed.
    pub fn process_os_event(&mut self, event: &WindowEvent) -> bool {
        self.native_ui.process_os_event(event)
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

    /// Rebuild the outliner entity list.  `entities` is (entity_index, display_name).
    pub fn update_outliner(&mut self, entities: &[(u32, String)], selected: Option<u32>) {
        let new_state = (entities.to_vec(), selected);
        if let Some(ref old_state) = self.last_outliner_state {
            if *old_state == new_state {
                return; // No changes, do not destroy widgets
            }
        }
        self.last_outliner_state = Some(new_state);

        self.native_ui.clear_children(self.outliner_stack);
        self.outliner_rows.clear();

        let font_id    = self.font_id;
        let scroll_h   = self.outliner_scroll;
        let _ = scroll_h; // used for scrolling; content lives in outliner_stack directly

        for &(eidx, ref name) in entities {
            let is_sel = selected == Some(eidx);
            let bg     = if is_sel { theme::ACCENT_BLUE } else { [0, 0, 0, 0] };

            let btn = ButtonBuilder::new(
                WidgetBuilder::new()
                    .with_height(22.0)
                    .with_background(bg),
            ).build();
            let btn_h = self.native_ui.add_node(btn, self.outliner_stack);

            let lbl = TextBuilder::new(
                WidgetBuilder::new()
                    .with_margin(Thickness { left: 8.0, top: 3.0, right: 0.0, bottom: 0.0 }),
            )
            .with_text(name.as_str())
            .with_font_size(12.0)
            .with_font_id(font_id)
            .with_color(theme::TEXT_PRIMARY)
            .build();
            self.native_ui.add_node(lbl, btn_h);

            self.outliner_rows.push((btn_h, eidx));
        }
    }

    /// Update inspector NumericFields from a Transform.
    /// `transform` is (translation, euler_degrees, scale)`.
    pub fn update_inspector(
        &mut self,
        entity_idx: Option<u32>,
        pos:        Option<[f32; 3]>,
        rot_deg:    Option<[f32; 3]>,
        scale:      Option<[f32; 3]>,
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

    /// Show or hide the inspector's Light section and refresh its values
    /// (Phase 13E). Pass `None` when the selection has no `LightComponent`.
    ///
    /// `values` is `[intensity, range, inner_deg, outer_deg]`.
    pub fn update_light_inspector(&mut self, values: Option<LightInspectorValues>) {
        let h = &self.inspector_handles;
        let (section, intensity, range, inner, outer) = (
            h.light_section, h.light_intensity, h.light_range, h.light_inner, h.light_outer,
        );
        match values {
            Some([i, r, ia, oa]) => {
                self.native_ui.set_visibility(section, true);
                self.native_ui.send(NumericFieldMessage::set_value(intensity, i));
                self.native_ui.send(NumericFieldMessage::set_value(range, r));
                self.native_ui.send(NumericFieldMessage::set_value(inner, ia));
                self.native_ui.send(NumericFieldMessage::set_value(outer, oa));
            }
            None => self.native_ui.set_visibility(section, false),
        }
    }

    /// Append a line to the output log panel (max 200 entries).
    pub fn append_log(&mut self, text: &str) {
        const MAX: usize = 200;
        if self.log_entry_count >= MAX { return; }
        let font_id = self.font_id;
        let entry = TextBuilder::new(
            WidgetBuilder::new()
                .with_margin(Thickness { left: 8.0, top: 1.0, right: 0.0, bottom: 0.0 }),
        )
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
            (h.pos_x, IF::PosX), (h.pos_y, IF::PosY), (h.pos_z, IF::PosZ),
            (h.rot_x, IF::RotX), (h.rot_y, IF::RotY), (h.rot_z, IF::RotZ),
            (h.sc_x,  IF::ScaleX), (h.sc_y, IF::ScaleY), (h.sc_z, IF::ScaleZ),
            (h.light_intensity, IF::LightIntensity),
            (h.light_range,     IF::LightRange),
            (h.light_inner,     IF::LightInnerAngle),
            (h.light_outer,     IF::LightOuterAngle),
        ];

        for msg in msgs {
            if let Some(ButtonMessage::Click) = msg.data::<ButtonMessage>() {
                // Outliner row
                if let Some(&(_, eidx)) = self.outliner_rows.iter().find(|(bh, _)| *bh == msg.destination) {
                    self.editor_events.push_back(EditorEvent::SelectEntity(Some(eidx)));
                    continue;
                }
                // Terrain tool button (Phase 14F)
                if let Some(&(_, tool)) = self.terrain_tool_items.iter().find(|(bh, _)| *bh == msg.destination) {
                    self.editor_events.push_back(EditorEvent::SetTerrainTool(tool));
                    continue;
                }
                // Create popup item
                if let Some(&(_, kind)) = self.create_popup_items.iter().find(|(bh, _)| *bh == msg.destination) {
                    self.editor_events.push_back(EditorEvent::CreateEntity(kind));
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
                if msg.destination == self.create_button {
                    self.create_popup_open = !self.create_popup_open;
                    let open = self.create_popup_open;
                    self.native_ui.send(UiMessage::new(
                        self.create_popup,
                        MessageDirection::ToWidget,
                        if open { PopupMessage::Open } else { PopupMessage::Close },
                    ));
                    self.native_ui.invalidate_ancestors(self.create_popup);
                    continue;
                }
            } else if let Some(PopupMessage::Close) = msg.data::<PopupMessage>() {
                if msg.destination == self.create_popup {
                    self.create_popup_open = false;
                    self.native_ui.invalidate_ancestors(self.create_popup);
                }
            }

            // — NumericField value changes ————————
            if let Some(NumericFieldMessage::ValueChanged(v)) = msg.data::<NumericFieldMessage>() {
                let v = *v;
                if let Some(&(_, field)) = field_map.iter().find(|(fh, _)| *fh == msg.destination) {
                    self.editor_events.push_back(EditorEvent::SetInspectorValue { field, value: v });
                }
            }
        }
    }
}

// ── Editor layout builder ─────────────────────────────────────────────────────

fn build_editor_layout(ui: &mut UserInterface, font_id: u8) -> EditorLayout {
    let root = ui.root();

    // ── Outer grid: 3 rows × 1 col ───────────────────────────────────────────
    let outer_grid = GridBuilder::new(
        WidgetBuilder::new().with_background(theme::TRANSPARENT),
    )
    .add_row(Row::strict(28.0))
    .add_row(Row::stretch())
    .add_row(Row::strict(160.0))
    .add_column(Column::stretch())
    .build();
    let outer_h = ui.add_node(outer_grid, root);

    // ── Row 0: menu bar ───────────────────────────────────────────────────────
    let menu_bar = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(0).with_column(0)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness { left: 0.0, right: 0.0, top: 0.0, bottom: 1.0 })
    .build();
    let menu_bar_h = ui.add_node(menu_bar, outer_h);

    // Menu bar grid: [stretch col for menu items | auto col for FPS counter]
    let menu_grid = GridBuilder::new(
        WidgetBuilder::new().with_background(theme::TRANSPARENT),
    )
    .add_row(Row::stretch())
    .add_column(Column::stretch())  // col 0 — menu items
    .add_column(Column::auto())     // col 1 — FPS (right-aligned)
    .build();
    let menu_grid_h = ui.add_node(menu_grid, menu_bar_h);

    // Horizontal stack for menu items (col 0)
    let menu_stack = StackPanelBuilder::new(
        WidgetBuilder::new()
            .with_row(0).with_column(0)
            .with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Horizontal)
    .build();
    let menu_stack_h = ui.add_node(menu_stack, menu_grid_h);

    // Engine title
    let title = TextBuilder::new(
        WidgetBuilder::new()
            .with_margin(Thickness { left: 10.0, right: 16.0, top: 6.0, bottom: 0.0 })
            .with_foreground(theme::TEXT_SECONDARY),
    )
    .with_text("Somnium Engine")
    .with_font_size(12.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_SECONDARY)
    .build();
    ui.add_node(title, menu_stack_h);

    // "File" / "Edit" — plain text (no action yet)
    for label in ["File", "Edit"] {
        let item = TextBuilder::new(
            WidgetBuilder::new().with_margin(Thickness::axes(8.0, 6.0)),
        )
        .with_text(label)
        .with_font_size(13.0)
        .with_font_id(font_id)
        .with_color(theme::TEXT_PRIMARY)
        .build();
        ui.add_node(item, menu_stack_h);
    }

    // "Create" — Menu so clicks are captured
    let create_btn_node = MenuBuilder::new(
        WidgetBuilder::new()
            .with_margin(Thickness { left: 0.0, right: 0.0, top: 0.0, bottom: 0.0 })
            .with_background(theme::TRANSPARENT),
    ).build();
    let create_button = ui.add_node(create_btn_node, menu_stack_h);
    let create_lbl = TextBuilder::new(
        WidgetBuilder::new().with_margin(Thickness::axes(8.0, 6.0)),
    )
    .with_text("Create")
    .with_font_size(13.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_PRIMARY)
    .build();
    ui.add_node(create_lbl, create_button);

    // "View" — plain text
    let view_item = TextBuilder::new(
        WidgetBuilder::new().with_margin(Thickness::axes(8.0, 6.0)),
    )
    .with_text("View")
    .with_font_size(13.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_PRIMARY)
    .build();
    ui.add_node(view_item, menu_stack_h);

    // ── Row 1: inner grid — toolbar | viewport | right panel ─────────────────
    let inner_grid = GridBuilder::new(
        WidgetBuilder::new()
            .with_row(1).with_column(0)
            .with_background(theme::TRANSPARENT),
    )
    .add_row(Row::stretch())
    .add_column(Column::strict(40.0))
    .add_column(Column::stretch())
    .add_column(Column::strict(280.0))
    .build();
    let inner_h = ui.add_node(inner_grid, outer_h);

    // Left toolbar strip
    let toolbar = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(0).with_column(0)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness { left: 0.0, right: 1.0, top: 0.0, bottom: 0.0 })
    .build();
    let toolbar_h = ui.add_node(toolbar, inner_h);

    // Terrain tool palette (Phase 14F): label + 6 brush mode buttons.
    // Active only while a terrain entity is selected (F6 toggles edit mode).
    let tool_stack = StackPanelBuilder::new(
        WidgetBuilder::new().with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Vertical)
    .build();
    let tool_stack_h = ui.add_node(tool_stack, toolbar_h);

    let ter_lbl = TextBuilder::new(
        WidgetBuilder::new().with_margin(Thickness { left: 7.0, top: 8.0, right: 0.0, bottom: 2.0 }),
    )
    .with_text("TER")
    .with_font_size(10.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_SECONDARY)
    .build();
    ui.add_node(ter_lbl, tool_stack_h);

    // (label, BrushMode index): Raise, Lower, Smooth, Flatten, Noise, Paint.
    const TERRAIN_TOOLS: &[(&str, u8)] =
        &[("Rs", 0), ("Lw", 1), ("Sm", 2), ("Fl", 3), ("Nz", 4), ("Pt", 5)];
    let mut terrain_tool_items = Vec::with_capacity(TERRAIN_TOOLS.len());
    for &(label, tool) in TERRAIN_TOOLS {
        let btn = ButtonBuilder::new(
            WidgetBuilder::new()
                .with_height(24.0)
                .with_margin(Thickness { left: 4.0, top: 2.0, right: 4.0, bottom: 0.0 })
                .with_background(theme::BG_DARK),
        ).build();
        let btn_h = ui.add_node(btn, tool_stack_h);

        let lbl = TextBuilder::new(
            WidgetBuilder::new()
                .with_margin(Thickness { left: 8.0, top: 5.0, right: 0.0, bottom: 0.0 }),
        )
        .with_text(label)
        .with_font_size(11.0)
        .with_font_id(font_id)
        .with_color(theme::TEXT_PRIMARY)
        .build();
        ui.add_node(lbl, btn_h);
        terrain_tool_items.push((btn_h, tool));
    }

    // Viewport area (col 1) — transparent, no hit-test. Mouse events in this region
    // will hit-test to this handle, which the UI knows to NOT consume.
    let viewport_border = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(0).with_column(1)
            .with_background(theme::TRANSPARENT)
            .with_foreground(theme::TRANSPARENT),
    )
    .with_stroke_thickness(Thickness::ZERO)
    .build();
    let viewport_handle = ui.add_node(viewport_border, inner_h);

    // Right panel: two sections (outliner top, inspector bottom)
    let right_border = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(0).with_column(2)
            .with_background(theme::BG_DARK)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness { left: 1.0, right: 0.0, top: 0.0, bottom: 0.0 })
    .build();
    let right_h = ui.add_node(right_border, inner_h);

    let right_grid = GridBuilder::new(
        WidgetBuilder::new().with_background(theme::TRANSPARENT),
    )
    .add_row(Row::strict(24.0))   // Outliner header
    .add_row(Row::strict(200.0))  // Outliner content
    .add_row(Row::strict(24.0))   // Inspector header
    .add_row(Row::stretch())      // Inspector content
    .add_column(Column::stretch())
    .build();
    let right_grid_h = ui.add_node(right_grid, right_h);

    // Outliner header
    let out_hdr = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(0).with_column(0)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness { left: 0.0, right: 0.0, top: 0.0, bottom: 1.0 })
    .build();
    let out_hdr_h = ui.add_node(out_hdr, right_grid_h);
    let out_hdr_txt = TextBuilder::new(
        WidgetBuilder::new()
            .with_margin(Thickness { left: 8.0, top: 5.0, right: 0.0, bottom: 0.0 }),
    )
    .with_text("OUTLINER")
    .with_font_size(11.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_SECONDARY)
    .build();
    ui.add_node(out_hdr_txt, out_hdr_h);

    // Outliner content (ScrollViewer + inner StackPanel)
    let out_scroll = ScrollViewerBuilder::new(
        WidgetBuilder::new()
            .with_row(1).with_column(0)
            .with_background(theme::BG_DARK),
    ).build();
    let outliner_scroll = ui.add_node(out_scroll, right_grid_h);

    let out_stack = StackPanelBuilder::new(
        WidgetBuilder::new().with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Vertical)
    .build();
    let outliner_stack = ui.add_node(out_stack, outliner_scroll);

    // Inspector header
    let ins_hdr = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(2).with_column(0)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness { left: 0.0, right: 0.0, top: 1.0, bottom: 1.0 })
    .build();
    let ins_hdr_h = ui.add_node(ins_hdr, right_grid_h);
    let ins_hdr_txt = TextBuilder::new(
        WidgetBuilder::new()
            .with_margin(Thickness { left: 8.0, top: 5.0, right: 0.0, bottom: 0.0 }),
    )
    .with_text("INSPECTOR")
    .with_font_size(11.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_SECONDARY)
    .build();
    ui.add_node(ins_hdr_txt, ins_hdr_h);

    // Inspector content
    let ins_content = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(3).with_column(0)
            .with_background(theme::BG_DARK)
            .with_foreground(theme::TRANSPARENT),
    )
    .with_stroke_thickness(Thickness::ZERO)
    .build();
    let ins_content_h = ui.add_node(ins_content, right_grid_h);

    let inspector_stack = StackPanelBuilder::new(
        WidgetBuilder::new().with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Vertical)
    .build();
    let inspector_stack = ui.add_node(inspector_stack, ins_content_h);

    let inspector_handles = build_inspector(ui, inspector_stack, font_id);

    // ── Row 2: bottom log panel ───────────────────────────────────────────────
    let bottom = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(2).with_column(0)
            .with_background(theme::BG_DARK)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness { left: 0.0, right: 0.0, top: 1.0, bottom: 0.0 })
    .build();
    let bottom_h = ui.add_node(bottom, outer_h);

    // Inner grid: header (strict) + scrollable log content (stretch)
    let log_grid = GridBuilder::new(
        WidgetBuilder::new().with_background(theme::TRANSPARENT),
    )
    .add_row(Row::strict(22.0))   // header bar
    .add_row(Row::stretch())      // log content
    .add_column(Column::stretch())
    .build();
    let log_grid_h = ui.add_node(log_grid, bottom_h);

    // Header
    let log_hdr_border = BorderBuilder::new(
        WidgetBuilder::new()
            .with_row(0).with_column(0)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness { left: 0.0, right: 0.0, top: 0.0, bottom: 1.0 })
    .build();
    let log_hdr_h = ui.add_node(log_hdr_border, log_grid_h);

    let log_header = TextBuilder::new(
        WidgetBuilder::new()
            .with_margin(Thickness { left: 8.0, top: 4.0, right: 0.0, bottom: 0.0 }),
    )
    .with_text("Output Log")
    .with_font_size(11.0)
    .with_font_id(font_id)
    .with_color(theme::TEXT_SECONDARY)
    .build();
    ui.add_node(log_header, log_hdr_h);

    // Scrollable log content
    let log_scroll = ScrollViewerBuilder::new(
        WidgetBuilder::new()
            .with_row(1).with_column(0)
            .with_background(theme::BG_DARK),
    ).build();
    let log_scroll_h = ui.add_node(log_scroll, log_grid_h);

    let log_stack_node = StackPanelBuilder::new(
        WidgetBuilder::new().with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Vertical)
    .build();
    let log_stack = ui.add_node(log_stack_node, log_scroll_h);

    // ── Create popup overlay (child of root, drawn on top) ────────────────────
    let (create_popup, create_popup_items) = build_create_popup(ui, root, font_id);

    EditorLayout {
        outliner_scroll,
        outliner_stack,
        inspector_stack,
        log_stack,
        create_button,
        create_popup,
        create_popup_items,
        terrain_tool_items,
        inspector_handles,
        viewport_handle,
        outer_grid: outer_h,
        menu_bar_h,
        inner_h,
        toolbar_h,
        right_h,
        bottom_h,
    }
}

/// Build the 9 NumericFields for the inspector TRS section.
/// Returns the inspector handle bundle.
fn build_inspector(ui: &mut UserInterface, parent: NodeHandle, font_id: u8) -> InspectorHandles {
    // `label_w` widens the gutter for the light section's longer labels.
    let make_row_w = |ui: &mut UserInterface,
                      label: &str,
                      label_w: f32,
                      font_id: u8,
                      parent: NodeHandle| {
        let row = StackPanelBuilder::new(
            WidgetBuilder::new()
                .with_height(22.0)
                .with_background(theme::TRANSPARENT),
        )
        .with_orientation(Orientation::Horizontal)
        .build();
        let row_h = ui.add_node(row, parent);

        let lbl = TextBuilder::new(
            WidgetBuilder::new()
                .with_width(label_w)
                .with_margin(Thickness { left: 6.0, top: 4.0, right: 4.0, bottom: 0.0 }),
        )
        .with_text(label)
        .with_font_size(11.0)
        .with_font_id(font_id)
        .with_color(theme::TEXT_SECONDARY)
        .build();
        ui.add_node(lbl, row_h);

        let field = NumericFieldBuilder::new(
            WidgetBuilder::new()
                .with_margin(Thickness { left: 0.0, top: 2.0, right: 4.0, bottom: 0.0 }),
        )
        .with_font_size(12.0)
        .with_font_id(font_id)
        .build();
        ui.add_node(field, row_h)
    };
    let make_row = |ui: &mut UserInterface, label: &str, font_id: u8, parent: NodeHandle| {
        make_row_w(ui, label, 20.0, font_id, parent)
    };

    let sec_label = |ui: &mut UserInterface, text: &str, font_id: u8, parent: NodeHandle| {
        let lbl = TextBuilder::new(
            WidgetBuilder::new()
                .with_margin(Thickness { left: 6.0, top: 6.0, right: 0.0, bottom: 2.0 }),
        )
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
    let sc_x  = make_row(ui, "X", font_id, parent);
    let sc_y  = make_row(ui, "Y", font_id, parent);
    let sc_z  = make_row(ui, "Z", font_id, parent);

    // ── Light section (Phase 13E) ────────────────────────────────────────────
    // Lives in its own panel so it can be hidden when the selection isn't a
    // light. Angles are shown in degrees; range/angles only apply to
    // point/spot lights (a directional light ignores them).
    let light_panel = StackPanelBuilder::new(
        WidgetBuilder::new().with_background(theme::TRANSPARENT),
    )
    .with_orientation(Orientation::Vertical)
    .build();
    let light_section = ui.add_node(light_panel, parent);

    sec_label(ui, "Light", font_id, light_section);
    let light_intensity = make_row_w(ui, "Int",  34.0, font_id, light_section);
    let light_range     = make_row_w(ui, "Rng",  34.0, font_id, light_section);
    let light_inner     = make_row_w(ui, "In°",  34.0, font_id, light_section);
    let light_outer     = make_row_w(ui, "Out°", 34.0, font_id, light_section);
    ui.set_visibility(light_section, false);

    InspectorHandles {
        pos_x, pos_y, pos_z, rot_x, rot_y, rot_z, sc_x, sc_y, sc_z,
        light_section, light_intensity, light_range, light_inner, light_outer,
    }
}

/// Build the Create dropdown popup (initially hidden, child of root).
fn build_create_popup(
    ui:      &mut UserInterface,
    root:    NodeHandle,
    font_id: u8,
) -> (NodeHandle, Vec<(NodeHandle, CreateKind)>) {
    let popup_backdrop = PopupBuilder::new(
        WidgetBuilder::new()
            .with_background(theme::TRANSPARENT)
    ).build();
    let popup_h = ui.add_node(popup_backdrop, root);

    let popup_border = BorderBuilder::new(
        WidgetBuilder::new()
            .with_desired_position(Vec2::new(148.0, 28.0))
            .with_width(160.0)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness::uniform(1.0))
    .build();
    let popup_border_h = ui.add_node(popup_border, popup_h);

    let popup_stack = StackPanelBuilder::new(
        WidgetBuilder::new().with_background(theme::TRANSPARENT),
    )
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
        ).build();
        let btn_h = ui.add_node(btn, popup_stack_h);

        let lbl = TextBuilder::new(
            WidgetBuilder::new()
                .with_margin(Thickness { left: 8.0, top: 4.0, right: 0.0, bottom: 0.0 }),
        )
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
