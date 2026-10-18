//! The combat sandbox: Rimeborn's combat (`somnium_combat`) in the editor.
//!
//! **Create > Examples > Combat Sandbox** builds an arena under one root: a
//! floor, a pond, dry grass, a snowbank, stone pillars, a draught, a fixed
//! three-quarter camera, the protagonist and two companions, and six raiders
//! (axe, spear, knife, bow, Lattice rifle, flame mage). Every fighter is an
//! ordinary entity with a **Combatant** component, so Details edits its team,
//! weapon, school, forms, pools and spells, and shows its live health, stamina,
//! mana and state during Play.
//!
//! **Play** and you are the protagonist, by mouse and keyboard
//! (`somnium_combat::input`): WASD, click to strike, right click to guard (the
//! first instant parries), Space or a tap of Shift to dodge, 1 to 0 for
//! spells (hold to shape with the mouse; the wheel edits the measure, R picks
//! which), F for the school's art, C for the Kindled Body, Q to lock on, Tab
//! to slow time. The AI fights on its own. Water freezes, grass burns, wind
//! carries the fire. The dead fall as Jolt ragdolls.
//!
//! Everyone on foot walks on `somnium_character`'s controller through Jolt:
//! run (hold Shift) into the low wall to vault it, into the ledge to climb
//! it, over the basin's wall to drop into deep water and swim, and into the
//! wall again to climb out. The AI takes the same course along the navmesh
//! Play bakes from the solid boxes, whose off-mesh links (the scene's
//! **Navigation Link** entities) lead over the wall and up the ledge.
//!
//! The camera is a fixed scene camera on purpose: the isometric rig is the
//! engine lead's (Rimeborn workstream 0). Editor overlays are off in Play, so
//! everything shown is scene content: pooled meshes for missiles, walls,
//! blooms and waves; tiles for the ground's state; particle emitters for fire
//! and impacts; decals for scorch and frost; a game-UI canvas for the HUD.
use glam::{Mat4, Quat, Vec2, Vec3};
use serde_json::{Value, json};
use somnium_ai::dormancy::{Change, Dormancy, DormancySettings, GroupId};
use somnium_ai::navigation::{BakeInput, BakeSettings, Bounds, NavWorld, OffMeshLink};
use somnium_asset::database::AssetId;
use somnium_character::{
    Contact, Controller, ControllerConfig, LocomotionEvent, MantleKind, Mode, MoveInput, Terrain,
};
use somnium_combat::ai::{self, Archetype};
use somnium_combat::components::{CombatSurfaceComponent, CombatantComponent, WindVolumeComponent};
use somnium_combat::environment::{EnvironmentField, Surface, WindVolume};
use somnium_combat::input::{AimView, CombatInput, combat_map, cursor_ray, ray_plane};
use somnium_combat::spell::{Element, MeasureAxis, ROTE_SIZE, rote_name};
use somnium_combat::weapon::{ProjectileKind, WeaponKind};
use somnium_combat::{
    Action, Attitude, CombatEvent, CombatQuery, CombatRules, CombatWorld, CombatantId, DamageType,
    HitFlags, StatusKind, TeamId, WorldHit,
};
use somnium_core::ai::NavigationLink;
use somnium_core::{
    CameraSettingsComponent, EngineContext, EngineEvent, GameUiFrame, InputState,
    MaterialComponent, MeshKind, Name, Parent, ParticleEmitter, Transform, UiCanvas,
    WorldTransform,
};
use somnium_ecs::{Entity, World};
use somnium_physics::body::{BodyId, MotionType, RigidBodyDescriptor};
use somnium_physics::shape::ColliderShape;
use somnium_ui::message::{NodeHandle, TextMessage};
use std::collections::{BTreeMap, HashMap, VecDeque};

/// Height of a fighter's body cylinder and the head above it.
const BODY_HEIGHT: f32 = 1.5;
const BODY_WIDTH: f32 = 0.62;
const HEAD: f32 = 0.32;
/// Where a pooled entity waits when nothing uses it.
const PARKED: Vec3 = Vec3::new(0.0, -10_000.0, 0.0);
const PROJECTILES: usize = 40;
const WALLS: usize = 6;
const BLOOMS: usize = 4;
const RING_BEADS: usize = 16;
const WAVE_BEADS: usize = 10;
const WAVES: usize = 2;
const FIRES: usize = 24;
const BURSTS: usize = 12;
const DECALS: usize = 14;
const LOG_LINES: usize = 6;
/// The vertical field of view hello_engine draws a fight's camera with
/// (`main.rs`, `on_render`): the aim and the marks project through it too.
const VIEW_FOV: f32 = 45.0;
/// "?" and "!" over the heads of those who have noticed the protagonist: how
/// many at once, how long each stays (it fades in two steps over the last
/// second), and their bare colours: bone for a look, a dull red for the alarm.
const MARKS: usize = 8;
const MARK_SECONDS: f32 = 2.5;
const MARK_ALPHA: [u8; 3] = [235, 150, 70];
const MARK_LOOK: [u8; 3] = [214, 206, 186];
const MARK_ALARM: [u8; 3] = [178, 64, 52];

fn material(name: &str) -> MaterialComponent {
    MaterialComponent {
        asset: AssetId::from_relative_path(std::path::Path::new(&format!(
            "materials/combat/{name}.sommat"
        ))),
        runtime_id: 0,
    }
}

pub fn register(r: &mut somnium_core::authoring::GameRegistration) {
    r.components.push(somnium_combat::components::register);
    r.components.push(register_encounter);
    r.presets.push(somnium_core::authoring::GamePreset {
        id: "hello.combat_sandbox",
        label: "Combat Sandbox",
        category: "Examples",
        schema: json!({"type":"object","properties":{"position":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3}},"additionalProperties":false}),
        build: spawn_sandbox,
    });
    r.presets.push(somnium_core::authoring::GamePreset {
        id: "hello.combat_wilds",
        label: "Combat Wilds",
        category: "Examples",
        schema: json!({"type":"object","properties":{
            "position":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},
            "floor":{"type":"boolean","description":"Lay a 60 m floor (untick on terrain: Play stands everyone on the ground)."}
        },"additionalProperties":false}),
        build: spawn_wilds,
    });
}

/// A beast: a long body (with the Combatant) and a head forward.
fn beast(
    world: &mut World,
    root: Entity,
    name: &str,
    feet: Vec3,
    facing: Vec3,
    size: Vec3,
    mat: &str,
    combat: CombatantComponent,
) -> Vec<Entity> {
    let body = world.spawn((
        Name::new(name),
        Transform {
            translation: feet + Vec3::Y * size.y * 0.5,
            rotation: yaw(facing),
            scale: size,
        },
        WorldTransform::identity(),
        Parent { entity: root },
        MeshKind::Cube,
        material(mat),
        combat,
    ));
    let head = world.spawn((
        Name::new(&format!("{name} head")),
        Transform {
            translation: Vec3::new(0.0, 0.25, 0.55),
            scale: Vec3::splat(0.36) / size,
            ..Default::default()
        },
        WorldTransform::identity(),
        Parent { entity: body },
        MeshKind::Sphere,
        material(mat),
    ));
    vec![body, head]
}

/// Combat Wilds: open ground with a horse, a rime wolf pack and a bandit camp,
/// all unaware of the protagonist at first (they face away). Sneak up at a
/// walk for a blow on the unaware; at night (Time of Day) they see a third as
/// far. Untick the floor to drop it on terrain: Play stands everyone on the
/// ground. `only` ("wolves" or "bandits") builds just the pack or the camp,
/// round the position; `facing` turns the whole layout (the pack lies ahead
/// and to the left of a Wilds facing +Z).
fn spawn_wilds(world: &mut World, args: &Value) -> Result<Vec<Entity>, String> {
    let at = origin(args)?;
    let floor = args.get("floor").and_then(Value::as_bool).unwrap_or(true);
    let only = args.get("only").and_then(Value::as_str);
    let all = only.is_none();
    // Built from a scene's Combat Encounter: the scene has its own camera and
    // protagonist.
    let encounter = args
        .get("encounter")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let facing = args
        .get("facing")
        .and_then(|f| serde_json::from_value::<[f32; 3]>(f.clone()).ok())
        .map_or(Vec3::Z, Vec3::from_array);
    let root = world.spawn((
        Name::new("Combat Wilds"),
        Transform {
            translation: at,
            rotation: yaw(facing),
            ..Default::default()
        },
        WorldTransform::identity(),
    ));
    let mut out = vec![root];
    if floor {
        let ground = block(
            world,
            root,
            "Wilds ground",
            Vec3::new(0.0, -0.1, 4.0),
            Vec3::new(60.0, 0.2, 60.0),
            MeshKind::Cube,
            "floor",
        );
        let _ = world.insert_component(
            ground,
            CombatSurfaceComponent {
                surface: Surface::Bare as u32,
                round: false,
                solid: true,
            },
        );
        out.push(ground);
    }
    if all {
        for (name, pos, scale) in [
            (
                "Tall grass (east)",
                Vec3::new(8.0, 0.012, 8.0),
                Vec3::new(8.0, 0.02, 10.0),
            ),
            (
                "Tall grass (west)",
                Vec3::new(-8.0, 0.012, 6.0),
                Vec3::new(7.0, 0.02, 8.0),
            ),
        ] {
            let e = block(world, root, name, pos, scale, MeshKind::Cube, "grass");
            let _ = world.insert_component(
                e,
                CombatSurfaceComponent {
                    surface: Surface::Grass as u32,
                    round: false,
                    solid: false,
                },
            );
            out.push(e);
        }
        for (i, (pos, scale)) in [
            (Vec3::new(3.0, 0.6, 12.0), Vec3::new(2.0, 1.2, 1.6)),
            (Vec3::new(-4.0, 0.9, 13.5), Vec3::new(1.6, 1.8, 2.2)),
            (Vec3::new(11.0, 0.5, 4.0), Vec3::new(1.4, 1.0, 1.4)),
        ]
        .into_iter()
        .enumerate()
        {
            let e = block(
                world,
                root,
                &format!("Boulder {}", i + 1),
                pos,
                scale,
                MeshKind::Cube,
                "stone",
            );
            let _ = world.insert_component(
                e,
                CombatSurfaceComponent {
                    surface: Surface::Stone as u32,
                    round: false,
                    solid: true,
                },
            );
            out.push(e);
        }
    }
    if !encounter {
        let eye = Vec3::new(0.0, 34.0, -30.0);
        let camera_world = Mat4::look_at_rh(eye, Vec3::new(0.0, 0.0, 8.0), Vec3::Y).inverse();
        let (_, rotation, _) = camera_world.to_scale_rotation_translation();
        out.push(world.spawn((
            Name::new("Combat Camera"),
            Transform {
                translation: eye,
                rotation,
                ..Default::default()
            },
            WorldTransform::identity(),
            Parent { entity: root },
            CameraSettingsComponent::default(),
        )));
        out.extend(fighter(
            world,
            root,
            "Protagonist",
            Vec3::new(0.0, 0.0, -8.0),
            Vec3::Z,
            CombatantComponent::protagonist(),
        ));
    }
    if all {
        out.push(world.spawn((
            Name::new("Horse"),
            Transform {
                translation: Vec3::new(3.0, 0.8, -8.5),
                rotation: yaw(Vec3::Z),
                scale: Vec3::new(0.85, 1.6, 2.4),
            },
            WorldTransform::identity(),
            Parent { entity: root },
            MeshKind::Cube,
            material("hide"),
            CombatantComponent {
                team: 0,
                mount: 1,
                max_health: 200.0,
                poise: 80.0,
                ..CombatantComponent::default()
            },
        )));
    }
    if all || only == Some("wolves") {
        let den = if all {
            Vec3::new(14.0, 0.0, 18.0)
        } else {
            Vec3::ZERO
        };
        spawn_pack(world, root, den, &mut out);
    }
    if all || only == Some("bandits") {
        let camp = if all {
            Vec3::new(-14.0, 0.0, 16.0)
        } else {
            Vec3::ZERO
        };
        spawn_camp(world, root, camp, &mut out);
    }
    Ok(out)
}

/// A rime wolf pack round its den, the alpha leading; facing away.
fn spawn_pack(world: &mut World, root: Entity, den: Vec3, out: &mut Vec<Entity>) {
    let wolf = |alpha: bool| CombatantComponent {
        team: 2,
        archetype: Archetype::Wolf as u32,
        weapon: WeaponKind::Fangs as u32,
        max_health: if alpha { 110.0 } else { 75.0 },
        max_stamina: 140.0,
        poise: if alpha { 45.0 } else { 30.0 },
        speed: 5.0,
        leader: alpha,
        ..CombatantComponent::default()
    };
    out.extend(beast(
        world,
        root,
        "Rime wolf (alpha)",
        den,
        Vec3::Z,
        Vec3::new(0.6, 0.85, 1.4),
        "frost",
        wolf(true),
    ));
    for i in 0..4 {
        let a = i as f32 / 4.0 * std::f32::consts::TAU;
        out.extend(beast(
            world,
            root,
            &format!("Rime wolf {}", i + 1),
            den + Vec3::new(a.cos(), 0.0, a.sin()) * 2.5,
            Vec3::new(a.sin(), 0.0, a.cos()),
            Vec3::new(0.5, 0.75, 1.25),
            "snow",
            wolf(false),
        ));
    }
}

/// A bandit camp round its chief; facing in.
fn spawn_camp(world: &mut World, root: Entity, camp: Vec3, out: &mut Vec<Entity>) {
    for (i, (name, archetype, weapon, leader)) in [
        ("Bandit chief", Archetype::Duelist, WeaponKind::Axe, true),
        (
            "Bandit (spear)",
            Archetype::Spearman,
            WeaponKind::Spear,
            false,
        ),
        ("Bandit (axe)", Archetype::Brute, WeaponKind::Axe, false),
        ("Bandit (bow)", Archetype::Archer, WeaponKind::Bow, false),
    ]
    .into_iter()
    .enumerate()
    {
        let a = i as f32 / 4.0 * std::f32::consts::TAU;
        let spot = camp + Vec3::new(a.cos(), 0.0, a.sin()) * 3.0;
        out.extend(fighter(
            world,
            root,
            name,
            spot,
            camp - spot,
            CombatantComponent {
                team: 1,
                archetype: archetype as u32,
                weapon: weapon as u32,
                max_health: if leader { 160.0 } else { 100.0 },
                armor: if leader { 4.0 } else { 2.0 },
                leader,
                ..CombatantComponent::default()
            },
        ));
    }
}

