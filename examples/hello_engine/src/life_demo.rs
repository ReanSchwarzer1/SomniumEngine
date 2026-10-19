//! Living Village: ambient life (`somnium_life`) in the editor.
//!
//! **Create > Examples > Living Village** builds a hamlet under one root with a
//! **Life Place**: houses along a high street and two back lanes (each door a
//! **Life Spot** of kind Home), an inn, a chapel, a smithy and a storehouse, a
//! green with a well, a pen and two fields; the lanes are **Splines** (people
//! walk them, carts drive them). Its Homes setting sizes it: a few dozen up to
//! a small town of hundreds.
//!
//! **Play** and walk among them (the first-person walker). With Populate on,
//! each home has a household with a trade each; they keep their day (fields
//! from dawn, the smithy, market, prayer, the inn of an evening, bed),
//! livestock grazes in the pen and carts ply the street by day. The day runs
//! by the scene's **Time of Day** when it moves (its timescale), else an hour
//! a minute (a day in 24 minutes) from half six. Raise Time of Day's **Eclipse** past
//! half and the Lid sends everyone indoors.
//!
//! Simulation LOD measures from the camera: villagers near it step every
//! frame, further out every few, far off rarely; the far and the indoor are
//! not drawn. Only the nearest 160 villagers, 48 beasts and 16 carts out of
//! doors get a body (`somnium_life::Bodies`), so a town of any size costs that
//! many entities. The step and the transform writes are timed and reported to MCP
//! (`life` in the authoring state) and the log every ten seconds.
//!
//! Places stream: a Life Place (or a hand-placed agent) that a streamed cell
//! installs after Play began is adopted when it appears, keyed by its name and
//! where it stands; when its cell unloads its people live on, and when it
//! streams back it finds them again (`LifeWorld::adopt_place`).
//!
//! A **Life Table** names a realm's places in one file (a game's tool writes it
//! from its settlements): each town is housed when the view comes within its
//! reach, and lives on cheaply after. Villagers stand on the scene's terrain.
//!
//! **B** rings the alarm bell where the camera is: everyone within 80 m runs
//! home and stays in for 45 seconds, beasts bolt, carts stop
//! (`LifeWorld::alarm`). A fight rings it too: wherever a **Combatant** in the
//! scene loses health, everyone within 60 m runs home. Walk past villagers
//! and those standing turn to look at you (`LifeWorld::notice`; a game plays
//! greeting barks for them); nobody walks through you (`LifeWorld::keep_clear`).
use glam::{Mat4, Quat, Vec3};
use serde_json::{Value, json};
use somnium_asset::database::AssetId;
use somnium_combat::components::{CombatSurfaceComponent, CombatantComponent};
use somnium_combat::environment::Surface;
use somnium_core::spline::{SAMPLES_PER_SEGMENT, SplineComponent};
use somnium_core::time_of_day::TimeOfDayComponent;
use somnium_core::{
    EngineContext, EngineEvent, InputState, KeyCode, MaterialComponent, MeshKind, Name, Parent,
    Transform, WorldTransform,
};
use somnium_ecs::{Entity, World};
use somnium_life::components::{
    CartComponent, LifePlaceComponent, LifeSpotComponent, LifeTableComponent, LivestockComponent,
    VillagerComponent,
};
use somnium_life::{
    BeastKind, Bodies, Clock, Ground, LifeTable, LifeWorld, LodConfig, Place, Role, Spot, SpotKind,
    Tier, VillagerDesc,
};
use somnium_physics::body::{BodyId, MotionType, RigidBodyDescriptor};
use somnium_physics::shape::ColliderShape;
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tracing::{info, warn};

/// Real seconds to a village hour when no Time of Day moves the clock.
const SECONDS_PER_HOUR: f32 = 60.0;
/// Where hidden visuals wait.
const PARKED: Vec3 = Vec3::new(0.0, -10_000.0, 0.0);
/// Bodies for the populated: only the nearest this many villagers, beasts and
/// carts out of doors are drawn, so a town of any size costs these entities.
const VILLAGER_BODIES: usize = 160;
const BEAST_BODIES: usize = 48;
const CART_BODIES: usize = 16;
/// The alarm bell (B): how far it frightens, and for how long.
const BELL_REACH: f32 = 80.0;
const BELL_SECONDS: f32 = 45.0;
/// A fight frightens this far.
const FIGHT_REACH: f32 = 60.0;

fn material(name: &str) -> MaterialComponent {
    MaterialComponent {
        asset: AssetId::from_relative_path(std::path::Path::new(&format!(
            "materials/combat/{name}.sommat"
        ))),
        runtime_id: 0,
    }
}

pub fn register(r: &mut somnium_core::authoring::GameRegistration) {
    r.components.push(somnium_life::components::register);
    r.presets.push(somnium_core::authoring::GamePreset {
        id: "hello.living_village",
        label: "Living Village",
        category: "Examples",
        schema: json!({"type":"object","properties":{
            "position":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},
            "homes":{"type":"integer","minimum":4,"maximum":400,"description":"Houses: a hamlet of a dozen to a small town of hundreds."}
        },"additionalProperties":false}),
        build: spawn_village,
    });
}

fn origin(args: &Value) -> Result<Vec3, String> {
    let p = Vec3::from_array(
        serde_json::from_value(args.get("position").cloned().unwrap_or(json!([0, 0.0, 0])))
            .map_err(|e| format!("{e}"))?,
    );
    if !p.is_finite() {
        return Err("Position must be finite".into());
    }
    Ok(p)
}

fn block(
    world: &mut World,
    root: Entity,
    name: &str,
    at: Vec3,
    scale: Vec3,
    kind: MeshKind,
    mat: &str,
) -> Entity {
    world.spawn((
        Name::new(name),
        Transform {
            translation: at,
            scale,
            ..Default::default()
        },
        WorldTransform::identity(),
        Parent { entity: root },
        kind,
        material(mat),
    ))
}

fn solid(world: &mut World, e: Entity) {
    let _ = world.insert_component(
        e,
        CombatSurfaceComponent {
            surface: Surface::Bare as u32,
            round: false,
            solid: true,
        },
    );
}

fn spot(
    world: &mut World,
    root: Entity,
    name: &str,
    at: Vec3,
    kind: SpotKind,
    size: Vec3,
) -> Entity {
    world.spawn((
        Name::new(name),
        Transform {
            translation: at,
            scale: size,
            ..Default::default()
        },
        WorldTransform::identity(),
        Parent { entity: root },
        LifeSpotComponent { kind: kind as u32 },
    ))
}

fn lane(world: &mut World, root: Entity, name: &str, points: Vec<Vec3>) -> Entity {
    world.spawn((
        Name::new(name),
        Transform::default(),
        WorldTransform::identity(),
        Parent { entity: root },
        SplineComponent {
            points,
            closed: false,
            tangents: Vec::new(),
        },
    ))
}

/// A building: a solid box and its door spot `door_side` metres in front.
#[allow(clippy::too_many_arguments)]
fn building(
    world: &mut World,
    root: Entity,
    name: &str,
    at: Vec3,
    size: Vec3,
    mat: &str,
    kind: SpotKind,
    facing: f32,
    out: &mut Vec<Entity>,
) {
    let body = block(
        world,
        root,
        name,
        at + Vec3::Y * size.y * 0.5,
        size,
        MeshKind::Cube,
        mat,
    );
    solid(world, body);
    out.push(body);
    let door = at + Vec3::new(0.0, 0.0, facing * (size.z * 0.5 + 0.6));
    out.push(spot(
        world,
        root,
        &format!("{name} door"),
        door,
        kind,
        Vec3::ONE,
    ));
}

