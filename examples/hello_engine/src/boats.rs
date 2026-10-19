//! Boats you board and row (Rimeborn's Act VI slice): a **Boat**
//! (`hello.Boat`) is a Jolt hull floated on the project's water surface
//! (`data/scatter/water_surfaces.json`, WS1's `tools/water_surfaces.py`): a
//! lake at its level inside its outline, else a river or canal reach on its
//! sloped surface (a river's current downstream, a canal still), else the sea.
//!
//! In Play the protagonist (the combat host's player) boards with E within
//! reach of a hull, sits at its seat and rows or sails with WASD (W ahead, S
//! astern, A and D to turn); E again steps off beside it. An empty boat holds
//! to where it was left, bobbing. Horses stay ashore: a rider cannot board.
//!
//! Kinds, by where they go: a Tyffir `rowboat` or `currach` for lakes and
//! rivers, a `fjord` boat (oars and a sail) for the sea, and a Vespergate canal
//! `barge`. A boat entity with no mesh of its own is drawn as a plain hull.

use glam::{Mat4, Quat, Vec2, Vec3};
use serde::Deserialize;
use somnium_core::{
    EngineContext, EngineEvent, InputState, KeyCode, MeshKind, Name, Parent, Transform,
    WorldTransform,
};
use somnium_ecs::component_schema;
use somnium_ecs::reflect::TypeRegistry;
use somnium_ecs::{Component, Entity, World};
use somnium_physics::body::{BodyId, MotionType, RigidBodyDescriptor};
use somnium_physics::layer::LAYER_MOVING;
use somnium_physics::shape::ColliderShape;
use somnium_physics::world::PhysicsWorld;
use std::collections::HashMap;
use tracing::{info, warn};

/// A boat to board and row (or sail).
#[derive(Clone, Debug, PartialEq)]
pub struct Boat {
    pub enabled: bool,
    /// `rowboat`, `currach`, `fjord` or `barge` (anything else rows as a rowboat).
    pub kind: String,
}

impl Component for Boat {}

impl Default for Boat {
    fn default() -> Self {
        Self {
            enabled: true,
            kind: "rowboat".into(),
        }
    }
}

fn schema(r: &mut TypeRegistry) {
    r.register(component_schema! {
        Boat as "hello.Boat", display "Boat", version 1,
        fields {
            enabled { group: "Boat" },
            kind { group: "Boat", doc: "rowboat or currach (lakes and rivers), fjord (the sea: oars and a sail), barge (Vespergate's canals). Its local +Z is the bow." },
        }
    });
}

pub fn register(r: &mut somnium_core::authoring::GameRegistration) {
    r.components.push(schema);
}

/// How a kind of boat is built and goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Kind {
    pub name: &'static str,
    /// The hull's half extents: beam, depth and length (+Z the bow).
    pub half: Vec3,
    /// Metres of immersion that take a sample from dry to fully wet; afloat and
    /// still it sits half that deep.
    pub draft: f32,
    /// Top speed (m/s) and how hard the oars (or sail) push (m/s²).
    pub speed: f32,
    pub accel: f32,
    /// How fast it turns (rad/s at the most).
    pub turn: f32,
    /// Where the protagonist sits, in the boat's frame (feet).
    pub seat: Vec3,
}

pub const KINDS: [Kind; 4] = [
    Kind {
        name: "rowboat",
        half: Vec3::new(0.65, 0.25, 1.8),
        draft: 0.4,
        speed: 2.2,
        accel: 1.2,
        turn: 0.9,
        seat: Vec3::new(0.0, 0.05, -0.3),
    },
    Kind {
        name: "currach",
        half: Vec3::new(0.7, 0.25, 2.25),
        draft: 0.4,
        speed: 2.6,
        accel: 1.3,
        turn: 0.8,
        seat: Vec3::new(0.0, 0.05, -0.5),
    },
    Kind {
        name: "fjord",
        half: Vec3::new(1.0, 0.4, 3.75),
        draft: 0.6,
        speed: 4.0,
        accel: 0.9,
        turn: 0.5,
        seat: Vec3::new(0.0, 0.1, -2.6),
    },
    Kind {
        name: "barge",
        half: Vec3::new(1.3, 0.45, 5.0),
        draft: 0.6,
        speed: 1.4,
        accel: 0.5,
        turn: 0.3,
        seat: Vec3::new(0.0, 0.1, -4.0),
    },
];