fn origin(args: &Value) -> Result<Vec3, String> {
    let p = Vec3::from_array(
        serde_json::from_value(args.get("position").cloned().unwrap_or(json!([0, 0.05, 0])))
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

fn yaw(facing: Vec3) -> Quat {
    let f = Vec3::new(facing.x, 0.0, facing.z).normalize_or(Vec3::Z);
    Quat::from_rotation_y(f.x.atan2(f.z))
}

/// A fighter: a body cylinder (with the Combatant), a head and a blade marking
/// which way it faces.
fn fighter(
    world: &mut World,
    root: Entity,
    name: &str,
    feet: Vec3,
    facing: Vec3,
    combat: CombatantComponent,
) -> Vec<Entity> {
    let team = if combat.team == 0 { "party" } else { "raider" };
    let body = world.spawn((
        Name::new(name),
        Transform {
            translation: feet + Vec3::Y * BODY_HEIGHT * 0.5,
            rotation: yaw(facing),
            scale: Vec3::new(BODY_WIDTH, BODY_HEIGHT, BODY_WIDTH),
        },
        WorldTransform::identity(),
        Parent { entity: root },
        MeshKind::Cylinder,
        material(team),
        combat,
    ));
    let s = Vec3::new(BODY_WIDTH, BODY_HEIGHT, BODY_WIDTH);
    let head = world.spawn((
        Name::new(&format!("{name} head")),
        Transform {
            translation: Vec3::new(0.0, (BODY_HEIGHT * 0.5 + HEAD * 0.5) / BODY_HEIGHT, 0.0),
            scale: Vec3::splat(HEAD) / s,
            ..Default::default()
        },
        WorldTransform::identity(),
        Parent { entity: body },
        MeshKind::Sphere,
        material("skin"),
    ));
    let blade = world.spawn((
        Name::new(&format!("{name} weapon")),
        Transform {
            translation: Vec3::new(0.25, 0.12, 0.45) / s,
            scale: Vec3::new(0.06, 0.06, 0.9) / s,
            ..Default::default()
        },
        WorldTransform::identity(),
        Parent { entity: body },
        MeshKind::Cube,
        material("dark"),
    ));
    vec![body, head, blade]
}

fn raider(archetype: Archetype, weapon: WeaponKind) -> CombatantComponent {
    CombatantComponent {
        team: 1,
        archetype: archetype as u32,
        weapon: weapon as u32,
        ..CombatantComponent::default()
    }
}

fn spawn_sandbox(world: &mut World, args: &Value) -> Result<Vec<Entity>, String> {
    let at = origin(args)?;
    let root = world.spawn((
        Name::new("Combat Sandbox"),
        Transform::from_translation(at),
        WorldTransform::identity(),
    ));
    let mut out = vec![root];

    let floor = block(
        world,
        root,
        "Arena floor",
        Vec3::new(0.0, -0.1, 0.0),
        Vec3::new(40.0, 0.2, 40.0),
        MeshKind::Cube,
        "floor",
    );
    let _ = world.insert_component(
        floor,
        CombatSurfaceComponent {
            surface: Surface::Bare as u32,
            round: false,
            solid: true,
        },
    );
    out.push(floor);
    for (name, pos, scale, kind, mat, surface, round) in [
        (
            "Pond",
            Vec3::new(-8.0, 0.012, 1.0),
            Vec3::new(7.0, 0.02, 7.0),
            MeshKind::Cylinder,
            "water",
            Surface::Water,
            true,
        ),
        (
            "Dry grass",
            Vec3::new(8.0, 0.012, 2.0),
            Vec3::new(10.0, 0.02, 9.0),
            MeshKind::Cube,
            "grass",
            Surface::Grass,
            false,
        ),
        (
            "Snowbank",
            Vec3::new(-9.0, 0.012, -9.0),
            Vec3::new(6.0, 0.02, 5.0),
            MeshKind::Cube,
            "snow",
            Surface::Snow,
            false,
        ),
    ] {
        let e = block(world, root, name, pos, scale, kind, mat);
        let _ = world.insert_component(
            e,
            CombatSurfaceComponent {
                surface: surface as u32,
                round,
                solid: false,
            },
        );
        out.push(e);
    }
    for (i, pos) in [
        Vec3::new(-3.0, 1.5, 2.0),
        Vec3::new(3.5, 1.5, 0.0),
        Vec3::new(0.5, 1.5, 11.0),
    ]
    .into_iter()
    .enumerate()
    {
        let e = block(
            world,
            root,
            &format!("Stone pillar {}", i + 1),
            pos,
            Vec3::new(1.2, 3.0, 1.2),
            MeshKind::Cube,
            "stone",
        );
        let _ = world.insert_component(
            e,
            CombatSurfaceComponent {
                surface: Surface::Stone as u32,
                round: false,
                solid: true,
            },
        );
        out.push(e);
    }
    // A traversal course for the controller: a wall to vault, a ledge to
    // climb, and a walled basin of deep water to swim in.
    let stone = |world: &mut World, name: &str, at: Vec3, scale: Vec3| {
        let e = block(world, root, name, at, scale, MeshKind::Cube, "stone");
        let _ = world.insert_component(
            e,
            CombatSurfaceComponent {
                surface: Surface::Stone as u32,
                round: false,
                solid: true,
            },
        );
        e
    };
    out.push(stone(
        world,
        "Low wall",
        Vec3::new(9.0, 0.5, -5.0),
        Vec3::new(0.3, 1.0, 3.0),
    ));
    out.push(stone(
        world,
        "Ledge",
        Vec3::new(13.0, 1.0, -5.0),
        Vec3::new(3.0, 2.0, 3.0),
    ));
    // As high as the ledge, two metres beyond it: a gap to leap.
    out.push(stone(
        world,
        "Far ledge",
        Vec3::new(18.0, 1.0, -5.0),
        Vec3::new(3.0, 2.0, 3.0),
    ));
    for (name, at, scale) in [
        (
            "Basin wall (north)",
            Vec3::new(12.5, 0.8, -8.65),
            Vec3::new(7.0, 1.6, 0.3),
        ),
        (
            "Basin wall (south)",
            Vec3::new(12.5, 0.8, -12.85),
            Vec3::new(7.0, 1.6, 0.3),
        ),
        (
            "Basin wall (west)",
            Vec3::new(9.15, 0.8, -10.75),
            Vec3::new(0.3, 1.6, 3.9),
        ),
        (
            "Basin wall (east)",
            Vec3::new(15.85, 0.8, -10.75),
            Vec3::new(0.3, 1.6, 3.9),
        ),
    ] {
        out.push(stone(world, name, at, scale));
    }
    // Water is a non-solid water surface; its top is the water line.
    let basin = block(
        world,
        root,
        "Basin water",
        Vec3::new(12.5, 1.48, -10.75),
        Vec3::new(6.4, 0.04, 3.9),
        MeshKind::Cube,
        "water",
    );
    let _ = world.insert_component(
        basin,
        CombatSurfaceComponent {
            surface: Surface::Water as u32,
            round: false,
            solid: false,
        },
    );
    out.push(basin);
    // West: a 6 m bluff with a 27 degree slope up to it from the north and
    // sheer drops on its other sides, for a mount's footing and a fall.
    out.push(stone(
        world,
        "Bluff",
        Vec3::new(-16.0, 3.0, -4.0),
        Vec3::new(6.0, 6.0, 4.0),
    ));
    let rise = (6.0f32 / 12.0).atan();
    let tilt = Quat::from_rotation_x(rise);
    let slope = world.spawn((
        Name::new("Bluff slope"),
        Transform {
            translation: Vec3::new(-16.0, 3.0, 4.0) - tilt * Vec3::Y * 0.25,
            rotation: tilt,
            scale: Vec3::new(6.0, 0.5, 12.0 / rise.cos()),
        },
        WorldTransform::identity(),
        Parent { entity: root },
        MeshKind::Cube,
        material("stone"),
        CombatSurfaceComponent {
            surface: Surface::Stone as u32,
            round: false,
            solid: true,
        },
    ));
    out.push(slope);
    // Off-mesh links, so the AI's routes take the course: each runs from
    // one side to the other. A link that rises is climbed; a level one is
    // jumped at its start (a gap is leapt, a low wall cleared, a higher one
    // met in the jump vaulted or climbed); one that falls is walked off. Swimming is slower
    // than walking, so the water's links cost a metre more.
    for (name, from, to, cost) in [
        (
            "Nav link (over the low wall)",
            Vec3::new(8.25, 0.0, -5.0),
            Vec3::new(9.75, 0.0, -5.0),
            0.0,
        ),
        (
            "Nav link (up the ledge)",
            Vec3::new(10.6, 0.0, -5.0),
            Vec3::new(12.6, 2.0, -5.0),
            0.0,
        ),
        (
            "Nav link (across the gap)",
            Vec3::new(14.1, 2.0, -5.0),
            Vec3::new(17.2, 2.0, -5.0),
            0.0,
        ),
        (
            "Nav link (into the basin)",
            Vec3::new(12.5, 0.0, -7.9),
            Vec3::new(12.5, 0.0, -9.7),
            1.0,
        ),
        (
            "Nav link (out of the basin)",
            Vec3::new(12.5, 0.0, -12.0),
            Vec3::new(12.5, 0.0, -13.7),
            1.0,
        ),
    ] {
        out.push(world.spawn((
            Name::new(name),
            Transform::from_translation(from),
            WorldTransform::identity(),
            Parent { entity: root },
            NavigationLink {
                destination: at + to,
                attachment_radius: 0.6,
                extra_cost: cost,
                ..NavigationLink::default()
            },
        )));
    }
    out.push(world.spawn((
        Name::new("Draught"),
        Transform::from_translation(Vec3::new(4.0, 1.0, 3.0)),
        WorldTransform::identity(),
        Parent { entity: root },
        WindVolumeComponent {
            direction: Vec3::X,
            strength: 7.0,
            radius: 9.0,
        },
    )));
    // A three-quarter view over the field; the isometric rig replaces it.
    let eye = Vec3::new(0.0, 26.0, -26.0);
    let camera_world = Mat4::look_at_rh(eye, Vec3::new(0.0, 0.0, 0.5), Vec3::Y).inverse();
    let (_, rotation, _) = camera_world.to_scale_rotation_translation();
    out.push(world.spawn((
        Name::new("Combat Camera"),
        Transform {
            translation: eye,
            rotation,
            ..Default::default()
        },
        WorldTransform::identity(),
        Parent { entity: root },
        CameraSettingsComponent::default(),
    )));

    let party = Vec3::Z;
    let foe = -Vec3::Z;
    out.extend(fighter(
        world,
        root,
        "Protagonist",
        Vec3::new(0.0, 0.0, -8.0),
        party,
        CombatantComponent::protagonist(),
    ));
    // Tyffir's mount, saddled beside him: E climbs on.
    out.push(world.spawn((
        Name::new("Frost-elk"),
        Transform {
            translation: Vec3::new(3.2, 0.8, -7.5),
            rotation: yaw(party),
            scale: Vec3::new(0.9, 1.6, 2.3),
        },
        WorldTransform::identity(),
        Parent { entity: root },
        MeshKind::Cube,
        material("hide"),
        CombatantComponent {
            team: 0,
            mount: 2,
            max_health: 220.0,
            poise: 90.0,
            ..CombatantComponent::default()
        },
    )));
    out.extend(fighter(
        world,
        root,
        "Companion (spear)",
        Vec3::new(-2.2, 0.0, -9.5),
        party,
        CombatantComponent {
            team: 0,
            archetype: Archetype::Companion as u32,
            weapon: WeaponKind::Spear as u32,
            max_health: 130.0,
            ..CombatantComponent::default()
        },
    ));
    out.extend(fighter(
        world,
        root,
        "Companion (mender)",
        Vec3::new(2.2, 0.0, -9.5),
        party,
        CombatantComponent {
            team: 0,
            archetype: Archetype::Companion as u32,
            weapon: WeaponKind::Unarmed as u32,
            capacity: 220.0,
            spells: "close_wound,water_bolt".into(),
            ..CombatantComponent::default()
        },
    ));
    for (name, pos, mut c) in [
        (
            "Raider (axe)",
            Vec3::new(-3.0, 0.0, 7.0),
            raider(Archetype::Brute, WeaponKind::Axe),
        ),
        (
            "Raider (spear)",
            Vec3::new(0.0, 0.0, 8.0),
            raider(Archetype::Spearman, WeaponKind::Spear),
        ),
        (
            "Raider (knife)",
            Vec3::new(3.5, 0.0, 6.5),
            raider(Archetype::Skirmisher, WeaponKind::Knife),
        ),
        (
            "Raider (bow)",
            Vec3::new(-7.0, 0.0, 13.0),
            raider(Archetype::Archer, WeaponKind::Bow),
        ),
        (
            "Raider (Lattice rifle)",
            Vec3::new(7.0, 0.0, 14.0),
            raider(Archetype::Rifleman, WeaponKind::LatticeRifle),
        ),
        (
            "Raider (flame mage)",
            Vec3::new(0.0, 0.0, 15.0),
            raider(Archetype::Caster, WeaponKind::Unarmed),
        ),
    ] {
        match c.weapon_kind() {
            WeaponKind::Axe => {
                c.max_health = 140.0;
                c.poise = 60.0;
                c.armor = 4.0;
            }
            WeaponKind::Knife => c.speed = 4.3,
            WeaponKind::Unarmed => {
                c.capacity = 160.0;
                c.rank = 2;
                c.spells = "ember_bolt,fireburst,close_wound".into();
            }
            _ => {}
        }
        out.extend(fighter(world, root, name, pos, foe, c));
    }
    Ok(out)
}

/// Build the sandbox at the origin outside the Create menu (the capture hook,
/// `SOMNIUM_COMBAT_SANDBOX=1`); with `SOMNIUM_COMBAT_WILDS=1` as well, the
/// Wilds instead; with `SOMNIUM_COMBAT_ENCOUNTERS=1` too, a sleeping bandit
/// camp beside it (woken at Play) and one 400 m off (asleep).
pub fn spawn_at_startup(world: &mut World) -> Result<usize, String> {
    let on = |name: &str| std::env::var(name).as_deref() == Ok("1");
    let mut spawned = if on("SOMNIUM_COMBAT_WILDS") {
        spawn_wilds(world, &json!({}))?
    } else {
        spawn_sandbox(world, &json!({}))?
    };
    if on("SOMNIUM_COMBAT_ENCOUNTERS") {
        for (name, at) in [
            ("Bandit camp (near)", Vec3::new(20.0, 0.05, 28.0)),
            ("Bandit camp (far)", Vec3::new(400.0, 0.05, 0.0)),
        ] {
            spawned.push(world.spawn((
                Name::new(name),
                Transform::from_translation(at),
                WorldTransform::identity(),
                CombatEncounterComponent {
                    kind: "bandits".into(),
                    ..CombatEncounterComponent::default()
                },
            )));
        }
    }
    Ok(spawned.len())
}

/// Whether the scene has a player-controlled fighter, so Play is the
/// sandbox's (hello_engine then spawns no first-person character).
pub fn claims_play(world: &World) -> bool {
    world
        .iter_with::<CombatantComponent>()
        .any(|(_, c)| c.enabled && c.player)
        || world
            .iter_with::<CombatEncounterComponent>()
            .any(|(_, c)| c.enabled)
}

/// A fight authored into a scene: on Play the host builds the encounter at
/// this entity's position, its fighters stood on the ground there, and takes
/// it away again on Stop. The scene keeps its own camera and protagonist.
#[derive(Clone, Debug, PartialEq)]
pub struct CombatEncounterComponent {
    /// Built on Play.
    pub enabled: bool,
    /// Which: `wilds` (a horse, a rime wolf pack and a bandit camp, none of
    /// them aware yet), `wolves` (the pack alone) or `bandits` (the camp).
    pub kind: String,
    /// Bring its own flat ground; off stands everyone on the scene's.
    pub floor: bool,
}

impl somnium_ecs::Component for CombatEncounterComponent {}

impl Default for CombatEncounterComponent {
    fn default() -> Self {
        Self {
            enabled: true,
            kind: "wilds".into(),
            floor: false,
        }
    }
}

fn register_encounter(registry: &mut somnium_ecs::reflect::TypeRegistry) {
    registry.register(somnium_ecs::component_schema! {
        CombatEncounterComponent as "somnium.CombatEncounter", display "Combat Encounter", version 1,
        fields {
            enabled { doc: "Built on Play at this entity's position, taken away on Stop." },
            kind { doc: "wilds: a horse, a rime wolf pack and a bandit camp, none of them aware yet; wolves: the pack alone; bandits: the camp alone." },
            floor { doc: "Bring its own flat ground; off stands everyone on the ground here." },
        }
    });
}

/// Every enabled Combat Encounter built at its entity's position (the
/// scene's own camera and protagonist kept): the entities made, for Stop to
/// take away.
fn expand_encounters(world: &mut World) -> Vec<Entity> {
    let wanted: Vec<(Mat4, CombatEncounterComponent)> = world
        .iter_with::<CombatEncounterComponent>()
        .filter(|(_, c)| c.enabled)
        .map(|(e, c)| (world_matrix(world, e), c.clone()))
        .collect();
    let mut made = Vec::new();
    for (m, encounter) in wanted {
        let (at, facing) = (m.transform_point3(Vec3::ZERO), m.transform_vector3(Vec3::Z));
        match build_encounter(world, &encounter.kind, at, facing, encounter.floor) {
            Ok(entities) => made.extend(entities),
            Err(e) => tracing::warn!("Combat Encounter at {at}: {e}"),
        }
    }
    made
}

/// One encounter of `kind` built at `at` and turned to `facing`, without a
/// camera or protagonist.
fn build_encounter(
    world: &mut World,
    kind: &str,
    at: Vec3,
    facing: Vec3,
    floor: bool,
) -> Result<Vec<Entity>, String> {
    let mut args = json!({
        "position": [at.x, at.y, at.z],
        "facing": [facing.x, facing.y, facing.z],
        "floor": floor,
        "encounter": true,
    });
    match kind.trim().to_ascii_lowercase().as_str() {
        "wilds" => {}
        part @ ("wolves" | "bandits") => args["only"] = json!(part),
        other => return Err(format!("no kind {other:?} (try wilds, wolves or bandits)")),
    }
    spawn_wilds(world, &args)
}

/// How far an encounter of `kind` spreads from its centre, metres.
fn encounter_extent(kind: &str) -> f32 {
    if kind.trim().eq_ignore_ascii_case("wilds") {
        25.0
    } else {
        6.0
    }
}

/// How often sleeping encounters are looked at, seconds (the check costs
/// only the grid cells around the protagonist, but there is no need to look
/// sixty times a second).
const TEND_EVERY: f32 = 0.5;

/// A scene encounter while Play runs: what it is, who of it is left once it
/// has woken and slept again, and while it is awake what it made.
#[derive(Clone, Debug, Default)]
struct Encounter {
    kind: String,
    floor: bool,
    /// Its entity's forward: the layout turns with it.
    facing: Vec3,
    /// Survivors by name, with their health, after its first sleep; `None`
    /// before it ever woke (everyone).
    left: Option<Vec<(String, f32)>>,
    /// Awake: its entities, fighters (and where each stood when it woke)
    /// and colliders.
    entities: Vec<Entity>,
    members: Vec<CombatantId>,
    starts: Vec<Vec3>,
    solids: Vec<BodyId>,
}

/// Every enabled Combat Encounter as a sleeping record at its entity.
fn encounters_asleep(world: &World) -> Dormancy<Encounter> {
    let mut set = Dormancy::new(DormancySettings::default());
    for (e, c) in world.iter_with::<CombatEncounterComponent>() {
        if c.enabled {
            let m = world_matrix(world, e);
            set.add(
                m.transform_point3(Vec3::ZERO),
                encounter_extent(&c.kind),
                Encounter {
                    kind: c.kind.clone(),
                    floor: c.floor,
                    facing: m.transform_vector3(Vec3::Z),
                    ..Encounter::default()
                },
            );
        }
    }
    set
}

/// Wake the encounters the protagonist has come near and put back to sleep
/// those it has left, every [`TEND_EVERY`] seconds (`dt` of infinity looks
/// now, and keeps looking until nothing more wakes: Play's start). Without
/// physics (headless) a woken group gets no colliders and is not stood on
/// the ground.
fn tend_encounters(
    world: &mut World,
    mut physics: Option<&mut somnium_physics::world::PhysicsWorld>,
    run: &mut Run,
    dt: f32,
) {
    if run.encounters.is_none() {
        return;
    }
    run.tended += dt;
    if run.tended < TEND_EVERY {
        return;
    }
    run.tended = 0.0;
    let watchers: Vec<Vec3> = run
        .player
        .and_then(|p| run.world.combatant(p))
        .map(|c| c.position)
        .into_iter()
        .collect();
    loop {
        let changes = run
            .encounters
            .as_mut()
            .map(|set| set.update(&watchers))
            .unwrap_or_default();
        let woke = changes.iter().any(|c| matches!(c, Change::Wake(_)));
        for change in changes {
            match change {
                Change::Wake(id) => wake(world, physics.as_deref_mut(), run, id),
                Change::Sleep(id) => sleep(world, physics.as_deref_mut(), run, id),
            }
        }
        if !(woke && dt.is_infinite()) {
            break;
        }
    }
}

/// Build a woken encounter at its record's place, with only its survivors
/// (at their health), into the running fight.
fn wake(
    world: &mut World,
    physics: Option<&mut somnium_physics::world::PhysicsWorld>,
    run: &mut Run,
    id: GroupId,
) {
    let Some(set) = &run.encounters else {
        return;
    };
    let (Some(at), Some(record)) = (set.position(id), set.data(id).cloned()) else {
        return;
    };
    if record.left.as_ref().is_some_and(Vec::is_empty) {
        // Cleared: nobody left to wake.
        return;
    }
    let made =
        build_encounter(world, &record.kind, at, record.facing, record.floor).unwrap_or_default();
    somnium_core::propagate_transforms(world);
    let mut left = record.left.clone();
    let mut members = Vec::new();
    let mut leaders = Vec::new();
    let mut health = Vec::new();
    let mut fallen = Vec::new();
    for &e in &made {
        let Some(c) = world
            .get::<CombatantComponent>(e)
            .filter(|c| c.enabled)
            .cloned()
        else {
            continue;
        };
        let name = world
            .get::<Name>(e)
            .map_or_else(String::new, |n| n.as_str().to_string());
        // One survivor of each name answers for one fighter of that name.
        let kept = match &mut left {
            None => Some(None),
            Some(left) => left
                .iter()
                .position(|(n, _)| *n == name)
                .map(|i| Some(left.remove(i).1)),
        };
        let Some(hp) = kept else {
            fallen.push(e);
            continue;
        };
        let (cid, feet, size) = enlist(&mut run.world, world, e, &c);
        if c.leader {
            leaders.push(cid);
        }
        if let Some(hp) = hp {
            health.push((cid, hp));
        }
        run.bodies.insert(cid, e);
        run.dims.insert(cid, size);
        if let Some(body) = run.world.combatant(cid) {
            let walker = Controller::new(walker_config(body), feet + Vec3::Y * 0.02);
            run.walkers.insert(cid, walker);
        }
        members.push(cid);
    }
    for (cid, hp) in health {
        if let Some(c) = run.world.combatant_mut(cid) {
            c.vitals.health.current = hp.min(c.vitals.health.max);
        }
    }
    // The fallen go with everything hung on them (a head, a blade).
    let under_fallen = |world: &World, e: Entity| {
        let mut at = Some(e);
        while let Some(x) = at {
            if fallen.contains(&x) {
                return true;
            }
            at = world.get::<Parent>(x).map(|p| p.entity);
        }
        false
    };
    let (gone, made): (Vec<Entity>, Vec<Entity>) =
        made.into_iter().partition(|e| under_fallen(world, *e));
    for e in gone {
        world.despawn(e);
    }
    organise(&mut run.world, &members, &leaders);
    let mut solids = Vec::new();
    if let Some(physics) = physics {
        let boxes: Vec<Mat4> = made
            .iter()
            .filter(|e| {
                world
                    .get::<CombatSurfaceComponent>(**e)
                    .is_some_and(|s| s.solid)
            })
            .map(|e| world_matrix(world, *e))
            .collect();
        solids = solid_bodies(physics, boxes);
        stand(physics, run, &members);
    }
    tracing::info!(
        "Combat Encounter {} ({}) woke at {at}: {} fighters",
        id.0,
        record.kind,
        members.len()
    );
    run.spawned.extend(made.iter().copied());
    run.colliders.extend(solids.iter().copied());
    let starts = members
        .iter()
        .map(|m| run.world.combatant(*m).map_or(at, |c| c.position))
        .collect();
    if let Some(record) = run.encounters.as_mut().and_then(|set| set.data_mut(id)) {
        record.entities = made;
        record.members = members;
        record.starts = starts;
        record.solids = solids;
    }
}

/// Save who of a group is left (and how hurt), move its record to where it
/// went, and take it out of the fight and the scene.
fn sleep(
    world: &mut World,
    mut physics: Option<&mut somnium_physics::world::PhysicsWorld>,
    run: &mut Run,
    id: GroupId,
) {
    let Some(record) = run.encounters.as_mut().and_then(|set| set.data_mut(id)) else {
        return;
    };
    let entities = std::mem::take(&mut record.entities);
    let members = std::mem::take(&mut record.members);
    let starts = std::mem::take(&mut record.starts);
    let solids = std::mem::take(&mut record.solids);
    let was_built = !members.is_empty();
    tracing::info!("Combat Encounter {} fell asleep", id.0);
    // Who is left, and how far they went since they woke: the record follows
    // them on the ground, and the layout is rebuilt round it next time.
    let alive: Vec<(String, f32, Vec3)> = members
        .iter()
        .zip(&starts)
        .filter_map(|(m, start)| Some((run.world.combatant(*m)?, *start)))
        .filter(|(c, _)| c.alive())
        .map(|(c, start)| (c.name.clone(), c.vitals.health.current, c.position - start))
        .collect();
    if let Some(set) = run.encounters.as_mut() {
        if was_built && let Some(record) = set.data_mut(id) {
            record.left = Some(alive.iter().map(|a| (a.0.clone(), a.1)).collect());
        }
        if let (false, Some(at)) = (alive.is_empty(), set.position(id)) {
            let went = alive.iter().map(|a| a.2).sum::<Vec3>() / alive.len() as f32;
            set.move_to(id, at + Vec3::new(went.x, 0.0, went.z));
        }
    }
    for m in &members {
        run.world.despawn(*m);
        run.bodies.remove(m);
        run.dims.remove(m);
        run.walkers.remove(m);
        run.marks.remove(m);
        if let (Some(ragdoll), Some(physics)) = (run.ragdolls.remove(m), physics.as_deref_mut()) {
            let _ = physics.destroy_ragdoll(ragdoll);
        }
    }
    run.spawned.retain(|e| !entities.contains(e));
    for e in entities {
        world.despawn(e);
    }
    run.colliders.retain(|b| !solids.contains(b));
    if let Some(physics) = physics {
        for b in solids {
            physics.destroy_body(b);
        }
    }
}

/// The physics world as combat sees it: a thin capsule swept through Jolt.
struct JoltQuery<'a>(&'a somnium_physics::world::PhysicsWorld);

impl CombatQuery for JoltQuery<'_> {
    fn sweep(&self, from: Vec3, to: Vec3, radius: f32) -> Option<WorldHit> {
        let hit = self
            .0
            .cast_capsule(somnium_physics::animation::CapsuleSweep {
                position: from,
                rotation: Quat::IDENTITY,
                displacement: to - from,
                half_height: 0.001,
                radius: radius.max(0.01),
                ignore_body: None,
            })
            .ok()??;
        Some(WorldHit {
            fraction: hit.fraction,
            normal: hit.normal,
        })
    }
}

