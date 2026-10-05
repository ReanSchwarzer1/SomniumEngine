//! Entity clipboard: copy and paste a subtree with handle remapping.
//!
//! Fyrox's `scene/clipboard.rs` is the precedent, and its central idea is the
//! one that makes this work across scenes: the clipboard holds *values*, never
//! [`Entity`] handles. A copied subtree is a tree of snapshots; parent and
//! child links are rebuilt from the tree's own shape at paste time, so a
//! handle that was valid in the source scene can never leak into the
//! destination one.

use crate::editor_commands::{EditorCommand, EntitySnapshot};
use crate::{Children, Parent};
use somnium_ecs::{Entity, World};
use std::collections::HashMap;

/// One copied entity and everything beneath it.
#[derive(Clone)]
pub struct ClipNode {
    source: Entity,
    snapshot: EntitySnapshot,
    scripts: Option<somnium_script::attachment::ScriptSet>,
    children: Vec<ClipNode>,
}

/// A copied forest. Empty until something is copied; a paste of an empty
/// clipboard is a no-op rather than an error, which is what every editor does.
#[derive(Clone, Default)]
pub struct EntityClipboard {
    roots: Vec<ClipNode>,
    external_ids: HashMap<Entity, somnium_ecs::PersistentId>,
}

impl EntityClipboard {
    /// Whether nothing has been copied yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// How many top-level entities a paste would create.
    #[must_use]
    pub fn root_count(&self) -> usize {
        self.roots.len()
    }

    /// Copy a selection, keeping only its *canonical roots*.
    ///
    /// Selecting a parent and its child and copying both would otherwise paste
    /// the child twice: once as a root and once inside its parent's subtree.
    /// Dropping any selected entity that has a selected ancestor is the same
    /// rule the reparent path uses, and it is why copy and drag agree about
    /// what "these entities" means.
    #[must_use]
    pub fn copy(world: &World, selection: &[Entity]) -> Self {
        let roots: Vec<_> = selection
            .iter()
            .copied()
            .filter(|entity| !has_selected_ancestor(world, *entity, selection))
            .map(|entity| capture(world, entity))
            .collect();
        let external_ids = world
            .entities()
            .filter_map(|entity| world.persistent_id(entity).map(|id| (entity, id)))
            .collect();
        Self {
            roots,
            external_ids,
        }
    }
}

fn has_selected_ancestor(world: &World, entity: Entity, selection: &[Entity]) -> bool {
    let mut cursor = parent_of(world, entity);
    while let Some(current) = cursor {
        if selection.contains(&current) {
            return true;
        }
        cursor = parent_of(world, current);
    }
    false
}

fn parent_of(world: &World, entity: Entity) -> Option<Entity> {
    world
        .get::<Parent>(entity)
        .map(|parent| parent.entity)
        .filter(|parent| world.is_alive(*parent))
}

fn capture(world: &World, entity: Entity) -> ClipNode {
    let mut snapshot = EntitySnapshot::capture(world, entity);
    // Links are rebuilt from the tree, so the captured handles are dead
    // weight — and worse, they are handles into the *source* world.
    snapshot.parent = None;
    snapshot.children = None;
    // A clipboard clone is a new authored entity, whereas a deletion snapshot
    // restores the old one. Generic copy breaks prefab linkage: sharing the
    // original instance root would make edits propagate into unrelated copies.
    snapshot.persistent_id = None;
    snapshot.prefab = None;
    let children = world
        .entities()
        .filter(|child| parent_of(world, *child) == Some(entity))
        .map(|child| capture(world, child))
        .collect();
    ClipNode {
        source: entity,
        snapshot,
        scripts: world
            .get::<somnium_script::attachment::ScriptSet>(entity)
            .cloned(),
        children,
    }
}

/// Paste the clipboard under an optional parent, as exactly one undo step.
pub struct PasteEntitiesCmd {
    clipboard: EntityClipboard,
    parent: Option<Entity>,
    /// Everything the last `execute` created, deepest last, so undo can walk
    /// it backwards and never despawn a parent before its children.
    spawned: Vec<Entity>,
    /// The roots, in clipboard order — what the paste selects afterwards.
    roots: Vec<Entity>,
}

