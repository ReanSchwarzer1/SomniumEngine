//! Retained-mode control that draws and edits the shared graph surface.

use glam::Vec2;

use super::{
    AnimationStateMachineDocument, AuthoredStateTransition, Graph, GraphSurface,
    NodeElementArchetype, NodeId, PinRef, geometry,
};
use crate::{
    draw::DrawingContext,
    message::{
        MessageDirection, MouseButton, NodeHandle, UiMessage, WHEEL_DELTA_PER_LINE, WidgetMessage,
    },
    node::{Control, CursorKind, LayoutCtx, UiNode},
    path::{Path, Stroke},
    primitive::Primitive,
    shaped::ShapedInstance,
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use somnium_anim::{CompareOp, Condition};

const PIN_GRAB: f32 = 9.0;
const GRID_STEP: f32 = 32.0;
const NODE_HEADER: f32 = 28.0;
const PIN_ROW: f32 = 24.0;
const NODE_PADDING: f32 = 8.0;
const LITERAL_LEFT: f32 = 78.0;
const LITERAL_RIGHT: f32 = 8.0;
const TRANSITION_LABEL_WIDTH: f32 = 176.0;
const TRANSITION_LABEL_HEIGHT: f32 = 20.0;
const TRANSITION_PANEL_WIDTH: f32 = 292.0;
const TRANSITION_PANEL_HEIGHT: f32 = 190.0;
/// A single wheel line changes graph scale by ten percent. Routed wheel
/// messages are in logical pixels, so pixel-delta trackpads contribute
/// fractions of this exponent instead of jumping a whole notch per pixel.
const ZOOM_PER_WHEEL_LINE: f32 = 1.1;

fn wheel_zoom_factor(delta: f32) -> Option<f32> {
    if !delta.is_finite() {
        return None;
    }
    // Keep the factor finite even for synthetic or broken-device deltas. This
    // span can reach either GraphView bound from the other in one event; the
    // view itself remains the single authority that clamps the final zoom.
    let max_exponent = (super::GraphView::MAX_ZOOM / super::GraphView::MIN_ZOOM).ln();
    let exponent = (delta / WHEEL_DELTA_PER_LINE) * ZOOM_PER_WHEEL_LINE.ln();
    Some(exponent.clamp(-max_exponent, max_exponent).exp())
}

/// Messages understood or emitted by [`GraphEditor`].
#[derive(Clone)]
pub enum GraphEditorMessage {
    /// Replace the authored graph without echoing a change.
    SetGraph(Graph),
    /// Route one CONTROL-A2 command id to this document.
    Command { id: String, paste_offset: Vec2 },
    /// Replace the complete animation state-machine document.
    SetStateMachineDocument(AnimationStateMachineDocument),
    /// Commit one visible literal field through graph-local undo history.
    SetLiteral {
        node: NodeId,
        pin: u16,
        value: String,
    },
    /// Mark a visible animation-state node as the machine entry state.
    SetInitialState(NodeId),
    /// Add one cyclic transition overlay edge.
    AddStateTransition(AuthoredStateTransition),
    /// Replace one transition, including conditions, blend time and sync track.
    SetStateTransition {
        index: usize,
        transition: AuthoredStateTransition,
    },
    /// Remove one transition overlay edge.
    RemoveStateTransition(usize),
    /// Undo one state-overlay edit without consuming graph history.
    UndoStateOverlay,
    /// Redo one state-overlay edit without consuming graph history.
    RedoStateOverlay,
    /// The user committed a graph mutation.
    Changed(Graph),
    /// The user committed a graph or overlay mutation in state-machine mode.
    StateMachineChanged(AnimationStateMachineDocument),
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

    /// Edit a node-body literal through the same routed control path as pointer
    /// and keyboard authoring.
    #[must_use]
    pub fn set_literal(
        destination: NodeHandle,
        node: NodeId,
        pin: u16,
        value: impl Into<String>,
    ) -> UiMessage {
        UiMessage::new(
            destination,
            MessageDirection::ToWidget,
            Self::SetLiteral {
                node,
                pin,
                value: value.into(),
            },
        )
    }

    /// Replace the graph control with an animation state-machine document.
    #[must_use]
    pub fn set_state_machine_document(
        destination: NodeHandle,
        document: AnimationStateMachineDocument,
    ) -> UiMessage {
        UiMessage::new(
            destination,
            MessageDirection::ToWidget,
            Self::SetStateMachineDocument(document),
        )
    }

    /// Route one initial-state authoring action.
    #[must_use]
    pub fn set_initial_state(destination: NodeHandle, state: NodeId) -> UiMessage {
        UiMessage::new(
            destination,
            MessageDirection::ToWidget,
            Self::SetInitialState(state),
        )
    }

    /// Route one transition creation action.
    #[must_use]
    pub fn add_state_transition(
        destination: NodeHandle,
        transition: AuthoredStateTransition,
    ) -> UiMessage {
        UiMessage::new(
            destination,
            MessageDirection::ToWidget,
            Self::AddStateTransition(transition),
        )
    }

    /// Route one transition replacement action.
    #[must_use]
    pub fn set_state_transition(
        destination: NodeHandle,
        index: usize,
        transition: AuthoredStateTransition,
    ) -> UiMessage {
        UiMessage::new(
            destination,
            MessageDirection::ToWidget,
            Self::SetStateTransition { index, transition },
        )
    }

    /// Route one transition deletion action.
    #[must_use]
    pub fn remove_state_transition(destination: NodeHandle, index: usize) -> UiMessage {
        UiMessage::new(
            destination,
            MessageDirection::ToWidget,
            Self::RemoveStateTransition(index),
        )
    }

    /// Route state-overlay undo independently from pose-graph undo.
    #[must_use]
    pub fn undo_state_overlay(destination: NodeHandle) -> UiMessage {
        UiMessage::new(
            destination,
            MessageDirection::ToWidget,
            Self::UndoStateOverlay,
        )
    }

    /// Route state-overlay redo independently from pose-graph redo.
    #[must_use]
    pub fn redo_state_overlay(destination: NodeHandle) -> UiMessage {
        UiMessage::new(
            destination,
            MessageDirection::ToWidget,
            Self::RedoStateOverlay,
        )
    }
}

enum Gesture {
    None,
    Pan { last: Vec2 },
    Move { last: Vec2, before: Graph },
    Box { start: Vec2 },
    Wire { from: PinRef },
    StateTransition { from: NodeId },
}

#[derive(Clone)]
enum EditorDocument {
    Graph(GraphSurface),
    StateMachine(AnimationStateMachineDocument),
}

impl EditorDocument {
    fn surface(&self) -> &GraphSurface {
        match self {
            Self::Graph(surface) => surface,
            Self::StateMachine(document) => document.surface(),
        }
    }

    fn surface_mut(&mut self) -> &mut GraphSurface {
        match self {
            Self::Graph(surface) => surface,
            Self::StateMachine(document) => document.surface_mut(),
        }
    }

    fn state_machine(&self) -> Option<&AnimationStateMachineDocument> {
        match self {
            Self::Graph(_) => None,
            Self::StateMachine(document) => Some(document),
        }
    }

    fn state_machine_mut(&mut self) -> Option<&mut AnimationStateMachineDocument> {
        match self {
            Self::Graph(_) => None,
            Self::StateMachine(document) => Some(document),
        }
    }
}

struct LiteralEdit {
    node: NodeId,
    pin: u16,
    draft: String,
    replace_on_input: bool,
}

#[derive(Clone, Copy)]
enum TransitionField {
    BlendSeconds,
    SyncTrack,
    Conditions,
}

struct TransitionEdit {
    index: usize,
    field: TransitionField,
    draft: String,
    replace_on_input: bool,
}

/// The concrete retained-mode control for MORROWIND-K's shared surface.
pub struct GraphEditor {
    document: EditorDocument,
    gesture: Gesture,
    literal_edit: Option<LiteralEdit>,
    selected_transition: Option<usize>,
    transition_edit: Option<TransitionEdit>,
    transition_error: Option<String>,
    font_id: u8,
}

impl GraphEditor {
    fn surface(&self) -> &GraphSurface {
        self.document.surface()
    }

    fn surface_mut(&mut self) -> &mut GraphSurface {
        self.document.surface_mut()
    }

    fn local(widget: &Widget, point: Vec2) -> Vec2 {
        point - widget.screen_bounds().pos()
    }

    fn graph_point(&self, widget: &Widget, point: Vec2) -> Vec2 {
        self.surface()
            .view
            .screen_to_graph(Self::local(widget, point))
    }

    fn screen_point(&self, widget: &Widget, point: Vec2) -> Vec2 {
        widget.screen_bounds().pos() + self.surface().view.graph_to_screen(point)
    }

    fn node_at(&self, widget: &Widget, point: Vec2) -> Option<NodeId> {
        let graph_point = self.graph_point(widget, point);
        geometry::layout_nodes(&self.surface().graph, &self.surface().catalogue)
            .into_iter()
            .rev()
            .find(|layout| layout.bounds.contains(graph_point))
            .map(|layout| layout.node)
    }

    fn pin_at(&self, widget: &Widget, point: Vec2) -> Option<PinRef> {
        geometry::layout_nodes(&self.surface().graph, &self.surface().catalogue)
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
        let message = self.document.state_machine().map_or_else(
            || GraphEditorMessage::Changed(self.surface().graph.clone()),
            |document| GraphEditorMessage::StateMachineChanged(document.clone()),
        );
        emit.push(UiMessage::new(
            widget.handle,
            MessageDirection::FromWidget,
            message,
        ));
    }

    fn is_state(&self, node: NodeId) -> bool {
        self.surface()
            .graph
            .node(node)
            .is_some_and(|node| node.archetype == "animation.state")
    }

    fn literal_visible(&self, node: NodeId, pin: u16) -> bool {
        let surface = self.surface();
        let Some(node) = surface.graph.node(node) else {
            return false;
        };
        let Some(archetype) = surface.catalogue.get(&node.archetype) else {
            return false;
        };
        if surface
            .graph
            .input_source(PinRef::input(node.id, pin))
            .is_some()
        {
            return false;
        }
        archetype
            .elements
            .iter()
            .any(|element| matches!(element, NodeElementArchetype::Literal(index) if *index == pin))
            || (archetype.elements.is_empty()
                && archetype
                    .inputs
                    .get(pin as usize)
                    .is_some_and(|input| input.default.is_some()))
    }

    fn literal_bounds(layout: &geometry::NodeLayout, pin: u16) -> Rect {
        Rect::new(
            layout.bounds.x + LITERAL_LEFT,
            layout.bounds.y + NODE_HEADER + NODE_PADDING + f32::from(pin) * PIN_ROW + 2.0,
            (layout.bounds.w - LITERAL_LEFT - LITERAL_RIGHT).max(20.0),
            PIN_ROW - 4.0,
        )
    }

    fn screen_rect(&self, widget: &Widget, rect: Rect) -> Rect {
        Rect::from_pos_size(
            self.screen_point(widget, rect.pos()),
            rect.size() * self.surface().view.zoom,
        )
    }

    fn literal_at(&self, widget: &Widget, point: Vec2) -> Option<(NodeId, u16, String)> {
        let graph_point = self.graph_point(widget, point);
        let surface = self.surface();
        geometry::layout_nodes(&surface.graph, &surface.catalogue)
            .into_iter()
            .rev()
            .find_map(|layout| {
                let node = surface.graph.node(layout.node)?;
                let archetype = surface.catalogue.get(&node.archetype)?;
                archetype
                    .inputs
                    .iter()
                    .enumerate()
                    .find_map(|(pin, input)| {
                        let pin = pin as u16;
                        if !self.literal_visible(node.id, pin)
                            || !Self::literal_bounds(&layout, pin).contains(graph_point)
                        {
                            return None;
                        }
                        Some((
                            node.id,
                            pin,
                            node.literals
                                .get(&pin)
                                .cloned()
                                .or_else(|| input.default.map(str::to_owned))
                                .unwrap_or_default(),
                        ))
                    })
            })
    }

    fn commit_literal(&mut self, widget: &Widget, emit: &mut Vec<UiMessage>) -> bool {
        let Some(edit) = self.literal_edit.take() else {
            return false;
        };
        let changed = self
            .surface_mut()
            .set_literal(edit.node, edit.pin, edit.draft);
        if changed {
            self.emit_changed(widget, emit);
        }
        changed
    }

    fn transition_label_rect(&self, widget: &Widget, index: usize) -> Option<Rect> {
        let document = self.document.state_machine()?;
        let transition = document.transitions().get(index)?;
        let layouts =
            geometry::layout_nodes(&document.surface().graph, &document.surface().catalogue);
        let from = layouts
            .iter()
            .find(|layout| layout.node == transition.from)?;
        let to = layouts.iter().find(|layout| layout.node == transition.to)?;
        let from = self.screen_point(
            widget,
            from.bounds.pos() + Vec2::new(from.bounds.w, from.bounds.h * 0.5),
        );
        let to = self.screen_point(widget, to.bounds.pos() + Vec2::new(0.0, to.bounds.h * 0.5));
        let middle = (from + to) * 0.5;
        Some(Rect::new(
            middle.x - TRANSITION_LABEL_WIDTH * 0.5,
            middle.y - TRANSITION_LABEL_HEIGHT * 0.5,
            TRANSITION_LABEL_WIDTH,
            TRANSITION_LABEL_HEIGHT,
        ))
    }

    fn transition_at(&self, widget: &Widget, point: Vec2) -> Option<usize> {
        let count = self.document.state_machine()?.transitions().len();
        (0..count).rev().find(|index| {
            self.transition_label_rect(widget, *index)
                .is_some_and(|rect| rect.contains(point))
        })
    }

    fn transition_panel(widget: &Widget) -> Rect {
        let bounds = widget.screen_bounds();
        Rect::new(
            bounds.x + bounds.w - TRANSITION_PANEL_WIDTH - 16.0,
            bounds.y + 16.0,
            TRANSITION_PANEL_WIDTH,
            TRANSITION_PANEL_HEIGHT,
        )
    }

    fn transition_field_rect(widget: &Widget, field: TransitionField) -> Rect {
        let panel = Self::transition_panel(widget);
        let y = match field {
            TransitionField::BlendSeconds => panel.y + 52.0,
            TransitionField::SyncTrack => panel.y + 82.0,
            TransitionField::Conditions => panel.y + 112.0,
        };
        Rect::new(panel.x + 98.0, y, panel.w - 110.0, 24.0)
    }

    fn transition_delete_rect(widget: &Widget) -> Rect {
        let panel = Self::transition_panel(widget);
        Rect::new(
            panel.x + panel.w - 82.0,
            panel.y + panel.h - 30.0,
            70.0,
            22.0,
        )
    }

    fn transition_field_at(&self, widget: &Widget, point: Vec2) -> Option<TransitionField> {
        self.selected_transition?;
        [
            TransitionField::BlendSeconds,
            TransitionField::SyncTrack,
            TransitionField::Conditions,
        ]
        .into_iter()
        .find(|field| Self::transition_field_rect(widget, *field).contains(point))
    }

    fn begin_transition_edit(&mut self, field: TransitionField) {
        let Some(index) = self.selected_transition else {
            return;
        };
        let Some(transition) = self
            .document
            .state_machine()
            .and_then(|document| document.transitions().get(index))
        else {
            return;
        };
        let draft = match field {
            TransitionField::BlendSeconds => transition.blend_seconds.to_string(),
            TransitionField::SyncTrack => transition.sync_track.clone().unwrap_or_default(),
            TransitionField::Conditions => format_conditions(&transition.conditions),
        };
        self.transition_edit = Some(TransitionEdit {
            index,
            field,
            draft,
            replace_on_input: true,
        });
        self.transition_error = None;
    }

    fn commit_transition_edit(&mut self, widget: &Widget, emit: &mut Vec<UiMessage>) -> bool {
        let Some(edit) = self.transition_edit.take() else {
            return false;
        };
        let Some(mut transition) = self
            .document
            .state_machine()
            .and_then(|document| document.transitions().get(edit.index))
            .cloned()
        else {
            return false;
        };
        let valid = match edit.field {
            TransitionField::BlendSeconds => edit
                .draft
                .trim()
                .parse::<f32>()
                .ok()
                .filter(|value| value.is_finite() && *value >= 0.0)
                .map(|value| transition.blend_seconds = value)
                .is_some(),
            TransitionField::SyncTrack => {
                let value = edit.draft.trim();
                transition.sync_track = (!value.is_empty()).then(|| value.to_owned());
                true
            }
            TransitionField::Conditions => parse_conditions(&edit.draft)
                .map(|conditions| transition.conditions = conditions)
                .is_some(),
        };
        if !valid {
            self.transition_error = Some(
                match edit.field {
                    TransitionField::BlendSeconds => {
                        "Blend Time must be a finite value ≥ 0 seconds"
                    }
                    TransitionField::SyncTrack => "Sync Track is a name or blank",
                    TransitionField::Conditions => {
                        "Conditions: trigger:name; bool:name:true; float:name:greater:0.5"
                    }
                }
                .into(),
            );
            self.transition_edit = Some(edit);
            return false;
        }
        let changed = self
            .document
            .state_machine_mut()
            .is_some_and(|document| document.set_transition(edit.index, transition));
        if changed {
            self.transition_error = None;
            self.emit_changed(widget, emit);
        }
        changed
    }

    fn draw_transition_inspector(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let Some(index) = self.selected_transition else {
            return;
        };
        let Some(transition) = self
            .document
            .state_machine()
            .and_then(|document| document.transitions().get(index))
        else {
            return;
        };
        let theme = crate::theme::active();
        let panel = Self::transition_panel(widget);
        ctx.push_primitive(
            Primitive::fill(panel, theme.semantic.surface.panel.bytes())
                .with_radius(theme.geometry.radius_tile)
                .with_border(1.0, theme.semantic.border.default.bytes()),
            None,
        );
        ctx.push_text(
            &format!("Transition {} → {}", transition.from.0, transition.to.0),
            Vec2::new(panel.x + 10.0, panel.y + 10.0),
            self.font_id,
            11.0,
            theme.semantic.text.primary.bytes(),
        );
        for (field, label, value) in [
            (
                TransitionField::BlendSeconds,
                "Blend Time",
                format!("{} s", transition.blend_seconds),
            ),
            (
                TransitionField::SyncTrack,
                "Sync Track",
                transition.sync_track.clone().unwrap_or_default(),
            ),
            (
                TransitionField::Conditions,
                "Conditions",
                format_conditions(&transition.conditions),
            ),
        ] {
            let field_rect = Self::transition_field_rect(widget, field);
            ctx.push_text(
                label,
                Vec2::new(panel.x + 10.0, field_rect.y + 5.0),
                self.font_id,
                9.0,
                theme.semantic.text.secondary.bytes(),
            );
            let editing = self.transition_edit.as_ref().is_some_and(|edit| {
                edit.index == index
                    && std::mem::discriminant(&edit.field) == std::mem::discriminant(&field)
            });
            ctx.push_primitive(
                Primitive::fill(field_rect, theme.semantic.surface.input.bytes()).with_border(
                    if editing { 2.0 } else { 1.0 },
                    if editing {
                        theme.semantic.border.focus.bytes()
                    } else {
                        theme.semantic.border.subtle.bytes()
                    },
                ),
                None,
            );
            let text = self
                .transition_edit
                .as_ref()
                .filter(|edit| {
                    edit.index == index
                        && std::mem::discriminant(&edit.field) == std::mem::discriminant(&field)
                })
                .map_or(value.as_str(), |edit| edit.draft.as_str());
            ctx.push_text(
                text,
                Vec2::new(field_rect.x + 4.0, field_rect.y + 5.0),
                self.font_id,
                9.0,
                theme.semantic.text.primary.bytes(),
            );
        }
        let delete = Self::transition_delete_rect(widget);
        ctx.push_primitive(
            Primitive::fill(delete, theme.semantic.status.error.bytes())
                .with_radius(theme.geometry.radius_input),
            None,
        );
        ctx.push_text(
            "Delete",
            Vec2::new(delete.x + 14.0, delete.y + 5.0),
            self.font_id,
            9.0,
            theme.semantic.text.inverse.bytes(),
        );
        ctx.push_text(
            self.transition_error
                .as_deref()
                .unwrap_or("Condition forms: trigger:name; bool:name:true; float:name:greater:0.5"),
            Vec2::new(panel.x + 10.0, panel.y + panel.h - 25.0),
            self.font_id,
            8.0,
            if self.transition_error.is_some() {
                theme.semantic.status.error.bytes()
            } else {
                theme.semantic.text.muted.bytes()
            },
        );
    }

    fn draw_grid(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let bounds = widget.screen_bounds();
        let step = GRID_STEP * self.surface().view.zoom;
        if step < 4.0 {
            return;
        }
        let colour = crate::theme::active().semantic.border.subtle.bytes();
        let mut x = bounds.x + self.surface().view.pan.x.rem_euclid(step);
        while x < bounds.x + bounds.w {
            ctx.push_rect_filled(Rect::new(x, bounds.y, 1.0, bounds.h), colour);
            x += step;
        }
        let mut y = bounds.y + self.surface().view.pan.y.rem_euclid(step);
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

        let surface = self.surface();
        let layouts = geometry::layout_nodes(&surface.graph, &surface.catalogue);
        for connection in surface.graph.connections() {
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

        if let Some(document) = self.document.state_machine() {
            for (index, transition) in document.transitions().iter().enumerate() {
                let Some(from) = layouts.iter().find(|layout| layout.node == transition.from)
                else {
                    continue;
                };
                let Some(to) = layouts.iter().find(|layout| layout.node == transition.to) else {
                    continue;
                };
                let from = from.bounds.pos() + Vec2::new(from.bounds.w, from.bounds.h * 0.5);
                let to = to.bounds.pos() + Vec2::new(0.0, to.bounds.h * 0.5);
                ctx.push_stroke(
                    &Path::wire(
                        self.screen_point(widget, from),
                        self.screen_point(widget, to),
                    ),
                    &Stroke::new(2.0),
                    ShapedInstance::identity(theme.semantic.status.warning.bytes()),
                );
                if let Some(label) = self.transition_label_rect(widget, index) {
                    let selected = self.selected_transition == Some(index);
                    ctx.push_primitive(
                        Primitive::fill(label, theme.semantic.surface.popup.bytes())
                            .with_radius(theme.geometry.radius_input)
                            .with_border(
                                if selected { 2.0 } else { 1.0 },
                                if selected {
                                    theme.semantic.border.focus.bytes()
                                } else {
                                    theme.semantic.status.warning.bytes()
                                },
                            ),
                        None,
                    );
                    let condition_count = transition.conditions.len();
                    ctx.push_text(
                        &format!(
                            "{:.2}s · {condition_count} condition{} · {}",
                            transition.blend_seconds,
                            if condition_count == 1 { "" } else { "s" },
                            transition.sync_track.as_deref().unwrap_or("no sync")
                        ),
                        Vec2::new(label.x + 6.0, label.y + 5.0),
                        self.font_id,
                        8.0,
                        theme.semantic.text.primary.bytes(),
                    );
                }
            }
        }

        for layout in layouts {
            let Some(node) = surface.graph.node(layout.node) else {
                continue;
            };
            let Some(archetype) = surface.catalogue.get(&node.archetype) else {
                continue;
            };
            let top_left = self.screen_point(widget, layout.bounds.pos());
            let size = layout.bounds.size() * surface.view.zoom;
            let node_bounds = Rect::from_pos_size(top_left, size);
            let selected = surface.selection.contains(node.id);
            let initial = self
                .document
                .state_machine()
                .is_some_and(|document| document.initial() == Some(node.id));
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
            let border = if initial {
                theme.semantic.status.success.bytes()
            } else if selected {
                theme.semantic.accent.default.bytes()
            } else {
                theme.semantic.border.default.bytes()
            };
            ctx.push_primitive(
                Primitive::fill(node_bounds, fill)
                    .with_radius(theme.geometry.radius_tile)
                    .with_border(if selected || initial { 2.0 } else { 1.0 }, border),
                None,
            );
            if !archetype.is_reroute {
                let header_height = NODE_HEADER * surface.view.zoom;
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
            for (pin, input) in archetype.inputs.iter().enumerate() {
                let y = node_bounds.y
                    + (NODE_HEADER + NODE_PADDING + pin as f32 * PIN_ROW + 5.0) * surface.view.zoom;
                ctx.push_text(
                    input.name,
                    Vec2::new(node_bounds.x + 8.0 * surface.view.zoom, y),
                    self.font_id,
                    9.0 * surface.view.zoom,
                    theme.semantic.text.secondary.bytes(),
                );
                let pin = pin as u16;
                if self.literal_visible(node.id, pin) {
                    let field = self.screen_rect(widget, Self::literal_bounds(&layout, pin));
                    let editing = self
                        .literal_edit
                        .as_ref()
                        .is_some_and(|edit| edit.node == node.id && edit.pin == pin);
                    ctx.push_primitive(
                        Primitive::fill(field, theme.semantic.surface.input.bytes()).with_border(
                            if editing { 2.0 } else { 1.0 },
                            if editing {
                                theme.semantic.border.focus.bytes()
                            } else {
                                theme.semantic.border.subtle.bytes()
                            },
                        ),
                        None,
                    );
                    let value = self
                        .literal_edit
                        .as_ref()
                        .filter(|edit| edit.node == node.id && edit.pin == pin)
                        .map(|edit| edit.draft.as_str())
                        .or_else(|| node.literals.get(&pin).map(String::as_str))
                        .or(input.default)
                        .unwrap_or_default();
                    let display = input
                        .unit
                        .map_or_else(|| value.to_owned(), |unit| format!("{value} {unit}"));
                    ctx.push_text(
                        &display,
                        Vec2::new(field.x + 4.0, field.y + 4.0),
                        self.font_id,
                        9.0 * surface.view.zoom,
                        theme.semantic.text.primary.bytes(),
                    );
                }
            }
            for (pin, output) in archetype.outputs.iter().enumerate() {
                let y = node_bounds.y
                    + (NODE_HEADER + NODE_PADDING + pin as f32 * PIN_ROW + 5.0) * surface.view.zoom;
                ctx.push_text(
                    output.name,
                    Vec2::new(node_bounds.x + node_bounds.w - 56.0 * surface.view.zoom, y),
                    self.font_id,
                    9.0 * surface.view.zoom,
                    theme.semantic.text.secondary.bytes(),
                );
            }
            for (row, element) in archetype.elements.iter().enumerate() {
                let y = node_bounds.y
                    + (NODE_HEADER + NODE_PADDING + row as f32 * PIN_ROW + 5.0) * surface.view.zoom;
                match element {
                    NodeElementArchetype::Label(label) => ctx.push_text(
                        label,
                        Vec2::new(node_bounds.x + 8.0 * surface.view.zoom, y),
                        self.font_id,
                        8.0 * surface.view.zoom,
                        theme.semantic.text.muted.bytes(),
                    ),
                    NodeElementArchetype::Separator => ctx.push_rect_filled(
                        Rect::new(
                            node_bounds.x + 8.0 * surface.view.zoom,
                            y + 5.0 * surface.view.zoom,
                            node_bounds.w - 16.0 * surface.view.zoom,
                            surface.view.zoom.max(1.0),
                        ),
                        theme.semantic.border.subtle.bytes(),
                    ),
                    NodeElementArchetype::Input(_)
                    | NodeElementArchetype::Output(_)
                    | NodeElementArchetype::Literal(_) => {}
                }
            }
            for pin in layout.pins {
                let centre = self.screen_point(widget, pin.position);
                let circle = Path::circle(centre, 4.0);
                ctx.push_path(&circle, ShapedInstance::identity(pin_colour(pin.ty)));
            }
        }
        self.draw_transition_inspector(widget, ctx);
    }

    fn cursor_icon(&self, widget: &Widget, pos: Vec2) -> CursorKind {
        if self.literal_at(widget, pos).is_some() || self.transition_field_at(widget, pos).is_some()
        {
            return CursorKind::Text;
        }
        if self.transition_at(widget, pos).is_some()
            || (self.selected_transition.is_some()
                && Self::transition_delete_rect(widget).contains(pos))
        {
            return CursorKind::Pointer;
        }
        match self.gesture {
            Gesture::Pan { .. } | Gesture::Move { .. } => CursorKind::Move,
            _ => CursorKind::Default,
        }
    }

    fn is_text_input(&self) -> bool {
        self.literal_edit.is_some() || self.transition_edit.is_some()
    }

    fn is_keyboard_focusable(&self) -> bool {
        true
    }

    fn gesture_active(&self) -> bool {
        !matches!(self.gesture, Gesture::None)
    }

    fn cancel_gesture(&mut self, _widget: &mut Widget, _emit: &mut Vec<UiMessage>) -> bool {
        let restore = match &self.gesture {
            Gesture::Move { before, .. } => Some(before.clone()),
            _ => None,
        };
        if let Some(before) = restore {
            self.surface_mut().graph = before;
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
                GraphEditorMessage::SetGraph(graph) => {
                    self.surface_mut().graph = graph;
                    self.literal_edit = None;
                    self.selected_transition = None;
                    self.transition_edit = None;
                    widget.invalidate_layout();
                }
                GraphEditorMessage::SetStateMachineDocument(document) => {
                    self.document = EditorDocument::StateMachine(document);
                    self.literal_edit = None;
                    self.selected_transition = None;
                    self.transition_edit = None;
                    widget.invalidate_layout();
                }
                GraphEditorMessage::SetLiteral { node, pin, value } => {
                    if self.surface_mut().set_literal(node, pin, value) {
                        self.emit_changed(widget, emit);
                    }
                }
                GraphEditorMessage::SetInitialState(state) => {
                    if self
                        .document
                        .state_machine_mut()
                        .is_some_and(|document| document.set_initial(state))
                    {
                        self.emit_changed(widget, emit);
                    }
                }
                GraphEditorMessage::AddStateTransition(transition) => {
                    if self
                        .document
                        .state_machine_mut()
                        .is_some_and(|document| document.add_transition(transition))
                    {
                        self.selected_transition = self
                            .document
                            .state_machine()
                            .and_then(|document| document.transitions().len().checked_sub(1));
                        self.emit_changed(widget, emit);
                    }
                }
                GraphEditorMessage::SetStateTransition { index, transition } => {
                    if self
                        .document
                        .state_machine_mut()
                        .is_some_and(|document| document.set_transition(index, transition))
                    {
                        self.emit_changed(widget, emit);
                    }
                }
                GraphEditorMessage::RemoveStateTransition(index) => {
                    if self
                        .document
                        .state_machine_mut()
                        .is_some_and(|document| document.remove_transition(index))
                    {
                        self.selected_transition = None;
                        self.transition_edit = None;
                        self.emit_changed(widget, emit);
                    }
                }
                GraphEditorMessage::UndoStateOverlay => {
                    if self
                        .document
                        .state_machine_mut()
                        .and_then(AnimationStateMachineDocument::undo_overlay)
                        .is_some()
                    {
                        self.emit_changed(widget, emit);
                    }
                }
                GraphEditorMessage::RedoStateOverlay => {
                    if self
                        .document
                        .state_machine_mut()
                        .and_then(AnimationStateMachineDocument::redo_overlay)
                        .is_some()
                    {
                        self.emit_changed(widget, emit);
                    }
                }
                GraphEditorMessage::Command { id, paste_offset } => {
                    if self.surface_mut().dispatch_command(&id, paste_offset) {
                        self.emit_changed(widget, emit);
                    }
                }
                GraphEditorMessage::Changed(_) | GraphEditorMessage::StateMachineChanged(_) => {}
            }
            return;
        }

        let Some(message) = msg.data::<WidgetMessage>().cloned() else {
            return;
        };
        match message {
            WidgetMessage::MouseWheel { pos, delta, .. } => {
                let local = Self::local(widget, pos);
                if let Some(factor) = wheel_zoom_factor(delta) {
                    self.surface_mut().view.zoom_at(local, factor);
                    msg.handled = true;
                }
            }
            WidgetMessage::MouseDown {
                pos,
                button: MouseButton::Middle,
                ..
            } => {
                self.gesture = Gesture::Pan { last: pos };
                msg.handled = true;
            }
            WidgetMessage::MouseDown {
                pos,
                button: MouseButton::Left,
                mods,
            } => {
                if self.selected_transition.is_some()
                    && Self::transition_delete_rect(widget).contains(pos)
                {
                    self.commit_literal(widget, emit);
                    self.commit_transition_edit(widget, emit);
                    if let Some(index) = self.selected_transition
                        && self
                            .document
                            .state_machine_mut()
                            .is_some_and(|document| document.remove_transition(index))
                    {
                        self.selected_transition = None;
                        self.transition_edit = None;
                        self.emit_changed(widget, emit);
                    }
                } else if let Some(field) = self.transition_field_at(widget, pos) {
                    self.commit_literal(widget, emit);
                    self.commit_transition_edit(widget, emit);
                    self.begin_transition_edit(field);
                } else if let Some(index) = self.transition_at(widget, pos) {
                    self.commit_literal(widget, emit);
                    self.commit_transition_edit(widget, emit);
                    self.selected_transition = Some(index);
                    self.transition_error = None;
                } else if let Some((node, pin, draft)) = self.literal_at(widget, pos) {
                    self.commit_transition_edit(widget, emit);
                    self.commit_literal(widget, emit);
                    self.literal_edit = Some(LiteralEdit {
                        node,
                        pin,
                        draft,
                        replace_on_input: true,
                    });
                } else if let Some(pin) = self.pin_at(widget, pos) {
                    self.commit_transition_edit(widget, emit);
                    self.commit_literal(widget, emit);
                    self.gesture = Gesture::Wire { from: pin };
                } else if let Some(node) = self.node_at(widget, pos) {
                    self.commit_transition_edit(widget, emit);
                    self.commit_literal(widget, emit);
                    if mods.alt && self.is_state(node) {
                        if self
                            .document
                            .state_machine_mut()
                            .is_some_and(|document| document.set_initial(node))
                        {
                            self.emit_changed(widget, emit);
                        }
                    } else if mods.shift && self.is_state(node) {
                        self.gesture = Gesture::StateTransition { from: node };
                    } else {
                        if mods.command() {
                            self.surface_mut().selection.toggle(node);
                        } else if !self.surface().selection.contains(node) {
                            self.surface_mut().selection.select_only(node);
                        }
                        self.gesture = Gesture::Move {
                            last: pos,
                            before: self.surface().graph.clone(),
                        };
                    }
                } else {
                    self.commit_transition_edit(widget, emit);
                    self.commit_literal(widget, emit);
                    self.surface_mut().selection.clear();
                    self.gesture = Gesture::Box {
                        start: self.graph_point(widget, pos),
                    };
                }
                msg.handled = true;
            }
            WidgetMessage::MouseMove { pos, .. } => {
                widget.tooltip = self
                    .transition_field_at(widget, pos)
                    .map(|field| match field {
                        TransitionField::BlendSeconds => {
                            "Transition blend duration in seconds; finite and at least zero".into()
                        }
                        TransitionField::SyncTrack => {
                            "Optional sync-marker track; blank disables transition sync".into()
                        }
                        TransitionField::Conditions => "Semicolon-separated conditions: trigger:name; bool:name:true; float:name:greater:0.5; int:name:equal:1".into(),
                    })
                    .or_else(|| self
                    .literal_at(widget, pos)
                    .and_then(|(node, pin, _)| {
                        let surface = self.surface();
                        let node = surface.graph.node(node)?;
                        let input = surface
                            .catalogue
                            .get(&node.archetype)?
                            .inputs
                            .get(pin as usize)?;
                        let mut help = input.tooltip.unwrap_or(input.name).to_owned();
                        if let Some((min, max)) = input.range {
                            help.push_str(&format!(" (range {min}–{max})"));
                        }
                        Some(help)
                    }))
                    .unwrap_or_else(|| {
                        "Animation Graph — Alt-click State: initial; Shift-drag State: transition"
                            .into()
                    });
                let zoom = self.surface().view.zoom;
                match &mut self.gesture {
                    Gesture::Pan { last } => {
                        let delta = pos - *last;
                        *last = pos;
                        self.surface_mut().view.pan_by(delta);
                        msg.handled = true;
                    }
                    Gesture::Move { last, .. } => {
                        let delta = (pos - *last) / zoom;
                        *last = pos;
                        let ids = self.surface().selection.ids();
                        self.surface_mut().graph.translate(&ids, delta);
                        msg.handled = true;
                    }
                    Gesture::Box { start } => {
                        let start = *start;
                        let end = self
                            .surface()
                            .view
                            .screen_to_graph(Self::local(widget, pos));
                        let layouts = geometry::layout_nodes(
                            &self.surface().graph,
                            &self.surface().catalogue,
                        );
                        self.surface_mut().selection.select_box(
                            &layouts,
                            Rect::new(start.x, start.y, end.x - start.x, end.y - start.y),
                        );
                        msg.handled = true;
                    }
                    Gesture::None | Gesture::Wire { .. } | Gesture::StateTransition { .. } => {}
                }
            }
            WidgetMessage::MouseUp {
                pos,
                button: MouseButton::Left,
                ..
            } => {
                let gesture = std::mem::replace(&mut self.gesture, Gesture::None);
                match gesture {
                    Gesture::Move { before, .. } => {
                        if self
                            .surface_mut()
                            .commit_gesture(before, "Move Graph Nodes")
                        {
                            self.emit_changed(widget, emit);
                        }
                    }
                    Gesture::Wire { from } => {
                        if let Some(to) = self.pin_at(widget, pos) {
                            let changed = self.surface_mut().connect(from, to).is_ok()
                                || self.surface_mut().reconnect(from, to).is_ok();
                            if changed {
                                self.emit_changed(widget, emit);
                            }
                        }
                    }
                    Gesture::StateTransition { from } => {
                        if let Some(to) = self.node_at(widget, pos).filter(|to| self.is_state(*to))
                        {
                            let transition = AuthoredStateTransition {
                                from,
                                to,
                                conditions: Vec::new(),
                                blend_seconds: 0.2,
                                sync_track: None,
                            };
                            if self
                                .document
                                .state_machine_mut()
                                .is_some_and(|document| document.add_transition(transition))
                            {
                                self.selected_transition =
                                    self.document.state_machine().and_then(|document| {
                                        document.transitions().len().checked_sub(1)
                                    });
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
            WidgetMessage::Text(text) => {
                if let Some(edit) = self.transition_edit.as_mut() {
                    if edit.replace_on_input {
                        edit.draft.clear();
                        edit.replace_on_input = false;
                    }
                    edit.draft.push_str(&text);
                    msg.handled = true;
                } else if let Some(edit) = self.literal_edit.as_mut() {
                    if edit.replace_on_input {
                        edit.draft.clear();
                        edit.replace_on_input = false;
                    }
                    edit.draft.push_str(&text);
                    msg.handled = true;
                }
            }
            WidgetMessage::KeyDown(key, _) => {
                if let Some(edit) = self.transition_edit.as_mut() {
                    match key {
                        crate::message::KeyCode::Backspace => {
                            edit.replace_on_input = false;
                            edit.draft.pop();
                            msg.handled = true;
                        }
                        crate::message::KeyCode::Enter | crate::message::KeyCode::NumpadEnter => {
                            self.commit_transition_edit(widget, emit);
                            msg.handled = true;
                        }
                        crate::message::KeyCode::Escape => {
                            self.transition_edit = None;
                            self.transition_error = None;
                            msg.handled = true;
                        }
                        _ => {}
                    }
                } else if let Some(edit) = self.literal_edit.as_mut() {
                    match key {
                        crate::message::KeyCode::Backspace => {
                            edit.replace_on_input = false;
                            edit.draft.pop();
                            msg.handled = true;
                        }
                        crate::message::KeyCode::Enter | crate::message::KeyCode::NumpadEnter => {
                            self.commit_literal(widget, emit);
                            msg.handled = true;
                        }
                        crate::message::KeyCode::Escape => {
                            self.literal_edit = None;
                            msg.handled = true;
                        }
                        _ => {}
                    }
                }
            }
            WidgetMessage::Unfocus => {
                if self.literal_edit.is_some() || self.transition_edit.is_some() {
                    self.commit_literal(widget, emit);
                    self.commit_transition_edit(widget, emit);
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

fn compare_op_text(op: CompareOp) -> &'static str {
    match op {
        CompareOp::Equal => "equal",
        CompareOp::NotEqual => "not_equal",
        CompareOp::Less => "less",
        CompareOp::LessEqual => "less_equal",
        CompareOp::Greater => "greater",
        CompareOp::GreaterEqual => "greater_equal",
    }
}

fn parse_compare_op(text: &str) -> Option<CompareOp> {
    match text.trim() {
        "equal" => Some(CompareOp::Equal),
        "not_equal" => Some(CompareOp::NotEqual),
        "less" => Some(CompareOp::Less),
        "less_equal" => Some(CompareOp::LessEqual),
        "greater" => Some(CompareOp::Greater),
        "greater_equal" => Some(CompareOp::GreaterEqual),
        _ => None,
    }
}

fn format_conditions(conditions: &[Condition]) -> String {
    conditions
        .iter()
        .map(|condition| match condition {
            Condition::Bool { parameter, value } => format!("bool:{parameter}:{value}"),
            Condition::Float {
                parameter,
                op,
                value,
            } => format!("float:{parameter}:{}:{value}", compare_op_text(*op)),
            Condition::Int {
                parameter,
                op,
                value,
            } => format!("int:{parameter}:{}:{value}", compare_op_text(*op)),
            Condition::Trigger { parameter } => format!("trigger:{parameter}"),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn parse_conditions(text: &str) -> Option<Vec<Condition>> {
    let text = text.trim();
    if text.is_empty() {
        return Some(Vec::new());
    }
    text.split(';')
        .map(str::trim)
        .map(|condition| {
            let fields: Vec<_> = condition.split(':').map(str::trim).collect();
            match fields.as_slice() {
                ["trigger", parameter] if !parameter.is_empty() => Some(Condition::Trigger {
                    parameter: (*parameter).to_owned(),
                }),
                ["bool", parameter, value] if !parameter.is_empty() => Some(Condition::Bool {
                    parameter: (*parameter).to_owned(),
                    value: value.parse().ok()?,
                }),
                ["float", parameter, op, value] if !parameter.is_empty() => {
                    let value = value.parse::<f32>().ok()?;
                    let op = parse_compare_op(op)?;
                    value.is_finite().then(|| Condition::Float {
                        parameter: (*parameter).to_owned(),
                        op,
                        value,
                    })
                }
                ["int", parameter, op, value] if !parameter.is_empty() => Some(Condition::Int {
                    parameter: (*parameter).to_owned(),
                    op: parse_compare_op(op)?,
                    value: value.parse().ok()?,
                }),
                _ => None,
            }
        })
        .collect()
}

/// Builder for the shared retained-mode graph control.
pub struct GraphEditorBuilder {
    widget: WidgetBuilder,
    document: EditorDocument,
    font_id: u8,
}

impl GraphEditorBuilder {
    /// Create a graph control for one feature catalogue.
    #[must_use]
    pub fn new(widget: WidgetBuilder, catalogue: super::Catalogue) -> Self {
        Self {
            widget,
            document: EditorDocument::Graph(GraphSurface::new(catalogue)),
            font_id: 0,
        }
    }

    /// Seed the control with an authored graph.
    #[must_use]
    pub fn with_graph(mut self, graph: Graph) -> Self {
        self.document.surface_mut().graph = graph;
        self
    }

    /// Seed the control with the graph and cyclic overlay owned by one
    /// animation state-machine document.
    #[must_use]
    pub fn with_state_machine_document(mut self, document: AnimationStateMachineDocument) -> Self {
        self.document = EditorDocument::StateMachine(document);
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
                document: self.document,
                gesture: Gesture::None,
                literal_edit: None,
                selected_transition: None,
                transition_edit: None,
                transition_error: None,
                font_id: self.font_id,
            }),
        )
    }
}