/// A water surface's footprint: centre, half size, round, height of the line.
#[derive(Clone, Copy)]
struct Pool {
    centre: Vec2,
    half: Vec2,
    round: bool,
    line: f32,
}

/// The world as the protagonist's controller sees it: upright capsules swept
/// through Jolt, and the scene's water surfaces.
struct JoltTerrain<'a> {
    physics: &'a somnium_physics::world::PhysicsWorld,
    water: &'a [Pool],
}

impl Terrain for JoltTerrain<'_> {
    fn sweep(&self, feet: Vec3, delta: Vec3, radius: f32, height: f32) -> Option<Contact> {
        let half = (height * 0.5 - radius).max(0.001);
        let hit = self
            .physics
            .cast_capsule(somnium_physics::animation::CapsuleSweep {
                position: feet + Vec3::Y * (half + radius),
                rotation: Quat::IDENTITY,
                displacement: delta,
                half_height: half,
                radius,
                ignore_body: None,
            })
            .ok()??;
        Some(Contact {
            fraction: hit.fraction,
            normal: hit.normal,
        })
    }

    fn water_surface(&self, x: f32, z: f32) -> Option<f32> {
        let p = Vec2::new(x, z);
        self.water
            .iter()
            .filter(|w| {
                let d = p - w.centre;
                if w.round {
                    d.length() <= w.half.max_element()
                } else {
                    d.abs().cmple(w.half).all()
                }
            })
            .map(|w| w.line)
            .reduce(f32::max)
    }
}

/// A terrain with the fight's standing spell walls (stone, ice, force) in it.
struct Walled<'a, T> {
    terrain: &'a T,
    combat: &'a CombatWorld,
}

