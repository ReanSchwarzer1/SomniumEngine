//! Native authoring presentation. The host supplies registered entries/actions;
//! this panel never owns scene semantics, credentials, or a second command list.
//!
//! Phase ALYX-C (18 Oct 2026): the panel is laid out as a status card (is an agent connected, when did it last ask
//! something, which revision), icon quick actions, the setup command an MCP host needs, a feed of recent requests,
//! and the host's entries grouped under headed sections. The header refreshes on its own; the body rebuilds only when
//! the actions, entries or activity change, so a status tick never disturbs search or scroll.
use crate::{
    EditorEvent,
    icons::IconId,
    message::{MessageDirection, NodeHandle, TextMessage, UiMessage},
    theme,
    types::{Thickness, VerticalAlignment},
    typography::TextRole,
    ui::UserInterface,
    widget::WidgetBuilder,
    widgets::{
        button::{ButtonBuilder, ButtonMessage},
        image::ImageBuilder,
        scroll_viewer::ScrollViewerBuilder,
        search_box::{SearchBoxBuilder, SearchBoxMessage},
        stack_panel::{Orientation, StackPanelBuilder},
        text::TextBuilder,
        text_box::{TextBoxBuilder, TextBoxMessage},
        wrap_panel::WrapPanelBuilder,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One registered, backend-enabled operation. `params` is public operation
/// data only: never put connection tokens or other credentials in panel state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuthoringAction {
    pub label: String,
    pub method: String,
    pub params: Value,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AuthoringEntry {
    #[serde(default)]
    pub fields: Vec<AuthoringField>,
    pub id: String,
    pub label: String,
    pub category: String,
    pub detail: String,
    pub actions: Vec<AuthoringAction>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuthoringField {
    pub pointer: String,
    pub label: String,
    pub value: Value,
}

/// Whether an agent is on the line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthoringStatus {
    /// The pipe is open and nobody is connected.
    #[default]
    Listening,
    /// A client holds the pipe now.
    Connected,
    /// Access was revoked this session; restarting the editor reopens it.
    Revoked,
}

/// One request the editor answered, newest last.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AuthoringActivity {
    /// Local clock time, `HH:MM:SS`.
    pub time: String,
    /// "Agent" for the pipe, "Editor" for this panel.
    pub source: String,
    /// What was asked, short: `execute · capture`.
    pub summary: String,
    pub ok: bool,
    /// The error's message when it failed.
    pub message: String,
}

/// View model shared by private games and ordinary engine projects. Categories
/// may contain presets, documents, capabilities, jobs or history; each entry's
/// allowed operations come from the host's shared authoring interface.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AuthoringState {
    pub enabled: bool,
    pub project: String,
    pub connection: String,
    pub revision: u64,
    pub actions: Vec<AuthoringAction>,
    pub entries: Vec<AuthoringEntry>,
    #[serde(default)]
    pub status: AuthoringStatus,
    /// When the last request arrived (local clock), empty before the first.
    #[serde(default)]
    pub last_request: String,
    #[serde(default)]
    pub activity: Vec<AuthoringActivity>,
    /// The command an MCP host runs to reach this editor (paths only, never the credential).
    #[serde(default)]
    pub setup_command: String,
    /// What agents may do, by preference field (see [`AGENT_PERMISSIONS`]), and whether it is on.
    #[serde(default)]
    pub permissions: Vec<(String, bool)>,
    /// One action per MCP client the editor can set up (Claude Code, Claude Desktop, Cursor, VS Code).
    #[serde(default)]
    pub setup: Vec<AuthoringAction>,
}

