//! Story authoring chrome on the graph control: a node inspector for long text, node search, problem marks, and
//! the dialogue play-through ("play from node").
//!
//! The inspector is generic (any node whose archetype has inspector-only or multi-line fields gets it); the rest is
//! for the dialogue and quest catalogues, whose documents are `somnium_story` assets rather than `.somgraph` files.

use super::*;
use crate::graph::{dialogue, quest};
use somnium_story::conversation::{Beat, Conversation};
use somnium_story::problem::{Problem, has_errors};
use somnium_story::{Story, StoryDatabase, StoryEvent};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The literal-edit address of a node's title (a story node's id) in the inspector.
pub(super) const TITLE_PIN: u16 = u16::MAX;

const INSPECTOR_WIDTH: f32 = 340.0;
const PREVIEW_HEIGHT: f32 = 214.0;
const FOOTER: f32 = 25.0;
/// Layout estimate of one character at the inspector's 11 px; drawing and hit-testing share it.
const CHAR_W: f32 = 6.2;
const LINE_H: f32 = 15.0;

/// What a graph cannot hold of a story asset, per catalogue.
pub(super) enum Extras {
    Dialogue(dialogue::Extras),
    Quest(quest::Extras),
}

/// The node search.
#[derive(Default)]
pub(super) struct Find {
    pub(super) query: String,
    pub(super) offset: usize,
}

/// A dialogue being played in the editor.
pub(super) struct Preview {
    database: Arc<StoryDatabase>,
    story: Story,
    conversation: Option<Conversation>,
    dialogue: String,
    entry: String,
    log: Vec<String>,
    error: Option<String>,
    /// Effects run before the play-through starts, to try a branch (`lang.patter = 'fluent'`).
    setup: String,
    setup_edit: Option<String>,
    followed: Option<NodeId>,
    /// Quest news for the Game View's notices, `(heading, text)`, not yet sent.
    notices: Vec<(String, String)>,
}

#[derive(Default)]
pub(super) struct StoryTools {
    pub(super) extras: BTreeMap<&'static str, Extras>,
    pub(super) problems: BTreeMap<NodeId, Vec<(String, bool)>>,
    pub(super) find: Option<Find>,
    pub(super) inspector_scroll: f32,
    pub(super) preview: Option<Preview>,
    /// The setup a writer typed survives replays.
    last_setup: String,
    /// What the Game View was last sent, so a beat is sent once.
    sent: Option<Beat>,
    /// Show play-throughs in the game's dialogue presenter over the editor; it stays on across replays.
    pub(super) game_view: bool,
    /// Moves to make as soon as the next play-through starts (a capture script's choices).
    pub(super) script: Vec<PreviewInput>,
}

/// What the Game View's presenter asked of the play-through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewInput {
    Next,
    Choose(usize),
    /// The player pressed Esc with nothing open: leave the Game View.
    Leave,
}

/// One inspector field.
struct Field {
    pin: u16,
    label: String,
    help: Option<&'static str>,
    value: String,
    wired: bool,
    lines: Vec<String>,
    label_y: f32,
    rect: Rect,
    help_lines: Vec<String>,
}

fn wrap(text: &str, width: f32) -> Vec<String> {
    crate::widgets::text::wrap_lines(text, width, |s| s.chars().count() as f32 * CHAR_W)
}

fn small_wrap(text: &str, width: f32) -> Vec<String> {
    crate::widgets::text::wrap_lines(text, width, |s| s.chars().count() as f32 * 5.4)
}

/// Markup removed, for the play-through's plain reading.
fn plain(text: &str) -> String {
    let cued = somnium_story::presenter::parse_cues(text);
    crate::text::markup::parse(&cued.display).map_or(cued.display, |(plain, _)| plain)
}

impl GraphEditor {
    // --- documents -------------------------------------------------------------------------------------------

    pub(super) fn story_fresh_id(&self, archetype: &str) -> String {
        if self.surface().catalogue.id == quest::CATALOGUE {
            quest::fresh_id(&self.surface().graph, archetype)
        } else {
            dialogue::fresh_id(&self.surface().graph, archetype)
        }
    }

    /// Open a story asset's JSON as this control's document.
    pub(super) fn open_story(&mut self, catalogue: &str, json: &str, source: Option<String>) {
        let opened = match catalogue {
            dialogue::CATALOGUE => somnium_story::DialogueAsset::from_json(json).map(|asset| {
                let (surface, extras) = dialogue::surface(&asset);
                (surface, Extras::Dialogue(extras), dialogue::CATALOGUE)
            }),
            quest::CATALOGUE => somnium_story::QuestAsset::from_json(json).map(|asset| {
                let (surface, extras) = quest::surface(&asset);
                (surface, Extras::Quest(extras), quest::CATALOGUE)
            }),
            other => Err(format!("{other} is not a story catalogue")),
        };
        match opened {
            Ok((surface, extras, id)) => {
                self.activate_authoring(surface, true);
                self.authoring.story.extras.insert(id, extras);
                if let Some(source) = source {
                    self.authoring.sources.insert(id.to_owned(), source);
                }
                self.authoring.status = "Opened · Check validates against the project".into();
                self.authoring.error = false;
            }
            Err(error) => {
                self.authoring.status = format!("Could not open: {error}");
                self.authoring.error = true;
            }
        }
    }

    /// Save, Check and Play: convert the graph to its asset and hand the asset's JSON to the host.
    pub(super) fn request_story_document(
        &mut self,
        widget: &Widget,
        apply: bool,
        preview: bool,
        emit: &mut Vec<UiMessage>,
    ) {
        let id = self.surface().catalogue.id;
        let graph = &self.surface().graph;
        let converted = if id == quest::CATALOGUE {
            let extras = match self.authoring.story.extras.get(id) {
                Some(Extras::Quest(extras)) => extras.clone(),
                _ => quest::Extras::default(),
            };
            quest::from_graph_with(graph, &extras).map(|asset| {
                let mut problems = asset.structural_problems();
                problems.extend(asset.unreachable().into_iter().map(|stage| {
                    Problem::warning(
                        format!("stage {stage}"),
                        "no path from the first stage reaches this stage",
                    )
                }));
                (asset.to_json(), problems)
            })
        } else {
            let extras = match self.authoring.story.extras.get(id) {
                Some(Extras::Dialogue(extras)) => extras.clone(),
                _ => dialogue::Extras::default(),
            };
            dialogue::from_graph_with(graph, &extras)
                .map(|asset| (asset.to_json(), asset.structural_problems()))
        };
        let (json, problems) = match converted {
            Ok((Ok(json), problems)) => (Some(json), problems),
            Ok((Err(error), problems)) => {
                let mut problems = problems;
                problems.push(Problem::error("", error));
                (None, problems)
            }
            Err(problems) => (None, problems),
        };
        let marks: Vec<(String, String, bool)> = problems
            .iter()
            .map(|p| (p.at.clone(), p.message.clone(), p.warning))
            .collect();
        self.set_story_problems(&marks);
        let Some(json) = json.filter(|_| !(apply && has_errors(&problems))) else {
            return;
        };
        emit.push(UiMessage::new(
            widget.handle,
            MessageDirection::FromWidget,
            GraphEditorMessage::Document {
                catalogue: id.into(),
                json,
                apply,
                preview,
                source: self.authoring.sources.get(id).cloned(),
            },
        ));
        if apply {
            self.authoring.status = if preview {
                "Checking against the project…".into()
            } else {
                "Starting the play-through…".into()
            };
        }
    }