fn spawn_village(world: &mut World, args: &Value) -> Result<Vec<Entity>, String> {
    let at = origin(args)?;
    let homes = args
        .get("homes")
        .and_then(Value::as_u64)
        .unwrap_or(14)
        .clamp(4, 400) as usize;
    // Rows of plots either side of the high street and of back lanes, nine
    // metres a plot, thirteen to a row.
    let per_row = 13usize;
    let rows = homes.div_ceil(per_row);
    let streets = rows.div_ceil(2);
    let half_width = per_row as f32 * 9.0 * 0.5 + 6.0;
    let reach = streets as f32 * 22.0 + 40.0;
    let root = world.spawn((
        Name::new("Living Village"),
        Transform::from_translation(at),
        WorldTransform::identity(),
        LifePlaceComponent::default(),
    ));
    let mut out = vec![root];
    let ground = block(
        world,
        root,
        "Village ground",
        Vec3::new(0.0, -0.1, 0.0),
        Vec3::new(half_width * 2.0 + 60.0, 0.2, reach * 2.0 + 80.0),
        MeshKind::Cube,
        "floor",
    );
    solid(world, ground);
    out.push(ground);
    // Streets along X at z = 0, +-22, +-44 ...; the high street at 0.
    let mut street_z = vec![0.0f32];
    for k in 1..streets {
        let z = k as f32 * 22.0;
        street_z.push(if k % 2 == 1 { z } else { -z });
    }
    for (k, z) in street_z.iter().enumerate() {
        let name = if k == 0 {
            "High street".to_string()
        } else {
            format!("Back lane {k}")
        };
        out.push(lane(
            world,
            root,
            &name,
            vec![
                Vec3::new(-half_width, 0.0, *z),
                Vec3::new(0.0, 0.0, *z),
                Vec3::new(half_width, 0.0, *z),
            ],
        ));
    }
    // A cross lane joins them and runs on to the chapel and the fields.
    let north = street_z.iter().copied().fold(0.0f32, f32::max) + 30.0;
    let south = street_z.iter().copied().fold(0.0f32, f32::min) - 34.0;
    // It shares a point with every street it crosses, so they join.
    let mut cross: Vec<f32> = street_z.clone();
    cross.extend([south, north]);
    cross.sort_by(f32::total_cmp);
    out.push(lane(
        world,
        root,
        "Church lane",
        cross.into_iter().map(|z| Vec3::new(0.0, 0.0, z)).collect(),
    ));
    // Houses: rows either side of each street, doors to the street.
    let mut placed = 0;
    'rows: for z in &street_z {
        for side in [1.0f32, -1.0] {
            for i in 0..per_row {
                if placed == homes {
                    break 'rows;
                }
                let x = -half_width + 6.0 + i as f32 * 9.0;
                if x.abs() < 5.0 {
                    continue; // the cross lane
                }
                let size = Vec3::new(6.0, 3.6 + (i % 3) as f32 * 0.6, 5.0);
                let mat = if (placed + i) % 3 == 0 {
                    "stone"
                } else {
                    "hide"
                };
                building(
                    world,
                    root,
                    &format!("House {}", placed + 1),
                    Vec3::new(x, 0.0, z + side * 6.5),
                    size,
                    mat,
                    SpotKind::Home,
                    -side,
                    &mut out,
                );
                placed += 1;
            }
        }
    }
    // The trades and the common ground, round the cross lane.
    let mid = 11.0;
    building(
        world,
        root,
        "The inn",
        Vec3::new(12.0, 0.0, mid),
        Vec3::new(9.0, 5.0, 7.0),
        "raider",
        SpotKind::Inn,
        -1.0,
        &mut out,
    );
    building(
        world,
        root,
        "The smithy",
        Vec3::new(-12.0, 0.0, mid),
        Vec3::new(6.0, 4.0, 6.0),
        "dark",
        SpotKind::Work,
        -1.0,
        &mut out,
    );
    building(
        world,
        root,
        "The storehouse",
        Vec3::new(-12.0, 0.0, -mid),
        Vec3::new(7.0, 4.5, 6.0),
        "hide",
        SpotKind::Work,
        1.0,
        &mut out,
    );
    building(
        world,
        root,
        "The chapel",
        Vec3::new(0.0, 0.0, north + 8.0),
        Vec3::new(8.0, 9.0, 12.0),
        "stone",
        SpotKind::Chapel,
        -1.0,
        &mut out,
    );
    let green = Vec3::new(12.0, 0.012, -mid);
    out.push(block(
        world,
        root,
        "The green",
        green,
        Vec3::new(10.0, 0.02, 8.0),
        MeshKind::Cube,
        "grass",
    ));
    out.push(spot(
        world,
        root,
        "The green spot",
        green,
        SpotKind::Green,
        Vec3::new(10.0, 1.0, 8.0),
    ));
    out.push(spot(
        world,
        root,
        "The market",
        green + Vec3::new(0.0, 0.0, 0.0),
        SpotKind::Market,
        Vec3::new(8.0, 1.0, 6.0),
    ));
    out.push(block(
        world,
        root,
        "The well",
        green + Vec3::new(5.5, 0.4, 0.0),
        Vec3::new(1.2, 0.8, 1.2),
        MeshKind::Cylinder,
        "stone",
    ));
    out.push(spot(
        world,
        root,
        "The well spot",
        green + Vec3::new(5.5, 0.0, 1.5),
        SpotKind::Well,
        Vec3::splat(2.0),
    ));
    // Two fields and a pen down the cross lane.
    for (name, x) in [("West field", -26.0f32), ("East field", 26.0)] {
        let c = Vec3::new(x, 0.012, south - 8.0);
        out.push(block(
            world,
            root,
            name,
            c,
            Vec3::new(36.0, 0.02, 24.0),
            MeshKind::Cube,
            "ash",
        ));
        out.push(spot(
            world,
            root,
            &format!("{name} spot"),
            c,
            SpotKind::Field,
            Vec3::new(36.0, 1.0, 24.0),
        ));
    }
    let pen = Vec3::new(-half_width + 10.0, 0.0, north - 6.0);
    out.push(spot(
        world,
        root,
        "The pen",
        pen,
        SpotKind::Pen,
        Vec3::new(14.0, 1.0, 14.0),
    ));
    for (dx, dz, sx, sz) in [
        (0.0, 7.0, 14.0, 0.2),
        (0.0, -7.0, 14.0, 0.2),
        (7.0, 0.0, 0.2, 14.0),
        (-7.0, 0.0, 0.2, 14.0),
    ] {
        out.push(block(
            world,
            root,
            "Pen fence",
            pen + Vec3::new(dx, 0.5, dz),
            Vec3::new(sx, 1.0, sz),
            MeshKind::Cube,
            "hide",
        ));
    }
    Ok(out)
}

/// Build the village at the origin outside the Create menu (a capture hook):
/// `SOMNIUM_LIFE_VILLAGE=1`, `SOMNIUM_LIFE_HOMES` for its size.
pub fn spawn_at_startup(world: &mut World) -> Result<usize, String> {
    let homes = std::env::var("SOMNIUM_LIFE_HOMES")
        .ok()
        .and_then(|h| h.parse::<u64>().ok())
        .unwrap_or(14);
    spawn_village(world, &json!({ "homes": homes })).map(|e| e.len())
}

fn world_matrix(world: &World, entity: Entity) -> Mat4 {
    let mut m = Mat4::IDENTITY;
    let mut e = entity;
    for _ in 0..64 {
        let Some(t) = world.get::<Transform>(e) else {
            break;
        };
        m = t.to_matrix() * m;
        match world.get::<Parent>(e) {
            Some(p) if p.entity != Entity::DANGLING => e = p.entity,
            _ => break,
        }
    }
    m
}