/// What an agent may be allowed to do (Phase ALYX-M): the Editor preference that holds it, its label, its glyph.
/// Reading (discover, query, plan, logs, jobs) is always allowed; these gate what changes things.
pub const AGENT_PERMISSIONS: &[(&str, &str, IconId)] = &[
    ("agent_edits", "Edit scenes and assets", IconId::Edit),
    ("agent_files", "Import, save and change files", IconId::File),
    ("agent_play", "Control Play and input", IconId::Play),
    ("agent_captures", "Capture the viewport", IconId::Screenshot),
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandCapability {
    pub id: String,
    pub label: String,
    pub category: String,
    pub help: String,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
}

/// Capture existing command declarations and current enablement; no manual
/// inventory that can drift away from menus/shortcuts is maintained here.
pub fn command_capabilities(context: &crate::commands::EditorCtx) -> Vec<CommandCapability> {
    crate::commands::registry()
        .commands()
        .iter()
        .map(|command| {
            let enabled = command.enabled(context);
            CommandCapability {
                id: command.id.into(),
                label: command.label.into(),
                category: command.category.into(),
                help: command.help.into(),
                enabled: enabled.is_enabled(),
                disabled_reason: enabled.reason().map(str::to_owned),
            }
        })
        .collect()
}

fn permitted_method(method: &str) -> bool {
    matches!(
        method,
        "authoring.discover"
            | "authoring.query"
            | "authoring.plan"
            | "authoring.commit"
            | "authoring.execute"
            | "authoring.history"
            | "authoring.jobs"
    )
}

fn action_event(action: &AuthoringAction) -> Option<EditorEvent> {
    (action.enabled && permitted_method(&action.method) && action.params.is_object()).then(|| {
        EditorEvent::AuthoringRequest {
            method: action.method.clone(),
            params: action.params.clone(),
        }
    })
}

/// The glyph for an action, from its label: the host's labels are the vocabulary.
fn action_icon(label: &str) -> IconId {
    let l = label.to_ascii_lowercase();
    match () {
        _ if l.starts_with("capture") => IconId::Screenshot,
        _ if l.starts_with("play") => IconId::Play,
        _ if l.starts_with("pause") => IconId::Pause,
        _ if l.starts_with("step") => IconId::Step,
        _ if l.starts_with("stop") => IconId::Stop,
        _ if l.starts_with("revoke") => IconId::ShieldX,
        _ if l.starts_with("create") || l.starts_with("add") => IconId::Add,
        _ if l.starts_with("open") => IconId::FolderOpen,
        _ if l.starts_with("select") => IconId::Focus,
        _ if l.starts_with("unprotect") => IconId::Unlocked,
        _ if l.starts_with("protect") => IconId::Locked,
        _ if l.starts_with("cancel") => IconId::Close,
        _ if l.starts_with("save") => IconId::Save,
        _ if l.contains("claude") || l.contains("cursor") || l.contains("vs code") => IconId::Agent,
        _ => IconId::More,
    }
}

/// The glyph for a section, from its category name.
fn category_icon(category: &str) -> IconId {
    let c = category.to_ascii_lowercase();
    match () {
        _ if c.contains("job") => IconId::Activity,
        _ if c.contains("document") => IconId::File,
        _ if c.contains("object") || c.contains("entit") => IconId::Cube,
        _ if c.contains("preset") || c.contains("game") => IconId::Prefab,
        _ if c.contains("history") => IconId::History,
        _ => IconId::Component,
    }
}

pub struct AuthoringPanel {
    pub root: NodeHandle,
    status_host: NodeHandle,
    status: NodeHandle,
    detail: NodeHandle,
    actions_host: NodeHandle,
    search: NodeHandle,
    body: NodeHandle,
    empty: NodeHandle,
    dynamic: Vec<NodeHandle>,
    buttons: Vec<(NodeHandle, AuthoringAction)>,
    copy_button: Option<NodeHandle>,
    /// The permission switches, with the preference each one sets.
    permission_boxes: Vec<(NodeHandle, &'static str)>,
    /// Each entry card, its search text, and the section it sits in.
    rows: Vec<(NodeHandle, String, usize)>,
    sections: Vec<NodeHandle>,
    state: AuthoringState,
    inputs: Vec<(NodeHandle, AuthoringField)>,
    editing: bool,
    query: String,
}

impl AuthoringPanel {
    pub fn build(ui: &mut UserInterface, parent: NodeHandle, font: u8) -> Self {
        let root = column(ui, parent);
        // The card: the bridge, its project, whether an agent is on the line, and when it last asked anything.
        let title_row = row(ui, root, Thickness::new(6.0, 6.0, 6.0, 2.0));
        glyph(ui, title_row, IconId::McpLink, 20.0, theme::active().semantic.accent.default.bytes());
        ui.add_node(
            TextBuilder::new(
                WidgetBuilder::new()
                    .with_margin(Thickness::new(8.0, 0.0, 0.0, 0.0))
                    .with_vertical_alignment(VerticalAlignment::Center),
            )
            .with_role(TextRole::Section)
            .with_text("Authoring bridge")
            .build(),
            title_row,
        );
        let status_host = row(ui, root, Thickness::new(8.0, 2.0, 6.0, 0.0));
        let status = status_line(ui, status_host, AuthoringStatus::Listening);
        let detail = label(ui, root, "No authoring connection", TextRole::Caption);
        let actions_host = ui.add_node(
            WrapPanelBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(6.0, 4.0)))
                .with_gap(4.0, 4.0)
                .build(),
            root,
        );
        let search = ui.add_node(
            SearchBoxBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(6.0, 4.0)))
                .with_font_id(font)
                .build(),
            root,
        );
        let scroll = ui.add_node(
            ScrollViewerBuilder::new(WidgetBuilder::new().with_height(420.0)).build(),
            root,
        );
        let body = column(ui, scroll);
        let empty = label(ui, root, "No matching authoring entries.", TextRole::Caption);
        ui.set_visibility(empty, false);
        ui.set_visibility(root, false);
        Self {
            root,
            status_host,
            status,
            detail,
            actions_host,
            search,
            body,
            empty,
            dynamic: Vec::new(),
            buttons: Vec::new(),
            copy_button: None,
            permission_boxes: Vec::new(),
            rows: Vec::new(),
            sections: Vec::new(),
            state: AuthoringState::default(),
            inputs: vec![],
            editing: false,
            query: String::new(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.state.enabled
    }

    /// Update the card in place, and rebuild the body only when its data changed, preserving the search widget and
    /// scroll container so a job refresh or a status tick does not steal keyboard focus.
    pub fn set_state(&mut self, ui: &mut UserInterface, state: AuthoringState) {
        if self.editing || self.state == state {
            return;
        }
        let body_changed = self.state.actions != state.actions
            || self.state.entries != state.entries
            || self.state.activity != state.activity
            || self.state.setup_command != state.setup_command
            || self.state.permissions != state.permissions
            || self.state.setup != state.setup;
        let status_changed = self.state.status != state.status;
        self.state = state;
        ui.set_visibility(self.root, self.state.enabled);

        if status_changed {
            ui.remove_node(self.status);
            self.status = status_line(ui, self.status_host, self.state.status);
        }
        ui.send(TextMessage::set_text(self.detail, detail_text(&self.state)));
        if body_changed {
            self.rebuild(ui);
        }
    }

    fn rebuild(&mut self, ui: &mut UserInterface) {
        for handle in self.dynamic.drain(..) {
            ui.remove_node(handle);
        }
        self.buttons.clear();
        self.inputs.clear();
        self.rows.clear();
        self.sections.clear();
        self.copy_button = None;
        self.permission_boxes.clear();
        // Quick actions, then the setup command an MCP host needs.
        for action in &self.state.actions {
            let button = action_button(ui, self.actions_host, action);
            self.dynamic.push(button);
            self.buttons.push((button, action.clone()));
        }
        if !self.state.setup_command.is_empty() {
            let copy = icon_button(
                ui,
                self.actions_host,
                IconId::Copy,
                "Copy setup command",
                true,
                "The command an MCP host runs to reach this editor. It holds paths only; the credential stays in \
                 the protected connection file.",
            );
            self.dynamic.push(copy);
            self.copy_button = Some(copy);
        }
        // What agents may do: each switch is the Editor preference of the same name.
        if !self.state.permissions.is_empty() {
            let section = section_header(ui, self.body, IconId::ShieldCheck, "Agents may", self.state.permissions.len());
            self.dynamic.push(section);
            for (key, on) in &self.state.permissions {
                let Some(&(field, text, icon)) = AGENT_PERMISSIONS.iter().find(|(field, _, _)| field == key) else {
                    continue;
                };
                let line = row(ui, self.body, Thickness::new(10.0, 1.0, 6.0, 1.0));
                self.dynamic.push(line);
                glyph(ui, line, icon, 14.0, theme::active().semantic.text.muted.bytes());
                let check = ui.add_node(
                    crate::widgets::check_box::CheckBoxBuilder::new(
                        WidgetBuilder::new()
                            .with_height(22.0)
                            .with_margin(Thickness::new(6.0, 0.0, 0.0, 0.0))
                            .with_vertical_alignment(VerticalAlignment::Center),
                    )
                    .with_label(text)
                    .with_checked(*on)
                    .build(),
                    line,
                );
                self.permission_boxes.push((check, field));
            }
        }
        // Connect an MCP client: each writes this editor's entry into that client's own settings file.
        if !self.state.setup.is_empty() {
            let section = section_header(ui, self.body, IconId::Agent, "Connect an agent", self.state.setup.len());
            self.dynamic.push(section);
            let wrap = ui.add_node(
                WrapPanelBuilder::new(WidgetBuilder::new().with_margin(Thickness::new(10.0, 2.0, 6.0, 4.0)))
                    .with_gap(4.0, 4.0)
                    .build(),
                self.body,
            );
            self.dynamic.push(wrap);
            for action in &self.state.setup {
                let button = action_button(ui, wrap, action);
                self.buttons.push((button, action.clone()));
            }
        }
        // What was asked most recently, newest first.
        if !self.state.activity.is_empty() {
            let section = section_header(ui, self.body, IconId::Activity, "Recent activity", self.state.activity.len());
            self.dynamic.push(section);
            for activity in self.state.activity.iter().rev().take(12) {
                let line = row(ui, self.body, Thickness::new(10.0, 1.0, 6.0, 1.0));
                self.dynamic.push(line);
                let status = theme::active().semantic.status;
                let (icon, tint) = if activity.ok {
                    (IconId::Ok, status.success.bytes())
                } else {
                    (IconId::Error, status.error.bytes())
                };
                glyph(ui, line, icon, 14.0, tint);
                let mut text = format!("{}  {} · {}", activity.time, activity.source, activity.summary);
                if !activity.ok && !activity.message.is_empty() {
                    text.push_str(" — ");
                    text.push_str(&activity.message);
                }
                ui.add_node(
                    TextBuilder::new(
                        WidgetBuilder::new()
                            .with_margin(Thickness::new(6.0, 0.0, 0.0, 0.0))
                            .with_vertical_alignment(VerticalAlignment::Center),
                    )
                    .with_role(TextRole::Caption)
                    .with_text(text)
                    .build(),
                    line,
                );
            }
        }
        // The host's entries, each category under its own header, in the order the host gave them.
        let mut categories: Vec<&str> = Vec::new();
        for entry in &self.state.entries {
            if !categories.contains(&entry.category.as_str()) {
                categories.push(&entry.category);
            }
        }
        for category in categories {
            let count = self.state.entries.iter().filter(|e| e.category == category).count();
            let header = section_header(ui, self.body, category_icon(category), category, count);
            self.dynamic.push(header);
            let section = self.sections.len();
            self.sections.push(header);
            for entry in self.state.entries.iter().filter(|e| e.category == category) {
                let card = ui.add_node(
                    StackPanelBuilder::new(
                        WidgetBuilder::new()
                            .with_background(theme::TRANSPARENT)
                            .with_margin(Thickness::new(10.0, 2.0, 6.0, 4.0)),
                    )
                    .with_orientation(Orientation::Vertical)
                    .build(),
                    self.body,
                );
                self.dynamic.push(card);
                label(ui, card, &entry.label, TextRole::BodyStrong);
                if !entry.detail.is_empty() {
                    label(ui, card, &entry.detail, TextRole::Caption);
                }
                for field in &entry.fields {
                    label(ui, card, &field.label, TextRole::Caption);
                    let text = field
                        .value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| field.value.to_string());
                    let input = ui.add_node(
                        TextBoxBuilder::new(WidgetBuilder::new().with_height(28.0))
                            .with_text(text)
                            .with_commit_on_blur(true)
                            .build(),
                        card,
                    );
                    self.inputs.push((input, field.clone()));
                }
                if !entry.actions.is_empty() {
                    let actions = ui.add_node(
                        WrapPanelBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(4.0, 2.0)))
                            .with_gap(4.0, 4.0)
                            .build(),
                        card,
                    );
                    for action in &entry.actions {
                        let button = action_button(ui, actions, action);
                        self.buttons.push((button, action.clone()));
                    }
                }
                self.rows.push((
                    card,
                    format!("{} {} {} {}", entry.id, entry.label, entry.category, entry.detail).to_lowercase(),
                    section,
                ));
            }
        }
        self.filter(ui);
    }

    fn filter(&self, ui: &mut UserInterface) {
        let query = self.query.to_lowercase();
        let mut visible = 0;
        let mut shown = vec![false; self.sections.len()];
        for (card, search, section) in &self.rows {
            let matches = query.split_whitespace().all(|word| search.contains(word));
            ui.set_visibility(*card, matches);
            if matches {
                visible += 1;
                shown[*section] = true;
            }
        }
        for (header, shown) in self.sections.iter().zip(shown) {
            ui.set_visibility(*header, shown);
        }
        ui.set_visibility(self.empty, visible == 0 && !self.rows.is_empty());
    }

    /// Returns whether the message belongs to this panel and any semantic
    /// request it produced. Disabled actions cannot dispatch even if spoofed.
    pub fn event(
        &mut self,
        ui: &mut UserInterface,
        message: &UiMessage,
    ) -> (bool, Option<EditorEvent>) {
        if message.direction != MessageDirection::FromWidget || !self.state.enabled {
            return (false, None);
        }
        if let Some((_, field)) = self
            .inputs
            .iter()
            .find(|(handle, _)| *handle == message.destination)
        {
            if let Some(TextBoxMessage::TextChanged(_)) = message.data::<TextBoxMessage>() {
                self.editing = true;
                return (true, None);
            }
            if let Some(TextBoxMessage::TextCommit(text)) = message.data::<TextBoxMessage>() {
                self.editing = false;
                let value = if field.value.is_string() {
                    Value::String(text.clone())
                } else {
                    serde_json::from_str(text).unwrap_or(Value::String(text.clone()))
                };
                return (
                    true,
                    Some(EditorEvent::AuthoringRequest {
                        method: "authoring.execute".into(),
                        params: serde_json::json!({"action":"document_edit","pointer":field.pointer,"value":value}),
                    }),
                );
            }
        }
        if let Some(&(_, field)) = self
            .permission_boxes
            .iter()
            .find(|(handle, _)| *handle == message.destination)
            && let Some(crate::widgets::check_box::CheckBoxMessage::Check(on)) =
                message.data::<crate::widgets::check_box::CheckBoxMessage>()
        {
            return (
                true,
                Some(EditorEvent::SetSettingByName {
                    component: somnium_ecs::reflect::StableId::new("somnium.EditorSettings"),
                    field_name: field,
                    value: somnium_ecs::reflect::ReflectValue::Bool(*on),
                }),
            );
        }
        if message.destination == self.search {
            if let Some(SearchBoxMessage::Query(query)) = message.data::<SearchBoxMessage>() {
                self.query = query.clone();
                self.filter(ui);
                return (true, None);
            }
        }
        if matches!(message.data::<ButtonMessage>(), Some(ButtonMessage::Click)) {
            if Some(message.destination) == self.copy_button {
                return (true, Some(EditorEvent::CopyText(self.state.setup_command.clone())));
            }
            if let Some((_, action)) = self
                .buttons
                .iter()
                .find(|(handle, _)| *handle == message.destination)
            {
                return (true, action_event(action));
            }
        }
        (false, None)
    }
}