/// A kind by name (a rowboat when unknown).
#[must_use]
pub fn kind_of(name: &str) -> Kind {
    KINDS
        .iter()
        .find(|k| k.name.eq_ignore_ascii_case(name.trim()))
        .copied()
        .unwrap_or(KINDS[0])
}

/// A river's current at its middle (m/s), downstream; canals have none.
pub const CURRENT: f32 = 0.6;
/// How far from its hull a boat can be boarded.
const REACH: f32 = 1.5;
/// Where a boat's water is looked for, in its project (WS1's water surface).
pub const WATER_MAP: &str = "data/scatter/water_surfaces.json";

/// The water at a point: its surface and how it moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Surface {
    pub height: f32,
    pub current: Vec3,
}

#[derive(Deserialize)]
struct MapFile {
    #[serde(default)]
    sea_level: Option<f32>,
    /// Where the scene's origin is in the file's frame (WS1 writes world
    /// metres; the composed scene is the eirvane frame), taken off every point.
    #[serde(default)]
    scene_origin: Option<[f32; 3]>,
    #[serde(default)]
    lakes: Vec<LakeFile>,
    #[serde(default)]
    rivers: Vec<RiverFile>,
}

#[derive(Deserialize)]
struct LakeFile {
    level: f32,
    #[serde(default)]
    frozen: bool,
    outline: Vec<[f32; 2]>,
}

#[derive(Deserialize)]
struct RiverFile {
    #[serde(default)]
    canal: bool,
    /// `[x, z, surface, half_width]` downstream, a fifth value its speed where WS1 gives one.
    pts: Vec<Vec<f32>>,
}

struct Lake {
    level: f32,
    outline: Vec<Vec2>,
    min: Vec2,
    max: Vec2,
}

struct River {
    points: Vec<Vec2>,
    surface: Vec<f32>,
    half: Vec<f32>,
    /// Its current downstream at each point (m/s; none on a canal).
    speed: Vec<f32>,
}

/// River segments by the cells they cross.
const CELL: f32 = 16.0;

/// Where the water is, in the world's frame.
pub struct WaterMap {
    sea_level: Option<f32>,
    lakes: Vec<Lake>,
    rivers: Vec<River>,
    cells: HashMap<(i32, i32), Vec<(u32, u32)>>,
}

fn cell(p: Vec2) -> (i32, i32) {
    ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32)
}

