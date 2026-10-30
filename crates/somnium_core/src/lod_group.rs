//! Distance levels of detail for imported buildings and props (Rimeborn's WS2-1).
//!
//! A `somnium.LodGroup` on a root picks which one of its direct children draws: the first (in name order, so `LOD0`,
//! `LOD1`, `LOD2` sort as meant) while the camera is nearer than `distances.x`, the second until `distances.y`, the
//! third beyond, and nothing past `distances.z` when that is above zero. Each level is an ordinary subtree of parts
//! (a building's LOD0 with its cutaway groups, its LOD1 shell...). The levels not chosen are skipped at submission,
//! shadows included, through a runtime set: nothing authored or saved changes, and a scene without a `LodGroup`
//! draws exactly as before.

use std::collections::{HashMap, HashSet};

use glam::Vec3;
use somnium_ecs::component_schema;
use somnium_ecs::reflect::TypeRegistry;
use somnium_ecs::{Component, Entity, World};

use crate::{Name, Parent, Transform, WorldTransform};

/// A root whose children are levels of detail.
#[derive(Debug, Clone, PartialEq)]
pub struct LodGroupComponent {
    /// Off: every level draws (for looking at them side by side).
    pub enabled: bool,
    /// Camera distances, metres: x leaves the first level, y the second; past z (if above zero) nothing draws.
    pub distances: Vec3,
}

impl Component for LodGroupComponent {}

impl Default for LodGroupComponent {
    fn default() -> Self {
        Self { enabled: true, distances: Vec3::new(80.0, 200.0, 0.0) }
    }
}

pub(crate) fn register(registry: &mut TypeRegistry) {
    registry.register(component_schema! {
        LodGroupComponent as "somnium.LodGroup", display "LOD Group", version 1,
        fields {
            enabled { group: "LOD", doc: "Off: every level draws." },
            distances { unit: "m", group: "LOD", doc: "Camera distances: x leaves the first child level, y the second; past z (if above zero) nothing draws. Levels are the direct children in name order (LOD0, LOD1, LOD2)." },
        }
    });
}

/// Which level a camera at `distance` draws, of `levels`; `None` past the cull.
#[must_use]
pub fn level_for(group: &LodGroupComponent, distance: f32, levels: usize) -> Option<usize> {
    if levels == 0 || (group.distances.z > 0.0 && distance > group.distances.z) {
        return None;
    }
    let passed = usize::from(distance > group.distances.x) + usize::from(distance > group.distances.y);
    Some(passed.min(levels - 1))
}

struct Group {
    root: Entity,
    /// Every entity of each level's subtree, the level's own root first.
    levels: Vec<Vec<Entity>>,
    active: Option<Option<usize>>,
}

/// The levels skipped this frame.
#[derive(Default)]
pub struct Lods {
    groups: Vec<Group>,
    built_for: usize,
    hidden: HashSet<Entity>,
}

impl Lods {
    /// Choose each group's level for a camera at `camera`; rebuilds the skip set only when a choice changes.
    pub fn update(&mut self, world: &World, camera: Vec3) {
        // ponytail: the level structure is rebuilt when the world's entity count changes; a reparent that keeps the
        // count is picked up at the next change. Hook the hierarchy's edits if that ever matters.
        if self.built_for != world.entity_count() || self.groups.iter().any(|g| !world.is_alive(g.root)) {
            self.rebuild(world);
        }
        let mut changed = false;
        for group in &mut self.groups {
            let Some(settings) = world.get::<LodGroupComponent>(group.root) else { continue };
            let choice = if settings.enabled {
                let at = world
                    .get::<WorldTransform>(group.root)
                    .map(|w| w.0.w_axis.truncate())
                    .or_else(|| world.get::<Transform>(group.root).map(|t| t.translation))
                    .unwrap_or(Vec3::ZERO);
                level_for(settings, at.distance(camera), group.levels.len())
            } else {
                Some(usize::MAX)
            };
            if group.active != Some(choice) {
                group.active = Some(choice);
                changed = true;
            }
        }
        if changed {
            self.hidden.clear();
            for group in &self.groups {
                let Some(choice) = group.active else { continue };
                if choice == Some(usize::MAX) {
                    continue;
                }
                for (i, level) in group.levels.iter().enumerate() {
                    if Some(i) != choice {
                        self.hidden.extend(level.iter().copied());
                    }
                }
            }
        }
    }

