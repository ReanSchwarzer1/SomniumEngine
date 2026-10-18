//! Cutaways for top-down and isometric views.
//!
//! When the viewer (the player, or the point the camera looks at) stands inside a building, the parts that would
//! hide the room from above (its roof, its upper floor) stop drawing for the camera but **keep their shadows**, so
//! the room is still lit as a room with a roof over it rather than flooded with sun. They are handed to the
//! renderer's shadow-only queue, the same path a foliage shadow proxy takes.
//!
//! A `somnium.Cutaway` sits on a building's root with a box in the root's frame and the name prefixes of the parts
//! it opens. Parts are matched by name (the entity or any ancestor below the root), so an imported glTF whose nodes
//! are called `Roof`, `Roof_Ridge` or `Upper_Floor` opens without per-part setup. A `somnium.CutawayViewer` says
//! where the viewer is; without one the caller passes the camera position.

use std::collections::HashSet;

use glam::{Mat4, Vec3};
use somnium_ecs::component_schema;
use somnium_ecs::reflect::TypeRegistry;
use somnium_ecs::{Component, Entity, World};

use crate::{Name, Parent, Transform, WorldTransform};

/// A building that opens when the viewer is inside it.
#[derive(Debug, Clone, PartialEq)]
pub struct CutawayComponent {
    /// Off: the building never opens.
    pub enabled: bool,
    /// Centre of the inside, in the root's frame, metres.
    pub center: Vec3,
    /// Half the inside's size along each of the root's axes, metres.
    pub half_extents: Vec3,
    /// Comma-separated name prefixes of the parts that open (case-insensitive).
    pub hide: String,
}

impl Component for CutawayComponent {}

impl Default for CutawayComponent {
    fn default() -> Self {
        Self { enabled: true, center: Vec3::new(0.0, 2.5, 0.0), half_extents: Vec3::new(5.0, 3.0, 5.0), hide: "Roof,Upper".into() }
    }
}

/// Where the viewer is, written by the game each frame (its player, or its camera's focus), on any one entity.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CutawayViewerComponent {
    /// The viewer's world position, metres.
    pub position: Vec3,
}

impl Component for CutawayViewerComponent {}

pub(crate) fn register(registry: &mut TypeRegistry) {
    registry.register(component_schema! {
        CutawayComponent as "somnium.Cutaway", display "Cutaway", version 1,
        fields {
            enabled { group: "Cutaway" },
            center { unit: "m", group: "Cutaway", doc: "Centre of the inside, in this entity's frame." },
            half_extents { unit: "m", group: "Cutaway", display_name: "Half Extents", doc: "Half the inside's size along each axis." },
            hide { group: "Cutaway", doc: "Comma-separated name prefixes of the parts that stop drawing for the camera (they keep their shadows) while the viewer is inside." },
        }
    });
    registry.register(component_schema! {
        CutawayViewerComponent as "somnium.CutawayViewer", display "Cutaway Viewer", version 1,
        fields {
            position { unit: "m", group: "Cutaway", doc: "The point whose being inside a building opens it. Games write it each frame." },
        }
    });
}

/// The viewer's position, if a game has said where it is.
#[must_use]
pub fn viewer(world: &World) -> Option<Vec3> {
    world.entities_with::<CutawayViewerComponent>().find_map(|e| world.get::<CutawayViewerComponent>(e).map(|v| v.position))
}

/// Which parts are open this frame. Recomputes the part set only when the set of open buildings changes.
#[derive(Debug, Default)]
pub struct Cutaways {
    open: Vec<Entity>,
    shadow_only: HashSet<Entity>,
}

impl Cutaways {
    /// Find the buildings the viewer is inside and, if that changed, which parts they open.
    pub fn update(&mut self, world: &World, viewer: Vec3) {
        let mut open: Vec<Entity> = world
            .entities_with::<CutawayComponent>()
            .filter(|root| {
                let Some(cutaway) = world.get::<CutawayComponent>(*root) else { return false };
                cutaway.enabled && contains(cutaway, world_matrix(world, *root), viewer)
            })
            .collect();
        open.sort_by_key(|e| e.index());
        if open == self.open {
            return;
        }
        self.open = open;
        self.shadow_only.clear();
        if self.open.is_empty() {
            return;
        }
        // Rare (on entering or leaving a building), so a scan of the parented entities is fine here.
        for entity in world.entities_with::<Parent>().collect::<Vec<_>>() {
            if opens(world, entity, &self.open) {
                self.shadow_only.insert(entity);
            }
        }
    }

    /// Whether this part should draw into the shadows only.
    #[must_use]
    pub fn is_shadow_only(&self, entity: Entity) -> bool {
        !self.shadow_only.is_empty() && self.shadow_only.contains(&entity)
    }

    /// The buildings open now.
    #[must_use]
    pub fn open(&self) -> &[Entity] {
        &self.open
    }
}