/// "The Somnus Fracture · Local pipe · current Windows user · revision 12 · last request 09:15:02".
fn detail_text(state: &AuthoringState) -> String {
    let mut text = format!("{} · revision {}", state.connection, state.revision);
    if !state.project.is_empty() {
        text = format!("{} · {text}", state.project);
    }
    if !state.last_request.is_empty() {
        text.push_str(" · last request ");
        text.push_str(&state.last_request);
    }
    text
}

fn status_line(ui: &mut UserInterface, parent: NodeHandle, status: AuthoringStatus) -> NodeHandle {
    let tokens = theme::active().semantic;
    let (icon, tint, text) = match status {
        AuthoringStatus::Connected => (IconId::PlugConnected, tokens.status.success, "Agent connected"),
        AuthoringStatus::Listening => (IconId::Plug, tokens.text.secondary, "Waiting for an agent"),
        AuthoringStatus::Revoked => (IconId::ShieldX, tokens.status.error, "Access revoked; restart to reconnect"),
    };
    let line = ui.add_node(
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Horizontal)
            .build(),
        parent,
    );
    glyph(ui, line, icon, 16.0, tint.bytes());
    ui.add_node(
        TextBuilder::new(
            WidgetBuilder::new()
                .with_margin(Thickness::new(6.0, 0.0, 0.0, 0.0))
                .with_vertical_alignment(VerticalAlignment::Center),
        )
        .with_role(TextRole::Label)
        .with_color(tint.bytes())
        .with_text(text)
        .build(),
        line,
    );
    line
}