impl<T: Terrain> Terrain for Walled<'_, T> {
    fn sweep(&self, feet: Vec3, delta: Vec3, radius: f32, height: f32) -> Option<Contact> {
        let ground = self.terrain.sweep(feet, delta, radius, height);
        match self.combat.wall_contact(feet, delta, radius, height) {
            Some(w) if ground.is_none_or(|g| w.fraction < g.fraction) => Some(Contact {
                fraction: w.fraction,
                normal: w.normal,
            }),
            _ => ground,
        }
    }

    fn water_surface(&self, x: f32, z: f32) -> Option<f32> {
        self.terrain.water_surface(x, z)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Look {
    Hidden,
    Shown(Vec3, Quat, Vec3),
}

/// Pooled entities for what comes and goes.
#[derive(Default)]
struct Pools {
    projectiles: Vec<(Entity, &'static str)>,
    walls: Vec<(Entity, &'static str)>,
    blooms: Vec<(Entity, &'static str)>,
    /// Which bloom a ring's beads belong to has nothing to do with order here:
    /// ring `i` is beads `i * RING_BEADS ..`.
    beads: Vec<Entity>,
    fires: Vec<(Entity, bool)>,
    bursts: Vec<Entity>,
    decals: Vec<Entity>,
    next_burst: usize,
    next_decal: usize,
    looks: HashMap<Entity, Look>,
}

struct Run {
    world: CombatWorld,
    /// Parent of every entity Play spawns (pools, ground tiles).
    effects: Option<Entity>,
    bodies: BTreeMap<CombatantId, Entity>,
    /// Each body's world size as authored (a fighter's cylinder, a mount's box).
    dims: BTreeMap<CombatantId, Vec3>,
    player: Option<CombatantId>,
    /// Locomotion (vault, climb, swim) for everyone but the mounts, which
    /// keep the combat world's own motion and carry their riders.
    walkers: BTreeMap<CombatantId, Controller>,
    /// The navmesh baked at Play from the solid boxes, with the scene's
    /// Navigation Links; an editor bake takes over when there is one.
    nav: Option<NavWorld>,
    water: Vec<Pool>,
    camera: Option<Entity>,
    restore: Vec<(Entity, Transform)>,
    colliders: Vec<BodyId>,
    ragdolls: BTreeMap<CombatantId, somnium_physics::animation::RagdollId>,
    /// Field tile (2 x 2 cells) to its entity and the surface it shows.
    tiles: BTreeMap<(u32, u32), (Entity, Surface)>,
    floor: f32,
    pools: Pools,
    spawned: Vec<Entity>,
    log: VecDeque<String>,
    /// Who has noticed the protagonist, how far (suspicious or alerted), and
    /// the seconds its mark has left.
    marks: BTreeMap<CombatantId, (somnium_combat::ai::Awareness, f32)>,
    /// The scene's encounters, asleep until the protagonist nears
    /// (`somnium_ai::dormancy`); `None` when everything was built at once.
    encounters: Option<Dormancy<Encounter>>,
    /// Seconds since the encounters were last tended.
    tended: f32,
    readouts: HashMap<Entity, (i32, i32, i32, &'static str)>,
}

struct Hud {
    canvas: UiCanvas,
    vitals: NodeHandle,
    hotbar: NodeHandle,
    log: NodeHandle,
    shown: [String; 3],
    /// The viewport rectangle (logical x, y, w, h) the panels were laid out in.
    rect: (u32, u32, u32, u32),
    /// Per mark slot, a glyph ("?", "!") at each fade step.
    marks: Vec<[[NodeHandle; 3]; 2]>,
    /// What each slot shows: glyph, fade step, where.
    marks_shown: Vec<Option<(usize, usize, [i32; 2])>>,
}

impl Hud {
    /// Panels placed inside the editor's viewport: in Play the editor keeps
    /// its docks, and the game canvas covers the whole window.
    fn new(rect: (u32, u32, u32, u32)) -> Self {
        use somnium_ui::types::{HorizontalAlignment as H, Thickness, VerticalAlignment as V};
        use somnium_ui::widget::WidgetBuilder;
        use somnium_ui::widgets::{border::BorderBuilder, text::TextBuilder};
        let (vx, vy, vw, vh) = (rect.0 as f32, rect.1 as f32, rect.2 as f32, rect.3 as f32);
        let mut canvas = UiCanvas::new(1280.0, 720.0);
        let font = canvas.font_id();
        let ui = canvas.ui_mut();
        let root = ui.root();
        let panel = |ui: &mut somnium_ui::ui::UserInterface, x: f32, y: f32, w: f32, h: f32| {
            let p = BorderBuilder::new(
                WidgetBuilder::new()
                    .with_width(w.max(40.0))
                    .with_height(h)
                    .with_horizontal_alignment(H::Left)
                    .with_vertical_alignment(V::Top)
                    .with_margin(Thickness {
                        left: x.max(0.0),
                        top: y.max(0.0),
                        right: 0.0,
                        bottom: 0.0,
                    })
                    .with_background([12, 14, 18, 200])
                    .with_foreground(somnium_ui::theme::BORDER_DARK),
            )
            .with_stroke_thickness(Thickness::uniform(1.0))
            .build();
            let p = ui.add_node(p, root);
            let text = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::uniform(8.0)))
                .with_text("")
                .with_font_id(font)
                .with_font_size(13.0)
                .with_wrap(true)
                .with_color(somnium_ui::theme::TEXT_PRIMARY)
                .build();
            ui.add_node(text, p)
        };
        let gap = 10.0;
        let vitals_w = 330.0f32.min(vw * 0.45);
        let vitals = panel(ui, vx + gap, vy + vh - gap - 112.0, vitals_w, 112.0);
        let hotbar_w = (vw - vitals_w - 3.0 * gap).max(120.0);
        let hotbar = panel(
            ui,
            vx + vitals_w + 2.0 * gap,
            vy + vh - gap - 112.0,
            hotbar_w,
            112.0,
        );
        let log_w = 330.0f32.min(vw * 0.5);
        // Below the viewport's own toolbar row.
        let log = panel(ui, vx + vw - gap - log_w, vy + 48.0, log_w, 140.0);
        // Bare glyphs, no box or outline: one node per glyph and fade step.
        let marks = (0..MARKS)
            .map(|_| {
                [("?", MARK_LOOK), ("!", MARK_ALARM)].map(|(glyph, rgb)| {
                    MARK_ALPHA.map(|a| {
                        let mark = TextBuilder::new(WidgetBuilder::new().with_visibility(false))
                            .with_text(glyph)
                            .with_font_id(font)
                            .with_font_size(22.0)
                            .with_color([rgb[0], rgb[1], rgb[2], a])
                            .build();
                        ui.add_node(mark, root)
                    })
                })
            })
            .collect();
        Self {
            canvas,
            vitals,
            hotbar,
            log,
            shown: Default::default(),
            rect,
            marks,
            marks_shown: vec![None; MARKS],
        }
    }

    /// Show each mark (glyph, fade step, centre on screen) and hide the rest,
    /// touching only the slots that changed.
    fn place_marks(&mut self, marks: &[(usize, usize, Vec2)]) {
        let ui = self.canvas.ui_mut();
        for (i, slot) in self.marks.iter().enumerate() {
            let want = marks
                .get(i)
                .map(|&(g, a, p)| (g, a, [p.x.round() as i32, p.y.round() as i32]));
            if self.marks_shown[i] == want {
                continue;
            }
            if let Some((g, a, _)) = self.marks_shown[i] {
                ui.set_visibility(slot[g][a], false);
            }
            if let Some((g, a, [x, y])) = want {
                let rect =
                    somnium_ui::types::Rect::new(x as f32 - 7.0, y as f32 - 14.0, 14.0, 28.0);
                ui.place_node(slot[g][a], rect);
                ui.set_visibility(slot[g][a], true);
            }
            self.marks_shown[i] = want;
        }
    }

    fn set(&mut self, which: usize, text: String) {
        if self.shown[which] != text {
            let node = [self.vitals, self.hotbar, self.log][which];
            self.canvas
                .ui_mut()
                .send(TextMessage::set_text(node, text.clone()));
            self.shown[which] = text;
        }
    }
}

/// The sandbox's runtime.
pub struct CombatDemo {
    run: Option<Run>,
    input: somnium_input::InputSystem,
    reader: CombatInput,
    cursor: Option<Vec2>,
    hud: Option<Hud>,
    /// Ctrl held: the wheel is the camera's, not the measure's.
    ctrl: bool,
}

impl Default for CombatDemo {
    fn default() -> Self {
        let mut input = somnium_input::InputSystem::new();
        input.add_map(combat_map());
        Self {
            run: None,
            input,
            reader: CombatInput::new(),
            cursor: None,
            hud: None,
            ctrl: false,
        }
    }
}

fn world_matrix(world: &World, entity: Entity) -> Mat4 {
    world
        .get::<WorldTransform>(entity)
        .map(|w| w.0)
        .unwrap_or_else(|| {
            world
                .get::<Transform>(entity)
                .map_or(Mat4::IDENTITY, Transform::to_matrix)
        })
}

fn surface_material(surface: Surface) -> &'static str {
    match surface {
        Surface::Bare => "floor",
        Surface::Grass => "grass",
        Surface::Water => "water",
        Surface::Ice => "ice",
        Surface::Snow => "snow",
        Surface::Burning => "fire",
        Surface::Ash => "ash",
        Surface::Stone => "stone",
    }
}

fn element_material(element: Element) -> &'static str {
    match element {
        Element::Flame => "fire",
        Element::Frost => "frost",
        Element::Water => "water",
        Element::Gale => "gale",
        Element::Stone => "stone",
        Element::Mending => "mending",
        Element::Force => "force",
    }
}

fn element_of(kind: DamageType) -> Option<Element> {
    Some(match kind {
        DamageType::Frost => Element::Frost,
        DamageType::Flame => Element::Flame,
        DamageType::Gale => Element::Gale,
        DamageType::Stone => Element::Stone,
        DamageType::Mana => Element::Force,
        _ => return None,
    })
}

fn burst_colour(element: Option<Element>, heal: bool) -> ([f32; 4], [f32; 4]) {
    if heal {
        return ([0.4, 1.0, 0.5, 1.0], [0.1, 0.5, 0.2, 0.0]);
    }
    match element {
        Some(Element::Flame) => ([1.0, 0.55, 0.12, 1.0], [0.4, 0.05, 0.0, 0.0]),
        Some(Element::Frost) => ([0.75, 0.9, 1.0, 1.0], [0.3, 0.5, 0.8, 0.0]),
        Some(Element::Water) => ([0.4, 0.6, 0.9, 1.0], [0.1, 0.2, 0.4, 0.0]),
        Some(Element::Gale) => ([0.85, 0.95, 0.9, 0.8], [0.6, 0.7, 0.7, 0.0]),
        Some(Element::Stone) => ([0.5, 0.45, 0.4, 1.0], [0.2, 0.18, 0.15, 0.0]),
        Some(Element::Force) => ([0.7, 0.55, 1.0, 1.0], [0.3, 0.2, 0.6, 0.0]),
        Some(Element::Mending) => ([0.4, 1.0, 0.5, 1.0], [0.1, 0.5, 0.2, 0.0]),
        None => ([0.7, 0.04, 0.03, 1.0], [0.25, 0.0, 0.0, 0.0]),
    }
}

impl CombatDemo {
    /// Feed the combat map from engine events.
    pub fn event(&mut self, event: &EngineEvent) {
        let devices = self.input.devices_mut();
        match event {
            EngineEvent::KeyInput { key, state } => {
                if let Some(name) = somnium_input::device::key_name(*key) {
                    devices.set_key(name, *state == InputState::Pressed);
                }
                if matches!(
                    key,
                    somnium_core::KeyCode::ControlLeft | somnium_core::KeyCode::ControlRight
                ) {
                    self.ctrl = *state == InputState::Pressed;
                }
            }
            EngineEvent::MouseButton { button, state } => {
                let name = match button {
                    somnium_core::MouseButton::Left => "leftbutton",
                    somnium_core::MouseButton::Right => "rightbutton",
                    somnium_core::MouseButton::Middle => "middlebutton",
                    _ => return,
                };
                devices.set_mouse_button(name, *state == InputState::Pressed);
            }
            // Ctrl+wheel is the camera's zoom (Rimeborn's rig), not the measure.
            EngineEvent::MouseWheel { delta_y } if !self.ctrl => {
                devices.add_scroll(*delta_y as f32);
            }
            EngineEvent::CursorMoved { x, y } => {
                self.cursor = Some(Vec2::new(*x as f32, *y as f32))
            }
            EngineEvent::WindowFocused(false) => {
                devices.release_all();
                self.ctrl = false;
            }
            _ => {}
        }
    }

    /// The sandbox camera's world matrix while a fight runs.
    pub fn view(&self, world: &World) -> Option<Mat4> {
        self.run.as_ref()?.camera.map(|c| world_matrix(world, c))
    }

    /// Build the fight from the scene's authored fighters, ground and wind.
    pub fn begin(&mut self, ctx: &mut EngineContext) {
        somnium_core::propagate_transforms(ctx.world);
        // With a protagonist to come near them, encounters sleep until it
        // does; without one, everything is built at once.
        let protagonist = ctx
            .world
            .iter_with::<CombatantComponent>()
            .any(|(_, c)| c.enabled && c.player);
        let asleep = protagonist.then(|| encounters_asleep(ctx.world));
        let encounters = if protagonist {
            Vec::new()
        } else {
            expand_encounters(ctx.world)
        };
        somnium_core::propagate_transforms(ctx.world);
        let Some(run) = build(ctx.world) else {
            for e in encounters {
                ctx.world.despawn(e);
            }
            return;
        };
        {
            self.run = Some(run);
            let run = self.run.as_mut().expect("just built");
            // Stop takes the encounters away with everything else Play made.
            run.spawned.extend(encounters);
            run.encounters = asleep;
            run.colliders = colliders(ctx);
            stand_on_ground(ctx, run);
            spawn_pools(ctx.world, run);
            group_under_effects(ctx.world, run);
            // Those already near wake now, the rest as the protagonist comes.
            tend_encounters(ctx.world, Some(&mut *ctx.physics), run, f32::INFINITY);
            run.log.push_back(
                "Fight! WASD, click, right click, Space, 1-0, F, C, Q, Tab; E rides a mount, H whistles it; run into walls to climb".into(),
            );
        }
    }

    /// Put everything back as authored.
    pub fn end(&mut self, ctx: &mut EngineContext) {
        let Some(run) = self.run.take() else {
            return;
        };
        for (entity, transform) in run.restore {
            if let Some(t) = ctx.world.get_mut::<Transform>(entity) {
                *t = transform;
            }
        }
        for entity in run.spawned {
            ctx.world.despawn(entity);
        }
        for (_, ragdoll) in run.ragdolls {
            let _ = ctx.physics.destroy_ragdoll(ragdoll);
        }
        for body in run.colliders {
            ctx.physics.destroy_body(body);
        }
        self.input.devices_mut().release_all();
    }

    /// The player's command for the coming fixed steps (once a frame).
    pub fn update(&mut self, ctx: &mut EngineContext) {
        let Some(run) = &mut self.run else {
            return;
        };
        self.input.update(ctx.dt());
        run.world.darkness = darkness(ctx.world);
        let Some(player) = run.player else {
            return;
        };
        let camera = run.camera.map(|c| world_matrix(ctx.world, c));
        let feet = run
            .world
            .combatant(player)
            .map_or(run.floor, |c| c.position.y);
        let aim = camera.zip(self.cursor).and_then(|(camera, cursor)| {
            let (view, _) = somnium_core::camera_view_from_world(camera);
            let (x, y, w, h) = ctx.ui.viewport_physical_rect(1.0);
            let size = Vec2::new(w as f32, h as f32);
            let projection =
                Mat4::perspective_rh(VIEW_FOV.to_radians(), size.x / size.y.max(1.0), 0.1, 1000.0);
            let (o, d) = cursor_ray(
                cursor - Vec2::new(x as f32, y as f32),
                size,
                view,
                projection,
            )?;
            ray_plane(o, d, feet)
        });
        let forward = camera.map_or(Vec3::Z, |m| -m.z_axis.truncate());
        let mut cmd = self.reader.command(
            &self.input,
            ctx.dt(),
            AimView {
                aim,
                camera_forward: forward,
            },
        );
        if run
            .walkers
            .get(&player)
            .is_some_and(|w| w.mantle().is_some())
        {
            // Hands on the ledge: no swing, guard, dodge or verse until up.
            cmd = somnium_combat::Command {
                move_dir: cmd.move_dir,
                sprint: cmd.sprint,
                aim: cmd.aim,
                ..somnium_combat::Command::default()
            };
        }
        run.world.command(player, cmd);
    }

    /// One fixed step of the fight, then everything it changed onto the scene.
    pub fn fixed(&mut self, ctx: &mut EngineContext) {
        let Some(run) = &mut self.run else {
            return;
        };
        let dt = ctx.simulation.fixed_delta_seconds;
        let editor = &ctx.navigation.cells.world;
        let nav = editor
            .cells()
            .next()
            .is_some()
            .then_some(editor)
            .or(run.nav.as_ref());
        let query = JoltQuery(ctx.physics);
        run.world.step(dt, &query, nav);
        let terrain = JoltTerrain {
            physics: ctx.physics,
            water: &run.water,
        };
        for (id, event) in walk(&mut run.world, &mut run.walkers, dt, &terrain) {
            log_locomotion(run, id, event);
        }
        let events = run.world.drain_events();
        for event in &events {
            react(ctx, run, event);
        }
        run.marks.retain(|_, (_, left)| {
            *left -= dt;
            *left > 0.0
        });
        tend_encounters(ctx.world, Some(&mut *ctx.physics), run, dt);
        write_scene(ctx, run);
    }

    /// HUD text (once a frame, before the UI is drawn).
    pub fn render(&mut self, ctx: &mut EngineContext) {
        let Some(run) = &self.run else {
            return;
        };
        let rect = ctx.ui.viewport_physical_rect(1.0);
        if self.hud.as_ref().is_none_or(|h| h.rect != rect) {
            self.hud = Some(Hud::new(rect));
        }
        let hud = self.hud.as_mut().expect("just built");
        let (vitals, hotbar) = player_lines(run);
        hud.place_marks(&mark_places(ctx, run));
        hud.set(0, vitals);
        hud.set(1, hotbar);
        hud.set(2, run.log.iter().cloned().collect::<Vec<_>>().join("\n"));
    }

    /// Draw the HUD.
    pub fn draw_ui(&mut self, frame: &mut GameUiFrame) {
        if self.run.is_some()
            && let Some(hud) = &mut self.hud
        {
            frame.draw(&mut hud.canvas);
        }
    }

    /// For MCP and automation.
    pub fn state(&self) -> Value {
        let Some(run) = &self.run else {
            return json!({"running": false});
        };
        let fighters: Vec<Value> = run
            .world
            .combatants()
            .map(|c| {
                json!({
                    "name": c.name,
                    "team": c.team.0,
                    "health": (c.vitals.health.current * 10.0).round() / 10.0,
                    "state": c.action.label(),
                    "doing": c
                        .brain
                        .as_ref()
                        .map(|b| b.mind.doing)
                        .or_else(|| c.herd.filter(|_| c.rider.is_none()).map(|h| h.doing.label())),
                    "position": c.position.to_array(),
                })
            })
            .collect();
        let locomotion = run
            .player
            .and_then(|p| run.walkers.get(&p))
            .map(|w| match w.mode {
                Mode::Ground => "ground",
                Mode::Air => "air",
                Mode::Mantle(m) if m.kind == MantleKind::Vault => "vault",
                Mode::Mantle(_) => "climb",
                Mode::Swim => "swim",
            });
        json!({
            "running": true,
            "time": run.world.time(),
            "locomotion": locomotion,
            "fighters": fighters,
            "log": run.log,
        })
    }
}

/// The combat world from the scene: fighters, teams, ground, wind.
fn build(world: &World) -> Option<Run> {
    let fighters: Vec<(Entity, CombatantComponent)> = world
        .iter_with::<CombatantComponent>()
        .filter(|(_, c)| c.enabled)
        .map(|(e, c)| (e, c.clone()))
        .collect();
    if fighters.is_empty() {
        return None;
    }
    // The controllers move people (`walk`), as in the game.
    let mut combat = CombatWorld::new(CombatRules {
        integrate_motion: false,
        seed: 0x00C0_FFEE,
        ..CombatRules::default()
    });
    let mut bodies = BTreeMap::new();
    let mut dims = BTreeMap::new();
    let mut restore = Vec::new();
    let mut player = None;
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    let mut floor = f32::NEG_INFINITY;
    let mut ids = Vec::new();
    let mut leaders = Vec::new();
    for (entity, c) in &fighters {
        let (id, feet, size) = enlist(&mut combat, world, *entity, c);
        if c.player && player.is_none() {
            player = Some(id);
        }
        ids.push(id);
        if c.leader {
            leaders.push(id);
        }
        bodies.insert(id, *entity);
        dims.insert(id, size);
        if let Some(t) = world.get::<Transform>(*entity) {
            restore.push((*entity, *t));
        }
        min = min.min(Vec2::new(feet.x, feet.z));
        max = max.max(Vec2::new(feet.x, feet.z));
        floor = floor.max(feet.y);
    }
    if let Some(p) = player {
        let companions: Vec<CombatantId> = combat
            .combatants()
            .filter(|c| {
                c.brain
                    .as_ref()
                    .is_some_and(|b| b.mind.archetype == Archetype::Companion)
            })
            .map(|c| c.id)
            .collect();
        for id in companions {
            if let Some(brain) = combat.combatant_mut(id).and_then(|c| c.brain.as_mut()) {
                brain.mind.leader = Some(p);
            }
        }
    }
    organise(&mut combat, &ids, &leaders);
    // The ground: every authored surface's footprint (axis-aligned), plus wind.
    let surfaces: Vec<(Mat4, CombatSurfaceComponent)> = world
        .iter_with::<CombatSurfaceComponent>()
        .map(|(e, s)| (world_matrix(world, e), *s))
        .collect();
    for (m, s) in &surfaces {
        if s.surface() == Surface::Bare {
            continue;
        }
        let (scale, _, centre) = m.to_scale_rotation_translation();
        let half = Vec2::new(scale.x, scale.z).abs() * 0.5;
        min = min.min(Vec2::new(centre.x, centre.z) - half);
        max = max.max(Vec2::new(centre.x, centre.z) + half);
    }
    let pad = Vec2::splat(12.0);
    combat.environment = EnvironmentField::new(min - pad, max + pad, 0.5);
    if let Some(field) = &mut combat.environment {
        for (m, s) in &surfaces {
            if s.surface() == Surface::Bare {
                continue;
            }
            let (scale, _, centre) = m.to_scale_rotation_translation();
            let c = Vec2::new(centre.x, centre.z);
            let half = Vec2::new(scale.x, scale.z).abs() * 0.5;
            if s.round {
                field.paint_circle(c, half.max_element(), s.surface());
            } else {
                field.paint_rect(c - half, c + half, s.surface());
            }
        }
        for (e, w) in world.iter_with::<WindVolumeComponent>() {
            let centre = world_matrix(world, e).transform_point3(Vec3::ZERO);
            field.winds.push(WindVolume {
                center: centre,
                radius: w.radius.max(0.5),
                velocity: Vec3::new(w.direction.x, 0.0, w.direction.z).normalize_or_zero()
                    * w.strength,
                time_left: f32::INFINITY,
            });
        }
    }
    let camera = world
        .iter_with::<CameraSettingsComponent>()
        .map(|(e, _)| e)
        .find(|e| {
            world
                .get::<Name>(*e)
                .is_some_and(|n| n.as_str() == "Combat Camera")
        });
    let water = surfaces
        .iter()
        .filter(|(_, s)| s.surface() == Surface::Water && !s.solid)
        .map(|(m, s)| {
            let (scale, _, centre) = m.to_scale_rotation_translation();
            Pool {
                centre: Vec2::new(centre.x, centre.z),
                half: Vec2::new(scale.x, scale.z).abs() * 0.5,
                round: s.round,
                line: centre.y + scale.y.abs() * 0.5,
            }
        })
        .collect();
    let walkers = combat
        .combatants()
        .map(|c| {
            (
                c.id,
                Controller::new(walker_config(c), c.position + Vec3::Y * 0.02),
            )
        })
        .collect();
    let floor = if floor.is_finite() { floor } else { 0.0 };
    Some(Run {
        world: combat,
        effects: None,
        bodies,
        dims,
        player,
        walkers,
        nav: bake_nav(world, floor),
        water,
        camera,
        restore,
        colliders: Vec::new(),
        ragdolls: BTreeMap::new(),
        tiles: BTreeMap::new(),
        floor,
        pools: Pools::default(),
        spawned: Vec::new(),
        log: VecDeque::new(),
        marks: BTreeMap::new(),
        encounters: None,
        tended: 0.0,
        readouts: HashMap::new(),
    })
}

/// A fighter entity into a fight: its combatant (feet under the body,
/// facing its Z, the entity as host), and its size as authored.
fn enlist(
    combat: &mut CombatWorld,
    world: &World,
    entity: Entity,
    c: &CombatantComponent,
) -> (CombatantId, Vec3, Vec3) {
    let m = world_matrix(world, entity);
    let feet = m.transform_point3(Vec3::new(0.0, -0.5, 0.0));
    let facing = m.transform_vector3(Vec3::Z);
    let name = world.get::<Name>(entity).map_or("Fighter", |n| n.as_str());
    let host = (u64::from(entity.generation()) << 32) | u64::from(entity.index());
    let id = combat.spawn(c.desc(name, feet, facing, host));
    (id, feet, m.to_scale_rotation_translation().0.abs())
}

/// Newly enlisted fighters (`ids`, of whom `leaders` lead) organised: their
/// teams hostile to the party, each side within 15 m of a leader following
/// it (a pack's alpha, a band's chief) and breaking when it falls, and each
/// raider band formed up, its melee in a line, its bows, rifles and mages
/// behind (knives flank on their own).
fn organise(combat: &mut CombatWorld, ids: &[CombatantId], leaders: &[CombatantId]) {
    let mut teams: Vec<TeamId> = ids
        .iter()
        .filter_map(|id| combat.combatant(*id).map(|c| c.team))
        .collect();
    teams.sort_unstable();
    teams.dedup();
    for &t in &teams {
        if t != TeamId(0) {
            combat.relations.set(TeamId(0), t, Attitude::Hostile);
        }
    }
    let leaders: Vec<(CombatantId, TeamId, Vec3)> = leaders
        .iter()
        .filter_map(|id| combat.combatant(*id).map(|c| (*id, c.team, c.position)))
        .collect();
    let followers: Vec<(CombatantId, CombatantId)> = ids
        .iter()
        .filter_map(|id| combat.combatant(*id))
        .filter(|c| {
            c.brain
                .as_ref()
                .is_some_and(|b| b.mind.archetype != Archetype::Companion)
        })
        .filter_map(|c| {
            leaders
                .iter()
                .filter(|(l, team, at)| {
                    *l != c.id && *team == c.team && flat_distance(*at, c.position) < 15.0
                })
                .min_by(|a, b| {
                    flat_distance(a.2, c.position).total_cmp(&flat_distance(b.2, c.position))
                })
                .map(|(l, _, _)| (c.id, *l))
        })
        .collect();
    for (id, leader) in followers {
        if let Some(brain) = combat.combatant_mut(id).and_then(|c| c.brain.as_mut()) {
            brain.mind.leader = Some(leader);
            brain.mind.profile.breaks_with_leader = true;
        }
    }
    for &t in teams.iter().filter(|t| **t != TeamId(0)) {
        let band: Vec<CombatantId> = ids
            .iter()
            .filter_map(|id| combat.combatant(*id))
            .filter(|c| c.team == t)
            .filter(|c| {
                c.brain
                    .as_ref()
                    .is_some_and(|b| b.mind.archetype != Archetype::Skirmisher)
            })
            .map(|c| c.id)
            .collect();
        ai::form_up(combat, &band);
    }
}

fn flat_distance(a: Vec3, b: Vec3) -> f32 {
    Vec2::new(a.x - b.x, a.z - b.z).length()
}

/// Stand every fighter on the ground under it (or over it): a preset dropped
/// on terrain, whose heightfield the engine has already given to Jolt.
fn stand_on_ground(ctx: &mut EngineContext, run: &mut Run) {
    let ids: Vec<CombatantId> = run.world.combatants().map(|c| c.id).collect();
    stand(ctx.physics, run, &ids);
}

/// [`stand_on_ground`] for some of the fight (a group that has just woken).
fn stand(physics: &somnium_physics::world::PhysicsWorld, run: &mut Run, ids: &[CombatantId]) {
    let query = JoltQuery(physics);
    let radius = 0.2;
    let ground = |from: Vec3, to: Vec3| {
        query
            .sweep(from, to, radius)
            .map(|h| from.y + (to.y - from.y) * h.fraction - radius)
    };
    for &id in ids {
        let Some(p) = run.world.combatant(id).map(|c| c.position) else {
            continue;
        };
        let Some(y) = ground(p + Vec3::Y, p - Vec3::Y * 80.0)
            .or_else(|| ground(p + Vec3::Y * 80.0, p + Vec3::Y))
        else {
            continue;
        };
        if (y - p.y).abs() > 0.05 {
            let feet = Vec3::new(p.x, y, p.z);
            if let Some(c) = run.world.combatant_mut(id) {
                c.position = feet;
            }
            if let Some(w) = run.walkers.get_mut(&id) {
                w.feet = feet + Vec3::Y * 0.02;
            }
        }
    }
}

/// How dark it is: night by the scene's Time of Day (light from about 05:30
/// to 20:30, an hour of dusk each side), or, under a fixed sun (Rimeborn's,
/// which never moves), its eclipse: 0 by day, 1 under the Lid.
fn darkness(world: &World) -> f32 {
    world
        .iter_with::<somnium_core::time_of_day::TimeOfDayComponent>()
        .find(|(_, t)| t.enabled)
        .map_or(0.0, |(_, t)| {
            if t.fixed_sun {
                // A sun that never moves (Rimeborn): its night is the Lid.
                return t.eclipse.clamp(0.0, 1.0);
            }
            let h = t.hour.rem_euclid(24.0);
            1.0 - (h - 5.5f32).clamp(0.0, 1.0).min((20.5 - h).clamp(0.0, 1.0))
        })
}

/// A body's controller: a person's walks, vaults, climbs and swims; a mount's
/// takes its own slopes and steps, swims, and neither vaults nor climbs, so it
/// can be ridden off an edge and fall.
fn walker_config(c: &somnium_combat::Combatant) -> ControllerConfig {
    let mut config = ControllerConfig {
        radius: c.body.radius,
        height: c.body.height,
        ..ControllerConfig::default()
    };
    if let Some(m) = c.mount_def {
        config.max_slope = m.max_slope;
        config.step = m.step;
        config.vault_max = 0.0;
        config.climb_max = 0.0;
        config.swim_depth = m.saddle * 0.85;
    }
    config
}

/// Each body's push out of those it stands inside, exactly as
/// `CombatWorld::separation` gives it (the same terms summed in the same
/// order, so the same bits), but from the bodies in the neighbouring cells
/// only: a call each scans everyone, which a crowd pays for every step.
fn separations(combat: &CombatWorld, ids: &[CombatantId]) -> Vec<(CombatantId, Vec3)> {
    // In id order, as `separation` folds them.
    let others: Vec<_> = combat
        .combatants()
        .filter(|o| o.alive() && o.mount.is_none())
        .collect();
    // Two bodies push only when closer than their radii together.
    let cell = combat
        .combatants()
        .map(|c| c.body.radius)
        .fold(0.0f32, f32::max)
        .mul_add(2.0, 1e-3)
        .max(0.1);
    let key = |p: Vec3| ((p.x / cell).floor() as i32, (p.z / cell).floor() as i32);
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (i, o) in others.iter().enumerate() {
        grid.entry(key(o.position)).or_default().push(i);
    }
    let mut near = Vec::new();
    ids.iter()
        .map(|&id| {
            let Some(me) = combat.combatant(id) else {
                return (id, Vec3::ZERO);
            };
            let (x, z) = key(me.position);
            near.clear();
            for dx in -1..=1 {
                for dz in -1..=1 {
                    let at = (x.saturating_add(dx), z.saturating_add(dz));
                    near.extend(grid.get(&at).into_iter().flatten().copied());
                }
            }
            near.sort_unstable();
            near.dedup();
            let push = near.iter().map(|&i| others[i]).filter(|o| o.id != id).fold(
                Vec3::ZERO,
                |push, o| {
                    let d = somnium_combat::geometry::flat(me.position - o.position);
                    let (len, min) = (d.length(), me.body.radius + o.body.radius);
                    if len < min && len > 1e-4 {
                        push + d / len * (min - len) * 0.5
                    } else {
                        push
                    }
                },
            );
            (id, push)
        })
        .collect()
}

/// Everyone's feet from their controllers, after the combat step decided
/// where each wants to go. Running into a ledge (the run key, or an AI on an
/// off-mesh link) vaults or climbs it; a brain on a level link jumps. Each
/// brain hears whether its body is mid-climb. Mounts walk their own slopes and
/// carry their riders. Returns what each body's locomotion did.
fn walk(
    combat: &mut CombatWorld,
    walkers: &mut BTreeMap<CombatantId, Controller>,
    dt: f32,
    terrain: &impl Terrain,
) -> Vec<(CombatantId, LocomotionEvent)> {
    // Pushes from where everyone stood before anyone moved.
    let ids: Vec<CombatantId> = walkers.keys().copied().collect();
    let pushes = separations(combat, &ids);
    let mut out = Vec::new();
    for (id, push) in pushes {
        let (Some(c), Some(walker)) = (combat.combatant(id), walkers.get_mut(&id)) else {
            continue;
        };
        if c.mount.is_some() || !c.alive() {
            // The mount carries the rider; the dead are the ragdoll's.
            walker.feet = c.position;
            walker.velocity = Vec3::ZERO;
            walker.mode = Mode::Air;
            continue;
        }
        // The body's own clock (focus slows it). On the ground the intent is
        // the velocity, so dodges, lunges and knockback keep their distance.
        let body_dt = dt * combat.time_scale_for(id);
        if body_dt <= 0.0 {
            continue;
        }
        let velocity = c.intent.velocity + push / body_dt;
        if walker.grounded() {
            walker.velocity.x = velocity.x;
            walker.velocity.z = velocity.z;
        }
        // The player climbs on the run key; a brain climbs only what its
        // route means it to (an off-mesh link), and jumps when it asks; a
        // mount never climbs (nor leaps a gap: its rider falls with it).
        let player = c.brain.is_none() && c.mount_def.is_none();
        let input = MoveInput {
            velocity,
            jump: ai::wants_jump(combat, id),
            climb: c.intent.climb || (player && c.intent.sprinting),
        };
        let events = walker.step(body_dt, &input, &Walled { terrain, combat });
        let (feet, velocity) = (walker.feet, walker.velocity);
        let climb = walker.mantle().map(|m| ai::Climb {
            progress: m.progress(),
            vulnerable: m.vulnerable(),
        });
        if let Some(c) = combat.combatant_mut(id) {
            c.position = feet;
            c.velocity = velocity;
        }
        // A hard landing hurts (and throws a mount's rider).
        for event in &events {
            if let LocomotionEvent::Landed { speed } = event {
                combat.landed(id, *speed);
            }
        }
        ai::set_climbing(combat, id, climb);
        out.extend(events.into_iter().map(|e| (id, e)));
    }
    somnium_combat::mount::seat_riders(combat);
    out
}

/// Anyone's vaults and climbs, and the protagonist's swims and hard landings.
fn log_locomotion(run: &mut Run, id: CombatantId, event: LocomotionEvent) {
    let player = run.player == Some(id);
    let line = match event {
        LocomotionEvent::MantleStarted { kind, height, .. } => {
            let verb = match kind {
                MantleKind::Vault => "vaulted",
                MantleKind::Climb => "climbed",
            };
            format!("{} {verb} {height:.1} m", name_of(run, id))
        }
        LocomotionEvent::EnteredWater if player => "Swimming".into(),
        LocomotionEvent::LeftWater if player => "Out of the water".into(),
        LocomotionEvent::Landed { speed } if player && speed > 6.0 => {
            format!("Hard landing ({speed:.1} m/s)")
        }
        _ => return,
    };
    log(run, line);
}

/// The sandbox's navmesh: the solid boxes baked at a quarter metre (the bake
/// erodes by the agent's radius twice, so half-metre voxels close the gap
/// between the low wall and the ledge), and the scene's Navigation Links.
/// Only ground a body could climb to counts, so the pillars' tops do not.
fn bake_nav(world: &World, floor: f32) -> Option<NavWorld> {
    let mut triangles = Vec::new();
    let (mut min, mut max) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
    for (e, s) in world.iter_with::<CombatSurfaceComponent>() {
        if !s.solid {
            continue;
        }
        let m = world_matrix(world, e);
        // Corner `i` has x, y, z from its bits 0, 1, 2.
        let c: [Vec3; 8] = std::array::from_fn(|i| {
            let bit = |b: usize| if i & b == 0 { -0.5 } else { 0.5 };
            m.transform_point3(Vec3::new(bit(1), bit(2), bit(4)))
        });
        for p in c {
            min = min.min(p);
            max = max.max(p);
        }
        for [i, j, k, l] in [
            [0, 1, 3, 2],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 3, 7, 6],
            [0, 2, 6, 4],
            [1, 3, 7, 5],
        ] {
            triangles.push([c[i], c[j], c[k]].map(|v| v.to_array()));
            triangles.push([c[i], c[k], c[l]].map(|v| v.to_array()));
        }
    }
    let climb = ControllerConfig::default().climb_max;
    let tile = somnium_ai::navigation::bake(&BakeInput {
        cell: [0, 0, 0],
        bounds: Bounds {
            min: [min.x, floor - 0.1, min.z],
            max: [max.x, floor + climb + 0.2, max.z],
        },
        triangles,
        obstacles: Vec::new(),
        settings: BakeSettings {
            voxel_size: 0.25,
            agent_radius: 0.35,
            ..BakeSettings::default()
        },
    })
    .ok()?;
    let mut nav = NavWorld::default();
    nav.install(tile);
    for (e, l) in world.iter_with::<NavigationLink>() {
        if l.enabled {
            let _ = nav.set_link(OffMeshLink {
                id: (u64::from(e.generation()) << 32) | u64::from(e.index()),
                start: world_matrix(world, e).transform_point3(Vec3::ZERO),
                end: l.destination,
                bidirectional: l.bidirectional,
                radius: l.attachment_radius,
                cost: l.extra_cost,
            });
        }
    }
    Some(nav)
}

