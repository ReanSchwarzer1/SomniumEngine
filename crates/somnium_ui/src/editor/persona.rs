//! PERSONA presentation state. Domain values and undo stay in generated bindings.
use super::inspector_gen::{GeneratedComponentPanel, GeneratedPropertyRow};
use crate::{
    message::{NodeHandle, TextMessage},
    theme,
    types::Thickness,
    typography::TextRole,
    ui::UserInterface,
    widget::WidgetBuilder,
    widgets::{
        button::{ButtonBuilder, ButtonMessage},
        combo_box::ComboBoxBuilder,
        property_row::PropertyRowMessage,
        stack_panel::{Orientation, StackPanelBuilder},
        text::TextBuilder,
    },
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pins: BTreeSet<String>,
    collapsed: BTreeSet<String>,
    pub favorites: BTreeSet<String>,
    pub recent: Vec<String>,
}
impl Preferences {
    fn path() -> Option<std::path::PathBuf> {
        std::env::var_os("APPDATA")
            .map(|p| std::path::PathBuf::from(p).join("SomniumEngine/persona.json"))
    }
    fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) {
        if cfg!(test) {
            return;
        }
        if let Some(path) = Self::path() {
            let result = (|| -> std::io::Result<()> {
                std::fs::create_dir_all(path.parent().unwrap())?;
                std::fs::write(path, serde_json::to_vec_pretty(self)?)
            })();
            if let Err(error) = result {
                eprintln!("Could not save editor presentation preferences: {error}");
            }
        }
    }
    pub fn visit(&mut self, path: &str) {
        if self.recent.first().is_some_and(|p| p == path) {
            return;
        }
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_owned());
        self.recent.truncate(12);
        self.save();
    }
}
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Modified,
    Pinned,
}
struct Row {
    handle: NodeHandle,
    key: String,
    search: String,
    modified: bool,
    mixed: bool,
}
struct Section {
    button: NodeHandle,
    text: NodeHandle,
    body: NodeHandle,
    key: String,
    label: String,
}