fn section_header(ui: &mut UserInterface, parent: NodeHandle, icon: IconId, title: &str, count: usize) -> NodeHandle {
    let header = row(ui, parent, Thickness::new(6.0, 10.0, 6.0, 2.0));
    glyph(ui, header, icon, 14.0, theme::active().semantic.text.muted.bytes());
    ui.add_node(
        TextBuilder::new(
            WidgetBuilder::new()
                .with_margin(Thickness::new(6.0, 0.0, 0.0, 0.0))
                .with_vertical_alignment(VerticalAlignment::Center),
        )
        .with_role(TextRole::SectionCaps)
        .with_text(format!("{title}  {count}"))
        .build(),
        header,
    );
    header
}

fn column(ui: &mut UserInterface, parent: NodeHandle) -> NodeHandle {
    ui.add_node(
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build(),
        parent,
    )
}

fn row(ui: &mut UserInterface, parent: NodeHandle, margin: Thickness) -> NodeHandle {
    ui.add_node(
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT).with_margin(margin))
            .with_orientation(Orientation::Horizontal)
            .build(),
        parent,
    )
}

fn glyph(ui: &mut UserInterface, parent: NodeHandle, icon: IconId, size: f32, tint: [u8; 4]) -> NodeHandle {
    ui.add_node(
        ImageBuilder::new(
            WidgetBuilder::new()
                .with_vertical_alignment(VerticalAlignment::Center)
                .with_hit_test_visibility(false),
        )
        .with_icon(icon)
        .with_size(size)
        .with_tint(tint)
        .build(),
        parent,
    )
}

