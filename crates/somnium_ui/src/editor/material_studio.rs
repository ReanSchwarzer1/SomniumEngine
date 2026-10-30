//! The material editor's studio: the live preview at the top of the Material panel, its shape and turntable
//! controls, and the presets.
//!
//! The picture is the renderer's preview studio (`somnium_renderer::pass::preview_studio`): the material on a sphere,
//! cube or plane, lit and drawn on the GPU every frame from the same material slot the level uses, so an edit shows as
//! it is made. The host hands the studio's texture over once (`UiManager::set_material_preview_texture`) and says each
//! frame, through `ToolContext::material_preview`, whether it drew this panel's material. Until it has, the panel shows
//! the material's CPU thumbnail, which is also what a headless editor and the tests see.
//!
//! The controls are rebuilt with Details; their handles live on the tool panel, whose event hook routes them, and
//! the state they show is core's.
use std::path::PathBuf;

use glam::Vec2;

use crate::{
    draw::DrawingContext,
    editor_event::{EditorEvent, MaterialStudioEvent},
    icons::IconId,
    message::{MessageDirection, NodeHandle, UiMessage, WidgetMessage},
    node::{Control, CursorKind, LayoutCtx, UiNode},
    theme,
    types::{HorizontalAlignment, Rect, Thickness, VerticalAlignment},
    typography::TextRole,
    ui::UserInterface,
    widget::{Widget, WidgetBuilder},
    widgets::{
        border::BorderBuilder,
        button::{ButtonBuilder, ButtonMessage},
        grid::{Column, GridBuilder, Row},
        stack_panel::{Orientation, StackPanelBuilder},
        text::TextBuilder,
        wrap_panel::WrapPanelBuilder,
    },
};

/// The preview's smallest and largest side, logical pixels. It fills the panel's width up to the largest.
const MIN_SIDE: f32 = 140.0;
const MAX_SIDE: f32 = 420.0;
/// Radians the preview turns per logical pixel dragged across it: its width is about a full turn.
const TURN_PER_PIXEL: f32 = 0.012;

/// Everything the studio shows a designer, for the guard against project names in designer text (`debug.rs`).
#[cfg(test)]
pub(crate) const STUDIO_TEXT: &[&str] = &[
    PREVIEW_TIP,
    SPIN_TIP,
    PRESETS_LABEL,
    PRESET_NOTE,
    SHAPE_TIPS[0],
    SHAPE_TIPS[1],
    SHAPE_TIPS[2],
];
const PREVIEW_TIP: &str = "This material as the renderer draws it, updated as you edit. Drag to turn it.";
const SPIN_TIP: &str = "Turn the preview slowly. Drag across it to turn it by hand.";
const PRESETS_LABEL: &str = "Presets";
const PRESET_NOTE: &str = "Changes how the surface takes light. Texture slots stay, and a base-colour texture keeps its colours. Ctrl+Z undoes it.";
const SHAPE_TIPS: [&str; 3] = [
    "Show the material on a sphere.",
    "Show the material on a cube: flat faces and hard edges.",
    "Show the material on a flat panel tilted toward you.",
];
const SHAPE_ICONS: [IconId; 3] = [IconId::Sphere, IconId::Cube, IconId::Plane];

/// What core says about the preview each frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialPreviewState {
    /// 0 sphere, 1 cube, 2 plane.
    pub shape: u8,
    /// On its turntable.
    pub spinning: bool,
    /// The renderer drew this panel's material into the preview this frame.
    pub live: bool,
}

impl Default for MaterialPreviewState {
    fn default() -> Self {
        Self { shape: 0, spinning: true, live: false }
    }
}

/// Messages to and from the preview picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MaterialPreviewMessage {
    /// To the widget: the UI texture slot holding the renderer's picture, and whether it shows this material.
    SetLive { slot: Option<u32>, live: bool },
    /// From the widget: a drag turned it by this many radians.
    Turn(f32),
}

/// The preview picture: the renderer's texture when it is live, the material's thumbnail until then.
pub struct MaterialPreview {
    slot: Option<u32>,
    live: bool,
    fallback: Option<PathBuf>,
    drag_x: Option<f32>,
}

