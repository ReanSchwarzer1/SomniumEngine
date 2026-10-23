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
use somnium_ai::navigation::{
    BakeInput, BakeSettings, Bounds, NavTile, NavWorld, OffMeshLink, heightfield,
};
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
    CameraSettingsComponent, EngineContext, EngineEvent, GameUiFrame, ImportedMesh, InputState,
    MaterialComponent, MeshKind, Name, Parent, ParticleEmitter, Transform, UiCanvas,
    WorldTransform,
};
use somnium_ecs::{Entity, World};
use somnium_physics::body::{BodyId, MotionType, RigidBodyDescriptor};
use somnium_physics::shape::ColliderShape;
use somnium_ui::message::{NodeHandle, TextMessage};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

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
/// How far from the camera a click finds the ground, metres.
const CLICK_REACH: f32 = 400.0;

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

pub(crate) fn material(name: &str) -> MaterialComponent {
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
    // The project's content root: its wolves and bandits wear WS3's bodies
    // where it has them.
    let content = args
        .get("bodies")
        .and_then(Value::as_str)
        .map(std::path::Path::new);
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
        spawn_pack(world, root, den, content, &mut out);
    }
    if all || only == Some("bandits") {
        let camp = if all {
            Vec3::new(-14.0, 0.0, 16.0)
        } else {
            Vec3::ZERO
        };
        spawn_camp(world, root, camp, content, &mut out);
    }
    if let Some((team, faces, members)) = only.and_then(band_of) {
        let from = out.len();
        spawn_line(world, root, Vec3::ZERO, content, team, faces, members, &mut out);
        if only == Some("lance") {
            saddle_up(world, root, content, team, from, &mut out);
        }
    }
    if only == Some("harrowcat") {
        spawn_harrowcat(world, root, Vec3::ZERO, content, &mut out);
    }
    if let Some(beast) = only.and_then(beast_of) {
        spawn_beasts(world, root, Vec3::ZERO, content, &beast, &mut out);
    }
    if only == Some("ledger_engine") {
        spawn_engine(world, root, Vec3::ZERO, content, &mut out);
    }
    Ok(out)
}

/// A rime wolf pack round its den, the alpha leading; facing away. With a
/// `content` root holding WS3's rime wolves, each wears one (the alpha its own).
fn spawn_pack(
    world: &mut World,
    root: Entity,
    den: Vec3,
    content: Option<&std::path::Path>,
    out: &mut Vec<Entity>,
) {
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
    let alpha = out.len();
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
    wear(world, out[alpha], content, "creatures/rime_wolf_alpha/rime_wolf_alpha");
    for i in 0..4 {
        let a = i as f32 / 4.0 * std::f32::consts::TAU;
        let at = out.len();
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
        wear(world, out[at], content, "creatures/rime_wolf/rime_wolf");
    }
}

/// A bandit camp round its chief; facing in. With a `content` root holding
/// WS3's ten bandits, each wears one: four different, which four by where the
/// camp stands.
fn spawn_camp(
    world: &mut World,
    root: Entity,
    camp: Vec3,
    content: Option<&std::path::Path>,
    out: &mut Vec<Entity>,
) {
    let first = first_face(world, root, camp, BANDITS);
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
        let at = out.len();
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
        // Three apart round ten: four different bandits.
        let n = (first + i * 3) % BANDITS;
        wear(world, out[at], content, &format!("characters/bandit_{n:02}/bandit_{n:02}"));
    }
}