fn label(ui: &mut UserInterface, parent: NodeHandle, text: &str, role: TextRole) -> NodeHandle {
    ui.add_node(
        TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(6.0, 2.0)))
            .with_text(text)
            .with_role(role)
            .with_wrap(true)
            .build(),
        parent,
    )
}

/// A button with its glyph before its words.
fn icon_button(
    ui: &mut UserInterface,
    parent: NodeHandle,
    icon: IconId,
    text: &str,
    enabled: bool,
    tooltip: &str,
) -> NodeHandle {
    let mut builder = WidgetBuilder::new()
        .with_height(theme::active().density.row_chrome)
        .with_enabled(enabled)
        .with_background(theme::active().semantic.surface.raised.bytes());
    if !tooltip.is_empty() {
        builder = builder.with_tooltip(tooltip);
    }
    let button = ui.add_node(ButtonBuilder::new(builder).build(), parent);
    let inner = row(ui, button, Thickness::axes(8.0, 0.0));
    let ink = if enabled {
        theme::active().semantic.text.primary.bytes()
    } else {
        theme::active().semantic.text.disabled.bytes()
    };
    glyph(ui, inner, icon, 16.0, ink);
    ui.add_node(
        TextBuilder::new(
            WidgetBuilder::new()
                .with_margin(Thickness::new(6.0, 0.0, 0.0, 0.0))
                .with_vertical_alignment(VerticalAlignment::Center)
                .with_hit_test_visibility(false),
        )
        .with_role(TextRole::Label)
        .with_color(ink)
        .with_text(text)
        .build(),
        inner,
    );
    button
}