/// Static Jolt boxes for solid surfaces (the floor, pillars), so ragdolls
/// land and the combat query sees them.
fn colliders(ctx: &mut EngineContext) -> Vec<BodyId> {
    let solids: Vec<Mat4> = ctx
        .world
        .iter_with::<CombatSurfaceComponent>()
        .filter(|(_, s)| s.solid)
        .map(|(e, _)| world_matrix(ctx.world, e))
        .collect();
    solid_bodies(ctx.physics, solids)
}

/// Static Jolt boxes for solid surfaces' matrices.
fn solid_bodies(
    physics: &mut somnium_physics::world::PhysicsWorld,
    solids: Vec<Mat4>,
) -> Vec<BodyId> {
    let mut made = Vec::new();
    for m in solids {
        let (scale, rotation, translation) = m.to_scale_rotation_translation();
        let body = physics.create_body(RigidBodyDescriptor {
            shape: ColliderShape::Box {
                half_extents: (scale.abs() * 0.5).max(Vec3::splat(0.01)),
            },
            position: translation,
            rotation,
            motion_type: MotionType::Static,
            object_layer: somnium_physics::layer::LAYER_NON_MOVING,
            ..RigidBodyDescriptor::default()
        });
        if body.is_valid() {
            made.push(body);
        }
    }
    made
}

fn pooled(world: &mut World, run: &mut Run, name: String, kind: MeshKind, mat: &str) -> Entity {
    let e = world.spawn((
        Name::new(&name),
        Transform::from_translation(PARKED),
        WorldTransform::identity(),
        kind,
        material(mat),
    ));
    run.spawned.push(e);
    run.pools.looks.insert(e, Look::Hidden);
    e
}

fn spawn_pools(world: &mut World, run: &mut Run) {
    let effects = world.spawn((
        Name::new("Combat (runtime)"),
        Transform::default(),
        WorldTransform::identity(),
    ));
    run.spawned.push(effects);
    run.effects = Some(effects);
    for i in 0..PROJECTILES {
        let e = pooled(
            world,
            run,
            format!("Combat missile {i}"),
            MeshKind::Sphere,
            "dark",
        );
        run.pools.projectiles.push((e, "dark"));
    }
    for i in 0..WALLS {
        let e = pooled(
            world,
            run,
            format!("Combat wall {i}"),
            MeshKind::Cube,
            "stone",
        );
        run.pools.walls.push((e, "stone"));
    }
    for i in 0..BLOOMS * RING_BEADS {
        let e = pooled(
            world,
            run,
            format!("Combat bloom {i}"),
            MeshKind::Sphere,
            "fire",
        );
        run.pools.blooms.push((e, "fire"));
    }
    for i in 0..WAVE_BEADS * WAVES {
        let e = pooled(
            world,
            run,
            format!("Combat wave {i}"),
            MeshKind::Sphere,
            "gale",
        );
        run.pools.beads.push(e);
    }
    for i in 0..FIRES {
        let e = world.spawn((
            Name::new(&format!("Combat fire {i}")),
            Transform::from_translation(PARKED),
            WorldTransform::identity(),
            ParticleEmitter {
                enabled: false,
                additive: true,
                max_particles: 40,
                spawn_rate: 16.0,
                lifetime: 0.8,
                initial_speed: 0.9,
                spread_angle: 0.35,
                size_start: 0.45,
                size_end: 0.08,
                gravity: -1.6,
                spawn_extents: [0.35, 0.0, 0.35],
                color_over_life: somnium_ecs::curve::Gradient::ramp(
                    [1.0, 0.55, 0.12, 1.0],
                    [0.6, 0.05, 0.0, 0.0],
                ),
                ..ParticleEmitter::default()
            },
        ));
        run.spawned.push(e);
        run.pools.fires.push((e, false));
    }
    for i in 0..BURSTS {
        let e = world.spawn((
            Name::new(&format!("Combat impact {i}")),
            Transform::from_translation(PARKED),
            WorldTransform::identity(),
            ParticleEmitter {
                additive: true,
                max_particles: 48,
                spawn_rate: 0.0,
                lifetime: 0.45,
                initial_speed: 3.0,
                spread_angle: 1.4,
                size_start: 0.16,
                size_end: 0.02,
                gravity: 4.0,
                ..ParticleEmitter::default()
            },
        ));
        run.spawned.push(e);
        run.pools.bursts.push(e);
    }
    for i in 0..DECALS {
        let e = world.spawn((
            Name::new(&format!("Combat mark {i}")),
            Transform::from_translation(PARKED),
            WorldTransform::identity(),
            somnium_core::decal::DecalComponent {
                multiply: true,
                roughness: 0.9,
                ..Default::default()
            },
            material("ash"),
        ));
        run.spawned.push(e);
        run.pools.decals.push(e);
    }
    // Ground tiles: one per two-by-two cells that is not bare.
    let Some(field) = &run.world.environment else {
        return;
    };
    let (w, d) = field.dimensions();
    let cell = field.cell_size();
    let mut tiles = Vec::new();
    for tz in 0..d.div_ceil(2) {
        for tx in 0..w.div_ceil(2) {
            let s = tile_surface(field, tx, tz);
            if s != Surface::Bare {
                tiles.push((tx, tz, s));
            }
        }
    }
    let floor = run.floor;
    for (tx, tz, s) in tiles {
        let c = field.center(tx * 2, tz * 2) + Vec2::splat(cell * 0.5);
        let e = world.spawn((
            Name::new(&format!("Combat ground {tx},{tz}")),
            Transform {
                translation: Vec3::new(c.x, floor + 0.03, c.y),
                scale: Vec3::new(cell * 2.0 - 0.03, 0.02, cell * 2.0 - 0.03),
                ..Default::default()
            },
            WorldTransform::identity(),
            MeshKind::Cube,
            material(surface_material(s)),
        ));
        run.spawned.push(e);
        run.tiles.insert((tx, tz), (e, s));
    }
}

/// Hang every runtime entity under the one root (at the origin, so local is world).
fn group_under_effects(world: &mut World, run: &Run) {
    let Some(root) = run.effects else {
        return;
    };
    for &e in &run.spawned {
        if e != root {
            let _ = world.insert_component(e, Parent { entity: root });
        }
    }
}

/// What a two-by-two tile shows: the most telling of its cells.
fn tile_surface(field: &EnvironmentField, tx: u32, tz: u32) -> Surface {
    let rank = |s: Surface| match s {
        Surface::Burning => 7,
        Surface::Ice => 6,
        Surface::Ash => 5,
        Surface::Water => 4,
        Surface::Snow => 3,
        Surface::Grass => 2,
        Surface::Stone => 1,
        Surface::Bare => 0,
    };
    let (w, d) = field.dimensions();
    let mut best = Surface::Bare;
    for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        let (x, z) = (tx * 2 + dx, tz * 2 + dz);
        if x >= w || z >= d {
            continue;
        }
        let c = field.center(x, z);
        let s = field.surface_at(Vec3::new(c.x, 0.0, c.y));
        if rank(s) > rank(best) {
            best = s;
        }
    }
    best
}

fn name_of(run: &Run, id: CombatantId) -> String {
    run.world
        .combatant(id)
        .map_or_else(|| "someone".into(), |c| c.name.clone())
}

fn log(run: &mut Run, line: String) {
    run.log.push_back(line);
    while run.log.len() > LOG_LINES {
        run.log.pop_front();
    }
}

fn burst(
    world: &mut World,
    run: &mut Run,
    at: Vec3,
    element: Option<Element>,
    heal: bool,
    count: u32,
) {
    let Some(&e) = run.pools.bursts.get(run.pools.next_burst % BURSTS) else {
        return;
    };
    run.pools.next_burst += 1;
    let (a, b) = burst_colour(element, heal);
    if let Some(t) = world.get_mut::<Transform>(e) {
        t.translation = at;
    }
    if let Some(p) = world.get_mut::<ParticleEmitter>(e) {
        p.color_over_life = somnium_ecs::curve::Gradient::ramp(a, b);
        p.burst = count;
    }
}

fn mark(world: &mut World, run: &mut Run, at: Vec3, size: f32, mat: &str) {
    let Some(&e) = run.pools.decals.get(run.pools.next_decal % DECALS) else {
        return;
    };
    run.pools.next_decal += 1;
    let mut t = somnium_core::decal::placement(at, Vec3::Y);
    t.scale = Vec3::new(size, 0.5, size);
    if let Some(slot) = world.get_mut::<Transform>(e) {
        *slot = t;
    }
    if let Some(m) = world.get_mut::<MaterialComponent>(e) {
        *m = material(mat);
    }
}

