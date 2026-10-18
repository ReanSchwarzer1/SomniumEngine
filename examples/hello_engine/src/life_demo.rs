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
//! not drawn. The step and the transform writes are timed and reported to MCP
//! (`life` in the authoring state) and the log every ten seconds.
use glam::{Mat4, Quat, Vec3};
use serde_json::{Value, json};
use somnium_asset::database::AssetId;
use somnium_combat::components::CombatSurfaceComponent;
use somnium_combat::environment::Surface;
use somnium_core::spline::{SAMPLES_PER_SEGMENT, SplineComponent};
use somnium_core::time_of_day::TimeOfDayComponent;
use somnium_core::{
    EngineContext, MaterialComponent, MeshKind, Name, Parent, Transform, WorldTransform,
};
use somnium_ecs::{Entity, World};
use somnium_life::components::{
    CartComponent, LifePlaceComponent, LifeSpotComponent, LivestockComponent, VillagerComponent,
};
use somnium_life::{
    BeastKind, Clock, Flat, LifeWorld, LodConfig, Place, Role, Spot, SpotKind, Tier, VillagerDesc,
};
use somnium_physics::body::{BodyId, MotionType, RigidBodyDescriptor};
use somnium_physics::shape::ColliderShape;
use std::collections::HashMap;
use std::time::Instant;
use tracing::info;

/// Real seconds to a village hour when no Time of Day moves the clock.
const SECONDS_PER_HOUR: f32 = 60.0;
/// Where hidden visuals wait.
const PARKED: Vec3 = Vec3::new(0.0, -10_000.0, 0.0);

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
enum Visual {
    Villager(usize),
    Beast(usize),
    Cart(usize),
}

struct Run {
    life: LifeWorld,
    /// Each agent's entity: pooled for the populated, the authored entity itself otherwise.
    visuals: Vec<(Entity, Visual, Vec3)>,
    looks: HashMap<Entity, Look>,
    authored: Vec<(Entity, Transform)>,
    villager_readouts: Vec<(Entity, usize, String)>,
    places: Vec<Entity>,
    spawned: Vec<Entity>,
    colliders: Vec<BodyId>,
    hour: f32,
    eclipse: f32,
    step_ms: f64,
    write_ms: f64,
    frames: u32,
    reported: f32,
}

#[derive(Default)]
pub struct LifeDemo {
    run: Option<Run>,
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
    }
}

/// What the scene declares, as a life world: its places (with their spots and
/// splines), and the hand-placed agents with the entities that show them.
struct Gathered {
    life: LifeWorld,
    places: Vec<Entity>,
    visuals: Vec<(Entity, Visual, Vec3)>,
    authored: Vec<(Entity, Transform)>,
    villager_readouts: Vec<(Entity, usize, String)>,
}

