//! Native authoring presentation. Core owns eligibility and every brush value.
use crate::widgets::numeric_field::NumericFieldBuilder;
use crate::*;
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolMode {
    #[default]
    Select,
    Landscape,
    Foliage,
    Lighting,
    Materials,
    VertexPaint,
}
#[derive(Default, Debug, Clone, PartialEq)]
pub struct ToolContext {
    pub mode: ToolMode,
    pub target: String,
    pub material_name: String,
    pub landscape_reason: Option<String>,
    pub foliage_reason: Option<String>,
    pub foliage_visible: bool,
    pub brush: [f32; 3],
    pub operation: usize,
    pub layer: usize,
    pub layer_count: usize,
    /// Why Vertex Paint cannot run on the selection, if it cannot.
    pub paint_reason: Option<String>,
    /// Vertex Paint radius, strength, falloff.
    pub paint_brush: [f32; 3],
    /// Vertex Paint erase, dirt, rust, wet, blood, preview.
    pub paint_flags: [bool; 6],
}
#[derive(Default)]
pub struct ToolPanel {
    pub host: NodeHandle,
    title: NodeHandle,
    target: NodeHandle,
    hint: NodeHandle,
    pub landscape: NodeHandle,
    pub foliage: NodeHandle,
    pub vertex_paint: NodeHandle,
    paint_checks: Vec<NodeHandle>,
    paint_fields: Vec<NodeHandle>,
    paint_actions: Vec<(NodeHandle, VertexPaintEvent)>,
    pub properties: NodeHandle,
    lighting_actions: NodeHandle,
    material_actions: NodeHandle,
    pub commands: Vec<(NodeHandle, &'static str)>,
    fields: Vec<NodeHandle>,
    operation: NodeHandle,
    layer: NodeHandle,
    pub popups: Vec<(NodeHandle, NodeHandle)>,
    finish: NodeHandle,
    previous: Option<(ToolContext, ToolMode)>,
}
impl ToolPanel {
    pub fn build(ui: &mut UserInterface, parent: NodeHandle, font: u8) -> Self {
        use crate::editor::persona::{action, combo};
        let stack = |ui: &mut UserInterface, parent| {
            ui.add_node(
                StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
                    .with_orientation(Orientation::Vertical)
                    .build(),
                parent,
            )
        };
        let host = stack(ui, parent);
        let label = |ui: &mut UserInterface, text: &str, role| {
            ui.add_node(
                TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(8.0, 5.0)))
                    .with_text(text)
                    .with_role(role)
                    .with_wrap(true)
                    .build(),
                host,
            )
        };
        let title = label(ui, "Authoring tools", TextRole::BodyStrong);
        let target = label(ui, "No target selected", TextRole::Body);
        let hint = label(
            ui,
            "Choose Landscape or Foliage in the toolbar.",
            TextRole::Caption,
        );
        let landscape = stack(ui, host);
        let (operation, operation_popup) = combo(ui, landscape, &TERRAIN_BRUSH_NAMES, 260.0, font);
        let mut fields = Vec::new();
        for (name, step, unit) in [
            ("Radius", 0.25, "m"),
            ("Strength", 0.01, ""),
            ("Hardness", 0.01, ""),
        ] {
            let row = ui.add_node(
                crate::widgets::property_row::PropertyRowBuilder::new(WidgetBuilder::new())
                    .with_label(name)
                    .build(),
                landscape,
            );
            fields.push(
                ui.add_node(
                    NumericFieldBuilder::new(WidgetBuilder::new())
                        .with_drag_step(step)
                        .with_unit(unit)
                        .build(),
                    row,
                ),
            );
        }
        let (layer, layer_popup) = combo(ui, landscape, &TERRAIN_LAYER_SHORT, 260.0, font);
        ui.nodes.borrow_mut(layer.transmute()).widget.tooltip =
            "Paint layer from the selected terrain's loaded layer palette".into();
        let foliage = stack(ui, host);
        let vertex_paint = stack(ui, host);
        let mut paint_checks = Vec::new();
        // A channel paints its layer material (set on the entity's Vertex
        // Paint component) or, without one, the weathering named here.
        for name in ["Erase", "R: layer / dirt", "G: layer / rust", "B: layer / wet", "A: layer / blood", "Preview masks"] {
            paint_checks.push(ui.add_node(
                crate::widgets::check_box::CheckBoxBuilder::new(
                    WidgetBuilder::new()
                        .with_height(theme::active().density.row_chrome)
                        .with_margin(Thickness::axes(8.0, 0.0)),
                )
                .with_label(name)
                .with_font_id(font)
                .build(),
                vertex_paint,
            ));
        }
        let mut paint_fields = Vec::new();
        for (name, step, unit) in [("Radius", 0.05, "m"), ("Strength", 0.01, ""), ("Falloff", 0.01, "")] {
            let row = ui.add_node(
                crate::widgets::property_row::PropertyRowBuilder::new(WidgetBuilder::new())
                    .with_label(name)
                    .build(),
                vertex_paint,
            );
            paint_fields.push(ui.add_node(
                NumericFieldBuilder::new(WidgetBuilder::new())
                    .with_drag_step(step)
                    .with_unit(unit)
                    .build(),
                row,
            ));
        }
        let paint_row = ui.add_node(
            crate::widgets::wrap_panel::WrapPanelBuilder::new(
                WidgetBuilder::new().with_background(theme::TRANSPARENT),
            )
            .with_gap(4.0, 4.0)
            .build(),
            vertex_paint,
        );
        let mut paint_actions = Vec::new();
        for (label, event) in [
            ("Fill", VertexPaintEvent::Fill),
            ("Clear", VertexPaintEvent::Clear),
            ("Copy", VertexPaintEvent::Copy),
            ("Paste", VertexPaintEvent::Paste),
            ("Auto weather", VertexPaintEvent::AutoWeather),
        ] {
            paint_actions.push((action(ui, paint_row, label, 120.0), event));
        }
        let mut commands = Vec::new();
        let lighting_actions = ui.add_node(
            crate::widgets::wrap_panel::WrapPanelBuilder::new(
                WidgetBuilder::new().with_background(theme::TRANSPARENT),
            )
            .with_gap(4.0, 4.0)
            .build(),
            host,
        );
        for (label, id) in [
            ("Point light", "editor.create.point_light"),
            ("Spot light", "editor.create.spot_light"),
            ("Sun light", "editor.create.directional_light"),
            ("Area light", "editor.create.area_light"),
        ] {
            commands.push((action(ui, lighting_actions, label, 140.0), id));
        }
        let material_actions = ui.add_node(
            crate::widgets::wrap_panel::WrapPanelBuilder::new(
                WidgetBuilder::new().with_background(theme::TRANSPARENT),
            )
            .with_gap(4.0, 4.0)
            .build(),
            host,
        );
        commands.push((
            action(ui, material_actions, "New material", 140.0),
            "editor.asset.new_material",
        ));
        commands.push((
            action(ui, material_actions, "Save", 100.0),
            "editor.scene.save",
        ));
        let properties = stack(ui, host);
        let finish = action(ui, host, "Finish · return to Select", 0.0);
        Self {
            host,
            title,
            target,
            hint,
            landscape,
            foliage,
            vertex_paint,
            paint_checks,
            paint_fields,
            paint_actions,
            properties,
            lighting_actions,
            material_actions,
            commands,
            fields,
            operation,
            layer,
            popups: vec![(operation, operation_popup), (layer, layer_popup)],
            finish,
            previous: None,
        }
    }
    pub fn refresh(&mut self, ui: &mut UserInterface, state: ToolContext, shown: ToolMode) -> bool {
        if self
            .previous
            .as_ref()
            .is_some_and(|(old, mode)| old == &state && *mode == shown)
        {
            return false;
        }
        let mode_changed = self
            .previous
            .as_ref()
            .is_none_or(|(old, old_shown)| old.mode != state.mode || *old_shown != shown);
        let reason = match shown {
            ToolMode::Landscape => state.landscape_reason.as_deref(),
            ToolMode::Foliage => state.foliage_reason.as_deref(),
            ToolMode::VertexPaint => state.paint_reason.as_deref(),
            ToolMode::Select | ToolMode::Lighting | ToolMode::Materials => None,
        };
        ui.send(TextMessage::set_text(
            self.title,
            match shown {
                ToolMode::Select => "Authoring tools",
                ToolMode::Landscape => "Landscape",
                ToolMode::Foliage => "Foliage",
                ToolMode::Lighting => "Lighting",
                ToolMode::Materials => "Material",
                ToolMode::VertexPaint => "Vertex Paint",
            },
        ));
        ui.send(TextMessage::set_text(
            self.target,
            if shown == ToolMode::Materials && !state.material_name.is_empty() {
                &state.material_name
            } else if state.target.is_empty() {
                "No target selected"
            } else {
                &state.target
            },
        ));
        ui.send(TextMessage::set_text(self.hint, reason.unwrap_or(match shown {
            ToolMode::Lighting => "Select a light or Environment to edit its lighting. Changes use the same Details and undo controls.",
            ToolMode::Materials => "Select a material in the Content Drawer, or an object using one. Save writes the material asset. Material graphs are planned for a later phase.",
            ToolMode::Select => "Choose Landscape or Foliage in the toolbar.",
            ToolMode::Landscape => "Choose an operation, then drag on terrain. Esc cancels a stroke; Ctrl+Z undoes it.",
            ToolMode::Foliage if !state.foliage_visible => "Foliage is hidden. Enable Visible below before painting.",
            ToolMode::Foliage => "Choose a kind, then paint on terrain. Placement respects slope and layer limits; Ctrl+Z undoes a dab.",
            ToolMode::VertexPaint => "Tick the channels, then drag on the selected mesh. Each channel paints the material set as its layer on the Vertex Paint component (Inspector), or dirt, rust, wet and blood when it has none. Erase removes paint; Ctrl+Z undoes a stroke. Auto weather builds grime, damp, rust and wear from the mesh's shape.",
        })));
        ui.set_visibility(self.lighting_actions, shown == ToolMode::Lighting);
        ui.set_visibility(self.material_actions, shown == ToolMode::Materials);
        ui.set_visibility(
            self.properties,
            matches!(shown, ToolMode::Lighting | ToolMode::Materials),
        );
        ui.set_visibility(self.landscape, shown == ToolMode::Landscape);
        ui.set_visibility(self.foliage, shown == ToolMode::Foliage);
        ui.set_visibility(self.vertex_paint, shown == ToolMode::VertexPaint);
        ui.nodes.borrow_mut(self.vertex_paint.transmute()).widget.enabled =
            state.paint_reason.is_none();
        for (handle, value) in self.paint_fields.iter().zip(state.paint_brush) {
            ui.send(NumericFieldMessage::set_value(*handle, value));
        }
        for (handle, on) in self.paint_checks.iter().zip(state.paint_flags) {
            ui.send(crate::widgets::check_box::CheckBoxMessage::set_checked(*handle, on));
        }
        ui.set_visibility(self.finish, state.mode != ToolMode::Select);
        ui.nodes
            .borrow_mut(self.landscape.transmute())
            .widget
            .enabled = state.landscape_reason.is_none();
        ui.nodes.borrow_mut(self.foliage.transmute()).widget.enabled =
            state.foliage_reason.is_none();
        for (handle, value) in self.fields.iter().zip(state.brush) {
            ui.send(NumericFieldMessage::set_value(*handle, value));
        }
        ui.send(ComboBoxMessage::set_selected(
            self.operation,
            state.operation,
        ));
        ui.set_visibility(self.layer, state.operation == 5 && state.layer_count > 0);
        // Unsupported unloaded palette entries cannot be selected.
        let labels: Vec<_> = TERRAIN_LAYER_SHORT
            .iter()
            .take(state.layer_count)
            .map(|s| s.to_string())
            .collect();
        for handle in [
            self.layer,
            ui.nodes
                .borrow(self.popups[1].1.transmute())
                .widget
                .children[0],
        ] {
            ui.send(UiMessage::new(
                handle,
                MessageDirection::ToWidget,
                ComboBoxMessage::SetItems(labels.clone()),
            ));
        }
        ui.send(ComboBoxMessage::set_selected(self.layer, state.layer));
        self.previous = Some((state, shown));
        mode_changed
    }
    pub fn event(&self, msg: &UiMessage) -> Option<EditorEvent> {
        if msg.direction != MessageDirection::FromWidget {
            return None;
        }
        if matches!(msg.data::<ButtonMessage>(), Some(ButtonMessage::Click))
            && msg.destination == self.finish
        {
            return Some(EditorEvent::SetGizmoMode(0));
        }
        if matches!(msg.data::<ButtonMessage>(), Some(ButtonMessage::Click))
            && let Some((_, event)) = self.paint_actions.iter().find(|(h, _)| *h == msg.destination)
        {
            return Some(EditorEvent::VertexPaint(*event));
        }
        if let Some(crate::widgets::check_box::CheckBoxMessage::Check(on)) =
            msg.data::<crate::widgets::check_box::CheckBoxMessage>()
            && let Some(index) = self.paint_checks.iter().position(|h| *h == msg.destination)
        {
            return Some(EditorEvent::VertexPaint(VertexPaintEvent::SetFlag(index as u8, *on)));
        }
        if let Some(index) = self.paint_fields.iter().position(|h| *h == msg.destination) {
            return match msg.data::<NumericFieldMessage>()? {
                NumericFieldMessage::ValueChanging(value) | NumericFieldMessage::ValueChanged(value) => {
                    Some(EditorEvent::VertexPaint(VertexPaintEvent::SetBrush(index as u8, *value)))
                }
                _ => None,
            };
        }
        if let Some(ComboBoxMessage::SelectionChanged(index)) = msg.data::<ComboBoxMessage>() {
            if msg.destination == self.operation {
                return Some(EditorEvent::SetTerrainTool(*index as u8));
            }
            if msg.destination == self.layer {
                return Some(EditorEvent::SetTerrainPaintLayer(*index as u8));
            }
        }
        let index = self.fields.iter().position(|h| *h == msg.destination)?;
        let (value, live) = match msg.data::<NumericFieldMessage>()? {
            NumericFieldMessage::ValueChanging(value) => (*value, true),
            NumericFieldMessage::ValueChanged(value) => (*value, false),
            _ => return None,
        };
        Some(EditorEvent::SetLandscapeBrush {
            field: index as u8,
            value,
            live,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_switches_keep_the_same_widgets_and_explain_ineligible_targets() {
        let mut ui = UserInterface::new(280.0, 600.0);
        let root = ui.root();
        let mut panel = ToolPanel::build(&mut ui, root, 0);
        let radius = panel.fields[0];
        let blocked = ToolContext {
            target: "Locked hillside".into(),
            landscape_reason: Some("Unlock the target".into()),
            ..Default::default()
        };
        panel.refresh(&mut ui, blocked.clone(), ToolMode::Landscape);
        ui.update();
        ui.perform_layout();
        assert!(ui.visibility(panel.landscape));
        assert!(!ui.visibility(panel.foliage));
        assert!(!ui.nodes.borrow(panel.landscape.transmute()).widget.enabled);
        assert_eq!(radius, panel.fields[0]);
        assert!(!panel.refresh(&mut ui, blocked, ToolMode::Landscape));
        panel.refresh(&mut ui, ToolContext::default(), ToolMode::Foliage);
        ui.update();
        assert!(ui.visibility(panel.foliage));
        assert!(!ui.visibility(panel.landscape));
    }
    #[test]
    fn hidden_foliage_explains_visibility_without_disabling_the_visible_checkbox() {
        let mut ui = UserInterface::new(280.0, 600.0);
        let root = ui.root();
        let mut panel = ToolPanel::build(&mut ui, root, 0);
        panel.refresh(&mut ui, ToolContext::default(), ToolMode::Foliage);
        assert!(ui.nodes.borrow(panel.foliage.transmute()).widget.enabled);
        let mut explained = false;
        while let Some(message) = ui.poll_message() {
            if message.destination == panel.hint {
                if let Some(TextMessage::SetText(text)) = message.data::<TextMessage>() {
                    explained = text.contains("Enable Visible");
                }
            }
        }
        assert!(explained);
    }
    #[test]
    fn authoring_actions_use_registered_commands_and_keep_their_context() {
        let mut ui = UserInterface::new(320.0, 600.0);
        let root = ui.root();
        let mut panel = ToolPanel::build(&mut ui, root, 0);
        for (_, id) in &panel.commands {
            assert!(crate::commands::registry().get(id).is_some(), "{id}");
        }
        panel.refresh(&mut ui, ToolContext::default(), ToolMode::Lighting);
        assert!(ui.visibility(panel.lighting_actions));
        assert!(!ui.visibility(panel.material_actions));
        panel.refresh(&mut ui, ToolContext::default(), ToolMode::Materials);
        assert!(ui.visibility(panel.material_actions));
        assert!(!ui.visibility(panel.lighting_actions));
        assert!(ui.visibility(panel.properties));
    }
    #[test]
    fn vertex_paint_panel_shows_in_its_mode_and_reports_its_controls() {
        let mut ui = UserInterface::new(280.0, 600.0);
        let root = ui.root();
        let mut panel = ToolPanel::build(&mut ui, root, 0);
        panel.refresh(&mut ui, ToolContext::default(), ToolMode::VertexPaint);
        ui.update();
        assert!(ui.visibility(panel.vertex_paint));
        assert!(!ui.visibility(panel.foliage));
        fn from<T: std::any::Any + Send>(dest: NodeHandle, data: T) -> UiMessage {
            UiMessage::new(dest, MessageDirection::FromWidget, data)
        }
        assert!(matches!(
            panel.event(&from(panel.paint_checks[2], crate::widgets::check_box::CheckBoxMessage::Check(true))),
            Some(EditorEvent::VertexPaint(VertexPaintEvent::SetFlag(2, true)))
        ));
        assert!(matches!(
            panel.event(&from(panel.paint_fields[0], NumericFieldMessage::ValueChanged(0.8))),
            Some(EditorEvent::VertexPaint(VertexPaintEvent::SetBrush(0, v))) if v == 0.8
        ));
        assert!(matches!(
            panel.event(&from(panel.paint_actions[4].0, ButtonMessage::Click)),
            Some(EditorEvent::VertexPaint(VertexPaintEvent::AutoWeather))
        ));
    }
    #[test]
    fn brush_commands_preserve_live_and_commit_and_ignore_model_refresh() {
        let mut ui = UserInterface::new(280.0, 600.0);
        let root = ui.root();
        let panel = ToolPanel::build(&mut ui, root, 0);
        assert!(
            panel
                .event(&NumericFieldMessage::set_value(panel.fields[0], 8.0))
                .is_none()
        );
        for (message, expected) in [
            (NumericFieldMessage::ValueChanging(12.0), true),
            (NumericFieldMessage::ValueChanged(12.0), false),
        ] {
            let event = panel.event(&UiMessage::new(
                panel.fields[0],
                MessageDirection::FromWidget,
                message,
            ));
            assert!(
                matches!(event,Some(EditorEvent::SetLandscapeBrush {field:0,value:12.0,live}) if live==expected)
            );
        }
        assert!(matches!(
            panel.event(&UiMessage::new(
                panel.operation,
                MessageDirection::FromWidget,
                ComboBoxMessage::SelectionChanged(5)
            )),
            Some(EditorEvent::SetTerrainTool(5))
        ));
    }
}