#[derive(Default)]
pub struct Persona {
    pub component_hosts: Vec<(somnium_ecs::reflect::StableId, NodeHandle)>,
    pub prefs: Preferences,
    persist: bool,
    indices: HashMap<
        (
            somnium_ecs::reflect::StableId,
            somnium_ecs::reflect::FieldId,
        ),
        usize,
    >,
    mixed_rows: HashSet<NodeHandle>,
    pub outliner_filters: Vec<(NodeHandle, &'static str)>,
    pub outliner_scope: &'static str,
    rows: Vec<Row>,
    sections: Vec<Section>,
    query: String,
    filter: Filter,
    dirty: bool,
    pub generated_host: NodeHandle,
    pub advanced: NodeHandle,
    pub advanced_body: NodeHandle,
    pub identity: NodeHandle,
    pub filters: Vec<(NodeHandle, Filter)>,
    pub empty: NodeHandle,
    pub clear: NodeHandle,
    pub workspace: NodeHandle,
    pub workspace_popup: NodeHandle,
    pub sort: NodeHandle,
    pub sort_popup: NodeHandle,
    pub size: NodeHandle,
    pub size_popup: NodeHandle,
    pub places: NodeHandle,
    pub places_popup: NodeHandle,
    pub favorite: NodeHandle,
    pub place_paths: Vec<String>,
    pub tools: NodeHandle,
    pub tool_hint: NodeHandle,
    pub floating_labels: Vec<(NodeHandle, NodeHandle)>,
    pub tool_panel: super::tool_context::ToolPanel,
}
impl Persona {
    pub fn load() -> Self {
        Self {
            persist: !cfg!(test),
            prefs: if cfg!(test) {
                Preferences::default()
            } else {
                Preferences::load()
            },
            ..Self::default()
        }
    }
    pub fn begin(&mut self) {
        self.component_hosts.clear();
        self.rows.clear();
        self.sections.clear();
        self.indices.clear();
        self.mixed_rows.clear();
        self.dirty = true;
    }
    pub fn section(
        &mut self,
        ui: &mut UserInterface,
        parent: NodeHandle,
        key: String,
        label: &str,
    ) -> NodeHandle {
        let button = action(ui, parent, label, 0.0);
        let text = ui.nodes.borrow(button.transmute()).widget.children[0];
        let body = ui.add_node(
            StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
                .with_orientation(Orientation::Vertical)
                .build(),
            parent,
        );
        self.sections.push(Section {
            button,
            text,
            body,
            key,
            label: label.to_owned(),
        });
        body
    }
    pub fn register(
        &mut self,
        ui: &mut UserInterface,
        handle: NodeHandle,
        model: &GeneratedPropertyRow,
        component_label: &str,
    ) {
        // FieldId is positional in this engine. Persist the schema's stable field NAME,
        // not its translated display label or a widget handle.
        let key = format!("{}/{}", model.component.as_str(), model.name);
        ui.send(crate::message::UiMessage::new(
            handle,
            crate::message::MessageDirection::ToWidget,
            PropertyRowMessage::SetPinned(self.prefs.pins.contains(&key)),
        ));
        self.indices
            .insert((model.component, model.field), self.rows.len());
        if model.mixed {
            self.mixed_rows.insert(handle);
        }
        self.rows.push(Row {
            handle,
            key,
            search: format!(
                "{} {} {} {} {}",
                component_label,
                model.group.unwrap_or(""),
                model.label,
                model.name,
                model.doc.unwrap_or("")
            )
            .to_lowercase(),
            modified: model.modified,
            mixed: model.mixed,
        });
    }
    pub fn sync(
        &mut self,
        ui: &mut UserInterface,
        panels: &[GeneratedComponentPanel],
        query: &str,
    ) {
        let query = query.trim().to_lowercase();
        if self.query != query {
            self.query = query;
            self.dirty = true;
        }
        for panel in panels {
            for model in &panel.rows {
                if let Some(&index) = self.indices.get(&(model.component, model.field)) {
                    let row = &mut self.rows[index];
                    if (row.modified, row.mixed) != (model.modified, model.mixed) {
                        row.modified = model.modified;
                        row.mixed = model.mixed;
                        self.dirty = true;
                        if model.mixed {
                            self.mixed_rows.insert(row.handle);
                        } else {
                            self.mixed_rows.remove(&row.handle);
                        }
                    }
                    ui.send(PropertyRowMessage::set_modified(
                        row.handle,
                        model.modified && !model.mixed,
                    ));
                    ui.send(crate::message::UiMessage::new(
                        row.handle,
                        crate::message::MessageDirection::ToWidget,
                        PropertyRowMessage::SetResettable(model.modified || model.mixed),
                    ));
                }
            }
        }
        self.apply(ui);
    }
    pub fn clear_filters(&mut self, ui: &mut UserInterface) {
        self.filter = Filter::All;
        self.dirty = true;
        self.search(ui, "");
    }
    pub fn search(&mut self, ui: &mut UserInterface, query: &str) {
        let q = query.trim().to_lowercase();
        if q != self.query {
            self.query = q;
            self.dirty = true;
        }
        self.apply(ui);
    }
    fn matches(&self, row: &Row) -> bool {
        row.search.contains(&self.query)
            && match self.filter {
                Filter::All => true,
                Filter::Modified => row.modified || row.mixed,
                Filter::Pinned => self.prefs.pins.contains(&row.key),
            }
    }
    pub fn apply(&mut self, ui: &mut UserInterface) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let mut count = 0;
        for row in &self.rows {
            let visible = self.matches(row);
            count += usize::from(visible);
            ui.set_visibility(row.handle, visible);
        }
        for section in &self.sections {
            let any = self
                .rows
                .iter()
                .any(|r| self.matches(r) && ui.is_under(r.handle, section.body));
            let expanded = !self.prefs.collapsed.contains(&section.key)
                || !self.query.is_empty()
                || self.filter != Filter::All;
            ui.set_visibility(section.button, any);
            ui.set_visibility(section.body, any && expanded);
            ui.send(TextMessage::set_text(
                section.text,
                format!("{}  {}", if expanded { "▾" } else { "▸" }, section.label),
            ));
        }
        ui.set_visibility(self.empty, count == 0 && !self.rows.is_empty());
        for (handle, filter) in &self.filters {
            ui.send(ButtonMessage::set_selected(*handle, *filter == self.filter));
        }
    }
    pub fn click(&mut self, ui: &mut UserInterface, handle: NodeHandle) -> bool {
        if handle == self.advanced {
            let visible = !ui
                .nodes
                .borrow(self.advanced_body.transmute())
                .widget
                .visibility;
            ui.set_visibility(self.advanced_body, visible);
            ui.send(ButtonMessage::set_selected(handle, visible));
            return true;
        }
        if let Some((_, filter)) = self.filters.iter().find(|(h, _)| *h == handle) {
            self.filter = *filter;
            self.dirty = true;
            self.apply(ui);
            return true;
        }
        if let Some(section) = self.sections.iter().find(|s| s.button == handle) {
            if !self.prefs.collapsed.remove(&section.key) {
                self.prefs.collapsed.insert(section.key.clone());
            }
            if self.persist {
                self.prefs.save();
            }
            self.dirty = true;
            self.apply(ui);
            return true;
        }
        false
    }
    pub fn pin(&mut self, ui: &mut UserInterface, handle: NodeHandle) {
        if let Some(row) = self.rows.iter().find(|r| r.handle == handle) {
            if !self.prefs.pins.remove(&row.key) {
                self.prefs.pins.insert(row.key.clone());
            }
            ui.send(crate::message::UiMessage::new(
                handle,
                crate::message::MessageDirection::ToWidget,
                PropertyRowMessage::SetPinned(self.prefs.pins.contains(&row.key)),
            ));
            if self.persist {
                self.prefs.save();
            }
            self.dirty = true;
            self.apply(ui);
        }
    }
    pub fn mixed(&self, handle: NodeHandle) -> bool {
        self.mixed_rows.contains(&handle)
    }
}