    /// Mark problems on their nodes; asset-level ones go to the footer.
    pub(super) fn set_story_problems(&mut self, problems: &[(String, String, bool)]) {
        let graph = &self.surface().graph;
        let quest = self.surface().catalogue.id == quest::CATALOGUE;
        let mut marks: BTreeMap<NodeId, Vec<(String, bool)>> = BTreeMap::new();
        let mut loose = Vec::new();
        for (at, message, warning) in problems {
            let node = if at.is_empty() {
                None
            } else if quest {
                quest::find_node(graph, at)
            } else {
                dialogue::find_node(graph, at)
            };
            match node {
                Some(node) => marks
                    .entry(node)
                    .or_default()
                    .push((message.clone(), *warning)),
                None => loose.push((at.clone(), message.clone(), *warning)),
            }
        }
        let errors = problems.iter().filter(|p| !p.2).count();
        let warnings = problems.len() - errors;
        self.authoring.story.problems = marks;
        self.authoring.error = errors > 0;
        self.authoring.status = if problems.is_empty() {
            "No problems".into()
        } else {
            let first = problems
                .iter()
                .find(|p| !p.2)
                .or_else(|| problems.first())
                .map(|(at, message, _)| {
                    if at.is_empty() {
                        message.clone()
                    } else {
                        format!("{at}: {message}")
                    }
                })
                .unwrap_or_default();
            let _ = loose;
            format!("{errors} error(s), {warnings} warning(s) · {first}")
        };
    }

    /// Border colour for a node the story chrome is marking: the play-through's node, then problems.
    pub(super) fn story_border(&self, node: NodeId) -> Option<[u8; 4]> {
        let theme = crate::theme::active();
        if self
            .authoring
            .story
            .preview
            .as_ref()
            .and_then(|p| p.followed)
            == Some(node)
        {
            return Some(theme.semantic.status.info.bytes());
        }
        let marks = self.authoring.story.problems.get(&node)?;
        Some(if marks.iter().any(|(_, warning)| !warning) {
            theme.semantic.status.error.bytes()
        } else {
            theme.semantic.status.warning.bytes()
        })
    }

