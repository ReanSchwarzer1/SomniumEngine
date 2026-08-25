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
