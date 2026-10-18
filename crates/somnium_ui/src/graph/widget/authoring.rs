//! Compact native authoring chrome around the shared graph control.
//! The graph remains the sole document owner, including inactive catalogues.

use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct AuthoringToolbar {
    pub(super) palette: Option<String>,
    offset: usize,
    documents: BTreeMap<String, EditorDocument>,
    pub(super) sources: BTreeMap<String, String>,
    pub(super) dirty: BTreeSet<String>,
    pub(super) status: String,
    pub(super) error: bool,
    /// Story catalogues' extra chrome: inspector, search, problem marks, the play-through.
    pub(super) story: super::story::StoryTools,
}

pub(super) const HEIGHT: f32 = 66.0;
const ROW: f32 = 24.0;
type Buttons = &'static [(&'static str, f32, f32, f32)];
const BUTTONS: Buttons = &[
    ("Scatter", 8.0, 5.0, 70.0),
    ("Behavior", 82.0, 5.0, 80.0),
    ("+ Add Node", 8.0, 35.0, 76.0),
    ("Open", 88.0, 35.0, 44.0),
    ("Save", 136.0, 35.0, 44.0),
    ("Preview", 184.0, 35.0, 60.0),
    ("Apply", 248.0, 35.0, 52.0),
];
/// Dialogue and quest graphs: no scene entity to apply to; Check validates against the project, Play runs it.
const STORY_BUTTONS: Buttons = &[
    ("Dialogue", 8.0, 5.0, 74.0),
    ("Quest", 86.0, 5.0, 60.0),
    ("+ Add Node", 8.0, 35.0, 76.0),
    ("Find", 88.0, 35.0, 44.0),
    ("Open", 136.0, 35.0, 44.0),
    ("Save", 184.0, 35.0, 44.0),
    ("Check", 232.0, 35.0, 52.0),
    ("Play", 288.0, 35.0, 44.0),
];

/// Whether a catalogue is a story graph (its document is a story asset, not a `.somgraph`).
pub(super) fn is_story_catalogue(id: &str) -> bool {
    matches!(id, super::super::dialogue::CATALOGUE | super::super::quest::CATALOGUE)
}

fn short(text: &str, width: f32) -> String {
    let limit = (width / 6.4).max(1.0) as usize;
    let mut value: String = text.chars().take(limit).collect();
    if text.chars().count() > limit {
        value.push('…');
    }
    value
}

impl GraphEditor {
    fn has_authoring(&self) -> bool {
        matches!(
            self.surface().catalogue.id,
            "somnium.scatter" | "somnium.behavior"
        ) || self.is_story()
    }

    pub(super) fn is_story(&self) -> bool {
        self.document.state_machine().is_none() && is_story_catalogue(self.surface().catalogue.id)
    }

    fn buttons(&self) -> Buttons {
        if self.is_story() { STORY_BUTTONS } else { BUTTONS }
    }

    fn button(&self, widget: &Widget, index: usize) -> Rect {
        let (_, x, y, width) = self.buttons()[index];
        let bounds = widget.screen_bounds();
        Rect::new(bounds.x + x, bounds.y + y, width, 24.0)
    }

    pub(super) fn authoring_offset(&self) -> Vec2 {
        if self.has_authoring() {
            Vec2::new(0.0, HEIGHT)
        } else {
            Vec2::ZERO
        }
    }

    pub(super) fn authoring_changed(&mut self) {
        // Marks from the last check describe the graph as it was; an edit makes them stale.
        self.authoring.story.problems.clear();
        if self.has_authoring() {
            self.authoring
                .dirty
                .insert(self.surface().catalogue.id.into());
            self.authoring.error = false;
            self.authoring.status =
                "Unsaved changes · Preview validates; Apply creates one undo step".into();
        }
    }