pub fn action(ui: &mut UserInterface, parent: NodeHandle, label: &str, width: f32) -> NodeHandle {
    let mut w = WidgetBuilder::new()
        .with_height(theme::active().density.row_chrome)
        .with_background(theme::TRANSPARENT)
        .with_tooltip(label);
    if width > 0.0 {
        w = w.with_width(width);
    }
    let h = ui.add_node(
        ButtonBuilder::new(w)
            .with_variant(crate::style::ButtonVariant::Quiet)
            .build(),
        parent,
    );
    ui.add_node(
        TextBuilder::new(
            WidgetBuilder::new()
                .with_margin(Thickness::axes(8.0, 0.0))
                .with_vertical_alignment(crate::types::VerticalAlignment::Center),
        )
        .with_text(label)
        .with_role(TextRole::Label)
        .build(),
        h,
    );
    h
}
pub fn combo(
    ui: &mut UserInterface,
    parent: NodeHandle,
    items: &[&str],
    width: f32,
    font: u8,
) -> (NodeHandle, NodeHandle) {
    let h = ui.add_node(
        ComboBoxBuilder::new(
            WidgetBuilder::new()
                .with_width(width)
                .with_height(theme::active().density.row_chrome)
                .with_margin(Thickness::axes(4.0, 1.0)),
        )
        .with_items(items.iter().copied())
        .with_font_id(font)
        .build(),
        parent,
    );
    let popup = super::parts::attach_combo_popup(ui, h, items, font);
    (h, popup)
}

