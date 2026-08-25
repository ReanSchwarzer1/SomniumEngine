//! Retained-mode control that draws and edits the shared graph surface.

use glam::Vec2;

use super::{Graph, GraphSurface, NodeId, PinRef, geometry};
use crate::{
    draw::DrawingContext,
    message::{MessageDirection, MouseButton, NodeHandle, UiMessage, WidgetMessage},
    node::{Control, CursorKind, LayoutCtx, UiNode},
    path::{Path, Stroke},
    primitive::Primitive,
    shaped::ShapedInstance,
    types::Rect,
    widget::{Widget, WidgetBuilder},
};

const PIN_GRAB: f32 = 9.0;
const GRID_STEP: f32 = 32.0;

/// Messages understood or emitted by [`GraphEditor`].
#[derive(Clone)]
pub enum GraphEditorMessage {
    /// Replace the authored graph without echoing a change.
    SetGraph(Graph),
    /// Route one CONTROL-A2 command id to this document.
    Command { id: String, paste_offset: Vec2 },
    /// The user committed a graph mutation.
    Changed(Graph),
}

impl GraphEditorMessage {
    /// Replace a widget's graph.
    #[must_use]
    pub fn set_graph(destination: NodeHandle, graph: Graph) -> UiMessage {
        UiMessage::new(
            destination,
            MessageDirection::ToWidget,
            Self::SetGraph(graph),
        )
    }

    /// Dispatch a registered editor command to a graph widget.
    #[must_use]
    pub fn command(
        destination: NodeHandle,
        id: impl Into<String>,
        paste_offset: Vec2,
    ) -> UiMessage {
        UiMessage::new(
            destination,
            MessageDirection::ToWidget,
            Self::Command {
                id: id.into(),
                paste_offset,
            },
        )
    }
}

enum Gesture {
    None,
    Pan { last: Vec2 },
    Move { last: Vec2, before: Graph },
    Box { start: Vec2 },
    Wire { from: PinRef },
}

/// The concrete retained-mode control for MORROWIND-K's shared surface.
pub struct GraphEditor {
    surface: GraphSurface,
    gesture: Gesture,
    font_id: u8,
}

impl GraphEditor {
    fn local(widget: &Widget, point: Vec2) -> Vec2 {
        point - widget.screen_bounds().pos()
    }

    fn graph_point(&self, widget: &Widget, point: Vec2) -> Vec2 {
        self.surface
            .view
            .screen_to_graph(Self::local(widget, point))
    }

    fn screen_point(&self, widget: &Widget, point: Vec2) -> Vec2 {
        widget.screen_bounds().pos() + self.surface.view.graph_to_screen(point)
    }

    fn node_at(&self, widget: &Widget, point: Vec2) -> Option<NodeId> {
        let graph_point = self.graph_point(widget, point);
        geometry::layout_nodes(&self.surface.graph, &self.surface.catalogue)
            .into_iter()
            .rev()
            .find(|layout| layout.bounds.contains(graph_point))
            .map(|layout| layout.node)
    }

    fn pin_at(&self, widget: &Widget, point: Vec2) -> Option<PinRef> {
        geometry::layout_nodes(&self.surface.graph, &self.surface.catalogue)
            .into_iter()
            .flat_map(|layout| layout.pins)
            .map(|pin| {
                (
                    pin.pin,
                    self.screen_point(widget, pin.position).distance(point),
                )
            })
            .filter(|(_, distance)| *distance <= PIN_GRAB)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(pin, _)| pin)
    }

    fn emit_changed(&self, widget: &Widget, emit: &mut Vec<UiMessage>) {
        emit.push(UiMessage::new(
            widget.handle,
            MessageDirection::FromWidget,
            GraphEditorMessage::Changed(self.surface.graph.clone()),
        ));
    }

    fn draw_grid(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let bounds = widget.screen_bounds();
        let step = GRID_STEP * self.surface.view.zoom;
        if step < 4.0 {
            return;
        }
        let colour = crate::theme::active().semantic.border.subtle.bytes();
        let mut x = bounds.x + self.surface.view.pan.x.rem_euclid(step);
        while x < bounds.x + bounds.w {
            ctx.push_rect_filled(Rect::new(x, bounds.y, 1.0, bounds.h), colour);
            x += step;
        }
        let mut y = bounds.y + self.surface.view.pan.y.rem_euclid(step);
        while y < bounds.y + bounds.h {
            ctx.push_rect_filled(Rect::new(bounds.x, y, bounds.w, 1.0), colour);
            y += step;
        }
    }
}