/// Turn one event into presentation: ragdolls, impacts, marks, the log.
fn react(ctx: &mut EngineContext, run: &mut Run, event: &CombatEvent) {
    match event {
        CombatEvent::Damaged {
            target,
            source,
            amount,
            kind,
            point,
            critical,
            flags,
        } => {
            if flags.contains(HitFlags::TICK) {
                return;
            }
            burst(
                ctx.world,
                run,
                *point,
                element_of(*kind),
                false,
                if *critical { 40 } else { 18 },
            );
            let by = source.map_or_else(|| "the ground".to_string(), |s| name_of(run, s));
            let crit = if *critical { " critical" } else { "" };
            log(
                run,
                format!(
                    "{by} > {}: {:.0} {}{crit}",
                    name_of(run, *target),
                    amount,
                    kind.name().to_lowercase()
                ),
            );
        }
        CombatEvent::Healed { target, amount, .. } => {
            if let Some(c) = run.world.combatant(*target) {
                let at = c.chest();
                burst(ctx.world, run, at, None, true, 24);
            }
            log(
                run,
                format!("{} healed {:.0}", name_of(run, *target), amount),
            );
        }
        CombatEvent::Parried {
            defender,
            attacker,
            point,
            perfect,
        } => {
            burst(ctx.world, run, *point, Some(Element::Force), false, 30);
            let noon = if *perfect {
                " at the instant (Noon)"
            } else {
                ""
            };
            log(
                run,
                format!(
                    "{} parried {}{noon}",
                    name_of(run, *defender),
                    name_of(run, *attacker)
                ),
            );
        }
        CombatEvent::Blocked { target, point, .. } => {
            burst(ctx.world, run, *point, Some(Element::Stone), false, 10);
            let _ = target;
        }
        CombatEvent::GuardBroken { target } => {
            log(run, format!("{}'s guard broke", name_of(run, *target)))
        }
        CombatEvent::Reflected { defender, .. } => log(
            run,
            format!("{} turned a bolt back", name_of(run, *defender)),
        ),
        CombatEvent::Shattered { target } => {
            log(run, format!("{} shattered", name_of(run, *target)))
        }
        CombatEvent::Feinted { attacker } => {
            log(run, format!("{} feinted", name_of(run, *attacker)))
        }
        CombatEvent::SpellReleased {
            caster, spell, at, ..
        } => {
            let name = run
                .world
                .spell(spell)
                .map_or(spell.clone(), |s| s.name.clone());
            log(run, format!("{}: {name}", name_of(run, *caster)));
            let _ = at;
        }
        CombatEvent::SpellRefused {
            caster,
            spell,
            error,
        } if Some(*caster) == run.player => {
            log(run, format!("{spell} refused: {error:?}"));
        }
        CombatEvent::SpellInterrupted { caster, .. } => {
            log(run, format!("{}'s verse broke", name_of(run, *caster)))
        }
        CombatEvent::Overdrawn { debt, .. } => log(
            run,
            format!("Overdrawn: {debt:.1} lamps on the Wheel's credit"),
        ),
        CombatEvent::Alerted { by, .. } => {
            log(run, format!("{} raised the alarm", name_of(run, *by)))
        }
        CombatEvent::Fled { who } => log(run, format!("{} broke and ran", name_of(run, *who))),
        CombatEvent::Whistled { who, mount } => {
            let line = match mount {
                Some(m) => format!(
                    "{} whistles; the {} comes",
                    name_of(run, *who),
                    name_of(run, *m)
                ),
                None => format!("{} whistles; nothing answers", name_of(run, *who)),
            };
            log(run, line);
        }
        CombatEvent::Fell { who, speed, .. } => log(
            run,
            format!("{} fell hard ({speed:.1} m/s)", name_of(run, *who)),
        ),
        CombatEvent::Kindled { who, on } if Some(*who) == run.player => {
            log(
                run,
                format!("Kindled Body {}", if *on { "on" } else { "off" }),
            );
        }
        CombatEvent::Focus { active, .. } => log(
            run,
            format!("Focus {}", if *active { "on: time slows" } else { "off" }),
        ),
        CombatEvent::PartBroken { target, part } => {
            log(run, format!("{}'s {part} broke", name_of(run, *target)))
        }
        CombatEvent::ProjectileEnded {
            point,
            normal,
            target: None,
            element,
            ..
        } => {
            if *normal != Vec3::ZERO {
                burst(ctx.world, run, *point, *element, false, 12);
                if normal.y > 0.5 {
                    match element {
                        Some(Element::Flame) => mark(ctx.world, run, *point, 1.6, "scorch"),
                        Some(Element::Frost) => mark(ctx.world, run, *point, 1.6, "frostmark"),
                        _ => {}
                    }
                }
            }
        }
        CombatEvent::Bloomed {
            at,
            radius,
            element,
        } => {
            burst(
                ctx.world,
                run,
                *at + Vec3::Y * 0.5,
                Some(*element),
                false,
                48,
            );
            match element {
                Element::Flame => mark(ctx.world, run, *at, radius * 2.4, "scorch"),
                Element::Frost => mark(ctx.world, run, *at, radius * 2.4, "frostmark"),
                _ => {}
            }
        }
        CombatEvent::Died {
            target,
            killer,
            impulse,
            point,
        } => {
            let by = killer.map_or_else(String::new, |k| format!(" to {}", name_of(run, k)));
            log(run, format!("{} fell{by}", name_of(run, *target)));
            ragdoll(ctx, run, *target, *impulse, *point);
        }
        CombatEvent::Surface(change) => {
            let key = (change.x / 2, change.z / 2);
            if let (Some((entity, shown)), Some(field)) =
                (run.tiles.get_mut(&key), &run.world.environment)
            {
                let now = tile_surface(field, key.0, key.1);
                if now != *shown {
                    *shown = now;
                    if let Some(m) = ctx.world.get_mut::<MaterialComponent>(*entity) {
                        *m = material(surface_material(now));
                    }
                }
            }
        }
        CombatEvent::Noticed {
            watcher,
            target,
            level,
            ..
        } => notice(run, *watcher, *target, *level),
        _ => {}
    }
}

/// A brain's awareness of the protagonist changed: a "?" for a look, a "!"
/// for the alarm, gone when it forgets. Others noticing others show nothing.
fn notice(
    run: &mut Run,
    watcher: CombatantId,
    target: CombatantId,
    level: somnium_combat::ai::Awareness,
) {
    if Some(target) != run.player {
        return;
    }
    if level == somnium_combat::ai::Awareness::Unaware {
        run.marks.remove(&watcher);
    } else {
        run.marks.insert(watcher, (level, MARK_SECONDS));
    }
}

/// [`marks_on_screen`] through the fight's camera and the editor's viewport.
fn mark_places(ctx: &EngineContext, run: &Run) -> Vec<(usize, usize, Vec2)> {
    match run.camera {
        Some(c) => marks_on_screen(
            run,
            world_matrix(ctx.world, c),
            ctx.ui.viewport_physical_rect(1.0),
        ),
        None => Vec::new(),
    }
}

/// Where each mark goes on screen (the viewport's x, y, width, height):
/// over the watcher's head through `camera`, its glyph ("?" 0, "!" 1) and
/// fade step; none off screen or behind the camera.
fn marks_on_screen(
    run: &Run,
    camera: Mat4,
    (x, y, w, h): (u32, u32, u32, u32),
) -> Vec<(usize, usize, Vec2)> {
    let size = Vec2::new(w as f32, h as f32);
    let (view, _) = somnium_core::camera_view_from_world(camera);
    let projection =
        Mat4::perspective_rh(VIEW_FOV.to_radians(), size.x / size.y.max(1.0), 0.1, 1000.0);
    let to_clip = projection * view;
    run.marks
        .iter()
        .filter_map(|(id, (level, left))| {
            let c = run.world.combatant(*id).filter(|c| c.alive())?;
            let head = c.position + Vec3::Y * (c.body.height + 0.35);
            let clip = to_clip * head.extend(1.0);
            if clip.w <= 0.0 {
                return None;
            }
            let ndc = clip.truncate() / clip.w;
            if ndc.x.abs() > 1.0 || ndc.y.abs() > 1.0 {
                return None;
            }
            let glyph = usize::from(*level == somnium_combat::ai::Awareness::Alerted);
            let fade = if *left > 1.0 {
                0
            } else if *left > 0.5 {
                1
            } else {
                2
            };
            let at = Vec2::new(
                x as f32 + (ndc.x * 0.5 + 0.5) * size.x,
                y as f32 + (0.5 - ndc.y * 0.5) * size.y,
            );
            Some((glyph, fade, at))
        })
        .take(MARKS)
        .collect()
}

/// A fallen fighter becomes a two-part Jolt ragdoll pushed by the blow.
fn ragdoll(ctx: &mut EngineContext, run: &mut Run, id: CombatantId, impulse: Vec3, point: Vec3) {
    let Some(c) = run.world.combatant(id) else {
        return;
    };
    if c.mount_def.is_some() {
        // A fallen mount lies where it fell; its rider was thrown.
        return;
    }
    let feet = c.position;
    let rotation = yaw(c.facing);
    let torso = feet + Vec3::Y * BODY_HEIGHT * 0.5;
    let neck = feet + Vec3::Y * BODY_HEIGHT;
    let parts = [
        somnium_physics::animation::RagdollPart {
            parent: None,
            position: torso,
            rotation,
            half_height: BODY_HEIGHT * 0.5 - BODY_WIDTH * 0.5,
            radius: BODY_WIDTH * 0.5,
            anchor: torso,
            swing_limit: 0.0,
            twist_limit: 0.0,
        },
        somnium_physics::animation::RagdollPart {
            parent: Some(0),
            position: neck + Vec3::Y * HEAD * 0.5,
            rotation,
            half_height: 0.02,
            radius: HEAD * 0.5,
            anchor: neck,
            swing_limit: 0.6,
            twist_limit: 0.4,
        },
    ];
    let Ok(ragdoll) = ctx.physics.create_ragdoll(&parts) else {
        return;
    };
    if let Ok(bodies) = ctx.physics.ragdoll_bodies(ragdoll) {
        let body = bodies[0];
        // A little up and the blow's own push, at the torso.
        let push = impulse.clamp_length_max(400.0) + Vec3::Y * 60.0;
        ctx.physics.add_impulse(body, push);
        let _ = point;
    }
    run.ragdolls.insert(id, ragdoll);
}

fn write_look(world: &mut World, looks: &mut HashMap<Entity, Look>, entity: Entity, look: Look) {
    let changed = looks.get(&entity).is_none_or(|old| match (old, look) {
        (Look::Hidden, Look::Hidden) => false,
        (Look::Shown(a, b, c), Look::Shown(x, y, z)) => {
            a.distance_squared(x) > 1e-6 || b.dot(y).abs() < 0.99999 || c.distance_squared(z) > 1e-6
        }
        _ => true,
    });
    if !changed {
        return;
    }
    looks.insert(entity, look);
    if let Some(t) = world.get_mut::<Transform>(entity) {
        match look {
            Look::Hidden => t.translation = PARKED,
            Look::Shown(p, r, s) => {
                t.translation = p;
                t.rotation = r;
                t.scale = s;
            }
        }
    }
}

fn set_material(world: &mut World, entity: Entity, slot: &mut &'static str, want: &'static str) {
    if *slot != want {
        *slot = want;
        if let Some(m) = world.get_mut::<MaterialComponent>(entity) {
            *m = material(want);
        }
    }
}

/// Fighters, ragdolls, missiles, walls, blooms, waves, fire and readouts.
fn write_scene(ctx: &mut EngineContext, run: &mut Run) {
    let world = &mut *ctx.world;
    // Fighters: world pose into the body's parent space.
    for (id, entity) in &run.bodies {
        let Some(c) = run.world.combatant(*id) else {
            continue;
        };
        let size =
            run.dims
                .get(id)
                .copied()
                .unwrap_or(Vec3::new(BODY_WIDTH, BODY_HEIGHT, BODY_WIDTH));
        let pose = match run
            .ragdolls
            .get(id)
            .and_then(|r| ctx.physics.ragdoll_pose(*r).ok())
        {
            Some(parts) => {
                let (_, r, p) = parts[0].to_scale_rotation_translation();
                (p, r)
            }
            // A mount's long body lies along its slope; people stand upright.
            None => {
                let up = run
                    .walkers
                    .get(id)
                    .filter(|w| c.mount_def.is_some() && w.grounded())
                    .map_or(Vec3::Y, |w| w.ground_normal);
                (
                    c.position + up * size.y * 0.5,
                    Quat::from_rotation_arc(Vec3::Y, up) * yaw(c.facing),
                )
            }
        };
        let parent = world
            .get::<Parent>(*entity)
            .map(|p| p.entity)
            .filter(|p| *p != Entity::DANGLING)
            .map_or(Mat4::IDENTITY, |p| world_matrix(world, p));
        let local = parent.inverse() * Mat4::from_scale_rotation_translation(size, pose.1, pose.0);
        let (s, r, p) = local.to_scale_rotation_translation();
        if let Some(t) = world.get_mut::<Transform>(*entity)
            && (t.translation.distance_squared(p) > 1e-6 || t.rotation.dot(r).abs() < 0.99999)
        {
            t.translation = p;
            t.rotation = r;
            t.scale = s;
        }
        // Live readouts, rounded so regeneration does not write every step.
        let state = c
            .herd
            .filter(|_| c.rider.is_none() && c.alive())
            .map_or(c.action.label(), |h| h.doing.label());
        let reading = (
            (c.vitals.health.current * 10.0).round() as i32,
            (c.vitals.stamina.current).round() as i32,
            (c.vitals.mana.current * 10.0).round() as i32,
            state,
        );
        if run.readouts.get(entity) != Some(&reading) {
            run.readouts.insert(*entity, reading);
            if let Some(cc) = world.get_mut::<CombatantComponent>(*entity) {
                cc.health = c.vitals.health.current;
                cc.stamina = c.vitals.stamina.current;
                cc.mana = c.vitals.mana.current;
                cc.state = state.to_string();
            }
        }
    }
    let pools = &mut run.pools;
    // Missiles.
    let flying = run.world.projectiles();
    for (i, (entity, slot)) in pools.projectiles.iter_mut().enumerate() {
        let look = flying.get(i).map(|p| {
            let (scale, mat) = match (p.def.kind, p.spell.as_ref().map(|s| s.element)) {
                (ProjectileKind::Spell, Some(e)) => (
                    Vec3::splat((p.def.radius * 2.0).max(0.2)),
                    element_material(e),
                ),
                (ProjectileKind::WindCut, _) => (Vec3::new(0.9, 0.06, 0.25), "gale"),
                (ProjectileKind::Round, _) => (Vec3::splat(0.06), "dark"),
                _ => (
                    Vec3::new(0.05, 0.05, 0.7),
                    if p.burning { "fire" } else { "dark" },
                ),
            };
            (Look::Shown(p.position, yaw(p.velocity), scale), mat)
        });
        if let Some((_, mat)) = look {
            set_material(world, *entity, slot, mat);
        }
        write_look(
            world,
            &mut pools.looks,
            *entity,
            look.map_or(Look::Hidden, |l| l.0),
        );
    }
    // Walls.
    let walls = run.world.walls();
    for (i, (entity, slot)) in pools.walls.iter_mut().enumerate() {
        let look = walls.get(i).map(|w| {
            (
                Look::Shown(w.obb.center, w.obb.rotation, w.obb.half * 2.0),
                element_material(w.element),
            )
        });
        if let Some((_, mat)) = look {
            set_material(world, *entity, slot, mat);
        }
        write_look(
            world,
            &mut pools.looks,
            *entity,
            look.map_or(Look::Hidden, |l| l.0),
        );
    }
    // Blooms gathering: a ring of beads on the ground where it will burst.
    let blooms = run.world.blooms();
    for (i, (entity, slot)) in pools.blooms.iter_mut().enumerate() {
        let (ring, bead) = (i / RING_BEADS, i % RING_BEADS);
        let look = blooms.get(ring).map(|b| {
            let a = bead as f32 / RING_BEADS as f32 * std::f32::consts::TAU;
            let at = Vec3::new(
                b.center.x + a.cos() * b.radius,
                run.floor + 0.12,
                b.center.z + a.sin() * b.radius,
            );
            (
                Look::Shown(at, Quat::IDENTITY, Vec3::splat(0.22)),
                element_material(b.element),
            )
        });
        if let Some((_, mat)) = look {
            set_material(world, *entity, slot, mat);
        }
        write_look(
            world,
            &mut pools.looks,
            *entity,
            look.map_or(Look::Hidden, |l| l.0),
        );
    }
    // Waves: beads along each front.
    let waves = run.world.waves();
    for (i, entity) in pools.beads.iter().enumerate() {
        let (w, k) = (i / WAVE_BEADS, i % WAVE_BEADS);
        let look = waves.get(w).map_or(Look::Hidden, |wave| {
            let a = -wave.half_angle + 2.0 * wave.half_angle * k as f32 / (WAVE_BEADS - 1) as f32;
            let p =
                wave.origin + Quat::from_rotation_y(-a) * wave.dir * wave.radius + Vec3::Y * 0.9;
            Look::Shown(p, Quat::IDENTITY, Vec3::splat(0.35))
        });
        write_look(world, &mut pools.looks, *entity, look);
    }
    // Fire on the ground.
    let burning: Vec<Vec2> = run
        .world
        .environment
        .as_ref()
        .map(|f| {
            let all: Vec<Vec2> = f.burning().collect();
            let stride = (all.len() / FIRES).max(1);
            all.into_iter().step_by(stride).take(FIRES).collect()
        })
        .unwrap_or_default();
    for (i, (entity, lit)) in pools.fires.iter_mut().enumerate() {
        let at = burning
            .get(i)
            .map(|c| Vec3::new(c.x, run.floor + 0.05, c.y));
        let want = at.is_some();
        if let Some(at) = at {
            write_look(
                world,
                &mut pools.looks,
                *entity,
                Look::Shown(at, Quat::IDENTITY, Vec3::ONE),
            );
        }
        if *lit != want {
            *lit = want;
            if let Some(p) = world.get_mut::<ParticleEmitter>(*entity) {
                p.enabled = want;
            }
        }
    }
}