/// Jolt worlds built at once in one test process fault (its global set-up):
/// every test that builds one (here, `boats`) holds this.
#[cfg(test)]
pub(crate) static JOLT: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A harrowcat (ECOLOGY_AND_BESTIARY: long, silent and black-furred, as tall
/// as a pony; it hunts alone in the forests near the Spires and in the
/// Taigan): a wolf's brain and jaws (its bite and its pounce play the cat's
/// own clips) in a far heavier body, with the beasts. With a `content` root
/// holding WS3's harrowcat it wears it.
fn spawn_harrowcat(
    world: &mut World,
    root: Entity,
    at: Vec3,
    content: Option<&std::path::Path>,
    out: &mut Vec<Entity>,
) {
    let n = out.len();
    out.extend(beast(
        world,
        root,
        "Harrowcat",
        at,
        Vec3::Z,
        Vec3::new(0.9, 1.4, 2.6),
        "dark",
        CombatantComponent {
            team: 2,
            archetype: Archetype::Wolf as u32,
            weapon: WeaponKind::Fangs as u32,
            max_health: 320.0,
            max_stamina: 200.0,
            poise: 90.0,
            armor: 3.0,
            speed: 6.5,
            leader: true,
            ..CombatantComponent::default()
        },
    ));
    wear(world, out[n], content, "creatures/harrowcat/harrowcat");
}

/// A creature of ECOLOGY_AND_BESTIARY that hunts as the wolves do (a wolf's
/// brain and jaws), with the beasts: what one is called, how many hunt
/// together (the first leads), its size (width, height, length), shape
/// material, health, poise, armour and speed, WS3's body if there is one,
/// and whether it is blind (it hunts by ear).
struct Beast {
    name: &'static str,
    count: usize,
    size: Vec3,
    mat: &'static str,
    health: f32,
    poise: f32,
    armor: f32,
    speed: f32,
    body: Option<&'static str>,
    blind: bool,
}

/// The bestiary's kinds beyond wolves and the harrowcat:
/// - `harrowdogs`: feral dogs gone strange near Spire leaks, packs on the
///   edge of the Stilled March (5.9), five, in WS3's sheepdog;
/// - `crocodile`: a Delta crocodile (5.7), one, long and heavily armoured, in
///   WS3's delta_crocodile;
/// - `icecrawlers`: many-legged, blind and pale, from the glacier crevasses,
///   the Rimewall's commonest enemy (5.3), three, in WS3's ice_crawler.
fn beast_of(kind: &str) -> Option<Beast> {
    Some(match kind.trim().to_ascii_lowercase().as_str() {
        "harrowdogs" => Beast {
            name: "Harrow-dog",
            count: 5,
            size: Vec3::new(0.45, 0.7, 1.15),
            mat: "ash",
            health: 60.0,
            poise: 20.0,
            armor: 0.0,
            speed: 5.5,
            body: Some("creatures/sheepdog/sheepdog"),
            blind: false,
        },
        "crocodile" => Beast {
            name: "Delta crocodile",
            count: 1,
            size: Vec3::new(1.3, 0.9, 6.0),
            mat: "stone",
            health: 420.0,
            poise: 140.0,
            armor: 6.0,
            speed: 3.6,
            body: Some("creatures/delta_crocodile/delta_crocodile"),
            blind: false,
        },
        "icecrawlers" => Beast {
            name: "Ice-crawler",
            count: 3,
            size: Vec3::new(1.1, 0.6, 1.8),
            mat: "ice",
            health: 90.0,
            poise: 35.0,
            armor: 3.0,
            speed: 4.6,
            body: Some("creatures/ice_crawler/ice_crawler"),
            blind: true,
        },
        _ => return None,
    })
}

/// A group of `beast` round its leader, facing away (as a pack is).
fn spawn_beasts(
    world: &mut World,
    root: Entity,
    at: Vec3,
    content: Option<&std::path::Path>,
    beast: &Beast,
    out: &mut Vec<Entity>,
) {
    for i in 0..beast.count {
        let leader = i == 0;
        let a = i as f32 / (beast.count - 1).max(1) as f32 * std::f32::consts::TAU;
        let (spot, facing) = if leader {
            (at, Vec3::Z)
        } else {
            (at + Vec3::new(a.cos(), 0.0, a.sin()) * 2.5, Vec3::new(a.sin(), 0.0, a.cos()))
        };
        let name = match (beast.count, leader) {
            (1, _) => beast.name.to_string(),
            (_, true) => format!("{} (leader)", beast.name),
            _ => format!("{} {i}", beast.name),
        };
        let size = if leader { beast.size * 1.1 } else { beast.size };
        let n = out.len();
        out.extend(beast_body(world, root, &name, spot, facing, size, beast, leader));
        if let Some(body) = beast.body {
            wear(world, out[n], content, body);
        }
    }
}

/// One of `beast`'s body (a long box and a head, as `beast` builds).
#[allow(clippy::too_many_arguments)]
fn beast_body(
    world: &mut World,
    root: Entity,
    name: &str,
    feet: Vec3,
    facing: Vec3,
    size: Vec3,
    kind: &Beast,
    leader: bool,
) -> Vec<Entity> {
    let scale = if leader { 1.2 } else { 1.0 };
    beast(
        world,
        root,
        name,
        feet,
        facing,
        size,
        kind.mat,
        CombatantComponent {
            team: u32::from(BEAST_TEAM),
            archetype: Archetype::Wolf as u32,
            weapon: WeaponKind::Fangs as u32,
            max_health: kind.health * scale,
            max_stamina: 160.0,
            poise: kind.poise * scale,
            armor: kind.armor,
            speed: kind.speed,
            leader,
            ..CombatantComponent::default()
        },
    )
}

/// What a kind's brains are beyond their archetype: a blind creature (the
/// ice-crawlers) sees only what touches it and hears far; the Engine's
/// glyph-boards have no brain at all (they stand and are broken).
fn tune(world: &mut CombatWorld, kind: &str, members: &[CombatantId]) {
    if kind.trim().eq_ignore_ascii_case("ledger_engine") {
        for m in members {
            if let Some(c) = world.combatant_mut(*m)
                && c.name.starts_with("Glyph-board")
            {
                c.brain = None;
                c.body.speed = 0.0;
            }
        }
        return;
    }
    if !beast_of(kind).is_some_and(|b| b.blind) {
        return;
    }
    for m in members {
        if let Some(brain) = world.combatant_mut(*m).and_then(|c| c.brain.as_mut()) {
            brain.mind.profile.sight = 3.0;
            brain.mind.profile.hearing *= 1.6;
        }
    }
}

/// The Ledger Engine's keeper (Act VI's climax: ACT6_MAIN_QUEST V6, "burn it
/// down") and its three glyph-boards, Caelan's own designs turned on him.
pub const ENGINE_KEEPER: &str = "Castellane vault-keeper";
pub const ENGINE_BOARDS: [&str; 3] = ["Glyph-board (Count)", "Glyph-board (Hold)", "Glyph-board (Sense)"];
/// How the Engine fights ([`engine_tick`]):
/// - **Count** (while its board stands): the keeper's wounds are counted
///   back, 1.5% of his health a second once he is 4 s unhurt;
/// - **Hold**: every 9 s (7 at two thirds, 5 at one third) a ring of rime
///   opens under Caelan, the tell, for 1.6 s; whoever is still in it then is
///   held (frozen 2.5 s). Below a third it opens where he is going;
/// - **Sense-a-named-person**: at two thirds and at one third of the keeper's
///   health the Engine calls two Castellane guards in by the door;
/// - the keeper quickens by phase (x1.15, x1.3). Every board broken stops the
///   Engine: the keeper reels. Cleared when the keeper and every board fall.
const HOLD_FIRST: f32 = 4.0;
const HOLD_EVERY: [f32; 3] = [9.0, 7.0, 5.0];
const HOLD_TELL: f32 = 1.6;
const HOLD_RADIUS: f32 = 2.5;
const HOLD_FREEZE: f32 = 2.5;
const HOLD_LEAD: f32 = 1.0;
const COUNT_QUIET: f32 = 4.0;
const COUNT_RATE: f32 = 0.015;
const PHASE_PACE: [f32; 3] = [1.0, 1.15, 1.3];
/// Where the Engine's called guards come in: ahead of the keeper (+Z), by the door.
const SENSE_FROM: f32 = 8.0;

/// The keeper before the Engine's boards, which stand in a row behind him
/// (-Z), 3 m apart. With a `content` root he wears WS3's Castellane boss.
fn spawn_engine(
    world: &mut World,
    root: Entity,
    at: Vec3,
    content: Option<&std::path::Path>,
    out: &mut Vec<Entity>,
) {
    let n = out.len();
    out.extend(fighter(
        world,
        root,
        ENGINE_KEEPER,
        at,
        Vec3::Z,
        CombatantComponent {
            team: 1,
            archetype: Archetype::Duelist as u32,
            weapon: WeaponKind::Sword as u32,
            max_health: 650.0,
            max_stamina: 220.0,
            poise: 140.0,
            armor: 4.0,
            speed: 4.2,
            leader: true,
            ..CombatantComponent::default()
        },
    ));
    wear(world, out[n], content, "characters/lampman_10/lampman_10");
    for (i, name) in ENGINE_BOARDS.iter().enumerate() {
        let size = Vec3::new(1.6, 2.4, 0.3);
        let feet = at + Vec3::new((i as f32 - 1.0) * 3.0, 0.0, -4.0);
        let n = out.len();
        out.push(world.spawn((
            Name::new(name),
            Transform {
                translation: feet + Vec3::Y * size.y * 0.5,
                rotation: Quat::IDENTITY,
                scale: size,
            },
            WorldTransform::identity(),
            Parent { entity: root },
            MeshKind::Cube,
            material("force"),
            CombatantComponent {
                team: 1,
                archetype: Archetype::Brute as u32,
                weapon: WeaponKind::Unarmed as u32,
                max_health: 160.0,
                poise: 9999.0,
                speed: 0.0,
                ..CombatantComponent::default()
            },
        )));
        // WS3's board where the project has it: its mesh on the ground under
        // the box (whose scale it undoes), the box no longer drawn.
        let file = board_file(i, false);
        if content.is_some_and(|c| c.join(&file).exists()) {
            let board = out[n];
            let _ = world.remove_component::<MeshKind>(board);
            out.push(world.spawn((
                Name::new(&format!("{name} mesh")),
                Transform {
                    translation: Vec3::new(0.0, -0.5, 0.0),
                    rotation: Quat::IDENTITY,
                    scale: Vec3::ONE / size,
                },
                WorldTransform::identity(),
                Parent { entity: board },
                ImportedMesh {
                    source: format!("assets/{file}"),
                    node: 0,
                },
                MaterialComponent {
                    asset: AssetId::NONE,
                    runtime_id: 0,
                },
            )));
        }
    }
}

/// WS3's glyph-board for [`ENGINE_BOARDS`]`[i]`, whole or broken
/// (`creatures/ledger_board_<count|hold|sense>[_broken].glb`, one mesh node).
fn board_file(i: usize, broken: bool) -> String {
    let board = ["count", "hold", "sense"][i.min(2)];
    let tail = if broken { "_broken" } else { "" };
    format!("creatures/ledger_board_{board}/ledger_board_{board}{tail}.glb")
}

/// The Engine's fight, once its keeper has woken ([`ENGINE_KEEPER`]).
#[derive(Clone, Debug, Default)]
struct EngineState {
    /// Seconds to the next Hold.
    hold_in: f32,
    /// The Hold showing: where, how long it has left, its ring.
    tell: Option<(Vec3, f32, Entity)>,
    /// Calls made (at two thirds and at one third).
    sensed: u8,
    /// Seconds since the keeper was last hurt, and his health then.
    quiet: f32,
    last: f32,
    /// His pace before the phases.
    pace: f32,
    /// Every board broken: the Engine has stopped.
    stopped: bool,
    /// Boards shown broken (WS3's broken mesh swapped in).
    broken: [bool; 3],
}

/// The Engine's turn each fixed step: Count, Hold and Sense (see
/// [`HOLD_EVERY`]) for every woken `ledger_engine`, and its tell's ring.
fn engine_tick(world: &mut World, run: &mut Run, dt: f32) {
    let Some(set) = run.encounters.as_mut() else {
        return;
    };
    let hero = run
        .player
        .and_then(|p| run.world.combatant(p))
        .map(|c| (c.id, c.position, c.velocity));
    let mut calls = Vec::new();
    let awake: Vec<GroupId> = set.awake().collect();
    for id in awake {
        let at = set.position(id);
        let Some(record) = set.data_mut(id) else {
            continue;
        };
        if !record.kind.trim().eq_ignore_ascii_case("ledger_engine") {
            continue;
        }
        let named = |name: &str| {
            record
                .members
                .iter()
                .copied()
                .find(|m| run.world.combatant(*m).is_some_and(|c| c.name == name && c.alive()))
        };
        let keeper = named(ENGINE_KEEPER);
        let boards = ENGINE_BOARDS.map(|b| named(b).is_some());
        let state = record.engine.get_or_insert_with(EngineState::default);
        // A board just broken shows WS3's broken board.
        for (i, name) in ENGINE_BOARDS.iter().enumerate() {
            if boards[i] || state.broken[i] {
                continue;
            }
            state.broken[i] = true;
            let file = board_file(i, true);
            if !run.content.as_ref().is_some_and(|c| c.join(&file).exists()) {
                continue;
            }
            let Some(board) = record
                .members
                .iter()
                .find(|m| run.world.combatant(**m).is_some_and(|c| c.name == *name))
                .and_then(|m| run.bodies.get(m).copied())
            else {
                continue;
            };
            let meshes: Vec<Entity> = world
                .iter_with::<ImportedMesh>()
                .filter(|(e, _)| world.get::<Parent>(*e).is_some_and(|p| p.entity == board))
                .map(|(e, _)| e)
                .collect();
            for e in meshes {
                if let Some(mesh) = world.get_mut::<ImportedMesh>(e) {
                    mesh.source = format!("assets/{file}");
                }
            }
        }
        let Some(keeper) = keeper else {
            if let Some((_, _, ring)) = state.tell.take() {
                world.despawn(ring);
                record.entities.retain(|e| *e != ring);
            }
            continue;
        };
        let Some(c) = run.world.combatant(keeper) else {
            continue;
        };
        let (health, max) = (c.vitals.health.current, c.vitals.health.max.max(1.0));
        if state.pace == 0.0 {
            state.pace = c.body.speed;
            state.last = health;
            state.hold_in = HOLD_FIRST;
        }
        let fraction = health / max;
        let phase = if fraction > 2.0 / 3.0 {
            0
        } else if fraction > 1.0 / 3.0 {
            1
        } else {
            2
        };
        state.quiet = if health < state.last - 1e-3 { 0.0 } else { state.quiet + dt };
        state.last = health;
        let (stopped, pace) = (boards.iter().all(|b| !b), state.pace);
        if let Some(c) = run.world.combatant_mut(keeper) {
            c.body.speed = pace * if stopped { 1.0 } else { PHASE_PACE[phase] };
            // Count: his wounds counted back while he is let be.
            if boards[0] && state.quiet >= COUNT_QUIET && fraction < 1.0 {
                c.vitals.heal(max * COUNT_RATE * dt);
                state.last = c.vitals.health.current;
            }
        }
        if stopped && !state.stopped {
            state.stopped = true;
            run.world.apply_status(keeper, StatusKind::Stagger, 1.5);
        }
        // Hold: the tell, then the hold on whoever stays in it.
        match state.tell.take() {
            Some((centre, left, ring)) if boards[1] => {
                let left = left - dt;
                if left > 0.0 {
                    if let Some(t) = world.get_mut::<Transform>(ring) {
                        let grow = 0.6 + 0.4 * (1.0 - left / HOLD_TELL);
                        t.scale = Vec3::new(2.0 * HOLD_RADIUS * grow, 0.04, 2.0 * HOLD_RADIUS * grow);
                    }
                    state.tell = Some((centre, left, ring));
                } else {
                    world.despawn(ring);
                    record.entities.retain(|e| *e != ring);
                    if let Some((who, p, _)) = hero {
                        let off = Vec3::new(p.x - centre.x, 0.0, p.z - centre.z);
                        if off.length() <= HOLD_RADIUS {
                            run.world.apply_status(who, StatusKind::Freeze, HOLD_FREEZE);
                        }
                    }
                    state.hold_in = HOLD_EVERY[phase];
                }
            }
            Some((_, _, ring)) => {
                // Its board broken under it: the Hold is gone.
                world.despawn(ring);
                record.entities.retain(|e| *e != ring);
            }
            None if boards[1] => {
                state.hold_in -= dt;
                if state.hold_in <= 0.0
                    && let Some((_, p, v)) = hero
                {
                    let lead = if phase == 2 { Vec3::new(v.x, 0.0, v.z) * HOLD_LEAD } else { Vec3::ZERO };
                    let centre = p + lead;
                    let ring = world.spawn((
                        Name::new("Hold (the Ledger Engine)"),
                        Transform {
                            translation: centre + Vec3::Y * 0.03,
                            rotation: Quat::IDENTITY,
                            scale: Vec3::new(1.2 * HOLD_RADIUS, 0.04, 1.2 * HOLD_RADIUS),
                        },
                        WorldTransform::identity(),
                        MeshKind::Cylinder,
                        material("frostmark"),
                    ));
                    record.entities.push(ring);
                    state.tell = Some((centre, HOLD_TELL, ring));
                }
            }
            None => {}
        }
        // Sense: the Engine calls its guards at two thirds and one third.
        let due = match state.sensed {
            0 => fraction <= 2.0 / 3.0,
            1 => fraction <= 1.0 / 3.0,
            _ => false,
        };
        if boards[2]
            && due
            && let Some(at) = at
        {
            state.sensed += 1;
            calls.push((at + record.facing.normalize_or(Vec3::Z) * SENSE_FROM, -record.facing, state.sensed));
        }
    }
    for (at, facing, n) in calls {
        set.add(
            at,
            encounter_extent("vault_guards"),
            Encounter {
                kind: "vault_guards".into(),
                facing,
                home: ("engine call".into(), [i64::from(n), 0, 0]),
                ..Encounter::default()
            },
        );
    }
}

/// A boss fight as a game's HUD shows it ([`CombatDemo::boss_view`]).
#[derive(Clone, Debug, PartialEq)]
pub struct BossView {
    /// The boss's name.
    pub name: String,
    /// Its health, 0 to 1.
    pub health: f32,
    /// 1 to 3, by its health (two thirds, one third).
    pub phase: u8,
    /// Its parts the player breaks, each with whether it still stands.
    pub parts: Vec<(String, bool)>,
    /// The Hold coming: where (its centre on the ground), its radius, and the
    /// seconds before it closes.
    pub hold: Option<(Vec3, f32, f32)>,
}

/// WS3's bandits (`characters/bandit_00` to `bandit_09`).
const BANDITS: usize = 10;

/// The first of a group's faces (three apart round `count`), by where it
/// stands: `local` under `root`.
fn first_face(world: &World, root: Entity, local: Vec3, count: usize) -> usize {
    let home = world
        .get::<Transform>(root)
        .map_or(local, |t| t.translation + local);
    // ponytail: the 100 m cell the group stands in picks its faces, so a group whose
    // survivors drift over a cell's edge between wakes changes faces (unseen at
    // the wake's 120 m); key it by the encounter if that ever shows.
    ((home.x / 100.0).floor() as i64 * 7 + (home.z / 100.0).floor() as i64 * 3).rem_euclid(count.max(1) as i64)
        as usize
}

/// The factions a game turns against the protagonist
/// ([`CombatDemo::set_hostile_factions`], [`CombatDemo::set_hunting_factions`]):
/// the five Lamps of Vespergate, its watch, Tyffir's Lance and the Margrave's
/// knights. Each is a team of its own from [`FACTION_TEAM`] on, neutral to the
/// party until it is turned.
pub const FACTIONS: [&str; 8] = [
    "lamp_red",
    "lamp_white",
    "lamp_amber",
    "lamp_green",
    "lamp_blue",
    "watch",
    "lance",
    "knights",
];
/// The first faction's team; the others follow in [`FACTIONS`]' order.
const FACTION_TEAM: u16 = 16;
/// The beasts' team: rime wolves, harrowcats and the rest of the bestiary.
pub const BEAST_TEAM: u16 = 2;

/// A faction's team, by its name in [`FACTIONS`].
fn faction_team(name: &str) -> Option<TeamId> {
    let name = name.trim();
    FACTIONS
        .iter()
        .position(|f| f.eq_ignore_ascii_case(name))
        .map(|i| TeamId(FACTION_TEAM + i as u16))
}

/// The Combat Encounter kind a faction sends: `lamp_red` a `lampmen red`
/// crew, the others their own name.
fn faction_kind(name: &str) -> Option<String> {
    faction_team(name)?;
    let name = name.trim().to_ascii_lowercase();
    Some(match name.strip_prefix("lamp_") {
        Some(colour) => format!("lampmen {colour}"),
        None => name,
    })
}

/// One of a band strung along a street or road: name, archetype, weapon,
/// health and armour. The first leads.
type Member = (&'static str, Archetype, WeaponKind, f32, f32);

/// The bodies a band wears: WS3's `<family>_NN`, its leader's from `leaders`
/// (or, with none, from `others`) and the rest from `others`, in turn.
#[derive(Clone, Copy)]
struct Faces {
    family: &'static str,
    leaders: &'static [u8],
    others: &'static [u8],
}

/// A band of `kind`: its team, the bodies it wears, and its members.
/// `lampmen` alone is the always-hostile crew it was (the Varga's guards);
/// `lampmen <colour>` (ACT6_PLACES_AND_CAST 4.3: knives red, clubs amber,
/// riding crops and pistols green, canes white, unseen blue), `watch`,
/// `lance` and `knights` are factions. Lamp-men wear WS3's Lamp-men of their
/// Lamp (`lampman_NN`, the bosses by age: the Varga's 21 or 15, the
/// Castellane's 10 or 18, amber 28, green 32, blue 30); the watch its guards;
/// the Lance and the knights its lances.
fn band_of(kind: &str) -> Option<(u32, Faces, &'static [Member])> {
    use Archetype::{Brute, Duelist, Rifleman, Skirmisher, Spearman};
    use WeaponKind::{Axe, Knife, LanceGlaive, LatticeRifle, Spear, Sword, Unarmed};
    const RED: &[Member] = &[
        ("Lamp-man boss", Duelist, Knife, 130.0, 1.0),
        ("Lamp-man (knife)", Skirmisher, Knife, 90.0, 1.0),
        ("Lamp-man (lookout)", Skirmisher, Knife, 90.0, 1.0),
        ("Lamp-man (rifle)", Rifleman, LatticeRifle, 90.0, 1.0),
    ];
    const WHITE: &[Member] = &[
        ("Lamp-man boss", Duelist, Sword, 130.0, 1.0),
        ("Lamp-man (cane)", Duelist, Sword, 90.0, 1.0),
        ("Lamp-man (swordstick)", Duelist, Sword, 90.0, 1.0),
        ("Lamp-man (pistol)", Rifleman, LatticeRifle, 90.0, 1.0),
    ];
    const AMBER: &[Member] = &[
        ("Lamp-man boss", Brute, Unarmed, 150.0, 2.0),
        ("Lamp-man (club)", Brute, Unarmed, 110.0, 2.0),
        ("Lamp-man (cudgel)", Brute, Unarmed, 110.0, 2.0),
        ("Lamp-man (knife)", Skirmisher, Knife, 90.0, 1.0),
    ];
    const GREEN: &[Member] = &[
        ("Lamp-man boss", Duelist, Sword, 130.0, 1.0),
        ("Lamp-man (crop)", Skirmisher, Unarmed, 90.0, 1.0),
        ("Lamp-man (pistol)", Rifleman, LatticeRifle, 90.0, 1.0),
        ("Lamp-man (second pistol)", Rifleman, LatticeRifle, 90.0, 1.0),
    ];
    const BLUE: &[Member] = &[
        ("Lamp-man boss", Skirmisher, Knife, 120.0, 0.0),
        ("Lamp-man (knife)", Skirmisher, Knife, 90.0, 0.0),
        ("Lamp-man (shadow)", Skirmisher, Knife, 90.0, 0.0),
        ("Lamp-man (lookout)", Skirmisher, Knife, 90.0, 0.0),
    ];
    const WATCH: &[Member] = &[
        ("Watch sergeant", Duelist, Sword, 140.0, 3.0),
        ("Watchman (spear)", Spearman, Spear, 110.0, 2.0),
        ("Watchman (pike)", Spearman, Spear, 110.0, 2.0),
    ];
    const LANCE: &[Member] = &[
        ("Lance captain", Duelist, Sword, 170.0, 4.0),
        ("Lance rider (glaive)", Spearman, LanceGlaive, 140.0, 3.0),
        ("Lance rider (spear)", Spearman, Spear, 140.0, 3.0),
    ];
    const VAULT: &[Member] = &[
        ("Castellane guard (cane)", Duelist, Sword, 100.0, 1.0),
        ("Castellane guard (pistol)", Rifleman, LatticeRifle, 90.0, 1.0),
    ];
    const KNIGHTS: &[Member] = &[
        ("Knight-captain", Duelist, Sword, 200.0, 6.0),
        ("Knight (sword)", Duelist, Sword, 170.0, 6.0),
        ("Knight (axe)", Brute, Axe, 170.0, 6.0),
        ("Knight (glaive)", Spearman, LanceGlaive, 170.0, 6.0),
    ];
    // WS3's Lamp-men by Lamp (rb-characters, 19 Oct).
    const VARGA: Faces = Faces {
        family: "lampman",
        leaders: &[21, 15],
        others: &[2, 7, 12, 17, 19, 23],
    };
    const CASTELLANE: Faces = Faces {
        family: "lampman",
        leaders: &[10, 18],
        others: &[0, 5, 16, 20, 22, 24],
    };
    const AMBER_MEN: Faces = Faces {
        family: "lampman",
        leaders: &[28],
        others: &[1, 6, 11, 25, 31],
    };
    const GREEN_MEN: Faces = Faces {
        family: "lampman",
        leaders: &[32],
        others: &[3, 8, 13, 26, 29],
    };
    const BLUE_MEN: Faces = Faces {
        family: "lampman",
        leaders: &[30],
        others: &[4, 9, 14, 27, 33],
    };
    const GUARDS: Faces = Faces {
        family: "guard",
        leaders: &[],
        others: &[0, 1, 2, 3, 4, 5, 6, 7],
    };
    const LANCES: Faces = Faces {
        family: "lance",
        leaders: &[],
        others: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
    };
    let kind = kind.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase();
    let (faction, faces, members) = match kind.as_str() {
        "lampmen" => return Some((1, VARGA, RED)),
        // The Engine's call (always hostile: they come for him).
        "vault_guards" => return Some((1, CASTELLANE, VAULT)),
        "lampmen red" => ("lamp_red", VARGA, RED),
        "lampmen white" => ("lamp_white", CASTELLANE, WHITE),
        "lampmen amber" => ("lamp_amber", AMBER_MEN, AMBER),
        "lampmen green" => ("lamp_green", GREEN_MEN, GREEN),
        "lampmen blue" => ("lamp_blue", BLUE_MEN, BLUE),
        "watch" => ("watch", GUARDS, WATCH),
        "lance" => ("lance", LANCES, LANCE),
        "knights" => ("knights", LANCES, KNIGHTS),
        _ => return None,
    };
    Some((u32::from(faction_team(faction)?.0), faces, members))
}

/// Where each of a line stands along its street (+Z), the first its leader.
const LINE: [f32; 4] = [0.0, 2.5, -2.5, -6.0];

/// A band strung along its street or road (+Z) within an alley's 4 m, facing
/// its leader: a Lamp crew, a watch post, a Lance patrol, knights. With a
/// `content` root each wears one of WS3's `faces`, which (as far as there are
/// enough) all differ.
#[allow(clippy::too_many_arguments)]
fn spawn_line(
    world: &mut World,
    root: Entity,
    at: Vec3,
    content: Option<&std::path::Path>,
    team: u32,
    faces: Faces,
    members: &[Member],
    out: &mut Vec<Entity>,
) {
    let count = faces.others.len();
    let first = first_face(world, root, at, count);
    // Called only with `count` > 0.
    let face = |i: usize| -> u8 {
        if faces.leaders.is_empty() {
            faces.others[(first + i) % count]
        } else if i == 0 {
            faces.leaders[first % faces.leaders.len()]
        } else {
            faces.others[(first + i - 1) % count]
        }
    };
    for (i, &(name, archetype, weapon, health, armor)) in members.iter().enumerate() {
        let along = LINE[i % LINE.len()] - 6.0 * (i / LINE.len()) as f32;
        let side = if i % 2 == 0 { 0.6 } else { -0.6 };
        let spot = at + Vec3::new(side, 0.0, along);
        let facing = if along == 0.0 { Vec3::Z } else { at - spot };
        let n = out.len();
        out.extend(fighter(
            world,
            root,
            name,
            spot,
            facing,
            CombatantComponent {
                team,
                archetype: archetype as u32,
                weapon: weapon as u32,
                max_health: health,
                armor,
                speed: if weapon == WeaponKind::Knife { 4.3 } else { 4.0 },
                leader: i == 0,
                ..CombatantComponent::default()
            },
        ));
        if count > 0 {
            let (family, b) = (faces.family, face(i));
            wear(world, out[n], content, &format!("characters/{family}_{b:02}/{family}_{b:02}"));
        }
    }
}

/// The Lance rides: a horse beside each fighter made into `out` from `from`,
/// named for its rider (`<rider>'s horse`) for [`seat_band`] to put them in
/// the saddle once they join the fight; with a `content` root they wear the
/// slice's march pony.
fn saddle_up(
    world: &mut World,
    root: Entity,
    content: Option<&std::path::Path>,
    team: u32,
    from: usize,
    out: &mut Vec<Entity>,
) {
    let riders: Vec<(String, Vec3)> = out[from..]
        .iter()
        .filter(|e| world.get::<CombatantComponent>(**e).is_some())
        .filter_map(|e| Some((world.get::<Name>(*e)?.as_str().to_string(), world.get::<Transform>(*e)?.translation)))
        .collect();
    for (name, at) in riders {
        // On the rider's outer side, along the road.
        let side = if at.x < 0.0 { -1.2 } else { 1.2 };
        let horse = world.spawn((
            Name::new(&format!("{name}'s horse")),
            Transform {
                translation: Vec3::new(at.x + side, 0.8, at.z),
                rotation: yaw(Vec3::Z),
                scale: Vec3::new(0.85, 1.6, 2.4),
            },
            WorldTransform::identity(),
            Parent { entity: root },
            MeshKind::Cube,
            material("hide"),
            CombatantComponent {
                team,
                mount: 1,
                max_health: 200.0,
                poise: 80.0,
                ..CombatantComponent::default()
            },
        ));
        wear(world, horse, content, "creatures/march_pony/march_pony");
        out.push(horse);
    }
}

/// Each `<rider>'s horse` among `ids` with its rider in the saddle: the
/// nearest fighter of that name on foot ([`saddle_up`]'s Lance).
fn seat_band(combat: &mut CombatWorld, ids: &[CombatantId]) {
    let horses: Vec<(CombatantId, String, Vec3)> = ids
        .iter()
        .filter_map(|id| combat.combatant(*id))
        .filter(|c| c.mount_def.is_some() && c.rider.is_none())
        .filter_map(|c| Some((c.id, c.name.strip_suffix("'s horse")?.to_string(), c.position)))
        .collect();
    for (horse, name, at) in horses {
        let rider = ids
            .iter()
            .filter_map(|id| combat.combatant(*id))
            .filter(|c| c.name == name && c.mount.is_none() && c.mount_def.is_none())
            .min_by(|a, b| flat_distance(a.position, at).total_cmp(&flat_distance(b.position, at)))
            .map(|c| c.id);
        if let Some(rider) = rider {
            let _ = somnium_combat::mount::mount(combat, rider, horse);
        }
    }
}

/// Dress a fighter in WS3's skinned body `<stem>.glb` (its `_lod1` and `_lod2`
/// beside it) when `content` has the file: in Play it draws through it
/// (`combat_bodies`). Without the file the shapes stay (the sandbox).
fn wear(world: &mut World, fighter: Entity, content: Option<&std::path::Path>, stem: &str) {
    let Some(content) = content else {
        return;
    };
    let file = |suffix: &str| format!("{stem}{suffix}.glb");
    if !content.join(file("")).exists() {
        return;
    }
    let level = |suffix: &str| {
        let f = file(suffix);
        if content.join(&f).exists() {
            AssetId::from_relative_path(&f)
        } else {
            AssetId::NONE
        }
    };
    let body = crate::skinned_crowd::SkinnedBody {
        source: AssetId::from_relative_path(file("")),
        lod1: level("_lod1"),
        lod2: level("_lod2"),
        ..Default::default()
    };
    let _ = world.insert_component(fighter, body);
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
    // Capture hooks: `SOMNIUM_COMBAT_BODY=<glb>` dresses the player in a skinned
    // body (its `_lod1`, `_lod2` beside it), `SOMNIUM_COMBAT_MOUNT_BODY=<glb>` the
    // first mount.
    let dress = |world: &mut World, var: &str, pick: &dyn Fn(&CombatantComponent) -> bool| {
        let Ok(glb) = std::env::var(var) else {
            return;
        };
        let Some(e) = world.iter_with::<CombatantComponent>().find(|(_, c)| pick(c)).map(|(e, _)| e) else {
            return;
        };
        let level = |k: &str| {
            glb.strip_suffix(".glb")
                .map_or(AssetId::NONE, |stem| AssetId::from_relative_path(format!("{stem}_{k}.glb")))
        };
        let body = crate::skinned_crowd::SkinnedBody {
            source: AssetId::from_relative_path(&glb),
            lod1: level("lod1"),
            lod2: level("lod2"),
            ..Default::default()
        };
        let _ = world.insert_component(e, body);
    };
    dress(world, "SOMNIUM_COMBAT_BODY", &|c| c.player);
    dress(world, "SOMNIUM_COMBAT_MOUNT_BODY", &|c| c.mount != 0);
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
        || world
            .iter_with::<CombatPatrolComponent>()
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

/// A band or pack that walks a route: the entity's `somnium.Spline`, as it is
/// drawn (just where it stands, without one). Asleep it is a record that
/// keeps walking the route; it wakes where it has got to when the
/// protagonist comes near, and walks on from there until it finds a fight.
#[derive(Clone, Debug, PartialEq)]
pub struct CombatPatrolComponent {
    /// Walks while Play runs.
    pub enabled: bool,
    /// Who walks: `bandits` (a band) or `wolves` (a pack).
    pub kind: String,
    /// Bring its own flat ground; off stands everyone on the ground where it
    /// wakes.
    pub floor: bool,
    /// Walking pace, metres a second.
    pub speed: f32,
}

impl somnium_ecs::Component for CombatPatrolComponent {}

impl Default for CombatPatrolComponent {
    fn default() -> Self {
        Self {
            enabled: true,
            kind: "bandits".into(),
            floor: false,
            speed: 1.3,
        }
    }
}

fn register_encounter(registry: &mut somnium_ecs::reflect::TypeRegistry) {
    registry.register(somnium_ecs::component_schema! {
        CombatPatrolComponent as "somnium.CombatPatrol", display "Combat Patrol", version 1,
        fields {
            enabled { doc: "Walks its route (this entity's Spline) while Play runs, asleep until the protagonist comes near." },
            kind { doc: "bandits: a band that walks the road; wolves: a pack that roams." },
            floor { doc: "Bring its own flat ground; off stands everyone on the ground where it wakes." },
            speed { min: 0.0, max: 6.0, unit: "m/s", doc: "Walking pace along the route." },
        }
    });
    registry.register(somnium_ecs::component_schema! {
        CombatEncounterComponent as "somnium.CombatEncounter", display "Combat Encounter", version 1,
        fields {
            enabled { doc: "Built on Play at this entity's position, taken away on Stop." },
            kind { doc: "wilds: a horse, a rime wolf pack and a bandit camp, none of them aware yet; wolves: the pack alone; bandits: the camp alone; harrowcat: one, hunting alone; harrowdogs, crocodile, icecrawlers: the bestiary's; lampmen [red, white, amber, green, blue]: a Lamp crew on a street; watch, lance, knights: the law's; ledger_engine: Act VI's climax, the vault-keeper and the Engine's three glyph-boards." },
            floor { doc: "Bring its own flat ground; off stands everyone on the ground here." },
        }
    });
}

/// Every enabled Combat Encounter built at its entity's position (the
/// scene's own camera and protagonist kept), in WS3's bodies where `content`
/// has them: the entities made, for Stop to take away.
fn expand_encounters(world: &mut World, content: Option<&std::path::Path>) -> Vec<Entity> {
    let wanted: Vec<(Mat4, CombatEncounterComponent)> = world
        .iter_with::<CombatEncounterComponent>()
        .filter(|(_, c)| c.enabled)
        .map(|(e, c)| (world_matrix(world, e), c.clone()))
        .collect();
    let mut wanted = wanted;
    // Without a protagonist to wake them, patrols stand at their route's start.
    let patrols: Vec<(Mat4, CombatEncounterComponent)> = world
        .iter_with::<CombatPatrolComponent>()
        .filter(|(_, c)| c.enabled)
        .map(|(e, c)| {
            let (route, closed) = patrol_route(world, e);
            let (at, way) = route_at(&route, closed, 0.0);
            let m = Mat4::from_rotation_translation(yaw(way), at);
            let encounter = CombatEncounterComponent {
                enabled: true,
                kind: c.kind.clone(),
                floor: c.floor,
            };
            (m, encounter)
        })
        .collect();
    wanted.extend(patrols);
    let mut made = Vec::new();
    for (m, encounter) in wanted {
        let (at, facing) = (m.transform_point3(Vec3::ZERO), m.transform_vector3(Vec3::Z));
        match build_encounter(world, &encounter.kind, at, facing, encounter.floor, content) {
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
    content: Option<&std::path::Path>,
) -> Result<Vec<Entity>, String> {
    let mut args = json!({
        "position": [at.x, at.y, at.z],
        "facing": [facing.x, facing.y, facing.z],
        "floor": floor,
        "encounter": true,
    });
    if let Some(content) = content {
        args["bodies"] = json!(content.to_string_lossy());
    }
    match kind.trim().to_ascii_lowercase().as_str() {
        "wilds" => {}
        part @ ("wolves" | "bandits" | "harrowcat") => args["only"] = json!(part),
        beast if beast_of(beast).is_some() => args["only"] = json!(beast),
        "ledger_engine" => args["only"] = json!("ledger_engine"),
        band if band_of(band).is_some() => args["only"] = json!(band),
        other => {
            return Err(format!(
                "no kind {other:?} (try wilds, wolves, bandits, harrowcat, harrowdogs, crocodile, icecrawlers, lampmen [red|white|amber|green|blue], watch, lance, knights or ledger_engine)"
            ));
        }
    }
    spawn_wilds(world, &args)
}

/// How far an encounter of `kind` spreads from its centre, metres.
fn encounter_extent(kind: &str) -> f32 {
    match kind.trim().to_ascii_lowercase().as_str() {
        "wilds" => 25.0,
        "ledger_engine" => 8.0,
        band if band_of(band).is_some() => 8.0,
        _ => 6.0,
    }
}

/// How often sleeping encounters are looked at, seconds (the check costs
/// only the grid cells around the protagonist, but there is no need to look
/// sixty times a second).
const TEND_EVERY: f32 = 0.5;

/// The cells of a woken encounter's navigation patch, metres: open land,
/// where the voxel bake's quarter metre would be thousands of times more.
const PATCH_CELL: f32 = 2.0;
/// How far past an encounter's own extent its patch reaches: room to chase.
const PATCH_REACH: f32 = 32.0;
/// The middle of a patch's cell id, apart from any baked tile's.
const PATCH_LAYER: i64 = 1 << 40;
/// Over a group's feet, the height from which an indoor patch is cast for:
/// above the heads, below an upper floor.
const INDOOR_HEAD: f32 = 2.2;

/// The navigation patch round a woken encounter, where no navmesh was baked:
/// the ground's height every [`PATCH_CELL`] metres from a cast down through
/// physics (terrain, rocks and roofs alike: a roof stands too high above the
/// street to join it), deep water left out.
fn patch(
    physics: &somnium_physics::world::PhysicsWorld,
    water: &[Pool],
    id: GroupId,
    at: Vec3,
    extent: f32,
) -> Option<NavTile> {
    let query = JoltQuery(physics);
    let pools = JoltTerrain { physics, water };
    let half = extent + PATCH_REACH;
    let snap = |v: f32| (v / PATCH_CELL).floor() * PATCH_CELL;
    let (x0, z0) = (snap(at.x - half), snap(at.z - half));
    let side = ((2.0 * half) / PATCH_CELL).ceil() * PATCH_CELL + PATCH_CELL;
    let cast = |top: Vec3, bottom: Vec3| {
        query
            .sweep(top, bottom, 0.05)
            .map(|h| top.y + (bottom.y - top.y) * h.fraction - 0.05)
    };
    // Indoors (a roof over the group and a floor at its feet: the Varga
    // warehouse's store) the ground is cast for from just over the heads, so
    // the patch is the floor and its walls, not the roof. Out of doors it is
    // cast for from high above, as ever.
    let down = Vec3::new(at.x, at.y - 60.0, at.z);
    let roof = cast(Vec3::new(at.x, at.y + 60.0, at.z), down);
    let floor = cast(Vec3::new(at.x, at.y + INDOOR_HEAD, at.z), down);
    let indoors = matches!((roof, floor), (Some(r), Some(f)) if r > at.y + INDOOR_HEAD && (f - at.y).abs() < 1.5);
    let from = if indoors { at.y + INDOOR_HEAD } else { at.y + 60.0 };
    let ground = |x: f32, z: f32| {
        let y = cast(Vec3::new(x, from, z), Vec3::new(x, at.y - 60.0, z))?;
        let deep = pools.water_surface(x, z).is_some_and(|w| w > y + 0.5);
        (!deep).then_some(y)
    };
    heightfield(
        [i64::from(id.0), PATCH_LAYER, 0],
        Bounds {
            min: [x0, at.y - 61.0, z0],
            max: [x0 + side, at.y + 61.0, z0 + side],
        },
        BakeSettings {
            voxel_size: PATCH_CELL,
            agent_radius: 0.35,
            ..BakeSettings::default()
        },
        ground,
    )
    .ok()
}

/// A scene encounter while Play runs: what it is, who of it is left once it
/// has woken and slept again, and while it is awake what it made.
#[derive(Clone, Debug, Default)]
struct Encounter {
    kind: String,
    floor: bool,
    /// Its kind and where it was placed, half-metre steps: how a streamed
    /// copy of the same encounter is known.
    home: (String, [i64; 3]),
    /// Its entity's forward: the layout turns with it.
    facing: Vec3,
    /// Survivors by name, with their health, after its first sleep; `None`
    /// before it ever woke (everyone).
    left: Option<Vec<(String, f32)>>,
    /// Where its navigation patch is centred, while it has one.
    patch_at: Option<Vec3>,
    /// A patrol's route in the world (empty for a camp or pack that stays),
    /// whether it closes, its pace, and how far along it the group has got
    /// (metres; it walks on while asleep).
    route: Vec<Vec3>,
    closed: bool,
    speed: f32,
    along: f32,
    /// A pack shadowing the protagonist (at dusk): its homes are off its own.
    shadowing: bool,
    /// The scene's Combat Encounter or Combat Patrol entity it came from (the
    /// latest copy, when its cell streams back in).
    source: Option<Entity>,
    /// Woken, and every one of its fighters fell: told once, never woken again.
    cleared: bool,
    /// A band its faction sent to hunt the protagonist ([`hunt`]): it walks to
    /// him asleep and makes for him awake.
    hunter: Option<String>,
    /// The Ledger Engine's fight, once woken ([`engine_tick`]).
    engine: Option<EngineState>,
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
    enlist_encounters(world, &mut set);
    set
}

/// Every enabled Combat Encounter that is not a record yet becomes one: those
/// a streamed cell brings in after Play began, or a script spawns. A record
/// is known by its kind and where it was placed (to half a metre), so a cell
/// that streams out and back in brings its encounters only once.
fn enlist_encounters(world: &World, set: &mut Dormancy<Encounter>) {
    let mut known: BTreeMap<(String, [i64; 3]), GroupId> = (0..set.len())
        .map(|i| GroupId(i as u32))
        .filter_map(|id| Some((set.data(id)?.home.clone(), id)))
        .collect();
    // The entity a record names is the latest copy of its encounter (a cell
    // that streamed out and back in brings a new one).
    fn seen(
        set: &mut Dormancy<Encounter>,
        known: &BTreeMap<(String, [i64; 3]), GroupId>,
        home: &(String, [i64; 3]),
        e: Entity,
    ) -> bool {
        let id = known.get(home).copied();
        if let Some(record) = id.and_then(|id| set.data_mut(id)) {
            record.source = Some(e);
        }
        id.is_some()
    }
    for (e, c) in world.iter_with::<CombatEncounterComponent>() {
        if !c.enabled {
            continue;
        }
        let m = world_matrix(world, e);
        let at = m.transform_point3(Vec3::ZERO);
        let home = (
            c.kind.trim().to_ascii_lowercase(),
            at.to_array().map(|v| (v * 2.0).round() as i64),
        );
        if seen(set, &known, &home, e) {
            continue;
        }
        let id = set.add(
            at,
            encounter_extent(&c.kind),
            Encounter {
                kind: c.kind.clone(),
                floor: c.floor,
                facing: m.transform_vector3(Vec3::Z),
                home: home.clone(),
                source: Some(e),
                ..Encounter::default()
            },
        );
        known.insert(home, id);
    }
    for (e, c) in world.iter_with::<CombatPatrolComponent>() {
        if !c.enabled {
            continue;
        }
        let (route, closed) = patrol_route(world, e);
        let (at, way) = route_at(&route, closed, 0.0);
        let home = (
            format!("patrol {}", c.kind.trim().to_ascii_lowercase()),
            at.to_array().map(|v| (v * 2.0).round() as i64),
        );
        if seen(set, &known, &home, e) {
            continue;
        }
        let id = set.add(
            at,
            encounter_extent(&c.kind),
            Encounter {
                kind: c.kind.clone(),
                floor: c.floor,
                facing: way,
                home: home.clone(),
                route,
                closed,
                speed: c.speed.max(0.0),
                source: Some(e),
                ..Encounter::default()
            },
        );
        known.insert(home, id);
    }
}

/// A patrol's route in the world: its entity's spline, sampled along its
/// curve, or just where the entity stands.
fn patrol_route(world: &World, e: Entity) -> (Vec<Vec3>, bool) {
    let m = world_matrix(world, e);
    match world.get::<somnium_core::SplineComponent>(e) {
        Some(spline) if spline.points.len() >= 2 => (
            spline
                .polyline()
                .into_iter()
                .map(|p| m.transform_point3(p))
                .collect(),
            spline.closed,
        ),
        _ => (vec![m.transform_point3(Vec3::ZERO)], false),
    }
}

/// Where a route has got to `along` metres in, and which way it is going: a
/// closed route goes round, an open one there and back.
fn route_at(route: &[Vec3], closed: bool, along: f32) -> (Vec3, Vec3) {
    let Some(&first) = route.first() else {
        return (Vec3::ZERO, Vec3::Z);
    };
    let mut legs: Vec<(Vec3, Vec3)> = route.windows(2).map(|w| (w[0], w[1])).collect();
    if closed && route.len() > 2 {
        legs.push((route[route.len() - 1], first));
    }
    let length: f32 = legs.iter().map(|(a, b)| a.distance(*b)).sum();
    if length <= 1e-3 || !along.is_finite() {
        return (first, Vec3::Z);
    }
    let (mut t, back) = if closed {
        (along.rem_euclid(length), false)
    } else {
        let t = along.rem_euclid(2.0 * length);
        if t > length {
            (2.0 * length - t, true)
        } else {
            (t, false)
        }
    };
    for (a, b) in &legs {
        let d = a.distance(*b);
        if t <= d && d > 1e-6 {
            let way = (*b - *a).normalize_or(Vec3::Z) * if back { -1.0 } else { 1.0 };
            return (a.lerp(*b, t / d), way);
        }
        t -= d;
    }
    let (a, b) = legs[legs.len() - 1];
    (
        b,
        (b - a).normalize_or(Vec3::Z) * if back { -1.0 } else { 1.0 },
    )
}

/// Patrols walk their routes while they sleep, so one is met where it would
/// have got to; a cleared one stays where it fell.
fn walk_asleep(set: &mut Dormancy<Encounter>, seconds: f32, hero: Option<Vec3>) {
    if seconds <= 0.0 || !seconds.is_finite() {
        return;
    }
    for i in 0..set.len() {
        let id = GroupId(i as u32);
        if set.is_awake(id) {
            continue;
        }
        let at = set.position(id);
        let Some(record) = set.data_mut(id) else {
            continue;
        };
        if record.hunter.is_some() {
            // A hunting band makes straight for him (on the ground plane).
            let (Some(at), Some(hero)) = (at, hero) else {
                continue;
            };
            if record.left.as_ref().is_some_and(Vec::is_empty) {
                continue;
            }
            let way = Vec3::new(hero.x - at.x, 0.0, hero.z - at.z);
            let step = way.normalize_or_zero() * (record.speed * seconds).min(way.length());
            record.facing = way.normalize_or(record.facing);
            set.move_to(id, Vec3::new(at.x + step.x, hero.y, at.z + step.z));
            continue;
        }
        if record.route.len() < 2 || record.left.as_ref().is_some_and(Vec::is_empty) {
            continue;
        }
        record.along += record.speed * seconds;
        let (at, way) = route_at(&record.route, record.closed, record.along);
        record.facing = way;
        set.move_to(id, at);
    }
}

/// Darkness from which wolves take up a trail (dusk under the Lid; night by
/// the clock).
const DUSK: f32 = 0.35;
/// How far off a shadowing pack keeps, and from how far it takes up a trail.
const SHADOW: f32 = 30.0;
const SHADOW_REACH: f32 = 150.0;
/// How close a patrol comes to its next point before it steps on, and how far.
const PATROL_NEAR: f32 = 5.0;
const PATROL_STEP: f32 = 6.0;

/// Awake groups not in a fight are led. A patrol walks on along its route
/// (formed up again where it is at each step, so a fight on the road is
/// fought there). At dusk a wolf pack shadows the protagonist from thirty
/// metres on its own side, its brains noticing, stalking and striking as they
/// would; by day, or once the protagonist is far, it goes back to where it
/// woke. A group with anyone in a fight is left to its brains.
fn lead(world: &World, run: &mut Run) {
    let Some(set) = run.encounters.as_mut() else {
        return;
    };
    let dusk = darkness(world) >= DUSK;
    let hero = run
        .player
        .and_then(|p| run.world.combatant(p))
        .map(|c| c.position);
    let awake: Vec<GroupId> = set.awake().collect();
    for id in awake {
        let Some(record) = set.data_mut(id) else {
            continue;
        };
        let alive: Vec<(CombatantId, Vec3, Vec3)> = record
            .members
            .iter()
            .zip(&record.starts)
            .filter_map(|(m, start)| {
                let c = run.world.combatant(*m).filter(|c| c.alive())?;
                Some((c.id, c.position, *start))
            })
            .collect();
        if alive.is_empty() {
            continue;
        }
        let fighting = alive.iter().any(|(m, ..)| {
            run.world
                .combatant(*m)
                .and_then(|c| c.brain.as_ref())
                .is_some_and(|b| b.mind.target.is_some())
        });
        if fighting {
            continue;
        }
        let centre = alive.iter().map(|a| a.1).sum::<Vec3>() / alive.len() as f32;
        let ground = |a: Vec3, b: Vec3| Vec3::new(a.x - b.x, 0.0, a.z - b.z);
        let trail = hero.filter(|h| {
            dusk && record.kind.trim().eq_ignore_ascii_case("wolves")
                && ground(centre, *h).length() <= SHADOW_REACH
        });
        let mut stepped = false;
        let hunting = record.hunter.is_some();
        let goal = if let (true, Some(h)) = (hunting, hero) {
            Some(h)
        } else if let Some(h) = trail {
            record.shadowing = true;
            Some(h + ground(centre, h).normalize_or(Vec3::X) * SHADOW)
        } else if record.route.len() >= 2 {
            record.shadowing = false;
            let (next, _) = route_at(&record.route, record.closed, record.along);
            if ground(centre, next).length() < PATROL_NEAR {
                record.along += PATROL_STEP;
                stepped = true;
            }
            Some(route_at(&record.route, record.closed, record.along).0)
        } else if record.shadowing {
            record.shadowing = false;
            None
        } else {
            continue;
        };
        for (i, (m, _, start)) in alive.iter().enumerate() {
            // Round the goal, the first (the chief, the alpha) in the middle;
            // back to where each woke when there is none.
            let home = goal.map_or(*start, |g| {
                let a = i as f32 * 2.4;
                g + Vec3::new(a.cos(), 0.0, a.sin()) * if i == 0 { 0.0 } else { 1.6 }
            });
            if let Some(brain) = run.world.combatant_mut(*m).and_then(|c| c.brain.as_mut()) {
                brain.mind.home = Some(home);
            }
            if let (true, Some(player), Some(h)) = (hunting, run.player, hero) {
                ai::alert(&mut run.world, *m, player, h);
            }
        }
        if stepped {
            let ids: Vec<CombatantId> = alive.iter().map(|a| a.0).collect();
            form_bands(&mut run.world, &ids);
        }
    }
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
    let elapsed = if dt.is_finite() { run.tended } else { 0.0 };
    run.tended = 0.0;
    let hero = run
        .player
        .and_then(|p| run.world.combatant(p))
        .map(|c| c.position);
    if let Some(set) = run.encounters.as_mut() {
        enlist_encounters(world, set);
        walk_asleep(set, elapsed, hero);
    }
    note_cleared(run);
    hunt(run);
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
    lead(world, run);
    if run.beasts_calm {
        // Those just woken were made hostile as every band is.
        run.world.relations.set(TeamId(0), TeamId(BEAST_TEAM), Attitude::Neutral);
    }
    if run.patched
        && let Some(physics) = physics
    {
        repatch(physics, run);
    }
}

/// How far off a hunting band sets out, how fast it walks to him asleep, and
/// how long after one is cleared the next sets out.
const HUNT_FROM: f32 = 260.0;
const HUNT_SPEED: f32 = 3.5;
const HUNT_AGAIN: f64 = 90.0;

/// The bands of the factions hunting the protagonist (`run.hunting`): one at
/// a time each, set out [`HUNT_FROM`] metres off and walking to him asleep, so
/// they wake into the fight at the usual distance; the next [`HUNT_AGAIN`]
/// seconds after one is cleared. A faction that stops hunting takes back its
/// bands still asleep (they never wake); an awake one stays until it sleeps.
fn hunt(run: &mut Run) {
    let Some(hero) = run
        .player
        .and_then(|p| run.world.combatant(p))
        .map(|c| c.position)
    else {
        return;
    };
    let now = run.world.time();
    let Some(set) = run.encounters.as_mut() else {
        return;
    };
    let (mut live, mut sent) = (BTreeSet::new(), BTreeSet::new());
    for i in 0..set.len() {
        let id = GroupId(i as u32);
        let awake = set.is_awake(id);
        let Some(record) = set.data_mut(id) else {
            continue;
        };
        let Some(faction) = record.hunter.clone() else {
            continue;
        };
        sent.insert(faction.clone());
        if record.cleared || record.left.as_ref().is_some_and(Vec::is_empty) {
            continue;
        }
        if !awake && !run.hunting.contains(&faction) {
            record.left = Some(Vec::new());
            continue;
        }
        live.insert(faction);
    }
    for faction in &run.hunting {
        if live.contains(faction) {
            run.hunt_due.remove(faction);
            continue;
        }
        let first = !sent.contains(faction);
        let due = *run
            .hunt_due
            .entry(faction.clone())
            .or_insert(if first { now } else { now + HUNT_AGAIN });
        let Some(kind) = faction_kind(faction).filter(|_| now >= due) else {
            continue;
        };
        run.hunt_due.remove(faction);
        // Round him by the golden angle, a new side each time.
        let a = run.hunts as f32 * 2.399_963;
        let way = Vec3::new(a.cos(), 0.0, a.sin());
        run.hunts += 1;
        set.add(
            hero + way * HUNT_FROM,
            encounter_extent(&kind),
            Encounter {
                facing: -way,
                home: (format!("hunt {faction}"), [i64::from(run.hunts), 0, 0]),
                hunter: Some(faction.clone()),
                speed: HUNT_SPEED,
                kind,
                ..Encounter::default()
            },
        );
    }
}

/// Awake encounters whose fighters have all fallen, each told once (its
/// scene entity and kind into `run.cleared`). Asleep it remembers no
/// survivors, so it never wakes again.
fn note_cleared(run: &mut Run) {
    let Some(set) = run.encounters.as_mut() else {
        return;
    };
    let awake: Vec<GroupId> = set.awake().collect();
    for id in awake {
        let Some(record) = set.data_mut(id) else {
            continue;
        };
        let fallen = |m: &CombatantId| run.world.combatant(*m).is_none_or(|c| !c.alive());
        if record.cleared || record.members.is_empty() || !record.members.iter().all(fallen) {
            continue;
        }
        record.cleared = true;
        if let Some(source) = record.source {
            run.cleared.push((source, record.kind.trim().to_ascii_lowercase()));
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
        build_encounter(world, &record.kind, at, record.facing, record.floor, run.content.as_deref())
            .unwrap_or_default();
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
    tune(&mut run.world, &record.kind, &members);
    let mut solids = Vec::new();
    let mut patched = None;
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
        // Its own ground to path on, where the scene baked none.
        if run.nav.is_none() || run.patched {
            if let Some(tile) = patch(physics, &run.water, id, at, encounter_extent(&record.kind)) {
                run.nav
                    .get_or_insert_with(NavWorld::default)
                    .stream_in(tile);
                run.patched = true;
                patched = Some(at);
            }
        }
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
        record.patch_at = patched;
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
        let route = set
            .data(id)
            .filter(|r| r.route.len() >= 2)
            .map(|r| route_at(&r.route, r.closed, r.along));
        if let Some((at, way)) = route {
            // A patrol sleeps where it had got to on its route.
            if let Some(record) = set.data_mut(id) {
                record.facing = way;
                record.shadowing = false;
            }
            set.move_to(id, at);
        } else if let (false, Some(at)) = (alive.is_empty(), set.position(id)) {
            let went = alive.iter().map(|a| a.2).sum::<Vec3>() / alive.len() as f32;
            set.move_to(id, at + Vec3::new(went.x, 0.0, went.z));
            if let Some(record) = set.data_mut(id) {
                record.shadowing = false;
            }
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
    if run.patched
        && let Some(nav) = run.nav.as_mut()
    {
        nav.stream_out([i64::from(id.0), PATCH_LAYER, 0]);
    }
    if let Some(record) = run.encounters.as_mut().and_then(|set| set.data_mut(id)) {
        record.patch_at = None;
    }
}

/// A woken group whose survivors have gone half its chase room from the
/// middle of its patch gets a new one round where they are now: one group a
/// look, so a valley of chases never lands on one frame.
fn repatch(physics: &somnium_physics::world::PhysicsWorld, run: &mut Run) {
    let Some(set) = &run.encounters else {
        return;
    };
    let moved = set.awake().find_map(|id| {
        let record = set.data(id)?;
        let at = record.patch_at?;
        let alive: Vec<Vec3> = record
            .members
            .iter()
            .filter_map(|m| run.world.combatant(*m))
            .filter(|c| c.alive())
            .map(|c| c.position)
            .collect();
        if alive.is_empty() {
            return None;
        }
        let centre = alive.iter().sum::<Vec3>() / alive.len() as f32;
        let gone = Vec3::new(centre.x - at.x, 0.0, centre.z - at.z).length();
        (gone > PATCH_REACH * 0.5).then(|| (id, centre, encounter_extent(&record.kind)))
    });
    let Some((id, centre, extent)) = moved else {
        return;
    };
    if let (Some(tile), Some(nav)) = (
        patch(physics, &run.water, id, centre, extent),
        run.nav.as_mut(),
    ) {
        nav.stream_in(tile);
    }
    if let Some(record) = run.encounters.as_mut().and_then(|set| set.data_mut(id)) {
        record.patch_at = Some(centre);
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
    /// `nav` holds woken encounters' patches (no navmesh was baked).
    patched: bool,
    readouts: HashMap<Entity, (i32, i32, i32, &'static str)>,
    /// Fighters and mounts drawn as their Skinned Bodies (the shapes stand in for the rest).
    skins: crate::combat_bodies::Skins,
    /// The project's content root: woken wolves and bandits wear WS3's bodies
    /// where it has them.
    content: Option<std::path::PathBuf>,
    /// Encounters cleared since the game last took them (see
    /// [`CombatDemo::take_cleared`]).
    cleared: Vec<(Entity, String)>,
    /// Factions hunting the protagonist (the game's, [`CombatDemo::set_hunting_factions`]);
    /// how many bands were sent; when each faction sends its next.
    hunting: BTreeSet<String>,
    hunts: u32,
    hunt_due: BTreeMap<String, f64>,
    /// The beasts are calm (the game's say): woken ones are made neutral too.
    beasts_calm: bool,
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

/// What the fight tells the game ([`CombatDemo::take_events`]), as entities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostEvent {
    /// The protagonist climbed into a saddle.
    Mounted { rider: Entity, mount: Entity },
    /// The protagonist reached for a mount the game refuses ([`CombatDemo::refused_mounts`]).
    MountRefused { mount: Entity },
    /// Someone hurt a combatant not hostile to them (a bystander, the watch, a horse).
    Hit { attacker: Entity, target: Entity },
}

/// The fight's steps a frame at most ([`CombatDemo::take_step`]).
const STEPS_PER_FRAME: u32 = 2;

/// The sandbox's runtime.
pub struct CombatDemo {
    run: Option<Run>,
    input: somnium_input::InputSystem,
    reader: CombatInput,
    cursor: Option<Vec2>,
    hud: Option<Hud>,
    /// Ctrl held: the wheel is the camera's, not the measure's.
    ctrl: bool,
    /// Draw the sandbox's text panels and the "?" and "!" marks. A game with
    /// its own HUD (Rimeborn's) turns it off and reads [`CombatDemo::hud_view`]
    /// and [`CombatDemo::take_hud_events`] instead.
    pub show_hud: bool,
    /// The fight's latest events, for a game's HUD (the newest [`HUD_EVENTS`]).
    hud_events: VecDeque<CombatEvent>,
    /// Mounts the protagonist may not take now, the game's say (someone else's horse before the story allows it).
    pub refused_mounts: std::collections::HashSet<Entity>,
    /// The protagonist sits in a boat at this seat (feet, its facing the bow), carried: the boats' host
    /// (`boats.rs`) rows it and sets this every fixed step; WASD and E are the boat's meanwhile.
    pub aboard: Option<Mat4>,
    /// What he plays in the boat and at what rate (its kind's: rowing, or at the tiller), set with `aboard`.
    pub aboard_clip: Option<(String, f32)>,
    /// E was the boats' this frame (boarding or stepping off), not a mount's.
    pub take_e: bool,
    /// Points brains think near as well as the protagonist and the camera (a second place the game holds streamed).
    pub focus_points: Vec<Vec3>,
    /// The protagonist's work clip (`A|B`, looped while he stands idle: chopping, fishing), from the game.
    pub player_clip: Option<String>,
    /// The frame the fight last stepped in, how many steps it took there, and the steps it let go (the measure).
    frame: u64,
    steps: u32,
    skipped: u64,
    events: Vec<HostEvent>,
    /// Factions the game has turned against the protagonist, and those hunting
    /// him ([`CombatDemo::set_hostile_factions`], [`CombatDemo::set_hunting_factions`]).
    hostile_factions: BTreeSet<String>,
    hunting_factions: BTreeSet<String>,
    /// The left button's click-move and attack-move (Diablo IV's mouse).
    click: crate::click_move::ClickMove,
    /// Whether the beasts (wolves, harrowcats, all of team [`BEAST_TEAM`])
    /// fight the protagonist ([`CombatDemo::set_beasts_hostile`]).
    beasts_hostile: bool,
}

/// The game's law on the fight: each faction hostile to the party when it is
/// turned or hunting, neutral otherwise; the hunters to send.
fn apply_law(run: &mut Run, hostile: &BTreeSet<String>, hunting: &BTreeSet<String>, beasts: bool) {
    run.beasts_calm = !beasts;
    let attitude = if beasts { Attitude::Hostile } else { Attitude::Neutral };
    run.world.relations.set(TeamId(0), TeamId(BEAST_TEAM), attitude);
    for faction in FACTIONS {
        let Some(team) = faction_team(faction) else {
            continue;
        };
        let turned = hostile.contains(faction) || hunting.contains(faction);
        let attitude = if turned { Attitude::Hostile } else { Attitude::Neutral };
        run.world.relations.set(TeamId(0), team, attitude);
    }
    run.hunting.clone_from(hunting);
}

/// The known faction names among `factions`, lower case.
fn factions_of<S: AsRef<str>>(factions: &[S]) -> BTreeSet<String> {
    factions
        .iter()
        .map(|f| f.as_ref().trim().to_ascii_lowercase())
        .filter(|f| faction_team(f).is_some())
        .collect()
}

/// How many of the fight's events wait for a game's HUD before the oldest go.
pub const HUD_EVENTS: usize = 64;

/// What a game's HUD reads of a running fight (see [`CombatDemo::hud_view`]).
pub struct HudView<'a> {
    /// The fight: every fighter, their vitals, actions and statuses.
    pub world: &'a CombatWorld,
    /// The protagonist, if the scene has one.
    pub player: Option<CombatantId>,
    /// Who has noticed the protagonist, how far, and the seconds their mark
    /// has left.
    pub marks: &'a BTreeMap<CombatantId, (somnium_combat::ai::Awareness, f32)>,
    /// The fight's camera, its world matrix (as [`CombatDemo::view`] gives).
    pub camera: Option<Mat4>,
}

/// Keep the newest of a step's `events` for a game's HUD, `cap` at most.
fn keep_hud_events(queue: &mut VecDeque<CombatEvent>, events: &[CombatEvent], cap: usize) {
    queue.extend(events.iter().cloned());
    while queue.len() > cap {
        queue.pop_front();
    }
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
            show_hud: true,
            hud_events: VecDeque::new(),
            refused_mounts: Default::default(),
            aboard: None,
            aboard_clip: None,
            take_e: false,
            focus_points: Vec::new(),
            player_clip: None,
            frame: u64::MAX,
            steps: 0,
            skipped: 0,
            events: Vec::new(),
            hostile_factions: BTreeSet::new(),
            hunting_factions: BTreeSet::new(),
            click: Default::default(),
            beasts_hostile: true,
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
                if *button == somnium_core::MouseButton::Left {
                    self.click.button(*state == InputState::Pressed);
                }
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

    /// The running fight as a game's HUD reads it; `None` when no fight runs.
    pub fn hud_view<'a>(&'a self, world: &World) -> Option<HudView<'a>> {
        let run = self.run.as_ref()?;
        Some(HudView {
            world: &run.world,
            player: run.player,
            marks: &run.marks,
            camera: self.view(world),
        })
    }

    /// The fight's events since the last call (the newest [`HUD_EVENTS`]), for
    /// a game's HUD: damage, healing, parries, broken poise, statuses, deaths.
    pub fn take_hud_events(&mut self) -> Vec<CombatEvent> {
        self.hud_events.drain(..).collect()
    }

    /// The scene's Combat Encounter and Combat Patrol entities (each with its
    /// kind, lowercased: `wolves`, `bandits`, `lampmen`, `wilds`) whose
    /// encounter woke and whose fighters have all fallen since the last call.
    /// Each comes once: a cleared encounter never wakes again.
    /// The factions (names in [`FACTIONS`]) the protagonist's deeds have
    /// turned: their fighters fight him on sight; every other faction is
    /// neutral until he strikes one of them. A game sets this as its story's
    /// facts change (Rimeborn every half second: a Lamp whose favour is low,
    /// the watch at Hue 3). Unknown names are ignored. Kept across Plays.
    pub fn set_hostile_factions<S: AsRef<str>>(&mut self, factions: &[S]) {
        self.hostile_factions = factions_of(factions);
        if let Some(run) = &mut self.run {
            apply_law(run, &self.hostile_factions, &self.hunting_factions, self.beasts_hostile);
        }
    }

    /// The factions hunting the protagonist (hostile as well): each sends a
    /// band from out of sight that walks to him and fights (the knights or
    /// the Lance at Hue 4 and 5), another a while after one is cleared.
    /// Whether the beasts (wolves, harrowcats, every creature of team
    /// [`BEAST_TEAM`]) fight the protagonist: on by default; off, they let
    /// him be unless he strikes one (STORY_GATES: in Act I animals never
    /// fight him). Kept across Plays.
    pub fn set_beasts_hostile(&mut self, hostile: bool) {
        self.beasts_hostile = hostile;
        if let Some(run) = &mut self.run {
            apply_law(run, &self.hostile_factions, &self.hunting_factions, self.beasts_hostile);
        }
    }

    pub fn set_hunting_factions<S: AsRef<str>>(&mut self, factions: &[S]) {
        self.hunting_factions = factions_of(factions);
        if let Some(run) = &mut self.run {
            apply_law(run, &self.hostile_factions, &self.hunting_factions, self.beasts_hostile);
        }
    }

    /// The boss fight under way (Act VI's Ledger Engine), for a game's HUD:
    /// its name, health and phase, the parts the player breaks, and the Hold
    /// coming (its ring, the readable tell). `None` with no boss awake.
    #[must_use]
    pub fn boss_view(&self) -> Option<BossView> {
        let run = self.run.as_ref()?;
        let set = run.encounters.as_ref()?;
        set.awake().find_map(|id| {
            let record = set.data(id)?;
            let state = record.engine.as_ref()?;
            let alive = |name: &str| {
                record
                    .members
                    .iter()
                    .filter_map(|m| run.world.combatant(*m))
                    .find(|c| c.name == name)
                    .filter(|c| c.alive())
            };
            let keeper = alive(ENGINE_KEEPER)?;
            let health = keeper.vitals.health.current / keeper.vitals.health.max.max(1.0);
            Some(BossView {
                name: ENGINE_KEEPER.into(),
                health,
                phase: if health > 2.0 / 3.0 {
                    1
                } else if health > 1.0 / 3.0 {
                    2
                } else {
                    3
                },
                parts: ENGINE_BOARDS.iter().map(|b| ((*b).to_string(), alive(b).is_some())).collect(),
                hold: state.tell.map(|(centre, left, _)| (centre, HOLD_RADIUS, left)),
            })
        })
    }

    pub fn take_cleared(&mut self) -> Vec<(Entity, String)> {
        self.run
            .as_mut()
            .map(|run| std::mem::take(&mut run.cleared))
            .unwrap_or_default()
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
            expand_encounters(ctx.world, Some(&ctx.config.content_root))
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
            // Fighters that carry a Skinned Body draw through it (Caelan, a horse).
            run.skins = crate::combat_bodies::Skins::new(Some(&ctx.config.content_root));
            run.content = Some(ctx.config.content_root.clone());
            run.skins.dress(ctx.world, &run.bodies, &run.world, &mut run.spawned);
            apply_law(run, &self.hostile_factions, &self.hunting_factions, self.beasts_hostile);
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
        run.skins.end(ctx.world);
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
        let ray = camera.zip(self.cursor).and_then(|(camera, cursor)| {
            let (view, _) = somnium_core::camera_view_from_world(camera);
            let (x, y, w, h) = ctx.ui.viewport_physical_rect(1.0);
            let size = Vec2::new(w as f32, h as f32);
            let projection =
                Mat4::perspective_rh(VIEW_FOV.to_radians(), size.x / size.y.max(1.0), 0.1, 1000.0);
            cursor_ray(
                cursor - Vec2::new(x as f32, y as f32),
                size,
                view,
                projection,
            )
        });
        let aim = ray.and_then(|(o, d)| ray_plane(o, d, feet));
        // Where a click sends him: the ground the cursor is on (up a hill, down a gully), else its height's plane.
        let ground = ray.and_then(|(o, d)| {
            let reach = o + d.normalize_or_zero() * CLICK_REACH;
            JoltQuery(ctx.physics).sweep(o, reach, 0.05).map(|h| o.lerp(reach, h.fraction))
        }).or(aim);
        let under = ray.and_then(|(o, d)| crate::click_move::pick(&run.world, player, o, d));
        let forward = camera.map_or(Vec3::Z, |m| -m.z_axis.truncate());
        let mut cmd = self.reader.command(
            &self.input,
            ctx.dt(),
            AimView {
                aim,
                camera_forward: forward,
            },
        );
        // Diablo IV's left button: click-move and attack-move (WASD drops them).
        let cells = &ctx.navigation.cells.world;
        let nav = cells.cells().next().is_some().then_some(cells).or(run.nav.as_ref());
        self.click.steer(&run.world, nav, player, under, ground, ctx.dt(), &mut cmd);
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
        self.events.extend(gate_mount(run, &self.refused_mounts, player, &mut cmd));
        if std::mem::take(&mut self.take_e) || self.aboard.is_some() {
            cmd.mount = false;
        }
        if self.aboard.is_some() {
            // The oars have the movement keys.
            cmd.move_dir = Vec3::ZERO;
            cmd.sprint = false;
            cmd.dodge = false;
            cmd.climb = false;
        }
        run.world.command(player, cmd);
    }

    /// The body to outline: the hostile under the cursor (its skinned body, else its entity).
    pub fn hovered_body(&self) -> Option<Entity> {
        let run = self.run.as_ref()?;
        let id = self.click.hovered()?;
        run.skins.body_of(id).or_else(|| run.bodies.get(&id).copied())
    }

    /// The protagonist's feet and whether they ride, while a fight runs.
    pub fn player_feet(&self) -> Option<(Vec3, bool)> {
        let run = self.run.as_ref()?;
        let c = run.world.combatant(run.player?)?;
        Some((c.position, c.mount.is_some()))
    }

    /// Put the protagonist back from a save: at `feet`, facing `yaw` (about +Y,
    /// +Z ahead), with `health`, on a mount if `riding` (the nearest free one,
    /// brought beside them) or on foot. Whatever they were doing is dropped, so
    /// a load mid-swing does not land the swing. False with no fight running.
    pub fn restore_player(&mut self, feet: Vec3, yaw: f32, health: f32, riding: bool) -> bool {
        self.aboard = None;
        let Some(run) = self.run.as_mut() else {
            return false;
        };
        let Some(player) = run.player else {
            return false;
        };
        if run.world.combatant(player).is_some_and(|c| c.mount.is_some()) {
            somnium_combat::mount::dismount(&mut run.world, player);
        }
        let facing = Vec3::new(yaw.sin(), 0.0, yaw.cos());
        let Some(c) = run.world.combatant_mut(player) else {
            return false;
        };
        c.position = feet;
        c.velocity = Vec3::ZERO;
        c.facing = facing;
        c.action = Action::Idle;
        c.lock = None;
        c.command = somnium_combat::Command::default();
        c.riposte_until = 0.0;
        c.vitals.health.current = health.clamp(0.0, c.vitals.health.max);
        let mut place = |id: CombatantId, at: Vec3| {
            if let Some(walker) = run.walkers.get_mut(&id) {
                walker.feet = at;
                walker.velocity = Vec3::ZERO;
                walker.mode = Mode::Air;
            }
        };
        place(player, feet);
        if riding && let Some(steed) = somnium_combat::mount::nearest_free(&run.world, player) {
            let beside = feet + Vec3::new(facing.z, 0.0, -facing.x) * 1.2;
            if let Some(m) = run.world.combatant_mut(steed) {
                m.position = beside;
                m.velocity = Vec3::ZERO;
                m.facing = facing;
            }
            place(steed, beside);
            let _ = somnium_combat::mount::mount(&mut run.world, player, steed);
        }
        true
    }

    /// Make the fighter at `entity` the played one (the dual protagonists'
    /// switch, DP-1, at `hand_over`): input, lock-on and the HUD follow him, his
    /// own brain gone. The one left keeps his body as an ordinary fighter of his
    /// style (his authored archetype's brain, home where he stands), his command
    /// and lock-on dropped. False when no fight runs or `entity` is not one of
    /// its fighters.
    pub fn set_player(&mut self, world: &World, entity: Entity) -> bool {
        self.aboard = None;
        let Some(run) = self.run.as_mut() else {
            return false;
        };
        let Some(new) = run.bodies.iter().find_map(|(id, e)| (*e == entity).then_some(*id)) else {
            return false;
        };
        if run.player == Some(new) {
            return true;
        }
        if let Some(c) = run.world.combatant_mut(new) {
            c.brain = None;
            c.command = somnium_combat::Command::default();
        }
        // A slowed world comes back to time at the flip.
        run.world.end_focus();
        if let Some(old) = run.player {
            let component = run.bodies.get(&old).and_then(|e| world.get::<CombatantComponent>(*e)).cloned();
            if let Some(c) = run.world.combatant_mut(old) {
                c.command = somnium_combat::Command::default();
                c.lock = None;
                // His own style and spells; his kit (the protagonist's perks) is his from enlisting, whoever is played.
                let brain = component.map_or_else(
                    || {
                        let mut b = ai::BrainDesc::new(Archetype::Companion);
                        b.home = Some(c.position);
                        b
                    },
                    |k| k.brain_desc(c.position),
                );
                c.brain = Some(ai::Brain::new(brain, old));
            }
        }
        run.player = Some(new);
        true
    }

    /// Off a boat: the protagonist stands (or falls, or swims) from `feet`.
    pub fn disembark(&mut self, feet: Vec3) {
        self.aboard = None;
        let Some(run) = self.run.as_mut() else {
            return;
        };
        let Some(player) = run.player else {
            return;
        };
        if let Some(c) = run.world.combatant_mut(player) {
            c.position = feet;
            c.velocity = Vec3::ZERO;
        }
        if let Some(walker) = run.walkers.get_mut(&player) {
            walker.feet = feet;
            walker.velocity = Vec3::ZERO;
            walker.mode = Mode::Air;
        }
    }

    /// Whether the fight takes this fixed step of `frame`. A slow frame runs
    /// several to catch up (up to six at 60 Hz: the engine caps a frame at
    /// 0.1 s); the fight takes [`STEPS_PER_FRAME`] of them, so one slow frame
    /// cannot make the next slower. ponytail: past that the fight runs slow for
    /// the frame (real time up to 33 ms frames); owe the time to the next
    /// frame's step if a slowed hitch ever reads badly.
    fn take_step(&mut self, frame: u64) -> bool {
        if frame != self.frame {
            self.frame = frame;
            self.steps = 0;
        }
        self.steps += 1;
        if self.steps > STEPS_PER_FRAME {
            self.skipped += 1;
            return false;
        }
        true
    }

    /// What the fight told the game since it last asked.
    pub fn take_events(&mut self) -> Vec<HostEvent> {
        std::mem::take(&mut self.events)
    }

    /// One fixed step of the fight, then everything it changed onto the scene.
    pub fn fixed(&mut self, ctx: &mut EngineContext) {
        if self.run.is_none() || !self.take_step(ctx.time.frame_count()) {
            return;
        }
        let Some(run) = &mut self.run else {
            return;
        };
        let dt = ctx.simulation.fixed_delta_seconds;
        // Brains think by their distance from what matters (the combat
        // world's LOD): the protagonist, and the fight's camera on the ground.
        let mut focus: Vec<Vec3> = run
            .player
            .and_then(|p| run.world.combatant(p))
            .map(|c| c.position)
            .into_iter()
            .collect();
        if let Some(camera) = run.camera {
            focus.push(world_matrix(ctx.world, camera).transform_point3(Vec3::ZERO));
        }
        focus.extend_from_slice(&self.focus_points);
        run.world.set_focus(&focus);
        let editor = &ctx.navigation.cells.world;
        let nav = editor
            .cells()
            .next()
            .is_some()
            .then_some(editor)
            .or(run.nav.as_ref());
        let query = JoltQuery(ctx.physics);
        let rode = run.player.and_then(|p| run.world.combatant(p)).and_then(|c| c.mount);
        run.world.step(dt, &query, nav);
        let terrain = JoltTerrain {
            physics: ctx.physics,
            water: &run.water,
        };
        for (id, event) in walk(&mut run.world, &mut run.walkers, dt, &terrain) {
            log_locomotion(run, id, event);
        }
        // In a boat the protagonist is carried at its seat, facing the bow.
        if let (Some(player), Some(seat)) = (run.player, self.aboard) {
            let (_, turn, feet) = seat.to_scale_rotation_translation();
            if let Some(c) = run.world.combatant_mut(player) {
                c.position = feet;
                c.velocity = Vec3::ZERO;
                c.facing = turn * Vec3::Z;
            }
            if let Some(walker) = run.walkers.get_mut(&player) {
                walker.feet = feet;
                walker.velocity = Vec3::ZERO;
                walker.mode = Mode::Air;
            }
        }
        run.skins.seated = run.player.zip(self.aboard.and(self.aboard_clip.clone()));
        run.skins.work = run.player.zip(self.player_clip.clone());
        let events = run.world.drain_events();
        keep_hud_events(&mut self.hud_events, &events, HUD_EVENTS);
        self.events.extend(news(run, rode, &events));
        for event in &events {
            react(ctx, run, event);
        }
        run.marks.retain(|_, (_, left)| {
            *left -= dt;
            *left > 0.0
        });
        tend_encounters(ctx.world, Some(&mut *ctx.physics), run, dt);
        engine_tick(ctx.world, run, dt);
        write_scene(ctx, run);
        // Bodies for fighters that woke this frame, and every body's pose.
        run.skins.dress(ctx.world, &run.bodies, &run.world, &mut run.spawned);
        run.skins.write(ctx.world, &run.world, &run.walkers, dt);
    }

    /// HUD text (once a frame, before the UI is drawn).
    pub fn render(&mut self, ctx: &mut EngineContext) {
        let Some(run) = &self.run else {
            return;
        };
        if !self.show_hud {
            return;
        }
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
            && self.show_hud
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
            "steps_skipped": self.skipped,
            "bodies": run.skins.count(),
            "time": run.world.time(),
            "locomotion": locomotion,
            "fighters": fighters,
            "log": run.log,
            // Where the left button sent him: a ground point, or the hostile he is to strike.
            "click": match self.click.goal() {
                Some(crate::click_move::Goal::Ground(at)) => json!({ "ground": [at.x, at.y, at.z] }),
                Some(crate::click_move::Goal::Foe(id)) => json!({ "foe": id.0 }),
                None => Value::Null,
            },
            "hovered": self.click.hovered().map(|id| id.0),
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
        patched: false,
        readouts: HashMap::new(),
        skins: Default::default(),
        content: None,
        cleared: Vec::new(),
        hunting: BTreeSet::new(),
        hunts: 0,
        hunt_due: BTreeMap::new(),
        beasts_calm: false,
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
/// behind (knives flank on their own). The Lance's riders are in their saddles.
fn organise(combat: &mut CombatWorld, ids: &[CombatantId], leaders: &[CombatantId]) {
    seat_band(combat, ids);
    let mut teams: Vec<TeamId> = ids
        .iter()
        .filter_map(|id| combat.combatant(*id).map(|c| c.team))
        .collect();
    teams.sort_unstable();
    teams.dedup();
    for &t in &teams {
        // A faction's attitude is the game's (`apply_law`).
        if t != TeamId(0) && t.0 < FACTION_TEAM {
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
    form_bands(combat, ids);
}

/// Each side's band among `ids` formed up where it stands, its melee in a
/// line and its bows, rifles and mages behind (knives flank on their own).
fn form_bands(combat: &mut CombatWorld, ids: &[CombatantId]) {
    let mut teams: Vec<TeamId> = ids
        .iter()
        .filter_map(|id| combat.combatant(*id).map(|c| c.team))
        .filter(|t| *t != TeamId(0))
        .collect();
    teams.sort_unstable();
    teams.dedup();
    for t in teams {
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
    let pushes = combat.separations(&ids);
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
/// The protagonist's reach for the nearest free mount, refused where the game
/// says (the command's `mount` dropped, so the step leaves it be).
fn gate_mount(
    run: &Run,
    refused: &std::collections::HashSet<Entity>,
    player: CombatantId,
    cmd: &mut somnium_combat::Command,
) -> Option<HostEvent> {
    if !cmd.mount || refused.is_empty() || run.world.combatant(player)?.mount.is_some() {
        return None;
    }
    let mount = *run.bodies.get(&somnium_combat::mount::nearest_free(&run.world, player)?)?;
    refused.contains(&mount).then(|| {
        cmd.mount = false;
        HostEvent::MountRefused { mount }
    })
}

/// A step's news for the game: the protagonist in a saddle it was not in
/// (`rode`, before the step), and hits on combatants not hostile to who hit.
fn news(run: &Run, rode: Option<CombatantId>, events: &[CombatEvent]) -> Vec<HostEvent> {
    let entity = |id: &CombatantId| run.bodies.get(id).copied();
    let mut out: Vec<HostEvent> = events
        .iter()
        .filter_map(|e| match e {
            CombatEvent::Damaged {
                target,
                source: Some(by),
                ..
            } if by != target && !run.world.hostile(*by, *target) => Some(HostEvent::Hit {
                attacker: entity(by)?,
                target: entity(target)?,
            }),
            _ => None,
        })
        .collect();
    if let Some(p) = run.player
        && let Some(m) = run.world.combatant(p).and_then(|c| c.mount)
        && rode != Some(m)
        && let (Some(rider), Some(mount)) = (entity(&p), entity(&m))
    {
        out.push(HostEvent::Mounted { rider, mount });
    }
    out
}

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
    if c.name.starts_with("Glyph-board") {
        // A broken board stands where it broke.
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

    /// A fighter carrying a Skinned Body draws through it: a driven body of
    /// its own at its feet, its shapes hidden, its sword sheathed until it
    /// fights; a mount gets a saddle; Stop gives everything back.
    #[test]
    fn fighters_with_bodies_draw_through_them() {
        use crate::skinned_crowd::{SkinnedBody, SkinnedSocket};
        let mut world = World::new();
        spawn_sandbox(&mut world, &json!({})).expect("preset builds");
        let pick = |world: &World, f: &dyn Fn(&CombatantComponent, &str) -> bool| {
            world
                .iter_with::<CombatantComponent>()
                .find(|(e, c)| f(c, world.get::<Name>(*e).map_or("", |n| n.as_str())))
                .map(|(e, _)| e)
                .unwrap()
        };
        let hero = pick(&world, &|c, _| c.player);
        let elk = pick(&world, &|_, n| n == "Frost-elk");
        for e in [hero, elk] {
            world
                .insert_component(e, SkinnedBody::default())
                .unwrap();
        }
        somnium_core::propagate_transforms(&mut world);
        let mut run = build(&world).expect("a fight");
        run.skins = crate::combat_bodies::Skins::new(None);
        run.skins
            .dress(&mut world, &run.bodies, &run.world, &mut run.spawned);
        assert_eq!(run.skins.count(), 2);
        assert!(world.get::<SkinnedBody>(hero).is_none());
        assert!(world.get::<somnium_core::EditorFlags>(hero).is_some_and(|f| f.hidden), "its cylinder is hidden");
        let body_of = |world: &World, name: &str| {
            world
                .iter_with::<SkinnedBody>()
                .find(|(e, _)| world.get::<Name>(*e).is_some_and(|n| n.as_str() == name))
                .map(|(e, b)| (e, b.clone()))
                .unwrap()
        };
        let hero_name = world.get::<Name>(hero).unwrap().as_str().to_string();
        let (body, look) = body_of(&world, &format!("{hero_name} body"));
        assert!(look.driven);
        let sword = world
            .iter_with::<SkinnedSocket>()
            .find(|(e, _)| world.get::<Parent>(*e).is_some_and(|p| p.entity == body))
            .map(|(e, s)| (e, s.joint.clone()))
            .expect("a sword socket");
        assert_eq!(sword.1, "sheath");
        let (elk_body, _) = body_of(&world, "Frost-elk body");
        assert!(
            world
                .iter_with::<SkinnedSocket>()
                .any(|(e, s)| s.joint == "saddle" && world.get::<Parent>(e).is_some_and(|p| p.entity == elk_body))
        );
        let terrain = boxes(&world, &run);
        for _ in 0..60 {
            let open = somnium_combat::OpenField;
            run.world.step(1.0 / 60.0, &open, run.nav.as_ref());
            walk(&mut run.world, &mut run.walkers, 1.0 / 60.0, &terrain);
            run.skins
                .write(&mut world, &run.world, &run.walkers, 1.0 / 60.0);
        }
        let player = run.player.unwrap();
        let feet = run.world.combatant(player).unwrap().position;
        let at = world.get::<Transform>(body).unwrap().translation;
        assert!(at.distance(feet) < 1e-3, "the body stands at the feet");
        assert!(!world.get::<SkinnedBody>(body).unwrap().clip.is_empty());
        // Raising the guard draws the blade.
        run.world.combatant_mut(player).unwrap().action = Action::Guard { t: 0.1 };
        run.skins
            .write(&mut world, &run.world, &run.walkers, 1.0 / 60.0);
        assert_eq!(world.get::<SkinnedSocket>(sword.0).unwrap().joint, "weapon_r");
        assert!(world.get::<SkinnedBody>(body).unwrap().clip.contains("Block"));
        let skins = std::mem::take(&mut run.skins);
        skins.end(&mut world);
        assert!(world.get::<SkinnedBody>(hero).is_some());
        assert!(!world.get::<somnium_core::EditorFlags>(hero).is_some_and(|f| f.hidden), "shown again at Stop");
    }

    /// Woken wolves and bandits wear WS3's bodies where the project has them
    /// (the rime wolf, the alpha its own, four different bandits), on their
    /// Combatant entities for `combat_bodies` to draw; the horse and a project
    /// without the files keep their shapes.
    #[test]
    fn woken_wolves_and_bandits_wear_ws3_bodies() {
        use crate::skinned_crowd::SkinnedBody;
        let content = std::env::temp_dir().join("somnium_combat_presets");
        let mut files = vec![
            "creatures/rime_wolf/rime_wolf".to_string(),
            "creatures/rime_wolf/rime_wolf_lod1".to_string(),
            "creatures/rime_wolf_alpha/rime_wolf_alpha".to_string(),
        ];
        files.extend((0..BANDITS).map(|n| format!("characters/bandit_{n:02}/bandit_{n:02}")));
        for f in &files {
            let path = content.join(format!("{f}.glb"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"").unwrap();
        }
        let wake_wilds = |content: Option<std::path::PathBuf>| {
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
                Name::new("Wilds"),
                Transform::from_translation(Vec3::new(40.0, 0.0, 0.0)),
                WorldTransform::identity(),
                CombatEncounterComponent {
                    kind: "wilds".into(),
                    ..CombatEncounterComponent::default()
                },
            ));
            somnium_core::propagate_transforms(&mut world);
            let mut run = build(&world).expect("a fight");
            run.content = content;
            run.encounters = Some(encounters_asleep(&world));
            tend_encounters(&mut world, None, &mut run, f32::INFINITY);
            let worn: BTreeMap<String, Option<SkinnedBody>> = run
                .world
                .combatants()
                .filter(|c| !c.name.starts_with("Protagonist"))
                .map(|c| (c.name.clone(), world.get::<SkinnedBody>(run.bodies[&c.id]).cloned()))
                .collect();
            (world, run, worn)
        };
        let (mut world, mut run, worn) = wake_wilds(Some(content));
        assert_eq!(worn.len(), 10, "{:?}", worn.keys());
        let id = |f: &str| AssetId::from_relative_path(f);
        let alpha = worn["Rime wolf (alpha)"].clone().expect("the alpha's body");
        assert_eq!(alpha.source, id("creatures/rime_wolf_alpha/rime_wolf_alpha.glb"));
        assert_eq!((alpha.lod1, alpha.lod2), (AssetId::NONE, AssetId::NONE), "no levels on disk");
        for n in 1..=4 {
            let wolf = worn[&format!("Rime wolf {n}")].clone().expect("a wolf's body");
            assert_eq!(wolf.source, id("creatures/rime_wolf/rime_wolf.glb"));
            assert_eq!(wolf.lod1, id("creatures/rime_wolf/rime_wolf_lod1.glb"));
            assert_eq!(wolf.lod2, AssetId::NONE);
        }
        let faces: std::collections::BTreeSet<AssetId> = worn
            .iter()
            .filter(|(name, _)| name.starts_with("Bandit"))
            .map(|(_, b)| b.as_ref().expect("a bandit's body").source)
            .collect();
        assert_eq!(faces.len(), 4, "four different bandits");
        assert!(worn["Horse"].is_none(), "the horse keeps its shapes");
        // combat_bodies dresses every one of them.
        run.skins = crate::combat_bodies::Skins::new(None);
        run.skins
            .dress(&mut world, &run.bodies, &run.world, &mut run.spawned);
        assert_eq!(run.skins.count(), 9);
        // A project without WS3's files: shapes only.
        let (_, _, bare) = wake_wilds(None);
        assert!(bare.values().all(Option::is_none));
    }

    /// An encounter whose fighters all fall is told once, as the scene's
    /// entity (the latest copy, after its cell streamed back in) and its kind,
    /// and never wakes again.
    #[test]
    fn a_cleared_camp_is_told_once_and_stays_cleared() {
        let mut world = World::new();
        let root = world.spawn((Name::new("Scene"), Transform::default(), WorldTransform::identity()));
        fighter(&mut world, root, "Protagonist", Vec3::ZERO, Vec3::X, CombatantComponent::protagonist());
        let camp = |world: &mut World| {
            world.spawn((
                Name::new("Encounter bandits 12"),
                Transform::from_translation(Vec3::new(40.0, 0.0, 0.0)),
                WorldTransform::identity(),
                CombatEncounterComponent {
                    kind: "Bandits".into(),
                    ..CombatEncounterComponent::default()
                },
            ))
        };
        let first = camp(&mut world);
        somnium_core::propagate_transforms(&mut world);
        let mut run = build(&world).expect("a fight");
        run.encounters = Some(encounters_asleep(&world));
        tend_encounters(&mut world, None, &mut run, f32::INFINITY);
        let bandits: Vec<CombatantId> = run
            .world
            .combatants()
            .filter(|c| c.name.starts_with("Bandit"))
            .map(|c| c.id)
            .collect();
        assert_eq!(bandits.len(), 4);
        // Its cell streams out and back in: a new entity for the same camp.
        world.despawn(first);
        let copy = camp(&mut world);
        somnium_core::propagate_transforms(&mut world);
        let mut demo = CombatDemo {
            run: Some(run),
            ..CombatDemo::default()
        };
        let run = demo.run.as_mut().unwrap();
        tend_encounters(&mut world, None, run, TEND_EVERY);
        assert!(demo.take_cleared().is_empty(), "still standing");
        let run = demo.run.as_mut().unwrap();
        for (n, b) in bandits.iter().enumerate() {
            run.world.combatant_mut(*b).unwrap().action = Action::Dead;
            tend_encounters(&mut world, None, run, TEND_EVERY);
            if n + 1 < bandits.len() {
                assert!(run.cleared.is_empty(), "one still stands");
            }
        }
        assert_eq!(demo.take_cleared(), vec![(copy, "bandits".to_string())]);
        assert!(demo.take_cleared().is_empty(), "taken once");
        // Away (asleep) and back: it does not wake, and is not told again.
        let run = demo.run.as_mut().unwrap();
        let hero = run.player.unwrap();
        for at in [Vec3::new(0.0, 0.0, 600.0), Vec3::ZERO] {
            run.world.combatant_mut(hero).unwrap().position = at;
            run.walkers.get_mut(&hero).unwrap().feet = at;
            tend_encounters(&mut world, None, run, TEND_EVERY);
        }
        assert!(run.world.combatants().all(|c| !c.name.starts_with("Bandit") || !c.alive()));
        assert!(demo.take_cleared().is_empty());
    }

    /// A Lamp crew: a boss, two knives and a rifleman strung along its
    /// street, each by its own name (survivors are remembered by name), in
    /// four different bodies where the project has them.
    #[test]
    fn a_lamp_crew_stands_along_its_street() {
        use crate::skinned_crowd::SkinnedBody;
        let content = std::env::temp_dir().join("somnium_combat_crew");
        for n in 0..34 {
            let path = content.join(format!("characters/lampman_{n:02}/lampman_{n:02}.glb"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"").unwrap();
        }
        let mut world = World::new();
        let made = build_encounter(&mut world, "lampmen", Vec3::new(5.0, 0.0, 9.0), Vec3::X, false, Some(&content))
            .expect("a crew");
        let crew: Vec<(String, CombatantComponent, Option<SkinnedBody>)> = made
            .iter()
            .filter_map(|e| {
                let c = world.get::<CombatantComponent>(*e)?.clone();
                let name = world.get::<Name>(*e)?.as_str().to_string();
                Some((name, c, world.get::<SkinnedBody>(*e).cloned()))
            })
            .collect();
        let names: std::collections::BTreeSet<&str> = crew.iter().map(|(n, ..)| n.as_str()).collect();
        assert_eq!((crew.len(), names.len()), (4, 4), "{names:?}");
        assert_eq!(crew.iter().filter(|(_, c, _)| c.leader).count(), 1);
        assert_eq!(
            crew.iter()
                .filter(|(_, c, _)| c.weapon_kind() == WeaponKind::Knife)
                .count(),
            3
        );
        assert!(crew.iter().any(|(_, c, _)| c.weapon_kind() == WeaponKind::LatticeRifle));
        let faces: std::collections::BTreeSet<AssetId> =
            crew.iter().map(|(_, _, b)| b.as_ref().expect("a body").source).collect();
        assert_eq!(faces.len(), 4);
        // The Varga's own: the boss one of its two, the rest its men.
        let varga = |n: &[u8]| -> Vec<AssetId> {
            n.iter()
                .map(|n| AssetId::from_relative_path(format!("characters/lampman_{n:02}/lampman_{n:02}.glb")))
                .collect()
        };
        let boss = crew.iter().find(|(_, c, _)| c.leader).and_then(|(_, _, b)| b.as_ref()).unwrap();
        assert!(varga(&[21, 15]).contains(&boss.source));
        let men = varga(&[2, 7, 12, 17, 19, 23]);
        assert!(crew.iter().filter(|(_, c, _)| !c.leader).all(|(_, _, b)| men.contains(&b.as_ref().unwrap().source)));
        // Every Lamp's crew wears four different men, its boss its own.
        for (colour, boss) in [("amber", 28u8), ("green", 32), ("blue", 30), ("white", 10)] {
            let mut world = World::new();
            let made = build_encounter(&mut world, &format!("lampmen {colour}"), Vec3::ZERO, Vec3::X, false, Some(&content))
                .expect("a crew");
            let worn: Vec<(bool, AssetId)> = made
                .iter()
                .filter_map(|e| {
                    let leader = world.get::<CombatantComponent>(*e)?.leader;
                    Some((leader, world.get::<SkinnedBody>(*e)?.source))
                })
                .collect();
            let distinct: std::collections::BTreeSet<AssetId> = worn.iter().map(|w| w.1).collect();
            assert_eq!((worn.len(), distinct.len()), (4, 4), "{colour}");
            if colour != "white" {
                assert!(worn.iter().any(|w| w.0 && varga(&[boss]).contains(&w.1)), "{colour}'s boss");
            }
        }
        assert!((encounter_extent("lampmen") - 8.0).abs() < 1e-6);
    }

    /// A mount the game refuses stays put when reached for; one it allows is
    /// mounted and told; a hit on a combatant not hostile to who hit is told,
    /// a hit on a foe is not.
    #[test]
    fn the_game_gates_mounting_and_hears_of_bystanders_hit() {
        let mut world = World::new();
        spawn_sandbox(&mut world, &json!({})).expect("preset builds");
        somnium_core::propagate_transforms(&mut world);
        let mut run = build(&world).expect("a fight");
        let player = run.player.unwrap();
        let id_of = |run: &Run, name: &str| run.world.combatants().find(|c| c.name == name).map(|c| c.id).unwrap();
        let elk = id_of(&run, "Frost-elk");
        let feet = run.world.combatant(player).unwrap().position;
        run.world.combatant_mut(elk).unwrap().position = feet + Vec3::X;
        let elk_entity = run.bodies[&elk];
        let reach = || somnium_combat::Command {
            mount: true,
            ..Default::default()
        };
        let refused = std::collections::HashSet::from([elk_entity]);
        let mut cmd = reach();
        assert_eq!(
            gate_mount(&run, &refused, player, &mut cmd),
            Some(HostEvent::MountRefused { mount: elk_entity })
        );
        assert!(!cmd.mount, "the reach is dropped");
        let mut cmd = reach();
        assert_eq!(gate_mount(&run, &Default::default(), player, &mut cmd), None);
        assert!(cmd.mount);
        run.world.command(player, cmd);
        run.world.step(1.0 / 60.0, &somnium_combat::OpenField, run.nav.as_ref());
        let events = run.world.drain_events();
        assert_eq!(run.world.combatant(player).unwrap().mount, Some(elk));
        assert!(news(&run, None, &events).contains(&HostEvent::Mounted {
            rider: run.bodies[&player],
            mount: elk_entity
        }));
        assert!(news(&run, Some(elk), &[]).is_empty(), "told once, on climbing up");
        let foe = run.world.combatants().find(|c| run.world.hostile(player, c.id)).map(|c| c.id).expect("a raider");
        let hit = |target| CombatEvent::Damaged {
            target,
            source: Some(player),
            amount: 5.0,
            kind: DamageType::Cut,
            point: Vec3::ZERO,
            critical: false,
            flags: HitFlags::default(),
        };
        assert!(!run.world.hostile(player, elk), "his own mount is no foe");
        assert_eq!(
            news(&run, Some(elk), &[hit(elk), hit(foe)]),
            vec![HostEvent::Hit {
                attacker: run.bodies[&player],
                target: elk_entity
            }]
        );
    }

    /// A Lance patrol rides: each of the three wakes in the saddle of a horse
    /// of its own.
    #[test]
    fn the_lance_wakes_in_the_saddle() {
        let (_world, demo) = host_with(&[("lance", Vec3::new(20.0, 0.0, 0.0))]);
        let w = &demo.run.as_ref().unwrap().world;
        let riders: Vec<(String, Option<String>)> = w
            .combatants()
            .filter(|c| c.name.starts_with("Lance") && c.mount_def.is_none())
            .map(|c| (c.name.clone(), c.mount.and_then(|m| w.combatant(m)).map(|m| m.name.clone())))
            .collect();
        assert_eq!(riders.len(), 3, "{riders:?}");
        for (name, horse) in riders {
            assert_eq!(horse, Some(format!("{name}'s horse")));
        }
    }

    /// The Wilds preset with only `part` (a crew, a post), the protagonist 8 m
    /// short of it: the world, the fight and him.
    fn clicking(part: &str) -> (World, Run, CombatantId) {
        let mut world = World::new();
        spawn_wilds(&mut world, &json!({ "only": part })).expect("preset builds");
        somnium_core::propagate_transforms(&mut world);
        let run = build(&world).expect("a fight");
        let hero = run.player.expect("the protagonist");
        (world, run, hero)
    }

    /// `seconds` of the fight with no keys down, the left button's command
    /// from `click` each step: what happened.
    fn click_for(
        world: &World,
        run: &mut Run,
        click: &mut crate::click_move::ClickMove,
        under: Option<CombatantId>,
        ground: Option<Vec3>,
        seconds: f32,
    ) -> Vec<CombatEvent> {
        let (terrain, hero, open) = (boxes(world, run), run.player.unwrap(), somnium_combat::OpenField);
        let mut events = Vec::new();
        for _ in 0..(seconds * 60.0) as usize {
            let mut cmd = somnium_combat::Command::default();
            click.steer(&run.world, run.nav.as_ref(), hero, under, ground, 1.0 / 60.0, &mut cmd);
            run.world.command(hero, cmd);
            run.world.step(1.0 / 60.0, &open, run.nav.as_ref());
            walk(&mut run.world, &mut run.walkers, 1.0 / 60.0, &terrain);
            events.extend(run.world.drain_events());
        }
        events
    }

    /// A left click on the ground walks him there and he stops (Diablo IV's
    /// mouse); held, he follows the cursor.
    #[test]
    fn a_click_on_the_ground_walks_him_there() {
        let (world, mut run, hero) = clicking("watch");
        let mut click = crate::click_move::ClickMove::default();
        let there = Vec3::new(5.0, 0.0, -2.0);
        click.button(true);
        click.button(false);
        click_for(&world, &mut run, &mut click, None, Some(there), 5.0);
        let c = run.world.combatant(hero).unwrap();
        let off = Vec2::new(c.position.x - there.x, c.position.z - there.z).length();
        assert!(off < 0.5, "arrived: {} for {there}", c.position);
        assert!(Vec2::new(c.velocity.x, c.velocity.z).length() < 0.2, "and stopped");
        assert_eq!(click.goal(), None);
        // Held, the cursor leads him on.
        click.button(true);
        let on = Vec3::new(-4.0, 0.0, 2.0);
        click_for(&world, &mut run, &mut click, None, Some(on), 4.0);
        let c = run.world.combatant(hero).unwrap();
        assert!(Vec2::new(c.position.x - on.x, c.position.z - on.z).length() < 0.5, "{}", c.position);
    }

    /// A left click on a hostile walks him into reach and strikes it once;
    /// held, he keeps striking.
    #[test]
    fn a_click_on_a_hostile_walks_into_reach_and_strikes() {
        let (world, mut run, hero) = clicking("lampmen");
        let boss = run.world.combatants().find(|c| c.name == "Lamp-man boss").map(|c| c.id).unwrap();
        // The crew fights back; he outlasts it here, so what is tested is the button.
        let health = &mut run.world.combatant_mut(hero).unwrap().vitals.health;
        (health.max, health.current) = (5000.0, 5000.0);
        // The cursor's ray from the camera's side onto the boss's chest finds him.
        let chest = run.world.combatant(boss).unwrap().chest();
        let eye = chest + Vec3::new(0.0, 12.0, -9.0);
        assert_eq!(crate::click_move::pick(&run.world, hero, eye, chest - eye), Some(boss));
        let swings = |events: &[CombatEvent]| {
            events
                .iter()
                .filter(|e| matches!(e, CombatEvent::AttackStarted { attacker, .. } if *attacker == hero))
                .count()
        };
        let mut click = crate::click_move::ClickMove::default();
        click.button(true);
        click.button(false);
        let events = click_for(&world, &mut run, &mut click, Some(boss), None, 4.0);
        let (me, him) = (run.world.combatant(hero).unwrap(), run.world.combatant(boss).unwrap());
        let gap = Vec2::new(me.position.x - him.position.x, me.position.z - him.position.z).length();
        assert!(gap < me.weapon.reach + me.body.radius + him.body.radius, "in reach: {gap}");
        assert_eq!(swings(&events), 1, "one click, one blow");
        click.button(true);
        let events = click_for(&world, &mut run, &mut click, Some(boss), None, 3.0);
        assert!(swings(&events) >= 2, "held, he keeps at it: {}", swings(&events));
    }

    /// The protagonist alone in a scene, with these encounters near him: Play's
    /// run woken round him, in a host.
    fn host_with(encounters: &[(&str, Vec3)]) -> (World, CombatDemo) {
        let mut world = World::new();
        let root = world.spawn((Name::new("Scene"), Transform::default(), WorldTransform::identity()));
        fighter(&mut world, root, "Protagonist", Vec3::ZERO, Vec3::X, CombatantComponent::protagonist());
        for (kind, at) in encounters {
            world.spawn((
                Name::new(&format!("Encounter {kind}")),
                Transform::from_translation(*at),
                WorldTransform::identity(),
                CombatEncounterComponent {
                    kind: (*kind).into(),
                    ..CombatEncounterComponent::default()
                },
            ));
        }
        somnium_core::propagate_transforms(&mut world);
        let mut run = build(&world).expect("a fight");
        run.encounters = Some(encounters_asleep(&world));
        tend_encounters(&mut world, None, &mut run, f32::INFINITY);
        let demo = CombatDemo {
            run: Some(run),
            ..CombatDemo::default()
        };
        (world, demo)
    }

    /// Lamp crews by colour and the watch are neutral until the game turns
    /// their faction; a plain `lampmen` crew is hostile as it always was.
    #[test]
    fn factions_are_neutral_until_the_game_turns_them() {
        let (_world, mut demo) = host_with(&[
            ("lampmen red", Vec3::new(30.0, 0.0, 0.0)),
            ("watch", Vec3::new(-30.0, 0.0, 0.0)),
            ("lampmen", Vec3::new(0.0, 0.0, 30.0)),
        ]);
        let first = |demo: &CombatDemo, name: &str, team: Option<TeamId>| -> CombatantId {
            let run = demo.run.as_ref().unwrap();
            run.world
                .combatants()
                .find(|c| c.name == name && team.is_none_or(|t| c.team == t))
                .map(|c| c.id)
                .unwrap_or_else(|| panic!("no {name}"))
        };
        let hero = demo.run.as_ref().unwrap().player.unwrap();
        let red = first(&demo, "Lamp-man (rifle)", faction_team("lamp_red"));
        let plain = first(&demo, "Lamp-man (rifle)", Some(TeamId(1)));
        let watch = first(&demo, "Watchman (spear)", None);
        let hostile = |demo: &CombatDemo, b| demo.run.as_ref().unwrap().world.hostile(hero, b);
        assert!(!hostile(&demo, red) && !hostile(&demo, watch), "neutral to begin with");
        assert!(hostile(&demo, plain), "a plain crew as before");
        demo.set_hostile_factions(&["lamp_red", "the moon"]);
        assert!(hostile(&demo, red) && !hostile(&demo, watch));
        demo.set_hostile_factions(&["WATCH"]);
        assert!(!hostile(&demo, red) && hostile(&demo, watch));
        demo.set_hostile_factions::<&str>(&[]);
        assert!(!hostile(&demo, watch));
        // Every faction has a band, each member its own name.
        for faction in FACTIONS {
            let kind = faction_kind(faction).unwrap();
            let (team, _, members) = band_of(&kind).unwrap_or_else(|| panic!("no band {kind}"));
            assert_eq!(Some(TeamId(team as u16)), faction_team(faction));
            let names: BTreeSet<&str> = members.iter().map(|m| m.0).collect();
            assert_eq!(names.len(), members.len(), "{kind}");
        }
    }

    /// A hunting faction sends a band from out of sight; asleep it walks to
    /// him, wakes into the fight hostile and makes for him. When the hunt is
    /// called off it is taken back once asleep, and no other is sent.
    #[test]
    fn a_hunting_faction_sends_a_band_that_finds_him() {
        let (mut world, mut demo) = host_with(&[]);
        demo.set_hunting_factions(&["knights"]);
        let hunters = |demo: &CombatDemo| -> Vec<(GroupId, bool, Vec3, bool)> {
            let set = demo.run.as_ref().unwrap().encounters.as_ref().unwrap();
            (0..set.len())
                .map(|i| GroupId(i as u32))
                .filter_map(|id| {
                    let r = set.data(id)?;
                    r.hunter.as_ref()?;
                    let gone = r.left.as_ref().is_some_and(Vec::is_empty);
                    Some((id, set.is_awake(id), set.position(id)?, gone))
                })
                .collect()
        };
        tend_encounters(&mut world, None, demo.run.as_mut().unwrap(), TEND_EVERY);
        let sent = hunters(&demo);
        assert_eq!(sent.len(), 1);
        let (_, awake, at, _) = sent[0];
        assert!(!awake && (at.length() - HUNT_FROM).abs() < 1.0, "{at}");
        let mut woke = false;
        for _ in 0..200 {
            tend_encounters(&mut world, None, demo.run.as_mut().unwrap(), TEND_EVERY);
            if hunters(&demo)[0].1 {
                woke = true;
                break;
            }
        }
        assert!(woke, "it came within waking distance");
        let run = demo.run.as_ref().unwrap();
        let hero = run.player.unwrap();
        let knights: Vec<&somnium_combat::world::Combatant> =
            run.world.combatants().filter(|c| c.name.starts_with("Knight")).collect();
        assert_eq!(knights.len(), 4);
        assert!(knights.iter().all(|k| run.world.hostile(hero, k.id)));
        let home = knights[0].brain.as_ref().and_then(|b| b.mind.home).expect("led");
        assert!(home.length() < 3.0, "making for him: {home}");
        // Called off: the band sleeps once he is far, is taken back, and none follows.
        demo.set_hunting_factions::<&str>(&[]);
        let run = demo.run.as_mut().unwrap();
        let far = Vec3::new(0.0, 0.0, 900.0);
        run.world.combatant_mut(hero).unwrap().position = far;
        run.walkers.get_mut(&hero).unwrap().feet = far;
        for _ in 0..4 {
            tend_encounters(&mut world, None, demo.run.as_mut().unwrap(), TEND_EVERY);
        }
        let after = hunters(&demo);
        assert_eq!(after.len(), 1, "none sent after");
        assert!(!after[0].1 && after[0].3, "asleep and taken back: {after:?}");
    }

    /// A harrowcat hunts alone with the beasts: one heavy body with a wolf's
    /// brain and jaws, hostile to the party, in WS3's cat where it exists.
    #[test]
    fn a_harrowcat_hunts_alone() {
        use crate::skinned_crowd::SkinnedBody;
        let content = std::env::temp_dir().join("somnium_combat_harrowcat");
        let file = content.join("creatures/harrowcat/harrowcat.glb");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"").unwrap();
        let mut world = World::new();
        let made = build_encounter(&mut world, "Harrowcat", Vec3::new(40.0, 0.0, 0.0), Vec3::X, false, Some(&content))
            .expect("a cat");
        let cats: Vec<Entity> = made
            .iter()
            .copied()
            .filter(|e| world.get::<CombatantComponent>(*e).is_some())
            .collect();
        assert_eq!(cats.len(), 1);
        let c = world.get::<CombatantComponent>(cats[0]).unwrap();
        assert_eq!((c.team, c.weapon_kind()), (2, WeaponKind::Fangs));
        assert!(c.max_health > 300.0);
        assert_eq!(
            world.get::<SkinnedBody>(cats[0]).unwrap().source,
            AssetId::from_relative_path("creatures/harrowcat/harrowcat.glb")
        );
        let (_world, demo) = host_with(&[("harrowcat", Vec3::new(30.0, 0.0, 0.0))]);
        let run = demo.run.as_ref().unwrap();
        let cat = run.world.combatants().find(|c| c.name == "Harrowcat").expect("woken");
        assert!(run.world.hostile(run.player.unwrap(), cat.id));
    }

    /// A load puts the protagonist back where the save had him, at its health,
    /// mid-swing no longer; riding, on the nearest mount brought beside him;
    /// on foot, off it.
    #[test]
    fn a_load_puts_the_protagonist_back() {
        let mut world = World::new();
        spawn_sandbox(&mut world, &json!({})).expect("preset builds");
        somnium_core::propagate_transforms(&mut world);
        let mut demo = CombatDemo::default();
        assert!(!demo.restore_player(Vec3::ZERO, 0.0, 1.0, false), "no fight, nothing to restore");
        demo.run = Some(build(&world).expect("a fight"));
        let player = demo.run.as_ref().unwrap().player.unwrap();
        let feet = Vec3::new(30.0, 0.0, -12.0);
        assert!(demo.restore_player(feet, std::f32::consts::FRAC_PI_2, 55.0, true));
        let run = demo.run.as_ref().unwrap();
        let c = run.world.combatant(player).unwrap();
        let elk = c.mount.expect("riding");
        assert!(matches!(c.action, Action::Idle));
        assert!((c.vitals.health.current - 55.0).abs() < 1e-4);
        assert!((c.facing - Vec3::X).length() < 1e-4, "a quarter turn faces +X");
        let steed = run.world.combatant(elk).unwrap().position;
        assert!(Vec3::new(steed.x - feet.x, 0.0, steed.z - feet.z).length() < 1.5, "the mount was brought: {steed}");
        assert!(demo.restore_player(feet, 0.0, 999.0, false));
        let c = demo.run.as_ref().unwrap().world.combatant(player).unwrap();
        assert!(c.mount.is_none(), "on foot");
        assert!(c.vitals.health.current <= c.vitals.health.max, "health within its pool");
    }

    /// The switch makes another fighter the played one: he has no brain now,
    /// and the one left has one (his style's), his command and lock-on dropped.
    #[test]
    fn the_switch_hands_the_fight_to_the_other_protagonist() {
        let mut world = World::new();
        spawn_sandbox(&mut world, &json!({})).expect("preset builds");
        somnium_core::propagate_transforms(&mut world);
        let mut demo = CombatDemo {
            run: Some(build(&world).expect("a fight")),
            ..Default::default()
        };
        let (caelan, other, other_entity) = {
            let run = demo.run.as_ref().unwrap();
            let caelan = run.player.unwrap();
            let (other, e) = run
                .bodies
                .iter()
                .find(|(id, _)| {
                    **id != caelan && run.world.combatant(**id).is_some_and(|c| c.brain.is_some() && c.mount_def.is_none())
                })
                .map(|(id, e)| (*id, *e))
                .expect("another fighter");
            (caelan, other, e)
        };
        demo.run.as_mut().unwrap().world.combatant_mut(caelan).unwrap().lock = Some(other);
        let nobody = world.spawn((Name::new("nobody"),));
        assert!(!demo.set_player(&world, nobody), "not a fighter");
        assert!(demo.set_player(&world, other_entity));
        let run = demo.run.as_ref().unwrap();
        assert_eq!(run.player, Some(other));
        assert!(run.world.combatant(other).unwrap().brain.is_none(), "his own brain gone: he is played");
        let left = run.world.combatant(caelan).unwrap();
        assert!(left.brain.is_some() && left.lock.is_none(), "the one left fights as anyone would");
        assert!(demo.set_player(&world, other_entity), "already him");
    }

    use super::JOLT;

    /// A post inside a roofed house (the Varga warehouse's store) gets a patch
    /// of its floor, walled in, not of its roof: a path across the room keeps
    /// to the floor and goes out by the door.
    #[test]
    fn an_indoor_post_gets_a_patch_of_its_floor() {
        let (mut world, mut demo) = host_with(&[]);
        let hero = demo.run.as_ref().unwrap().player.unwrap();
        let inside = Vec3::new(-4.0, 0.0, -2.0);
        {
            let run = demo.run.as_mut().unwrap();
            run.world.combatant_mut(hero).unwrap().position = inside;
            run.walkers.get_mut(&hero).unwrap().feet = inside;
        }
        world.spawn((
            Name::new("Store post"),
            Transform::from_translation(Vec3::new(2.0, 0.0, 1.0)),
            WorldTransform::identity(),
            CombatEncounterComponent {
                kind: "lampmen".into(),
                ..CombatEncounterComponent::default()
            },
        ));
        somnium_core::propagate_transforms(&mut world);
        let _jolt = JOLT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut physics = somnium_physics::world::PhysicsWorld::new(Default::default());
        let slab = |size: Vec3, at: Vec3| Mat4::from_scale_rotation_translation(size, Quat::IDENTITY, at);
        // A 16 x 12 house: walls 4.2 m high, a door 2 m wide in the south
        // wall, an upper floor at 4.2 and a roof at 8.
        solid_bodies(
            &mut physics,
            vec![
                slab(Vec3::new(300.0, 1.0, 300.0), Vec3::new(0.0, -0.5, 0.0)),
                slab(Vec3::new(16.0, 4.2, 0.4), Vec3::new(0.0, 2.1, 6.0)),
                slab(Vec3::new(0.4, 4.2, 12.0), Vec3::new(-8.0, 2.1, 0.0)),
                slab(Vec3::new(0.4, 4.2, 12.0), Vec3::new(8.0, 2.1, 0.0)),
                slab(Vec3::new(7.0, 4.2, 0.4), Vec3::new(-4.5, 2.1, -6.0)),
                slab(Vec3::new(7.0, 4.2, 0.4), Vec3::new(4.5, 2.1, -6.0)),
                slab(Vec3::new(16.4, 0.3, 12.4), Vec3::new(0.0, 4.35, 0.0)),
                slab(Vec3::new(16.4, 0.3, 12.4), Vec3::new(0.0, 8.15, 0.0)),
            ],
        );
        let run = demo.run.as_mut().unwrap();
        run.encounters = Some(encounters_asleep(&world));
        tend_encounters(&mut world, Some(&mut physics), run, f32::INFINITY);
        let nav = run.nav.as_ref().expect("a patch");
        let across = nav
            .path(Vec3::new(-6.0, 0.0, -3.0), Vec3::new(6.0, 0.0, 4.0), 1.0)
            .expect("across the room");
        assert!(across.points.iter().all(|p| p.y.abs() < 0.3), "on the floor: {:?}", across.points);
        let out = nav
            .path(Vec3::new(5.0, 0.0, 3.0), Vec3::new(0.0, 0.0, -14.0), 1.0)
            .expect("out of the house");
        let through_wall = |p: &Vec3| p.z < -5.0 && p.z > -7.0 && p.x.abs() > 1.2;
        assert!(
            out.points.windows(2).all(|w| !through_wall(&w[0].lerp(w[1], 0.5))) && out.points.iter().all(|p| p.y.abs() < 0.3),
            "by the door: {:?}",
            out.points
        );
    }

    /// A slow frame's catch-up steps beyond two are let go, counted; the
    /// next frame steps afresh.
    #[test]
    fn a_slow_frame_takes_two_fight_steps() {
        let mut demo = CombatDemo::default();
        let taken: Vec<bool> = (0..6).map(|_| demo.take_step(7)).collect();
        assert_eq!(taken, [true, true, false, false, false, false]);
        assert_eq!(demo.skipped, 4);
        assert!(demo.take_step(8) && demo.take_step(8) && !demo.take_step(8));
    }

    /// Calm beasts (Act I) let him be, those that wake later too; turned
    /// back, they hunt him again.
    #[test]
    fn calm_beasts_let_him_be() {
        let (mut world, mut demo) = host_with(&[("wolves", Vec3::new(30.0, 0.0, 0.0))]);
        let hero = demo.run.as_ref().unwrap().player.unwrap();
        let wolf = |demo: &CombatDemo| {
            let run = demo.run.as_ref().unwrap();
            run.world.combatants().filter(|c| c.name.starts_with("Rime wolf")).map(|c| c.id).collect::<Vec<_>>()
        };
        let hostile = |demo: &CombatDemo, id| demo.run.as_ref().unwrap().world.hostile(hero, id);
        let first = wolf(&demo)[0];
        assert!(hostile(&demo, first), "wolves hunt by default");
        demo.set_beasts_hostile(false);
        assert!(!hostile(&demo, first));
        // A cat woken after: calm as well.
        world.spawn((
            Name::new("Cat"),
            Transform::from_translation(Vec3::new(-30.0, 0.0, 0.0)),
            WorldTransform::identity(),
            CombatEncounterComponent {
                kind: "harrowcat".into(),
                ..CombatEncounterComponent::default()
            },
        ));
        somnium_core::propagate_transforms(&mut world);
        tend_encounters(&mut world, None, demo.run.as_mut().unwrap(), f32::INFINITY);
        let cat = demo.run.as_ref().unwrap().world.combatants().find(|c| c.name == "Harrowcat").map(|c| c.id).unwrap();
        assert!(!hostile(&demo, cat) && !hostile(&demo, first));
        demo.set_beasts_hostile(true);
        assert!(hostile(&demo, cat) && hostile(&demo, first));
    }

    /// The bestiary's new kinds wake as the beasts: harrow-dogs a pack of five
    /// in WS3's sheepdog, a crocodile alone, ice-crawlers three and blind.
    #[test]
    fn the_bestiary_hunts_with_the_beasts() {
        use crate::skinned_crowd::SkinnedBody;
        let content = std::env::temp_dir().join("somnium_combat_bestiary");
        for stem in ["sheepdog/sheepdog", "delta_crocodile/delta_crocodile", "ice_crawler/ice_crawler"] {
            let file = content.join(format!("creatures/{stem}.glb"));
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, b"").unwrap();
        }
        let mut world = World::new();
        let dogs = build_encounter(&mut world, "harrowdogs", Vec3::ZERO, Vec3::X, false, Some(&content)).expect("dogs");
        let bodies = dogs.iter().filter(|e| world.get::<CombatantComponent>(**e).is_some()).count();
        let worn = dogs.iter().filter(|e| world.get::<SkinnedBody>(**e).is_some()).count();
        assert_eq!((bodies, worn), (5, 5));
        for (kind, stem, count) in [("crocodile", "delta_crocodile", 1), ("icecrawlers", "ice_crawler", 3)] {
            let made = build_encounter(&mut world, kind, Vec3::ZERO, Vec3::X, false, Some(&content)).expect(kind);
            let sources: Vec<AssetId> = made.iter().filter_map(|e| world.get::<SkinnedBody>(*e)).map(|b| b.source).collect();
            let want = AssetId::from_relative_path(format!("creatures/{stem}/{stem}.glb"));
            assert_eq!(sources, vec![want; count], "{kind}");
        }
        let (_world, demo) = host_with(&[
            ("crocodile", Vec3::new(30.0, 0.0, 0.0)),
            ("icecrawlers", Vec3::new(-30.0, 0.0, 0.0)),
        ]);
        let run = demo.run.as_ref().unwrap();
        let hero = run.player.unwrap();
        let croc: Vec<_> = run.world.combatants().filter(|c| c.name == "Delta crocodile").collect();
        let crawlers: Vec<_> = run.world.combatants().filter(|c| c.name.starts_with("Ice-crawler")).collect();
        assert_eq!((croc.len(), crawlers.len()), (1, 3));
        assert!(run.world.hostile(hero, croc[0].id) && crawlers.iter().all(|c| run.world.hostile(hero, c.id)));
        assert!(crawlers.iter().all(|c| c.brain.as_ref().is_some_and(|b| b.mind.profile.sight <= 3.0)), "blind");
        assert!(croc[0].brain.as_ref().is_some_and(|b| b.mind.profile.sight > 3.0));
    }

    /// The Ledger Engine: a keeper and three brainless boards; the Hold's ring
    /// shows, then holds him if he stays and not if he steps out; Count heals
    /// the keeper let be; Sense calls guards at two thirds; broken boards stop
    /// each; all broken, the keeper reels; the HUD sees it all.
    #[test]
    fn the_ledger_engine_fights_in_phases_with_a_readable_tell() {
        let (mut world, mut demo) = host_with(&[("ledger_engine", Vec3::new(0.0, 0.0, 12.0))]);
        let dt = 1.0 / 60.0;
        let id = |demo: &CombatDemo, name: &str| {
            demo.run.as_ref().unwrap().world.combatants().find(|c| c.name == name).map(|c| c.id).expect(name)
        };
        let keeper = id(&demo, ENGINE_KEEPER);
        let boards = ENGINE_BOARDS.map(|b| id(&demo, b));
        let hero = demo.run.as_ref().unwrap().player.unwrap();
        {
            let run = demo.run.as_ref().unwrap();
            assert!(boards.iter().all(|b| run.world.combatant(*b).unwrap().brain.is_none()), "boards think not");
            assert!(run.world.hostile(hero, keeper));
        }
        // The first Hold: its ring shows for the tell, then holds him where he stands.
        let tick = |world: &mut World, demo: &mut CombatDemo, seconds: f32| {
            for _ in 0..(seconds / dt).round() as usize {
                engine_tick(world, demo.run.as_mut().unwrap(), dt);
            }
        };
        tick(&mut world, &mut demo, HOLD_FIRST + 0.05);
        let view = demo.boss_view().expect("a boss");
        assert_eq!((view.phase, view.parts.len()), (1, 3));
        let (centre, radius, left) = view.hold.expect("the tell");
        assert!(centre.distance(Vec3::ZERO) < 0.1 && radius == HOLD_RADIUS && left > 1.0);
        assert!(world.iter_with::<Name>().any(|(_, n)| n.as_str() == "Hold (the Ledger Engine)"), "its ring");
        tick(&mut world, &mut demo, HOLD_TELL);
        let frozen = |demo: &CombatDemo| {
            demo.run.as_ref().unwrap().world.combatant(hero).unwrap().status.active(StatusKind::Freeze)
        };
        assert!(frozen(&demo), "held");
        assert!(demo.boss_view().unwrap().hold.is_none());
        // The next: he steps out of the ring in time.
        demo.run.as_mut().unwrap().world.combatant_mut(hero).unwrap().status = Default::default();
        tick(&mut world, &mut demo, HOLD_EVERY[0] + 0.05);
        assert!(demo.boss_view().unwrap().hold.is_some());
        demo.run.as_mut().unwrap().world.combatant_mut(hero).unwrap().position = Vec3::new(5.0, 0.0, 0.0);
        tick(&mut world, &mut demo, HOLD_TELL);
        assert!(!frozen(&demo), "stepped out");
        // Count: the keeper's wounds counted back while he is let be.
        let health = |demo: &CombatDemo| demo.run.as_ref().unwrap().world.combatant(keeper).unwrap().vitals.health.current;
        demo.run.as_mut().unwrap().world.combatant_mut(keeper).unwrap().vitals.health.current = 500.0;
        tick(&mut world, &mut demo, COUNT_QUIET + 1.0);
        assert!(health(&demo) > 500.0, "counted back: {}", health(&demo));
        // Sense: at two thirds the Engine calls its guards, who wake by the door.
        demo.run.as_mut().unwrap().world.combatant_mut(keeper).unwrap().vitals.health.current = 400.0;
        tick(&mut world, &mut demo, dt);
        tend_encounters(&mut world, None, demo.run.as_mut().unwrap(), f32::INFINITY);
        let guards = demo.run.as_ref().unwrap().world.combatants().filter(|c| c.name.starts_with("Castellane guard")).count();
        assert_eq!(guards, 2);
        assert_eq!(demo.boss_view().unwrap().phase, 2);
        // Broken boards: no more Count, no more Hold; all broken, the keeper reels.
        for b in boards {
            demo.run.as_mut().unwrap().world.combatant_mut(b).unwrap().action = Action::Dead;
        }
        demo.run.as_mut().unwrap().world.combatant_mut(keeper).unwrap().vitals.health.current = 300.0;
        tick(&mut world, &mut demo, HOLD_EVERY[1] + COUNT_QUIET + 1.0);
        let view = demo.boss_view().unwrap();
        assert!(view.hold.is_none() && view.parts.iter().all(|p| !p.1));
        assert!((health(&demo) - 300.0).abs() < 1e-3, "nothing counts him back");
        let run = demo.run.as_ref().unwrap();
        assert!(run.world.combatant(keeper).unwrap().status.active(StatusKind::Stagger), "the Engine stopped");
    }

    /// The Engine's boards wear WS3's boards (their boxes not drawn); a board
    /// broken shows WS3's broken board and stands where it broke.
    #[test]
    fn the_engines_boards_wear_ws3_and_break() {
        let content = std::env::temp_dir().join("somnium_combat_boards");
        for i in 0..3 {
            for broken in [false, true] {
                let file = content.join(board_file(i, broken));
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(&file, b"").unwrap();
            }
        }
        let mut world = World::new();
        let root = world.spawn((Name::new("Scene"), Transform::default(), WorldTransform::identity()));
        fighter(&mut world, root, "Protagonist", Vec3::ZERO, Vec3::X, CombatantComponent::protagonist());
        world.spawn((
            Name::new("Vault"),
            Transform::from_translation(Vec3::new(0.0, 0.0, 12.0)),
            WorldTransform::identity(),
            CombatEncounterComponent {
                kind: "ledger_engine".into(),
                ..CombatEncounterComponent::default()
            },
        ));
        somnium_core::propagate_transforms(&mut world);
        let mut run = build(&world).expect("a fight");
        run.content = Some(content.clone());
        run.encounters = Some(encounters_asleep(&world));
        tend_encounters(&mut world, None, &mut run, f32::INFINITY);
        let mesh_of = |world: &World, name: &str| -> String {
            let board = world.iter_with::<Name>().find(|(_, n)| n.as_str() == name).map(|(e, _)| e).unwrap();
            assert!(world.get::<MeshKind>(board).is_none(), "{name}'s box not drawn");
            world
                .iter_with::<ImportedMesh>()
                .find(|(e, _)| world.get::<Parent>(*e).is_some_and(|p| p.entity == board))
                .map(|(_, m)| m.source.clone())
                .expect("its mesh")
        };
        assert_eq!(mesh_of(&world, ENGINE_BOARDS[1]), format!("assets/{}", board_file(1, false)));
        let hold = run.world.combatants().find(|c| c.name == ENGINE_BOARDS[1]).map(|c| c.id).unwrap();
        run.world.combatant_mut(hold).unwrap().action = Action::Dead;
        engine_tick(&mut world, &mut run, 1.0 / 60.0);
        assert_eq!(mesh_of(&world, ENGINE_BOARDS[1]), format!("assets/{}", board_file(1, true)));
        assert_eq!(mesh_of(&world, ENGINE_BOARDS[0]), format!("assets/{}", board_file(0, false)), "the others whole");
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
        let made = expand_encounters(&mut world, None);
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
        assert!(expand_encounters(&mut off, None).is_empty());
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
        expand_encounters(&mut world, None);
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

    /// The pushes the host's walk takes for everyone at once
    /// (`CombatWorld::separations`) are `separation`'s, to the bit, in a
    /// packed and partly fallen crowd with a horse in it: a second guard
    /// beside the combat crate's own.
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
        for (id, push) in run.world.separations(&ids) {
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

    /// Where the scene baked no navmesh, a woken camp gets a patch of ground
    /// to path on (two-metre cells from casts down through physics): round a
    /// house, not through it. Asleep again, its patch goes.
    #[test]
    fn a_woken_camp_paths_round_a_house_on_its_patch() {
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
            Vec3::new(-30.0, 0.0, 0.0),
            Vec3::X,
            CombatantComponent::protagonist(),
        );
        world.spawn((
            Name::new("Camp"),
            Transform::default(),
            WorldTransform::identity(),
            CombatEncounterComponent {
                kind: "bandits".into(),
                ..CombatEncounterComponent::default()
            },
        ));
        somnium_core::propagate_transforms(&mut world);
        let mut run = build(&world).expect("a fight");
        assert!(run.nav.is_none(), "no solids, nothing baked");
        run.encounters = Some(encounters_asleep(&world));
        let _jolt = JOLT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut physics = somnium_physics::world::PhysicsWorld::new(Default::default());
        let house = Vec3::new(20.0, 3.0, 0.0);
        solid_bodies(
            &mut physics,
            vec![
                Mat4::from_scale_rotation_translation(
                    Vec3::new(300.0, 1.0, 300.0),
                    Quat::IDENTITY,
                    Vec3::new(0.0, -0.5, 0.0),
                ),
                Mat4::from_scale_rotation_translation(
                    Vec3::new(8.0, 6.0, 12.0),
                    Quat::IDENTITY,
                    house,
                ),
            ],
        );
        let started = std::time::Instant::now();
        tend_encounters(&mut world, Some(&mut physics), &mut run, f32::INFINITY);
        let built = started.elapsed();
        let nav = run.nav.as_ref().expect("a patch");
        assert_eq!(nav.cells().count(), 1);
        let path = nav
            .path(Vec3::new(5.0, 0.0, 0.0), Vec3::new(34.0, 0.0, 0.0), 1.0)
            .expect("round the house");
        let inside = |p: &Vec3| (p.x - house.x).abs() < 4.0 && p.z.abs() < 6.0;
        assert!(
            path.points.iter().all(|p| !inside(p) && p.y.abs() < 0.1),
            "{:?}",
            path.points
        );
        println!("patch woken and built in {built:?}");
        // The camp runs 30 m east: past its patch, until it is patched anew.
        let far = Vec3::new(60.0, 0.0, 0.0);
        assert!(
            run.nav
                .as_ref()
                .unwrap()
                .path(Vec3::new(30.0, 0.0, 0.0), far, 1.0)
                .is_none()
        );
        let hero = run.player.expect("the protagonist");
        let camp: Vec<CombatantId> = run
            .world
            .combatants()
            .filter(|c| c.id != hero)
            .map(|c| c.id)
            .collect();
        for id in camp {
            run.world.combatant_mut(id).unwrap().position += Vec3::X * 30.0;
        }
        tend_encounters(&mut world, Some(&mut physics), &mut run, f32::INFINITY);
        let nav = run.nav.as_ref().unwrap();
        assert_eq!(nav.cells().count(), 1, "one patch, moved");
        assert!(
            nav.path(Vec3::new(30.0, 0.0, 0.0), far, 1.0).is_some(),
            "re-patched"
        );
        // Away: asleep, and its patch gone.
        run.world.combatant_mut(hero).unwrap().position = Vec3::new(-400.0, 0.0, 0.0);
        tend_encounters(&mut world, Some(&mut physics), &mut run, f32::INFINITY);
        assert_eq!(run.nav.as_ref().map(|n| n.cells().count()), Some(0));
    }

    /// An encounter that arrives after Play began (a streamed cell, a script)
    /// is taken up at the next look and wakes like any other; the same
    /// encounter streamed in again is not taken up twice.
    #[test]
    fn an_encounter_streamed_in_after_play_wakes_once() {
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
        somnium_core::propagate_transforms(&mut world);
        let mut run = build(&world).expect("a fight");
        run.encounters = Some(encounters_asleep(&world));
        tend_encounters(&mut world, None, &mut run, f32::INFINITY);
        assert_eq!(run.world.combatants().count(), 1, "nothing to wake yet");
        let den = |world: &mut World| {
            let e = world.spawn((
                Name::new("Streamed den"),
                Transform::from_translation(Vec3::new(40.0, 0.0, 0.0)),
                WorldTransform::identity(),
                CombatEncounterComponent {
                    kind: "wolves".into(),
                    ..CombatEncounterComponent::default()
                },
            ));
            somnium_core::propagate_transforms(world);
            e
        };
        let first = den(&mut world);
        tend_encounters(&mut world, None, &mut run, f32::INFINITY);
        let wolves = |run: &Run| {
            run.world
                .combatants()
                .filter(|c| c.name.starts_with("Rime wolf"))
                .count()
        };
        assert_eq!(wolves(&run), 5, "taken up and woken");
        // The cell streams out and back in: a new entity, the same encounter.
        world.despawn(first);
        den(&mut world);
        tend_encounters(&mut world, None, &mut run, f32::INFINITY);
        assert_eq!(wolves(&run), 5, "not twice");
        assert_eq!(run.encounters.as_ref().map(|s| s.len()), Some(1));
    }

    /// Flat ground a kilometre across, for bodies to walk on headless.
    fn plain() -> somnium_character::boxes::Boxes {
        somnium_character::boxes::Boxes {
            solids: vec![somnium_character::boxes::Solid {
                center: Vec3::new(0.0, -0.5, 0.0),
                half: Vec3::new(500.0, 0.5, 500.0),
                rotation: Quat::IDENTITY,
            }],
            ..Default::default()
        }
    }

    /// A scene with the protagonist at `hero` and nothing else.
    fn hero_scene(hero: Vec3) -> (World, Entity) {
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
            hero,
            Vec3::X,
            CombatantComponent::protagonist(),
        );
        (world, root)
    }

    /// A game's HUD reads the running fight (none without one) and takes its
    /// newest events, at most [`HUD_EVENTS`] waiting.
    #[test]
    fn hud_view_and_events_for_a_game() {
        let (world, _) = hero_scene(Vec3::ZERO);
        let mut demo = CombatDemo::default();
        assert!(demo.hud_view(&world).is_none() && demo.show_hud);
        demo.run = build(&world);
        let view = demo.hud_view(&world).expect("a fight");
        let hero = view.player.expect("the protagonist");
        assert!(view.world.combatant(hero).is_some() && view.marks.is_empty());
        let broken = |n: u32| CombatEvent::GuardBroken { target: CombatantId(n) };
        let events: Vec<CombatEvent> = (0..100).map(broken).collect();
        keep_hud_events(&mut demo.hud_events, &events, HUD_EVENTS);
        let taken = demo.take_hud_events();
        assert_eq!(taken.len(), HUD_EVENTS);
        assert!(matches!(taken[0], CombatEvent::GuardBroken { target: CombatantId(36) }), "the oldest go");
        assert!(demo.take_hud_events().is_empty(), "taken once");
    }

    /// A band patrolling a road: along a spline, from `at`, `length` metres
    /// along +X.
    fn patrol(world: &mut World, at: Vec3, length: f32) -> Entity {
        world.spawn((
            Name::new("Patrol"),
            Transform::from_translation(at),
            WorldTransform::identity(),
            somnium_core::SplineComponent {
                points: vec![Vec3::ZERO, Vec3::X * length],
                closed: false,
                tangents: Vec::new(),
            },
            CombatPatrolComponent::default(),
        ))
    }

    /// Play's fixed steps without the engine: the fight, the walkers, and a
    /// look at the encounters every half second.
    fn play(world: &mut World, run: &mut Run, seconds: f32) {
        let ground = plain();
        let open = somnium_combat::OpenField;
        for frame in 0..(seconds * 60.0) as usize {
            run.world.step(1.0 / 60.0, &open, run.nav.as_ref());
            walk(&mut run.world, &mut run.walkers, 1.0 / 60.0, &ground);
            if frame % 30 == 29 {
                tend_encounters(world, None, run, 0.5);
            }
        }
    }

    fn centre_of(run: &Run, prefix: &str) -> Vec3 {
        let at: Vec<Vec3> = run
            .world
            .combatants()
            .filter(|c| c.name.starts_with(prefix) && c.alive())
            .map(|c| c.position)
            .collect();
        at.iter().sum::<Vec3>() / at.len().max(1) as f32
    }

    /// A patrol keeps walking its road while it sleeps, wakes where it has
    /// got to when the protagonist comes near, and walks on from there.
    #[test]
    fn a_patrol_walks_its_road_asleep_and_awake() {
        let (mut world, _) = hero_scene(Vec3::new(-200.0, 0.0, 0.0));
        patrol(&mut world, Vec3::ZERO, 120.0);
        somnium_core::propagate_transforms(&mut world);
        let mut run = build(&world).expect("a fight");
        run.encounters = Some(encounters_asleep(&world));
        tend_encounters(&mut world, None, &mut run, f32::INFINITY);
        assert_eq!(run.world.combatants().count(), 1, "asleep, 200 m off");
        // Ten seconds asleep at 1.3 m/s: thirteen metres down the road.
        for _ in 0..20 {
            tend_encounters(&mut world, None, &mut run, 0.5);
        }
        let at = run
            .encounters
            .as_ref()
            .unwrap()
            .position(GroupId(0))
            .unwrap();
        assert!((at.x - 13.0).abs() < 0.1, "asleep, it walked to {at}");
        // The protagonist comes within reach: it wakes there.
        let hero = run.player.unwrap();
        let near = Vec3::new(-95.0, 0.0, 0.0);
        run.world.combatant_mut(hero).unwrap().position = near;
        run.walkers.get_mut(&hero).unwrap().feet = near + Vec3::Y * 0.02;
        tend_encounters(&mut world, None, &mut run, f32::INFINITY);
        let woke = centre_of(&run, "Bandit");
        assert!((woke.x - 13.0).abs() < 3.0, "woken at {woke}");
        // Awake, it walks on down the road.
        play(&mut world, &mut run, 30.0);
        let now = centre_of(&run, "Bandit");
        assert!(now.x > woke.x + 12.0, "from {woke} to {now}");
        assert!(now.z.abs() < 6.0, "on the road: {now}");
        assert_eq!(run.world.combatants().count(), 5, "nobody fought");
    }

    /// At dusk a woken wolf pack shadows the protagonist from thirty metres on
    /// its own side; by day it goes back to where it woke.
    #[test]
    fn wolves_shadow_the_protagonist_at_dusk() {
        use somnium_core::time_of_day::TimeOfDayComponent;
        let (mut world, _) = hero_scene(Vec3::ZERO);
        let sky = world.spawn((
            Name::new("Sky"),
            TimeOfDayComponent {
                enabled: true,
                fixed_sun: true,
                eclipse: 0.6,
                ..TimeOfDayComponent::default()
            },
        ));
        world.spawn((
            Name::new("Den"),
            Transform::from_translation(Vec3::new(80.0, 0.0, 0.0)),
            WorldTransform::identity(),
            CombatEncounterComponent {
                kind: "wolves".into(),
                ..CombatEncounterComponent::default()
            },
        ));
        somnium_core::propagate_transforms(&mut world);
        let mut run = build(&world).expect("a fight");
        run.encounters = Some(encounters_asleep(&world));
        tend_encounters(&mut world, None, &mut run, f32::INFINITY);
        let homes = |run: &Run| -> Vec<(Vec3, Vec3)> {
            run.world
                .combatants()
                .filter(|c| c.name.starts_with("Rime wolf"))
                .map(|c| (c.brain.as_ref().unwrap().mind.home.unwrap(), c.position))
                .collect()
        };
        let trail = homes(&run);
        assert_eq!(trail.len(), 5, "the pack woke");
        let shadow = Vec3::new(SHADOW, 0.0, 0.0);
        assert!(
            trail.iter().all(|(home, _)| home.distance(shadow) < 2.0),
            "on the trail thirty metres off, its side: {trail:?}"
        );
        // Day under the fixed sun: back to where each woke.
        world.get_mut::<TimeOfDayComponent>(sky).unwrap().eclipse = 0.0;
        tend_encounters(&mut world, None, &mut run, 0.5);
        assert!(
            homes(&run)
                .iter()
                .all(|(home, at)| home.distance(*at) < 0.5),
            "home again: {:?}",
            homes(&run)
        );
    }

    /// What patrols cost: bands walking their roads awake (the fight, the
    /// walkers and the looks every half second), and the records of a
    /// Tyffir's worth of camps, packs and patrols asleep. Run with
    /// `cargo test --release -p hello_engine --bin hello_engine patrol_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement"]
    fn patrol_cost() {
        for bands in [1usize, 4, 16] {
            let (mut world, _) = hero_scene(Vec3::new(0.0, 0.0, -20.0));
            for b in 0..bands {
                let z = (b % 8) as f32 * 12.0 + 10.0;
                let x = if b < 8 { -60.0 } else { 20.0 };
                patrol(&mut world, Vec3::new(x, 0.0, z), 200.0);
            }
            somnium_core::propagate_transforms(&mut world);
            let mut run = build(&world).expect("a fight");
            run.encounters = Some(encounters_asleep(&world));
            tend_encounters(&mut world, None, &mut run, f32::INFINITY);
            let awake = run.encounters.as_ref().unwrap().awake().count();
            play(&mut world, &mut run, 2.0);
            let started = std::time::Instant::now();
            play(&mut world, &mut run, 5.0);
            let per = started.elapsed().as_secs_f64() * 1e3 / 300.0;
            let started = std::time::Instant::now();
            for _ in 0..100 {
                tend_encounters(&mut world, None, &mut run, 0.5);
            }
            let look = started.elapsed().as_secs_f64() * 1e3 / 100.0;
            println!(
                "{awake} bands awake ({} bodies): {per:.3} ms a step, {:.3} a band; a look {look:.3} ms",
                run.world.combatants().count(),
                per / awake.max(1) as f64
            );
        }
        // Asleep: 131 camps and packs and 25 patrols over 10 km, the
        // protagonist far from all.
        let (mut world, _) = hero_scene(Vec3::new(-5000.0, 0.0, -5000.0));
        for i in 0..131 {
            let a = i as f32 * 0.37;
            world.spawn((
                Name::new("Encounter"),
                Transform::from_translation(Vec3::new(
                    a.sin() * 4000.0,
                    0.0,
                    a.cos() * 4000.0 + i as f32,
                )),
                WorldTransform::identity(),
                CombatEncounterComponent {
                    kind: if i % 5 == 0 { "bandits" } else { "wolves" }.into(),
                    ..CombatEncounterComponent::default()
                },
            ));
        }
        for i in 0..25 {
            patrol(
                &mut world,
                Vec3::new(i as f32 * 300.0 - 3750.0, 0.0, 3000.0),
                400.0,
            );
        }
        somnium_core::propagate_transforms(&mut world);
        let mut run = build(&world).expect("a fight");
        run.encounters = Some(encounters_asleep(&world));
        tend_encounters(&mut world, None, &mut run, f32::INFINITY);
        let started = std::time::Instant::now();
        for _ in 0..200 {
            tend_encounters(&mut world, None, &mut run, 0.5);
        }
        let look = started.elapsed().as_secs_f64() * 1e3 / 200.0;
        println!(
            "156 asleep ({} records), a look {look:.3} ms",
            run.encounters.as_ref().unwrap().len()
        );
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
        expand_encounters(&mut world, None);
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