impl Control for GraphEditor {
    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        Vec2::new(available.x.max(320.0), available.y.max(200.0))
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let theme = crate::theme::active();
        let bounds = widget.screen_bounds();
        ctx.push_rect_filled(bounds, theme.semantic.surface.canvas.bytes());
        self.draw_grid(widget, ctx);

        let layouts = geometry::layout_nodes(&self.surface.graph, &self.surface.catalogue);
        for connection in self.surface.graph.connections() {
            let Some(from) = layouts
                .iter()
                .find_map(|layout| layout.pin(connection.from))
            else {
                continue;
            };
            let Some(to) = layouts.iter().find_map(|layout| layout.pin(connection.to)) else {
                continue;
            };
            let wire = Path::wire(
                self.screen_point(widget, from.position),
                self.screen_point(widget, to.position),
            );
            ctx.push_stroke(
                &wire,
                &Stroke::new(2.0),
                ShapedInstance::identity(theme.semantic.accent.default.bytes()),
            );
        }

        for layout in layouts {
            let Some(node) = self.surface.graph.node(layout.node) else {
                continue;
            };
            let Some(archetype) = self.surface.catalogue.get(&node.archetype) else {
                continue;
            };
            let top_left = self.screen_point(widget, layout.bounds.pos());
            let size = layout.bounds.size() * self.surface.view.zoom;
            let node_bounds = Rect::from_pos_size(top_left, size);
            let selected = self.surface.selection.contains(node.id);
            let fill = if node.archetype == "graph.comment" || node.archetype == "graph.group" {
                [
                    theme.semantic.accent.default.bytes()[0],
                    theme.semantic.accent.default.bytes()[1],
                    theme.semantic.accent.default.bytes()[2],
                    34,
                ]
            } else {
                theme.semantic.surface.raised.bytes()
            };
            let border = if selected {
                theme.semantic.accent.default.bytes()
            } else {
                theme.semantic.border.default.bytes()
            };
            ctx.push_primitive(
                Primitive::fill(node_bounds, fill)
                    .with_radius(theme.geometry.radius_tile)
                    .with_border(if selected { 2.0 } else { 1.0 }, border),
                None,
            );
            if !archetype.is_reroute {
                let header_height = 28.0 * self.surface.view.zoom;
                ctx.push_rect_filled(
                    Rect::new(node_bounds.x, node_bounds.y, node_bounds.w, header_height),
                    theme.semantic.surface.header.bytes(),
                );
                let title = if node.title.is_empty() {
                    archetype.title
                } else {
                    &node.title
                };
                ctx.push_text(
                    title,
                    Vec2::new(node_bounds.x + 8.0, node_bounds.y + 7.0),
                    self.font_id,
                    11.0,
                    theme.semantic.text.primary.bytes(),
                );
            }
            for pin in layout.pins {
                let centre = self.screen_point(widget, pin.position);
                let circle = Path::circle(centre, 4.0);
                ctx.push_path(&circle, ShapedInstance::identity(pin_colour(pin.ty)));
            }
        }
    }

    fn cursor_icon(&self, _widget: &Widget, _pos: Vec2) -> CursorKind {
        match self.gesture {
            Gesture::Pan { .. } | Gesture::Move { .. } => CursorKind::Move,
            _ => CursorKind::Default,
        }
    }

    fn gesture_active(&self) -> bool {
        !matches!(self.gesture, Gesture::None)
    }

    fn cancel_gesture(&mut self, _widget: &mut Widget, _emit: &mut Vec<UiMessage>) -> bool {
        if let Gesture::Move { before, .. } = &self.gesture {
            self.surface.graph.clone_from(before);
        }
        let changed = !matches!(self.gesture, Gesture::None);
        self.gesture = Gesture::None;
        changed
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if let Some(message) = msg.data::<GraphEditorMessage>().cloned() {
            match message {
                GraphEditorMessage::SetGraph(graph) => self.surface.graph = graph,
                GraphEditorMessage::Command { id, paste_offset } => {
                    if self.surface.dispatch_command(&id, paste_offset) {
                        self.emit_changed(widget, emit);
                    }
                }
                GraphEditorMessage::Changed(_) => {}
            }
            return;
        }

        let Some(message) = msg.data::<WidgetMessage>() else {
            return;
        };
        match message {
            WidgetMessage::MouseWheel { pos, delta, .. } => {
                let local = Self::local(widget, *pos);
                self.surface.view.zoom_at(local, 1.1_f32.powf(*delta));
                msg.handled = true;
            }
            WidgetMessage::MouseDown {
                pos,
                button: MouseButton::Middle,
                ..
            } => {
                self.gesture = Gesture::Pan { last: *pos };
                msg.handled = true;
            }
            WidgetMessage::MouseDown {
                pos,
                button: MouseButton::Left,
                mods,
            } => {
                if let Some(pin) = self.pin_at(widget, *pos) {
                    self.gesture = Gesture::Wire { from: pin };
                } else if let Some(node) = self.node_at(widget, *pos) {
                    if mods.command() {
                        self.surface.selection.toggle(node);
                    } else if !self.surface.selection.contains(node) {
                        self.surface.selection.select_only(node);
                    }
                    self.gesture = Gesture::Move {
                        last: *pos,
                        before: self.surface.graph.clone(),
                    };
                } else {
                    self.surface.selection.clear();
                    self.gesture = Gesture::Box {
                        start: self.graph_point(widget, *pos),
                    };
                }
                msg.handled = true;
            }
            WidgetMessage::MouseMove { pos, .. } => match &mut self.gesture {
                Gesture::Pan { last } => {
                    self.surface.view.pan_by(*pos - *last);
                    *last = *pos;
                    msg.handled = true;
                }
                Gesture::Move { last, .. } => {
                    let delta = (*pos - *last) / self.surface.view.zoom;
                    self.surface
                        .graph
                        .translate(&self.surface.selection.ids(), delta);
                    *last = *pos;
                    msg.handled = true;
                }
                Gesture::Box { start } => {
                    let end = self.surface.view.screen_to_graph(Self::local(widget, *pos));
                    let layouts =
                        geometry::layout_nodes(&self.surface.graph, &self.surface.catalogue);
                    self.surface.selection.select_box(
                        &layouts,
                        Rect::new(start.x, start.y, end.x - start.x, end.y - start.y),
                    );
                    msg.handled = true;
                }
                Gesture::None | Gesture::Wire { .. } => {}
            },
            WidgetMessage::MouseUp {
                pos,
                button: MouseButton::Left,
                ..
            } => {
                let gesture = std::mem::replace(&mut self.gesture, Gesture::None);
                match gesture {
                    Gesture::Move { before, .. } => {
                        if self.surface.commit_gesture(before, "Move Graph Nodes") {
                            self.emit_changed(widget, emit);
                        }
                    }
                    Gesture::Wire { from } => {
                        if let Some(to) = self.pin_at(widget, *pos) {
                            let changed = self.surface.connect(from, to).is_ok()
                                || self.surface.reconnect(from, to).is_ok();
                            if changed {
                                self.emit_changed(widget, emit);
                            }
                        }
                    }
                    Gesture::None | Gesture::Pan { .. } | Gesture::Box { .. } => {}
                }
                msg.handled = true;
            }
            WidgetMessage::MouseUp {
                button: MouseButton::Middle,
                ..
            } => {
                if matches!(self.gesture, Gesture::Pan { .. }) {
                    self.gesture = Gesture::None;
                    msg.handled = true;
                }
            }
            _ => {}
        }
    }
}

