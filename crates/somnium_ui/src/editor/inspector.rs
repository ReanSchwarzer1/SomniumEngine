//! Schema-generated Details plus the small set of renderer/tool controls
//! which deliberately do not belong to ECS component schemas.

#![allow(clippy::too_many_arguments)]

use crate::*;
use crate::{
    editor::parts::*,
    message::NodeHandle,
    theme,
    types::Thickness,
    typography::TextRole,
    ui::UserInterface,
    widget::WidgetBuilder,
    widgets::{
        border::BorderBuilder,
        button::ButtonBuilder,
        check_box::CheckBoxBuilder,
        color_picker::ColorSwatchBuilder,
        combo_box::ComboBoxBuilder,
        combo_box::{ComboBoxMessage, ComboDropdownBuilder},
        curve_editor::CurveEditorBuilder,
        gradient_editor::GradientEditorBuilder,
        image::ImageBuilder,
        numeric_field::NumericFieldBuilder,
        popup::{PopupBuilder, PopupPlacement},
        property_row::PropertyRowBuilder,
        search_box::SearchBoxBuilder,
        stack_panel::{Orientation, StackPanelBuilder},
        text::TextBuilder,
        text_box::TextBoxBuilder,
    },
};

fn section(ui: &mut UserInterface, parent: NodeHandle, label: &str) -> NodeHandle {
    let panel = StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .with_orientation(Orientation::Vertical)
        .build();
    let panel = ui.add_node(panel, parent);
    let band = BorderBuilder::new(
        WidgetBuilder::new()
            .with_height(theme::NOCTURNE.density.row_tree)
            .with_background(theme::BG_HEADER)
            .with_foreground(theme::BORDER_DARK),
    )
    .with_stroke_thickness(Thickness {
        left: 0.0,
        top: 1.0,
        right: 0.0,
        bottom: 1.0,
    })
    .build();
    let band = ui.add_node(band, panel);
    let heading = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(10.0, 7.0)))
        .with_role(TextRole::SectionCaps)
        .with_text(label)
        .build();
    ui.add_node(heading, band);
    panel
}

fn number(
    ui: &mut UserInterface,
    parent: NodeHandle,
    label: &str,
    step: f32,
    unit: &'static str,
) -> NodeHandle {
    let row = PropertyRowBuilder::new(
        WidgetBuilder::new()
            .with_clip_to_bounds(false)
            .with_background(theme::TRANSPARENT),
    )
    .with_label(label)
    .build();
    let row = ui.add_node(row, parent);
    let field =
        NumericFieldBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(0.0, 1.0)))
            .with_drag_step(step)
            .with_unit(unit)
            .build();
    ui.add_node(field, row)
}

fn check(ui: &mut UserInterface, parent: NodeHandle, label: &str, font_id: u8) -> NodeHandle {
    let node = CheckBoxBuilder::new(WidgetBuilder::new().with_height(theme::ROW_HEIGHT))
        .with_label(label)
        .with_font_id(font_id)
        .build();
    ui.add_node(node, parent)
}

fn button(ui: &mut UserInterface, parent: NodeHandle, label: &str) -> (NodeHandle, NodeHandle) {
    let node = ButtonBuilder::new(WidgetBuilder::new().with_height(theme::ROW_HEIGHT)).build();
    let handle = ui.add_node(node, parent);
    let text = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(8.0, 4.0)))
        .with_role(TextRole::Caption)
        .with_text(label)
        .build();
    let text = ui.add_node(text, handle);
    (handle, text)
}