fn contains(cutaway: &CutawayComponent, root: Mat4, point: Vec3) -> bool {
    let local = root.inverse().transform_point3(point) - cutaway.center;
    let half = cutaway.half_extents.abs();
    local.x.abs() <= half.x && local.y.abs() <= half.y && local.z.abs() <= half.z
}

/// Whether `entity` is a part an open building hides: its nearest cutaway ancestor is open, and it or an ancestor
/// below that root carries one of the root's prefixes.
fn opens(world: &World, entity: Entity, open: &[Entity]) -> bool {
    let mut names_below_root: Vec<Entity> = Vec::new();
    let mut current = entity;
    for _ in 0..64 {
        if world.get::<CutawayComponent>(current).is_some_and(|c| c.enabled) {
            if !open.contains(&current) {
                return false;
            }
            let prefixes = world.get::<CutawayComponent>(current).map(|c| prefixes(&c.hide)).unwrap_or_default();
            return names_below_root.iter().any(|e| {
                world.get::<Name>(*e).is_some_and(|n| {
                    let name = n.as_str().to_ascii_lowercase();
                    prefixes.iter().any(|p| name.starts_with(p.as_str()))
                })
            });
        }
        names_below_root.push(current);
        let Some(parent) = world.get::<Parent>(current).map(|p| p.entity) else { return false };
        if parent == Entity::DANGLING {
            return false;
        }
        current = parent;
    }
    false
}

fn prefixes(hide: &str) -> Vec<String> {
    hide.split(',').map(|p| p.trim().to_ascii_lowercase()).filter(|p| !p.is_empty()).collect()
}

/// An entity's world matrix: its `WorldTransform` if it has one, else its local transforms composed upward.
fn world_matrix(world: &World, mut entity: Entity) -> Mat4 {
    let mut local = Mat4::IDENTITY;
    for _ in 0..64 {
        if let Some(transform) = world.get::<WorldTransform>(entity) {
            return transform.0 * local;
        }
        local = world.get::<Transform>(entity).map_or(Mat4::IDENTITY, |t| t.to_matrix()) * local;
        match world.get::<Parent>(entity).map(|p| p.entity) {
            Some(parent) if parent != Entity::DANGLING => entity = parent,
            _ => break,
        }
    }
    local
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
    fn a_roof_opens_only_while_the_viewer_is_inside() {
        let mut world = World::new();
        let house = spawn(&mut world, "Cottage", None, Vec3::new(100.0, 545.0, 250.0));
        world.insert_component(house, CutawayComponent::default()).unwrap();
        let roof = spawn(&mut world, "Roof", Some(house), Vec3::new(0.0, 5.0, 0.0));
        let ridge = spawn(&mut world, "ridge_tiles", Some(roof), Vec3::ZERO);
        let walls = spawn(&mut world, "Walls", Some(house), Vec3::ZERO);
        let upper = spawn(&mut world, "upper_floor_boards", Some(house), Vec3::new(0.0, 3.0, 0.0));

        let mut cutaways = Cutaways::default();
        cutaways.update(&world, Vec3::new(100.0, 546.0, 250.0));
        assert_eq!(cutaways.open(), &[house]);
        assert!(cutaways.is_shadow_only(roof));
        assert!(cutaways.is_shadow_only(ridge), "a child of the roof opens with it");
        assert!(cutaways.is_shadow_only(upper), "prefixes match case-insensitively");
        assert!(!cutaways.is_shadow_only(walls));

        cutaways.update(&world, Vec3::new(130.0, 546.0, 250.0));
        assert!(cutaways.open().is_empty());
        assert!(!cutaways.is_shadow_only(roof));
    }

    #[test]
    fn the_box_turns_with_the_building_and_a_disabled_cutaway_stays_shut() {
        let mut world = World::new();
        let house = world.spawn((
            Name::new("Longhouse"),
            Transform { rotation: glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2), ..Transform::from_translation(Vec3::ZERO) },
        ));
        world
            .insert_component(house, CutawayComponent { half_extents: Vec3::new(10.0, 3.0, 2.0), ..CutawayComponent::default() })
            .unwrap();
        let roof = spawn(&mut world, "Roof", Some(house), Vec3::ZERO);
        let mut cutaways = Cutaways::default();
        // Long along the house's X, which the quarter turn lays along world Z.
        cutaways.update(&world, Vec3::new(0.0, 2.0, 8.0));
        assert!(cutaways.is_shadow_only(roof));
        cutaways.update(&world, Vec3::new(8.0, 2.0, 0.0));
        assert!(!cutaways.is_shadow_only(roof));

        world.get_mut::<CutawayComponent>(house).unwrap().enabled = false;
        cutaways.update(&world, Vec3::new(0.0, 2.0, 8.0));
        assert!(!cutaways.is_shadow_only(roof));
    }
}