impl WaterMap {
    /// From WS1's water surface JSON (`tools/water_surfaces.py`). Frozen lakes
    /// are ice, not water.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let file: MapFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let o = file.scene_origin.unwrap_or_default();
        let (ox, oy, oz) = (o[0], o[1], o[2]);
        let lakes = file
            .lakes
            .into_iter()
            .filter(|l| !l.frozen && l.outline.len() >= 3)
            .map(|l| {
                let outline: Vec<Vec2> = l
                    .outline
                    .iter()
                    .map(|p| Vec2::new(p[0] - ox, p[1] - oz))
                    .collect();
                let min = outline
                    .iter()
                    .copied()
                    .reduce(Vec2::min)
                    .unwrap_or_default();
                let max = outline
                    .iter()
                    .copied()
                    .reduce(Vec2::max)
                    .unwrap_or_default();
                Lake {
                    level: l.level - oy,
                    outline,
                    min,
                    max,
                }
            })
            .collect();
        let rivers: Vec<River> = file
            .rivers
            .into_iter()
            .filter(|r| r.pts.len() >= 2 && r.pts.iter().all(|p| p.len() >= 4))
            .map(|r| River {
                points: r
                    .pts
                    .iter()
                    .map(|p| Vec2::new(p[0] - ox, p[1] - oz))
                    .collect(),
                surface: r.pts.iter().map(|p| p[2] - oy).collect(),
                half: r.pts.iter().map(|p| p[3]).collect(),
                speed: r
                    .pts
                    .iter()
                    .map(|p| {
                        if r.canal {
                            0.0
                        } else {
                            p.get(4).copied().unwrap_or(CURRENT)
                        }
                    })
                    .collect(),
            })
            .collect();
        let mut cells: HashMap<(i32, i32), Vec<(u32, u32)>> = HashMap::new();
        for (i, r) in rivers.iter().enumerate() {
            for j in 0..r.points.len() - 1 {
                let pad = Vec2::splat(r.half[j].max(r.half[j + 1]));
                let (lo, hi) = (
                    cell(r.points[j].min(r.points[j + 1]) - pad),
                    cell(r.points[j].max(r.points[j + 1]) + pad),
                );
                for cx in lo.0..=hi.0 {
                    for cz in lo.1..=hi.1 {
                        cells
                            .entry((cx, cz))
                            .or_default()
                            .push((i as u32, j as u32));
                    }
                }
            }
        }
        Ok(Self {
            sea_level: file.sea_level.map(|l| l - oy),
            lakes,
            rivers,
            cells,
        })
    }

    /// The water over `at` (the scene's x, z), as WS1 queries it: a lake's
    /// level inside its outline, else the nearest river or canal reach within
    /// its half width (the surface interpolated along it, its current
    /// downstream), else the sea's level (a hull above it floats on nothing).
    #[must_use]
    pub fn surface(&self, at: Vec2) -> Option<Surface> {
        let still = |height| Surface {
            height,
            current: Vec3::ZERO,
        };
        // ponytail: every lake's box is tried; a grid like the rivers' if the map grows past a few hundred lakes.
        if let Some(l) = self
            .lakes
            .iter()
            .find(|l| at.cmpge(l.min).all() && at.cmple(l.max).all() && inside(&l.outline, at))
        {
            return Some(still(l.level));
        }
        let mut best: Option<(f32, Surface)> = None;
        for &(i, j) in self.cells.get(&cell(at)).into_iter().flatten() {
            let r = &self.rivers[i as usize];
            let (j, a, b) = (j as usize, r.points[j as usize], r.points[j as usize + 1]);
            let along = b - a;
            let t = ((at - a).dot(along) / along.length_squared().max(1e-6)).clamp(0.0, 1.0);
            let d = at.distance(a + along * t);
            let lerp = |v: &[f32]| v[j] + (v[j + 1] - v[j]) * t;
            if d > lerp(&r.half) || best.is_some_and(|(e, _)| e <= d) {
                continue;
            }
            let way = along.normalize_or_zero() * lerp(&r.speed);
            best = Some((
                d,
                Surface {
                    height: lerp(&r.surface),
                    current: Vec3::new(way.x, 0.0, way.y),
                },
            ));
        }
        best.map(|(_, s)| s).or_else(|| self.sea_level.map(still))
    }
}

/// Even-odd point in polygon.
fn inside(outline: &[Vec2], p: Vec2) -> bool {
    let mut odd = false;
    let mut j = outline.len() - 1;
    for i in 0..outline.len() {
        let (a, b) = (outline[i], outline[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            odd = !odd;
        }
        j = i;
    }
    odd
}

/// The oars: ahead (1) or astern (-1), and a turn to port (1) or starboard (-1).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Oars {
    pub ahead: f32,
    pub turn: f32,
}

/// Jolt's default density: a hull's mass is its box's volume of water.
const DENSITY: f32 = 1000.0;
const GRAVITY: f32 = 9.81;

/// A Jolt body for a hull of `kind` at `at`.
pub fn hull_body(physics: &mut PhysicsWorld, kind: &Kind, at: Vec3, rotation: Quat) -> BodyId {
    physics.create_body(RigidBodyDescriptor {
        shape: ColliderShape::Box {
            half_extents: kind.half,
        },
        position: at,
        rotation,
        motion_type: MotionType::Dynamic,
        object_layer: LAYER_MOVING,
        friction: 0.3,
        restitution: 0.02,
        linear_damping: 0.05,
        angular_damping: 0.3,
        allow_sleeping: false,
        ..Default::default()
    })
}