/// Build only controls whose state is owned by an editor tool or renderer.
/// Every component-backed property is materialized by `build_generated_details`.
pub(crate) fn build_inspector(
    ui: &mut UserInterface,
    parent: NodeHandle,
    font_id: u8,
) -> ToolHandles {
    let diagnostics = section(ui, parent, "Renderer Diagnostics");
    let post_census_toggle = check(ui, diagnostics, "Pixel census", font_id);
    let post_bins_toggle = check(ui, diagnostics, "Shade bins", font_id);

    let terrain_section = section(ui, parent, "Terrain Tools");
    let terrain_paint_toggle = check(ui, terrain_section, "Layer paint", font_id);
    let terrain_hex_toggle = check(ui, terrain_section, "Hex anti-tiling", font_id);
    let terrain_parallax_toggle = check(ui, terrain_section, "Parallax", font_id);
    let terrain_clipmap_toggle = check(ui, terrain_section, "Clipmap", font_id);
    let terrain_aerial_toggle = check(ui, terrain_section, "Aerial split", font_id);
    let terrain_aerial_hero_toggle = check(ui, terrain_section, "Hero bank only", font_id);
    let terrain_morph_toggle = check(ui, terrain_section, "LOD morph", font_id);
    let terrain_aerial_dist = number(ui, terrain_section, "Aerial distance", 10.0, "m");
    let terrain_morph_start = number(ui, terrain_section, "Morph start", 0.01, "");
    let mut terrain_brush_items = Vec::new();
    for (index, name) in TERRAIN_BRUSH_NAMES.iter().enumerate() {
        let (handle, text) = button(ui, terrain_section, name);
        terrain_brush_items.push((handle, text, index as u8));
    }
    let terrain_layer = number(ui, terrain_section, "Paint layer", 1.0, "");
    let mut terrain_palette = [NodeHandle::NONE; 32];
    let mut terrain_palette_labels = [NodeHandle::NONE; 32];
    for (index, name) in TERRAIN_LAYER_SHORT.iter().enumerate() {
        let (handle, text) = button(ui, terrain_section, name);
        terrain_palette[index] = handle;
        terrain_palette_labels[index] = text;
    }
    let terrain_tile = number(ui, terrain_section, "Tile scale", 0.01, "");
    let terrain_relief = number(ui, terrain_section, "Relief", 0.05, "");
    let terrain_wetness = number(ui, terrain_section, "Wetness", 0.02, "");
    let terrain_macro = number(ui, terrain_section, "Macro variation", 0.02, "");
    let terrain_debug = number(ui, terrain_section, "Debug view", 1.0, "");
    ui.set_visibility(terrain_section, false);

    let foliage_section = section(ui, parent, "Foliage Brush");
    let foliage_toggle = check(ui, foliage_section, "Visible", font_id);
    let foliage_paint_toggle = check(ui, foliage_section, "Paint", font_id);
    let foliage_erase_toggle = check(ui, foliage_section, "Erase", font_id);
    let foliage_single_toggle = check(ui, foliage_section, "Single", font_id);
    let foliage_kind_button = ComboBoxBuilder::new(WidgetBuilder::new())
        .with_items(FOLIAGE_KIND_NAMES)
        .with_selected(0)
        .with_font_id(font_id)
        .build();
    let foliage_kind_button = ui.add_node(foliage_kind_button, foliage_section);
    let foliage_density = number(ui, foliage_section, "Density", 0.1, "");
    let foliage_seed = number(ui, foliage_section, "Radius", 0.25, "m");
    let foliage_slope = number(ui, foliage_section, "Max slope", 1.0, "°");
    let foliage_layer = number(ui, foliage_section, "Kind", 1.0, "");
    let foliage_smin = number(ui, foliage_section, "Scale min", 0.01, "");
    let foliage_smax = number(ui, foliage_section, "Scale max", 0.01, "");
    ui.set_visibility(foliage_section, false);

    let script_section = section(ui, parent, "Scripts");
    let (script_add, _) = button(ui, script_section, "New Script");
    let script_list =
        StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .with_orientation(Orientation::Vertical)
            .build();
    let script_list = ui.add_node(script_list, script_section);
    ui.set_visibility(script_section, false);

    ToolHandles {
        post_section: diagnostics,
        post_census_toggle,
        post_bins_toggle,
        terrain_section,
        terrain_paint_toggle,
        terrain_hex_toggle,
        terrain_parallax_toggle,
        terrain_clipmap_toggle,
        terrain_aerial_toggle,
        terrain_aerial_dist,
        terrain_aerial_hero_toggle,
        terrain_morph_toggle,
        terrain_morph_start,
        terrain_brush_items,
        terrain_layer,
        terrain_palette,
        terrain_palette_labels,
        terrain_tile,
        terrain_relief,
        terrain_wetness,
        terrain_macro,
        terrain_debug,
        foliage_section,
        foliage_toggle,
        foliage_paint_toggle,
        foliage_erase_toggle,
        foliage_single_toggle,
        foliage_kind_button,
        foliage_density,
        foliage_seed,
        foliage_slope,
        foliage_layer,
        foliage_smin,
        foliage_smax,
        script_section,
        script_add,
        script_list,
        ..ToolHandles::default()
    }
}

