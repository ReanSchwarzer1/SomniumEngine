use super::{
    Catalogue, ConnectError, Connection, Graph, NodeId, PinDirection, PinRef,
    archetype::{NodeArchetype, PinArchetype, PinType},
    catalogues,
    geometry::{self, Alignment, GraphSelection, GraphView},
    serial,
};
use crate::types::Rect;
use glam::Vec2;

fn two_node_graph() -> (Catalogue, Graph, NodeId, NodeId) {
    let catalogue = catalogues::material();
    let mut graph = Graph::new();
    let scalar = graph
        .add(&catalogue, "material.scalar", Vec2::new(20.0, 30.0))
        .unwrap();
    let output = graph
        .add(&catalogue, "material.surface", Vec2::new(300.0, 30.0))
        .unwrap();
    (catalogue, graph, scalar, output)
}

#[test]
fn pin_types_widen_but_never_narrow_or_cross_flow() {
    assert!(PinType::Float.connects_to(PinType::Vec3));
    assert!(PinType::Color.connects_to(PinType::Vec4));
    assert!(!PinType::Vec3.connects_to(PinType::Float));
    assert!(!PinType::Vec4.connects_to(PinType::Color));
    assert!(!PinType::Flow.connects_to(PinType::Float));
}

#[test]
fn opaque_names_round_trip_only_through_the_owning_catalogue() {
    let animation = catalogues::animation();
    assert_eq!(
        PinType::parse("animation.pose", animation.opaques()),
        Some(PinType::Opaque("animation.pose"))
    );
    assert_eq!(PinType::parse("animation.pose", &[]), None);
}

#[test]
fn the_second_catalogue_is_not_a_material_catalogue() {
    let material = catalogues::material();
    let animation = catalogues::animation();
    assert_ne!(material.id, animation.id);
    assert_eq!(material.root(), Some("material.surface"));
    assert_eq!(animation.root(), Some("animation.output"));
    assert!(animation.get("animation.blend1d").is_some());
    assert!(material.get("animation.blend1d").is_none());
}

#[test]
fn palette_groups_have_an_explicit_stable_order() {
    let material = catalogues::material();
    let ids: Vec<_> = material.groups().iter().map(|group| group.id).collect();
    assert_eq!(ids, ["inputs", "math", "layout", "output"]);
}

#[test]
fn palette_search_reads_keywords_as_well_as_titles() {
    let material = catalogues::material();
    assert_eq!(material.search("times")[0].id, "material.multiply");
    assert_eq!(material.search("RGB")[0].id, "material.color");
}

#[test]
fn connections_normalise_the_direction_the_user_dragged() {
    let (catalogue, mut graph, scalar, output) = two_node_graph();
    let connection = graph
        .connect(
            &catalogue,
            PinRef::input(output, 1),
            PinRef::output(scalar, 0),
        )
        .unwrap();
    assert_eq!(connection.from.direction, PinDirection::Output);
    assert_eq!(connection.to.direction, PinDirection::Input);
}

#[test]
fn an_input_takes_one_wire_and_outputs_fan_out() {
    let (catalogue, mut graph, scalar, output) = two_node_graph();
    let other = graph
        .add(&catalogue, "material.scalar", Vec2::ZERO)
        .unwrap();
    graph
        .connect(
            &catalogue,
            PinRef::output(scalar, 0),
            PinRef::input(output, 1),
        )
        .unwrap();
    assert_eq!(
        graph.connect(
            &catalogue,
            PinRef::output(other, 0),
            PinRef::input(output, 1)
        ),
        Err(ConnectError::InputOccupied)
    );
    graph
        .connect(
            &catalogue,
            PinRef::output(scalar, 0),
            PinRef::input(output, 2),
        )
        .unwrap();
}

#[test]
fn a_cycle_is_refused_and_topological_order_stays_deterministic() {
    let mut catalogue = Catalogue::new("test");
    catalogue.register(
        NodeArchetype::new("through", "Through", "Test")
            .with_input(PinArchetype::new("In", PinType::Float))
            .with_output(PinArchetype::new("Out", PinType::Float)),
    );
    let mut graph = Graph::new();
    let a = graph.add(&catalogue, "through", Vec2::ZERO).unwrap();
    let b = graph.add(&catalogue, "through", Vec2::ZERO).unwrap();
    graph
        .connect(&catalogue, PinRef::output(a, 0), PinRef::input(b, 0))
        .unwrap();
    assert_eq!(
        graph.connect(&catalogue, PinRef::output(b, 0), PinRef::input(a, 0)),
        Err(ConnectError::WouldCycle)
    );
    assert_eq!(graph.topological_order().unwrap(), [a, b]);
}

#[test]
fn a_failed_same_direction_reconnect_changes_nothing() {
    let (catalogue, mut graph, scalar, output) = two_node_graph();
    let connection = graph
        .connect(
            &catalogue,
            PinRef::output(scalar, 0),
            PinRef::input(output, 1),
        )
        .unwrap();
    let before = graph.connections().to_vec();
    assert_eq!(
        graph.reconnect(&catalogue, connection.to, PinRef::input(output, 2)),
        Err(ConnectError::SameDirection)
    );
    assert_eq!(graph.connections(), before);
}