/// One fixed step's forces on a hull: buoyancy at eight samples round its
/// bottom against the water map's surface, drag against the water (it slides
/// ahead freely and hardly at all sideways, so it carves its turns and drifts
/// with a river), a righting push, the oars, and, left alone, a pull back to
/// where it was moored. Returns how wet it is (0 dry to 8).
pub fn float(
    physics: &mut PhysicsWorld,
    body: BodyId,
    kind: &Kind,
    water: &WaterMap,
    oars: Oars,
    moored: Option<Vec3>,
) -> f32 {
    let position = physics.get_position(body);
    let rotation = physics.get_rotation(body);
    let velocity = physics.get_linear_velocity(body);
    let spin = physics.get_angular_velocity(body);
    let h = kind.half;
    let mass = 8.0 * h.x * h.y * h.z * DENSITY;
    // Half wet when afloat and still.
    let lift = mass * GRAVITY / 4.0;
    let push = mass * kind.accel;
    // At top speed, eight half-wet samples' drag ahead matches the oars.
    let ahead_drag = push / (4.0 * kind.speed.max(0.1));
    let drag = Vec3::new(ahead_drag * 8.0, ahead_drag * 3.0, ahead_drag);
    let mut wet = 0.0;
    let mut current = Vec3::ZERO;
    for sx in [-1.0, 1.0] {
        for sz in [-0.85, -0.3, 0.3, 0.85] {
            let offset = rotation * Vec3::new(sx * h.x * 0.8, -h.y, sz * h.z);
            let point = position + offset;
            let Some(surface) = water.surface(Vec2::new(point.x, point.z)) else {
                continue;
            };
            let submerged = ((surface.height - point.y) / kind.draft.max(0.05)).clamp(0.0, 1.0);
            if submerged <= 0.0 {
                continue;
            }
            wet += submerged;
            current += surface.current * submerged;
            let relative = rotation.inverse() * (surface.current - (velocity + spin.cross(offset)));
            let resist = rotation * (relative * drag) * submerged;
            physics.add_force_at_position(body, Vec3::Y * lift * submerged + resist, point);
        }
    }
    if wet <= 0.5 {
        return wet;
    }
    // Afloat and still is half wet: the oars at full there.
    let share = (wet / 4.0).clamp(0.0, 1.0);
    // Righting: the keel back under the middle when it leans.
    let up = rotation * Vec3::Y;
    let lean = Vec3::new(up.x, 0.0, up.z);
    if lean.length_squared() > 1e-5 {
        physics.add_force_at_position(
            body,
            -lean * mass * GRAVITY * 2.0 * share,
            position + rotation * Vec3::new(0.0, -h.y, 0.0),
        );
    }
    let forward = rotation * Vec3::Z;
    let flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    let side = Vec3::new(flat.z, 0.0, -flat.x);
    physics.add_force(body, flat * push * oars.ahead.clamp(-0.5, 1.0) * share);
    // A turn is a couple at bow and stern towards the turn's rate, so it
    // eases off at the rate rather than spinning up.
    let inertia = mass * (4.0 * h.z * h.z + 4.0 * h.x * h.x) / 12.0;
    let want = oars.turn.clamp(-1.0, 1.0) * kind.turn;
    // The keel's sideways drag resists the turn (half-wet samples, at their
    // stations): that much more holds the rate once it is reached.
    let keel: f32 = [0.85f32, 0.3]
        .iter()
        .map(|z| 4.0 * 0.5 * drag.x * (z * h.z).powi(2))
        .sum();
    let torque = (inertia * (want - spin.y) * 2.0 + keel * want) * share;
    let couple = torque / (2.0 * h.z);
    physics.add_force_at_position(body, side * couple, position + flat * h.z);
    physics.add_force_at_position(body, -side * couple, position - flat * h.z);
    // Left alone it holds to its mooring (with the river's pull taken off).
    if let Some(home) = moored {
        let off = Vec3::new(home.x - position.x, 0.0, home.z - position.z);
        let still = Vec3::new(velocity.x, 0.0, velocity.z) - current / wet;
        physics.add_force(body, (off * 1.5 - still * 2.5) * mass * share);
    }
    wet
}

/// A boat in Play.
struct Afloat {
    entity: Entity,
    body: BodyId,
    kind: Kind,
    /// Where it was authored (given back at Stop) and where it holds now.
    authored: Transform,
    moored: Option<Vec3>,
    /// The plain hull drawn for a boat with no mesh of its own.
    hull: Vec<Entity>,
}

/// Boats in Play.
#[derive(Default)]
pub struct BoatHost {
    water: Option<WaterMap>,
    /// The water map was looked for (once a Play).
    looked: bool,
    playing: bool,
    afloat: Vec<Afloat>,
    /// The boat the protagonist sits in, and where they stood before boarding.
    aboard: Option<(usize, Vec3)>,
    keys: [bool; 4],
    board: bool,
    /// Seconds to the next look for boats that streamed in or out.
    look_in: f32,
}