fn action_button(
    ui: &mut UserInterface,
    parent: NodeHandle,
    action: &AuthoringAction,
) -> NodeHandle {
    let enabled = action_event(action).is_some();
    let tooltip = action.disabled_reason.clone().unwrap_or_default();
    icon_button(ui, parent, action_icon(&action.label), &action.label, enabled, &tooltip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn inventory_is_exactly_the_current_registry_and_preserves_disabled_reasons() {
        let rows = command_capabilities(&crate::commands::EditorCtx::default());
        assert_eq!(rows.len(), crate::commands::registry().commands().len());
        for command in crate::commands::registry().commands() {
            assert_eq!(rows.iter().filter(|row| row.id == command.id).count(), 1);
        }
        assert!(
            rows.iter()
                .any(|row| !row.enabled && row.disabled_reason.is_some())
        );
    }
    #[test]
    fn private_operation_args_survive_and_disabled_actions_cannot_dispatch() {
        let mut action = AuthoringAction {
            label: "Preview".into(),
            method: "authoring.execute".into(),
            params: json!({"command":"private.preview","entity":"persistent-id","expected_revision":7}),
            enabled: true,
            disabled_reason: None,
        };
        let Some(EditorEvent::AuthoringRequest { method, params }) = action_event(&action) else {
            panic!("missing request");
        };
        assert_eq!(method, "authoring.execute");
        assert_eq!(params, action.params);
        action.enabled = false;
        assert!(action_event(&action).is_none());
        action.enabled = true;
        action.method = "shell".into();
        assert!(action_event(&action).is_none());
    }
    #[test]
    fn the_card_says_when_the_last_request_came() {
        let mut state = AuthoringState {
            connection: "Local pipe · current Windows user".into(),
            revision: 12,
            ..Default::default()
        };
        assert_eq!(detail_text(&state), "Local pipe · current Windows user · revision 12");
        state.last_request = "09:15:02".into();
        assert!(detail_text(&state).ends_with("last request 09:15:02"));
    }
    #[test]
    fn host_labels_pick_their_glyphs() {
        assert_eq!(action_icon("Capture"), IconId::Screenshot);
        assert_eq!(action_icon("Revoke MCP"), IconId::ShieldX);
        assert_eq!(action_icon("Select / frame"), IconId::Focus);
        assert_eq!(action_icon("Unprotect"), IconId::Unlocked);
        assert_eq!(category_icon("Jobs"), IconId::Activity);
    }
    #[test]
    fn older_state_without_the_new_fields_still_loads() {
        let state: AuthoringState = serde_json::from_value(json!({"enabled":true,"project":"p","connection":"c",
            "revision":1,"actions":[],"entries":[]}))
        .unwrap();
        assert_eq!(state.status, AuthoringStatus::Listening);
        assert!(state.activity.is_empty());
    }
}
