//! Doors that swing about their hinge (Rimeborn, 19 Oct 2026; contract `docs/editor/interaction.md`).
//!
//! A door is a `somnium.Interactable` of kind 0 whose entity is its hinge: its origin is the hinge, its +X runs along
//! the closed leaf to the free edge, +Y is up. What swings is the entity's children, and the entities under its
//! parent whose names start with `leaves` (a building's imported `Door / <material>` parts, wherever they sit under
//! the building). At a use's commit event ([`crate::interact::Used`], the session's contact) it eases open by
//! `open_angle` about the hinge's vertical over `swing_duration`, or closed again; with `close_after` it closes by
//! itself once nobody is in its way. While closed, a static box fills the leaf (`blocks`), so a locked door stops
//! the player; it goes as the door starts to open.
//!
//! `open` is both the authored start and the live state: the system writes it when a door is used, and a game or
//! script that sets it swings the door. Villagers and other walkers open doors through [`Doors::pass`].
//!
//! Idle doors cost nothing: each frame walks the interactables (one component query, no lookups) to see that they are
//! the ones of the last frame, and touches only doors that are moving or waiting to close.

use std::collections::{HashMap, HashSet};

use glam::{Mat4, Quat, Vec3};
use somnium_ecs::{ComponentId, Entity, World};
use somnium_physics::body::{BodyId, MotionType, RigidBodyDescriptor};
use somnium_physics::shape::ColliderShape;
use somnium_physics::world::PhysicsWorld;

use crate::interact::{KIND_DOOR, Used};
use crate::interaction::Interactable;
use crate::{Name, Parent, Transform};

/// A door's `somnium.Interactable` for a building's door record: the hinge in the building's frame, the leaf's yaw
/// (radians about +Y: 0 lies along the building's +X), size, and the side it opens to (+1 or -1, as WS2's `opens`).
/// Spawn the two on an entity under the building root.
#[must_use]
pub fn door_at_hinge(hinge: Vec3, yaw: f32, width: f32, height: f32, opens: f32, leaves: &str) -> (Transform, Interactable) {
    let transform = Transform { translation: hinge, rotation: Quat::from_rotation_y(yaw), scale: Vec3::ONE };
    let door = Interactable {
        kind: KIND_DOOR,
        prompt: "Open".into(),
        // The reach to the handle before the door moves: short, as a hand on a latch.
        duration: 0.4,
        anchor: Vec3::new(0.5 * width, (0.5 * height).min(1.0), 0.0),
        open_angle: 100.0 * if opens < 0.0 { -1.0 } else { 1.0 },
        leaf_width: width,
        leaf_height: height,
        leaves: leaves.into(),
        ..Interactable::default()
    };
    (transform, door)
}

/// What the doors did this frame, for sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DoorEvent {
    /// It started to swing open (a creak).
    Opening(Entity),
    /// It started to swing shut.
    Closing(Entity),
    /// It shut (a thud).
    Closed(Entity),
    /// Used while locked.
    Refused(Entity),
}

/// Doorways walkers asked for since the doors last looked ([`request_pass`]).
#[derive(Default)]
struct Passes(Vec<Vec3>);

/// A walker (a villager, a guard) goes through the doorway at `at`. Any system holding the world can call it (the
/// life sim, the AI); the doors open that door on their next update, as [`Doors::pass`] does.
pub fn request_pass(world: &mut World, at: Vec3) {
    let mut passes: Passes = world.take_cache();
    passes.0.push(at);
    world.put_cache(passes);
}

/// Drop the walkers' requests (outside play nobody answers them).
pub fn clear_passes(world: &mut World) {
    drop(world.take_cache::<Passes>());
}

/// How near a walker's doorway point must be to a door's for [`Doors::pass`], metres along the ground. Generous: the
/// life sim's door point is on the ground about 0.6 m out from the building's front middle, which can be most of a
/// leaf aside from the door itself; the nearest door wins.
pub const PASS_REACH: f32 = 2.0;
/// And up or down from it, metres (within a storey).
pub const PASS_HEIGHT: f32 = 2.0;
/// Seconds a door a walker opened stays open, whatever its own `close_after`.
pub const PASS_CLOSE: f32 = 1.5;

struct Leaf {
    entity: Entity,
    /// The hinge frame in the leaf's parent's frame, and back.
    hinge: Mat4,
    hinge_inv: Mat4,
    closed: Mat4,
}