#[test]
fn rerouting_is_transactional_and_preserves_types() {
    let (catalogue, mut graph, scalar, output) = two_node_graph();
    let original = graph
        .connect(
            &catalogue,
            PinRef::output(scalar, 0),
            PinRef::input(output, 1),
        )
        .unwrap();
    let reroute = graph
        .insert_reroute(&catalogue, original, Vec2::new(180.0, 60.0))
        .unwrap();
    assert_eq!(graph.connections().len(), 2);
    assert_eq!(
        graph.input_source(PinRef::input(reroute, 0)),
        Some(original.from)
    );
    assert_eq!(
        graph.input_source(original.to),
        Some(PinRef::output(reroute, 0))
    );
}

#[test]
fn a_malformed_reroute_archetype_leaves_the_wire_untouched() {
    let mut catalogue = Catalogue::new("bad");
    catalogue.register(
        NodeArchetype::new("through", "Through", "Test")
            .with_input(PinArchetype::new("In", PinType::Float))
            .with_output(PinArchetype::new("Out", PinType::Float)),
    );
    catalogue.register(
        NodeArchetype::new("bad-reroute", "Bad", "Test")
            .with_input(PinArchetype::new("In", PinType::Float))
            .with_output(PinArchetype::new("Out", PinType::Texture))
            .as_reroute(),
    );
    let mut graph = Graph::new();
    let a = graph.add(&catalogue, "through", Vec2::ZERO).unwrap();
    let b = graph.add(&catalogue, "through", Vec2::X).unwrap();
    let wire = graph
        .connect(&catalogue, PinRef::output(a, 0), PinRef::input(b, 0))
        .unwrap();
    let before = graph.clone();
    assert_eq!(graph.insert_reroute(&catalogue, wire, Vec2::ZERO), None);
    assert_eq!(graph, before);
}

#[test]
fn copy_and_paste_remap_ids_and_only_keep_internal_wires() {
    let (catalogue, mut graph, scalar, output) = two_node_graph();
    graph
        .connect(
            &catalogue,
            PinRef::output(scalar, 0),
            PinRef::input(output, 1),
        )
        .unwrap();
    let fragment = graph.copy(&[scalar, output]);
    let pasted = graph
        .paste(&catalogue, &fragment, Vec2::new(40.0, 50.0))
        .unwrap();
    assert_eq!(pasted.len(), 2);
    assert!(pasted.iter().all(|id| *id != scalar && *id != output));
    assert_eq!(graph.connections().len(), 2);
}

#[test]
fn a_bad_fragment_is_refused_without_partial_nodes() {
    let (catalogue, mut graph, scalar, _) = two_node_graph();
    let mut fragment = graph.copy(&[scalar]);
    fragment.nodes[0].archetype = "missing".to_string();
    let before = graph.clone();
    assert!(graph.paste(&catalogue, &fragment, Vec2::ZERO).is_err());
    assert_eq!(graph, before);
}

#[test]
fn context_breadcrumbs_are_a_stack_and_ignore_blank_names() {
    let mut graph = Graph::new();
    graph.enter_context("Locomotion");
    graph.enter_context("  ");
    graph.enter_context("Run");
    assert_eq!(graph.context(), ["Locomotion", "Run"]);
    assert_eq!(graph.leave_context().as_deref(), Some("Run"));
}

#[test]
fn zoom_keeps_the_pointer_over_the_same_graph_point() {
    let mut view = GraphView::default();
    view.pan_by(Vec2::new(35.0, -12.0));
    let pointer = Vec2::new(320.0, 180.0);
    let before = view.screen_to_graph(pointer);
    view.zoom_at(pointer, 1.7);
    let after = view.screen_to_graph(pointer);
    assert!(before.abs_diff_eq(after, 1e-4));
}

#[test]
fn zoom_is_bounded_and_rejects_invalid_factors() {
    let mut view = GraphView::default();
    view.zoom_at(Vec2::ZERO, 1_000.0);
    assert_eq!(view.zoom, GraphView::MAX_ZOOM);
    view.zoom_at(Vec2::ZERO, 0.0);
    assert_eq!(view.zoom, GraphView::MAX_ZOOM);
    view.zoom_at(Vec2::ZERO, 0.00001);
    assert_eq!(view.zoom, GraphView::MIN_ZOOM);
}

#[test]
fn node_layout_puts_inputs_left_and_outputs_right() {
    let (catalogue, graph, scalar, _) = two_node_graph();
    let layout = geometry::layout_nodes(&graph, &catalogue)
        .into_iter()
        .find(|layout| layout.node == scalar)
        .unwrap();
    let input = layout.pin(PinRef::input(scalar, 0)).unwrap();
    let output = layout.pin(PinRef::output(scalar, 0)).unwrap();
    assert_eq!(input.position.x, layout.bounds.x);
    assert_eq!(output.position.x, layout.bounds.x + layout.bounds.w);
}

#[test]
fn box_selection_accepts_a_reverse_drag() {
    let (catalogue, graph, scalar, output) = two_node_graph();
    let layouts = geometry::layout_nodes(&graph, &catalogue);
    let mut selection = GraphSelection::default();
    selection.select_box(&layouts, Rect::new(250.0, 200.0, -250.0, -200.0));
    assert!(selection.contains(scalar));
    assert!(!selection.contains(output));
}