impl BoatHost {
    /// Float the scene's boats (Play), on the project's water map.
    pub fn begin(&mut self, ctx: &mut EngineContext) {
        self.end(ctx);
        self.playing = true;
        self.adopt(ctx);
    }

    /// Float boats not yet afloat (a cell streamed them in); let go of those
    /// whose entity went (streamed out).
    fn adopt(&mut self, ctx: &mut EngineContext) {
        let gone: Vec<usize> = (0..self.afloat.len())
            .filter(|&i| ctx.world.get::<Boat>(self.afloat[i].entity).is_none())
            .collect();
        for i in gone.into_iter().rev() {
            let a = self.afloat.remove(i);
            ctx.physics.destroy_body(a.body);
            for e in a.hull {
                ctx.world.despawn(e);
            }
            self.aboard = match self.aboard {
                Some((b, _)) if b == i => None,
                Some((b, stood)) if b > i => Some((b - 1, stood)),
                other => other,
            };
        }
        let boats: Vec<(Entity, Boat, Transform)> = ctx
            .world
            .iter_with::<Boat>()
            .filter(|(e, b)| b.enabled && !self.afloat.iter().any(|a| a.entity == *e))
            .filter_map(|(e, b)| Some((e, b.clone(), *ctx.world.get::<Transform>(e)?)))
            .collect();
        if boats.is_empty() {
            return;
        }
        if !self.looked {
            self.looked = true;
            let path = ctx.config.project_root.as_deref().map_or_else(
                || std::path::PathBuf::from(WATER_MAP),
                |r| r.join(WATER_MAP),
            );
            match std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| WaterMap::from_json(&t))
            {
                Ok(map) => self.water = Some(map),
                Err(error) => {
                    warn!(path = %path.display(), %error, "boats: no water map; they sit where they are")
                }
            }
        }
        for (entity, boat, authored) in boats {
            let kind = kind_of(&boat.kind);
            let at = world_matrix(ctx.world, entity).to_scale_rotation_translation();
            let body = hull_body(ctx.physics, &kind, at.2, at.1);
            if !body.is_valid() {
                warn!(boat = %name_of(ctx.world, entity), "boats: its hull could not be made");
                continue;
            }
            let has_mesh = ctx
                .world
                .iter_with::<Parent>()
                .any(|(_, p)| p.entity == entity)
                || ctx.world.get::<MeshKind>(entity).is_some();
            // Its own mesh, else rb-architecture's hull for its kind, else a plain one.
            let hull = if has_mesh {
                Vec::new()
            } else {
                let file = format!("props/boats/boat_{}.glb", kind.name);
                let nodes = crate::combat_bodies::mesh_nodes(&ctx.config.content_root.join(&file));
                if nodes.is_empty() {
                    plain_hull(ctx.world, entity, &kind, authored.scale)
                } else {
                    hull_mesh(ctx.world, entity, &file, &nodes, authored.scale)
                }
            };
            self.afloat.push(Afloat {
                entity,
                body,
                kind,
                authored,
                moored: Some(at.2),
                hull,
            });
        }
        info!(
            boats = self.afloat.len(),
            water = self.water.is_some(),
            "boats afloat"
        );
    }

    /// Back where they were authored (Stop).
    pub fn end(&mut self, ctx: &mut EngineContext) {
        self.playing = false;
        self.looked = false;
        for a in self.afloat.drain(..) {
            ctx.physics.destroy_body(a.body);
            if let Some(t) = ctx.world.get_mut::<Transform>(a.entity) {
                *t = a.authored;
            }
            for e in a.hull {
                ctx.world.despawn(e);
            }
        }
        self.aboard = None;
        self.water = None;
    }

    pub fn event(&mut self, event: &EngineEvent) {
        if let EngineEvent::KeyInput { key, state } = event {
            let down = *state == InputState::Pressed;
            match key {
                KeyCode::KeyW => self.keys[0] = down,
                KeyCode::KeyS => self.keys[1] = down,
                KeyCode::KeyA => self.keys[2] = down,
                KeyCode::KeyD => self.keys[3] = down,
                KeyCode::KeyE if down => self.board = true,
                _ => {}
            }
        }
    }

    /// Boarding and leaving (E), and each hull's pose onto its entity (once a frame, before the fight's).
    pub fn update(&mut self, ctx: &mut EngineContext, combat: &mut crate::combat_demo::CombatDemo) {
        let pressed = std::mem::take(&mut self.board);
        if !self.playing {
            return;
        }
        self.look_in -= ctx.dt();
        if self.look_in <= 0.0 {
            self.look_in = 1.0;
            self.adopt(ctx);
        }
        for a in &self.afloat {
            let pose = Mat4::from_rotation_translation(
                ctx.physics.get_rotation(a.body),
                ctx.physics.get_position(a.body),
            );
            // Its Transform is its parent's frame's.
            let parent = ctx
                .world
                .get::<Parent>(a.entity)
                .map(|p| world_matrix(ctx.world, p.entity));
            let (_, turn, at) = parent
                .map_or(pose, |m| m.inverse() * pose)
                .to_scale_rotation_translation();
            if let Some(t) = ctx.world.get_mut::<Transform>(a.entity) {
                t.translation = at;
                t.rotation = turn;
            }
        }
        if !pressed {
            return;
        }
        match self.aboard.take() {
            Some((i, stood)) => {
                // Off beside the hull, on the side nearer where they boarded.
                let a = &mut self.afloat[i];
                let (at, turn) = (
                    ctx.physics.get_position(a.body),
                    ctx.physics.get_rotation(a.body),
                );
                let off = (a.kind.half.x + 0.8) * (turn * Vec3::X);
                let step = [at + off, at - off]
                    .into_iter()
                    .min_by(|p, q| p.distance(stood).total_cmp(&q.distance(stood)))
                    .unwrap_or(at);
                a.moored = Some(at);
                combat.disembark(step + Vec3::Y * a.kind.half.y);
                combat.take_e = true;
            }
            None => {
                let Some((feet, riding)) = combat.player_feet() else {
                    return;
                };
                if riding {
                    return;
                }
                let near = self.afloat.iter().position(|a| {
                    let local = Mat4::from_rotation_translation(
                        ctx.physics.get_rotation(a.body),
                        ctx.physics.get_position(a.body),
                    )
                    .inverse()
                    .transform_point3(feet);
                    local.x.abs() <= a.kind.half.x + REACH && local.z.abs() <= a.kind.half.z + REACH
                });
                if let Some(i) = near {
                    self.aboard = Some((i, feet));
                    self.afloat[i].moored = None;
                    combat.take_e = true;
                }
            }
        }
    }

    /// One fixed step of every hull (before the fight's, and before Jolt's).
    pub fn fixed(&mut self, ctx: &mut EngineContext, combat: &mut crate::combat_demo::CombatDemo) {
        let Some(water) = self.water.as_ref() else {
            combat.aboard = None;
            return;
        };
        let k = |i: usize| if self.keys[i] { 1.0 } else { 0.0 };
        let rowing = Oars {
            ahead: k(0) - k(1),
            turn: k(2) - k(3),
        };
        for (i, a) in self.afloat.iter_mut().enumerate() {
            let mine = self.aboard.is_some_and(|(b, _)| b == i);
            let oars = if mine { rowing } else { Oars::default() };
            // An empty boat holds to wherever it was left (stepped off, or a load took its rower away).
            if !mine && a.moored.is_none() {
                a.moored = Some(ctx.physics.get_position(a.body));
            }
            float(ctx.physics, a.body, &a.kind, water, oars, a.moored);
        }
        combat.aboard = self.aboard.map(|(i, _)| {
            let a = &self.afloat[i];
            let hull = Mat4::from_rotation_translation(
                ctx.physics.get_rotation(a.body),
                ctx.physics.get_position(a.body),
            );
            hull * Mat4::from_translation(a.kind.seat + Vec3::Y * a.kind.half.y)
        });
    }

    /// Nobody in any boat (a load put the protagonist elsewhere).
    pub fn cast_off(&mut self) {
        self.aboard = None;
    }

    /// For MCP and automation.
    pub fn state(&self) -> serde_json::Value {
        serde_json::json!({
            "boats": self.afloat.len(),
            "water": self.water.is_some(),
            "aboard": self.aboard.map(|(i, _)| self.afloat[i].kind.name),
        })
    }
}