    /// Whether this entity belongs to a level not drawn this frame.
    #[must_use]
    pub fn is_hidden(&self, entity: Entity) -> bool {
        !self.hidden.is_empty() && self.hidden.contains(&entity)
    }

    fn rebuild(&mut self, world: &World) {
        self.built_for = world.entity_count();
        self.hidden.clear();
        let roots: Vec<Entity> = world.entities_with::<LodGroupComponent>().collect();
        self.groups.clear();
        if roots.is_empty() {
            return;
        }
        let mut children: HashMap<Entity, Vec<Entity>> = HashMap::new();
        for entity in world.entities_with::<Parent>() {
            if let Some(parent) = world.get::<Parent>(entity).map(|p| p.entity) {
                if parent != Entity::DANGLING {
                    children.entry(parent).or_default().push(entity);
                }
            }
        }
        let name = |e: Entity| world.get::<Name>(e).map_or_else(String::new, |n| n.as_str().to_owned());
        for root in roots {
            let mut direct = children.get(&root).cloned().unwrap_or_default();
            direct.sort_by_key(|e| name(*e));
            let levels = direct
                .into_iter()
                .map(|level| {
                    let mut all = vec![level];
                    let mut i = 0;
                    while i < all.len() {
                        if let Some(kids) = children.get(&all[i]) {
                            all.extend(kids.iter().copied());
                        }
                        i += 1;
                    }
                    all
                })
                .collect();
            self.groups.push(Group { root, levels, active: None });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn(world: &mut World, name: &str, parent: Option<Entity>, at: Vec3) -> Entity {
        let e = world.spawn((Name::new(name), Transform::from_translation(at)));
        if let Some(parent) = parent {
            world.insert_component(e, Parent { entity: parent }).unwrap();
        }
        e
    }

    #[test]
    fn the_level_follows_the_distance() {
        let g = LodGroupComponent { enabled: true, distances: Vec3::new(80.0, 200.0, 500.0) };
        assert_eq!(level_for(&g, 10.0, 3), Some(0));
        assert_eq!(level_for(&g, 120.0, 3), Some(1));
        assert_eq!(level_for(&g, 300.0, 3), Some(2));
        assert_eq!(level_for(&g, 300.0, 2), Some(1), "fewer levels: the last one holds");
        assert_eq!(level_for(&g, 600.0, 3), None, "past the cull nothing draws");
        assert_eq!(level_for(&LodGroupComponent::default(), 1.0e6, 3), Some(2), "z of 0 never culls");
    }

    #[test]
    fn only_the_chosen_level_draws() {
        let mut world = World::new();
        let house = spawn(&mut world, "cottage", None, Vec3::new(100.0, 0.0, 0.0));
        world.insert_component(house, LodGroupComponent::default()).unwrap();
        // Out of name order on purpose.
        let far = spawn(&mut world, "LOD1", Some(house), Vec3::ZERO);
        let near = spawn(&mut world, "LOD0", Some(house), Vec3::ZERO);
        let roof = spawn(&mut world, "Roof / slate", Some(near), Vec3::ZERO);
        let shell = spawn(&mut world, "Walls / shell", Some(far), Vec3::ZERO);
        let plain = spawn(&mut world, "well", None, Vec3::ZERO);

        let mut lods = Lods::default();
        lods.update(&world, Vec3::new(100.0, 30.0, 40.0));
        assert!(!lods.is_hidden(near) && !lods.is_hidden(roof));
        assert!(lods.is_hidden(far) && lods.is_hidden(shell));
        assert!(!lods.is_hidden(plain), "nothing outside a group is touched");

        lods.update(&world, Vec3::new(100.0, 30.0, 140.0));
        assert!(lods.is_hidden(near) && lods.is_hidden(roof));
        assert!(!lods.is_hidden(far) && !lods.is_hidden(shell));

        world.get_mut::<LodGroupComponent>(house).unwrap().enabled = false;
        lods.update(&world, Vec3::new(100.0, 30.0, 140.0));
        assert!(!lods.is_hidden(near) && !lods.is_hidden(far), "off: every level draws");
    }
}