#[test]
fn alignment_uses_the_first_selected_node_as_anchor() {
    let (catalogue, mut graph, scalar, output) = two_node_graph();
    assert!(geometry::align_nodes(
        &mut graph,
        &catalogue,
        &[scalar, output],
        Alignment::Top
    ));
    assert_eq!(graph.node(scalar).unwrap().position.y, 30.0);
    assert_eq!(graph.node(output).unwrap().position.y, 30.0);
}

#[test]
fn a_wire_uses_the_shared_morrowind_d_path_primitive() {
    let (catalogue, mut graph, scalar, output) = two_node_graph();
    let connection = graph
        .connect(
            &catalogue,
            PinRef::output(scalar, 0),
            PinRef::input(output, 1),
        )
        .unwrap();
    let layouts = geometry::layout_nodes(&graph, &catalogue);
    let flattened = geometry::wire_path(&layouts, connection)
        .unwrap()
        .flatten(0.5);
    assert_eq!(flattened.len(), 1);
    assert!(flattened[0].points.len() > 2);
}

#[test]
fn graph_assets_round_trip_byte_identically() {
    let (catalogue, mut graph, scalar, output) = two_node_graph();
    graph
        .node_mut(scalar)
        .unwrap()
        .literals
        .insert(0, "0.25".into());
    graph.enter_context("Material Function");
    graph
        .connect(
            &catalogue,
            PinRef::output(scalar, 0),
            PinRef::input(output, 1),
        )
        .unwrap();
    let first = serial::to_json(&graph, &catalogue).unwrap();
    let loaded = serial::from_json(&first, &catalogue).unwrap();
    let second = serial::to_json(&loaded, &catalogue).unwrap();
    assert_eq!(first, second);
    assert_eq!(graph, loaded);
}

#[test]
fn deleted_high_ids_are_not_reused_after_a_save() {
    let (catalogue, mut graph, _, _) = two_node_graph();
    let deleted = graph
        .add(&catalogue, "material.scalar", Vec2::ZERO)
        .unwrap();
    graph.remove(deleted);
    let json = serial::to_json(&graph, &catalogue).unwrap();
    let mut loaded = serial::from_json(&json, &catalogue).unwrap();
    let next = loaded
        .add(&catalogue, "material.scalar", Vec2::ZERO)
        .unwrap();
    assert!(next > deleted);
}

#[test]
fn a_future_graph_version_is_refused_not_guessed() {
    let catalogue = catalogues::material();
    let json = r#"{"version":999,"catalogue":"somnium.material"}"#;
    assert!(matches!(
        serial::from_json(json, &catalogue),
        Err(serial::GraphAssetError::FutureVersion(999))
    ));
}

#[test]
fn an_unversioned_graph_migrates_and_gets_a_monotonic_cursor() {
    let catalogue = catalogues::material();
    let json = r#"{
        "catalogue":"somnium.material",
        "nodes":[{"id":7,"archetype":"material.scalar","position":[0.0,0.0]}]
    }"#;
    let mut graph = serial::from_json(json, &catalogue).unwrap();
    assert_eq!(
        graph
            .add(&catalogue, "material.scalar", Vec2::ZERO)
            .unwrap(),
        NodeId(8)
    );
}

#[test]
fn a_graph_cannot_be_loaded_under_the_wrong_catalogue() {
    let (material, graph, _, _) = two_node_graph();
    let json = serial::to_json(&graph, &material).unwrap();
    assert!(matches!(
        serial::from_json(&json, &catalogues::animation()),
        Err(serial::GraphAssetError::CatalogueMismatch { .. })
    ));
}

#[test]
fn invalid_connections_in_a_hand_edited_asset_are_refused() {
    let catalogue = catalogues::material();
    let json = r#"{
      "version":1,
      "catalogue":"somnium.material",
      "next_id":2,
      "nodes":[
        {"id":0,"archetype":"material.scalar","position":[0.0,0.0]},
        {"id":1,"archetype":"material.scalar","position":[1.0,0.0]}
      ],
      "connections":[{
        "from":{"node":0,"index":0,"direction":"output"},
        "to":{"node":1,"index":0,"direction":"output"}
      }]
    }"#;
    assert!(matches!(
        serial::from_json(json, &catalogue),
        Err(serial::GraphAssetError::InvalidConnection(
            ConnectError::SameDirection
        ))
    ));
}

#[test]
fn connection_identity_includes_both_pin_ends() {
    let connection = Connection {
        from: PinRef::output(NodeId(1), 2),
        to: PinRef::input(NodeId(3), 4),
    };
    assert_ne!(connection.from, connection.to);
}

#[test]
fn property_and_graph_materials_compile_to_the_same_runtime_object() {
    let catalogue = catalogues::material();
    let mut graph = Graph::new();
    graph
        .add(&catalogue, "material.surface", Vec2::ZERO)
        .unwrap();
    let property = somnium_asset::material::MaterialAsset::default();
    let compiled = super::material::compile(&graph, &catalogue, &property).unwrap();
    assert_eq!(compiled.material, property);
    naga::front::wgsl::parse_str(&compiled.wgsl).expect("generated WGSL must validate");
}

