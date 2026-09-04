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
        grid::{Column, GridBuilder, Row},
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

    let dreams_section = section(ui, parent, "DREAMS Sampling");
    let dreams_grain_toggle = check(ui, dreams_section, "Shared grain", font_id);
    let dreams_stf_toggle = check(ui, dreams_section, "Terrain STF", font_id);

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
    // Phase TSUSHIMA. Grouped after the older material dials because they are
    // the landscape-scale ones: what the ground is made of between one texel
    // and one map, rather than how one layer is drawn.
    let terrain_horizon_toggle = check(ui, terrain_section, "Horizon shadow", font_id);
    let terrain_skyvis_toggle = check(ui, terrain_section, "Baked sky visibility", font_id);
    let terrain_relief_map_toggle = check(ui, terrain_section, "Relief normal", font_id);
    let terrain_skyvis = number(ui, terrain_section, "Sky visibility", 0.02, "");
    let terrain_relief_takeover = number(ui, terrain_section, "Relief takeover", 5.0, "m");
    let terrain_splat_noise = number(ui, terrain_section, "Splat noise", 0.02, "");
    let terrain_splat_noise_scale = number(ui, terrain_section, "Splat noise scale", 0.02, "/m");
    let terrain_macro_octaves = number(ui, terrain_section, "Macro octaves", 0.05, "");
    let terrain_damp_tint = number(ui, terrain_section, "Damp tint", 0.05, "");
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
    // The escape hatch for the layer filter. Next to the rest of the brush
    // because that is where someone looks after the log tells them a dab was
    // refused for the ground it was over.
    let foliage_min_weight = number(ui, foliage_section, "Min layer", 0.05, "");
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
        dreams_section,
        dreams_grain_toggle,
        dreams_stf_toggle,
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
        terrain_horizon_toggle,
        terrain_skyvis_toggle,
        terrain_relief_map_toggle,
        terrain_skyvis,
        terrain_relief_takeover,
        terrain_splat_noise,
        terrain_splat_noise_scale,
        terrain_macro_octaves,
        terrain_damp_tint,
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
        foliage_min_weight,
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
    HashMap<
        NodeHandle,
        (
            somnium_ecs::reflect::StableId,
            somnium_ecs::reflect::FieldId,
            CollectionAction,
        ),
    >,
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
    let mut collection_actions = HashMap::new();

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
                    // MORROWIND-AC: a Grid of equal stretch columns, not a
                    // horizontal StackPanel of fixed 58 px lanes.
                    //
                    // The old form pinned each lane at 58 px regardless of how
                    // much room the row had. `NumericField::split_rects` then
                    // laid out a 56 px text field starting 46 px in, so 44 px
                    // of every number fell outside the widget and was clipped —
                    // one digit survived, and a Translation of 14 read as 1.
                    // A stretch column gives each lane an equal share of what
                    // the panel actually has, so the number grows with the
                    // Details width instead of being decided at build time.
                    let lane_count = lane_values.len().max(1);
                    let mut lane_grid = GridBuilder::new(widget).add_row(Row::auto());
                    for _ in 0..lane_count {
                        lane_grid = lane_grid.add_column(Column::stretch());
                    }
                    let lane_panel = ui.add_node(lane_grid.build(), row_handle);
                    for (lane, value) in lane_values.into_iter().enumerate() {
                        let handle = ui.add_node(
                            NumericFieldBuilder::new(
                                WidgetBuilder::new()
                                    .with_row(0)
                                    .with_column(lane)
                                    .with_margin(Thickness::axes(1.0, 0.0)),
                            )
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
                        ("Use Selected", AssetPickerAction::UseDrawerSelection),
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
                PropertyEditorKind::Collection => {
                    // One row per element, each a strip of numeric lanes, and
                    // a footer that adds. Before this the row printed
                    // `Array([Vec3([...])])` as a caption - accurate, and
                    // completely unusable for the one field anybody has that
                    // is an array: a spline's control points.
                    let items = match &model.value {
                        somnium_ecs::reflect::ReflectValue::Array(items) => items.as_slice(),
                        _ => &[],
                    };
                    let column = StackPanelBuilder::new(widget.with_background(theme::TRANSPARENT))
                        .with_orientation(Orientation::Vertical)
                        .build();
                    let column = ui.add_node(column, row_handle);

                    for (index, item) in items.iter().enumerate() {
                        let lanes = crate::element_lane_count(item);
                        if lanes == 0 {
                            continue;
                        }
                        // Index label, the lanes, then the two per-element
                        // buttons. A grid rather than a stack so the numbers
                        // grow with the panel instead of being pinned at build
                        // time - the same reason the vector editor above uses
                        // one.
                        let mut grid = GridBuilder::new(
                            WidgetBuilder::new().with_margin(Thickness::axes(0.0, 1.0)),
                        )
                        .add_row(Row::auto())
                        .add_column(Column::strict(26.0));
                        for _ in 0..lanes {
                            grid = grid.add_column(Column::stretch());
                        }
                        grid = grid.add_column(Column::strict(22.0));
                        grid = grid.add_column(Column::strict(22.0));
                        let strip = ui.add_node(grid.build(), column);

                        let label = TextBuilder::new(
                            WidgetBuilder::new()
                                .with_row(0)
                                .with_column(0)
                                .with_margin(Thickness::axes(2.0, 3.0)),
                        )
                        .with_role(TextRole::Caption)
                        .with_text(format!("{index}"))
                        .build();
                        ui.add_node(label, strip);

                        for lane in 0..lanes {
                            let value =
                                crate::element_lane(items, index as u16, lane as u8).unwrap_or(0.0);
                            let handle = ui.add_node(
                                NumericFieldBuilder::new(
                                    WidgetBuilder::new()
                                        .with_row(0)
                                        .with_column(lane + 1)
                                        .with_margin(Thickness::axes(1.0, 0.0)),
                                )
                                .with_value(value)
                                .with_drag_step(model.step.unwrap_or(0.05) as f32)
                                .with_unit(model.unit.unwrap_or(""))
                                .build(),
                                strip,
                            );
                            let mut binding = base.clone();
                            binding.edit = GeneratedEdit::Element {
                                index: index as u16,
                                lane: lane as u8,
                            };
                            bindings.insert(handle, binding);
                        }

                        for (offset, glyph, action) in [
                            (lanes + 1, "+", CollectionAction::Duplicate(index as u16)),
                            (
                                lanes + 2,
                                "\u{2212}",
                                CollectionAction::Remove(index as u16),
                            ),
                        ] {
                            let button = ui.add_node(
                                ButtonBuilder::new(
                                    WidgetBuilder::new()
                                        .with_row(0)
                                        .with_column(offset)
                                        .with_margin(Thickness::axes(1.0, 0.0)),
                                )
                                .build(),
                                strip,
                            );
                            let glyph = TextBuilder::new(
                                WidgetBuilder::new().with_margin(Thickness::axes(6.0, 2.0)),
                            )
                            .with_role(TextRole::Caption)
                            .with_text(glyph)
                            .build();
                            ui.add_node(glyph, button);
                            collection_actions.insert(button, (base.component, base.field, action));
                        }
                    }

                    let add = ui.add_node(
                        ButtonBuilder::new(
                            WidgetBuilder::new()
                                .with_height(theme::ROW_HEIGHT)
                                .with_margin(Thickness::axes(0.0, 2.0)),
                        )
                        .build(),
                        column,
                    );
                    let add_label = TextBuilder::new(
                        WidgetBuilder::new().with_margin(Thickness::axes(8.0, 3.0)),
                    )
                    .with_role(TextRole::Caption)
                    .with_text(if items.is_empty() {
                        "Add first point"
                    } else {
                        "Add point"
                    })
                    .build();
                    ui.add_node(add_label, add);
                    collection_actions
                        .insert(add, (base.component, base.field, CollectionAction::Append));
                }
                PropertyEditorKind::EntityPicker | PropertyEditorKind::Unsupported => {
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
        collection_actions,
    )
}