struct Bound {
    leaves: Vec<Leaf>,
    /// 0 closed, 1 open.
    progress: f32,
    goal: bool,
    angle: f32,
    duration: f32,
    close_after: f32,
    /// Clock time it starts closing by itself.
    closes_at: Option<f32>,
    /// World: the hinge, and the leaf's middle at hand height.
    hinge: Vec3,
    doorway: Vec3,
    width: f32,
    /// The closed leaf's box (centre, rotation, half extents), if it blocks.
    block: Option<(Vec3, Quat, Vec3)>,
    body: Option<BodyId>,
}

/// The doors of a play session.
#[derive(Default)]
pub struct Doors {
    bound: HashMap<Entity, Bound>,
    /// The door archetypes' entities at the last sync, in order, and the doors' change tick then.
    seen: Vec<Entity>,
    seen_tick: u64,
    /// Doors swinging or waiting to close.
    active: Vec<Entity>,
    clock: f32,
    /// What happened since the last update returned (a pass between updates included).
    events: Vec<DoorEvent>,
}

impl Doors {
    /// How many doors are bound (found, with their leaves).
    #[must_use]
    pub fn len(&self) -> usize {
        self.bound.len()
    }

    /// No door is bound.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bound.is_empty()
    }

    /// Doors swinging or waiting to close.
    #[must_use]
    pub fn active(&self) -> usize {
        self.active.len()
    }

    /// The play session ended: every door's body goes and everything is forgotten (Stop restores the leaves).
    pub fn stop(&mut self, physics: &mut PhysicsWorld) {
        for bound in self.bound.values_mut() {
            if let Some(body) = bound.body.take() {
                physics.destroy_body(body);
            }
        }
        *self = Self::default();
    }

    /// One frame of play. `used`: what the interact key used this frame; `near`: where the people the doors must
    /// not close on stand (the player). Returns what the doors did since the last update.
    pub fn update(
        &mut self,
        world: &mut World,
        physics: &mut PhysicsWorld,
        used: Option<&Used>,
        near: &[Vec3],
        dt: f32,
    ) -> Vec<DoorEvent> {
        self.clock += dt;
        self.sync(world, physics);
        for at in world.take_cache::<Passes>().0 {
            self.pass(world, physics, at);
        }
        if let Some(used) = used.filter(|u| u.kind == KIND_DOOR && self.bound.contains_key(&u.entity)) {
            if used.refused {
                self.events.push(DoorEvent::Refused(used.entity));
            } else {
                let goal = !self.bound[&used.entity].goal;
                self.set_goal(world, physics, used.entity, goal, None);
            }
        }
        self.step(world, physics, near, dt);
        std::mem::take(&mut self.events)
    }

    /// A walker (a villager, a guard) goes through a doorway at `at`: the door there opens, ignoring its lock, and
    /// closes [`PASS_CLOSE`] seconds later. Returns the door, if there is one within [`PASS_REACH`] along the ground
    /// and [`PASS_HEIGHT`] up or down.
    pub fn pass(&mut self, world: &mut World, physics: &mut PhysicsWorld, at: Vec3) -> Option<Entity> {
        let door = self
            .bound
            .iter()
            .filter(|(_, b)| (b.doorway.y - at.y).abs() <= PASS_HEIGHT)
            .map(|(&e, b)| (e, (b.doorway - at).with_y(0.0).length_squared()))
            .filter(|&(_, d)| d <= PASS_REACH * PASS_REACH)
            .min_by(|a, b| a.1.total_cmp(&b.1))?
            .0;
        let closes = self.clock + PASS_CLOSE + self.bound[&door].duration;
        self.set_goal(world, physics, door, true, Some(closes));
        Some(door)
    }

    fn set_goal(
        &mut self,
        world: &mut World,
        physics: &mut PhysicsWorld,
        door: Entity,
        goal: bool,
        closes_at: Option<f32>,
    ) {
        let Some(bound) = self.bound.get_mut(&door) else { return };
        let changed = bound.goal != goal;
        bound.goal = goal;
        bound.closes_at = if goal { closes_at.or((bound.close_after > 0.0).then_some(f32::INFINITY)) } else { None };
        if goal && let Some(body) = bound.body.take() {
            physics.destroy_body(body);
        }
        if changed {
            self.events.push(if goal { DoorEvent::Opening(door) } else { DoorEvent::Closing(door) });
            if let Some(component) = world.get_mut::<Interactable>(door) {
                component.open = goal;
            }
            // Our own write is not news.
            self.seen_tick = world.change_tick(ComponentId::of::<Interactable>());
        }
        if !self.active.contains(&door) {
            self.active.push(door);
        }
    }

    fn step(&mut self, world: &mut World, physics: &mut PhysicsWorld, near: &[Vec3], dt: f32) {
        let mut i = 0;
        while i < self.active.len() {
            let door = self.active[i];
            let Some(bound) = self.bound.get_mut(&door) else {
                self.active.swap_remove(i);
                continue;
            };
            let target = if bound.goal { 1.0 } else { 0.0 };
            if bound.progress != target {
                let step = dt / bound.duration.max(1e-3);
                bound.progress =
                    if bound.goal { (bound.progress + step).min(1.0) } else { (bound.progress - step).max(0.0) };
                let p = bound.progress;
                let angle = bound.angle.to_radians() * p * p * (3.0 - 2.0 * p);
                pose(world, &bound.leaves, angle);
                if bound.progress == target && !bound.goal {
                    self.events.push(DoorEvent::Closed(door));
                    bound.body = bound.block.map(|block| block_body(physics, block));
                }
                i += 1;
                continue;
            }
            // At rest: open and due to close, or done.
            match bound.closes_at {
                Some(at) if at == f32::INFINITY => {
                    bound.closes_at = Some(self.clock + bound.close_after);
                    i += 1;
                }
                Some(at) => {
                    let clear = bound.width + 0.4;
                    let blocked = near.iter().any(|p| {
                        let d = *p - bound.hinge;
                        d.y.abs() < 2.5 && d.x * d.x + d.z * d.z < clear * clear
                    });
                    if self.clock >= at && !blocked {
                        self.set_goal(world, physics, door, false, None);
                    }
                    i += 1;
                }
                None => {
                    self.active.swap_remove(i);
                }
            }
        }
    }

    /// Find doors that arrived, forget ones that went, and follow `open` written by others. Costs one walk of the
    /// interactables, compared with the last frame's, when nothing changed.
    fn sync(&mut self, world: &mut World, physics: &mut PhysicsWorld) {
        let tick = world.change_tick(ComponentId::of::<Interactable>());
        let mut count = 0;
        let mut same = true;
        for (entity, _) in world.iter_with::<Interactable>() {
            same &= self.seen.get(count) == Some(&entity);
            count += 1;
        }
        if same && count == self.seen.len() && tick == self.seen_tick {
            return;
        }
        self.seen.clear();
        self.seen.extend(world.entities_with::<Interactable>());
        self.seen_tick = tick;
        let present: HashSet<Entity> = self.seen.iter().copied().collect();
        let gone: Vec<Entity> = self.bound.keys().copied().filter(|e| !present.contains(e)).collect();
        for door in gone {
            if let Some(body) = self.bound.remove(&door).and_then(|b| b.body) {
                physics.destroy_body(body);
            }
        }
        let arrived: Vec<Entity> = self
            .seen
            .iter()
            .copied()
            .filter(|e| !self.bound.contains_key(e) && world.get::<Interactable>(*e).is_some_and(|d| d.enabled && d.kind == KIND_DOOR))
            .collect();
        if !arrived.is_empty() {
            self.bind(world, physics, &arrived);
        }
        // `open` written by a game or script.
        let moved: Vec<(Entity, bool)> = self
            .bound
            .iter()
            .filter_map(|(&e, b)| world.get::<Interactable>(e).filter(|d| d.open != b.goal).map(|d| (e, d.open)))
            .collect();
        for (door, open) in moved {
            self.set_goal(world, physics, door, open, None);
        }
    }

    fn bind(&mut self, world: &mut World, physics: &mut PhysicsWorld, doors: &[Entity]) {
        for (door, mut bound, open) in Self::bound_for(world, doors) {
            if open {
                // Authored open: there at once.
                (bound.goal, bound.progress) = (true, 1.0);
                pose(world, &bound.leaves, bound.angle.to_radians());
            } else {
                bound.body = bound.block.map(|block| block_body(physics, block));
            }
            self.bound.insert(door, bound);
        }
    }

    /// Each door's leaves and world frame (one pass over the parented entities, one over the named ones if any door
    /// takes named parts), and whether it starts open.
    fn bound_for(world: &World, doors: &[Entity]) -> Vec<(Entity, Bound, bool)> {
        let parent_of = |e: Entity| world.get::<Parent>(e).map(|p| p.entity).filter(|p| *p != Entity::DANGLING);
        let local = |e: Entity| world.get::<Transform>(e).map_or(Mat4::IDENTITY, Transform::to_matrix);
        let is_door: HashSet<Entity> = doors.iter().copied().collect();
        let mut leaves: HashMap<Entity, Vec<Entity>> = HashMap::new();
        // Children of the doors.
        for (e, parent) in world.iter_with::<Parent>() {
            if is_door.contains(&parent.entity) {
                leaves.entry(parent.entity).or_default().push(e);
            }
        }
        // Named parts under the doors' parents.
        let mut by_root: HashMap<Entity, Vec<(Entity, String)>> = HashMap::new();
        for &door in doors {
            let prefix = world.get::<Interactable>(door).map(|d| d.leaves.clone()).unwrap_or_default();
            if let (false, Some(root)) = (prefix.is_empty(), parent_of(door)) {
                by_root.entry(root).or_default().push((door, prefix));
            }
        }
        if !by_root.is_empty() {
            let prefixes: Vec<String> = by_root.values().flatten().map(|(_, p)| p.clone()).collect();
            for (e, name) in world.iter_with::<Name>() {
                let name = name.as_str();
                if is_door.contains(&e) || !prefixes.iter().any(|p| name.starts_with(p.as_str())) {
                    continue;
                }
                let mut up = parent_of(e);
                for _ in 0..16 {
                    let Some(ancestor) = up else { break };
                    if let Some(candidates) = by_root.get(&ancestor) {
                        for (door, prefix) in candidates {
                            if name.starts_with(prefix.as_str()) {
                                leaves.entry(*door).or_default().push(e);
                            }
                        }
                        break;
                    }
                    up = parent_of(ancestor);
                }
            }
        }
        let mut made = Vec::new();
        for &door in doors {
            let Some(component) = world.get::<Interactable>(door).cloned() else { continue };
            let root = parent_of(door);
            let door_local = local(door);
            let parts = leaves
                .remove(&door)
                .unwrap_or_default()
                .into_iter()
                .map(|leaf| {
                    // The leaf's parent's frame in the door's parent's frame.
                    let mut frame = Mat4::IDENTITY;
                    let mut up = parent_of(leaf);
                    while let Some(e) = up.filter(|e| Some(*e) != root) {
                        frame = local(e) * frame;
                        up = parent_of(e);
                    }
                    let hinge = frame.inverse() * door_local;
                    Leaf { entity: leaf, hinge, hinge_inv: hinge.inverse(), closed: local(leaf) }
                })
                .collect::<Vec<_>>();
            // From the local transforms, not `WorldTransform`: a door spawned this frame has not been propagated yet.
            let mut frame = door_local;
            let mut up = root;
            while let Some(e) = up {
                frame = local(e) * frame;
                up = parent_of(e);
            }
            let (w, h, t) = (component.leaf_width, component.leaf_height, component.leaf_thickness);
            let (scale, rotation, _) = frame.to_scale_rotation_translation();
            let block = component.blocks.then(|| {
                (frame.transform_point3(Vec3::new(0.5 * w, 0.5 * h, 0.0)), rotation, 0.5 * Vec3::new(w, h, t) * scale.abs())
            });
            let bound = Bound {
                leaves: parts,
                progress: 0.0,
                goal: false,
                angle: component.open_angle,
                duration: component.swing_duration,
                close_after: component.close_after,
                closes_at: None,
                hinge: frame.transform_point3(Vec3::ZERO),
                doorway: frame.transform_point3(Vec3::new(0.5 * w, (0.5 * h).min(1.0), 0.0)),
                width: w * scale.x.abs(),
                block,
                body: None,
            };
            made.push((door, bound, component.open));
        }
        made
    }
}