    pub(super) fn activate_authoring(&mut self, surface: GraphSurface, replace: bool) {
        let target = surface.catalogue.id.to_owned();
        let current = self.surface().catalogue.id.to_owned();
        if current == target && !replace {
            return;
        }
        let next = if replace {
            self.authoring.documents.remove(&target);
            EditorDocument::Graph(surface)
        } else {
            self.authoring
                .documents
                .remove(&target)
                .unwrap_or(EditorDocument::Graph(surface))
        };
        let old = std::mem::replace(&mut self.document, next);
        if current != target {
            self.authoring.documents.insert(current, old);
        }
        if replace {
            self.authoring.sources.remove(&target);
            self.authoring.dirty.remove(&target);
        }
        self.gesture = Gesture::None;
        self.literal_edit = None;
        self.selected_transition = None;
        self.transition_edit = None;
        self.authoring.palette = None;
        self.authoring.error = false;
        self.authoring.story.problems.clear();
        self.authoring.story.preview = None;
        self.authoring.status = match target.as_str() {
            "somnium.scatter" => {
                "Select a mesh · Scatter Settings controls the region · Preview before Apply"
            }
            super::super::dialogue::CATALOGUE => {
                "Wire lines to lines and choices · Check validates against the project · Play runs it from the selected node"
            }
            super::super::quest::CATALOGUE => {
                "Wire stages through Next 1-4 · objectives and journal lines into their stage · Check validates"
            }
            _ => "Connect tasks to Root · Preview validates · Apply attaches to the selected entity",
        }
        .into();
    }

    pub(super) fn activate_state_machine(&mut self, document: AnimationStateMachineDocument) {
        let current = self.surface().catalogue.id.to_owned();
        let preserve = self.has_authoring();
        let old = std::mem::replace(&mut self.document, EditorDocument::StateMachine(document));
        if preserve {
            self.authoring.documents.insert(current, old);
        }
        self.gesture = Gesture::None;
        self.authoring.palette = None;
    }

    pub(super) fn request_authoring_document(
        &mut self,
        widget: &Widget,
        apply: bool,
        preview: bool,
        emit: &mut Vec<UiMessage>,
    ) {
        if !self.has_authoring() {
            return;
        }
        if self.is_story() {
            self.request_story_document(widget, apply, preview, emit);
            return;
        }
        if apply {
            let validation = match self.surface().catalogue.id {
                "somnium.scatter" => {
                    super::super::scatter::compile(&self.surface().graph).map(|_| ())
                }
                _ => super::super::behavior::compile(&self.surface().graph).map(|_| ()),
            };
            if let Err(error) = validation {
                self.authoring.status = error;
                self.authoring.error = true;
                return;
            }
        }
        let surface = self.surface();
        match super::super::serial::to_json(&surface.graph, &surface.catalogue) {
            Ok(json) => emit.push(UiMessage::new(
                widget.handle,
                MessageDirection::FromWidget,
                GraphEditorMessage::Document {
                    catalogue: surface.catalogue.id.into(),
                    json,
                    apply,
                    preview,
                    source: self.authoring.sources.get(surface.catalogue.id).cloned(),
                },
            )),
            Err(error) => {
                self.authoring.status = error.to_string();
                self.authoring.error = true;
            }
        }
    }

    fn palette_rect(widget: &Widget) -> Rect {
        let bounds = widget.screen_bounds();
        Rect::new(
            bounds.x + 8.0,
            bounds.y + HEIGHT,
            (bounds.w - 16.0).min(340.0),
            (bounds.h - HEIGHT - 30.0).max(70.0),
        )
    }

    fn palette_items(&self) -> Vec<(String, String)> {
        self.surface()
            .palette(self.authoring.palette.as_deref().unwrap_or_default())
            .into_iter()
            .map(|node| {
                (
                    node.id.to_owned(),
                    format!("{} · {}", node.title, node.category),
                )
            })
            .collect()
    }