#[test]
fn material_compile_uses_only_nodes_reaching_the_root() {
    let catalogue = catalogues::material();
    let mut graph = Graph::new();
    let scalar = graph
        .add(&catalogue, "material.scalar", Vec2::ZERO)
        .unwrap();
    graph
        .node_mut(scalar)
        .unwrap()
        .literals
        .insert(0, "0.25".into());
    let root = graph.add(&catalogue, "material.surface", Vec2::X).unwrap();
    graph
        .connect(
            &catalogue,
            PinRef::output(scalar, 0),
            PinRef::input(root, 1),
        )
        .unwrap();
    // Reachable texture nodes are deliberately unsupported in v1, but a dead
    // one must not poison a deterministic compile.
    graph.add(&catalogue, "material.texture", Vec2::Y).unwrap();
    let compiled = super::material::compile(
        &graph,
        &catalogue,
        &somnium_asset::material::MaterialAsset::default(),
    )
    .unwrap();
    assert_eq!(compiled.material.roughness, 0.25);
    assert!(!compiled.wgsl.contains("textureSample"));
    let mut shaders = somnium_shader::ShaderSystem::new();
    let key = compiled.install(&mut shaders);
    assert_eq!(shaders.source(key).unwrap(), compiled.wgsl);
}

#[test]
fn one_surface_gesture_is_one_undo_entry() {
    let mut surface = super::GraphSurface::new(catalogues::material());
    let node = surface
        .add("material.scalar", Vec2::new(10.0, 20.0))
        .unwrap();
    surface.selection.select_only(node);
    assert!(surface.move_selection(Vec2::new(5.0, -2.0)));
    assert_eq!(
        surface.history().labels(),
        ["Add Graph Node", "Move Graph Nodes"]
    );
    assert_eq!(
        surface.graph.node(node).unwrap().position,
        Vec2::new(15.0, 18.0)
    );
    assert!(surface.undo());
    assert_eq!(
        surface.graph.node(node).unwrap().position,
        Vec2::new(10.0, 20.0)
    );
    assert!(surface.redo());
    assert_eq!(
        surface.graph.node(node).unwrap().position,
        Vec2::new(15.0, 18.0)
    );
}

#[test]
fn control_registry_commands_drive_the_active_graph_history() {
    let mut surface = super::GraphSurface::new(catalogues::material());
    let id = surface.add("material.scalar", Vec2::ZERO).unwrap();
    surface.selection.select_only(id);

    for command in [
        "editor.edit.undo",
        "editor.edit.redo",
        "editor.edit.copy",
        "editor.edit.paste",
        "editor.edit.delete",
    ] {
        assert!(crate::commands::registry().get(command).is_some());
    }

    assert!(surface.dispatch_command("editor.edit.undo", Vec2::splat(24.0)));
    assert!(surface.graph.is_empty());
    assert!(surface.dispatch_command("editor.edit.redo", Vec2::splat(24.0)));
    surface.selection.select_only(id);
    assert!(surface.dispatch_command("editor.edit.copy", Vec2::splat(24.0)));
    assert!(surface.dispatch_command("editor.edit.paste", Vec2::splat(24.0)));
    assert_eq!(surface.graph.nodes().len(), 2);
    assert!(!surface.dispatch_command("not.a.command", Vec2::ZERO));
}

#[test]
fn retained_graph_control_draws_nodes_pins_and_the_bezier_wire() {
    let (catalogue, mut graph, scalar, output) = two_node_graph();
    graph
        .connect(
            &catalogue,
            PinRef::output(scalar, 0),
            PinRef::input(output, 1),
        )
        .unwrap();
    let mut ui = crate::ui::UserInterface::new(640.0, 360.0);
    let root = ui.root();
    let editor = super::GraphEditorBuilder::new(
        crate::widget::WidgetBuilder::new()
            .with_width(640.0)
            .with_height(360.0),
        catalogue,
    )
    .with_graph(graph)
    .build();
    ui.add_node(editor, root);
    ui.perform_layout();
    ui.draw();

    assert!(
        ui.draw_ctx.instances.len() >= 4,
        "canvas, grid, and both node bodies use the frozen primitive stream"
    );
    assert!(
        ui.draw_ctx.shaped.instances.len() >= 2,
        "pins and the MORROWIND-D bezier wire use the shaped stream"
    );
}

fn graph_node_rects(ui: &crate::ui::UserInterface) -> Vec<Rect> {
    let raised = crate::theme::active().semantic.surface.raised.bytes();
    let mut rects: Vec<_> = ui
        .draw_ctx
        .instances
        .iter()
        .filter(|primitive| primitive.fill_a == raised)
        .map(|primitive| {
            Rect::new(
                primitive.rect[0],
                primitive.rect[1],
                primitive.rect[2],
                primitive.rect[3],
            )
        })
        .collect();
    rects.sort_by(|left, right| left.x.total_cmp(&right.x));
    rects
}

fn wheel_zoomed_graph(delta: winit::event::MouseScrollDelta, repeats: usize) -> Vec<Rect> {
    use winit::event::{DeviceId, TouchPhase, WindowEvent};

    let (catalogue, graph, _, _) = two_node_graph();
    let mut ui = crate::ui::UserInterface::new(640.0, 360.0);
    let editor = super::GraphEditorBuilder::new(
        crate::widget::WidgetBuilder::new()
            .with_width(640.0)
            .with_height(360.0),
        catalogue,
    )
    .with_graph(graph)
    .build();
    ui.add_node(editor, ui.root());
    ui.perform_layout();
    ui.process_os_event(&WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: winit::dpi::PhysicalPosition::new(200.0, 150.0),
    });
    for _ in 0..repeats {
        assert!(ui.process_os_event(&WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta,
            phase: TouchPhase::Moved,
        }));
        ui.update();
    }
    ui.draw();
    graph_node_rects(&ui)
}