/// Keep the actual scene value representable even when picker search excludes it.
pub fn retain_asset_choice(
    choices: &mut Vec<Option<somnium_ecs::reflect::AssetRef>>,
    current: Option<somnium_ecs::reflect::AssetRef>,
) -> (usize, bool) {
    if let Some(index) = choices.iter().position(|choice| *choice == current) {
        return (index, false);
    }
    choices.push(current);
    (choices.len() - 1, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        editor::{
            editing_rules::EditingRulesRegistry, inspector::build_generated_details,
            inspector_gen::generate_component_panel, property_editors::PropertyEditorRegistry,
        },
        message::{KeyCode, MessageDirection, Modifiers, UiMessage, WidgetMessage},
        widgets::property_row::PropertyRowBuilder,
    };
    #[test]
    fn an_assigned_asset_remains_representable_after_filtering_and_undo() {
        let current = Some(somnium_ecs::reflect::AssetRef::from_raw(42));
        let mut choices = vec![None];
        assert_eq!(retain_asset_choice(&mut choices, current), (1, true));
        assert_eq!(choices[1], current);
        assert_eq!(retain_asset_choice(&mut choices, current), (1, false));
        assert_eq!(retain_asset_choice(&mut choices, None), (0, false));
        assert_eq!(choices.len(), 2);
    }
    fn panels() -> Vec<GeneratedComponentPanel> {
        let schema = somnium_asset::material::material_asset_schema();
        let mut world = somnium_ecs::World::new();
        let entity = world.spawn((somnium_asset::material::MaterialAsset::default(),));
        let values = (schema.snapshot)(&world, entity).unwrap();
        vec![generate_component_panel(
            &schema,
            &values,
            &PropertyEditorRegistry::standard(),
            &EditingRulesRegistry::default(),
        )]
    }
    fn build(
        ui: &mut UserInterface,
        state: &mut Persona,
        panels: &[GeneratedComponentPanel],
    ) -> NodeHandle {
        let root = ui.root();
        build_generated_details(ui, root, 0, panels, &Default::default(), state).0
    }
    #[test]
    fn filters_pins_and_folded_groups_preserve_live_rows() {
        let mut ui = UserInterface::new(340.0, 720.0);
        let mut state = Persona::default();
        let mut panels = panels();
        let rough = panels[0]
            .rows
            .iter_mut()
            .find(|r| r.name == "roughness")
            .unwrap();
        rough.modified = true;
        build(&mut ui, &mut state, &panels);
        state.sync(&mut ui, &panels, "roughness");
        let handle = state
            .rows
            .iter()
            .find(|r| r.key.ends_with("/roughness"))
            .unwrap()
            .handle;
        assert!(ui.visibility(handle));
        assert_eq!(
            state
                .rows
                .iter()
                .filter(|r| ui.visibility(r.handle))
                .count(),
            2,
            "value and roughness texture both match"
        );
        state.pin(&mut ui, handle);
        state.filter = Filter::Pinned;
        state.dirty = true;
        state.search(&mut ui, "");
        assert_eq!(
            state
                .rows
                .iter()
                .filter(|r| ui.visibility(r.handle))
                .count(),
            1
        );
        state.filter = Filter::All;
        state.dirty = true;
        let section = state.sections[0].button;
        let body = state.sections[0].body;
        assert!(state.click(&mut ui, section));
        assert!(!ui.visibility(body));
        state.search(&mut ui, "roughness");
        assert!(
            ui.visibility(body),
            "search reveals hits without changing saved expansion"
        );
        state.search(&mut ui, "");
        assert!(!ui.visibility(body));
        assert!(
            ui.nodes.try_borrow(handle.transmute()).is_ok(),
            "no edit subtree was rebuilt"
        );
    }
    #[test]
    fn pins_survive_field_reindexing_and_display_name_changes() {
        let mut ui = UserInterface::new(340.0, 720.0);
        let mut state = Persona::default();
        let mut panels = panels();
        let root = build(&mut ui, &mut state, &panels);
        let handle = state
            .rows
            .iter()
            .find(|r| r.key.ends_with("/roughness"))
            .unwrap()
            .handle;
        state.pin(&mut ui, handle);
        let json = serde_json::to_string(&state.prefs).unwrap();
        ui.remove_node(root);
        let row = panels[0]
            .rows
            .iter_mut()
            .find(|r| r.name == "roughness")
            .unwrap();
        row.field = somnium_ecs::reflect::FieldId(500);
        row.label = "Translated label".into();
        state.prefs = serde_json::from_str(&json).unwrap();
        build(&mut ui, &mut state, &panels);
        state.filter = Filter::Pinned;
        state.dirty = true;
        state.apply(&mut ui);
        let visible: Vec<_> = state
            .rows
            .iter()
            .filter(|r| ui.visibility(r.handle))
            .collect();
        assert_eq!(visible.len(), 1);
        assert!(visible[0].key.ends_with("/roughness"));
    }
    #[test]
    fn pin_and_reset_have_distinct_keyboard_requests() {
        let mut ui = UserInterface::new(340.0, 80.0);
        let root = ui.root();
        let row = ui.add_node(
            PropertyRowBuilder::new(WidgetBuilder::new())
                .with_label("Roughness")
                .with_pinnable(true)
                .with_modified(false)
                .build(),
            root,
        );
        ui.send(PropertyRowMessage::set_modified(row, true));
        for (key, pin) in [(KeyCode::KeyP, true), (KeyCode::Backspace, false)] {
            ui.send(UiMessage::new(
                row,
                MessageDirection::ToWidget,
                WidgetMessage::KeyDown(key, Modifiers::default()),
            ));
            let events = ui.update();
            assert!(events.iter().any(|e| match e.data::<PropertyRowMessage>() {
                Some(PropertyRowMessage::PinRequested) => pin,
                Some(PropertyRowMessage::RevertRequested) => !pin,
                _ => false,
            }));
        }
        ui.send(UiMessage::new(
            row,
            MessageDirection::ToWidget,
            WidgetMessage::KeyDown(
                KeyCode::KeyP,
                Modifiers {
                    ctrl: true,
                    ..Default::default()
                },
            ),
        ));
        assert!(!ui.update().iter().any(|event| matches!(
            event.data::<PropertyRowMessage>(),
            Some(PropertyRowMessage::PinRequested)
        )));
    }
    #[test]
    fn text_refresh_during_edit_does_not_rename_the_property_label() {
        let mut ui = UserInterface::new(340.0, 80.0);
        let root = ui.root();
        let row = ui.add_node(
            PropertyRowBuilder::new(WidgetBuilder::new())
                .with_label("Object name")
                .build(),
            root,
        );
        let input = ui.add_node(
            crate::widgets::text_box::TextBoxBuilder::new(WidgetBuilder::new())
                .with_text("Lamp")
                .build(),
            row,
        );
        ui.send(UiMessage::new(
            input,
            MessageDirection::ToWidget,
            WidgetMessage::Focus,
        ));
        ui.send(TextMessage::set_text(input, "Model refresh"));
        let events = ui.update();
        assert!(
            ui.nodes
                .borrow(row.transmute())
                .control
                .a11y_name()
                .unwrap()
                .starts_with("Object name.")
        );
        assert!(!events.iter().any(|event| {
            event
                .data::<crate::widgets::text_box::TextBoxMessage>()
                .is_some()
        }));
    }

    #[test]
    fn recent_places_are_bounded_unique_and_serializable() {
        let mut prefs = Preferences::default();
        for i in 0..20 {
            prefs.visit(&format!("folder/{i}"));
        }
        prefs.visit("folder/12");
        prefs.favorites.insert("lighting".into());
        let restored: Preferences =
            serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert_eq!(restored.recent.len(), 12);
        assert_eq!(restored.recent[0], "folder/12");
        assert_eq!(restored.recent.iter().collect::<BTreeSet<_>>().len(), 12);
        assert!(restored.favorites.contains("lighting"));
    }
    #[test]
    fn floating_log_toolbar_wraps_without_losing_actions_at_minimum_width() {
        let mut ui = UserInterface::new(1280.0, 720.0);
        let font = crate::load_fonts(&mut ui);
        let layout = super::super::shell::build_editor_layout(
            &mut ui,
            font,
            crate::layout_persist::ChromeLayout::default().resolved(1280.0, 720.0),
        );
        ui.set_visibility(layout.log_panel, true);
        ui.perform_layout();
        ui.detach(layout.log_panel, glam::Vec2::new(900.0, 320.0));
        ui.perform_layout();
        ui.set_detached_size(layout.log_panel, glam::Vec2::new(480.0, 240.0));
        ui.perform_layout();
        for handle in [
            layout.log_search,
            layout.log_copy,
            layout.log_clear,
            layout.log_jobs_toggle,
            layout.log_history_toggle,
            layout.log_float,
        ] {
            let b = ui.screen_bounds(handle);
            assert!(
                ui.clip_bounds(handle)
                    .contains(glam::Vec2::new(b.x + b.w / 2.0, b.y + b.h / 2.0)),
                "action is clipped: {b:?}, clip {:?}",
                ui.clip_bounds(handle)
            );
            assert!(
                b.w >= 8.0 && b.h >= 18.0,
                "action must have a usable rectangle: {b:?}"
            );
            assert!(
                b.x >= 0.0 && b.x + b.w <= 480.1 && b.y + b.h <= 240.0,
                "action must fit: {b:?}"
            );
        }
        for (root, button, size) in [
            (
                layout.details_grid,
                layout.details_float,
                glam::Vec2::new(320.0, 360.0),
            ),
            (
                layout.outliner_grid,
                layout.outliner_float,
                glam::Vec2::new(300.0, 240.0),
            ),
        ] {
            ui.detach(root, glam::Vec2::new(600.0, 600.0));
            ui.perform_layout();
            ui.set_detached_size(root, size);
            ui.perform_layout();
            let b = ui.screen_bounds(button);
            assert!(b.x >= 0.0 && b.x + b.w <= size.x && b.h >= 24.0);
            assert!(
                ui.clip_bounds(button)
                    .contains(glam::Vec2::new(b.x + b.w / 2.0, b.y + b.h / 2.0))
            );
        }
    }

    #[test]
    fn selected_properties_precede_advanced_controls_at_both_target_sizes() {
        for (w, h) in [(1280.0, 720.0), (1920.0, 1080.0)] {
            let mut ui = UserInterface::new(w, h);
            let mut layout = super::super::shell::build_editor_layout(
                &mut ui,
                0,
                crate::layout_persist::ChromeLayout::default().resolved(w, h),
            );
            ui.set_visibility(layout.details_empty, false);
            let mut panels = panels();
            let mut metadata = panels[0].clone();
            metadata.component = somnium_ecs::reflect::StableId::new("somnium.Name");
            metadata.label = "Name".into();
            metadata.rows.truncate(1);
            metadata.rows[0].component = metadata.component;
            panels.insert(0, metadata);
            let host = layout.persona.generated_host;
            build_generated_details(
                &mut ui,
                host,
                0,
                &panels,
                &Default::default(),
                &mut layout.persona,
            );
            layout.persona.sync(&mut ui, &panels, "");
            ui.perform_layout();
            assert!(
                !layout.persona.rows[0].key.starts_with("somnium.Name/"),
                "authored properties precede repeated identity metadata"
            );
            let field = ui.screen_bounds(layout.persona.rows[0].handle);
            assert!(field.w > 100.0 && field.h >= 24.0);
            assert!(
                field.y + field.h
                    < h - theme::BOTTOM_DRAWER_HEIGHT - theme::active().density.status
            );
            assert!(!ui.visibility(layout.persona.advanced_body));
            assert!(!ui.visibility(layout.persona.tools));
        }
    }
}