fn gather(world: &World) -> Option<Gathered> {
    let places: Vec<(Entity, LifePlaceComponent)> = world
        .iter_with::<LifePlaceComponent>()
        .filter(|(_, p)| p.enabled)
        .map(|(e, p)| (e, p.clone()))
        .collect();
    let authored_villagers: Vec<(Entity, VillagerComponent)> = world
        .iter_with::<VillagerComponent>()
        .filter(|(_, v)| v.enabled)
        .map(|(e, v)| (e, v.clone()))
        .collect();
    let authored_beasts: Vec<(Entity, LivestockComponent)> = world
        .iter_with::<LivestockComponent>()
        .filter(|(_, b)| b.enabled)
        .map(|(e, b)| (e, b.clone()))
        .collect();
    let authored_carts: Vec<Entity> = world
        .iter_with::<CartComponent>()
        .filter(|(_, c)| c.enabled)
        .map(|(e, _)| e)
        .collect();
    if places.is_empty()
        && authored_villagers.is_empty()
        && authored_beasts.is_empty()
        && authored_carts.is_empty()
    {
        return None;
    }
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
                    label: world
                        .get::<Name>(e)
                        .map_or(String::new(), |n| n.as_str().to_string()),
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
    let mut life = LifeWorld::new(LodConfig::default());
    let mut place_entities = Vec::new();
    for (e, p) in &places {
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
        let name = world
            .get::<Name>(*e)
            .map_or("Place".to_string(), |n| n.as_str().to_string());
        let index = life.add_place(Place {
            name,
            spots: mine,
            roads: lanes.clone(),
            lanes,
        });
        if p.populate {
            life.populate(index, u64::from(p.seed));
        }
        place_entities.push(*e);
    }
    // Hand-placed agents keep to the nearest place (made on the spot if none).
    let nearest_place = |life: &LifeWorld, at: Vec3| -> Option<usize> {
        (0..place_entities.len()).min_by(|a, b| {
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
    let mut visuals = Vec::new();
    let mut authored = Vec::new();
    let mut villager_readouts = Vec::new();
    for (e, v) in &authored_villagers {
        let at = world_matrix(world, *e).w_axis.truncate();
        let Some(pi) = nearest_place(&life, at) else {
            continue;
        };
        let p = life.place(pi).expect("exists").clone();
        let near = |kind: SpotKind| {
            p.spots_of(kind).min_by(|a, b| {
                p.spots[*a]
                    .position
                    .distance(at)
                    .total_cmp(&p.spots[*b].position.distance(at))
            })
        };
        let Some(home) = near(SpotKind::Home) else {
            continue;
        };
        let role = v.role();
        let work = role.workplace().and_then(near);
        let name = world
            .get::<Name>(*e)
            .map_or("Villager".into(), |n| n.as_str().to_string());
        if life
            .spawn_villager(VillagerDesc {
                name,
                role,
                place: pi,
                home,
                work,
            })
            .is_some()
        {
            let index = life.villagers().len() - 1;
            visuals.push((*e, Visual::Villager(index), Vec3::ONE));
            villager_readouts.push((*e, index, String::new()));
            if let Some(t) = world.get::<Transform>(*e) {
                authored.push((*e, *t));
            }
        }
    }
    for (e, b) in &authored_beasts {
        let at = world_matrix(world, *e).w_axis.truncate();
        let Some(pi) = nearest_place(&life, at) else {
            continue;
        };
        let p = life.place(pi).expect("exists");
        let pen = p
            .spots
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s.kind, SpotKind::Pen | SpotKind::Field))
            .min_by(|a, b| {
                a.1.position
                    .distance(at)
                    .total_cmp(&b.1.position.distance(at))
            })
            .map(|(i, _)| i);
        if let Some(pen) = pen
            && life.spawn_beast(b.kind(), pi, pen).is_some()
        {
            visuals.push((*e, Visual::Beast(life.beasts().len() - 1), Vec3::ONE));
            if let Some(t) = world.get::<Transform>(*e) {
                authored.push((*e, *t));
            }
        }
    }
    for e in &authored_carts {
        let at = world_matrix(world, *e).w_axis.truncate();
        let Some(pi) = nearest_place(&life, at) else {
            continue;
        };
        let p = life.place(pi).expect("exists");
        let road = (0..p.roads.len()).min_by(|a, b| {
            let d = |r: usize| {
                p.roads[r]
                    .iter()
                    .map(|q| q.distance(at))
                    .fold(f32::INFINITY, f32::min)
            };
            d(*a).total_cmp(&d(*b))
        });
        if let Some(road) = road
            && life.spawn_cart(pi, road, 0.0).is_some()
        {
            visuals.push((*e, Visual::Cart(life.carts().len() - 1), Vec3::ONE));
            if let Some(t) = world.get::<Transform>(*e) {
                authored.push((*e, *t));
            }
        }
    }
    Some(Gathered {
        life,
        places: place_entities,
        visuals,
        authored,
        villager_readouts,
    })
}

impl LifeDemo {
    /// Build the world from the scene's places, spots, lanes and hand-placed
    /// agents; pooled visuals for everyone the places populated; colliders for
    /// the solid blocks.
    pub fn begin(&mut self, ctx: &mut EngineContext) {
        somnium_core::propagate_transforms(ctx.world);
        let Some(Gathered {
            life,
            places: place_entities,
            mut visuals,
            authored,
            villager_readouts,
        }) = gather(ctx.world)
        else {
            return;
        };
        let world = &mut *ctx.world;
        let mut spawned = Vec::new();
        let effects = world.spawn((
            Name::new("Life (runtime)"),
            Transform::default(),
            WorldTransform::identity(),
        ));
        spawned.push(effects);
        let has_visual = |list: &Vec<(Entity, Visual, Vec3)>, which: &dyn Fn(&Visual) -> bool| {
            list.iter().any(|(_, v, _)| which(v))
        };
        for (i, v) in life.villagers().iter().enumerate() {
            if has_visual(&visuals, &|w| matches!(w, Visual::Villager(j) if *j == i)) {
                continue;
            }
            let size = if v.role == Role::Child {
                Vec3::new(0.38, 1.05, 0.38)
            } else {
                Vec3::new(0.5, 1.45, 0.5)
            };
            let body = world.spawn((
                Name::new(&v.name),
                Transform {
                    translation: PARKED,
                    scale: size,
                    ..Default::default()
                },
                WorldTransform::identity(),
                Parent { entity: effects },
                MeshKind::Cylinder,
                material(role_material(v.role)),
            ));
            let head = world.spawn((
                Name::new(&format!("{} head", v.name)),
                Transform {
                    translation: Vec3::new(0.0, 0.62, 0.0),
                    scale: Vec3::splat(0.3) / size,
                    ..Default::default()
                },
                WorldTransform::identity(),
                Parent { entity: body },
                MeshKind::Sphere,
                material("skin"),
            ));
            spawned.extend([body, head]);
            visuals.push((body, Visual::Villager(i), size));
        }
        for (i, b) in life.beasts().iter().enumerate() {
            if has_visual(&visuals, &|w| matches!(w, Visual::Beast(j) if *j == i)) {
                continue;
            }
            let (size, mat) = beast_look(b.kind);
            let e = world.spawn((
                Name::new(BeastKind::NAMES[b.kind as usize]),
                Transform {
                    translation: PARKED,
                    scale: size,
                    ..Default::default()
                },
                WorldTransform::identity(),
                Parent { entity: effects },
                MeshKind::Cube,
                material(mat),
            ));
            spawned.push(e);
            visuals.push((e, Visual::Beast(i), size));
        }
        for (i, _) in life.carts().iter().enumerate() {
            if has_visual(&visuals, &|w| matches!(w, Visual::Cart(j) if *j == i)) {
                continue;
            }
            let size = Vec3::new(1.4, 1.1, 2.6);
            let e = world.spawn((
                Name::new("Cart"),
                Transform {
                    translation: PARKED,
                    scale: size,
                    ..Default::default()
                },
                WorldTransform::identity(),
                Parent { entity: effects },
                MeshKind::Cube,
                material("hide"),
            ));
            spawned.push(e);
            visuals.push((e, Visual::Cart(i), size));
        }
        // Authored visuals keep their own scale.
        for (e, _, size) in &mut visuals {
            if let Some((_, t)) = authored.iter().find(|(a, _)| a == e) {
                *size = t.scale;
            }
        }
        let colliders = colliders(ctx);
        let tod = ctx
            .world
            .iter_with::<TimeOfDayComponent>()
            .find(|(_, t)| t.enabled)
            .map(|(_, t)| (t.hour, t.eclipse));
        let population = life.villagers().len() + life.beasts().len() + life.carts().len();
        info!(
            villagers = life.villagers().len(),
            beasts = life.beasts().len(),
            carts = life.carts().len(),
            "life: {population} agents in {} places",
            place_entities.len()
        );
        self.run = Some(Run {
            life,
            visuals,
            looks: HashMap::new(),
            authored,
            villager_readouts,
            places: place_entities,
            spawned,
            colliders,
            hour: tod.map_or(6.5, |t| t.0),
            eclipse: tod.map_or(0.0, |t| t.1),
            step_ms: 0.0,
            write_ms: 0.0,
            frames: 0,
            reported: 0.0,
        });
    }

    /// Put everything back as authored.
    pub fn end(&mut self, ctx: &mut EngineContext) {
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
        let Some(run) = &mut self.run else { return };
        let dt = ctx.dt();
        if !(dt.is_finite() && dt > 0.0) {
            return;
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
        let t0 = Instant::now();
        run.life.step(
            dt,
            Clock {
                hour: run.hour,
                eclipse: run.eclipse,
            },
            &focus,
            &Flat,
        );
        let t1 = Instant::now();
        write_visuals(ctx.world, run);
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
            "places": run.places.len(),
            "villagers": run.life.villagers().len(),
            "outdoors": out,
            "beasts": run.life.beasts().len(),
            "carts": run.life.carts().len(),
            "lod": {"near": s.near, "mid": s.mid, "far": s.far, "stepped": s.stepped, "deferred": s.deferred},
            "step_ms": run.step_ms,
            "write_ms": run.write_ms,
        })
    }
}

/// Write each visual's pose when it changed; the far and the indoor are parked.
fn write_visuals(world: &mut World, run: &mut Run) {
    for (entity, which, size) in &run.visuals {
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
    for (entity, index, last) in &mut run.villager_readouts {
        let v = &run.life.villagers()[*index];
        let doing = format!("{:?} ({:?})", v.activity, v.pose);
        if *last != doing {
            last.clone_from(&doing);
            if let Some(c) = world.get_mut::<VillagerComponent>(*entity) {
                c.doing = doing;
            }
        }
    }
    let population = run.life.villagers().len() + run.life.beasts().len() + run.life.carts().len();
    for e in &run.places {
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

    /// The village gathered as Play gathers it keeps its day: by late morning
    /// the farmers are in the fields and most are out; under the Lid all are in.
    #[test]
    fn the_village_keeps_its_day_and_goes_in_under_the_lid() {
        let mut world = World::new();
        spawn_village(&mut world, &json!({"homes": 20})).expect("builds");
        somnium_core::propagate_transforms(&mut world);
        let Gathered {
            mut life, places, ..
        } = gather(&world).expect("a place");
        assert_eq!(places.len(), 1);
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
