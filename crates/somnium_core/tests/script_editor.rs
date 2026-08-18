//! Phase 16-D: the editor half — attaching, authoring and importing.
//!
//! The widget tree is not testable without a window, so what is tested
//! here is everything underneath it: the undo commands the panel emits,
//! the file import the Content Drawer triggers, and the template the
//! "New Script" button writes. Those are where the behaviour lives; the
//! widgets only send the messages.

use somnium_core::editor_commands::{
    AttachScriptCmd, DetachScriptCmd, ReorderScriptCmd, SetScriptEnabledCmd, SetScriptPropertyCmd,
    UndoStack,
};
use somnium_core::script_host::{NEW_SCRIPT_TEMPLATE, ScriptHost, display_path};
use somnium_core::{Name, Transform};
use somnium_ecs::{Entity, World};
use somnium_script::attachment::ScriptSet;
use somnium_script::backend::Budget;
use somnium_script::ids::ScriptAssetId;
use somnium_script::value::ScriptValue;

fn world_with_entity() -> (World, Entity) {
    let mut world = World::new();
    let entity = world.spawn((Name::new("Subject"), Transform::default()));
    (world, entity)
}

fn set(world: &World, entity: Entity) -> ScriptSet {
    world.get::<ScriptSet>(entity).cloned().unwrap_or_default()
}

/// A scratch directory of this test's own, cleaned up on the way out.
fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "somnium_script_editor_{}_{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ── Undo ───────────────────────────────────────────────────────────────

#[test]
fn attaching_a_script_is_one_undo_step() {
    let (mut world, entity) = world_with_entity();
    let mut selected = Some(entity);
    let mut stack = UndoStack::new(8);
    let asset = ScriptAssetId::from_path("scripts/a.luau");

    stack.push(
        Box::new(AttachScriptCmd::new(entity.index(), asset)),
        &mut world,
        &mut selected,
    );
    assert_eq!(set(&world, entity).len(), 1);
    let instance = set(&world, entity).attachments[0].instance;

    assert!(stack.undo(&mut world, &mut selected));
    assert_eq!(set(&world, entity).len(), 0);

    assert!(stack.redo(&mut world, &mut selected));
    assert_eq!(
        set(&world, entity).attachments[0].instance,
        instance,
        "a redo must restore the same attachment identity — migrated state \
         is keyed by it"
    );
}

#[test]
fn removing_a_script_and_undoing_puts_it_back_where_it_was() {
    let (mut world, entity) = world_with_entity();
    let mut selected = Some(entity);
    let mut stack = UndoStack::new(8);
    for name in ["a", "b", "c"] {
        stack.push(
            Box::new(AttachScriptCmd::new(
                entity.index(),
                ScriptAssetId::from_path(&format!("scripts/{name}.luau")),
            )),
            &mut world,
            &mut selected,
        );
    }
    let before: Vec<_> = set(&world, entity)
        .attachments
        .iter()
        .map(|a| a.asset)
        .collect();

    stack.push(
        Box::new(DetachScriptCmd::new(entity.index(), 1)),
        &mut world,
        &mut selected,
    );
    assert_eq!(set(&world, entity).len(), 2);
    assert_eq!(set(&world, entity).attachments[1].asset, before[2]);

    stack.undo(&mut world, &mut selected);
    let after: Vec<_> = set(&world, entity)
        .attachments
        .iter()
        .map(|a| a.asset)
        .collect();
    assert_eq!(after, before, "the middle row comes back in the middle");
}

#[test]
fn reordering_renumbers_execution_order_and_undo_restores_the_authored_values() {
    let (mut world, entity) = world_with_entity();
    let mut selected = Some(entity);
    let mut stack = UndoStack::new(8);
    for name in ["a", "b"] {
        stack.push(
            Box::new(AttachScriptCmd::new(
                entity.index(),
                ScriptAssetId::from_path(&format!("scripts/{name}.luau")),
            )),
            &mut world,
            &mut selected,
        );
    }
    // Authored values a reorder is about to overwrite.
    {
        let mut authored = set(&world, entity);
        authored.attachments[0].execution_order = -50;
        authored.attachments[1].execution_order = 90;
        world.insert_component(entity, authored).unwrap();
    }
    let first = set(&world, entity).attachments[0].asset;

    stack.push(
        Box::new(ReorderScriptCmd::new(entity.index(), 0, 1)),
        &mut world,
        &mut selected,
    );
    let moved = set(&world, entity);
    assert_eq!(moved.attachments[1].asset, first, "it moved down");
    assert_eq!(
        (
            moved.attachments[0].execution_order,
            moved.attachments[1].execution_order
        ),
        (0, 1),
        "the list order becomes the run order"
    );

    stack.undo(&mut world, &mut selected);
    let back = set(&world, entity);
    assert_eq!(back.attachments[0].asset, first);
    assert_eq!(
        (
            back.attachments[0].execution_order,
            back.attachments[1].execution_order
        ),
        (-50, 90),
        "undo restores what the author had, not a tidy 0,1"
    );
}

#[test]
fn switching_an_attachment_off_is_undoable() {
    let (mut world, entity) = world_with_entity();
    let mut selected = Some(entity);
    let mut stack = UndoStack::new(8);
    stack.push(
        Box::new(AttachScriptCmd::new(
            entity.index(),
            ScriptAssetId::from_path("scripts/a.luau"),
        )),
        &mut world,
        &mut selected,
    );
    assert!(set(&world, entity).attachments[0].enabled);

    stack.push(
        Box::new(SetScriptEnabledCmd::new(entity.index(), 0, false)),
        &mut world,
        &mut selected,
    );
    assert!(!set(&world, entity).attachments[0].enabled);
    stack.undo(&mut world, &mut selected);
    assert!(set(&world, entity).attachments[0].enabled);
}