impl PasteEntitiesCmd {
    /// Build the paste. Nothing is spawned until `execute`.
    #[must_use]
    pub fn new(mut clipboard: EntityClipboard, parent: Option<Entity>) -> Self {
        fn assign_identity(node: &mut ClipNode) {
            node.snapshot.persistent_id = Some(somnium_ecs::PersistentId::mint());
            if let Some(scripts) = &mut node.scripts {
                for attachment in &mut scripts.attachments {
                    attachment.instance = somnium_script::ids::InstanceUuid::mint();
                }
            }
            for child in &mut node.children {
                assign_identity(child);
            }
        }
        for node in &mut clipboard.roots {
            assign_identity(node);
        }
        Self {
            clipboard,
            parent,
            spawned: Vec::new(),
            roots: Vec::new(),
        }
    }

    /// The top-level entities the paste created. Empty before `execute`.
    #[must_use]
    pub fn roots(&self) -> &[Entity] {
        &self.roots
    }

    fn spawn_node(&mut self, world: &mut World, node: &ClipNode, parent: Option<Entity>) -> Entity {
        let entity = node.snapshot.clone().respawn(world);
        self.spawned.push(entity);
        if let Some(scripts) = &node.scripts {
            let _ = world.insert_component(entity, scripts.clone());
        }
        if let Some(parent) = parent {
            attach(world, entity, parent);
        }
        for child in &node.children {
            self.spawn_node(world, child, Some(entity));
        }
        entity
    }
}

fn attach(world: &mut World, child: Entity, parent: Entity) {
    let _ = world.insert_component(child, Parent { entity: parent });
    if let Some(children) = world.get_mut::<Children>(parent) {
        children.push(child);
    } else {
        let mut children = Children::empty();
        children.push(child);
        let _ = world.insert_component(parent, children);
    }
}

impl EditorCommand for PasteEntitiesCmd {
    fn execute(&mut self, world: &mut World, selected: &mut Option<Entity>) {
        self.spawned.clear();
        self.roots.clear();
        let roots = self.clipboard.roots.clone();
        let parent = self.parent.filter(|parent| world.is_alive(*parent));
        for node in &roots {
            let entity = self.spawn_node(world, node, parent);
            self.roots.push(entity);
        }
        // Internal targets follow the copy; external targets resolve by durable
        // identity. Raw handles can collide with unrelated entities in another
        // scene, so an external target without durable identity is cleared.
        fn sources(node: &ClipNode, out: &mut Vec<Entity>) {
            out.push(node.source);
            for child in &node.children {
                sources(child, out);
            }
        }
        let mut original = Vec::new();
        for root in &roots {
            sources(root, &mut original);
        }
        let mapping: std::collections::HashMap<_, _> = original
            .into_iter()
            .zip(self.spawned.iter().copied())
            .collect();
        let registry = crate::reflect_registry::component_registry();
        let mut references = self
            .clipboard
            .external_ids
            .iter()
            .filter_map(|(source, id)| {
                world
                    .entity_by_persistent_id(*id)
                    .map(|entity| (*source, entity))
            })
            .collect::<HashMap<_, _>>();
        references.extend(mapping);
        fn remap(value: &mut somnium_ecs::ReflectValue, mapping: &HashMap<Entity, Entity>) {
            use somnium_ecs::ReflectValue;
            match value {
                ReflectValue::Entity(target) => {
                    *target = target.and_then(|old| mapping.get(&old).copied())
                }
                ReflectValue::Array(values) => {
                    for value in values {
                        remap(value, mapping);
                    }
                }
                ReflectValue::Object(values) => {
                    for value in values.values_mut() {
                        remap(value, mapping);
                    }
                }
                ReflectValue::Map(values) => {
                    for value in values.values_mut() {
                        remap(value, mapping);
                    }
                }
                _ => {}
            }
        }
        for &entity in &self.spawned {
            for schema in registry.schemas_on(world, entity) {
                if schema.stable_id.as_str() == "somnium.Parent" {
                    continue;
                }
                if let Some(mut values) = (schema.snapshot)(world, entity) {
                    for value in values.values_mut() {
                        remap(value, &references);
                    }
                    let _ = (schema.apply)(world, entity, &values);
                }
            }
            if let Some(scripts) = world.get_mut::<somnium_script::attachment::ScriptSet>(entity) {
                for attachment in &mut scripts.attachments {
                    for value in attachment.properties.values_mut() {
                        remap(value, &references);
                    }
                }
            }
        }
        if let Some(last) = self.roots.last() {
            *selected = Some(*last);
        }
    }