/// The static box of a closed leaf.
fn block_body(physics: &mut PhysicsWorld, (position, rotation, half_extents): (Vec3, Quat, Vec3)) -> BodyId {
    physics.create_body(RigidBodyDescriptor {
        shape: ColliderShape::Box { half_extents },
        position,
        rotation,
        motion_type: MotionType::Static,
        object_layer: somnium_physics::layer::LAYER_NON_MOVING,
        ..Default::default()
    })
}

/// Each leaf's local transform with its door open by `angle` radians: the closed one turned about the hinge.
#[must_use]
pub fn swung(hinge: Mat4, hinge_inv: Mat4, closed: Mat4, angle: f32) -> Mat4 {
    hinge * Mat4::from_rotation_y(angle) * hinge_inv * closed
}

fn pose(world: &mut World, leaves: &[Leaf], angle: f32) {
    for leaf in leaves {
        let (scale, rotation, translation) =
            swung(leaf.hinge, leaf.hinge_inv, leaf.closed, angle).to_scale_rotation_translation();
        if let Some(t) = world.get_mut::<Transform>(leaf.entity) {
            (t.translation, t.rotation, t.scale) = (translation, rotation, scale);
        }
    }
}

#[cfg(test)]
#[path = "door_tests.rs"]
mod tests;