    /// The hover text for a marked node.
    pub(super) fn story_tooltip(&self, widget: &Widget, pos: Vec2) -> Option<String> {
        let node = self.node_at(widget, pos)?;
        let marks = self.authoring.story.problems.get(&node)?;
        Some(
            marks
                .iter()
                .map(|(message, warning)| {
                    format!("{}: {message}", if *warning { "Warning" } else { "Error" })
                })
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    // --- layout ----------------------------------------------------------------------------------------------

    fn preview_rect(widget: &Widget) -> Rect {
        let b = widget.screen_bounds();
        Rect::new(
            b.x + 8.0,
            b.y + b.h - FOOTER - 8.0 - PREVIEW_HEIGHT,
            (b.w - 16.0).max(80.0),
            PREVIEW_HEIGHT,
        )
    }

    /// The single selected node the inspector shows, if it wants one.
    fn inspector_node(&self) -> Option<NodeId> {
        if self.document.state_machine().is_some() {
            return None;
        }
        let ids = self.surface().selection.ids();
        let [id] = ids.as_slice() else { return None };
        let node = self.surface().graph.node(*id)?;
        self.surface()
            .catalogue
            .get(&node.archetype)?
            .wants_inspector()
            .then_some(*id)
    }

    fn inspector_rect(&self, widget: &Widget) -> Option<Rect> {
        self.inspector_node()?;
        let b = widget.screen_bounds();
        if b.w < 620.0 {
            return None;
        }
        let top = b.y + self.authoring_offset().y + 8.0;
        let bottom = if self.authoring.story.preview.is_some() {
            Self::preview_rect(widget).y - 8.0
        } else {
            b.y + b.h - FOOTER - 8.0
        };
        Some(Rect::new(
            b.x + b.w - INSPECTOR_WIDTH - 8.0,
            top,
            INSPECTOR_WIDTH,
            (bottom - top).max(60.0),
        ))
    }

    fn inspector_fields(&self, rect: Rect) -> Vec<Field> {
        let Some(id) = self.inspector_node() else {
            return Vec::new();
        };
        let surface = self.surface();
        let Some(node) = surface.graph.node(id) else {
            return Vec::new();
        };
        let Some(archetype) = surface.catalogue.get(&node.archetype) else {
            return Vec::new();
        };
        let width = rect.w - 24.0;
        let mut y = rect.y + 40.0 - self.authoring.story.inspector_scroll;
        let mut fields = Vec::new();
        let editing = |pin: u16| {
            self.literal_edit
                .as_ref()
                .filter(|e| e.node == id && e.pin == pin)
                .map(|e| e.draft.clone())
        };
        let mut push = |pin: u16,
                        label: String,
                        help: Option<&'static str>,
                        value: String,
                        wired: bool,
                        multiline: bool| {
            let shown = editing(pin).map_or(value.clone(), |draft| format!("{draft}|"));
            let mut lines = wrap(&shown, width - 10.0);
            let min_lines = if multiline { 3 } else { 1 };
            while lines.len() < min_lines {
                lines.push(String::new());
            }
            let label_y = y;
            let rect = Rect::new(
                rect.x + 12.0,
                y + 15.0,
                width,
                lines.len() as f32 * LINE_H + 8.0,
            );
            let help_lines: Vec<String> = help
                .map(|h| small_wrap(h, width).into_iter().take(3).collect())
                .unwrap_or_default();
            y = rect.y + rect.h + 4.0 + help_lines.len() as f32 * 12.0 + 10.0;
            fields.push(Field {
                pin,
                label,
                help,
                value,
                wired,
                lines,
                label_y,
                rect,
                help_lines,
            });
        };
        if self.is_story()
            && !matches!(
                node.archetype.as_str(),
                dialogue::START | dialogue::ENTRY | quest::QUEST
            )
        {
            let automatic = node.title.trim().is_empty();
            let shown = if automatic {
                let ids = if surface.catalogue.id == quest::CATALOGUE {
                    quest::node_ids(&surface.graph)
                } else {
                    dialogue::node_ids(&surface.graph)
                };
                ids.get(&id).cloned().unwrap_or_default()
            } else {
                node.title.clone()
            };
            push(
                TITLE_PIN,
                if automatic {
                    "Id (automatic)".into()
                } else {
                    "Id".into()
                },
                Some(
                    "What saves, string keys, jumps and conditions (visited, done) name this node by. Rename with care.",
                ),
                shown,
                false,
                false,
            );
        }
        for (pin, input) in archetype.inputs.iter().enumerate() {
            if input.default.is_none() {
                continue;
            }
            let pin = pin as u16;
            let wired = surface.graph.input_source(PinRef::input(id, pin)).is_some();
            let value = if wired {
                "(wired)".to_owned()
            } else {
                node.literals
                    .get(&pin)
                    .cloned()
                    .or_else(|| input.default.map(str::to_owned))
                    .unwrap_or_default()
            };
            push(
                pin,
                input.name.to_owned(),
                input.tooltip,
                value,
                wired,
                input.multiline,
            );
        }
        fields
    }

    // --- input -----------------------------------------------------------------------------------------------

    pub(super) fn toggle_find(&mut self) {
        self.authoring.palette = None;
        self.authoring.story.find = match self.authoring.story.find {
            Some(_) => None,
            None => Some(Find::default()),
        };
    }

    fn find_results(&self) -> Vec<(NodeId, String)> {
        let Some(find) = &self.authoring.story.find else {
            return Vec::new();
        };
        let query = find.query.trim().to_lowercase();
        let surface = self.surface();
        surface
            .graph
            .nodes()
            .iter()
            .filter_map(|node| {
                let archetype = surface.catalogue.get(&node.archetype)?;
                let mut haystack = format!("{} {} ", node.title, archetype.title);
                for value in node.literals.values() {
                    haystack.push_str(value);
                    haystack.push(' ');
                }
                if !query.is_empty() && !haystack.to_lowercase().contains(&query) {
                    return None;
                }
                let snippet = node
                    .literals
                    .iter()
                    .filter(|(pin, value)| {
                        !value.trim().is_empty()
                            && archetype
                                .inputs
                                .get(usize::from(**pin))
                                .is_some_and(|p| p.multiline)
                    })
                    .map(|(_, v)| v.replace('\n', " "))
                    .next()
                    .unwrap_or_default();
                let name = if node.title.is_empty() {
                    archetype.title
                } else {
                    &node.title
                };
                Some((node.id, format!("{name} · {} · {snippet}", archetype.title)))
            })
            .collect()
    }

    /// Select a node and bring it into view.
    pub(super) fn reveal(&mut self, widget: &Widget, node: NodeId) {
        let surface = self.surface();
        let Some(layout) = surface
            .graph
            .node(node)
            .and_then(|n| geometry::layout_node(n, &surface.catalogue))
        else {
            return;
        };
        let canvas = widget.screen_bounds().size() - self.authoring_offset();
        let centre = layout.bounds.pos() + layout.bounds.size() * 0.5;
        let view = &mut self.surface_mut().view;
        let on_screen = view.graph_to_screen(centre);
        if on_screen.x < 40.0
            || on_screen.y < 40.0
            || on_screen.x > canvas.x - 380.0
            || on_screen.y > canvas.y - 260.0
        {
            view.pan = Vec2::new(canvas.x * 0.38, canvas.y * 0.36) - centre * view.zoom;
        }
        self.surface_mut().selection.select_only(node);
    }

    /// Story chrome input. Returns whether it took the message.
    pub(super) fn handle_story_input(
        &mut self,
        widget: &mut Widget,
        message: &WidgetMessage,
        emit: &mut Vec<UiMessage>,
    ) -> bool {
        if self.authoring.story.find.is_some() && self.is_story() {
            return self.handle_find(widget, message);
        }
        if self.authoring.story.preview.is_some() && self.handle_preview(widget, message, emit) {
            return true;
        }
        let Some(rect) = self.inspector_rect(widget) else {
            return false;
        };
        match message {
            WidgetMessage::MouseWheel { pos, delta, .. } if rect.contains(*pos) => {
                let fields = self.inspector_fields(rect);
                let content = fields.last().map_or(0.0, |f| {
                    f.rect.y + f.rect.h + 40.0 + self.authoring.story.inspector_scroll - rect.y
                });
                let limit = (content - rect.h).max(0.0);
                self.authoring.story.inspector_scroll = (self.authoring.story.inspector_scroll
                    - delta.signum() * 48.0)
                    .clamp(0.0, limit);
                true
            }
            WidgetMessage::MouseDown {
                pos,
                button: MouseButton::Left,
                ..
            } if rect.contains(*pos) => {
                let Some(node) = self.inspector_node() else {
                    return true;
                };
                let hit = self
                    .inspector_fields(rect)
                    .into_iter()
                    .find(|f| f.rect.contains(*pos) && f.rect.y >= rect.y + 30.0 && !f.wired);
                self.commit_literal(widget, emit);
                if let Some(field) = hit {
                    self.literal_edit = Some(LiteralEdit {
                        node,
                        pin: field.pin,
                        draft: if field.pin == TITLE_PIN
                            && self
                                .surface()
                                .graph
                                .node(node)
                                .is_some_and(|n| n.title.trim().is_empty())
                        {
                            String::new()
                        } else {
                            field.value
                        },
                        replace_on_input: false,
                    });
                }
                true
            }
            WidgetMessage::MouseDown { pos, .. } | WidgetMessage::MouseUp { pos, .. }
                if rect.contains(*pos) =>
            {
                true
            }
            _ => false,
        }
    }

    fn find_rect(widget: &Widget) -> Rect {
        let b = widget.screen_bounds();
        Rect::new(
            b.x + 8.0,
            b.y + authoring::HEIGHT,
            (b.w - 16.0).min(420.0),
            (b.h - authoring::HEIGHT - 30.0).max(70.0),
        )
    }

    fn handle_find(&mut self, widget: &mut Widget, message: &WidgetMessage) -> bool {
        let rect = Self::find_rect(widget);
        let rows = ((rect.h - 32.0) / 22.0).floor().max(0.0) as usize;
        let results = self.find_results();
        let Some(find) = self.authoring.story.find.as_mut() else {
            return false;
        };
        match message {
            WidgetMessage::Text(text) => {
                find.query.push_str(text);
                find.offset = 0;
            }
            WidgetMessage::KeyDown(key, _) => match key {
                crate::message::KeyCode::Escape => self.authoring.story.find = None,
                crate::message::KeyCode::Backspace => {
                    find.query.pop();
                    find.offset = 0;
                }
                crate::message::KeyCode::ArrowDown => {
                    find.offset = (find.offset + 1).min(results.len().saturating_sub(1));
                }
                crate::message::KeyCode::ArrowUp => find.offset = find.offset.saturating_sub(1),
                crate::message::KeyCode::Enter | crate::message::KeyCode::NumpadEnter => {
                    let at = find.offset;
                    if let Some((node, _)) = results.get(at) {
                        let node = *node;
                        self.authoring.story.find = None;
                        self.reveal(widget, node);
                    }
                }
                _ => return false,
            },
            WidgetMessage::MouseWheel { delta, .. } => {
                if *delta < 0.0 {
                    find.offset = (find.offset + 1).min(results.len().saturating_sub(1));
                } else {
                    find.offset = find.offset.saturating_sub(1);
                }
            }
            WidgetMessage::MouseDown { pos, .. } => {
                if rect.contains(*pos) && pos.y >= rect.y + 32.0 {
                    let first = find
                        .offset
                        .saturating_sub(rows.saturating_sub(1).min(find.offset));
                    let index = first + ((pos.y - rect.y - 32.0) / 22.0) as usize;
                    if let Some((node, _)) = results.get(index) {
                        let node = *node;
                        self.authoring.story.find = None;
                        self.reveal(widget, node);
                    }
                } else if !rect.contains(*pos) && !self.authoring_pointer(widget, *pos) {
                    self.authoring.story.find = None;
                    return false;
                } else if !rect.contains(*pos) {
                    return false;
                }
            }
            WidgetMessage::Unfocus => self.authoring.story.find = None,
            _ => return false,
        }
        true
    }

    // --- the play-through ------------------------------------------------------------------------------------

    /// Start (or restart) playing the open dialogue over `database` from the selected node.
    pub(super) fn start_preview(&mut self, widget: &Widget, database: Arc<StoryDatabase>) {
        if self.surface().catalogue.id != dialogue::CATALOGUE {
            return;
        }
        let graph = &self.surface().graph;
        let dialogue_id = graph
            .nodes()
            .iter()
            .find(|n| n.archetype == dialogue::START)
            .and_then(|n| n.literals.get(&0))
            .map(|s| s.trim().to_owned())
            .unwrap_or_default();
        let entry = match self.surface().selection.ids().as_slice() {
            [one] => dialogue::node_ids(graph)
                .get(one)
                .cloned()
                .unwrap_or_default(),
            _ => String::new(),
        };
        let setup = self.authoring.story.preview.as_ref().map_or_else(
            || self.authoring.story.last_setup.clone(),
            |p| p.setup.clone(),
        );
        let mut preview = Preview {
            story: Story::new(Arc::clone(&database), 1),
            database,
            conversation: None,
            dialogue: dialogue_id,
            entry,
            log: Vec::new(),
            error: None,
            setup,
            setup_edit: None,
            followed: None,
            notices: Vec::new(),
        };
        Self::run_preview(&mut preview);
        self.authoring.status = match &preview.error {
            Some(error) => error.clone(),
            None => format!(
                "Playing {} from {} · Space: next · 1-9: choose · Esc: stop",
                preview.dialogue,
                if preview.entry.is_empty() {
                    "its entry"
                } else {
                    preview.entry.as_str()
                }
            ),
        };
        self.authoring.error = preview.error.is_some();
        self.authoring.story.last_setup.clone_from(&preview.setup);
        self.authoring.story.preview = Some(preview);
        for input in std::mem::take(&mut self.authoring.story.script) {
            self.preview_input(widget, input);
        }
        self.follow_preview(widget);
    }

    fn run_preview(preview: &mut Preview) {
        preview.story = Story::new(Arc::clone(&preview.database), 1);
        preview.log.clear();
        preview.error = None;
        if let Err(error) = preview.story.run(&preview.setup) {
            preview.error = Some(format!("Before: {error}"));
        }
        preview.story.drain_events();
        match Conversation::start(&mut preview.story, &preview.dialogue, &preview.entry) {
            Ok(conversation) => preview.conversation = Some(conversation),
            Err(error) => {
                preview.conversation = None;
                preview.error = Some(error);
            }
        }
        Self::drain_preview(preview);
    }

    fn drain_preview(preview: &mut Preview) {
        for event in preview.story.drain_events() {
            let story = &preview.story;
            let title = |quest: &str| {
                story
                    .database()
                    .quest(quest)
                    .map_or_else(|| quest.to_owned(), |q| story.text(&q.asset.title, None))
            };
            let notice = match &event {
                StoryEvent::QuestStarted { quest } => Some(("QUEST STARTED", title(quest))),
                StoryEvent::QuestCompleted { quest } => Some(("QUEST COMPLETED", title(quest))),
                StoryEvent::QuestFailed { quest } => Some(("QUEST FAILED", title(quest))),
                StoryEvent::ObjectiveDone { quest, objective } => story
                    .objectives(quest)
                    .into_iter()
                    .find(|o| o.id == *objective)
                    .map(|o| ("OBJECTIVE DONE", o.text)),
                _ => None,
            };
            if let Some((heading, text)) = notice {
                preview.notices.push((heading.to_owned(), text));
            }
            let line = match event {
                StoryEvent::FactChanged { fact, new, .. } => format!("{fact} = {new}"),
                StoryEvent::Command { name, args } => format!(
                    "{name}({})",
                    args.iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                StoryEvent::QuestStarted { quest } => format!("quest {quest} started"),
                StoryEvent::QuestStage { quest, stage } => format!("{quest}: stage {stage}"),
                StoryEvent::QuestCompleted { quest } => format!("quest {quest} completed"),
                StoryEvent::QuestFailed { quest } => format!("quest {quest} failed"),
                StoryEvent::QuestReset { quest } => format!("quest {quest} reset"),
                StoryEvent::ObjectiveDone { quest, objective } => {
                    format!("{quest}/{objective} done")
                }
                StoryEvent::ObjectiveFailed { quest, objective } => {
                    format!("{quest}/{objective} failed")
                }
                StoryEvent::JournalAdded { sequence } => format!("journal entry {sequence}"),
                StoryEvent::JournalObscured { note, obscured } => {
                    format!("{note} {}", if obscured { "obscured" } else { "restored" })
                }
                StoryEvent::Problem { message } => {
                    preview.error = Some(message.clone());
                    format!("problem: {message}")
                }
            };
            preview.log.push(line);
        }
        let excess = preview.log.len().saturating_sub(64);
        preview.log.drain(..excess);
    }

    /// Highlight (and bring into view) the node the play-through stands at.
    fn follow_preview(&mut self, widget: &Widget) {
        let Some(preview) = &self.authoring.story.preview else {
            return;
        };
        let node = preview.conversation.as_ref().and_then(|c| {
            let (dialogue_id, node) = c.position();
            (dialogue_id == preview.dialogue && !c.is_over())
                .then(|| dialogue::find_node(&self.surface().graph, node))
                .flatten()
        });
        let changed = node != preview.followed;
        if let Some(preview) = &mut self.authoring.story.preview {
            preview.followed = node;
        }
        if changed && let Some(node) = node {
            let keep = self.surface().selection.clone();
            self.reveal(widget, node);
            self.surface_mut().selection = keep;
        }
    }

    fn preview_act(
        &mut self,
        widget: &Widget,
        act: impl FnOnce(&mut Conversation, &mut Story) -> Result<(), String>,
    ) {
        let Some(preview) = self.authoring.story.preview.as_mut() else {
            return;
        };
        if let Some(conversation) = preview.conversation.as_mut() {
            if let Err(error) = act(conversation, &mut preview.story) {
                preview.error = Some(error);
            }
            Self::drain_preview(preview);
        }
        self.follow_preview(widget);
    }

    fn preview_buttons(widget: &Widget) -> [Rect; 3] {
        let panel = Self::preview_rect(widget);
        [
            Rect::new(panel.x + panel.w - 236.0, panel.y + 7.0, 90.0, 22.0),
            Rect::new(panel.x + panel.w - 140.0, panel.y + 7.0, 66.0, 22.0),
            Rect::new(panel.x + panel.w - 68.0, panel.y + 7.0, 58.0, 22.0),
        ]
    }

    /// Tell the host what the Game View should show, when that changed.
    pub(super) fn emit_story_beat(&mut self, widget: &Widget, emit: &mut Vec<UiMessage>) {
        let game_view = self.authoring.story.game_view;
        let beat = self
            .authoring
            .story
            .preview
            .as_ref()
            .filter(|_| game_view)
            .and_then(|p| p.conversation.as_ref())
            .map(|c| c.beat().clone());
        if beat != self.authoring.story.sent {
            self.authoring.story.sent.clone_from(&beat);
            emit.push(UiMessage::new(
                widget.handle,
                MessageDirection::FromWidget,
                GraphEditorMessage::PreviewBeat(beat),
            ));
        }
        // After the beat, so the notices stack over the presenter the beat made.
        if let Some(preview) = self.authoring.story.preview.as_mut() {
            for (heading, text) in preview.notices.drain(..) {
                if game_view {
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        GraphEditorMessage::PreviewNotice { heading, text },
                    ));
                }
            }
        }
    }

    /// The Game View's presenter moved the play-through.
    pub(super) fn preview_input(&mut self, widget: &Widget, input: PreviewInput) {
        match input {
            PreviewInput::Next => {
                let beat = self
                    .authoring
                    .story
                    .preview
                    .as_ref()
                    .and_then(|p| p.conversation.as_ref().map(|c| c.beat().clone()));
                match beat {
                    Some(Beat::Line(_)) => self.preview_act(widget, |c, s| c.advance(s)),
                    Some(Beat::Wait { .. }) => self.preview_act(widget, |c, s| c.resume(s)),
                    _ => {}
                }
            }
            PreviewInput::Choose(index) => self.preview_act(widget, |c, s| c.choose(s, index)),
            PreviewInput::Leave => self.authoring.story.game_view = false,
        }
    }

    fn preview_setup_rect(widget: &Widget) -> Rect {
        let panel = Self::preview_rect(widget);
        Rect::new(panel.x + 10.0, panel.y + 36.0, panel.w * 0.66 - 16.0, 20.0)
    }

    fn preview_choice_rects(widget: &Widget, count: usize, text_lines: usize) -> Vec<Rect> {
        let panel = Self::preview_rect(widget);
        let top = panel.y + 84.0 + text_lines.min(3) as f32 * LINE_H + 6.0;
        (0..count)
            .map(|i| {
                Rect::new(
                    panel.x + 10.0,
                    top + i as f32 * 21.0,
                    panel.w * 0.66 - 16.0,
                    20.0,
                )
            })
            .collect()
    }

    fn handle_preview(
        &mut self,
        widget: &mut Widget,
        message: &WidgetMessage,
        emit: &mut Vec<UiMessage>,
    ) -> bool {
        let panel = Self::preview_rect(widget);
        let editing_setup = self
            .authoring
            .story
            .preview
            .as_ref()
            .is_some_and(|p| p.setup_edit.is_some());
        if editing_setup {
            let preview = self.authoring.story.preview.as_mut().expect("checked");
            let draft = preview.setup_edit.as_mut().expect("checked");
            match message {
                WidgetMessage::Text(text) => draft.push_str(text),
                WidgetMessage::KeyDown(crate::message::KeyCode::Backspace, _) => {
                    draft.pop();
                }
                WidgetMessage::KeyDown(crate::message::KeyCode::Escape, _) => {
                    preview.setup_edit = None
                }
                WidgetMessage::KeyDown(
                    crate::message::KeyCode::Enter | crate::message::KeyCode::NumpadEnter,
                    _,
                )
                | WidgetMessage::Unfocus => {
                    preview.setup = preview.setup_edit.take().unwrap_or_default();
                    self.authoring.story.last_setup.clone_from(&preview.setup);
                    Self::run_preview(preview);
                    self.follow_preview(widget);
                }
                WidgetMessage::MouseDown { pos, .. }
                    if !Self::preview_setup_rect(widget).contains(*pos) =>
                {
                    preview.setup = preview.setup_edit.take().unwrap_or_default();
                    Self::run_preview(preview);
                    return false;
                }
                _ => {}
            }
            return true;
        }
        let beat = self
            .authoring
            .story
            .preview
            .as_ref()
            .and_then(|p| p.conversation.as_ref().map(|c| c.beat().clone()));
        let text_lines = match &beat {
            Some(Beat::Line(line)) => wrap(&plain(&line.text), panel.w * 0.66 - 24.0).len(),
            _ => 1,
        };
        let choice_count = match &beat {
            Some(Beat::Choices { choices, .. }) => choices.len(),
            _ => 0,
        };
        let go_on = |editor: &mut Self, widget: &Widget| {
            let beat = editor
                .authoring
                .story
                .preview
                .as_ref()
                .and_then(|p| p.conversation.as_ref().map(|c| c.beat().clone()));
            match beat {
                Some(Beat::Line(_)) => editor.preview_act(widget, |c, s| c.advance(s)),
                Some(Beat::Wait { .. }) => editor.preview_act(widget, |c, s| c.resume(s)),
                Some(Beat::End { .. }) | None => {
                    if let Some(preview) = editor.authoring.story.preview.as_mut() {
                        Self::run_preview(preview);
                    }
                    editor.follow_preview(widget);
                }
                Some(Beat::Choices { .. }) => {}
            }
        };
        match message {
            WidgetMessage::MouseDown {
                pos,
                button: MouseButton::Left,
                ..
            } if panel.contains(*pos) => {
                self.commit_literal(widget, emit);
                let [game_view, restart, stop] = Self::preview_buttons(widget);
                if game_view.contains(*pos) {
                    self.authoring.story.game_view = !self.authoring.story.game_view;
                } else if restart.contains(*pos) {
                    if let Some(preview) = self.authoring.story.preview.as_mut() {
                        Self::run_preview(preview);
                    }
                    self.follow_preview(widget);
                } else if stop.contains(*pos) {
                    self.authoring.story.preview = None;
                    self.authoring.status = "Play-through stopped".into();
                    self.authoring.error = false;
                } else if Self::preview_setup_rect(widget).contains(*pos) {
                    if let Some(preview) = self.authoring.story.preview.as_mut() {
                        preview.setup_edit = Some(preview.setup.clone());
                    }
                } else if let Some(index) =
                    Self::preview_choice_rects(widget, choice_count, text_lines)
                        .iter()
                        .position(|r| r.contains(*pos))
                {
                    self.preview_act(widget, |c, s| c.choose(s, index));
                } else {
                    go_on(self, widget);
                }
                true
            }
            WidgetMessage::MouseDown { pos, .. }
            | WidgetMessage::MouseUp { pos, .. }
            | WidgetMessage::MouseWheel { pos, .. }
                if panel.contains(*pos) =>
            {
                true
            }
            WidgetMessage::KeyDown(key, _)
                if self.literal_edit.is_none() && self.authoring.palette.is_none() =>
            {
                use crate::message::KeyCode as K;
                let digit = match key {
                    K::Digit1 | K::Numpad1 => Some(0),
                    K::Digit2 | K::Numpad2 => Some(1),
                    K::Digit3 | K::Numpad3 => Some(2),
                    K::Digit4 | K::Numpad4 => Some(3),
                    K::Digit5 | K::Numpad5 => Some(4),
                    K::Digit6 | K::Numpad6 => Some(5),
                    K::Digit7 | K::Numpad7 => Some(6),
                    K::Digit8 | K::Numpad8 => Some(7),
                    K::Digit9 | K::Numpad9 => Some(8),
                    _ => None,
                };
                if let Some(index) = digit {
                    if index < choice_count {
                        self.preview_act(widget, |c, s| c.choose(s, index));
                    }
                    return true;
                }
                match key {
                    K::Space | K::Enter | K::NumpadEnter => {
                        go_on(self, widget);
                        true
                    }
                    K::Escape => {
                        self.authoring.story.preview = None;
                        self.authoring.status = "Play-through stopped".into();
                        self.authoring.error = false;
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    // --- drawing ---------------------------------------------------------------------------------------------

    pub(super) fn draw_story(&self, widget: &Widget, ctx: &mut DrawingContext) {
        self.draw_inspector(widget, ctx);
        if self.is_story() {
            self.draw_preview(widget, ctx);
            self.draw_find(widget, ctx);
        }
    }

    fn panel(ctx: &mut DrawingContext, rect: Rect) {
        let theme = crate::theme::active();
        ctx.push_primitive(
            Primitive::fill(rect, theme.semantic.surface.panel.bytes())
                .with_radius(theme.geometry.radius_tile)
                .with_border(1.0, theme.semantic.border.default.bytes()),
            None,
        );
    }

    fn draw_inspector(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let Some(rect) = self.inspector_rect(widget) else {
            return;
        };
        let Some(id) = self.inspector_node() else {
            return;
        };
        let surface = self.surface();
        let Some(node) = surface.graph.node(id) else {
            return;
        };
        let Some(archetype) = surface.catalogue.get(&node.archetype) else {
            return;
        };
        let theme = crate::theme::active();
        Self::panel(ctx, rect);
        let title = if node.title.is_empty() {
            archetype.title.to_owned()
        } else {
            format!("{} · {}", archetype.title, node.title)
        };
        ctx.push_text(
            &title,
            Vec2::new(rect.x + 12.0, rect.y + 11.0),
            self.font_id,
            12.0,
            theme.semantic.text.primary.bytes(),
        );
        if let Some(marks) = self.authoring.story.problems.get(&id) {
            let colour = if marks.iter().any(|(_, w)| !w) {
                theme.semantic.status.error.bytes()
            } else {
                theme.semantic.status.warning.bytes()
            };
            ctx.push_text(
                &format!("{} problem(s) · hover the node", marks.len()),
                Vec2::new(rect.x + 12.0, rect.y + 25.0),
                self.font_id,
                9.0,
                colour,
            );
        }
        let body = Rect::new(rect.x, rect.y + 36.0, rect.w, rect.h - 38.0);
        ctx.push_clip_rect(body);
        for field in self.inspector_fields(rect) {
            ctx.push_text(
                &field.label,
                Vec2::new(field.rect.x, field.label_y + 1.0),
                self.font_id,
                10.0,
                theme.semantic.text.secondary.bytes(),
            );
            let editing = self
                .literal_edit
                .as_ref()
                .is_some_and(|e| e.node == id && e.pin == field.pin);
            ctx.push_primitive(
                Primitive::fill(field.rect, theme.semantic.surface.input.bytes())
                    .with_radius(theme.geometry.radius_input)
                    .with_border(
                        if editing { 2.0 } else { 1.0 },
                        if editing {
                            theme.semantic.border.focus.bytes()
                        } else {
                            theme.semantic.border.subtle.bytes()
                        },
                    ),
                None,
            );
            ctx.push_clip_rect(field.rect);
            for (row, line) in field.lines.iter().enumerate() {
                ctx.push_text(
                    line,
                    Vec2::new(field.rect.x + 5.0, field.rect.y + 4.0 + row as f32 * LINE_H),
                    self.font_id,
                    11.0,
                    if field.wired {
                        theme.semantic.text.muted.bytes()
                    } else {
                        theme.semantic.text.primary.bytes()
                    },
                );
            }
            ctx.pop_clip_rect();
            for (row, line) in field.help_lines.iter().enumerate() {
                ctx.push_text(
                    line,
                    Vec2::new(
                        field.rect.x,
                        field.rect.y + field.rect.h + 3.0 + row as f32 * 12.0,
                    ),
                    self.font_id,
                    9.0,
                    theme.semantic.text.muted.bytes(),
                );
            }
            let _ = (&field.value, field.help);
        }
        ctx.pop_clip_rect();
        ctx.push_text(
            "Enter commits · Shift+Enter: new line in long text · Esc cancels",
            Vec2::new(rect.x + 12.0, rect.y + rect.h - 16.0),
            self.font_id,
            8.5,
            theme.semantic.text.muted.bytes(),
        );
    }

    fn draw_find(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let Some(find) = &self.authoring.story.find else {
            return;
        };
        let theme = crate::theme::active();
        let rect = Self::find_rect(widget);
        ctx.push_primitive(
            Primitive::fill(rect, theme.semantic.surface.popup.bytes())
                .with_radius(theme.geometry.radius_input)
                .with_border(1.0, theme.semantic.border.focus.bytes()),
            None,
        );
        ctx.push_text(
            &format!("Find node: {}|", find.query),
            Vec2::new(rect.x + 8.0, rect.y + 10.0),
            self.font_id,
            11.0,
            theme.semantic.text.primary.bytes(),
        );
        let results = self.find_results();
        let rows = ((rect.h - 32.0) / 22.0).floor().max(0.0) as usize;
        let first = find
            .offset
            .saturating_sub(rows.saturating_sub(1).min(find.offset));
        ctx.push_clip_rect(rect);
        for (row, (_, label)) in results.iter().enumerate().skip(first).take(rows) {
            let y = rect.y + 32.0 + (row - first) as f32 * 22.0;
            if row == find.offset {
                ctx.push_primitive(
                    Primitive::fill(
                        Rect::new(rect.x + 4.0, y - 3.0, rect.w - 8.0, 20.0),
                        theme.semantic.accent.selected_bg.bytes(),
                    )
                    .with_radius(theme.geometry.radius_input),
                    None,
                );
            }
            let limit = ((rect.w - 16.0) / CHAR_W) as usize;
            let mut text: String = label.chars().take(limit).collect();
            if label.chars().count() > limit {
                text.push('…');
            }
            ctx.push_text(
                &text,
                Vec2::new(rect.x + 8.0, y),
                self.font_id,
                11.0,
                theme.semantic.text.primary.bytes(),
            );
        }
        if results.is_empty() {
            ctx.push_text(
                "No node matches",
                Vec2::new(rect.x + 8.0, rect.y + 34.0),
                self.font_id,
                11.0,
                theme.semantic.text.secondary.bytes(),
            );
        }
        ctx.pop_clip_rect();
    }

    fn draw_preview(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let Some(preview) = &self.authoring.story.preview else {
            return;
        };
        let theme = crate::theme::active();
        let panel = Self::preview_rect(widget);
        Self::panel(ctx, panel);
        let from = if preview.entry.is_empty() {
            "its entry"
        } else {
            preview.entry.as_str()
        };
        ctx.push_text(
            &format!("Play-through · {} from {from}", preview.dialogue),
            Vec2::new(panel.x + 10.0, panel.y + 11.0),
            self.font_id,
            12.0,
            theme.semantic.text.primary.bytes(),
        );
        for (rect, label) in Self::preview_buttons(widget).into_iter().zip([
            if self.authoring.story.game_view {
                "Game View ✓"
            } else {
                "Game View"
            },
            "Restart",
            "Stop",
        ]) {
            ctx.push_primitive(
                Primitive::fill(rect, theme.semantic.surface.popup.bytes())
                    .with_radius(theme.geometry.radius_input)
                    .with_border(1.0, theme.semantic.border.default.bytes()),
                None,
            );
            ctx.push_text(
                label,
                Vec2::new(rect.x + 9.0, rect.y + 5.0),
                self.font_id,
                11.0,
                theme.semantic.text.primary.bytes(),
            );
        }
        let setup = Self::preview_setup_rect(widget);
        ctx.push_primitive(
            Primitive::fill(setup, theme.semantic.surface.input.bytes()).with_border(
                if preview.setup_edit.is_some() {
                    2.0
                } else {
                    1.0
                },
                if preview.setup_edit.is_some() {
                    theme.semantic.border.focus.bytes()
                } else {
                    theme.semantic.border.subtle.bytes()
                },
            ),
            None,
        );
        let setup_text = match &preview.setup_edit {
            Some(draft) => format!("Before: {draft}|"),
            None if preview.setup.is_empty() => {
                "Before: (click to set facts first, e.g. lang.patter = 'fluent'; skill.mending = 4)"
                    .to_owned()
            }
            None => format!("Before: {}", preview.setup),
        };
        ctx.push_clip_rect(setup);
        ctx.push_text(
            &setup_text,
            Vec2::new(setup.x + 5.0, setup.y + 4.0),
            self.font_id,
            10.0,
            if preview.setup.is_empty() && preview.setup_edit.is_none() {
                theme.semantic.text.muted.bytes()
            } else {
                theme.semantic.text.primary.bytes()
            },
        );
        ctx.pop_clip_rect();
        let left = Rect::new(
            panel.x + 10.0,
            panel.y + 62.0,
            panel.w * 0.66 - 16.0,
            panel.h - 70.0,
        );
        ctx.push_clip_rect(left);
        let text_colour = theme.semantic.text.primary.bytes();
        match preview.conversation.as_ref().map(Conversation::beat) {
            Some(Beat::Line(line)) => {
                let who = if line.name.is_empty() {
                    "Narrator".to_owned()
                } else {
                    line.name.clone()
                };
                let detail = [line.expression.as_str(), line.language.as_str()]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ");
                ctx.push_text(
                    &if detail.is_empty() {
                        who
                    } else {
                        format!("{who} ({detail})")
                    },
                    Vec2::new(left.x, left.y),
                    self.font_id,
                    11.5,
                    line.color.unwrap_or(theme.semantic.accent.default.bytes()),
                );
                for (row, text) in wrap(&plain(&line.text), left.w - 8.0)
                    .iter()
                    .take(6)
                    .enumerate()
                {
                    ctx.push_text(
                        text,
                        Vec2::new(left.x, left.y + 18.0 + row as f32 * LINE_H),
                        self.font_id,
                        11.0,
                        text_colour,
                    );
                }
                ctx.push_text(
                    if line.menu_next {
                        "Space: the choices"
                    } else {
                        "Space or click: next"
                    },
                    Vec2::new(left.x, left.y + left.h - 14.0),
                    self.font_id,
                    9.0,
                    theme.semantic.text.muted.bytes(),
                );
            }
            Some(Beat::Choices {
                choices,
                timeout_ms,
                ..
            }) => {
                ctx.push_text(
                    &match timeout_ms {
                        Some(ms) => format!(
                            "Choose (1-{}) · timed: {:.1} s",
                            choices.len(),
                            *ms as f32 / 1000.0
                        ),
                        None => format!("Choose (1-{})", choices.len()),
                    },
                    Vec2::new(left.x, left.y),
                    self.font_id,
                    11.0,
                    theme.semantic.text.secondary.bytes(),
                );
                let rects = Self::preview_choice_rects(widget, choices.len(), 1);
                for (choice, rect) in choices.iter().zip(rects) {
                    let check = choice
                        .check
                        .as_ref()
                        .map(|c| match (c.passes, c.chance) {
                            (_, Some(chance)) => format!("  [{} · {chance}%]", c.label),
                            (Some(true), _) => format!("  [{} ✓]", c.label),
                            _ => format!("  [{}]", c.label),
                        })
                        .unwrap_or_default();
                    let taken = if choice.taken { "  (taken before)" } else { "" };
                    ctx.push_text(
                        &format!(
                            "{}. {}{check}{taken}",
                            choice.index + 1,
                            plain(&choice.text)
                        ),
                        Vec2::new(rect.x + 4.0, rect.y + 3.0),
                        self.font_id,
                        11.0,
                        if choice.available {
                            text_colour
                        } else {
                            theme.semantic.text.muted.bytes()
                        },
                    );
                }
            }
            Some(Beat::Wait { node, .. }) => ctx.push_text(
                &format!("{node} is waiting for the game · Space: resume"),
                Vec2::new(left.x, left.y),
                self.font_id,
                11.0,
                theme.semantic.text.secondary.bytes(),
            ),
            Some(Beat::End { result }) => ctx.push_text(
                &if result.is_empty() {
                    "The end · Space: play again".to_owned()
                } else {
                    format!("The end · result: {result} · Space: play again")
                },
                Vec2::new(left.x, left.y),
                self.font_id,
                11.0,
                theme.semantic.text.secondary.bytes(),
            ),
            None => {}
        }
        if let Some(error) = &preview.error {
            ctx.push_text(
                error,
                Vec2::new(left.x, left.y + left.h - 28.0),
                self.font_id,
                10.0,
                theme.semantic.status.error.bytes(),
            );
        }
        if let Some(check) = preview
            .conversation
            .as_ref()
            .and_then(Conversation::last_check)
        {
            let roll = check
                .roll
                .map(|(die, total)| format!(" (rolled {die}, total {total})"))
                .unwrap_or_default();
            ctx.push_text(
                &format!(
                    "Last check {}{roll}",
                    if check.passed { "passed" } else { "failed" }
                ),
                Vec2::new(left.x + left.w * 0.5, left.y + left.h - 14.0),
                self.font_id,
                9.0,
                if check.passed {
                    theme.semantic.status.success.bytes()
                } else {
                    theme.semantic.status.warning.bytes()
                },
            );
        }
        ctx.pop_clip_rect();
        let log = Rect::new(
            panel.x + panel.w * 0.66,
            panel.y + 36.0,
            panel.w * 0.34 - 10.0,
            panel.h - 44.0,
        );
        ctx.push_rect_filled(
            Rect::new(log.x - 6.0, log.y, 1.0, log.h),
            theme.semantic.border.subtle.bytes(),
        );
        ctx.push_text(
            "What changed",
            Vec2::new(log.x, log.y),
            self.font_id,
            10.0,
            theme.semantic.text.secondary.bytes(),
        );
        let rows = ((log.h - 18.0) / 13.0).floor().max(0.0) as usize;
        ctx.push_clip_rect(log);
        for (row, line) in preview.log.iter().rev().take(rows).rev().enumerate() {
            ctx.push_text(
                line,
                Vec2::new(log.x, log.y + 16.0 + row as f32 * 13.0),
                self.font_id,
                9.5,
                theme.semantic.text.primary.bytes(),
            );
        }
        ctx.pop_clip_rect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_story::database::StorySources;

    const DIALOGUE: &str = r#"{
        "version": 1, "id": "intro", "title": "Intro",
        "nodes": [
            {"type": "line", "id": "n1", "speaker": "mira", "text": {"key": "kept.key", "source": "You're late."},
             "effects": "met.mira = true", "next": ["o1", "o2"]},
            {"type": "option", "id": "o1", "text": "Sorry.", "next": ["n2"]},
            {"type": "option", "id": "o2", "text": "[Patter] Late?", "check": {"category": "language", "fact": "lang.patter", "at_least": "fluent"}, "next": ["n2"]},
            {"type": "line", "id": "n2", "speaker": "mira", "text": "Come.", "next": ["e1"]},
            {"type": "end", "id": "e1", "result": "met"}
        ]}"#;

    const MANIFEST: &str = r#"{
        "version": 1, "id": "test", "facts": [
            {"id": "met.mira", "type": "bool"},
            {"id": "lang.patter", "type": "enum", "values": ["none", "phrases", "fluent"]}],
        "cast": [{"id": "mira", "name": "Mira", "portrait": "portraits/mira"}]
    }"#;

    fn editor() -> (GraphEditor, Widget) {
        let mut editor = GraphEditor {
            document: EditorDocument::Graph(super::super::super::scatter::default_surface()),
            gesture: Gesture::None,
            literal_edit: None,
            selected_transition: None,
            transition_edit: None,
            transition_error: None,
            font_id: 0,
            authoring: authoring::AuthoringToolbar::default(),
        };
        editor.open_story(
            dialogue::CATALOGUE,
            DIALOGUE,
            Some("story/intro.somdialogue".into()),
        );
        let mut widget = WidgetBuilder::new().build();
        widget.actual_local_size = Vec2::new(1280.0, 800.0);
        (editor, widget)
    }

    fn saved(editor: &mut GraphEditor, widget: &Widget, apply: bool) -> Option<String> {
        let mut emit = Vec::new();
        editor.request_story_document(widget, apply, false, &mut emit);
        emit.iter()
            .find_map(|m| match m.data::<GraphEditorMessage>() {
                Some(GraphEditorMessage::Document { json, .. }) => Some(json.clone()),
                _ => None,
            })
    }

    fn click(rect: Rect) -> WidgetMessage {
        WidgetMessage::MouseDown {
            pos: rect.pos() + Vec2::new(8.0, 6.0),
            button: MouseButton::Left,
            mods: Default::default(),
        }
    }

    fn key(key: crate::message::KeyCode) -> WidgetMessage {
        WidgetMessage::KeyDown(key, Default::default())
    }

    fn beat(editor: &GraphEditor) -> Beat {
        editor
            .authoring
            .story
            .preview
            .as_ref()
            .and_then(|p| p.conversation.as_ref())
            .map(|c| c.beat().clone())
            .expect("a play-through is running")
    }

    #[test]
    fn a_story_asset_opens_and_saves_back_losslessly() {
        let (mut editor, widget) = editor();
        assert!(
            editor.is_story() && !editor.authoring.error,
            "{}",
            editor.authoring.status
        );
        let json = saved(&mut editor, &widget, false).expect("a save emits the asset");
        let back = somnium_story::DialogueAsset::from_json(&json).unwrap();
        let mut original = somnium_story::DialogueAsset::from_json(DIALOGUE).unwrap();
        for (saved, original) in back.nodes.iter().zip(original.nodes.iter_mut()) {
            original.set_pos(saved.pos());
        }
        assert_eq!(back, original, "keys and checks survive the graph");
        assert_eq!(
            editor
                .authoring
                .sources
                .get(dialogue::CATALOGUE)
                .map(String::as_str),
            Some("story/intro.somdialogue")
        );
    }

    #[test]
    fn the_inspector_edits_long_text_and_renames_nodes() {
        let (mut editor, mut widget) = editor();
        let line = dialogue::find_node(&editor.surface().graph, "n2").unwrap();
        editor.surface_mut().selection.select_only(line);
        let rect = editor
            .inspector_rect(&widget)
            .expect("a line opens the inspector");
        let fields = editor.inspector_fields(rect);
        let labels: Vec<&str> = fields.iter().map(|f| f.label.as_str()).collect();
        assert_eq!(labels[..5], ["Id", "Speaker", "Expression", "Text", "When"]);
        assert!(
            labels.contains(&"Voice") && labels.contains(&"Note"),
            "inspector-only fields are here"
        );
        let text = fields.iter().find(|f| f.label == "Text").unwrap();
        assert!(text.lines.len() >= 3, "long text gets room");
        let mut emit = Vec::new();
        assert!(editor.handle_story_input(&mut widget, &click(text.rect), &mut emit));
        let edit = editor.literal_edit.as_mut().expect("editing the text");
        edit.draft.push_str(" Now.\nQuietly.");
        assert!(editor.commit_literal(&widget, &mut emit));
        let node = editor.surface().graph.node(line).unwrap();
        assert_eq!(node.literals[&3], "Come. Now.\nQuietly.");
        let id = editor
            .inspector_fields(rect)
            .into_iter()
            .find(|f| f.label == "Id")
            .unwrap();
        assert!(editor.handle_story_input(&mut widget, &click(id.rect), &mut emit));
        editor.literal_edit.as_mut().unwrap().draft = "come_along".into();
        assert!(editor.commit_literal(&widget, &mut emit));
        assert_eq!(
            editor.surface().graph.node(line).unwrap().title,
            "come_along"
        );
        let asset =
            somnium_story::DialogueAsset::from_json(&saved(&mut editor, &widget, false).unwrap())
                .unwrap();
        assert!(asset.node("come_along").is_some());
        assert_eq!(
            asset.node("o1").unwrap().links()[0].1,
            ["come_along"],
            "wires follow a rename"
        );
    }

    #[test]
    fn find_selects_and_reveals_a_node() {
        let (mut editor, mut widget) = editor();
        editor.toggle_find();
        editor.handle_story_input(
            &mut widget,
            &WidgetMessage::Text("late".into()),
            &mut Vec::new(),
        );
        let results = editor.find_results();
        assert_eq!(results.len(), 2, "{results:?}");
        editor.handle_story_input(
            &mut widget,
            &key(crate::message::KeyCode::ArrowDown),
            &mut Vec::new(),
        );
        editor.handle_story_input(
            &mut widget,
            &key(crate::message::KeyCode::Enter),
            &mut Vec::new(),
        );
        assert!(editor.authoring.story.find.is_none());
        let o2 = dialogue::find_node(&editor.surface().graph, "o2").unwrap();
        assert_eq!(editor.surface().selection.ids(), [o2]);
    }

    #[test]
    fn messages_reveal_a_node_by_id_and_turn_the_game_view_on() {
        use crate::node::Control;
        let (mut editor, mut widget) = editor();
        for message in [
            GraphEditorMessage::RevealNode("o2".into()),
            GraphEditorMessage::PreviewGameView(true),
        ] {
            let mut msg = UiMessage::new(widget.handle, MessageDirection::ToWidget, message);
            editor.handle_routed_message(&mut widget, &mut msg, &mut Vec::new());
        }
        let o2 = dialogue::find_node(&editor.surface().graph, "o2").unwrap();
        assert_eq!(editor.surface().selection.ids(), [o2]);
        assert!(editor.authoring.story.game_view, "on before any play-through exists");
        let mut msg = UiMessage::new(
            widget.handle,
            MessageDirection::ToWidget,
            GraphEditorMessage::PreviewScript(vec![PreviewInput::Next]),
        );
        editor.handle_routed_message(&mut widget, &mut msg, &mut Vec::new());
        let mut sources = StorySources::default();
        sources.add_file("story.somstory", MANIFEST);
        sources.add_file("intro.somdialogue", &saved(&mut editor, &widget, true).unwrap());
        editor.surface_mut().selection.clear();
        editor.start_preview(&widget, Arc::new(StoryDatabase::build(sources).unwrap().0));
        assert!(matches!(beat(&editor), Beat::Choices { .. }), "the script went on past n1");
        assert!(editor.authoring.story.script.is_empty());
    }

    #[test]
    fn problems_mark_their_nodes_until_the_next_edit() {
        let (mut editor, _) = editor();
        editor.set_story_problems(&[
            ("n1".into(), "mira has no grin expression".into(), false),
            (String::new(), "file-level".into(), true),
        ]);
        let n1 = dialogue::find_node(&editor.surface().graph, "n1").unwrap();
        assert_eq!(
            editor.story_border(n1),
            Some(crate::theme::active().semantic.status.error.bytes())
        );
        assert!(
            editor
                .authoring
                .status
                .starts_with("1 error(s), 1 warning(s)")
        );
        editor.authoring_changed();
        assert_eq!(editor.story_border(n1), None);
    }

    #[test]
    fn the_game_view_gets_each_beat_once_and_drives_the_play_through() {
        let (mut editor, widget) = editor();
        let mut sources = StorySources::default();
        sources.add_file("story.somstory", MANIFEST);
        sources.add_file(
            "intro.somdialogue",
            &saved(&mut editor, &widget, true).unwrap(),
        );
        sources.add_file(
            "lamps.somquest",
            r#"{"version": 1, "id": "lamps", "title": "Relight the lamps",
                "stages": [{"id": "s", "objectives": [{"id": "o", "text": "Light them"}],
                    "next": [{"to": "e"}]}, {"id": "e", "end": "complete"}]}"#,
        );
        let (database, _) = StoryDatabase::build(sources).unwrap_or_else(|i| panic!("{i:?}"));
        editor.start_preview(&widget, Arc::new(database));
        let beats = |editor: &mut GraphEditor| {
            let mut emit = Vec::new();
            editor.emit_story_beat(&widget, &mut emit);
            emit.iter()
                .filter_map(|m| match m.data::<GraphEditorMessage>() {
                    Some(GraphEditorMessage::PreviewBeat(beat)) => Some(beat.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert!(beats(&mut editor).is_empty(), "the Game View is off");
        editor.authoring.story.game_view = true;
        let first = beats(&mut editor);
        assert!(matches!(first.as_slice(), [Some(Beat::Line(l))] if l.node == "n1"));
        assert!(
            beats(&mut editor).is_empty(),
            "the same beat is not sent twice"
        );
        editor.preview_input(&widget, PreviewInput::Next);
        assert!(matches!(
            beats(&mut editor).as_slice(),
            [Some(Beat::Choices { .. })]
        ));
        editor.preview_input(&widget, PreviewInput::Choose(0));
        assert!(matches!(beats(&mut editor).as_slice(), [Some(Beat::Line(l))] if l.node == "n2"));
        // Quest news goes up as a notice, after the beat.
        let preview = editor.authoring.story.preview.as_mut().unwrap();
        preview.story.start_quest("lamps").unwrap();
        GraphEditor::drain_preview(preview);
        let mut emit = Vec::new();
        editor.emit_story_beat(&widget, &mut emit);
        let notices: Vec<_> = emit
            .iter()
            .filter_map(|m| match m.data::<GraphEditorMessage>() {
                Some(GraphEditorMessage::PreviewNotice { heading, text }) => {
                    Some((heading.as_str(), text.as_str()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(notices, [("QUEST STARTED", "Relight the lamps")]);
        editor.preview_input(&widget, PreviewInput::Leave);
        assert_eq!(beats(&mut editor), [None], "leaving closes the presenter");
    }

    #[test]
    fn a_dialogue_plays_from_the_selected_node_with_setup_facts() {
        let (mut editor, mut widget) = editor();
        let mut sources = StorySources::default();
        sources.add_file("story.somstory", MANIFEST);
        let json = saved(&mut editor, &widget, true).expect("Play hands the host the asset");
        sources.add_file("intro.somdialogue", &json);
        let (database, warnings) =
            StoryDatabase::build(sources).unwrap_or_else(|i| panic!("{i:?}"));
        assert!(warnings.is_empty(), "{warnings:?}");
        let database = Arc::new(database);
        editor.start_preview(&widget, Arc::clone(&database));
        let preview = editor.authoring.story.preview.as_ref().unwrap();
        assert!(preview.error.is_none(), "{:?}", preview.error);
        assert!(matches!(beat(&editor), Beat::Line(l) if l.node == "n1"));
        let preview = editor.authoring.story.preview.as_ref().unwrap();
        assert_eq!(
            preview.followed,
            dialogue::find_node(&editor.surface().graph, "n1")
        );
        assert!(
            preview.log.iter().any(|l| l == "met.mira = true"),
            "{:?}",
            preview.log
        );
        assert!(editor.handle_story_input(
            &mut widget,
            &key(crate::message::KeyCode::Space),
            &mut Vec::new()
        ));
        let Beat::Choices { choices, .. } = beat(&editor) else {
            panic!()
        };
        assert!(!choices[1].available, "Patter none of fluent: locked");
        editor.handle_story_input(
            &mut widget,
            &key(crate::message::KeyCode::Digit2),
            &mut Vec::new(),
        );
        assert!(
            matches!(beat(&editor), Beat::Choices { .. }),
            "a locked choice is not taken"
        );
        // The other road: the language set first, played from the selected choice.
        let o2 = dialogue::find_node(&editor.surface().graph, "o2").unwrap();
        editor.surface_mut().selection.select_only(o2);
        editor.authoring.story.preview.as_mut().unwrap().setup = "lang.patter = 'fluent'".into();
        editor.start_preview(&widget, database);
        let Beat::Choices { choices, .. } = beat(&editor) else {
            panic!()
        };
        assert_eq!(choices.len(), 1);
        assert!(choices[0].available);
        editor.handle_story_input(
            &mut widget,
            &key(crate::message::KeyCode::Digit1),
            &mut Vec::new(),
        );
        editor.handle_story_input(
            &mut widget,
            &key(crate::message::KeyCode::Space),
            &mut Vec::new(),
        );
        assert_eq!(
            beat(&editor),
            Beat::End {
                result: "met".into()
            }
        );
        editor.handle_story_input(
            &mut widget,
            &key(crate::message::KeyCode::Escape),
            &mut Vec::new(),
        );
        assert!(editor.authoring.story.preview.is_none());
    }
}