fn player_lines(run: &Run) -> (String, String) {
    let Some(c) = run.player.and_then(|p| run.world.combatant(p)) else {
        return ("No player-controlled fighter".into(), String::new());
    };
    let v = &c.vitals;
    let mut flags = Vec::new();
    if c.kindled {
        flags.push("Kindled".to_string());
    }
    if let Some(t) = c.lock.and_then(|t| run.world.combatant(t)) {
        flags.push(format!("Lock: {}", t.name));
    }
    if v.overdrawn() {
        flags.push("Overdrawn".into());
    }
    if let Some(w) = c.ward {
        flags.push(format!("Ward {:.0}", w.strength));
    }
    if run.world.focusing() == Some(c.id) {
        flags.push("Focus".into());
    }
    for kind in StatusKind::ALL {
        if c.status.active(kind) {
            flags.push(match kind {
                StatusKind::Chill | StatusKind::Bleed => {
                    format!("{} {:.0}", kind.name(), c.status.buildup(kind))
                }
                _ => kind.name().to_string(),
            });
        }
    }
    let state = if matches!(c.action, Action::Dead) {
        "Fallen"
    } else {
        c.action.label()
    };
    let vitals = format!(
        "{} - {state}\nHealth {:.0}/{:.0}   Stamina {:.0}/{:.0}   Poise {:.0}\nMana {:.1}/{:.0} lamps   Focus {:.0}\n{}",
        c.name,
        v.health.current,
        v.health.max,
        v.stamina.current,
        v.stamina.max,
        v.poise.current,
        v.mana.current,
        v.mana.max,
        v.focus.current,
        flags.join(" | ")
    );
    let now = run.world.time();
    let slots: Vec<String> = c
        .magic
        .hotbar
        .iter()
        .enumerate()
        .filter_map(|(i, slot)| {
            let slot = slot.as_ref()?;
            let name = run
                .world
                .spell(&slot.spell)
                .map_or(slot.spell.clone(), |s| s.name.clone());
            let key = if i == 9 { 0 } else { i + 1 };
            let cooling = c
                .cooldowns
                .get(&slot.spell)
                .map_or(0.0, |r| (r - now).max(0.0));
            Some(if cooling > 0.0 {
                format!("{key} {name} ({cooling:.1}s)")
            } else {
                format!("{key} {name}")
            })
        })
        .collect();
    let shaping = c.shaping.as_ref().map_or_else(
        || "Hold a number to shape: the mouse aims, the wheel edits the measure, R picks which".to_string(),
        |s| {
            let m = s.request.measure;
            let size = rote_name(m.size, &ROTE_SIZE).map_or_else(|| format!("{:.2} m", m.size), |r| format!("{r} ({:.2} m)", m.size));
            let axis = match s.axis {
                MeasureAxis::Size => "size",
                MeasureAxis::Range => "range",
                MeasureAxis::Speed => "speed",
                MeasureAxis::Count => "count",
                MeasureAxis::Duration => "duration",
            };
            let cost = match &s.preview {
                Ok(cost) => format!("{:.1} lamps, {:.1} s", cost.mana, cost.gather),
                Err(e) => format!("refused: {e:?}"),
            };
            format!(
                "{} {:?}: size {size}, range {:.0} m, speed {:.0} m/s, x{} | wheel: {axis} | {cost}",
                run.world.spell(&s.request.spell).map_or(s.request.spell.as_str(), |a| a.name.as_str()),
                s.request.shape,
                m.range,
                m.speed,
                m.count,
            )
        },
    );
    (vitals, format!("{}\n{shaping}", slots.join("   ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every material the sandbox names exists, and the round marks point at
    /// the smoke sprite by the engine's own asset id.
    #[test]
    fn the_sandbox_materials_load_and_the_marks_are_round() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/materials/combat");
        for name in [
            "floor",
            "party",
            "raider",
            "skin",
            "dark",
            "water",
            "ice",
            "grass",
            "snow",
            "ash",
            "stone",
            "fire",
            "gale",
            "force",
            "mending",
            "frost",
            "hide",
            "scorch",
            "frostmark",
        ] {
            let asset = somnium_asset::material::load_material(dir.join(format!("{name}.sommat")))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            if matches!(name, "scorch" | "frostmark") {
                assert_eq!(
                    asset.albedo_map,
                    AssetId::from_relative_path(std::path::Path::new(
                        "textures/particles/smoke_01.png"
                    )),
                    "{name} uses the soft round sprite"
                );
            }
        }
    }

    /// The boxes `colliders` gives Jolt, and the basin's water.
    fn boxes(world: &World, run: &Run) -> somnium_character::boxes::Boxes {
        use somnium_character::boxes::{Boxes, Solid, Water};
        let mut terrain = Boxes::default();
        for (e, s) in world.iter_with::<CombatSurfaceComponent>() {
            if s.solid {
                let (scale, rotation, center) =
                    world_matrix(world, e).to_scale_rotation_translation();
                terrain.solids.push(Solid {
                    center,
                    half: scale.abs() * 0.5,
                    rotation,
                });
            }
        }
        for w in run.water.iter().filter(|w| !w.round) {
            terrain.water.push(Water {
                min: (w.centre - w.half).to_array(),
                max: (w.centre + w.half).to_array(),
                surface: w.line,
            });
        }
        terrain
    }

    /// Ridden at a gallop, the frost-elk takes the slope up the bluff on its
    /// own controller, runs off the far edge, falls 6 m and is hurt; its rider
    /// holds on (a horse's fall would need to be far longer to throw them).
    #[test]
    fn the_elk_takes_the_slope_and_falls_off_the_bluff() {
        let mut world = World::new();
        spawn_sandbox(&mut world, &json!({})).expect("preset builds");
        let others: Vec<Entity> = world
            .iter_with::<CombatantComponent>()
            .filter(|(e, _)| {
                world
                    .get::<Name>(*e)
                    .is_none_or(|n| !matches!(n.as_str(), "Protagonist" | "Frost-elk"))
            })
            .map(|(e, _)| e)
            .collect();
        for e in others {
            world.get_mut::<CombatantComponent>(e).unwrap().enabled = false;
        }
        somnium_core::propagate_transforms(&mut world);
        let mut run = build(&world).expect("a fight");
        let terrain = boxes(&world, &run);
        let rider = run.player.expect("the protagonist");
        let elk = run
            .world
            .combatants()
            .find(|c| c.name == "Frost-elk")
            .map(|c| c.id)
            .expect("the elk");
        let floor = run.floor;
        for (id, feet) in [
            (elk, Vec3::new(-16.0, floor, 12.5)),
            (rider, Vec3::new(-14.8, floor, 12.5)),
        ] {
            let c = run.world.combatant_mut(id).unwrap();
            c.position = feet;
            c.facing = -Vec3::Z;
            let walker = run.walkers.get_mut(&id).unwrap();
            walker.feet = feet + Vec3::Y * 0.02;
            walker.mode = Mode::Air;
        }
        assert_eq!(
            somnium_combat::mount::mount(&mut run.world, rider, elk),
            Ok(())
        );
        let full = run.world.combatant(elk).unwrap().vitals.health.current;
        let open = somnium_combat::OpenField;
        let (mut top, mut fell) = (f32::MIN, false);
        for _ in 0..8 * 60 {
            run.world.command(
                rider,
                somnium_combat::Command {
                    move_dir: -Vec3::Z,
                    sprint: true,
                    ..Default::default()
                },
            );
            run.world.step(1.0 / 60.0, &open, run.nav.as_ref());
            walk(&mut run.world, &mut run.walkers, 1.0 / 60.0, &terrain);
            fell |= run
                .world
                .drain_events()
                .iter()
                .any(|e| matches!(e, CombatEvent::Fell { who, .. } if *who == elk));
            let at = run.world.combatant(elk).unwrap().position;
            top = top.max(at.y - floor);
            if fell && run.walkers[&elk].grounded() {
                break;
            }
        }
        // The fall's harm resolves on the next step.
        run.world.command(rider, somnium_combat::Command::default());
        run.world.step(1.0 / 60.0, &open, run.nav.as_ref());
        walk(&mut run.world, &mut run.walkers, 1.0 / 60.0, &terrain);
        let at = run.world.combatant(elk).unwrap().position;
        assert!(top > 5.9, "up the slope onto the bluff: {top}");
        assert!(fell, "and off its edge: {at}");
        assert!(
            at.z < -6.0 && (at.y - floor).abs() < 0.1,
            "landed below: {at}"
        );
        assert!(run.world.combatant(elk).unwrap().vitals.health.current < full);
        assert_eq!(
            run.world.combatant(rider).unwrap().mount,
            Some(elk),
            "held on"
        );
        let seat = run.world.combatant(rider).unwrap().position;
        assert!(
            (seat - at).length() < 2.5,
            "and rode it down: {seat} vs {at}"
        );
    }

    /// Combat Wilds: the pack follows its alpha and the band its chief (both
    /// break when their leader falls), the horse grazes, and nobody has
    /// noticed the protagonist yet.
    #[test]
    fn the_wilds_have_a_pack_a_band_and_a_horse_none_of_them_aware() {
        let mut world = World::new();
        spawn_wilds(&mut world, &json!({})).expect("preset builds");
        assert!(claims_play(&world));
        somnium_core::propagate_transforms(&mut world);
        let mut run = build(&world).expect("a fight");
        let id = |name: &str| {
            run.world
                .combatants()
                .find(|c| c.name == name)
                .map(|c| c.id)
                .unwrap_or_else(|| panic!("{name}"))
        };
        let (alpha, chief, horse, hero) = (
            id("Rime wolf (alpha)"),
            id("Bandit chief"),
            id("Horse"),
            id("Protagonist"),
        );
        for c in run.world.combatants() {
            let Some(brain) = &c.brain else { continue };
            let expected = if c.name.starts_with("Rime wolf ") && c.id != alpha {
                Some(alpha)
            } else if c.name.starts_with("Bandit (") {
                Some(chief)
            } else {
                None
            };
            assert_eq!(brain.mind.leader, expected, "{}", c.name);
            if expected.is_some() {
                assert!(brain.mind.profile.breaks_with_leader, "{}", c.name);
            }
        }
        let wolf = run.world.combatant(alpha).unwrap();
        assert!(wolf.body.radius < 0.5, "a wolf's body");
        assert!(
            run.world.combatant(horse).unwrap().herd.is_some(),
            "the horse grazes"
        );
        assert_eq!(darkness(&world), 0.0, "no Time of Day: day");
        let terrain = boxes(&world, &run);
        let open = somnium_combat::OpenField;
        let mut events = Vec::new();
        for _ in 0..4 * 60 {
            run.world.step(1.0 / 60.0, &open, run.nav.as_ref());
            walk(&mut run.world, &mut run.walkers, 1.0 / 60.0, &terrain);
            events.extend(run.world.drain_events());
        }
        let noticed = events
            .iter()
            .any(|e| matches!(e, CombatEvent::Noticed { target, .. } if *target == hero));
        assert!(!noticed, "nobody has noticed the protagonist: {events:?}");
        for c in run.world.combatants().filter(|c| c.brain.is_some()) {
            assert!(
                somnium_combat::ai::awareness(&run.world, c.id, hero) < 0.35,
                "{}",
                c.name
            );
        }
    }

    /// The preset builds a fight the combat world accepts, and the fight runs
    /// headless through the same `build` Play uses.
    #[test]
    fn the_course_is_vaulted_climbed_and_swum() {
        let mut world = World::new();
        spawn_sandbox(&mut world, &json!({})).expect("preset builds");
        somnium_core::propagate_transforms(&mut world);
        let run = build(&world).expect("a fight");
        let terrain = boxes(&world, &run);
        let mut walker = run.walkers[&run.player.expect("a protagonist")].clone();
        let go = |walker: &mut Controller, velocity: Vec3, seconds: f32| {
            let input = MoveInput {
                velocity,
                jump: false,
                climb: true,
            };
            let mut events = Vec::new();
            for _ in 0..(seconds * 60.0) as usize {
                events.extend(walker.step(1.0 / 60.0, &input, &terrain));
            }
            events
        };
        let kinds = |events: &[LocomotionEvent]| -> Vec<MantleKind> {
            events
                .iter()
                .filter_map(|e| match e {
                    LocomotionEvent::MantleStarted { kind, .. } => Some(*kind),
                    _ => None,
                })
                .collect()
        };
        // East over the low wall and up the ledge.
        // The preset stands on its root, 5 cm up.
        walker.feet = Vec3::new(6.0, run.floor + 0.02, -5.0);
        walker.mode = Mode::Air;
        let events = go(&mut walker, Vec3::X * 4.0, 3.0);
        assert_eq!(
            kinds(&events),
            [MantleKind::Vault, MantleKind::Climb],
            "{events:?}"
        );
        assert!(
            walker.grounded() && (walker.feet.y - run.floor - 2.0).abs() < 0.05,
            "{:?} {:?}",
            walker.feet,
            walker.mode
        );
        // South over the basin's wall, into deep water, across and out.
        walker.feet = Vec3::new(12.5, run.floor + 0.02, -6.5);
        walker.mode = Mode::Air;
        // (Until it stands outside: the arena's edge is not far beyond.)
        let mut events = Vec::new();
        for _ in 0..600 {
            events.extend(go(&mut walker, Vec3::Z * -3.5, 1.0 / 60.0));
            if events.contains(&LocomotionEvent::LeftWater)
                && walker.grounded()
                && (walker.feet.y - run.floor).abs() < 0.05
            {
                break;
            }
        }
        assert!(
            events.contains(&LocomotionEvent::EnteredWater),
            "{events:?}"
        );
        assert!(events.contains(&LocomotionEvent::LeftWater), "{events:?}");
        assert_eq!(kinds(&events).len(), 2, "over the wall and out: {events:?}");
        assert!(
            walker.grounded() && walker.feet.z < -13.0 && (walker.feet.y - run.floor).abs() < 0.05,
            "{:?} {:?} {events:?}",
            walker.feet,
            walker.mode
        );
    }

    #[test]
    fn the_sandbox_builds_and_fights_without_a_window() {
        let mut world = World::new();
        let spawned = spawn_sandbox(&mut world, &json!({})).expect("preset builds");
        assert!(spawned.len() > 20);
        assert!(claims_play(&world));
        somnium_core::propagate_transforms(&mut world);
        let mut run = build(&world).expect("a fight");
        assert_eq!(run.world.combatants().count(), 10);
        let elk = run
            .world
            .combatants()
            .find(|c| c.name == "Frost-elk")
            .expect("the elk");
        assert!(elk.mount_def.is_some() && elk.brain.is_none());
        assert!(run.player.is_some());
        assert!(run.camera.is_some());
        let field = run.world.environment.as_ref().expect("ground");
        let census = field.census();
        assert!(census[Surface::Water as usize] > 50 && census[Surface::Grass as usize] > 100);
        let feet = run.world.combatant(run.player.unwrap()).unwrap().position;
        assert!(feet.y.abs() < 0.1, "feet on the floor: {feet}");
        assert!(run.nav.is_some(), "the course baked");
        // Every body has a controller, the elk too.
        assert_eq!(run.walkers.len(), 10);
        let terrain = boxes(&world, &run);
        let mut events = Vec::new();
        for _ in 0..(20.0 / (1.0 / 60.0)) as usize {
            let open = somnium_combat::OpenField;
            run.world.step(1.0 / 60.0, &open, run.nav.as_ref());
            walk(&mut run.world, &mut run.walkers, 1.0 / 60.0, &terrain);
            events.extend(run.world.drain_events());
        }
        let harmed = events
            .iter()
            .filter(|e| matches!(e, CombatEvent::Damaged { .. }))
            .count();
        let cast = events
            .iter()
            .filter(|e| matches!(e, CombatEvent::SpellReleased { .. }))
            .count();
        assert!(harmed > 10, "they fought: {harmed} hits");
        assert!(cast > 0, "the mage cast");
        let (vitals, hotbar) = player_lines(&run);
        assert!(vitals.contains("Health") && hotbar.contains("Ice Lance"));
    }

    /// The axe raider alone against the protagonist, the raider's feet at
    /// `from` and the target's at `to` (above the floor): the navmesh's route
    /// takes `links` off-mesh links, and within thirty headless seconds of
    /// the same `build` and `walk` as Play (on the boxes `colliders` hands
    /// Jolt) the raider stands within 2 m of the target at its height.
    /// Returns the raider's locomotion.
    fn chase(from: Vec3, to: Vec3, links: usize) -> Vec<LocomotionEvent> {
        let mut world = World::new();
        spawn_sandbox(&mut world, &json!({})).expect("preset builds");
        let others: Vec<Entity> = world
            .iter_with::<CombatantComponent>()
            .filter(|(e, _)| {
                world
                    .get::<Name>(*e)
                    .is_none_or(|n| !matches!(n.as_str(), "Protagonist" | "Raider (axe)"))
            })
            .map(|(e, _)| e)
            .collect();
        for e in others {
            world.get_mut::<CombatantComponent>(e).unwrap().enabled = false;
        }
        somnium_core::propagate_transforms(&mut world);
        let mut run = build(&world).expect("a fight");
        let terrain = boxes(&world, &run);
        let target = run.player.expect("the protagonist");
        let raider = run
            .world
            .combatants()
            .find(|c| c.name == "Raider (axe)")
            .map(|c| c.id)
            .expect("the raider");
        let (from, to) = (from + Vec3::Y * run.floor, to + Vec3::Y * run.floor);
        for (id, feet) in [(target, to), (raider, from)] {
            run.world.combatant_mut(id).unwrap().position = feet;
            let walker = run.walkers.get_mut(&id).unwrap();
            walker.feet = feet + Vec3::Y * 0.02;
            walker.mode = Mode::Air;
        }
        let route = run
            .nav
            .as_ref()
            .expect("the course baked")
            .path(from, to, 0.7);
        assert_eq!(
            route.map(|p| p.links.len()),
            Some(links),
            "the route's links"
        );
        ai::alert(&mut run.world, raider, target, to);
        let open = somnium_combat::OpenField;
        let (mut events, mut trail) = (Vec::new(), Vec::new());
        for step in 0..30 * 60 {
            run.world.step(1.0 / 60.0, &open, run.nav.as_ref());
            let moved = walk(&mut run.world, &mut run.walkers, 1.0 / 60.0, &terrain);
            events.extend(
                moved
                    .into_iter()
                    .filter(|(id, _)| *id == raider)
                    .map(|(_, e)| e),
            );
            run.world.drain_events();
            let r = run.world.combatant(raider).unwrap().position;
            let t = run.world.combatant(target).unwrap().position;
            if step % 60 == 0 {
                trail.push(r);
            }
            if (r.y - t.y).abs() < 0.3 && Vec2::new(r.x - t.x, r.z - t.z).length() < 2.0 {
                return events;
            }
        }
        panic!("the raider went {trail:?}; {events:?}");
    }

    fn mantles(events: &[LocomotionEvent]) -> Vec<MantleKind> {
        events
            .iter()
            .filter_map(|e| match e {
                LocomotionEvent::MantleStarted { kind, .. } => Some(*kind),
                _ => None,
            })
            .collect()
    }

    /// Running flat out into the ledge, the player climbs it (the run key
    /// is its climb) and a raider does not: a brain climbs only where its
    /// route says to.
    #[test]
    fn a_running_raider_climbs_only_on_purpose() {
        let mut world = World::new();
        spawn_sandbox(&mut world, &json!({})).expect("preset builds");
        somnium_core::propagate_transforms(&mut world);
        let mut run = build(&world).expect("a fight");
        let terrain = boxes(&world, &run);
        let raider = run
            .world
            .combatants()
            .find(|c| c.name == "Raider (axe)")
            .map(|c| c.id)
            .expect("the raider");
        for (id, climbs) in [
            (raider, false),
            (run.player.expect("the protagonist"), true),
        ] {
            let feet = Vec3::new(10.9, run.floor, -5.0);
            run.world.combatant_mut(id).unwrap().position = feet;
            let walker = run.walkers.get_mut(&id).unwrap();
            walker.feet = feet + Vec3::Y * 0.02;
            walker.mode = Mode::Air;
            let mut climbed = false;
            for _ in 0..60 {
                let c = run.world.combatant_mut(id).unwrap();
                c.intent.velocity = Vec3::X * 6.0;
                c.intent.sprinting = true;
                c.intent.climb = false;
                c.intent.rooted = false;
                let moved = walk(&mut run.world, &mut run.walkers, 1.0 / 60.0, &terrain);
                climbed |= moved.iter().any(|(who, e)| {
                    *who == id && matches!(e, LocomotionEvent::MantleStarted { .. })
                });
            }
            assert_eq!(climbed, climbs, "{}", run.world.combatant(id).unwrap().name);
            run.world.combatant_mut(id).unwrap().position = Vec3::new(0.0, run.floor, 30.0);
            run.walkers.get_mut(&id).unwrap().feet = Vec3::new(0.0, run.floor, 30.0);
        }
    }

    /// From west of the low wall to a target on the 2 m ledge: over the wall
    /// (jumped at the link's start: a 1 m wall is cleared by the jump, or
    /// vaulted if the jump meets it low) and up the ledge.
    #[test]
    fn a_raider_vaults_and_climbs_to_a_target_on_the_ledge() {
        let events = chase(Vec3::new(5.0, 0.0, -5.0), Vec3::new(13.0, 2.0, -5.0), 2);
        let kinds = mantles(&events);
        assert!(
            events.contains(&LocomotionEvent::Jumped) || kinds.contains(&MantleKind::Vault),
            "over the wall: {events:?}"
        );
        assert_eq!(
            kinds.last(),
            Some(&MantleKind::Climb),
            "up the ledge: {events:?}"
        );
    }

    /// Up the ledge and across the 2 m gap to a target on the far ledge.
    #[test]
    fn a_raider_leaps_the_gap_to_a_target_on_the_far_ledge() {
        let events = chase(Vec3::new(10.3, 0.0, -5.0), Vec3::new(18.5, 2.0, -5.0), 2);
        assert_eq!(mantles(&events), [MantleKind::Climb], "{events:?}");
        assert!(events.contains(&LocomotionEvent::Jumped), "{events:?}");
    }

    /// Through the walled basin to a target beyond it, shorter than going
    /// round: over the north wall into the water, a swim, and out the south.
    #[test]
    fn a_raider_swims_the_basin_to_a_target_beyond_it() {
        let events = chase(Vec3::new(12.5, 0.0, -7.4), Vec3::new(12.5, 0.0, -16.5), 2);
        assert!(
            events.contains(&LocomotionEvent::EnteredWater),
            "{events:?}"
        );
        assert!(events.contains(&LocomotionEvent::LeftWater), "{events:?}");
        assert_eq!(mantles(&events).len(), 2, "in and out: {events:?}");
    }

    /// Under a fixed sun (Rimeborn's, which never moves) the dark is the
    /// Lid's eclipse, whatever the hour; otherwise the hour decides.
    #[test]
    fn a_fixed_sun_takes_its_dark_from_the_eclipse() {
        use somnium_core::time_of_day::TimeOfDayComponent;
        let mut world = World::new();
        let sky = world.spawn((
            Name::new("Sky"),
            TimeOfDayComponent {
                enabled: true,
                hour: 12.0,
                ..TimeOfDayComponent::default()
            },
        ));
        assert_eq!(darkness(&world), 0.0, "noon");
        world.get_mut::<TimeOfDayComponent>(sky).unwrap().hour = 0.0;
        assert_eq!(darkness(&world), 1.0, "midnight");
        let t = world.get_mut::<TimeOfDayComponent>(sky).unwrap();
        t.fixed_sun = true;
        t.eclipse = 0.0;
        assert_eq!(darkness(&world), 0.0, "day under a fixed sun, at midnight");
        world.get_mut::<TimeOfDayComponent>(sky).unwrap().eclipse = 0.8;
        assert!(
            (darkness(&world) - 0.8).abs() < 1e-6,
            "most of the way under the Lid"
        );
    }

    /// A Combat Encounter in a scene: Play builds its horse, wolves and
    /// bandits where the entity stands (no second camera or protagonist),
    /// and they join the fight.
    #[test]
    fn a_scene_encounter_is_built_where_it_stands() {
        let mut world = World::new();
        let at = Vec3::new(40.0, 3.0, -20.0);
        world.spawn((
            Name::new("Camp"),
            Transform::from_translation(at),
            WorldTransform::identity(),
            CombatEncounterComponent::default(),
        ));
        assert!(claims_play(&world), "an encounter alone claims Play");
        somnium_core::propagate_transforms(&mut world);
        let made = expand_encounters(&mut world);
        somnium_core::propagate_transforms(&mut world);
        let names: Vec<String> = made
            .iter()
            .filter_map(|e| world.get::<Name>(*e).map(|n| n.as_str().to_string()))
            .collect();
        assert!(
            !names
                .iter()
                .any(|n| n == "Combat Camera" || n == "Protagonist"),
            "{names:?}"
        );
        assert!(names.iter().any(|n| n == "Horse"), "{names:?}");
        assert!(names.iter().any(|n| n == "Bandit chief"), "{names:?}");
        let run = build(&world).expect("a fight");
        assert!(
            run.player.is_none(),
            "the scene's protagonist, not the encounter's"
        );
        let wolves: Vec<Vec3> = run
            .world
            .combatants()
            .filter(|c| c.name.starts_with("Rime wolf"))
            .map(|c| c.position)
            .collect();
        assert_eq!(wolves.len(), 5);
        assert!(
            wolves
                .iter()
                .all(|p| p.distance(at) < 40.0 && (p.y - at.y).abs() < 1.0),
            "{wolves:?}"
        );
        // Off, or of an unknown kind, it builds nothing.
        let mut off = World::new();
        off.spawn((
            Name::new("Nothing here"),
            Transform::default(),
            WorldTransform::identity(),
            CombatEncounterComponent {
                kind: "dragons".into(),
                ..CombatEncounterComponent::default()
            },
        ));
        somnium_core::propagate_transforms(&mut off);
        assert!(expand_encounters(&mut off).is_empty());
    }

    /// An encounter turned a quarter turn lays its pack and camp out turned
    /// with it, awake at Play or woken later.
    #[test]
    fn an_encounter_turns_with_its_entity() {
        let at = Vec3::new(10.0, 0.0, -40.0);
        let turn = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let den = at + turn * Vec3::new(14.0, 0.0, 18.0);
        let scene = |hero: bool| {
            let mut world = World::new();
            if hero {
                let root = world.spawn((
                    Name::new("Scene"),
                    Transform::default(),
                    WorldTransform::identity(),
                ));
                fighter(
                    &mut world,
                    root,
                    "Protagonist",
                    at,
                    Vec3::X,
                    CombatantComponent::protagonist(),
                );
            }
            world.spawn((
                Name::new("Turned"),
                Transform {
                    translation: at,
                    rotation: turn,
                    ..Default::default()
                },
                WorldTransform::identity(),
                CombatEncounterComponent::default(),
            ));
            somnium_core::propagate_transforms(&mut world);
            world
        };
        let alpha = |run: &Run| {
            run.world
                .combatants()
                .find(|c| c.name == "Rime wolf (alpha)")
                .map(|c| c.position)
                .expect("the alpha")
        };
        // Built at Play.
        let mut world = scene(false);
        expand_encounters(&mut world);
        somnium_core::propagate_transforms(&mut world);
        let run = build(&world).expect("a fight");
        assert!(alpha(&run).distance(den) < 0.5, "{} not {den}", alpha(&run));
        // Woken from sleep.
        let mut world = scene(true);
        let mut run = build(&world).expect("a fight");
        run.encounters = Some(encounters_asleep(&world));
        tend_encounters(&mut world, None, &mut run, f32::INFINITY);
        assert!(alpha(&run).distance(den) < 0.5, "{} not {den}", alpha(&run));
        // Asleep and woken twice where nobody went: the camp stays put.
        let hero = run.player.expect("the protagonist");
        let go = |run: &mut Run, world: &mut World, p: Vec3| {
            run.world.combatant_mut(hero).unwrap().position = p;
            tend_encounters(world, None, run, f32::INFINITY);
        };
        for _ in 0..2 {
            go(&mut run, &mut world, at + Vec3::X * 400.0);
            go(&mut run, &mut world, at);
        }
        assert!(
            alpha(&run).distance(den) < 0.5,
            "drifted to {}",
            alpha(&run)
        );
        // Everyone ten metres east before it sleeps: it wakes ten metres east.
        let ids: Vec<CombatantId> = run
            .world
            .combatants()
            .filter(|c| c.id != hero)
            .map(|c| c.id)
            .collect();
        for id in ids {
            run.world.combatant_mut(id).unwrap().position += Vec3::X * 10.0;
        }
        go(&mut run, &mut world, at + Vec3::X * 400.0);
        go(&mut run, &mut world, at);
        let east = den + Vec3::X * 10.0;
        assert!(
            alpha(&run).distance(east) < 0.5,
            "{} not {east}",
            alpha(&run)
        );
    }

    /// The host's pushes for everyone at once are `separation`'s, to the
    /// bit, in a packed and partly fallen crowd with a horse in it (if
    /// `separation` changes, this says so).
    #[test]
    fn everyone_is_pushed_as_separation_pushes_them() {
        let mut world = World::new();
        let root = world.spawn((
            Name::new("Scene"),
            Transform::default(),
            WorldTransform::identity(),
        ));
        let mut seed = 0x5851_f42d_4c95_7f2du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f32 / (1u64 << 53) as f32
        };
        for i in 0..240 {
            let at = Vec3::new(next() * 14.0 - 7.0, 0.0, next() * 14.0 - 7.0);
            fighter(
                &mut world,
                root,
                &format!("Body {i}"),
                at,
                Vec3::Z,
                CombatantComponent::default(),
            );
        }
        world.spawn((
            Name::new("Horse"),
            Transform {
                translation: Vec3::new(0.5, 0.8, 0.5),
                scale: Vec3::new(0.85, 1.6, 2.4),
                ..Default::default()
            },
            WorldTransform::identity(),
            Parent { entity: root },
            CombatantComponent {
                mount: 1,
                ..CombatantComponent::default()
            },
        ));
        somnium_core::propagate_transforms(&mut world);
        let mut run = build(&world).expect("a fight");
        let ids: Vec<CombatantId> = run.world.combatants().map(|c| c.id).collect();
        for &id in ids.iter().step_by(7) {
            run.world.combatant_mut(id).unwrap().action = Action::Dead;
        }
        let mut pushed = 0;
        for (id, push) in separations(&run.world, &ids) {
            assert_eq!(push, run.world.separation(id), "{id:?}");
            pushed += usize::from(push != Vec3::ZERO);
        }
        assert!(pushed > 100, "a crowd that pushes: {pushed}");
    }

    /// What the host's walkers cost on Jolt ground: bodies standing idle
    /// (as a woken camp at rest) and the same bodies walking. Run with
    /// `cargo test --release -p hello_engine --bin hello_engine walker_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement"]
    fn walker_cost() {
        for count in [100usize, 400] {
            let mut world = World::new();
            let root = world.spawn((
                Name::new("Scene"),
                Transform::default(),
                WorldTransform::identity(),
            ));
            let side = (count as f32).sqrt().ceil() as usize;
            for i in 0..count {
                let at = Vec3::new((i % side) as f32 * 4.0, 0.0, (i / side) as f32 * 4.0);
                fighter(
                    &mut world,
                    root,
                    &format!("Body {i}"),
                    at,
                    Vec3::Z,
                    CombatantComponent::default(),
                );
            }
            somnium_core::propagate_transforms(&mut world);
            let mut run = build(&world).expect("a fight");
            let mut physics = somnium_physics::world::PhysicsWorld::new(Default::default());
            let extent = side as f32 * 4.0 + 20.0;
            solid_bodies(
                &mut physics,
                vec![Mat4::from_scale_rotation_translation(
                    Vec3::new(extent, 1.0, extent),
                    Quat::IDENTITY,
                    Vec3::new(extent * 0.5 - 10.0, -0.5, extent * 0.5 - 10.0),
                )],
            );
            let terrain = JoltTerrain {
                physics: &physics,
                water: &[],
            };
            let dt = 1.0 / 60.0;
            for _ in 0..30 {
                walk(&mut run.world, &mut run.walkers, dt, &terrain);
            }
            let time = |run: &mut Run| {
                let start = std::time::Instant::now();
                for _ in 0..60 {
                    walk(&mut run.world, &mut run.walkers, dt, &terrain);
                }
                start.elapsed().as_secs_f64() * 1e3 / 60.0
            };
            let idle = time(&mut run);
            let ids: Vec<CombatantId> = run.world.combatants().map(|c| c.id).collect();
            for id in ids {
                run.world.combatant_mut(id).unwrap().intent.velocity = Vec3::X * 1.5;
            }
            let walking = time(&mut run);
            println!("{count} bodies: idle {idle:.3} ms a step, walking {walking:.3} ms a step");
        }
    }

    /// A pack and a camp placed apart (as across Tyffir): each builds only
    /// itself, round its own entity.
    #[test]
    fn a_pack_and_a_camp_are_encounters_of_their_own() {
        let mut world = World::new();
        let (den, camp) = (Vec3::new(-60.0, 2.0, 10.0), Vec3::new(80.0, 1.0, -30.0));
        for (name, at, kind) in [("Den", den, "wolves"), ("Camp", camp, "Bandits")] {
            world.spawn((
                Name::new(name),
                Transform::from_translation(at),
                WorldTransform::identity(),
                CombatEncounterComponent {
                    kind: kind.into(),
                    ..CombatEncounterComponent::default()
                },
            ));
        }
        somnium_core::propagate_transforms(&mut world);
        expand_encounters(&mut world);
        somnium_core::propagate_transforms(&mut world);
        let run = build(&world).expect("a fight");
        let near = |prefix: &str, at: Vec3| {
            let found: Vec<Vec3> = run
                .world
                .combatants()
                .filter(|c| c.name.starts_with(prefix))
                .map(|c| c.position)
                .collect();
            assert!(
                found.iter().all(|p| p.distance(at) < 6.0),
                "{prefix}: {found:?}"
            );
            found.len()
        };
        assert_eq!(near("Rime wolf", den), 5);
        assert_eq!(near("Bandit", camp), 4);
        assert_eq!(
            run.world.combatants().count(),
            9,
            "no horse, no protagonist"
        );
        assert!(
            world
                .iter_with::<Name>()
                .all(|(_, n)| !n.as_str().starts_with("Boulder"))
        );
        assert!(encounter_extent("wolves") < encounter_extent("wilds"));
    }

    /// "?" over a raider who has seen the protagonist, "!" over one who is
    /// alerted, fading at the end; nothing for raiders noticing someone else,
    /// and gone when forgotten.
    #[test]
    fn marks_show_who_has_noticed_the_protagonist() {
        use somnium_combat::ai::Awareness;
        let mut world = World::new();
        spawn_sandbox(&mut world, &json!({})).expect("preset builds");
        somnium_core::propagate_transforms(&mut world);
        let mut run = build(&world).expect("a fight");
        let hero = run.player.expect("the protagonist");
        let raiders: Vec<CombatantId> = run
            .world
            .combatants()
            .filter(|c| c.name.starts_with("Raider ("))
            .map(|c| c.id)
            .take(3)
            .collect();
        notice(&mut run, raiders[0], hero, Awareness::Suspicious);
        notice(&mut run, raiders[1], hero, Awareness::Alerted);
        notice(&mut run, raiders[2], raiders[0], Awareness::Alerted);
        run.marks.get_mut(&raiders[1]).unwrap().1 = 0.3;
        let camera = world_matrix(&world, run.camera.expect("the camera"));
        let screen = (0, 0, 1280, 720);
        let shown = marks_on_screen(&run, camera, screen);
        assert_eq!(shown.len(), 2, "{shown:?}");
        assert_eq!((shown[0].0, shown[0].1), (0, 0), "a full ?");
        assert_eq!((shown[1].0, shown[1].1), (1, 2), "a fading !");
        assert!(
            shown
                .iter()
                .all(|(_, _, p)| (0.0..1280.0).contains(&p.x) && (0.0..720.0).contains(&p.y)),
            "{shown:?}"
        );
        notice(&mut run, raiders[0], hero, Awareness::Unaware);
        assert_eq!(marks_on_screen(&run, camera, screen).len(), 1);
    }

    /// Ctrl and the wheel are the camera's zoom (Rimeborn's rig): the
    /// measure is left alone; the plain wheel still edits it.
    #[test]
    fn ctrl_and_the_wheel_leave_the_measure_alone() {
        let mut demo = CombatDemo::default();
        let wheel = |demo: &mut CombatDemo| {
            demo.event(&EngineEvent::MouseWheel { delta_y: 1.0 });
            demo.input.update(1.0 / 60.0);
            demo.input.axis("MeasureWheel")
        };
        let ctrl = |demo: &mut CombatDemo, state| {
            demo.event(&EngineEvent::KeyInput {
                key: somnium_core::KeyCode::ControlLeft,
                state,
            });
        };
        ctrl(&mut demo, InputState::Pressed);
        assert_eq!(wheel(&mut demo), 0.0, "Ctrl held");
        ctrl(&mut demo, InputState::Released);
        assert!(wheel(&mut demo) != 0.0, "the plain wheel");
    }

    /// A Combat Encounter 300 m off sleeps (no bodies, no brains) while the
    /// protagonist is far; it wakes into the fight when the protagonist
    /// comes near, sleeps again when it leaves, remembering who fell, and
    /// wakes with only the survivors.
    #[test]
    fn a_far_camp_sleeps_until_the_protagonist_nears_and_remembers_its_dead() {
        let mut world = World::new();
        let root = world.spawn((
            Name::new("Scene"),
            Transform::default(),
            WorldTransform::identity(),
        ));
        fighter(
            &mut world,
            root,
            "Protagonist",
            Vec3::ZERO,
            Vec3::X,
            CombatantComponent::protagonist(),
        );
        world.spawn((
            Name::new("Camp"),
            Transform::from_translation(Vec3::new(300.0, 0.0, 0.0)),
            WorldTransform::identity(),
            CombatEncounterComponent {
                floor: true,
                ..CombatEncounterComponent::default()
            },
        ));
        somnium_core::propagate_transforms(&mut world);
        let mut run = build(&world).expect("a fight");
        run.encounters = Some(encounters_asleep(&world));
        let hero = run.player.expect("the protagonist");
        let go = |run: &mut Run, world: &mut World, x: f32| {
            run.world.combatant_mut(hero).unwrap().position = Vec3::new(x, 0.0, 0.0);
            tend_encounters(world, None, run, f32::INFINITY);
        };
        let count = |run: &Run, prefix: &str| {
            run.world
                .combatants()
                .filter(|c| c.name.starts_with(prefix))
                .count()
        };
        let named = |world: &World, prefix: &str| {
            world
                .iter_with::<Name>()
                .filter(|(_, n)| n.as_str().starts_with(prefix))
                .count()
        };
        let bodies = |world: &World, prefix: &str| {
            world
                .iter_with::<CombatantComponent>()
                .filter(|(e, _)| {
                    world
                        .get::<Name>(*e)
                        .is_some_and(|n| n.as_str().starts_with(prefix))
                })
                .count()
        };
        go(&mut run, &mut world, 0.0);
        assert_eq!(run.world.combatants().count(), 1, "asleep 300 m off");
        assert_eq!(named(&world, "Rime wolf"), 0, "no bodies while it sleeps");
        go(&mut run, &mut world, 200.0);
        assert_eq!(count(&run, "Rime wolf"), 5, "woken 75 m from its edge");
        assert!(count(&run, "Bandit") >= 3 && count(&run, "Horse") == 1);
        assert_eq!(
            run.walkers.len(),
            run.world.combatants().count(),
            "every body walks"
        );
        // Two wolves fall, one is hurt.
        let wolves: Vec<CombatantId> = run
            .world
            .combatants()
            .filter(|c| c.name.starts_with("Rime wolf "))
            .map(|c| c.id)
            .collect();
        for &w in &wolves[..2] {
            run.world.combatant_mut(w).unwrap().action = Action::Dead;
        }
        let hurt = run.world.combatant(wolves[2]).unwrap().name.clone();
        run.world
            .combatant_mut(wolves[2])
            .unwrap()
            .vitals
            .health
            .current = 7.0;
        // Away: it sleeps, and every body and entity of it goes.
        go(&mut run, &mut world, -200.0);
        assert_eq!(run.world.combatants().count(), 1, "asleep again");
        assert_eq!(named(&world, "Rime wolf"), 0);
        assert_eq!(run.walkers.len(), 1);
        // Back: only the survivors wake, the hurt one still hurt.
        go(&mut run, &mut world, 200.0);
        assert_eq!(count(&run, "Rime wolf"), 3, "the two that fell stay fallen");
        let again = run
            .world
            .combatants()
            .find(|c| c.name == hurt)
            .expect("the hurt wolf");
        assert!((again.vitals.health.current - 7.0).abs() < 1e-3);
        assert_eq!(bodies(&world, "Rime wolf"), 3, "and only their bodies");
        assert_eq!(
            named(&world, "Rime wolf"),
            2 * 3,
            "each with its head, the fallen ones' heads gone too"
        );
    }
}