impl MaterialPreview {
    /// The square the picture fills inside `bounds`, centred.
    fn square(bounds: Rect) -> Rect {
        let side = bounds.w.min(bounds.h).max(0.0);
        Rect::new(bounds.x + (bounds.w - side) * 0.5, bounds.y + (bounds.h - side) * 0.5, side, side)
    }
}

impl Control for MaterialPreview {
    fn role(&self) -> crate::a11y::Role {
        crate::a11y::Role::Image
    }

    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let width = if available.x.is_finite() { available.x } else { MAX_SIDE };
        Vec2::new(width, width.clamp(MIN_SIDE, MAX_SIDE))
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let rect = Self::square(widget.screen_bounds());
        if let (true, Some(slot)) = (self.live, self.slot) {
            ctx.push_primitive(srgb_picture(rect, [0.0, 0.0, 1.0, 1.0]), Some(slot));
            return;
        }
        if let Some(uv) = self.fallback.as_deref().and_then(|path| ctx.thumbnails.uv(path)) {
            // The 64-texel thumbnail, kept to half the stage so it does not read as the real preview blown up.
            let half = Rect::new(rect.x + rect.w * 0.25, rect.y + rect.h * 0.25, rect.w * 0.5, rect.h * 0.5);
            ctx.push_primitive(srgb_picture(half, uv), Some(crate::thumbnail::THUMBNAIL_ATLAS_TEXTURE_ID));
            return;
        }
        let icon = Rect::new(rect.x + rect.w * 0.5 - 24.0, rect.y + rect.h * 0.5 - 24.0, 48.0, 48.0);
        let (uv, texture) = IconId::MaterialBall.draw_quad(icon);
        ctx.push_textured_rect(icon, uv, theme::active().semantic.text.secondary.bytes(), texture);
    }

    fn cursor_icon(&self, _widget: &Widget, _pos: Vec2) -> CursorKind {
        CursorKind::EwResize
    }

    fn handle_routed_message(&mut self, widget: &mut Widget, msg: &mut UiMessage, emit: &mut Vec<UiMessage>) {
        if msg.destination != widget.handle {
            return;
        }
        if let Some(MaterialPreviewMessage::SetLive { slot, live }) = msg.data::<MaterialPreviewMessage>() {
            if msg.direction == MessageDirection::ToWidget {
                self.slot = *slot;
                self.live = *live;
                msg.handled = true;
            }
            return;
        }
        match msg.data::<WidgetMessage>() {
            Some(WidgetMessage::MouseDown { pos, .. }) => {
                self.drag_x = Some(pos.x);
                msg.handled = true;
            }
            Some(WidgetMessage::MouseMove { pos, .. }) => {
                if let Some(last) = self.drag_x {
                    let dx = pos.x - last;
                    if dx != 0.0 {
                        self.drag_x = Some(pos.x);
                        emit.push(UiMessage::new(
                            widget.handle,
                            MessageDirection::FromWidget,
                            MaterialPreviewMessage::Turn(dx * TURN_PER_PIXEL),
                        ));
                    }
                    msg.handled = true;
                }
            }
            Some(WidgetMessage::MouseUp { .. }) => {
                if self.drag_x.take().is_some() {
                    msg.handled = true;
                }
            }
            _ => {}
        }
    }
}

/// A textured quad whose texels are sRGB colour (the renderer's picture, a thumbnail), decoded as the fills are.
pub(crate) fn srgb_picture(rect: Rect, uv: [f32; 4]) -> crate::primitive::Primitive {
    let mut quad = crate::primitive::Primitive::textured(rect, uv, [255; 4]);
    quad.flags |= crate::primitive::FLAG_SRGB_TEXELS;
    quad
}

/// The studio's widgets and the state they last showed.
#[derive(Default)]
pub struct MaterialStudio {
    pub preview: NodeHandle,
    pub shapes: [NodeHandle; 3],
    pub spin: NodeHandle,
    pub presets: Vec<(NodeHandle, usize)>,
    pub state: MaterialPreviewState,
    /// The UI texture slot the renderer's picture is bound to, once the host has handed it over.
    pub slot: Option<u32>,
    /// A Material row (`somnium.asset.Material/<field>`) to scroll into view once it exists: the capture hook's.
    pub reveal: Option<String>,
}