    fn undo(&mut self, world: &mut World, selected: &mut Option<Entity>) {
        for entity in self.spawned.iter().rev() {
            if let Some(parent) = parent_of(world, *entity)
                && let Some(children) = world.get_mut::<Children>(parent)
            {
                children.remove(*entity);
            }
            world.despawn(*entity);
        }
        if selected.is_some_and(|entity| !world.is_alive(entity)) {
            *selected = None;
        }
        self.spawned.clear();
        self.roots.clear();
    }

    fn description(&self) -> &str {
        "Paste"
    }

    fn is_no_op(&self) -> bool {
        self.clipboard.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_commands::UndoStack;
    use crate::{MeshComponent, Name, Transform, WorldTransform};

    #[test]
    fn copy_remaps_generic_and_script_targets_with_distinct_redo_stable_attachments() {
        use somnium_ecs::ReflectValue;
        use somnium_script::{
            attachment::{ScriptAttachment, ScriptSet},
            ids::ScriptAssetId,
        };
        let mut world = World::new();
        let target = world.spawn((Name::new("target"), Transform::default()));
        let mut attachment = ScriptAttachment::new(ScriptAssetId::mint());
        let original_instance = attachment.instance;
        attachment.properties.insert(
            "nested".into(),
            ReflectValue::Map(std::collections::BTreeMap::from([(
                "target".into(),
                ReflectValue::Entity(Some(target)),
            )])),
        );
        let sensor = world.spawn((
            Name::new("sensor"),
            Transform::default(),
            crate::ai::PerceptionComponent {
                target,
                ..Default::default()
            },
            ScriptSet {
                attachments: vec![attachment],
            },
        ));
        let mut paste =
            PasteEntitiesCmd::new(EntityClipboard::copy(&world, &[sensor, target]), None);
        let mut selected = None;
        paste.execute(&mut world, &mut selected);
        let (copy_sensor, copy_target) = (paste.roots()[0], paste.roots()[1]);
        assert_eq!(
            world
                .get::<crate::ai::PerceptionComponent>(copy_sensor)
                .unwrap()
                .target,
            copy_target
        );
        let script = &world.get::<ScriptSet>(copy_sensor).unwrap().attachments[0];
        let copied_instance = script.instance;
        assert_ne!(copied_instance, original_instance);
        assert_eq!(
            script.properties["nested"],
            ReflectValue::Map(std::collections::BTreeMap::from([(
                "target".into(),
                ReflectValue::Entity(Some(copy_target))
            )]))
        );
        paste.undo(&mut world, &mut selected);
        paste.execute(&mut world, &mut selected);
        assert_eq!(
            world
                .get::<ScriptSet>(paste.roots()[0])
                .unwrap()
                .attachments[0]
                .instance,
            copied_instance
        );
        assert_eq!(
            world
                .get::<crate::ai::PerceptionComponent>(paste.roots()[0])
                .unwrap()
                .target,
            paste.roots()[1]
        );
    }

    #[test]
    fn cross_scene_external_reference_resolves_identity_and_never_a_colliding_handle() {
        let mut source = World::new();
        let target = source.spawn((Name::new("target"),));
        let id = source.ensure_persistent_id(target).unwrap();
        let sensor = source.spawn((crate::ai::PerceptionComponent {
            target,
            ..Default::default()
        },));
        let clipboard = EntityClipboard::copy(&source, &[sensor]);
        let mut destination = World::new();
        let unrelated = destination.spawn((Name::new("different scene"),));
        assert_eq!(
            unrelated, target,
            "fixture exercises the raw-handle collision"
        );
        let mut selected = None;
        let mut paste = PasteEntitiesCmd::new(clipboard.clone(), None);
        paste.execute(&mut destination, &mut selected);
        assert_eq!(
            destination
                .get::<crate::ai::PerceptionComponent>(selected.unwrap())
                .unwrap()
                .target,
            Entity::DANGLING
        );
        let actual_target = destination.spawn((Name::new("same durable target"),));
        destination.set_persistent_id(actual_target, id).unwrap();
        let mut paste = PasteEntitiesCmd::new(clipboard, None);
        paste.execute(&mut destination, &mut selected);
        assert_eq!(
            destination
                .get::<crate::ai::PerceptionComponent>(selected.unwrap())
                .unwrap()
                .target,
            actual_target
        );
    }

    #[test]
    fn repeated_pastes_get_new_identity_break_prefab_links_and_redo_keeps_identity() {
        let mut world = World::new();
        let source = world.spawn((
            Transform::default(),
            crate::blockout::BlockoutComponent::default(),
        ));
        let source_id = world.ensure_persistent_id(source).unwrap();
        world
            .insert_component(
                source,
                crate::prefab::PrefabMember {
                    template: somnium_asset::database::AssetId::from_relative_path(
                        "test.somprefab",
                    ),
                    source: "test.somprefab".into(),
                    root: source_id.to_string(),
                    path: vec![source_id.to_string()],
                    baseline: serde_json::json!({}),
                    orphaned: Vec::new(),
                },
            )
            .unwrap();
        let clipboard = EntityClipboard::copy(&world, &[source]);
        let mut first = PasteEntitiesCmd::new(clipboard.clone(), None);
        let mut selected = None;
        first.execute(&mut world, &mut selected);
        let first_entity = selected.unwrap();
        let first_id = world.persistent_id(first_entity).unwrap();
        assert_ne!(first_id, source_id);
        assert!(
            world
                .get::<crate::prefab::PrefabMember>(first_entity)
                .is_none()
        );
        assert!(
            world
                .get::<crate::blockout::BlockoutComponent>(first_entity)
                .is_some()
        );
        first.undo(&mut world, &mut selected);
        first.execute(&mut world, &mut selected);
        assert_eq!(world.persistent_id(selected.unwrap()), Some(first_id));
        let mut second = PasteEntitiesCmd::new(clipboard, None);
        second.execute(&mut world, &mut selected);
        assert_ne!(world.persistent_id(selected.unwrap()), Some(first_id));
        assert_eq!(world.persistent_id(source), Some(source_id));
    }

    fn subtree(world: &mut World) -> (Entity, Entity, Entity) {
        let root = world.spawn((
            Transform::from_translation(glam::Vec3::new(1.0, 2.0, 3.0)),
            Name::new("root"),
            WorldTransform::identity(),
            Children::empty(),
        ));
        let child = world.spawn((
            Transform::from_translation(glam::Vec3::new(0.0, 1.0, 0.0)),
            Name::new("child"),
            WorldTransform::identity(),
            MeshComponent {
                vertex_offset: 7,
                index_offset: 8,
                index_count: 9,
            },
            Parent { entity: root },
            Children::empty(),
        ));
        let grandchild = world.spawn((
            Transform::from_translation(glam::Vec3::new(0.0, 0.0, 5.0)),
            Name::new("grandchild"),
            WorldTransform::identity(),
            Parent { entity: child },
        ));
        world.get_mut::<Children>(root).unwrap().push(child);
        world.get_mut::<Children>(child).unwrap().push(grandchild);
        (root, child, grandchild)
    }

    /// CONTROL-F's exit clause: copy a subtree, paste it, and the hierarchy
    /// and every property survive — into a *new* set of handles.
    #[test]
    fn a_pasted_subtree_keeps_its_shape_and_its_properties() {
        let mut world = World::new();
        let (root, _, _) = subtree(&mut world);
        let clipboard = EntityClipboard::copy(&world, &[root]);

        let mut selected = None;
        let mut undo = UndoStack::new(4);
        let mut command = PasteEntitiesCmd::new(clipboard, None);
        command.execute(&mut world, &mut selected);
        let pasted = command.roots()[0];
        assert_ne!(pasted, root, "paste must mint new handles");

        let child = world.get::<Children>(pasted).unwrap().as_slice()[0];
        assert_eq!(world.get::<Name>(child).unwrap().as_str(), "child");
        assert_eq!(world.get::<Parent>(child).unwrap().entity, pasted);
        assert_eq!(world.get::<MeshComponent>(child).unwrap().index_count, 9);

        let grandchild = world.get::<Children>(child).unwrap().as_slice()[0];
        assert_eq!(world.get::<Parent>(grandchild).unwrap().entity, child);
        assert_eq!(
            world.get::<Transform>(grandchild).unwrap().translation.z,
            5.0
        );

        // …and the original is untouched.
        assert_eq!(world.get::<Children>(root).unwrap().count, 1);
        undo.push(Box::new(command), &mut world, &mut selected);
    }

    /// Pasting is one gesture: one undo removes the whole subtree.
    #[test]
    fn paste_is_exactly_one_undo_step() {
        let mut world = World::new();
        let (root, _, _) = subtree(&mut world);
        let before = world.entities().count();
        let clipboard = EntityClipboard::copy(&world, &[root]);

        let mut selected = None;
        let mut undo = UndoStack::new(4);
        undo.push(
            Box::new(PasteEntitiesCmd::new(clipboard, None)),
            &mut world,
            &mut selected,
        );
        assert_eq!(world.entities().count(), before + 3);

        assert!(undo.undo(&mut world, &mut selected));
        assert_eq!(world.entities().count(), before);
        assert!(undo.redo(&mut world, &mut selected));
        assert_eq!(world.entities().count(), before + 3);
    }

    /// Copying a parent *and* its child copies the subtree once, not twice.
    #[test]
    fn copy_keeps_only_the_canonical_roots() {
        let mut world = World::new();
        let (root, child, _) = subtree(&mut world);
        let clipboard = EntityClipboard::copy(&world, &[root, child]);
        assert_eq!(clipboard.root_count(), 1);

        let mut selected = None;
        let mut command = PasteEntitiesCmd::new(clipboard, None);
        command.execute(&mut world, &mut selected);
        assert_eq!(command.roots().len(), 1);
        let pasted = command.roots()[0];
        assert_eq!(
            world.get::<Children>(pasted).unwrap().count,
            1,
            "the child must appear once, inside its parent"
        );
    }

    /// Pasting under a parent links into that parent's child list, so paste
    /// and reparent agree about what the hierarchy is.
    #[test]
    fn paste_under_a_parent_links_both_directions() {
        let mut world = World::new();
        let (root, _, _) = subtree(&mut world);
        let host = world.spawn((
            Transform::default(),
            Name::new("host"),
            WorldTransform::identity(),
            Children::empty(),
        ));
        let clipboard = EntityClipboard::copy(&world, &[root]);

        let mut selected = None;
        let mut command = PasteEntitiesCmd::new(clipboard, Some(host));
        command.execute(&mut world, &mut selected);
        let pasted = command.roots()[0];
        assert_eq!(world.get::<Parent>(pasted).unwrap().entity, host);
        assert!(
            world
                .get::<Children>(host)
                .unwrap()
                .as_slice()
                .contains(&pasted)
        );
    }

    #[test]
    fn an_empty_clipboard_pastes_nothing() {
        let mut world = World::new();
        let mut selected = None;
        let mut undo = UndoStack::new(4);
        let before = world.entities().count();
        undo.push(
            Box::new(PasteEntitiesCmd::new(EntityClipboard::default(), None)),
            &mut world,
            &mut selected,
        );
        assert_eq!(world.entities().count(), before);
        assert!(!undo.can_undo(), "a no-op paste must not enter history");
    }
}