#[test]
fn one_routed_wheel_line_zooms_ten_percent_about_the_cursor() {
    let nodes = wheel_zoomed_graph(winit::event::MouseScrollDelta::LineDelta(0.0, 1.0), 1);
    let scalar = nodes.first().expect("the scalar node is drawn");

    assert!((scalar.w - 198.0).abs() < 0.01, "one tick: {scalar:?}");
    assert!(
        (scalar.x - 2.0).abs() < 0.01,
        "cursor-anchored x: {scalar:?}"
    );
    assert!(
        (scalar.y - 18.0).abs() < 0.01,
        "cursor-anchored y: {scalar:?}"
    );
}

#[test]
fn routed_line_and_pixel_wheel_deltas_have_the_same_smooth_scale() {
    let line = wheel_zoomed_graph(winit::event::MouseScrollDelta::LineDelta(0.0, 1.0), 1);
    let pixels = wheel_zoomed_graph(
        winit::event::MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, 20.0)),
        1,
    );
    let pixel_quarters = wheel_zoomed_graph(
        winit::event::MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, 5.0)),
        4,
    );

    assert_eq!(line.len(), pixels.len());
    assert_eq!(line.len(), pixel_quarters.len());
    for ((line, pixels), quarters) in line.iter().zip(&pixels).zip(&pixel_quarters) {
        for (actual, expected) in [line.x, line.y, line.w, line.h]
            .into_iter()
            .zip([pixels.x, pixels.y, pixels.w, pixels.h])
        {
            assert!((actual - expected).abs() < 0.01);
        }
        for (actual, expected) in [line.x, line.y, line.w, line.h]
            .into_iter()
            .zip([quarters.x, quarters.y, quarters.w, quarters.h])
        {
            assert!((actual - expected).abs() < 0.01);
        }
    }
}

#[test]
fn routed_extreme_pixel_deltas_stop_at_graph_zoom_bounds() {
    let maximum = wheel_zoomed_graph(
        winit::event::MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(
            0.0,
            1_000_000.0,
        )),
        1,
    );
    let minimum = wheel_zoomed_graph(
        winit::event::MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(
            0.0,
            -1_000_000.0,
        )),
        1,
    );

    assert!((maximum[0].w - 180.0 * GraphView::MAX_ZOOM).abs() < 0.01);
    assert!((minimum[0].w - 180.0 * GraphView::MIN_ZOOM).abs() < 0.01);
}

#[test]
fn routed_graph_literal_edit_is_visible_and_undoable() {
    let (catalogue, graph, scalar, _) = two_node_graph();
    let mut ui = crate::ui::UserInterface::new(640.0, 360.0);
    let root = ui.root();
    let editor = super::GraphEditorBuilder::new(
        crate::widget::WidgetBuilder::new()
            .with_width(640.0)
            .with_height(360.0),
        catalogue,
    )
    .with_graph(graph)
    .build();
    let editor = ui.add_node(editor, root);

    ui.send(super::GraphEditorMessage::set_literal(
        editor, scalar, 0, "0.75",
    ));
    let changed = ui
        .update()
        .into_iter()
        .find_map(
            |message| match message.data::<super::GraphEditorMessage>() {
                Some(super::GraphEditorMessage::Changed(graph)) => Some(graph.clone()),
                _ => None,
            },
        )
        .expect("a routed literal edit emits the authored graph");
    assert_eq!(
        changed
            .node(scalar)
            .unwrap()
            .literals
            .get(&0)
            .map(String::as_str),
        Some("0.75")
    );

    ui.send(super::GraphEditorMessage::command(
        editor,
        "editor.edit.undo",
        Vec2::ZERO,
    ));
    let undone = ui
        .update()
        .into_iter()
        .find_map(
            |message| match message.data::<super::GraphEditorMessage>() {
                Some(super::GraphEditorMessage::Changed(graph)) => Some(graph.clone()),
                _ => None,
            },
        )
        .expect("literal undo travels through the graph widget");
    assert!(!undone.node(scalar).unwrap().literals.contains_key(&0));
}