fn world_matrix(world: &World, e: Entity) -> Mat4 {
    world
        .get::<WorldTransform>(e)
        .map(|w| w.0)
        .filter(|m| *m != Mat4::IDENTITY)
        .or_else(|| world.get::<Transform>(e).map(Transform::to_matrix))
        .unwrap_or(Mat4::IDENTITY)
}

fn name_of(world: &World, e: Entity) -> String {
    world
        .get::<Name>(e)
        .map_or_else(|| "Boat".into(), |n| n.as_str().to_string())
}

/// rb-architecture's hull for the kind (`assets/props/boats/boat_<kind>.glb`:
/// its origin the hull box's middle, +Z the bow), every mesh node of it.
fn hull_mesh(
    world: &mut World,
    boat: Entity,
    file: &str,
    nodes: &[u32],
    scale: Vec3,
) -> Vec<Entity> {
    nodes
        .iter()
        .map(|&node| {
            world.spawn((
                Name::new("Boat hull"),
                Transform {
                    scale: Vec3::ONE / scale.max(Vec3::splat(1e-3)),
                    ..Default::default()
                },
                WorldTransform::identity(),
                Parent { entity: boat },
                somnium_core::ImportedMesh {
                    source: format!("assets/{file}"),
                    node,
                },
                somnium_core::MaterialComponent {
                    asset: somnium_asset::database::AssetId::NONE,
                    runtime_id: 0,
                },
            ))
        })
        .collect()
}