/// Materialize reflected component rows into live widgets. Each editable
/// handle carries only a durable schema address and a neutral value.
pub(crate) fn build_generated_details(
    ui: &mut UserInterface,
    parent: NodeHandle,
    font_id: u8,
    panels: &[GeneratedComponentPanel],
    assets: &somnium_asset::database::AssetDbSnapshot,
) -> (
    NodeHandle,
    HashMap<NodeHandle, GeneratedBinding>,
    HashMap<NodeHandle, GeneratedBinding>,
    HashMap<NodeHandle, Vec<Option<somnium_ecs::reflect::AssetRef>>>,
    HashMap<NodeHandle, GeneratedAssetPicker>,
    HashMap<NodeHandle, (NodeHandle, AssetPickerAction)>,
) {
    let root = StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
        .with_orientation(Orientation::Vertical)
        .build();
    let root = ui.add_node(root, parent);
    let mut bindings = HashMap::new();
    let mut rows = HashMap::new();
    let mut asset_choices = HashMap::new();
    let mut asset_searches = HashMap::new();
    let mut asset_actions = HashMap::new();

    for panel in panels {
        let heading =
            TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(10.0, 7.0)))
                .with_role(TextRole::SectionCaps)
                .with_text(&panel.label)
                .build();
        ui.add_node(heading, root);
        if let Some(path) = panel.preview_path.clone() {
            ui.draw_ctx.thumbnails.request(&path, true);
            let preview = ImageBuilder::new(
                WidgetBuilder::new()
                    .with_width(96.0)
                    .with_height(96.0)
                    .with_margin(Thickness::uniform(8.0)),
            )
            .with_asset(path)
            .with_size(96.0)
            .build();
            ui.add_node(preview, root);
        }
        let mut last_group = None;
        for model in &panel.rows {
            if model.group != last_group {
                if let Some(group) = model.group {
                    let label = TextBuilder::new(
                        WidgetBuilder::new().with_margin(Thickness::axes(10.0, 4.0)),
                    )
                    .with_role(TextRole::Caption)
                    .with_text(group)
                    .build();
                    ui.add_node(label, root);
                }
                last_group = model.group;
            }
            let row = PropertyRowBuilder::new(
                WidgetBuilder::new()
                    .with_clip_to_bounds(false)
                    .with_background(theme::TRANSPARENT),
            )
            .with_label(&model.label)
            .with_modified(model.modified && !model.mixed)
            .with_read_only(model.read_only)
            .build();
            let row_handle = ui.add_node(row, root);
            let base = GeneratedBinding {
                component: model.component,
                field: model.field,
                value: model.value.clone(),
                default: model.default.clone(),
                edit: GeneratedEdit::Whole,
                asset_kind_mask: model.asset_kind_mask,
            };
            rows.insert(row_handle, base.clone());
            let widget = WidgetBuilder::new()
                .with_enabled(!model.read_only)
                .with_margin(Thickness::axes(0.0, 1.0));
            match model.editor {
                PropertyEditorKind::CheckBox => {
                    let checked =
                        matches!(model.value, somnium_ecs::reflect::ReflectValue::Bool(true));
                    let handle = ui.add_node(
                        CheckBoxBuilder::new(widget)
                            .with_checked(checked)
                            .with_mixed(model.mixed)
                            .with_label("")
                            .with_font_id(font_id)
                            .build(),
                        row_handle,
                    );
                    bindings.insert(handle, base);
                }
                PropertyEditorKind::Integer | PropertyEditorKind::Number => {
                    let value = match model.value {
                        somnium_ecs::reflect::ReflectValue::I64(v) => v as f32,
                        somnium_ecs::reflect::ReflectValue::F64(v) => v as f32,
                        _ => 0.0,
                    };
                    let mut builder = NumericFieldBuilder::new(widget)
                        .with_value(value)
                        .with_mixed(model.mixed)
                        .with_drag_step(model.step.unwrap_or(0.05) as f32)
                        .with_slider_curve(model.slider)
                        .with_unit(model.unit.unwrap_or(""));
                    if let (Some(min), Some(max)) =
                        (model.soft_min.or(model.min), model.soft_max.or(model.max))
                    {
                        builder = builder.with_range(min as f32, max as f32);
                    }
                    let handle = ui.add_node(builder.build(), row_handle);
                    bindings.insert(handle, base);
                }
                PropertyEditorKind::Text => {
                    let text = match &model.value {
                        somnium_ecs::reflect::ReflectValue::Str(v) => v.clone(),
                        _ => String::new(),
                    };
                    let handle = ui.add_node(
                        TextBoxBuilder::new(widget)
                            .with_text(text)
                            .with_mixed(model.mixed)
                            .with_font_id(font_id)
                            .build(),
                        row_handle,
                    );
                    bindings.insert(handle, base);
                }
                PropertyEditorKind::Vec2
                | PropertyEditorKind::Vec3
                | PropertyEditorKind::Vec4
                | PropertyEditorKind::Euler => {
                    let lane_values: Vec<f32> = match &model.value {
                        somnium_ecs::reflect::ReflectValue::Vec2(v) => v.to_vec(),
                        somnium_ecs::reflect::ReflectValue::Vec3(v) => v.to_vec(),
                        somnium_ecs::reflect::ReflectValue::Vec4(v) => v.to_vec(),
                        somnium_ecs::reflect::ReflectValue::Quat(v) => {
                            let (x, y, z) =
                                glam::Quat::from_array(*v).to_euler(glam::EulerRot::XYZ);
                            vec![x.to_degrees(), y.to_degrees(), z.to_degrees()]
                        }
                        _ => Vec::new(),
                    };
                    let lane_panel = StackPanelBuilder::new(widget)
                        .with_orientation(Orientation::Horizontal)
                        .build();
                    let lane_panel = ui.add_node(lane_panel, row_handle);
                    for (lane, value) in lane_values.into_iter().enumerate() {
                        let handle = ui.add_node(
                            NumericFieldBuilder::new(WidgetBuilder::new().with_width(58.0))
                                .with_value(value)
                                .with_mixed(model.mixed)
                                .with_drag_step(model.step.unwrap_or(0.05) as f32)
                                .with_unit(if model.editor == PropertyEditorKind::Euler {
                                    "°"
                                } else {
                                    model.unit.unwrap_or("")
                                })
                                .build(),
                            lane_panel,
                        );
                        let mut binding = base.clone();
                        binding.edit = if model.editor == PropertyEditorKind::Euler {
                            GeneratedEdit::Euler(lane as u8)
                        } else {
                            GeneratedEdit::Lane(lane as u8)
                        };
                        bindings.insert(handle, binding);
                    }
                }
                PropertyEditorKind::ColorSwatch => {
                    let color = match model.value {
                        somnium_ecs::reflect::ReflectValue::Vec3(v) => [v[0], v[1], v[2], 1.0],
                        somnium_ecs::reflect::ReflectValue::Vec4(v) => v,
                        _ => [1.0; 4],
                    };
                    let handle = ui.add_node(
                        ColorSwatchBuilder::new(widget).with_color(color).build(),
                        row_handle,
                    );
                    bindings.insert(handle, base);
                }
                PropertyEditorKind::ComboBox => {
                    let names = match &model.ty {
                        somnium_ecs::reflect::FieldType::Enum(names) => *names,
                        _ => &[],
                    };
                    let selected = match model.value {
                        somnium_ecs::reflect::ReflectValue::I64(v) => v.max(0) as usize,
                        _ => 0,
                    };
                    let handle = ui.add_node(
                        ComboBoxBuilder::new(widget)
                            .with_items(names.iter().copied())
                            .with_selected(selected)
                            .with_mixed(model.mixed)
                            .with_font_id(font_id)
                            .build(),
                        row_handle,
                    );
                    attach_combo_popup(ui, handle, names, font_id);
                    bindings.insert(handle, base);
                }
                PropertyEditorKind::AssetPicker => {
                    let candidates = crate::editor::property_editors::AssetEditorContext::query(
                        assets,
                        "",
                        model.asset_kind_mask,
                    );
                    let mut labels = Vec::with_capacity(candidates.len() + 1);
                    labels.push("None".to_string());
                    labels.extend(candidates.iter().map(|candidate| candidate.label.clone()));
                    let mut choices = Vec::with_capacity(candidates.len() + 1);
                    choices.push(None);
                    choices.extend(candidates.iter().map(|candidate| Some(candidate.id)));
                    let selected = match model.value {
                        somnium_ecs::reflect::ReflectValue::Asset(current) => choices
                            .iter()
                            .position(|choice| *choice == current)
                            .unwrap_or(0),
                        _ => 0,
                    };
                    let handle = ui.add_node(
                        ComboBoxBuilder::new(widget)
                            .with_items(labels.iter().map(String::as_str))
                            .with_selected(selected)
                            .with_font_id(font_id)
                            .build(),
                        row_handle,
                    );
                    let popup =
                        PopupBuilder::new(WidgetBuilder::new().with_background(theme::BG_PANEL))
                            .with_anchor(handle)
                            .with_placement(PopupPlacement::AnchorBelow)
                            .build();
                    let popup = ui.add_node(popup, ui.root());
                    let column = StackPanelBuilder::new(
                        WidgetBuilder::new()
                            .with_width(360.0)
                            .with_background(theme::BG_PANEL),
                    )
                    .with_orientation(Orientation::Vertical)
                    .build();
                    let column = ui.add_node(column, popup);
                    let search = SearchBoxBuilder::new(
                        WidgetBuilder::new()
                            .with_height(theme::ROW_HEIGHT)
                            .with_background(theme::BG_INPUT),
                    )
                    .with_font_id(font_id)
                    .build();
                    let search = ui.add_node(search, column);
                    let paths = std::iter::once(None)
                        .chain(candidates.iter().map(|candidate| {
                            assets
                                .get(somnium_asset::database::AssetId::from_raw(
                                    candidate.id.raw(),
                                ))
                                .map(|record| record.absolute_path.clone())
                        }))
                        .collect::<Vec<_>>();
                    let list = ComboDropdownBuilder::new(WidgetBuilder::new())
                        .with_items(labels.iter().map(String::as_str))
                        .with_asset_paths(paths)
                        .with_combo(handle)
                        .with_popup(popup)
                        .with_font_id(font_id)
                        .build();
                    let list = ui.add_node(list, column);
                    let actions = StackPanelBuilder::new(WidgetBuilder::new())
                        .with_orientation(Orientation::Horizontal)
                        .build();
                    let actions = ui.add_node(actions, column);
                    for (label, action) in [
                        ("Edit", AssetPickerAction::Edit),
                        ("Locate", AssetPickerAction::Locate),
                        ("Make Unique", AssetPickerAction::MakeUnique),
                    ] {
                        let button =
                            ButtonBuilder::new(WidgetBuilder::new().with_height(theme::ROW_HEIGHT))
                                .build();
                        let button = ui.add_node(button, actions);
                        let text = TextBuilder::new(
                            WidgetBuilder::new().with_margin(Thickness::axes(7.0, 4.0)),
                        )
                        .with_role(TextRole::Caption)
                        .with_text(label)
                        .build();
                        ui.add_node(text, button);
                        asset_actions.insert(button, (handle, action));
                    }
                    ui.send(ComboBoxMessage::bind_popup(handle, popup, list));
                    asset_searches.insert(
                        search,
                        GeneratedAssetPicker {
                            combo: handle,
                            list,
                            kind_mask: model.asset_kind_mask,
                        },
                    );
                    asset_choices.insert(handle, choices);
                    bindings.insert(handle, base);
                }
                // CONTROL-K. A curve row is taller than an ordinary one, so
                // it lays out inside the row rather than beside the label; the
                // `PropertyRow` still owns the label, the modified dot and the
                // revert affordance, so a curve reverts like any other field.
                PropertyEditorKind::Curve => {
                    let curve = match &model.value {
                        somnium_ecs::reflect::ReflectValue::Curve(curve) => curve.clone(),
                        _ => somnium_ecs::curve::Curve::empty(),
                    };
                    #[allow(clippy::cast_possible_truncation)]
                    let handle = ui.add_node(
                        CurveEditorBuilder::new(widget)
                            .with_curve(curve)
                            .with_domain(
                                model.soft_min.unwrap_or(0.0) as f32,
                                model.soft_max.unwrap_or(1.0) as f32,
                            )
                            .with_range(
                                model.min.unwrap_or(0.0) as f32,
                                model.max.unwrap_or(1.0) as f32,
                            )
                            .with_font_id(font_id)
                            .build(),
                        row_handle,
                    );
                    bindings.insert(handle, base);
                }
                PropertyEditorKind::Gradient => {
                    let gradient = match &model.value {
                        somnium_ecs::reflect::ReflectValue::Gradient(gradient) => gradient.clone(),
                        _ => somnium_ecs::curve::Gradient::empty(),
                    };
                    let handle = ui.add_node(
                        GradientEditorBuilder::new(widget)
                            .with_gradient(gradient)
                            .with_font_id(font_id)
                            .build(),
                        row_handle,
                    );
                    bindings.insert(handle, base);
                }
                PropertyEditorKind::EntityPicker
                | PropertyEditorKind::Collection
                | PropertyEditorKind::Unsupported => {
                    let value = TextBuilder::new(widget)
                        .with_role(TextRole::Caption)
                        .with_text(format!("{:?}", model.value))
                        .build();
                    ui.add_node(value, row_handle);
                }
            }
        }
    }
    (
        root,
        bindings,
        rows,
        asset_choices,
        asset_searches,
        asset_actions,
    )
}