/// The thumbnail-cache key a preset's swatch is filed under (not a file: the cache is keyed by path).
fn swatch_key(index: usize) -> PathBuf {
    PathBuf::from(format!("somnium-material-preset/{index}"))
}

/// Each preset on the CPU studio sphere, made once.
fn swatches() -> &'static [Vec<u8>] {
    static SWATCHES: std::sync::OnceLock<Vec<Vec<u8>>> = std::sync::OnceLock::new();
    SWATCHES.get_or_init(|| {
        somnium_asset::material::MATERIAL_PRESETS
            .iter()
            .map(|preset| somnium_asset::preview::render_material_sphere(&preset.sample()))
            .collect()
    })
}

/// Put any preset swatch the thumbnail atlas does not hold (never delivered, or evicted while hidden) back in it.
pub(crate) fn keep_swatches(ui: &mut UserInterface) {
    for (index, rgba) in swatches().iter().enumerate() {
        let key = swatch_key(index);
        if !matches!(ui.draw_ctx.thumbnails.state(&key), Some(crate::thumbnail::ThumbState::Ready(_))) {
            ui.draw_ctx.thumbnails.deliver(&key, rgba);
        }
    }
}

impl MaterialStudio {
    /// Build the stage, its controls and the presets under `parent`. `fallback` is the material's file, whose
    /// thumbnail stands in until the renderer's picture is live.
    pub fn build(&mut self, ui: &mut UserInterface, parent: NodeHandle, fallback: Option<PathBuf>, font_id: u8) {
        let t = theme::active();
        if let Some(path) = fallback.as_deref() {
            ui.draw_ctx.thumbnails.request(path, true);
        }
        let stage = ui.add_node(
            BorderBuilder::new(
                WidgetBuilder::new()
                    .with_margin(Thickness::axes(8.0, 6.0))
                    .with_background(t.semantic.surface.canvas.bytes())
                    .with_foreground(t.semantic.border.subtle.bytes()),
            )
            .with_stroke_thickness(Thickness::uniform(t.geometry.stroke_hairline))
            .build(),
            parent,
        );
        let cell = ui.add_node(
            GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
                .add_row(Row::auto())
                .add_column(Column::stretch())
                .build(),
            stage,
        );
        self.preview = ui.add_node(
            UiNode::new(
                WidgetBuilder::new().with_tooltip(PREVIEW_TIP).build(),
                Box::new(MaterialPreview { slot: self.slot, live: self.state.live, fallback, drag_x: None }),
            ),
            cell,
        );
        let corner = |ui: &mut UserInterface, align| {
            ui.add_node(
                StackPanelBuilder::new(
                    WidgetBuilder::new()
                        .with_horizontal_alignment(align)
                        .with_vertical_alignment(VerticalAlignment::Top)
                        .with_margin(Thickness::uniform(6.0))
                        .with_background(theme::TRANSPARENT),
                )
                .with_orientation(Orientation::Horizontal)
                .build(),
                cell,
            )
        };
        let shapes = corner(ui, HorizontalAlignment::Left);
        self.shapes = std::array::from_fn(|i| super::parts::icon_tool_button(ui, shapes, SHAPE_ICONS[i], SHAPE_TIPS[i]));
        let turn = corner(ui, HorizontalAlignment::Right);
        self.spin = super::parts::icon_tool_button(ui, turn, IconId::Rotate, SPIN_TIP);

        ui.add_node(
            TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::new(8.0, 4.0, 8.0, 2.0)))
                .with_text(PRESETS_LABEL)
                .with_role(TextRole::Label)
                .build(),
            parent,
        );
        let chips = ui.add_node(
            WrapPanelBuilder::new(WidgetBuilder::new().with_margin(Thickness::new(8.0, 0.0, 8.0, 6.0)).with_background(theme::TRANSPARENT))
                .with_gap(4.0, 4.0)
                .build(),
            parent,
        );
        keep_swatches(ui);
        self.presets = somnium_asset::material::MATERIAL_PRESETS
            .iter()
            .enumerate()
            .map(|(index, preset)| {
                let chip = ui.add_node(
                    ButtonBuilder::new(
                        WidgetBuilder::new()
                            .with_height(t.density.row_chrome)
                            .with_background(t.semantic.surface.raised.bytes())
                            .with_tooltip(format!("{} {PRESET_NOTE}", preset.description)),
                    )
                    .build(),
                    chips,
                );
                let line = ui.add_node(
                    StackPanelBuilder::new(
                        WidgetBuilder::new()
                            .with_margin(Thickness::new(4.0, 0.0, 8.0, 0.0))
                            .with_background(theme::TRANSPARENT)
                            .with_hit_test_visibility(false),
                    )
                    .with_orientation(Orientation::Horizontal)
                    .build(),
                    chip,
                );
                ui.add_node(
                    UiNode::new(
                        WidgetBuilder::new().with_vertical_alignment(VerticalAlignment::Center).build(),
                        Box::new(crate::widgets::combo_box::AssetSwatch {
                            asset: Some(swatch_key(index)),
                            icon: IconId::MaterialBall,
                            size: 20.0,
                            framed: false,
                        }),
                    ),
                    line,
                );
                ui.add_node(
                    TextBuilder::new(
                        WidgetBuilder::new()
                            .with_margin(Thickness::new(6.0, 0.0, 0.0, 0.0))
                            .with_vertical_alignment(VerticalAlignment::Center),
                    )
                    .with_text(preset.name)
                    .with_role(TextRole::Label)
                    .with_font_id(font_id)
                    .build(),
                    line,
                );
                (chip, index)
            })
            .collect();
        let state = self.state;
        self.show(ui, state);
    }

    /// Forget the widgets (Details was rebuilt without a material). The state, slot and any pending reveal stay.
    pub fn clear(&mut self) {
        let (state, slot, reveal) = (self.state, self.slot, self.reveal.take());
        *self = Self { state, slot, reveal, ..Self::default() };
    }

    /// Show `state` on the widgets, if there are any.
    pub fn show(&mut self, ui: &mut UserInterface, state: MaterialPreviewState) {
        self.state = state;
        if self.preview.is_none() {
            return;
        }
        for (i, handle) in self.shapes.iter().enumerate() {
            ui.send(ButtonMessage::set_selected(*handle, i == usize::from(state.shape)));
        }
        ui.send(ButtonMessage::set_selected(self.spin, state.spinning));
        ui.send(UiMessage::new(
            self.preview,
            MessageDirection::ToWidget,
            MaterialPreviewMessage::SetLive { slot: self.slot, live: state.live },
        ));
    }

    /// The editor event a message from one of the studio's widgets asks for.
    #[must_use]
    pub fn event(&self, msg: &UiMessage) -> Option<EditorEvent> {
        if msg.direction != MessageDirection::FromWidget || msg.destination.is_none() {
            return None;
        }
        if let Some(MaterialPreviewMessage::Turn(radians)) = msg.data::<MaterialPreviewMessage>() {
            return (msg.destination == self.preview).then_some(EditorEvent::MaterialStudio(MaterialStudioEvent::Turn(*radians)));
        }
        if !matches!(msg.data::<ButtonMessage>(), Some(ButtonMessage::Click)) {
            return None;
        }
        let event = if let Some(shape) = self.shapes.iter().position(|h| *h == msg.destination) {
            MaterialStudioEvent::Shape(shape as u8)
        } else if msg.destination == self.spin {
            MaterialStudioEvent::Spin(!self.state.spinning)
        } else {
            let (_, index) = self.presets.iter().find(|(h, _)| *h == msg.destination)?;
            MaterialStudioEvent::Preset(*index)
        };
        Some(EditorEvent::MaterialStudio(event))
    }
}