#[test]
fn routed_state_overlay_authors_draws_edits_and_undoes_one_document() {
    let catalogue = catalogues::animation();
    let mut surface = super::GraphSurface::new(catalogue.clone());
    let idle = surface
        .add("animation.state", Vec2::new(40.0, 60.0))
        .unwrap();
    let moving = surface
        .add("animation.state", Vec2::new(360.0, 180.0))
        .unwrap();
    let document = super::AnimationStateMachineDocument::new(surface);
    let mut ui = crate::ui::UserInterface::new(720.0, 420.0);
    let root = ui.root();
    let editor = super::GraphEditorBuilder::new(
        crate::widget::WidgetBuilder::new()
            .with_width(720.0)
            .with_height(420.0),
        catalogue,
    )
    .with_state_machine_document(document)
    .build();
    let editor = ui.add_node(editor, root);
    ui.perform_layout();
    ui.draw();
    let without_overlay = ui.draw_ctx.shaped.instances.len();

    ui.send(super::GraphEditorMessage::set_initial_state(editor, idle));
    ui.send(super::GraphEditorMessage::add_state_transition(
        editor,
        super::AuthoredStateTransition {
            from: idle,
            to: moving,
            conditions: vec![somnium_anim::Condition::Trigger {
                parameter: "move".into(),
            }],
            blend_seconds: 0.25,
            sync_track: Some("locomotion".into()),
        },
    ));
    let authored = ui
        .update()
        .into_iter()
        .filter_map(
            |message| match message.data::<super::GraphEditorMessage>() {
                Some(super::GraphEditorMessage::StateMachineChanged(document)) => {
                    Some(document.clone())
                }
                _ => None,
            },
        )
        .last()
        .expect("state authoring emits the complete durable document");
    assert_eq!(authored.initial(), Some(idle));
    assert_eq!(authored.transitions().len(), 1);

    ui.perform_layout();
    ui.draw();
    assert!(
        ui.draw_ctx.shaped.instances.len() > without_overlay,
        "the authored cyclic transition is drawn over the shared graph"
    );

    let mut edited = authored.transitions()[0].clone();
    edited.blend_seconds = 0.5;
    edited.sync_track = None;
    ui.send(super::GraphEditorMessage::set_state_transition(
        editor, 0, edited,
    ));
    let edited = ui
        .update()
        .into_iter()
        .find_map(
            |message| match message.data::<super::GraphEditorMessage>() {
                Some(super::GraphEditorMessage::StateMachineChanged(document)) => {
                    Some(document.clone())
                }
                _ => None,
            },
        )
        .expect("transition editing stays on the routed widget path");
    assert_eq!(edited.transitions()[0].blend_seconds, 0.5);
    assert_eq!(edited.transitions()[0].sync_track, None);

    ui.send(super::GraphEditorMessage::undo_state_overlay(editor));
    let undone = ui
        .update()
        .into_iter()
        .find_map(
            |message| match message.data::<super::GraphEditorMessage>() {
                Some(super::GraphEditorMessage::StateMachineChanged(document)) => {
                    Some(document.clone())
                }
                _ => None,
            },
        )
        .expect("overlay undo emits the restored document");
    assert_eq!(undone.transitions()[0].blend_seconds, 0.25);
    assert_eq!(
        undone.transitions()[0].sync_track.as_deref(),
        Some("locomotion")
    );
}

#[test]
fn canvas_transition_inspector_edits_every_field_and_deletes() {
    use crate::message::{
        KeyCode, MessageDirection, Modifiers, MouseButton, UiMessage, WidgetMessage,
    };

    let catalogue = catalogues::animation();
    let mut surface = super::GraphSurface::new(catalogue.clone());
    let idle = surface
        .add("animation.state", Vec2::new(40.0, 60.0))
        .unwrap();
    let moving = surface
        .add("animation.state", Vec2::new(280.0, 160.0))
        .unwrap();
    let mut ui = crate::ui::UserInterface::new(720.0, 420.0);
    let root = ui.root();
    let editor = ui.add_node(
        super::GraphEditorBuilder::new(
            crate::widget::WidgetBuilder::new()
                .with_width(720.0)
                .with_height(420.0),
            catalogue,
        )
        .with_state_machine_document(super::AnimationStateMachineDocument::new(surface))
        .build(),
        root,
    );
    ui.perform_layout();
    ui.send(super::GraphEditorMessage::add_state_transition(
        editor,
        super::AuthoredStateTransition {
            from: idle,
            to: moving,
            conditions: Vec::new(),
            blend_seconds: 0.2,
            sync_track: None,
        },
    ));
    ui.update();

    let click = |position| {
        UiMessage::new(
            editor,
            MessageDirection::ToWidget,
            WidgetMessage::MouseDown {
                pos: position,
                button: MouseButton::Left,
                mods: Modifiers::default(),
            },
        )
    };
    let text = |value: &str| {
        UiMessage::new(
            editor,
            MessageDirection::ToWidget,
            WidgetMessage::Text(value.to_owned()),
        )
    };
    let enter = || {
        UiMessage::new(
            editor,
            MessageDirection::ToWidget,
            WidgetMessage::KeyDown(KeyCode::Enter, Modifiers::default()),
        )
    };
    let changed_document = |messages: Vec<UiMessage>| {
        messages
            .into_iter()
            .find_map(
                |message| match message.data::<super::GraphEditorMessage>() {
                    Some(super::GraphEditorMessage::StateMachineChanged(document)) => {
                        Some(document.clone())
                    }
                    _ => None,
                },
            )
            .expect("an inspector edit emits the durable state-machine document")
    };

    // Panel layout for the fixed 720×420 editor: fields begin at x=510.
    ui.send(click(Vec2::new(520.0, 75.0)));
    ui.send(text("0.6"));
    ui.send(enter());
    let document = changed_document(ui.update());
    assert_eq!(document.transitions()[0].blend_seconds, 0.6);

    ui.send(click(Vec2::new(520.0, 105.0)));
    ui.send(text("locomotion"));
    ui.send(enter());
    let document = changed_document(ui.update());
    assert_eq!(
        document.transitions()[0].sync_track.as_deref(),
        Some("locomotion")
    );

    ui.send(click(Vec2::new(520.0, 135.0)));
    ui.send(text("float:speed:greater:0.4; trigger:move"));
    ui.send(enter());
    let document = changed_document(ui.update());
    assert!(matches!(
        document.transitions()[0].conditions.as_slice(),
        [
            somnium_anim::Condition::Float {
                parameter,
                op: somnium_anim::CompareOp::Greater,
                value
            },
            somnium_anim::Condition::Trigger { parameter: trigger }
        ] if parameter == "speed" && (*value - 0.4).abs() < f32::EPSILON && trigger == "move"
    ));

    ui.send(click(Vec2::new(630.0, 182.0)));
    let document = changed_document(ui.update());
    assert!(document.transitions().is_empty());
}

