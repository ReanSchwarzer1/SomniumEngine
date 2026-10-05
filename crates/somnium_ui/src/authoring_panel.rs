//! Native authoring presentation. The host supplies registered entries/actions;
//! this panel never owns scene semantics, credentials, or a second command list.
use crate::{
    EditorEvent,
    message::{MessageDirection, NodeHandle, TextMessage, UiMessage},
    theme,
    types::Thickness,
    typography::TextRole,
    ui::UserInterface,
    widget::WidgetBuilder,
    widgets::{
        button::{ButtonBuilder, ButtonMessage},
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
}

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

pub struct AuthoringPanel {
    pub root: NodeHandle,
    heading: NodeHandle,
    status: NodeHandle,
    search: NodeHandle,
    body: NodeHandle,
    empty: NodeHandle,
    dynamic: Vec<NodeHandle>,
    buttons: Vec<(NodeHandle, AuthoringAction)>,
    rows: Vec<(NodeHandle, String)>,
    state: AuthoringState,
    inputs: Vec<(NodeHandle, AuthoringField)>,
    editing: bool,
    query: String,
}

impl AuthoringPanel {
    pub fn build(ui: &mut UserInterface, parent: NodeHandle, font: u8) -> Self {
        let root = column(ui, parent);
        let heading = label(ui, root, "Game / Authoring", TextRole::BodyStrong);
        let status = label(ui, root, "No authoring connection", TextRole::Caption);
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
        let empty = label(
            ui,
            root,
            "No matching authoring entries.",
            TextRole::Caption,
        );
        ui.set_visibility(empty, false);
        ui.set_visibility(root, false);
        Self {
            root,
            heading,
            status,
            search,
            body,
            empty,
            dynamic: Vec::new(),
            buttons: Vec::new(),
            rows: Vec::new(),
            state: AuthoringState::default(),
            inputs: vec![],
            editing: false,
            query: String::new(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.state.enabled
    }

    /// Rebuild only when semantic data changes, preserving the search widget
    /// and scroll container so a job refresh does not steal keyboard focus.
    pub fn set_state(&mut self, ui: &mut UserInterface, state: AuthoringState) {
        if self.editing || self.state == state {
            return;
        }
        self.state = state;
        ui.set_visibility(self.root, self.state.enabled);
        ui.send(TextMessage::set_text(
            self.heading,
            if self.state.project.is_empty() {
                "Game / Authoring".into()
            } else {
                format!("Game / Authoring · {}", self.state.project)
            },
        ));
        ui.send(TextMessage::set_text(
            self.status,
            format!(
                "{} · revision {}",
                self.state.connection, self.state.revision
            ),
        ));
        for handle in self.dynamic.drain(..) {
            ui.remove_node(handle);
        }
        self.buttons.clear();
        self.inputs.clear();
        self.rows.clear();
        let toolbar = ui.add_node(
            WrapPanelBuilder::new(WidgetBuilder::new())
                .with_gap(4.0, 4.0)
                .build(),
            self.body,
        );
        self.dynamic.push(toolbar);
        for action in &self.state.actions {
            let button = action_button(ui, toolbar, action);
            self.buttons.push((button, action.clone()));
        }
        for entry in &self.state.entries {
            let row = column(ui, self.body);
            self.dynamic.push(row);
            label(
                ui,
                row,
                &format!("{} · {}", entry.category, entry.label),
                TextRole::BodyStrong,
            );
            if !entry.detail.is_empty() {
                label(ui, row, &entry.detail, TextRole::Caption);
            }
            for field in &entry.fields {
                label(ui, row, &field.label, TextRole::Caption);
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
                    row,
                );
                self.inputs.push((input, field.clone()));
            }
            let actions = ui.add_node(
                WrapPanelBuilder::new(WidgetBuilder::new())
                    .with_gap(4.0, 4.0)
                    .build(),
                row,
            );
            for action in &entry.actions {
                let button = action_button(ui, actions, action);
                self.buttons.push((button, action.clone()));
            }
            self.rows.push((
                row,
                format!(
                    "{} {} {} {}",
                    entry.id, entry.label, entry.category, entry.detail
                )
                .to_lowercase(),
            ));
        }
        self.filter(ui);
    }

    fn filter(&self, ui: &mut UserInterface) {
        let query = self.query.to_lowercase();
        let mut visible = 0;
        for (row, search) in &self.rows {
            let matches = query.split_whitespace().all(|word| search.contains(word));
            ui.set_visibility(*row, matches);
            if matches {
                visible += 1;
            }
        }
        ui.set_visibility(self.empty, visible == 0);
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
        if message.destination == self.search {
            if let Some(SearchBoxMessage::Query(query)) = message.data::<SearchBoxMessage>() {
                self.query = query.clone();
                self.filter(ui);
                return (true, None);
            }
        }
        if matches!(message.data::<ButtonMessage>(), Some(ButtonMessage::Click)) {
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

fn column(ui: &mut UserInterface, parent: NodeHandle) -> NodeHandle {
    ui.add_node(
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build(),
        parent,
    )
}

fn label(ui: &mut UserInterface, parent: NodeHandle, text: &str, role: TextRole) -> NodeHandle {
    ui.add_node(
        TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(6.0, 4.0)))
            .with_text(text)
            .with_role(role)
            .with_wrap(true)
            .build(),
        parent,
    )
}

fn action_button(
    ui: &mut UserInterface,
    parent: NodeHandle,
    action: &AuthoringAction,
) -> NodeHandle {
    let enabled = action_event(action).is_some();
    let button = ui.add_node(
        ButtonBuilder::new(
            WidgetBuilder::new()
                .with_height(theme::active().density.row_chrome)
                .with_width(110.0)
                .with_enabled(enabled),
        )
        .build(),
        parent,
    );
    label(ui, button, &action.label, TextRole::Caption);
    if let Some(reason) = &action.disabled_reason {
        ui.nodes.borrow_mut(button.transmute()).widget.tooltip = reason.clone();
    }
    button
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
}