#[test]
fn undoing_a_property_edit_removes_the_override_rather_than_writing_the_default() {
    // The distinction matters: a scene that records `speed = 4.0` because
    // someone dragged the field and undid it is no longer following the
    // script's default when the author changes it.
    let (mut world, entity) = world_with_entity();
    let mut selected = Some(entity);
    let mut stack = UndoStack::new(8);
    stack.push(
        Box::new(AttachScriptCmd::new(
            entity.index(),
            ScriptAssetId::from_path("scripts/a.luau"),
        )),
        &mut world,
        &mut selected,
    );
    assert!(set(&world, entity).attachments[0].properties.is_empty());

    stack.push(
        Box::new(SetScriptPropertyCmd::new(
            entity.index(),
            0,
            "speed".into(),
            ScriptValue::F64(9.0),
        )),
        &mut world,
        &mut selected,
    );
    assert_eq!(
        set(&world, entity).attachments[0].properties["speed"],
        ScriptValue::F64(9.0)
    );

    stack.undo(&mut world, &mut selected);
    assert!(
        !set(&world, entity).attachments[0]
            .properties
            .contains_key("speed"),
        "there was no override before, so there must be none after"
    );

    stack.redo(&mut world, &mut selected);
    assert_eq!(
        set(&world, entity).attachments[0].properties["speed"],
        ScriptValue::F64(9.0)
    );
}

#[test]
fn a_second_edit_of_the_same_property_undoes_to_the_first_value() {
    let (mut world, entity) = world_with_entity();
    let mut selected = Some(entity);
    let mut stack = UndoStack::new(8);
    stack.push(
        Box::new(AttachScriptCmd::new(
            entity.index(),
            ScriptAssetId::from_path("scripts/a.luau"),
        )),
        &mut world,
        &mut selected,
    );
    for value in [2.0, 5.0] {
        stack.push(
            Box::new(SetScriptPropertyCmd::new(
                entity.index(),
                0,
                "speed".into(),
                ScriptValue::F64(value),
            )),
            &mut world,
            &mut selected,
        );
    }
    stack.undo(&mut world, &mut selected);
    assert_eq!(
        set(&world, entity).attachments[0].properties["speed"],
        ScriptValue::F64(2.0)
    );
}

// ── Import ─────────────────────────────────────────────────────────────

#[test]
fn the_new_script_template_compiles_and_declares_what_the_panel_draws() {
    let mut host = ScriptHost::new(Budget::default());
    let asset = ScriptAssetId::from_path("scripts/NewScript.luau");
    host.load_script(asset, "NewScript.luau", NEW_SCRIPT_TEMPLATE)
        .unwrap_or_else(|d| panic!("the template a new script starts from must compile:\n{d}"));

    let schema = host.runtime().asset_schema(asset).unwrap();
    assert_eq!(schema.fields.len(), 1, "one property, so the panel is not empty");
    assert_eq!(schema.fields[0].name, "speed");
    assert!(
        schema
            .callbacks
            .has(somnium_script::backend::Callback::FixedUpdate),
        "and an onFixedUpdate to type into"
    );
}

#[test]
fn importing_the_same_file_twice_is_a_reload_not_a_second_asset() {
    let dir = scratch("import");
    let path = dir.join("thing.luau");
    std::fs::write(&path, NEW_SCRIPT_TEMPLATE).unwrap();

    let mut host = ScriptHost::new(Budget::default());
    let first = host.import_script_file(&path).unwrap();
    let second = host.import_script_file(&path).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        host.runtime().assets().count(),
        1,
        "a re-import must not leave two assets pointing at one file"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn importing_a_file_that_does_not_compile_reports_it_and_loads_nothing() {
    let dir = scratch("broken");
    let path = dir.join("broken.luau");
    std::fs::write(&path, "return Script.define({").unwrap();

    let mut host = ScriptHost::new(Budget::default());
    let error = host.import_script_file(&path).unwrap_err();
    assert!(error.has_errors());
    assert_eq!(host.runtime().assets().count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn f5_recompiles_from_disk_and_a_broken_edit_leaves_the_old_module_live() {
    let dir = scratch("reload");
    let path = dir.join("live.luau");
    std::fs::write(
        &path,
        "return Script.define({ onFixedUpdate = function(self, ctx, dt) end })",
    )
    .unwrap();

    let mut host = ScriptHost::new(Budget::default());
    let asset = host.import_script_file(&path).unwrap();
    assert!(host.runtime().asset_schema(asset).is_some());

    // A good edit.
    std::fs::write(
        &path,
        "return Script.define({ schemaVersion = 7, \
         onFixedUpdate = function(self, ctx, dt) end })",
    )
    .unwrap();
    assert_eq!(host.reload_all_from_disk(), (1, 0));
    assert_eq!(host.runtime().asset_schema(asset).unwrap().schema_version, 7);

    // A bad one. The module on record must not change.
    std::fs::write(&path, "return Script.define({").unwrap();
    assert_eq!(host.reload_all_from_disk(), (0, 1));
    assert_eq!(
        host.runtime().asset_schema(asset).unwrap().schema_version,
        7,
        "a syntax error must leave the last good module in place"
    );
    assert!(
        !host.take_diagnostics().is_empty(),
        "and it must say why, in the Output Log"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_display_path_is_platform_neutral_so_the_asset_id_is_too() {
    let shown = display_path(std::path::Path::new("assets\\scripts\\a.luau"));
    assert!(!shown.contains('\\'), "got {shown}");
    assert_eq!(
        ScriptAssetId::from_path(&shown),
        ScriptAssetId::from_path("assets/scripts/a.luau")
    );
}