#[test]
fn animation_catalogue_compiles_to_the_ui_neutral_runtime_graph() {
    use glam::{Mat4, Quat, Vec3};
    use somnium_anim::{
        AnimationClip, ClipId, GraphId, Keyframe, ParameterDefinition, ParameterSchema,
        ParameterSchemaId, ParameterValue, PoseCache, Skeleton, SkeletonId, Transform,
        TransformTrack,
    };

    let skeleton = Skeleton::new(
        SkeletonId(41),
        vec!["root".into()],
        vec![somnium_anim::NO_PARENT],
        vec![Mat4::IDENTITY],
        vec![Transform::IDENTITY],
    )
    .unwrap()
    .0;
    let make_clip = |id, distance| {
        AnimationClip::new(
            ClipId(id),
            &skeleton,
            1.0,
            vec![TransformTrack {
                joint: 0,
                translation: vec![
                    Keyframe::new(0.0, Vec3::ZERO),
                    Keyframe::new(1.0, Vec3::X * distance),
                ],
                rotation: vec![Keyframe::new(0.0, Quat::IDENTITY)],
                scale: vec![],
            }],
            vec![],
        )
        .unwrap()
    };
    let parameters = ParameterSchema::new(
        ParameterSchemaId(7),
        vec![ParameterDefinition::new(
            "speed",
            ParameterValue::Float(0.5),
        )],
    )
    .unwrap();
    let mut values = parameters.instantiate();
    values.set("speed", ParameterValue::Float(0.5)).unwrap();

    let catalogue = catalogues::animation();
    let mut graph = Graph::new();
    let slow = graph.add(&catalogue, "animation.clip", Vec2::ZERO).unwrap();
    let fast = graph
        .add(&catalogue, "animation.clip", Vec2::new(0.0, 120.0))
        .unwrap();
    graph.node_mut(slow).unwrap().literals.insert(0, "1".into());
    graph.node_mut(fast).unwrap().literals.insert(0, "2".into());
    let blend = graph
        .add(&catalogue, "animation.blend1d", Vec2::new(240.0, 60.0))
        .unwrap();
    let output = graph
        .add(&catalogue, "animation.output", Vec2::new(500.0, 60.0))
        .unwrap();
    let idle = graph
        .add(&catalogue, "animation.state", Vec2::new(0.0, 260.0))
        .unwrap();
    let moving = graph
        .add(&catalogue, "animation.state", Vec2::new(240.0, 260.0))
        .unwrap();
    graph.node_mut(idle).unwrap().title = "Idle".into();
    graph.node_mut(idle).unwrap().literals.insert(1, "0".into());
    graph.node_mut(moving).unwrap().title = "Moving".into();
    graph
        .node_mut(moving)
        .unwrap()
        .literals
        .insert(1, "1".into());
    for (from, to) in [
        (PinRef::output(slow, 0), PinRef::input(blend, 0)),
        (PinRef::output(fast, 0), PinRef::input(blend, 1)),
        (PinRef::output(blend, 0), PinRef::input(output, 0)),
        (PinRef::output(slow, 0), PinRef::input(idle, 0)),
        (PinRef::output(fast, 0), PinRef::input(moving, 0)),
    ] {
        graph.connect(&catalogue, from, to).unwrap();
    }

    let compiled = super::compile_animation_document(
        &graph,
        &catalogue,
        GraphId(3),
        7,
        &skeleton,
        vec![make_clip(1, 1.0), make_clip(2, 3.0)],
        parameters,
    )
    .unwrap();
    let runtime = compiled.asset();
    let pose = runtime
        .evaluate(&skeleton, &values, 0.5, 1, &mut PoseCache::default())
        .unwrap();
    assert!((pose.local[0].translation.x - 1.0).abs() < 1e-5);
    assert_eq!(runtime.nodes().len(), 3);
    assert_eq!(runtime.version(), 7);

    // The cyclic state-machine overlay reuses the same K surface without
    // weakening the pose graph's acyclic wire invariant. States connect to
    // durable authored pose nodes, never compiler-generated runtime indices.
    let mut states = super::GraphSurface::new(catalogue);
    states.graph = graph;
    let mut document = super::AnimationStateMachineDocument::new(states);
    assert!(document.set_initial(idle));
    assert!(document.add_transition(super::AuthoredStateTransition {
        from: idle,
        to: moving,
        conditions: vec![somnium_anim::Condition::Float {
            parameter: "speed".into(),
            op: somnium_anim::CompareOp::Greater,
            value: 0.4,
        }],
        blend_seconds: 0.2,
        sync_track: None,
    }));
    assert_eq!(document.undo_overlay(), Some("Add Animation Transition"));
    assert_eq!(document.redo_overlay(), Some("Add Animation Transition"));
    let json = document.to_json().unwrap();
    let document =
        super::AnimationStateMachineDocument::from_json(&json, catalogues::animation()).unwrap();
    assert_eq!(json, document.to_json().unwrap());
    let machine =
        super::compile_state_machine(&document, &compiled, somnium_anim::MachineId(8), 1).unwrap();
    assert_eq!(machine.states()[0].name, "Idle");
    assert_eq!(machine.transitions().len(), 1);
    let mut player = somnium_anim::StateMachinePlayer::new(&machine);
    player
        .advance(&machine, runtime, &mut values, 0.01)
        .unwrap();
    assert!(player.is_transitioning());
}