fn under(world: &World, entity: Entity, root: Entity) -> bool {
    let mut e = entity;
    for _ in 0..64 {
        if e == root {
            return true;
        }
        match world.get::<Parent>(e) {
            Some(p) if p.entity != Entity::DANGLING => e = p.entity,
            _ => return false,
        }
    }
    false
}

fn yaw(facing: Vec3) -> Quat {
    let f = Vec3::new(facing.x, 0.0, facing.z).normalize_or(Vec3::Z);
    Quat::from_rotation_y(f.x.atan2(f.z))
}

#[derive(Clone, Copy, PartialEq)]
struct Look(Vec3, Quat);

/// Who an agent's visual is.
#[derive(Clone, Copy)]
enum Visual {
    Villager(usize),
    Beast(usize),
    Cart(usize),
}

struct Run {
    life: LifeWorld,
    /// Hand-placed agents' own entities.
    visuals: Vec<(Entity, Visual, Vec3)>,
    /// Pooled bodies for everyone the places populated.
    villagers: Pool,
    beasts: Pool,
    carts: Pool,
    looks: HashMap<Entity, Look>,
    /// Hand-placed entities' authored transforms, put back on Stop.
    authored: HashMap<Entity, Transform>,
    /// What each hand-placed villager was last shown doing.
    readouts: HashMap<Entity, String>,
    links: Links,
    /// Seconds since the scene was last looked over for places and agents.
    synced: f32,
    /// Villagers who have noticed the camera walking past, all told.
    greeted: usize,
    /// Each combatant's health when last looked at, to hear a fight.
    health: HashMap<Entity, f32>,
    /// Alarms rung by fights, all told.
    fights: usize,
    /// A Life Table's places, housed as the view comes near.
    table: Option<TableRun>,
    spawned: Vec<Entity>,
    colliders: Vec<BodyId>,
    hour: f32,
    eclipse: f32,
    step_ms: f64,
    write_ms: f64,
    frames: u32,
    reported: f32,
}

/// One kind's bodies: each slot's entity (and a villager's head).
struct Pool {
    bodies: Bodies<usize>,
    slots: Vec<(Entity, Option<Entity>)>,
    /// Agents of this kind shown by their own hand-placed entity.
    own: HashSet<usize>,
}

impl Pool {
    fn new(
        world: &mut World,
        spawned: &mut Vec<Entity>,
        parent: Entity,
        (n, name, mesh, head): (usize, &str, MeshKind, bool),
        own: HashSet<usize>,
    ) -> Self {
        let slots = (0..n)
            .map(|_| {
                let body = world.spawn((
                    Name::new(name),
                    Transform {
                        translation: PARKED,
                        ..Default::default()
                    },
                    WorldTransform::identity(),
                    Parent { entity: parent },
                    mesh,
                    material("ash"),
                ));
                spawned.push(body);
                let head = head.then(|| {
                    let h = world.spawn((
                        Name::new(&format!("{name} head")),
                        Transform::from_translation(Vec3::new(0.0, 0.62, 0.0)),
                        WorldTransform::identity(),
                        Parent { entity: body },
                        MeshKind::Sphere,
                        material("skin"),
                    ));
                    spawned.push(h);
                    h
                });
                (body, head)
            })
            .collect();
        Self {
            bodies: Bodies::new(n),
            slots,
            own,
        }
    }

    /// Hand the bodies to the nearest `candidates`; the slots that changed
    /// hands, with their new holder.
    fn assign(&mut self, mut candidates: Vec<(usize, f32)>) -> Vec<(usize, Option<usize>)> {
        self.bodies
            .assign(&mut candidates)
            .into_iter()
            .map(|slot| (slot, self.bodies.holder(slot)))
            .collect()
    }
}

fn villager_size(role: Role) -> Vec3 {
    if role == Role::Child {
        Vec3::new(0.38, 1.05, 0.38)
    } else {
        Vec3::new(0.5, 1.45, 0.5)
    }
}

/// Dress a pooled entity for its new holder.
fn dress(world: &mut World, entity: Entity, name: &str, size: Vec3, look: &str) {
    if let Some(t) = world.get_mut::<Transform>(entity) {
        t.scale = size;
    }
    if let Some(n) = world.get_mut::<Name>(entity) {
        *n = Name::new(name);
    }
    if let Some(m) = world.get_mut::<MaterialComponent>(entity) {
        *m = material(look);
    }
}

#[derive(Default)]
pub struct LifeDemo {
    run: Option<Run>,
    /// Seconds since Play last looked for life that had not streamed in yet.
    idle: f32,
    /// B was pressed: ring the alarm bell next frame.
    bell: bool,
}

fn role_material(role: Role) -> &'static str {
    match role {
        Role::Farmer => "party",
        Role::Smith => "dark",
        Role::Herder => "hide",
        Role::Keeper => "raider",
        Role::Priest => "mending",
        Role::Elder => "snow",
        Role::Labourer => "frost",
        Role::Child => "fire",
    }
}

fn beast_look(kind: BeastKind) -> (Vec3, &'static str) {
    match kind {
        BeastKind::Sheep => (Vec3::new(0.6, 0.75, 1.0), "snow"),
        BeastKind::Cow => (Vec3::new(0.9, 1.4, 2.1), "hide"),
        BeastKind::Goat => (Vec3::new(0.45, 0.7, 0.9), "ash"),
        BeastKind::Pig => (Vec3::new(0.6, 0.6, 1.1), "skin"),
        BeastKind::Hen => (Vec3::new(0.25, 0.35, 0.3), "fire"),
        BeastKind::Dog => (Vec3::new(0.3, 0.55, 0.8), "dark"),
    }
}

/// What the scene declares, kept in step as cells stream in and out: places
/// adopted by a stable key (housed the first time only) and hand-placed
/// agents linked to their entities. An entity that streams out leaves its
/// place or agent living on in life (an agent with no entity of its own is
/// drawn from the pool); one that streams back finds it again by its key.
#[derive(Default)]
struct Links {
    places: HashMap<Entity, usize>,
    agents: HashMap<Entity, Visual>,
    keys: HashMap<u64, Visual>,
    /// The world's entity count when last synced.
    seen: usize,
}

/// A Life Table's places and which have been housed.
struct TableRun {
    entity: Entity,
    table: LifeTable,
    adopted: Vec<bool>,
    reach: f32,
    count: u32,
}

/// Read the scene's Life Table, if it has one.
fn load_table(world: &World, root: Option<&std::path::Path>) -> Option<TableRun> {
    let (entity, settings) = world
        .iter_with::<LifeTableComponent>()
        .find(|(_, t)| t.enabled && !t.file.is_empty())
        .map(|(e, t)| (e, t.clone()))?;
    let path = root.map_or_else(
        || std::path::PathBuf::from(&settings.file),
        |r| r.join(&settings.file),
    );
    match std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|text| LifeTable::from_json(&text))
    {
        Ok(table) => {
            info!(
                places = table.places.len(),
                "life: table {}",
                path.display()
            );
            Some(TableRun {
                entity,
                adopted: vec![false; table.places.len()],
                table,
                reach: settings.reach,
                count: 0,
            })
        }
        Err(error) => {
            warn!(%error, "life: no table at {}", path.display());
            None
        }
    }
}

/// House the table's towns the view has come within reach of; how many.
fn adopt_near(run: &mut Run, at: Vec3) -> usize {
    let Some(t) = run.table.as_mut() else {
        return 0;
    };
    let near: Vec<usize> = t
        .table
        .near(at, t.reach)
        .filter(|i| !t.adopted[*i])
        .collect();
    for &i in &near {
        let p = &t.table.places[i];
        run.life.adopt_place(p.key(), p.place.clone(), Some(p.seed));
        t.adopted[i] = true;
    }
    t.count += near.len() as u32;
    near.len()
}