/// The colour behind the preview, linear: the stage's own, so the picture sits in the panel without a seam.
#[must_use]
pub fn preview_backdrop() -> [f32; 3] {
    let linear = theme::LinearRgba::from_srgb(theme::active().semantic.surface.canvas).0;
    [linear[0], linear[1], linear[2]]
}

impl crate::UiManager {
    /// Bind the renderer's preview picture (8-bit, sRGB-encoded bytes) as one of the UI's textures. Once: later calls
    /// return true without rebinding. False when the UI has no texture slot left.
    pub fn set_material_preview_texture(&mut self, view: wgpu::TextureView) -> bool {
        let studio = &mut self.persona.tool_panel.studio;
        if studio.slot.is_some() {
            return true;
        }
        let Some(slot) = self.native_ui.draw_ctx.register_texture() else {
            return false;
        };
        if !self.ui_pass.set_texture(slot, view) {
            return false;
        }
        studio.slot = Some(slot);
        let state = studio.state;
        studio.show(&mut self.native_ui, state);
        true
    }

    /// Once a frame while the editor runs: keep the preset swatches in the thumbnail atlas, and scroll a row asked
    /// for by the capture hook into view once it has been laid out.
    pub fn keep_material_swatches(&mut self) {
        if self.persona.tool_panel.studio.preview.is_none() {
            return;
        }
        keep_swatches(&mut self.native_ui);
        if let Some(key) = self.persona.tool_panel.studio.reveal.clone()
            && let Some(row) = self.persona.row_handle(&key)
            && self.native_ui.screen_bounds(row).h > 0.0
        {
            self.native_ui.bring_focus_into_view(row);
            self.persona.tool_panel.studio.reveal = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built() -> (UserInterface, MaterialStudio) {
        let mut ui = UserInterface::new(420.0, 800.0);
        let root = ui.root();
        let mut studio = MaterialStudio::default();
        studio.build(&mut ui, root, None, 0);
        ui.update();
        ui.perform_layout();
        (ui, studio)
    }

    fn from<T: std::any::Any + Send>(dest: NodeHandle, data: T) -> UiMessage {
        UiMessage::new(dest, MessageDirection::FromWidget, data)
    }

    #[test]
    fn every_control_asks_core_for_something() {
        let (_, studio) = built();
        assert_eq!(studio.presets.len(), somnium_asset::material::MATERIAL_PRESETS.len());
        for (i, shape) in studio.shapes.iter().enumerate() {
            assert!(matches!(
                studio.event(&from(*shape, ButtonMessage::Click)),
                Some(EditorEvent::MaterialStudio(MaterialStudioEvent::Shape(s))) if usize::from(s) == i
            ));
        }
        // The turntable button flips what core said last.
        assert!(matches!(
            studio.event(&from(studio.spin, ButtonMessage::Click)),
            Some(EditorEvent::MaterialStudio(MaterialStudioEvent::Spin(false)))
        ));
        for (chip, index) in &studio.presets {
            assert!(matches!(
                studio.event(&from(*chip, ButtonMessage::Click)),
                Some(EditorEvent::MaterialStudio(MaterialStudioEvent::Preset(i))) if i == *index
            ));
        }
        assert!(studio.event(&from(NodeHandle::NONE, ButtonMessage::Click)).is_none());
    }

    #[test]
    fn dragging_the_preview_turns_it() {
        let (mut ui, studio) = built();
        let bounds = ui.screen_bounds(studio.preview);
        assert!(bounds.w > MIN_SIDE && bounds.h >= MIN_SIDE, "the preview fills the panel: {bounds:?}");
        let start = Vec2::new(bounds.x + bounds.w * 0.5, bounds.y + bounds.h * 0.5);
        // As the pointer delivers them: to the pressed (captured) widget.
        let to_preview = |data: WidgetMessage| UiMessage::new(studio.preview, MessageDirection::ToWidget, data);
        ui.send(to_preview(WidgetMessage::MouseDown {
            pos: start,
            button: crate::message::MouseButton::Left,
            mods: Default::default(),
        }));
        for step in 1..=5 {
            ui.send(to_preview(WidgetMessage::MouseMove { pos: start + Vec2::new(10.0 * step as f32, 0.0), mods: Default::default() }));
        }
        ui.send(to_preview(WidgetMessage::MouseUp { pos: start, button: crate::message::MouseButton::Left, mods: Default::default() }));
        // A move after release turns nothing.
        ui.send(to_preview(WidgetMessage::MouseMove { pos: start + Vec2::new(90.0, 0.0), mods: Default::default() }));
        let turned: f32 = ui
            .update()
            .iter()
            .filter_map(|message| match studio.event(message) {
                Some(EditorEvent::MaterialStudio(MaterialStudioEvent::Turn(radians))) => Some(radians),
                _ => None,
            })
            .sum();
        assert!((turned - 50.0 * TURN_PER_PIXEL).abs() < 1.0e-4, "turned {turned}");
    }

    #[test]
    fn the_preview_draws_the_renderers_picture_only_when_live() {
        let (mut ui, mut studio) = built();
        // The texture slot a drawn quad samples, as `with_texture_layer` packs it.
        let drawn_from = |ui: &mut UserInterface, studio: &MaterialStudio| {
            ui.update();
            ui.perform_layout();
            ui.draw_ctx.instances.clear();
            let node = ui.nodes.borrow(studio.preview.transmute());
            node.control.draw(&node.widget, &mut ui.draw_ctx);
            assert_eq!(ui.draw_ctx.instances.len(), 1, "one quad");
            ui.draw_ctx.instances[0].flags
        };
        let slot_flags = |slot| crate::primitive::Primitive::fill(Rect::new(0.0, 0.0, 1.0, 1.0), [0; 4]).with_texture_layer(slot).flags;
        studio.slot = Some(9);
        studio.show(&mut ui, MaterialPreviewState { live: false, ..Default::default() });
        let fallback = drawn_from(&mut ui, &studio);
        studio.show(&mut ui, MaterialPreviewState { live: true, ..Default::default() });
        let live = drawn_from(&mut ui, &studio);
        assert_eq!(live & slot_flags(9), slot_flags(9), "live, the renderer's texture slot is sampled");
        assert_ne!(fallback, live, "not live, the stand-in is drawn instead");
        // The picture holds sRGB bytes; the UI decodes them like its own colours, and says so to the shader.
        assert_ne!(live & crate::primitive::FLAG_SRGB_TEXELS, 0);
        assert!(
            include_str!("../ui_pass.wgsl")
                .contains(&format!("const FLAG_SRGB_TEXELS: u32 = {}u;", crate::primitive::FLAG_SRGB_TEXELS)),
            "the shader's flag is the primitive's"
        );
    }

    /// The real Material panel, generated from the schema as core generates it, brings the studio; the tool panel's
    /// event hook (the one the editor already calls) routes its clicks; a rebuild without a material drops it.
    #[test]
    fn a_material_panel_brings_the_studio_and_the_tool_panel_routes_it() {
        let schema = somnium_asset::material::material_asset_schema();
        let mut world = somnium_ecs::World::new();
        let entity = world.spawn((somnium_asset::material::MaterialAsset::default(),));
        let values = (schema.snapshot)(&world, entity).unwrap();
        let mut panel = crate::editor::inspector_gen::generate_component_panel(
            &schema,
            &values,
            &crate::editor::property_editors::PropertyEditorRegistry::standard(),
            &crate::editor::editing_rules::EditingRulesRegistry::default(),
        );
        panel.preview_path = Some(PathBuf::from("stone.sommat"));
        let mut ui = UserInterface::new(420.0, 1600.0);
        let root = ui.root();
        let mut persona = crate::editor::persona::Persona::default();
        persona.tool_panel = crate::editor::tool_context::ToolPanel::build(&mut ui, root, 0);
        crate::editor::inspector::build_generated_details(&mut ui, root, 0, &[panel], &Default::default(), &mut persona);
        let studio = &persona.tool_panel.studio;
        assert!(studio.preview.is_some());
        assert_eq!(studio.presets.len(), somnium_asset::material::MATERIAL_PRESETS.len());
        assert!(matches!(
            persona.tool_panel.event(&from(studio.shapes[1], ButtonMessage::Click)),
            Some(EditorEvent::MaterialStudio(MaterialStudioEvent::Shape(1)))
        ));
        assert!(matches!(
            persona.tool_panel.event(&from(studio.presets[3].0, ButtonMessage::Click)),
            Some(EditorEvent::MaterialStudio(MaterialStudioEvent::Preset(3)))
        ));
        crate::editor::inspector::build_generated_details(&mut ui, root, 0, &[], &Default::default(), &mut persona);
        assert!(persona.tool_panel.studio.preview.is_none(), "stale handles are dropped with their tree");
    }

    #[test]
    fn a_cleared_studio_keeps_its_state_and_slot() {
        let (_, mut studio) = built();
        studio.state.shape = 2;
        studio.slot = Some(4);
        studio.clear();
        assert!(studio.preview.is_none() && studio.presets.is_empty());
        assert_eq!((studio.state.shape, studio.slot), (2, Some(4)));
    }
}