#[test]
fn animation_catalogue_authors_multitriangle_masks_parameters_and_sync_leaders() {
    use glam::Mat4;
    use somnium_anim::{
        AnimNode, AnimationClip, ClipId, GraphId, LayerWeight, ParameterDefinition,
        ParameterSchema, ParameterSchemaId, ParameterValue, Skeleton, SkeletonId, Transform,
    };

    let skeleton = Skeleton::new(
        SkeletonId(42),
        vec!["root".into(), "hand".into()],
        vec![somnium_anim::NO_PARENT, 0],
        vec![Mat4::IDENTITY; 2],
        vec![Transform::IDENTITY; 2],
    )
    .unwrap()
    .0;
    let parameters = ParameterSchema::new(
        ParameterSchemaId(8),
        vec![
            ParameterDefinition::new("x", ParameterValue::Float(0.0)),
            ParameterDefinition::new("y", ParameterValue::Float(0.0)),
            ParameterDefinition::new("upper", ParameterValue::Float(0.5)),
        ],
    )
    .unwrap();
    let catalogue = catalogues::animation();
    let mut graph = Graph::new();
    let clips: Vec<_> = (0..4)
        .map(|index| {
            let node = graph
                .add(
                    &catalogue,
                    "animation.clip",
                    Vec2::new(0.0, index as f32 * 100.0),
                )
                .unwrap();
            graph
                .node_mut(node)
                .unwrap()
                .literals
                .insert(0, (index + 1).to_string());
            node
        })
        .collect();
    let blend = graph
        .add(&catalogue, "animation.blend2d4", Vec2::new(260.0, 120.0))
        .unwrap();
    graph
        .node_mut(blend)
        .unwrap()
        .literals
        .insert(16, "3".into());
    let layer = graph
        .add(&catalogue, "animation.layer", Vec2::new(500.0, 120.0))
        .unwrap();
    graph
        .node_mut(layer)
        .unwrap()
        .literals
        .extend([(3, "upper".into()), (4, "1.0,0.0".into())]);
    let output = graph
        .add(&catalogue, "animation.output", Vec2::new(720.0, 120.0))
        .unwrap();
    for (index, clip) in clips.iter().copied().enumerate() {
        graph
            .connect(
                &catalogue,
                PinRef::output(clip, 0),
                PinRef::input(blend, index as u16),
            )
            .unwrap();
    }
    for (from, to) in [
        (PinRef::output(blend, 0), PinRef::input(layer, 0)),
        (PinRef::output(clips[0], 0), PinRef::input(layer, 1)),
        (PinRef::output(layer, 0), PinRef::input(output, 0)),
    ] {
        graph.connect(&catalogue, from, to).unwrap();
    }
    let runtime = super::compile_animation(
        &graph,
        &catalogue,
        GraphId(4),
        2,
        &skeleton,
        clips
            .iter()
            .enumerate()
            .map(|(index, _)| {
                AnimationClip::new(ClipId(index as u64 + 1), &skeleton, 1.0, vec![], vec![])
                    .unwrap()
            })
            .collect(),
        parameters,
    )
    .unwrap();
    assert!(matches!(
        &runtime.nodes()[4],
        AnimNode::Blend2D {
            triangles,
            sync_leader: 3,
            ..
        } if triangles == &vec![[0, 1, 2], [0, 2, 3]]
    ));
    assert!(matches!(
        &runtime.nodes()[5],
        AnimNode::Layer { layers, .. }
            if matches!(layers[0].weight, LayerWeight::Parameter(ref name) if name == "upper")
                && layers[0].mask.is_some()
    ));
}

#[test]
fn moving_a_group_moves_every_nested_member_and_serialises_membership() {
    let catalogue = catalogues::material();
    let mut surface = super::GraphSurface::new(catalogue.clone());
    let a = surface
        .add("material.scalar", Vec2::new(10.0, 10.0))
        .unwrap();
    let b = surface
        .add("material.scalar", Vec2::new(20.0, 20.0))
        .unwrap();
    surface.selection.select_only(a);
    surface.selection.toggle(b);
    let group = surface
        .group_selection(Vec2::ZERO, Vec2::new(200.0, 120.0))
        .unwrap();
    assert_eq!(surface.graph.node(a).unwrap().group, Some(group));
    surface.selection.select_only(group);
    surface.move_selection(Vec2::new(30.0, 0.0));
    assert_eq!(surface.graph.node(a).unwrap().position.x, 40.0);
    assert_eq!(surface.graph.node(b).unwrap().position.x, 50.0);
    let json = serial::to_json(&surface.graph, &catalogue).unwrap();
    let loaded = serial::from_json(&json, &catalogue).unwrap();
    assert_eq!(loaded.node(a).unwrap().group, Some(group));
}