/// The scene's terrains, for villagers' feet; nowhere off them.
struct TerrainGround<'a> {
    terrains: Vec<(&'a somnium_renderer::terrain::TerrainData, Vec3)>,
}

impl Ground for TerrainGround<'_> {
    fn height(&self, x: f32, z: f32) -> Option<f32> {
        self.terrains.iter().find_map(|(t, origin)| {
            let [w, d] = t.desc.world_size();
            let (lx, lz) = (x - origin.x, z - origin.z);
            (lx >= 0.0 && lz >= 0.0 && lx <= w && lz <= d)
                .then(|| origin.y + t.world_height_at(lx, lz))
        })
    }
}

fn declares_life(world: &World) -> bool {
    world
        .iter_with::<LifeTableComponent>()
        .any(|(_, t)| t.enabled)
        || world
            .iter_with::<LifePlaceComponent>()
            .any(|(_, p)| p.enabled)
        || world
            .iter_with::<VillagerComponent>()
            .any(|(_, v)| v.enabled)
        || world
            .iter_with::<LivestockComponent>()
            .any(|(_, b)| b.enabled)
        || world.iter_with::<CartComponent>().any(|(_, c)| c.enabled)
}

fn name_of(world: &World, e: Entity, or: &str) -> String {
    world
        .get::<Name>(e)
        .map_or(or.to_string(), |n| n.as_str().to_string())
}

/// Ring the alarm where a combatant lost health since the last look; returns
/// how many rang.
fn hear_fights(world: &World, run: &mut Run) -> usize {
    let mut rang = 0;
    let mut seen = HashMap::new();
    for (e, c) in world.iter_with::<CombatantComponent>() {
        if run
            .health
            .get(&e)
            .is_some_and(|before| c.health < *before - 0.5)
        {
            let at = world_matrix(world, e).w_axis.truncate();
            run.life.alarm(at, FIGHT_REACH, BELL_SECONDS);
            rang += 1;
        }
        seen.insert(e, c.health);
    }
    run.health = seen;
    rang
}

/// Adopt the places and hand-placed agents that appeared since the last look,
/// and let go of the entities that went.
fn sync(world: &World, run: &mut Run) {
    let links = &mut run.links;
    let life = &mut run.life;
    links.seen = world.entity_count();
    links.places.retain(|e, _| world.is_alive(*e));
    links.agents.retain(|e, _| world.is_alive(*e));
    run.authored.retain(|e, _| world.is_alive(*e));
    run.readouts.retain(|e, _| world.is_alive(*e));
    let new_places: Vec<(Entity, LifePlaceComponent)> = world
        .iter_with::<LifePlaceComponent>()
        .filter(|(e, p)| p.enabled && !links.places.contains_key(e))
        .map(|(e, p)| (e, p.clone()))
        .collect();
    if !new_places.is_empty() {
        let spots: Vec<(Entity, Spot)> = world
            .iter_with::<LifeSpotComponent>()
            .map(|(e, s)| {
                let (scale, _, position) = world_matrix(world, e).to_scale_rotation_translation();
                let kind = s.kind();
                let radius = if kind.indoor() {
                    0.5
                } else {
                    (scale.x.abs().min(scale.z.abs()) * 0.5).max(1.0)
                };
                (
                    e,
                    Spot {
                        kind,
                        position,
                        radius,
                        label: name_of(world, e, ""),
                    },
                )
            })
            .collect();
        let splines: Vec<(Entity, Vec<Vec3>)> = world
            .iter_with::<SplineComponent>()
            .map(|(e, s)| {
                let m = world_matrix(world, e);
                let mut points = Vec::new();
                for seg in 0..s.segment_count() {
                    for k in 0..SAMPLES_PER_SEGMENT {
                        points.push(m.transform_point3(
                            s.sample_segment(seg, k as f32 / SAMPLES_PER_SEGMENT as f32),
                        ));
                    }
                }
                if let Some(last) = s.points.last() {
                    points.push(m.transform_point3(*last));
                }
                (e, points)
            })
            .filter(|(_, p)| p.len() >= 2)
            .collect();
        for (e, p) in &new_places {
            let mine: Vec<Spot> = spots
                .iter()
                .filter(|(s, _)| under(world, *s, *e))
                .map(|(_, s)| s.clone())
                .collect();
            let lanes: Vec<Vec<Vec3>> = splines
                .iter()
                .filter(|(s, _)| under(world, *s, *e))
                .map(|(_, l)| l.clone())
                .collect();
            let name = name_of(world, *e, "Place");
            let key = Place::key(&name, world_matrix(world, *e).w_axis.truncate());
            let place = Place {
                name,
                spots: mine,
                roads: lanes.clone(),
                lanes,
            };
            let (index, _) = life.adopt_place(key, place, p.populate.then_some(u64::from(p.seed)));
            links.places.insert(*e, index);
        }
    }
    // Hand-placed agents keep to the nearest place.
    let nearest_place = |life: &LifeWorld, at: Vec3| -> Option<usize> {
        (0..life.place_count()).min_by(|a, b| {
            let d = |i: usize| {
                life.place(i).map_or(f32::INFINITY, |p| {
                    p.spots
                        .iter()
                        .map(|s| s.position.distance(at))
                        .fold(f32::INFINITY, f32::min)
                })
            };
            d(*a).total_cmp(&d(*b))
        })
    };
    let mut link = |links: &mut Links, e: Entity, key: u64, which: Option<Visual>| {
        if let Some(which) = which {
            links.keys.insert(key, which);
            links.agents.insert(e, which);
            if let Some(t) = world.get::<Transform>(e) {
                run.authored.insert(e, *t);
            }
        }
    };
    let villagers: Vec<(Entity, VillagerComponent)> = world
        .iter_with::<VillagerComponent>()
        .filter(|(e, v)| v.enabled && !links.agents.contains_key(e))
        .map(|(e, v)| (e, v.clone()))
        .collect();
    for (e, v) in &villagers {
        let at = world_matrix(world, *e).w_axis.truncate();
        let name = name_of(world, *e, "Villager");
        let key = Place::key(&format!("villager {name}"), at);
        let which = links.keys.get(&key).copied().or_else(|| {
            let pi = nearest_place(life, at)?;
            let p = life.place(pi).expect("exists").clone();
            let near = |kind: SpotKind| {
                p.spots_of(kind).min_by(|a, b| {
                    p.spots[*a]
                        .position
                        .distance(at)
                        .total_cmp(&p.spots[*b].position.distance(at))
                })
            };
            let home = near(SpotKind::Home)?;
            let role = v.role();
            let work = role.workplace().and_then(near);
            life.spawn_villager(VillagerDesc {
                name,
                role,
                place: pi,
                home,
                work,
            })?;
            Some(Visual::Villager(life.villagers().len() - 1))
        });
        link(links, *e, key, which);
    }
    let beasts: Vec<(Entity, LivestockComponent)> = world
        .iter_with::<LivestockComponent>()
        .filter(|(e, b)| b.enabled && !links.agents.contains_key(e))
        .map(|(e, b)| (e, b.clone()))
        .collect();
    for (e, b) in &beasts {
        let at = world_matrix(world, *e).w_axis.truncate();
        let key = Place::key(&format!("beast {}", name_of(world, *e, "")), at);
        let which = links.keys.get(&key).copied().or_else(|| {
            let pi = nearest_place(life, at)?;
            let p = life.place(pi).expect("exists");
            // A dog keeps to the nearest home; livestock to a pen or field.
            let dog = b.kind() == BeastKind::Dog;
            let pen = p
                .spots
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    if dog {
                        s.kind == SpotKind::Home
                    } else {
                        matches!(s.kind, SpotKind::Pen | SpotKind::Field)
                    }
                })
                .min_by(|a, b| {
                    a.1.position
                        .distance(at)
                        .total_cmp(&b.1.position.distance(at))
                })
                .map(|(i, _)| i)?;
            life.spawn_beast(b.kind(), pi, pen)?;
            Some(Visual::Beast(life.beasts().len() - 1))
        });
        link(links, *e, key, which);
    }
    let carts: Vec<Entity> = world
        .iter_with::<CartComponent>()
        .filter(|(e, c)| c.enabled && !links.agents.contains_key(e))
        .map(|(e, _)| e)
        .collect();
    for e in &carts {
        let at = world_matrix(world, *e).w_axis.truncate();
        let key = Place::key(&format!("cart {}", name_of(world, *e, "")), at);
        let which = links.keys.get(&key).copied().or_else(|| {
            let pi = nearest_place(life, at)?;
            let p = life.place(pi).expect("exists");
            let road = (0..p.roads.len()).min_by(|a, b| {
                let d = |r: usize| {
                    p.roads[r]
                        .iter()
                        .map(|q| q.distance(at))
                        .fold(f32::INFINITY, f32::min)
                };
                d(*a).total_cmp(&d(*b))
            })?;
            life.spawn_cart(pi, road, 0.0)?;
            Some(Visual::Cart(life.carts().len() - 1))
        });
        link(links, *e, key, which);
    }
    // Hand-placed visuals keep their own scale; the pools draw everyone else.
    run.visuals = links
        .agents
        .iter()
        .map(|(e, w)| (*e, *w, run.authored.get(e).map_or(Vec3::ONE, |t| t.scale)))
        .collect();
    let own = |pick: fn(&Visual) -> Option<usize>| -> HashSet<usize> {
        links.agents.values().filter_map(pick).collect()
    };
    run.villagers.own = own(|v| match v {
        Visual::Villager(i) => Some(*i),
        _ => None,
    });
    run.beasts.own = own(|v| match v {
        Visual::Beast(i) => Some(*i),
        _ => None,
    });
    run.carts.own = own(|v| match v {
        Visual::Cart(i) => Some(*i),
        _ => None,
    });
}