/// A plain hull (bottom and two sides) for a boat with no mesh of its own.
fn plain_hull(world: &mut World, boat: Entity, kind: &Kind, scale: Vec3) -> Vec<Entity> {
    let h = kind.half / scale.max(Vec3::splat(1e-3));
    let wall = 0.06 / scale.x.max(1e-3);
    [
        (
            Vec3::new(0.0, -h.y * 0.8, 0.0),
            Vec3::new(h.x, h.y * 0.2, h.z),
        ),
        (Vec3::new(h.x - wall, 0.0, 0.0), Vec3::new(wall, h.y, h.z)),
        (Vec3::new(-h.x + wall, 0.0, 0.0), Vec3::new(wall, h.y, h.z)),
    ]
    .into_iter()
    .map(|(at, half)| {
        world.spawn((
            Name::new("Boat hull"),
            Transform {
                translation: at,
                scale: half * 2.0,
                ..Default::default()
            },
            WorldTransform::identity(),
            Parent { entity: boat },
            MeshKind::Cube,
            crate::combat_demo::material("hide"),
        ))
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_physics::config::PhysicsConfig;

    /// Jolt keeps process-wide state: one world at a time, the combat host's
    /// tests included (one lock for the crate).
    use crate::combat_demo::JOLT;

    fn map() -> WaterMap {
        // In world metres with the scene's origin at (1000, 0, 2000): a square lake at 10 m round the scene's (0, 0);
        // a river from the scene's (100, 0) east, falling 2 m over 100 m, 8 m wide; a canal; a frozen lake.
        WaterMap::from_json(
            r#"{"sea_level": 0.0, "scene_origin": [1000, 0, 2000],
            "lakes": [{"level": 10.0, "outline": [[980, 1980], [1020, 1980], [1020, 2020], [980, 2020]]},
                      {"level": 50.0, "frozen": true, "outline": [[1300, 2300], [1340, 2300], [1340, 2340], [1300, 2340]]}],
            "rivers": [{"pts": [[1100, 2000, 8, 4], [1150, 2000, 7, 4], [1200, 2000, 6, 4]]},
                       {"canal": true, "pts": [[1000, 2100, 0.6, 5], [1000, 2200, 0.6, 5]]}]}"#,
        )
        .unwrap()
    }

    /// The project's real water and moorings (`RIMEBORN=<games/Rimeborn>`): the
    /// map reads WS1's file, and every mooring is on water at about its height.
    #[test]
    #[ignore = "needs RIMEBORN pointing at the game's folder"]
    fn every_mooring_is_on_the_real_water() {
        let root = std::path::PathBuf::from(std::env::var("RIMEBORN").expect("RIMEBORN"));
        let map =
            WaterMap::from_json(&std::fs::read_to_string(root.join(WATER_MAP)).unwrap()).unwrap();
        let moorings: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("data/boats/moorings.json")).unwrap(),
        )
        .unwrap();
        let mut off = Vec::new();
        let all = moorings["moorings"].as_array().unwrap();
        for m in all {
            let at: Vec<f32> = m["at"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap() as f32)
                .collect();
            match map.surface(Vec2::new(at[0], at[2])) {
                Some(s) if (s.height - (at[1] - 0.15)).abs() < 0.3 => {}
                other => off.push((m["place"].clone(), at, other.map(|s| s.height))),
            }
        }
        println!("{} moorings, {} off their water", all.len(), off.len());
        assert!(off.is_empty(), "{off:?}");
    }

    /// A river's surface slopes along it and runs downstream; a lake is level
    /// inside its outline; a canal is still; the sea is everywhere else.
    #[test]
    fn the_water_map_finds_rivers_lakes_canals_and_sea() {
        let m = map();
        let r = m.surface(Vec2::new(125.0, 3.0)).unwrap();
        assert!((r.height - 7.5).abs() < 1e-4);
        assert!(
            (r.current - Vec3::X * CURRENT).length() < 1e-4,
            "downstream"
        );
        assert_eq!(
            m.surface(Vec2::new(125.0, 4.5)).unwrap().height,
            0.0,
            "past the bank: the sea"
        );
        assert_eq!(m.surface(Vec2::new(5.0, -5.0)).unwrap().height, 10.0);
        assert_eq!(
            m.surface(Vec2::new(4.0, 150.0)).unwrap().current,
            Vec3::ZERO,
            "a canal"
        );
        assert_eq!(
            m.surface(Vec2::new(320.0, 320.0)).unwrap().height,
            0.0,
            "a frozen lake is ice: the sea's"
        );
        let none = WaterMap::from_json(r#"{"lakes": [], "rivers": []}"#).unwrap();
        assert!(none.surface(Vec2::ZERO).is_none(), "no sea, no water");
        assert_eq!(kind_of("Fjord").name, "fjord");
        assert_eq!(kind_of("raft").name, "rowboat");
    }

    fn sail(at: Vec3, oars: Oars, seconds: f32) -> (Vec3, Quat) {
        let _jolt = JOLT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut physics = PhysicsWorld::new(PhysicsConfig::default());
        let kind = kind_of("rowboat");
        let water = map();
        let body = hull_body(&mut physics, &kind, at, Quat::IDENTITY);
        for _ in 0..(seconds * 60.0) as usize {
            float(&mut physics, body, &kind, &water, oars, None);
            physics.step(1.0 / 60.0);
        }
        let out = (physics.get_position(body), physics.get_rotation(body));
        physics.destroy_body(body);
        out
    }

    /// Dropped onto the lake it settles afloat, level, a little in the water.
    #[test]
    fn a_boat_floats_level_on_a_lake() {
        let (at, turn) = sail(Vec3::new(0.0, 11.0, 0.0), Oars::default(), 8.0);
        let bottom = at.y - kind_of("rowboat").half.y;
        assert!(
            bottom < 10.0 && bottom > 9.6,
            "its bottom {bottom} just under the level"
        );
        assert!((turn * Vec3::Y).y > 0.99, "upright");
        assert!(Vec2::new(at.x, at.z).length() < 0.5, "and stays put");
    }

    /// Rowed, it goes ahead (+Z, its bow) at about its top speed; turning, it turns.
    #[test]
    fn a_boat_rows_ahead_and_turns() {
        let (at, _) = sail(
            Vec3::new(0.0, 10.1, -15.0),
            Oars {
                ahead: 1.0,
                turn: 0.0,
            },
            8.0,
        );
        assert!(
            at.z > -6.0 && at.z < 3.0,
            "up to about 2 m/s in 2 s, 8 s from -15: {at}"
        );
        let (_, turn) = sail(
            Vec3::new(0.0, 10.1, 0.0),
            Oars {
                ahead: 0.0,
                turn: 1.0,
            },
            2.0,
        );
        let bow = turn * Vec3::Z;
        assert!(bow.x > 0.5, "turned to port, its bow towards +X: {bow}");
    }

    /// On a sloped river it rides the surface and drifts downstream.
    #[test]
    fn a_boat_drifts_down_a_sloped_river() {
        let (at, _) = sail(Vec3::new(110.0, 8.2, 0.0), Oars::default(), 10.0);
        assert!(at.x > 113.0, "downstream from 110: {at}");
        let surface = 8.0 - (at.x - 100.0) / 50.0;
        assert!(
            (at.y - surface).abs() < 0.3,
            "on the slope: {at} by {surface}"
        );
    }
}
