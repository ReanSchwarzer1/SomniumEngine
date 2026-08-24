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

/// One copied entity and everything beneath it.
#[derive(Clone)]
pub struct ClipNode {
    snapshot: EntitySnapshot,
    scripts: Option<somnium_script::attachment::ScriptSet>,
    children: Vec<ClipNode>,
}

/// A copied forest. Empty until something is copied; a paste of an empty
/// clipboard is a no-op rather than an error, which is what every editor does.
#[derive(Clone, Default)]
pub struct EntityClipboard {
    roots: Vec<ClipNode>,
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
        Self { roots }
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
    let children = world
        .get::<Children>(entity)
        .map(|children| children.as_slice().to_vec())
        .unwrap_or_default()
        .into_iter()
        .filter(|child| world.is_alive(*child))
        .map(|child| capture(world, child))
        .collect();
    ClipNode {
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
    pub fn new(clipboard: EntityClipboard, parent: Option<Entity>) -> Self {
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