impl LifeDemo {
    /// Build the world from the scene's places, spots, lanes and hand-placed
    /// agents; a pool of bodies for everyone the places populated; colliders
    /// for the solid blocks.
    pub fn begin(&mut self, ctx: &mut EngineContext) {
        let Some(mut run) = Self::start(ctx.world, ctx.config.project_root.as_deref()) else {
            return;
        };
        run.colliders = colliders(ctx);
        self.run = Some(run);
    }

    /// Everything but the colliders: the pools of bodies, then the scene's
    /// places and agents as they stand.
    fn start(world: &mut World, root: Option<&std::path::Path>) -> Option<Run> {
        if !declares_life(world) {
            return None;
        }
        somnium_core::propagate_transforms(world);
        let mut spawned = Vec::new();
        let effects = world.spawn((
            Name::new("Life (runtime)"),
            Transform::default(),
            WorldTransform::identity(),
        ));
        spawned.push(effects);
        let villagers = Pool::new(
            world,
            &mut spawned,
            effects,
            (VILLAGER_BODIES, "Villager", MeshKind::Cylinder, true),
            HashSet::new(),
        );
        let beasts = Pool::new(
            world,
            &mut spawned,
            effects,
            (BEAST_BODIES, "Beast", MeshKind::Cube, false),
            HashSet::new(),
        );
        let mut carts = Pool::new(
            world,
            &mut spawned,
            effects,
            (CART_BODIES, "Cart", MeshKind::Cube, false),
            HashSet::new(),
        );
        for (body, _) in &carts.slots {
            dress(world, *body, "Cart", Vec3::new(1.4, 1.1, 2.6), "hide");
        }
        carts.bodies.keep = 10.0;
        let tod = world
            .iter_with::<TimeOfDayComponent>()
            .find(|(_, t)| t.enabled)
            .map(|(_, t)| (t.hour, t.eclipse));
        let mut run = Run {
            life: LifeWorld::new(LodConfig::default()),
            visuals: Vec::new(),
            villagers,
            beasts,
            carts,
            looks: HashMap::new(),
            authored: HashMap::new(),
            readouts: HashMap::new(),
            links: Links::default(),
            synced: 0.0,
            greeted: 0,
            health: HashMap::new(),
            fights: 0,
            table: None,
            spawned,
            colliders: Vec::new(),
            // `SOMNIUM_LIFE_HOUR` starts the day at another hour (captures).
            hour: std::env::var("SOMNIUM_LIFE_HOUR")
                .ok()
                .and_then(|h| h.parse().ok())
                .or(tod.map(|t| t.0))
                .unwrap_or(6.5),
            eclipse: tod.map_or(0.0, |t| t.1),
            step_ms: 0.0,
            write_ms: 0.0,
            frames: 0,
            reported: 0.0,
        };
        sync(world, &mut run);
        run.table = load_table(world, root);
        // Look round for the table's towns on the first frame.
        run.synced = 1.0;
        let life = &run.life;
        info!(
            villagers = life.villagers().len(),
            beasts = life.beasts().len(),
            carts = life.carts().len(),
            "life: {} agents in {} places",
            life.villagers().len() + life.beasts().len() + life.carts().len(),
            life.place_count()
        );
        Some(run)
    }

    /// Put everything back as authored.
    /// B rings the alarm bell while life runs.
    pub fn event(&mut self, event: &EngineEvent) {
        if self.run.is_some()
            && matches!(
                event,
                EngineEvent::KeyInput {
                    key: KeyCode::KeyB,
                    state: InputState::Pressed
                }
            )
        {
            self.bell = true;
        }
    }

    pub fn end(&mut self, ctx: &mut EngineContext) {
        self.idle = 0.0;
        self.bell = false;
        let Some(run) = self.run.take() else { return };
        for (e, t) in run.authored {
            if let Some(slot) = ctx.world.get_mut::<Transform>(e) {
                *slot = t;
            }
        }
        for e in run.spawned {
            ctx.world.despawn(e);
        }
        for body in run.colliders {
            ctx.physics.destroy_body(body);
        }
    }