    fn add_palette_node(&mut self, widget: &Widget, index: usize, emit: &mut Vec<UiMessage>) {
        let Some((id, _)) = self.palette_items().get(index).cloned() else {
            return;
        };
        let at = self
            .surface()
            .view
            .screen_to_graph(Vec2::new(widget.screen_bounds().w * 0.42, 80.0));
        if self.is_story() {
            let title = self.story_fresh_id(&id);
            self.surface_mut().add_named(&id, at, title);
        } else {
            self.surface_mut().add(&id, at);
        }
        self.authoring.palette = None;
        self.emit_changed(widget, emit);
    }

    pub(super) fn authoring_pointer(&self, widget: &Widget, pos: Vec2) -> bool {
        self.has_authoring()
            && ((0..self.buttons().len()).any(|i| self.button(widget, i).contains(pos))
                || (self.authoring.palette.is_some() && Self::palette_rect(widget).contains(pos)))
    }

    pub(super) fn handle_authoring_input(
        &mut self,
        widget: &mut Widget,
        message: &WidgetMessage,
        emit: &mut Vec<UiMessage>,
    ) -> bool {
        if !self.has_authoring() {
            return false;
        }
        if let WidgetMessage::MouseDown {
            pos,
            button: MouseButton::Left,
            ..
        } = message
        {
            if let Some(index) =
                (0..self.buttons().len()).find(|i| self.button(widget, *i).contains(*pos))
            {
                self.commit_literal(widget, emit);
                if self.is_story() {
                    use crate::editor_event::GraphToolAction as A;
                    let action = match index {
                        0 => Some(A::Dialogue),
                        1 => Some(A::Quest),
                        2 => {
                            self.authoring.palette = if self.authoring.palette.is_some() {
                                None
                            } else {
                                Some(String::new())
                            };
                            self.authoring.story.find = None;
                            self.authoring.offset = 0;
                            None
                        }
                        3 => {
                            self.toggle_find();
                            None
                        }
                        4 => Some(A::Open),
                        5 => Some(A::Save),
                        6 => Some(A::Preview),
                        _ => (self.surface().catalogue.id == super::super::dialogue::CATALOGUE)
                            .then_some(A::Play),
                    };
                    if let Some(action) = action {
                        emit.push(UiMessage::new(
                            widget.handle,
                            MessageDirection::FromWidget,
                            GraphEditorMessage::Tool(action),
                        ));
                    }
                    return true;
                }
                if index == 2 {
                    self.authoring.palette = if self.authoring.palette.is_some() {
                        None
                    } else {
                        Some(String::new())
                    };
                    self.authoring.offset = 0;
                } else {
                    use crate::editor_event::GraphToolAction as A;
                    let action = match index {
                        0 => A::Scatter,
                        1 => A::Behavior,
                        3 => A::Open,
                        4 => A::Save,
                        5 => A::Preview,
                        _ => A::Apply,
                    };
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        GraphEditorMessage::Tool(action),
                    ));
                }
                return true;
            }
            if self.authoring.palette.is_some() {
                let rect = Self::palette_rect(widget);
                if rect.contains(*pos) && pos.y >= rect.y + 32.0 {
                    let index = self.authoring.offset + ((pos.y - rect.y - 32.0) / ROW) as usize;
                    self.add_palette_node(widget, index, emit);
                } else if !rect.contains(*pos) {
                    self.authoring.palette = None;
                }
                return true;
            }
            let bounds = widget.screen_bounds();
            if pos.y < bounds.y + HEIGHT || pos.y >= bounds.y + bounds.h - 25.0 {
                return true;
            }
        }
        if self.authoring.palette.is_some() {
            match message {
                WidgetMessage::Text(value) => {
                    self.authoring.palette.as_mut().unwrap().push_str(value);
                    self.authoring.offset = 0;
                }
                WidgetMessage::KeyDown(key, _) => match key {
                    crate::message::KeyCode::Escape => self.authoring.palette = None,
                    crate::message::KeyCode::Backspace => {
                        self.authoring.palette.as_mut().unwrap().pop();
                        self.authoring.offset = 0;
                    }
                    crate::message::KeyCode::Enter | crate::message::KeyCode::NumpadEnter => {
                        self.add_palette_node(widget, self.authoring.offset, emit)
                    }
                    _ => {}
                },
                WidgetMessage::MouseWheel { delta, .. } => {
                    let last = self.palette_items().len().saturating_sub(1);
                    if *delta < 0.0 {
                        self.authoring.offset = (self.authoring.offset + 1).min(last);
                    } else {
                        self.authoring.offset = self.authoring.offset.saturating_sub(1);
                    }
                }
                WidgetMessage::Unfocus => self.authoring.palette = None,
                _ => return false,
            }
            return true;
        }
        false
    }

    pub(super) fn draw_authoring(&self, widget: &Widget, ctx: &mut DrawingContext) {
        if !self.has_authoring() {
            return;
        }
        let theme = crate::theme::active();
        let bounds = widget.screen_bounds();
        ctx.push_rect_filled(
            Rect::new(bounds.x, bounds.y, bounds.w, HEIGHT),
            theme.semantic.surface.panel.bytes(),
        );
        let id = self.surface().catalogue.id;
        for (index, (label, _, _, _)) in self.buttons().iter().enumerate() {
            let active = match (self.is_story(), index) {
                (false, 0) => id == "somnium.scatter",
                (false, 1) => id == "somnium.behavior",
                (true, 0) => id == super::super::dialogue::CATALOGUE,
                (true, 1) => id == super::super::quest::CATALOGUE,
                (true, 3) => self.authoring.story.find.is_some(),
                (true, 7) => self.authoring.story.preview.is_some(),
                _ => false,
            };
            if self.is_story() && index == 7 && id != super::super::dialogue::CATALOGUE {
                continue;
            }
            let rect = self.button(widget, index);
            ctx.push_primitive(
                Primitive::fill(
                    rect,
                    if active {
                        theme.semantic.accent.default.bytes()
                    } else {
                        theme.semantic.surface.popup.bytes()
                    },
                )
                .with_radius(theme.geometry.radius_input)
                .with_border(1.0, theme.semantic.border.default.bytes()),
                None,
            );
            ctx.push_text(
                label,
                Vec2::new(rect.x + 7.0, rect.y + 6.0),
                self.font_id,
                11.0,
                theme.semantic.text.primary.bytes(),
            );
        }
        let source = self
            .authoring
            .sources
            .get(id)
            .and_then(|path| path.rsplit(['/', '\\']).next())
            .unwrap_or("Untitled graph");
        let title = format!(
            "{}{}",
            source,
            if self.authoring.dirty.contains(id) {
                " *"
            } else {
                ""
            }
        );
        let title_x = if self.is_story() { 154.0 } else { 174.0 };
        ctx.push_text(
            &short(&title, bounds.w - title_x - 4.0),
            Vec2::new(bounds.x + title_x, bounds.y + 12.0),
            self.font_id,
            11.0,
            theme.semantic.text.secondary.bytes(),
        );
        let footer = Rect::new(bounds.x, bounds.y + bounds.h - 25.0, bounds.w, 25.0);
        ctx.push_rect_filled(footer, theme.semantic.surface.panel.bytes());
        ctx.push_text(
            &short(&self.authoring.status, bounds.w - 16.0),
            Vec2::new(footer.x + 8.0, footer.y + 7.0),
            self.font_id,
            10.0,
            if self.authoring.error {
                theme.semantic.status.warning.bytes()
            } else {
                theme.semantic.text.secondary.bytes()
            },
        );
        if let Some(query) = &self.authoring.palette {
            let rect = Self::palette_rect(widget);
            ctx.push_primitive(
                Primitive::fill(rect, theme.semantic.surface.popup.bytes())
                    .with_radius(theme.geometry.radius_input)
                    .with_border(1.0, theme.semantic.border.focus.bytes()),
                None,
            );
            ctx.push_text(
                &short(&format!("Search nodes: {query}|"), rect.w - 16.0),
                Vec2::new(rect.x + 8.0, rect.y + 10.0),
                self.font_id,
                11.0,
                theme.semantic.text.primary.bytes(),
            );
            let count = ((rect.h - 32.0) / ROW).floor().max(0.0) as usize;
            for (row, (_, title)) in self
                .palette_items()
                .iter()
                .skip(self.authoring.offset)
                .take(count)
                .enumerate()
            {
                ctx.push_text(
                    &short(title, rect.w - 16.0),
                    Vec2::new(rect.x + 8.0, rect.y + 39.0 + row as f32 * ROW),
                    self.font_id,
                    11.0,
                    theme.semantic.text.primary.bytes(),
                );
            }
            if self.palette_items().is_empty() {
                ctx.push_text(
                    "No matching nodes",
                    Vec2::new(rect.x + 8.0, rect.y + 39.0),
                    self.font_id,
                    11.0,
                    theme.semantic.text.secondary.bytes(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn editor() -> GraphEditor {
        GraphEditor {
            document: EditorDocument::Graph(super::super::super::scatter::default_surface()),
            gesture: Gesture::None,
            literal_edit: None,
            selected_transition: None,
            transition_edit: None,
            transition_error: None,
            font_id: 0,
            authoring: AuthoringToolbar::default(),
        }
    }

    #[test]
    fn story_catalogues_get_their_own_toolbar_and_named_nodes() {
        let mut editor = editor();
        editor.activate_authoring(super::super::super::dialogue::default_surface(), false);
        assert!(editor.is_story());
        assert_eq!(editor.buttons().len(), 8);
        editor.authoring.palette = Some("choice".into());
        editor.add_palette_node(&WidgetBuilder::new().build(), 0, &mut Vec::new());
        let added = editor.surface().selection.ids()[0];
        assert_eq!(editor.surface().graph.node(added).unwrap().title, "o1");
        assert!(editor.surface_mut().undo(), "adding a named node is one undo step");
        assert!(editor.surface().graph.node(added).is_none());
        editor.activate_authoring(super::super::super::scatter::default_surface(), false);
        assert!(!editor.is_story());
        assert_eq!(editor.buttons().len(), 7);
    }

    #[test]
    fn switching_catalogues_preserves_edits_history_and_source() {
        let mut editor = editor();
        let node = editor.surface().graph.nodes()[0].id;
        editor.surface_mut().set_literal(node, 0, "17");
        editor
            .authoring
            .sources
            .insert("somnium.scatter".into(), "assets/forest.somgraph".into());
        editor.activate_authoring(super::super::super::behavior::default_surface(), false);
        editor.activate_authoring(super::super::super::scatter::default_surface(), false);
        editor.activate_state_machine(AnimationStateMachineDocument::new(GraphSurface::new(
            super::super::super::catalogues::animation(),
        )));
        editor.activate_authoring(super::super::super::scatter::default_surface(), false);
        assert_eq!(
            editor.surface().graph.node(node).unwrap().literals[&0],
            "17"
        );
        assert!(editor.surface_mut().undo());
        assert_ne!(
            editor
                .surface()
                .graph
                .node(node)
                .unwrap()
                .literals
                .get(&0)
                .map(String::as_str),
            Some("17")
        );
        assert_eq!(
            editor.authoring.sources["somnium.scatter"],
            "assets/forest.somgraph"
        );
    }

    #[test]
    fn palette_filters_registered_nodes_and_invalid_apply_keeps_document() {
        let mut editor = editor();
        editor.authoring.palette = Some("altitude".into());
        assert_eq!(editor.palette_items().len(), 1);
        let root = editor
            .surface()
            .graph
            .nodes()
            .iter()
            .find(|n| n.archetype == "scatter.output")
            .unwrap()
            .id;
        editor.surface_mut().set_literal(root, 2, "NaN");
        let before = editor.surface().graph.clone();
        let mut emit = Vec::new();
        editor.request_authoring_document(&WidgetBuilder::new().build(), true, false, &mut emit);
        assert!(emit.is_empty() && editor.authoring.error);
        assert_eq!(editor.surface().graph, before);
    }
}