fn pin_colour(ty: super::PinType) -> [u8; 4] {
    match ty {
        super::PinType::Bool => [218, 92, 92, 255],
        super::PinType::Int | super::PinType::Float => [102, 196, 129, 255],
        super::PinType::Vec2 | super::PinType::Vec3 | super::PinType::Vec4 => [86, 171, 221, 255],
        super::PinType::Color => [218, 154, 74, 255],
        super::PinType::Texture => [170, 111, 210, 255],
        super::PinType::Flow => [230, 230, 230, 255],
        super::PinType::Opaque(_) => [118, 188, 183, 255],
    }
}

/// Builder for the shared retained-mode graph control.
pub struct GraphEditorBuilder {
    widget: WidgetBuilder,
    surface: GraphSurface,
    font_id: u8,
}

impl GraphEditorBuilder {
    /// Create a graph control for one feature catalogue.
    #[must_use]
    pub fn new(widget: WidgetBuilder, catalogue: super::Catalogue) -> Self {
        Self {
            widget,
            surface: GraphSurface::new(catalogue),
            font_id: 0,
        }
    }

    /// Seed the control with an authored graph.
    #[must_use]
    pub fn with_graph(mut self, graph: Graph) -> Self {
        self.surface.graph = graph;
        self
    }

    /// Font atlas slot used for node titles.
    #[must_use]
    pub fn with_font(mut self, font_id: u8) -> Self {
        self.font_id = font_id;
        self
    }

    /// Build the UI node.
    #[must_use]
    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(GraphEditor {
                surface: self.surface,
                gesture: Gesture::None,
                font_id: self.font_id,
            }),
        )
    }
}