    /// One frame of life: the clock, the step with the camera as focus, then
    /// the visuals that changed.
    pub fn update(&mut self, ctx: &mut EngineContext) {
        let dt = ctx.dt();
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        if self.run.is_none() {
            // Life may stream in after Play began (cells load around the view).
            self.idle += dt;
            if self.idle >= 1.0 {
                self.idle = 0.0;
                self.begin(ctx);
            }
            return;
        }
        let ring = std::mem::take(&mut self.bell);
        let run = self.run.as_mut().expect("running");
        // Places and hand-placed agents stream in and out with their cells.
        run.synced += dt;
        if run.synced >= 1.0 || (run.synced >= 0.25 && ctx.world.entity_count() != run.links.seen) {
            run.synced = 0.0;
            sync(ctx.world, run);
            run.fights += hear_fights(ctx.world, run);
            if let Some(at) = ctx.renderer.as_ref().map(|r| r.camera_pos)
                && adopt_near(run, at) > 0
                && let Some(t) = &run.table
            {
                info!(
                    adopted = t.count,
                    "life: towns housed as the view came near"
                );
                if let Some(c) = ctx.world.get_mut::<LifeTableComponent>(t.entity) {
                    c.adopted = t.count;
                }
            }
        }
        let tod = ctx
            .world
            .iter_with::<TimeOfDayComponent>()
            .find(|(_, t)| t.enabled)
            .map(|(_, t)| (t.hour, t.timescale, t.eclipse));
        match tod {
            Some((hour, scale, _)) if scale > 0.0 => run.hour = hour,
            _ => run.hour = (run.hour + dt / SECONDS_PER_HOUR).rem_euclid(24.0),
        }
        run.eclipse = tod.map_or(0.0, |t| t.2);
        let focus: Vec<Vec3> = ctx
            .renderer
            .as_ref()
            .map(|r| r.camera_pos)
            .into_iter()
            .collect();
        if ring && let Some(at) = focus.first() {
            let frightened = run.life.alarm(*at, BELL_REACH, BELL_SECONDS);
            info!(frightened, "life: the alarm bell");
        }
        let terrains: Vec<(u32, Vec3)> = ctx
            .world
            .iter_with::<somnium_core::TerrainComponent>()
            .map(|(e, t)| {
                let origin = ctx
                    .world
                    .get::<WorldTransform>(e)
                    .map_or(Vec3::ZERO, |w| w.0.w_axis.truncate());
                (t.terrain_id, origin)
            })
            .collect();
        let ground = TerrainGround {
            terrains: terrains
                .iter()
                .filter_map(|(id, origin)| {
                    ctx.renderer.as_ref()?.terrain(*id).map(|t| (t, *origin))
                })
                .collect(),
        };
        let t0 = Instant::now();
        run.life.step(
            dt,
            Clock {
                hour: run.hour,
                eclipse: run.eclipse,
            },
            &focus,
            &ground,
        );
        if let Some(at) = focus.first() {
            run.life.keep_clear(*at, 0.7);
            run.greeted += run.life.notice(*at, 3.5).len();
        }
        let t1 = Instant::now();
        write_visuals(ctx.world, run, focus.first().copied().unwrap_or_default());
        let t2 = Instant::now();
        let ms = |a: Instant, b: Instant| (b - a).as_secs_f64() * 1000.0;
        run.frames += 1;
        let k = 1.0 / f64::from(run.frames.min(120));
        run.step_ms += (ms(t0, t1) - run.step_ms) * k;
        run.write_ms += (ms(t1, t2) - run.write_ms) * k;
        run.reported += dt;
        if run.reported >= 10.0 {
            run.reported = 0.0;
            let s = run.life.stats();
            info!(
                hour = run.hour,
                near = s.near,
                mid = s.mid,
                far = s.far,
                stepped = s.stepped,
                step_ms = run.step_ms,
                write_ms = run.write_ms,
                frame_ms = f64::from(dt) * 1000.0,
                "life frame"
            );
        }
    }

    /// For MCP and automation.
    pub fn state(&self) -> Value {
        let Some(run) = &self.run else {
            return json!({"running": false});
        };
        let s = run.life.stats();
        let out = run.life.villagers().iter().filter(|v| v.visible).count();
        json!({
            "running": true,
            "hour": run.hour,
            "eclipse": run.eclipse,
            "places": run.life.place_count(),
            "places_loaded": run.links.places.len(),
            "greeted": run.greeted,
            "table": run.table.as_ref().map(|t| json!({"places": t.table.places.len(), "adopted": t.count})),
            "fights": run.fights,
            "villagers": run.life.villagers().len(),
            "outdoors": out,
            "beasts": run.life.beasts().len(),
            "carts": run.life.carts().len(),
            "lod": {"near": s.near, "mid": s.mid, "far": s.far, "stepped": s.stepped, "deferred": s.deferred},
            "step_ms": run.step_ms,
            "write_ms": run.write_ms,
            "bodies": {
                "villagers": [run.villagers.bodies.held(), VILLAGER_BODIES],
                "beasts": [run.beasts.bodies.held(), BEAST_BODIES],
                "carts": [run.carts.bodies.held(), CART_BODIES],
            },
        })
    }
}

/// Hand the pooled bodies to the nearest agents out of doors, then write each
/// visual's pose when it changed; the far, the indoor and empty bodies are
/// parked.
fn write_visuals(world: &mut World, run: &mut Run, focus: Vec3) {
    let life = &run.life;
    let near = |p: Vec3| p.distance(focus);
    let changed = run.villagers.assign(
        life.villagers()
            .iter()
            .enumerate()
            .filter(|(i, v)| v.visible && v.tier != Tier::Far && !run.villagers.own.contains(i))
            .map(|(i, v)| (i, near(v.position)))
            .collect(),
    );
    for (slot, holder) in changed {
        let (body, head) = run.villagers.slots[slot];
        run.looks.remove(&body);
        if let Some(i) = holder {
            let v = &life.villagers()[i];
            let size = villager_size(v.role);
            dress(world, body, &v.name, size, role_material(v.role));
            if let Some(t) = head.and_then(|h| world.get_mut::<Transform>(h)) {
                t.scale = Vec3::splat(0.3) / size;
            }
        }
    }
    let changed = run.beasts.assign(
        life.beasts()
            .iter()
            .enumerate()
            .filter(|(i, b)| b.tier != Tier::Far && !run.beasts.own.contains(i))
            .map(|(i, b)| (i, near(b.position)))
            .collect(),
    );
    for (slot, holder) in changed {
        let body = run.beasts.slots[slot].0;
        run.looks.remove(&body);
        if let Some(i) = holder {
            let kind = life.beasts()[i].kind;
            let (size, look) = beast_look(kind);
            dress(world, body, BeastKind::NAMES[kind as usize], size, look);
        }
    }
    let changed = run.carts.assign(
        life.carts()
            .iter()
            .enumerate()
            .filter(|(i, c)| c.tier != Tier::Far && !run.carts.own.contains(i))
            .map(|(i, c)| (i, near(c.position)))
            .collect(),
    );
    for (slot, _) in changed {
        run.looks.remove(&run.carts.slots[slot].0);
    }
    let pooled = |pool: &Pool, kind: fn(usize) -> Visual, size: &dyn Fn(usize) -> Vec3| {
        pool.slots
            .iter()
            .enumerate()
            .map(|(slot, (e, _))| match pool.bodies.holder(slot) {
                Some(i) => (*e, Some(kind(i)), size(i)),
                None => (*e, None, Vec3::ONE),
            })
            .collect::<Vec<_>>()
    };
    let mut all: Vec<(Entity, Option<Visual>, Vec3)> = run
        .visuals
        .iter()
        .map(|(e, v, size)| (*e, Some(*v), *size))
        .collect();
    all.extend(pooled(&run.villagers, Visual::Villager, &|i| {
        villager_size(life.villagers()[i].role)
    }));
    all.extend(pooled(&run.beasts, Visual::Beast, &|i| {
        beast_look(life.beasts()[i].kind).0
    }));
    all.extend(pooled(&run.carts, Visual::Cart, &|_| {
        Vec3::new(1.4, 1.1, 2.6)
    }));
    for (entity, which, size) in &all {
        let Some(which) = which else {
            let look = Look(PARKED, Quat::IDENTITY);
            if run.looks.get(entity) != Some(&look) {
                run.looks.insert(*entity, look);
                if let Some(t) = world.get_mut::<Transform>(*entity) {
                    t.translation = PARKED;
                }
            }
            continue;
        };
        let (shown, position, facing) = match which {
            Visual::Villager(i) => {
                let v = &run.life.villagers()[*i];
                (v.visible && v.tier != Tier::Far, v.position, v.facing)
            }
            Visual::Beast(i) => {
                let b = &run.life.beasts()[*i];
                (b.tier != Tier::Far, b.position, b.facing)
            }
            Visual::Cart(i) => {
                let c = &run.life.carts()[*i];
                (c.tier != Tier::Far, c.position, c.facing)
            }
        };
        let look = if shown {
            Look(position + Vec3::Y * size.y * 0.5, yaw(facing))
        } else {
            Look(PARKED, Quat::IDENTITY)
        };
        if run.looks.get(entity) == Some(&look) {
            continue;
        }
        run.looks.insert(*entity, look);
        let parent = world
            .get::<Parent>(*entity)
            .map(|p| p.entity)
            .filter(|p| *p != Entity::DANGLING)
            .map_or(Mat4::IDENTITY, |p| world_matrix(world, p));
        let local = parent.inverse() * Mat4::from_rotation_translation(look.1, look.0);
        let (_, r, p) = local.to_scale_rotation_translation();
        if let Some(t) = world.get_mut::<Transform>(*entity) {
            t.translation = p;
            t.rotation = r;
        }
    }
    for (entity, which) in &run.links.agents {
        let Visual::Villager(index) = which else {
            continue;
        };
        let v = &run.life.villagers()[*index];
        let doing = format!("{:?} ({:?})", v.activity, v.pose);
        let last = run.readouts.entry(*entity).or_default();
        if *last != doing {
            last.clone_from(&doing);
            if let Some(c) = world.get_mut::<VillagerComponent>(*entity) {
                c.doing = doing;
            }
        }
    }
    let population = run.life.villagers().len() + run.life.beasts().len() + run.life.carts().len();
    for e in run.links.places.keys() {
        if let Some(p) = world.get_mut::<LifePlaceComponent>(*e)
            && p.population != population as u32
        {
            p.population = population as u32;
        }
    }
}

/// Static Jolt boxes for the solid blocks (the ground, the buildings), so the
/// walker stands and cannot pass through walls.
fn colliders(ctx: &mut EngineContext) -> Vec<BodyId> {
    let solids: Vec<Mat4> = ctx
        .world
        .iter_with::<CombatSurfaceComponent>()
        .filter(|(_, s)| s.solid)
        .filter(|(e, _)| ctx.world.get::<Name>(*e).is_some())
        .map(|(e, _)| world_matrix(ctx.world, e))
        .collect();
    solids
        .into_iter()
        .map(|m| {
            let (scale, rotation, translation) = m.to_scale_rotation_translation();
            ctx.physics.create_body(RigidBodyDescriptor {
                shape: ColliderShape::Box {
                    half_extents: (scale.abs() * 0.5).max(Vec3::splat(0.01)),
                },
                position: translation,
                rotation,
                motion_type: MotionType::Static,
                object_layer: somnium_physics::layer::LAYER_NON_MOVING,
                ..RigidBodyDescriptor::default()
            })
        })
        .filter(|b| b.is_valid())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_life::Flat;

    /// The village gathered as Play gathers it keeps its day: by late morning
    /// the farmers are in the fields and most are out; under the Lid all are in.
    #[test]
    fn the_village_keeps_its_day_and_goes_in_under_the_lid() {
        let mut world = World::new();
        spawn_village(&mut world, &json!({"homes": 20})).expect("builds");
        let Run {
            mut life, links, ..
        } = LifeDemo::start(&mut world, None).expect("a place");
        assert_eq!(links.places.len(), 1);
        assert!(life.villagers().len() >= 20, "a household a home");
        assert!(!life.beasts().is_empty() && !life.carts().is_empty());
        let mut hour = 6.0;
        while hour < 11.0 {
            life.step(0.5, Clock::at(hour), &[], &Flat);
            hour += 0.5 / SECONDS_PER_HOUR;
        }
        let place = life.place(0).unwrap().clone();
        let out = life.villagers().iter().filter(|v| v.visible).count();
        assert!(out * 2 > life.villagers().len(), "out and about: {out}");
        for v in life.villagers().iter().filter(|v| v.role == Role::Farmer) {
            let field = &place.spots[v.work.expect("a field")];
            assert!(
                v.position.distance(field.position) <= field.radius + 0.5,
                "{} in the field",
                v.name
            );
        }
        for _ in 0..400 {
            life.step(
                0.5,
                Clock {
                    hour: 11.0,
                    eclipse: 1.0,
                },
                &[],
                &Flat,
            );
        }
        assert!(
            life.villagers().iter().all(|v| !v.visible),
            "all in under the Lid"
        );
    }

    /// A town of any size spawns the same pool of bodies, and they go to the
    /// nearest out of doors.
    #[test]
    fn a_big_village_draws_only_its_pooled_bodies_nearest_first() {
        let mut world = World::new();
        spawn_village(&mut world, &json!({"homes": 300})).unwrap();
        let before = world.entity_count();
        let mut run = LifeDemo::start(&mut world, None).unwrap();
        let villagers = run.life.villagers().len();
        assert!(villagers > 2 * VILLAGER_BODIES, "{villagers}");
        // One entity per body (and head), not per agent.
        let pooled = VILLAGER_BODIES * 2 + BEAST_BODIES + CART_BODIES + 1;
        assert_eq!(world.entity_count(), before + pooled);
        let focus = Vec3::ZERO;
        let clock = Clock {
            hour: 11.0,
            eclipse: 0.0,
        };
        for _ in 0..600 {
            run.life.step(0.5, clock, &[focus], &Flat);
        }
        write_visuals(&mut world, &mut run, focus);
        let out: Vec<f32> = run
            .life
            .villagers()
            .iter()
            .filter(|v| v.visible && v.tier != Tier::Far)
            .map(|v| v.position.distance(focus))
            .collect();
        assert!(
            out.len() > VILLAGER_BODIES,
            "more out than bodies: {}",
            out.len()
        );
        let held = run.villagers.bodies.held();
        assert_eq!(held, out.len().min(VILLAGER_BODIES));
        // Every drawn villager is nearer than every outdoor one left undrawn.
        let drawn: Vec<usize> = (0..VILLAGER_BODIES)
            .filter_map(|s| run.villagers.bodies.holder(s))
            .collect();
        let far_drawn = drawn
            .iter()
            .map(|i| run.life.villagers()[*i].position.distance(focus))
            .fold(0.0, f32::max);
        let near_left = run
            .life
            .villagers()
            .iter()
            .enumerate()
            .filter(|(i, v)| v.visible && v.tier != Tier::Far && !drawn.contains(i))
            .map(|(_, v)| v.position.distance(focus))
            .fold(f32::INFINITY, f32::min);
        assert!(far_drawn <= near_left, "{far_drawn} > {near_left}");
        // A drawn body stands where its villager is; an empty one is parked.
        for slot in 0..VILLAGER_BODIES {
            let (body, _) = run.villagers.slots[slot];
            let at = world.get::<Transform>(body).unwrap().translation;
            match run.villagers.bodies.holder(slot) {
                Some(i) => {
                    let v = &run.life.villagers()[i];
                    assert!((at.x - v.position.x).abs() + (at.z - v.position.z).abs() < 1e-3);
                }
                None => assert_eq!(at, PARKED),
            }
        }
    }

    /// A village that streams out keeps living, and streams back with the same
    /// people: nobody housed twice, a hand-placed villager finds itself again.
    #[test]
    fn a_village_that_streams_out_and_back_keeps_its_people() {
        let mut world = World::new();
        let village = spawn_village(&mut world, &json!({"homes": 20})).unwrap();
        let smith = world.spawn((
            Name::new("Old Hal"),
            Transform::from_translation(Vec3::new(3.0, 0.0, 3.0)),
            WorldTransform::identity(),
            VillagerComponent {
                role: Role::ALL.iter().position(|r| *r == Role::Smith).unwrap() as u32,
                ..Default::default()
            },
        ));
        let mut run = LifeDemo::start(&mut world, None).unwrap();
        let people = run.life.villagers().len();
        assert_eq!(run.links.agents.len(), 1, "the hand-placed smith");
        // The cell unloads: every entity it held goes.
        for e in village.iter().copied().chain([smith]) {
            world.despawn(e);
        }
        sync(&world, &mut run);
        assert!(run.links.places.is_empty() && run.links.agents.is_empty());
        assert!(
            run.villagers.own.is_empty(),
            "the smith is the pool's to draw now"
        );
        for _ in 0..100 {
            run.life.step(0.5, Clock::at(9.0), &[], &Flat);
        }
        // It streams back: the same place and the same smith, nobody new.
        let back = spawn_village(&mut world, &json!({"homes": 20})).unwrap();
        let smith = world.spawn((
            Name::new("Old Hal"),
            Transform::from_translation(Vec3::new(3.0, 0.0, 3.0)),
            WorldTransform::identity(),
            VillagerComponent {
                role: Role::ALL.iter().position(|r| *r == Role::Smith).unwrap() as u32,
                ..Default::default()
            },
        ));
        sync(&world, &mut run);
        assert_eq!(run.life.villagers().len(), people);
        assert_eq!(run.life.place_count(), 1);
        assert_eq!(run.links.places.len(), 1);
        assert!(back.iter().any(|e| run.links.places.contains_key(e)));
        assert!(matches!(
            run.links.agents.get(&smith),
            Some(Visual::Villager(_))
        ));
        assert_eq!(run.villagers.own.len(), 1);
    }

    /// The worst tick of a whole day in the biggest preset town, with the
    /// camera in its middle (release: `cargo test -p hello_engine --release
    /// day_spikes -- --ignored --nocapture`).
    #[test]
    #[ignore = "timing; run in release"]
    fn day_spikes() {
        let day = |paths: usize| {
            let mut world = World::new();
            spawn_village(&mut world, &json!({"homes": 400})).unwrap();
            let mut run = LifeDemo::start(&mut world, None).unwrap();
            run.life.paths_per_tick = paths;
            let dt = 1.0 / 60.0;
            let mut hour = 0.0_f32;
            let mut ticks = Vec::new();
            while hour < 24.0 {
                let t = Instant::now();
                run.life.step(dt, Clock::at(hour), &[Vec3::ZERO], &Flat);
                ticks.push((t.elapsed().as_secs_f64() * 1000.0, hour));
                hour += dt / 6.0;
            }
            let mean = ticks.iter().map(|t| t.0).sum::<f64>() / ticks.len() as f64;
            ticks.sort_by(|a, b| a.0.total_cmp(&b.0));
            let at = |q: f64| ticks[((ticks.len() - 1) as f64 * q) as usize].0;
            let over = ticks.iter().filter(|t| t.0 > 1.0).count();
            let worst = ticks.last().unwrap();
            eprintln!(
                "paths a tick {paths}: {} ticks, mean {mean:.4} ms, p99 {:.3}, p99.9 {:.3}, worst {:.3} at {:.2} h, {over} over 1 ms",
                ticks.len(),
                at(0.99),
                at(0.999),
                worst.0,
                worst.1
            );
        };
        for _ in 0..2 {
            day(usize::MAX);
            day(24);
        }
    }

    /// A combatant hurt in the village sends the villagers near it home.
    #[test]
    fn a_fight_in_the_village_rings_the_alarm() {
        let mut world = World::new();
        spawn_village(&mut world, &json!({"homes": 20})).unwrap();
        let fighter = world.spawn((
            Name::new("Brawler"),
            Transform::from_translation(Vec3::new(0.0, 0.0, 0.0)),
            WorldTransform::identity(),
            CombatantComponent::default(),
        ));
        let mut run = LifeDemo::start(&mut world, None).unwrap();
        for _ in 0..1200 {
            run.life.step(0.5, Clock::at(11.0), &[Vec3::ZERO], &Flat);
        }
        assert_eq!(
            hear_fights(&world, &mut run),
            0,
            "first look: nothing to compare"
        );
        assert_eq!(hear_fights(&world, &mut run), 0, "no harm, no alarm");
        // Those in earshot when the blow lands run home and stay in.
        let earshot: Vec<_> = run
            .life
            .villagers()
            .iter()
            .filter(|v| v.position.distance(Vec3::ZERO) <= FIGHT_REACH)
            .map(|v| v.id)
            .collect();
        assert!(!earshot.is_empty());
        world.get_mut::<CombatantComponent>(fighter).unwrap().health -= 10.0;
        assert_eq!(hear_fights(&world, &mut run), 1);
        for _ in 0..80 {
            run.life.step(0.5, Clock::at(11.0), &[Vec3::ZERO], &Flat);
        }
        let out: Vec<_> = run
            .life
            .villagers()
            .iter()
            .filter(|v| earshot.contains(&v.id) && v.visible)
            .map(|v| (v.name.clone(), v.position, v.activity))
            .collect();
        assert!(out.is_empty(), "still out after the fight: {out:?}");
    }

    /// A scene's Life Table is read from the project, and its towns are
    /// housed only as the view comes within reach, once each.
    #[test]
    fn a_life_table_houses_towns_as_the_view_comes_near() {
        let town = |name: &str, x: f32| somnium_life::TablePlace {
            centre: [x, 0.0, 0.0],
            radius: 30.0,
            seed: 3,
            place: Place {
                name: name.into(),
                spots: (0..5)
                    .map(|i| Spot::new(SpotKind::Home, Vec3::new(x + i as f32 * 8.0, 0.0, 0.0)))
                    .collect(),
                lanes: vec![vec![
                    Vec3::new(x - 20.0, 0.0, 4.0),
                    Vec3::new(x + 50.0, 0.0, 4.0),
                ]],
                roads: Vec::new(),
            },
        };
        let table = LifeTable {
            version: 1,
            places: vec![town("Here", 0.0), town("Yonder", 3_000.0)],
        };
        let root = std::env::temp_dir().join(format!("life_table_{}", std::process::id()));
        std::fs::create_dir_all(root.join("data")).unwrap();
        std::fs::write(
            root.join("data/realm.json"),
            serde_json::to_string(&table).unwrap(),
        )
        .unwrap();
        let mut world = World::new();
        world.spawn((
            Name::new("Realm"),
            LifeTableComponent {
                file: "data/realm.json".into(),
                ..Default::default()
            },
        ));
        let mut run = LifeDemo::start(&mut world, Some(&root)).expect("a table is life");
        assert_eq!(run.life.place_count(), 0, "nobody before the view comes");
        assert_eq!(adopt_near(&mut run, Vec3::ZERO), 1);
        assert_eq!(adopt_near(&mut run, Vec3::ZERO), 0, "once");
        assert_eq!(run.life.place_count(), 1);
        assert!(run.life.villagers().len() >= 5);
        assert_eq!(
            adopt_near(&mut run, Vec3::new(2_500.0, 0.0, 0.0)),
            1,
            "Yonder, on the way there"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// The preset gives a place with homes for every house, the trades, the
    /// green, fields and a pen; its lanes are splines.
    #[test]
    fn the_village_preset_has_its_spots_and_lanes() {
        let mut world = World::new();
        spawn_village(&mut world, &json!({"homes": 30})).expect("builds");
        let kinds: Vec<SpotKind> = world
            .iter_with::<LifeSpotComponent>()
            .map(|(_, s)| s.kind())
            .collect();
        let count = |k: SpotKind| kinds.iter().filter(|x| **x == k).count();
        assert_eq!(count(SpotKind::Home), 30);
        assert_eq!(count(SpotKind::Inn), 1);
        assert_eq!(count(SpotKind::Chapel), 1);
        assert_eq!(count(SpotKind::Work), 2);
        assert_eq!(count(SpotKind::Field), 2);
        assert_eq!(count(SpotKind::Pen), 1);
        assert!(
            world.iter_with::<SplineComponent>().count() >= 3,
            "streets and the cross lane"
        );
        assert_eq!(world.iter_with::<LifePlaceComponent>().count(), 1);
    }
}
