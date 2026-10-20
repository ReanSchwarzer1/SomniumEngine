//! Fighters and mounts drawn as skinned bodies (Rimeborn's Act VI slice).
//!
//! A Combatant entity that also carries a `hello.SkinnedBody` (Caelan with
//! `caelan_adult`, a horse with its own rig) draws through it in Play instead
//! of the cylinder, head and blade: the host takes the body off the fighter
//! (whose transform is the scaled cylinder), puts it on an entity of its own
//! at the fighter's feet, hides the shapes, and gives everything back at
//! Stop. Fighters without a body keep their shapes.
//!
//! Each frame the body is **driven**: the clip and its time come from the
//! fighter's combat state ([`pose_of`]): idle, walk, run and sprint by speed
//! and direction; a move's clip warped so its hit lands on the move's frame
//! (`ClipTable::warp`); guard, dodge by direction, cast (gather, channel,
//! release); vault, climb, swim and falling from the character controller;
//! hit reactions and death; riding. Clips may list alternatives (`A|B`), so
//! one rule serves people and beasts: a wolf has no `Loco_Walk_F_Loop` and
//! plays its `Walk_F_Loop`. Move names resolve through the project's
//! `animation/clips.json` aliases.
//!
//! A fighter's weapon is WS3's mesh for its kind (`clips.json` `gear`: the
//! spear, axe and knife on `weapon_r`, the bow on `weapon_l`; a plain blade
//! for a sword until there is a sword mesh). A sword or knife goes to the
//! `sheath` socket when not fighting, beside its scabbard. A rider's Root sits
//! on the mount's `saddle` (WS3's ride library is made so), playing the
//! `Ride_*` loop of its mount's gait at its mount's rate, in step with it.

use crate::skinned_crowd::{SkinnedBody, SkinnedSocket};
use glam::{Mat4, Quat, Vec3};
use somnium_character::{Controller, MantleKind, Mode};
use somnium_combat::CombatantId;
use somnium_combat::clips::ClipTable;
use somnium_combat::mount::Herding;
use somnium_combat::weapon::WeaponKind;
use somnium_combat::world::{Action, CastPhase, CombatWorld, Combatant, Phase};
use somnium_core::{
    EditorFlags, ImportedMesh, MaterialComponent, MeshKind, Name, Parent, Transform, WorldTransform,
};
use somnium_ecs::{Entity, World};
use std::collections::{BTreeMap, BTreeSet};

/// Seconds after the last fighting before the blade goes back in its sheath.
const SHEATHE_AFTER: f64 = 6.0;
/// A hurt flinch lasts this long.
const FLINCH: f64 = 0.35;

/// How a clip runs this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Timing {
    /// Looping on the body's own clock at this rate.
    Loop(f32),
    /// At this moment of the clip (a move, a dodge, a climb).
    At(f32),
    /// From its start, once, held at its end (a flinch, a death).
    Once,
}

/// A mount's gait at `speed` (m/s), 0 standing to 4 galloping, and the rate its
/// clips play. WS3's `Ride_*` loops are each two of the mount's own cycles, so
/// a rider's plays at its mount's rate and the two keep step.
fn gait(speed: f32) -> (usize, f32) {
    match speed {
        s if s < 0.3 => (0, 1.0),
        s if s < 2.5 => (1, (s / 1.6).clamp(0.6, 1.6)),
        s if s < 5.5 => (2, (s / 3.5).clamp(0.6, 1.6)),
        s if s < 8.0 => (3, (s / 6.0).clamp(0.6, 1.6)),
        s => (4, (s / 9.0).clamp(0.6, 1.6)),
    }
}

/// A mount's clips by gait.
const MOUNT_GAITS: [&str; 5] = [
    "Idle_Loop",
    "Walk_F_Loop",
    "Trot_F_Loop|Walk_F_Loop",
    "Canter_F_Loop|Gallop_F_Loop|Trot_F_Loop",
    "Gallop_F_Loop|Canter_F_Loop|Trot_F_Loop",
];

/// A rider's clips by its mount's gait (a body without WS3's ride library,
/// which only the players carry, sits).
const RIDER_GAITS: [&str; 5] = [
    "Ride_Idle_Loop|Sit_Idle_Loop",
    "Ride_Walk_Loop|Ride_Idle_Loop|Sit_Idle_Loop",
    "Ride_Trot_Loop|Ride_Walk_Loop|Sit_Idle_Loop",
    "Ride_Canter_Loop|Ride_Gallop_Loop|Sit_Idle_Loop",
    "Ride_Gallop_Loop|Ride_Canter_Loop|Sit_Idle_Loop",
];

/// What a fighter's state calls for: a clip (alternatives `A|B`) and how it
/// runs. A rider's gait is its `steed`'s.
#[must_use]
pub fn pose_of(
    c: &Combatant,
    steed: Option<&Combatant>,
    mode: Option<Mode>,
    now: f64,
    drawn: bool,
    table: Option<&ClipTable>,
) -> (String, Timing) {
    let seconds = |clip: &str| -> Option<f32> {
        let t = table?;
        clip.split('|')
            .find_map(|n| t.clips.get(n).map(|c| c.seconds))
    };
    let at = |clip: &str, fraction: f32, fallback: f32| -> (String, Timing) {
        let time = seconds(clip).map_or(fallback, |s| fraction.clamp(0.0, 1.0) * s);
        (clip.to_string(), Timing::At(time))
    };
    // A parry opens the riposte window: the blade meets the blow.
    if now < c.riposte_until && matches!(c.action, Action::Guard { .. } | Action::Idle) {
        return (
            "Sword_Meridian_Parry_Mid|Sword_Aether_Rimeguard_Block|Idle_Loop".into(),
            Timing::Once,
        );
    }
    match &c.action {
        Action::Dead => return ("Death_F|Death".into(), Timing::Once),
        Action::Attack(a) => {
            let d = &a.def;
            let total = match a.phase {
                Phase::Windup => a.t,
                Phase::Active => d.windup + a.t,
                Phase::Recovery => d.windup + d.active + a.t,
            };
            let clip = resolve(table, &d.animation);
            let clip = if clip.is_empty() {
                "Sword_Basic_Attack|Attack_Bite".to_string()
            } else {
                clip
            };
            let time = table
                .and_then(|t| t.warp(d))
                .map_or(total, |w| w.clip_time(total));
            return (clip, Timing::At(time));
        }
        Action::Guard { .. } if stance(c.weapon.kind).is_some() => {
            let (guard, _) = stance(c.weapon.kind).unwrap();
            return (guard.into(), Timing::Loop(1.0));
        }
        Action::Aim { .. } if c.weapon.kind == WeaponKind::Bow => {
            return ("Bow_Aim_Loop|Idle_Loop".into(), Timing::Loop(1.0));
        }
        Action::Guard { t } => {
            let clip = "Sword_Aether_Rimeguard_Block|Sword_Basic_Block|Idle_Loop";
            let hold = seconds(clip).map_or(0.3, |s| s * 0.5);
            return (clip.into(), Timing::At(t.min(hold)));
        }
        Action::Dodge(d) => {
            let (fwd, side) = relative(c.facing, d.dir);
            let clip = if fwd > 0.5 {
                "Roll_F|Trot_F_Loop"
            } else if fwd < -0.5 {
                "Dodge_B|Trot_F_Loop"
            } else if side > 0.0 {
                "Dodge_R|Trot_F_Loop"
            } else {
                "Dodge_L|Trot_F_Loop"
            };
            return at(clip, d.t / d.duration.max(1e-3), d.t);
        }
        Action::Cast(s) => {
            return match s.phase {
                CastPhase::Gather => (
                    "Cast_Incant_Loop|Cast_Basic_Idle_Loop|Idle_Loop".into(),
                    Timing::Loop(1.0),
                ),
                CastPhase::Channel => (
                    "Cast_Channel_Loop|Cast_Basic_Idle_Loop|Idle_Loop".into(),
                    Timing::Loop(1.0),
                ),
                CastPhase::Recover => (
                    "Cast_Release_Bolt|Cast_Basic_Release|Idle_Loop".into(),
                    Timing::At(s.t),
                ),
            };
        }
        Action::Idle | Action::Aim { .. } | Action::Reload { .. } | Action::Recover { .. } => {}
    }
    if c.alive() && now - c.last_hurt < FLINCH && c.last_hurt > 0.0 {
        return ("Hit_F_Light|Hit".into(), Timing::Once);
    }
    let flat = Vec3::new(c.velocity.x, 0.0, c.velocity.z);
    let speed = flat.length();
    if c.mount.is_some() {
        // Carried, the rider has no pace of its own: its mount's.
        let paced = steed.map_or(speed, |m| {
            Vec3::new(m.velocity.x, 0.0, m.velocity.z).length()
        });
        let (g, rate) = gait(paced);
        return (RIDER_GAITS[g].into(), Timing::Loop(rate));
    }
    match mode {
        Some(Mode::Mantle(m)) => {
            let clip = match m.kind {
                MantleKind::Vault => "Vault_Low|Jump_Start",
                MantleKind::Climb => "Climb_Wall_1m|Jump_Start",
            };
            return at(clip, m.t / m.duration.max(1e-3), m.t);
        }
        Some(Mode::Swim) => {
            return if speed > 0.3 {
                (
                    "Swim_F_Loop|Walk_F_Loop".into(),
                    Timing::Loop((speed / 1.2).clamp(0.6, 1.8)),
                )
            } else {
                ("Swim_Idle_Loop|Idle_Loop".into(), Timing::Loop(1.0))
            };
        }
        Some(Mode::Air) => return ("Jump_Loop|Idle_Loop".into(), Timing::Loop(1.0)),
        Some(Mode::Ground) | None => {}
    }
    if c.mount_def.is_some() {
        // A mount's own gaits.
        let grazing = c
            .herd
            .as_ref()
            .is_some_and(|h| matches!(h.doing, Herding::Grazing));
        let (g, rate) = gait(speed);
        let clip = if g == 0 && grazing {
            "Graze_Loop|Idle_Loop"
        } else {
            MOUNT_GAITS[g]
        };
        return (clip.into(), Timing::Loop(rate));
    }
    if speed < 0.25 {
        // A spearman (the watch, a garrison) stands sentry out of a fight and
        // on guard in one; an axe or knife on guard with it out.
        if let Some((guard, sentry)) = stance(c.weapon.kind) {
            let clip = if drawn { guard } else { sentry };
            return (clip.into(), Timing::Loop(1.0));
        }
        let clip = if drawn {
            "Sword_Aether_Idle_Loop|Sword_Basic_Idle_Loop|Idle_Neutral_Loop|Idle_Loop"
        } else {
            "Idle_Neutral_Loop|Idle_Loop"
        };
        return (clip.into(), Timing::Loop(1.0));
    }
    let (fwd, side) = relative(c.facing, flat / speed);
    let way = if fwd < -0.5 {
        'B'
    } else if side.abs() > 0.7 {
        if side > 0.0 { 'R' } else { 'L' }
    } else {
        'F'
    };
    let (clip, rate) = if speed < 2.2 {
        (
            format!("Loco_Walk_{way}_Loop|Loco_Walk_F_Loop|Walk_F_Loop"),
            (speed / 0.86).clamp(0.6, 2.4),
        )
    } else if speed < 6.5 {
        let clip = if way == 'F' {
            "Loco_Jog_F_Loop".to_string()
        } else {
            format!("Loco_Run_{way}_Loop")
        };
        (
            format!("{clip}|Loco_Jog_F_Loop|Trot_F_Loop|Gallop_F_Loop"),
            (speed / 5.6).clamp(0.75, 1.4),
        )
    } else {
        (
            "Loco_Sprint_F_Loop|Loco_Jog_F_Loop|Gallop_F_Loop".to_string(),
            (speed / 7.5).clamp(0.8, 1.4),
        )
    };
    (clip, Timing::Loop(rate))
}

/// A move's clip name: its alias in the table (`wolf_bite` -> `rime_wolf/Attack_Bite`, the creature's prefix dropped), else
/// the name itself.
fn resolve(table: Option<&ClipTable>, animation: &str) -> String {
    let name = table
        .and_then(|t| t.aliases.get(animation))
        .map_or(animation, String::as_str);
    name.rsplit('/').next().unwrap_or(name).to_string()
}

/// How much of `dir` is forward of `facing` and how much to its right (glTF bodies face +Z).
fn relative(facing: Vec3, dir: Vec3) -> (f32, f32) {
    let f = Vec3::new(facing.x, 0.0, facing.z).normalize_or(Vec3::Z);
    let right = Vec3::new(-f.z, 0.0, f.x);
    (dir.dot(f), dir.dot(right))
}

fn yaw(facing: Vec3) -> Quat {
    let f = Vec3::new(facing.x, 0.0, facing.z).normalize_or(Vec3::Z);
    Quat::from_rotation_y(f.x.atan2(f.z))
}

/// A piece of WS3's gear (`clips.json` `gear`): its file under the assets, nodes and joint.
#[derive(Clone, Debug, PartialEq)]
struct Piece {
    file: String,
    nodes: Vec<u32>,
    joint: String,
}

/// The piece a weapon of `kind` is, and whether it goes to the sheath out of a fight.
fn piece_for(kind: WeaponKind) -> Option<(&'static str, bool)> {
    match kind {
        WeaponKind::Sword => Some(("sword", true)),
        WeaponKind::Knife => Some(("knife", true)),
        WeaponKind::Spear | WeaponKind::LanceGlaive => Some(("spear", false)),
        WeaponKind::Axe => Some(("axe", false)),
        WeaponKind::Bow => Some(("bow", false)),
        WeaponKind::LatticeRifle => Some(("rifle", false)),
        _ => None,
    }
}

/// A weapon's own stances (on guard, and standing out of a fight), where WS3
/// made them; a sword's are the blade's idles and blocks.
fn stance(kind: WeaponKind) -> Option<(&'static str, &'static str)> {
    match kind {
        WeaponKind::Spear | WeaponKind::LanceGlaive => Some((
            "Spear_Guard_Loop|Idle_Neutral_Loop|Idle_Loop",
            "Spear_Sentry_Loop|Idle_Neutral_Loop|Idle_Loop",
        )),
        WeaponKind::Axe => Some((
            "Axe_Guard_Loop|Idle_Neutral_Loop|Idle_Loop",
            "Idle_Neutral_Loop|Idle_Loop",
        )),
        WeaponKind::Knife => Some((
            "Knife_Guard_Loop|Idle_Neutral_Loop|Idle_Loop",
            "Idle_Neutral_Loop|Idle_Loop",
        )),
        _ => None,
    }
}

/// One fighter's body.
struct Skin {
    body: Entity,
    /// The weapon's socket, the joint it is held on, and whether it is sheathed out of a fight.
    weapon: Option<(Entity, String, bool)>,
    /// A mount's saddle socket, where its rider sits.
    saddle: Option<Entity>,
    /// A swordsman's scabbard, on the `sheath` joint.
    scabbard: Option<Entity>,
    clip: String,
    clock: f32,
    /// When it last fought (the blade comes out, and stays out a while).
    fought: f64,
    /// Its weapon is in its hand (not in the sheath).
    in_hand: bool,
    /// A draw (true) or a sheathe begun then: the blade changes joint at the clip's grab.
    swap: Option<(f64, bool)>,
    /// The work clip whose tools it holds (clips.json `gear`), and their sockets.
    worn: Option<(String, Vec<Entity>)>,
}

/// WS3's grab in Sword_Draw and release in Sword_Sheathe, unless clips.json says (`events.draw`, `events.sheathe`).
const DRAW_GRAB: f64 = 0.2;
const SHEATHE_RELEASE: f64 = 0.6;

impl Skin {
    /// The body and everything placed on it (despawn does not reach children).
    fn despawn(self, world: &mut World) {
        let sockets: Vec<Entity> = self
            .weapon
            .map(|w| w.0)
            .into_iter()
            .chain(self.saddle)
            .chain(self.scabbard)
            .chain(self.worn.into_iter().flat_map(|w| w.1))
            .collect();
        let mut parts: Vec<Entity> = world
            .iter_with::<Parent>()
            .filter(|(_, p)| sockets.contains(&p.entity))
            .map(|(e, _)| e)
            .collect();
        parts.extend(sockets);
        parts.push(self.body);
        for e in parts {
            world.despawn(e);
        }
    }
}

/// The nodes of a glTF binary that carry meshes, from its JSON chunk.
pub(crate) fn mesh_nodes(path: &std::path::Path) -> Vec<u32> {
    mesh_node_names(path).into_iter().map(|(i, _)| i).collect()
}

/// The nodes of a glTF binary that carry meshes, with their names.
pub(crate) fn mesh_node_names(path: &std::path::Path) -> Vec<(u32, String)> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let Some(len) = bytes
        .get(12..16)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    else {
        return Vec::new();
    };
    let doc: serde_json::Value = bytes
        .get(20..20 + len)
        .and_then(|j| serde_json::from_slice(j).ok())
        .unwrap_or_default();
    doc.get("nodes")
        .and_then(|n| n.as_array())
        .into_iter()
        .flatten()
        .enumerate()
        .filter(|(_, n)| n.get("mesh").is_some())
        .map(|(i, n)| {
            (
                i as u32,
                n.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .collect()
}

/// A piece's meshes, hung on `socket`.
fn hang(world: &mut World, socket: Entity, name: &str, piece: &str, g: &Piece) {
    for &node in &g.nodes {
        world.spawn((
            Name::new(&format!("{name} {piece} {node}")),
            Transform::default(),
            WorldTransform::identity(),
            Parent { entity: socket },
            ImportedMesh {
                source: format!("assets/{}", g.file),
                node,
            },
            MaterialComponent {
                asset: somnium_asset::database::AssetId::NONE,
                runtime_id: 0,
            },
        ));
    }
}

/// WS3's scabbard (`characters/gear/scabbard.glb`, in the `sheath` frame), where it is there.
const SCABBARD: &str = "characters/gear/scabbard.glb";

/// Which pieces each clip holds (clips.json's `gear` table: clip to pieces).
fn clip_gear(text: &str) -> BTreeMap<String, Vec<String>> {
    let doc: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
    doc.get("gear")
        .and_then(|g| g.as_object())
        .into_iter()
        .flatten()
        .map(|(clip, list)| {
            let pieces = list
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|g| g.get("piece").and_then(|v| v.as_str()).map(str::to_string))
                .collect();
            (clip.clone(), pieces)
        })
        .collect()
}

/// A socket on `body`'s `joint`.
fn socket_on(world: &mut World, body: Entity, joint: &str) -> Entity {
    world.spawn((
        Name::new(joint),
        Transform::default(),
        WorldTransform::identity(),
        Parent { entity: body },
        SkinnedSocket {
            joint: joint.into(),
        },
    ))
}

/// Each gear piece `clips.json` names (its `gear` table, clip to pieces), once, where its file is there.
fn gear_pieces(content: &std::path::Path, text: &str) -> BTreeMap<String, Piece> {
    let doc: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
    let mut pieces = BTreeMap::new();
    for list in doc
        .get("gear")
        .and_then(|g| g.as_object())
        .into_iter()
        .flat_map(|g| g.values())
    {
        for g in list.as_array().into_iter().flatten() {
            let (Some(name), Some(file), Some(joint)) = (
                g.get("piece").and_then(|v| v.as_str()),
                g.get("file").and_then(|v| v.as_str()),
                g.get("joint").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            if pieces.contains_key(name) || !content.join(file).exists() {
                continue;
            }
            let nodes = g
                .get("nodes")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|n| n.as_u64())
                        .map(|n| n as u32)
                        .collect()
                })
                .unwrap_or_default();
            pieces.insert(
                name.to_string(),
                Piece {
                    file: file.to_string(),
                    nodes,
                    joint: joint.to_string(),
                },
            );
        }
    }
    pieces
}

/// The fight's skinned bodies.
#[derive(Default)]
pub struct Skins {
    table: Option<ClipTable>,
    /// WS3's gear pieces by name, the ones whose files are there.
    gear: BTreeMap<String, Piece>,
    seen: BTreeSet<CombatantId>,
    skins: BTreeMap<CombatantId, Skin>,
    /// Bodies taken off fighters and shapes hidden (with the flags they had), given back at Stop.
    taken: Vec<(Entity, SkinnedBody)>,
    /// A fighter sitting in a boat (the combat host sets it), the clip it plays there and its rate (rowing, holding
    /// the clip's oars, or at the tiller).
    pub seated: Option<(CombatantId, (String, f32))>,
    /// A fighter at work (the game's activity clip, `A|B`, looped while it stands idle), holding the clip's tools.
    pub work: Option<(CombatantId, String)>,
    /// Each clip's pieces (clips.json's `gear`).
    clip_gear: BTreeMap<String, Vec<String>>,
    hidden: Vec<(Entity, Option<EditorFlags>)>,
}

impl Skins {
    /// With the project's clip table (`<content>/animation/clips.json`), if it has one.
    #[must_use]
    pub fn new(content: Option<&std::path::Path>) -> Self {
        let text =
            content.and_then(|c| std::fs::read_to_string(c.join("animation/clips.json")).ok());
        let table = text.as_deref().and_then(|t| ClipTable::from_json(t).ok());
        let mut gear = match (content, text.as_deref()) {
            (Some(content), Some(text)) => gear_pieces(content, text),
            _ => BTreeMap::new(),
        };
        if let Some(path) = content.map(|c| c.join(SCABBARD)).filter(|p| p.exists()) {
            gear.insert(
                "scabbard".into(),
                Piece {
                    file: SCABBARD.into(),
                    nodes: mesh_nodes(&path),
                    joint: "sheath".into(),
                },
            );
        }
        Self {
            table,
            gear,
            clip_gear: text.as_deref().map(clip_gear).unwrap_or_default(),
            ..Default::default()
        }
    }

    /// The skinned body drawn for fighter `id`, if it wears one.
    pub(crate) fn body_of(&self, id: CombatantId) -> Option<Entity> {
        self.skins.get(&id).map(|s| s.body)
    }

    /// Fighters drawn as bodies.
    #[must_use]
    pub fn count(&self) -> usize {
        self.skins.len()
    }

    /// Give each newly enlisted fighter that carries a Skinned Body its body:
    /// an entity of its own at its feet (the new entities go into `spawned`),
    /// its shapes hidden.
    pub fn dress(
        &mut self,
        world: &mut World,
        fighters: &BTreeMap<CombatantId, Entity>,
        combat: &CombatWorld,
        spawned: &mut Vec<Entity>,
    ) {
        let new: Vec<(CombatantId, Entity)> = fighters
            .iter()
            .filter(|(id, _)| !self.seen.contains(id))
            .map(|(id, e)| (*id, *e))
            .collect();
        for (id, fighter) in new {
            self.seen.insert(id);
            let Some(look) = world.get::<SkinnedBody>(fighter).cloned() else {
                continue;
            };
            let Some(c) = combat.combatant(id) else {
                continue;
            };
            let name = world
                .get::<Name>(fighter)
                .map_or("Fighter", |n| n.as_str())
                .to_string();
            let body = world.spawn((
                Name::new(&format!("{name} body")),
                Transform {
                    translation: c.position,
                    rotation: yaw(c.facing),
                    ..Default::default()
                },
                WorldTransform::identity(),
                SkinnedBody {
                    driven: true,
                    ..look.clone()
                },
            ));
            spawned.push(body);
            let _ = world.remove_component::<SkinnedBody>(fighter);
            self.taken.push((fighter, look));
            // The cylinder, and its head and blade.
            let children: Vec<Entity> = world
                .iter_with::<Parent>()
                .filter(|(_, p)| p.entity == fighter)
                .map(|(e, _)| e)
                .collect();
            // Hidden from the mesh loop (which draws their uploaded meshes, not the kinds).
            for e in std::iter::once(fighter).chain(children) {
                if world.get::<MeshKind>(e).is_none() {
                    continue;
                }
                let had = world.get::<EditorFlags>(e).copied();
                let flags = EditorFlags {
                    hidden: true,
                    ..had.unwrap_or_default()
                };
                let _ = world.insert_component(e, flags);
                self.hidden.push((e, had));
            }
            let socket = |world: &mut World, joint: &str| {
                world.spawn((
                    Name::new(joint),
                    Transform::default(),
                    WorldTransform::identity(),
                    Parent { entity: body },
                    SkinnedSocket {
                        joint: joint.into(),
                    },
                ))
            };
            // Its weapon: WS3's mesh for the kind on its joint; a sword without a
            // mesh is a plain blade.
            let weapon =
                piece_for(c.weapon.kind).and_then(|(piece, sheathes)| match self.gear.get(piece) {
                    Some(g) => {
                        // Out of a fight to begin with: a sword or knife starts in its sheath.
                        let s = socket(world, if sheathes { "sheath" } else { &g.joint });
                        hang(world, s, &name, piece, g);
                        Some((s, g.joint.clone(), sheathes))
                    }
                    None if piece == "sword" => {
                        let s = socket(world, "sheath");
                        world.spawn((
                            Name::new(&format!("{name} sword")),
                            Transform {
                                translation: Vec3::new(0.0, 0.45, 0.0),
                                scale: Vec3::new(0.035, 0.9, 0.012),
                                ..Default::default()
                            },
                            WorldTransform::identity(),
                            Parent { entity: s },
                            MeshKind::Cube,
                            crate::combat_demo::material("dark"),
                        ));
                        Some((s, "weapon_r".to_string(), true))
                    }
                    None => None,
                });
            // A swordsman wears the scabbard the blade goes back into.
            let scabbard = self
                .gear
                .get("scabbard")
                .filter(|_| c.weapon.kind == WeaponKind::Sword)
                .map(|g| {
                    let s = socket(world, "sheath");
                    hang(world, s, &name, "scabbard", g);
                    s
                });
            let saddle = c.mount_def.is_some().then(|| socket(world, "saddle"));
            // A sheathed weapon starts in its sheath; any other in hand.
            let in_hand = !weapon.as_ref().is_some_and(|w| w.2);
            self.skins.insert(
                id,
                Skin {
                    body,
                    weapon,
                    saddle,
                    scabbard,
                    clip: String::new(),
                    clock: 0.0,
                    fought: f64::NEG_INFINITY,
                    in_hand,
                    swap: None,
                    worn: None,
                },
            );
        }
    }

    /// Each body where its fighter stands (a rider on its mount's saddle),
    /// playing what its state calls for, its blade drawn or sheathed.
    pub fn write(
        &mut self,
        world: &mut World,
        combat: &CombatWorld,
        walkers: &BTreeMap<CombatantId, Controller>,
        dt: f32,
    ) {
        let now = combat.time();
        // A fighter gone from the fight (its encounter fell asleep) takes its
        // body, sockets and gear with it; waking again is a new fighter.
        let gone: Vec<CombatantId> = self
            .skins
            .keys()
            .filter(|id| combat.combatant(**id).is_none())
            .copied()
            .collect();
        for id in gone {
            self.skins.remove(&id).expect("listed").despawn(world);
        }
        // Saddles where they were last drawn (a frame behind the mount's pose).
        let saddles: BTreeMap<CombatantId, Mat4> = self
            .skins
            .iter()
            .filter_map(|(id, s)| Some((*id, world.get::<WorldTransform>(s.saddle?)?.0)))
            .collect();
        for (id, skin) in &mut self.skins {
            let Some(c) = combat.combatant(*id) else {
                continue;
            };
            if !matches!(c.action, Action::Idle) || c.lock.is_some() {
                skin.fought = now;
            }
            let drawn = now - skin.fought < SHEATHE_AFTER;
            let mode = walkers.get(id).map(|w| w.mode);
            let steed = c.mount.and_then(|m| combat.combatant(m));
            // A sword out of its sheath at Sword_Draw's grab and back at
            // Sword_Sheathe's release, the clip played standing; in the hand at
            // once when a move cannot wait.
            let sheathes = skin.weapon.as_ref().is_some_and(|w| w.2);
            let mut swapping = None;
            if sheathes {
                if !matches!(c.action, Action::Idle) {
                    skin.in_hand = true;
                    skin.swap = None;
                } else if skin.swap.is_none() && drawn != skin.in_hand {
                    skin.swap = Some((now, drawn));
                }
            }
            if let Some((start, drawing)) = skin.swap {
                let (name, event, grab) = if drawing {
                    ("Sword_Draw", "draw", DRAW_GRAB)
                } else {
                    ("Sword_Sheathe", "sheathe", SHEATHE_RELEASE)
                };
                let info = self.table.as_ref().and_then(|t| t.clips.get(name));
                let grab = info
                    .and_then(|i| i.events.get(event))
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(grab);
                let length = info.map_or(grab + 0.3, |i| f64::from(i.seconds));
                let t = now - start;
                if t >= grab {
                    skin.in_hand = drawing;
                }
                let standing =
                    Vec3::new(c.velocity.x, 0.0, c.velocity.z).length() < 0.25 && c.mount.is_none();
                if t >= length {
                    skin.swap = None;
                } else if standing {
                    swapping = Some((
                        format!("{name}|Idle_Neutral_Loop|Idle_Loop"),
                        Timing::At(t as f32),
                    ));
                }
            }
            let shown = if sheathes { skin.in_hand } else { drawn };
            let idle = matches!(c.action, Action::Idle);
            let work = self
                .work
                .as_ref()
                .filter(|(w, _)| w == id && idle)
                .map(|(_, clip)| clip.clone());
            let seat = self
                .seated
                .as_ref()
                .filter(|(s, _)| s == id && idle)
                .map(|(_, pose)| pose.clone());
            // The tools its work (or its seat's) clip holds, swapped when that changes.
            let tools = seat
                .as_ref()
                .map(|(clip, _)| clip.as_str())
                .or(work.as_deref())
                .and_then(|w| w.split('|').find(|n| self.clip_gear.contains_key(*n)))
                .map(str::to_string);
            if skin.worn.as_ref().map(|w| &w.0) != tools.as_ref() {
                for s in skin.worn.take().into_iter().flat_map(|w| w.1) {
                    let held: Vec<Entity> = world
                        .iter_with::<Parent>()
                        .filter(|(_, p)| p.entity == s)
                        .map(|(e, _)| e)
                        .collect();
                    for e in held.into_iter().chain([s]) {
                        world.despawn(e);
                    }
                }
                if let Some(t) = tools {
                    let name = world
                        .get::<Name>(skin.body)
                        .map_or("Fighter", |n| n.as_str())
                        .to_string();
                    let mut sockets = Vec::new();
                    for piece in self.clip_gear.get(&t).into_iter().flatten() {
                        if let Some(g) = self.gear.get(piece) {
                            let s = socket_on(world, skin.body, &g.joint);
                            hang(world, s, &name, piece, g);
                            sockets.push(s);
                        }
                    }
                    skin.worn = Some((t, sockets));
                }
            }
            let (clip, timing) = if let Some((clip, rate)) = seat {
                (clip, Timing::Loop(rate))
            } else if let Some(work) = work {
                (work, Timing::Loop(1.0))
            } else if let Some(swap) = swapping {
                swap
            } else {
                pose_of(c, steed, mode, now, shown, self.table.as_ref())
            };
            if clip != skin.clip {
                skin.clip.clone_from(&clip);
                skin.clock = 0.0;
            }
            let time = match timing {
                Timing::Loop(rate) => {
                    skin.clock += dt * rate;
                    skin.clock
                }
                Timing::At(t) => t,
                Timing::Once => {
                    let end = self
                        .table
                        .as_ref()
                        .and_then(|t| {
                            clip.split('|')
                                .find_map(|n| t.clips.get(n).map(|c| c.seconds))
                        })
                        .unwrap_or(1.5);
                    skin.clock = (skin.clock + dt).min((end - 0.05).max(0.0));
                    skin.clock
                }
            };
            // A rider sits on its mount's saddle when the mount has one drawn near it.
            let seat = c
                .mount
                .and_then(|m| saddles.get(&m))
                .filter(|s| s.w_axis.truncate().distance(c.position) < 3.0);
            let (at, turn) = match seat {
                // Its Root on the saddle, no offset (WS3: the ride loops sit the
                // pelvis 0.10 m above it, legs astride).
                Some(s) => {
                    let (_, r, t) = s.to_scale_rotation_translation();
                    (t, r)
                }
                None => (c.position, yaw(c.facing)),
            };
            if let Some(t) = world.get_mut::<Transform>(skin.body) {
                t.translation = at;
                t.rotation = turn;
            }
            if let Some(b) = world.get_mut::<SkinnedBody>(skin.body) {
                if b.clip != clip {
                    b.clip.clone_from(&clip);
                }
                b.time = time;
            }
            if let Some((s, held, _)) = &skin.weapon {
                let joint = if skin.in_hand {
                    held.as_str()
                } else {
                    "sheath"
                };
                if let Some(sock) = world.get_mut::<SkinnedSocket>(*s)
                    && sock.joint != joint
                {
                    sock.joint = joint.into();
                }
            }
        }
        // A rider in its mount's gait keeps its mount's clock: the two loops
        // start together and run at one rate, so stride and seat agree.
        let clocks: BTreeMap<CombatantId, f32> =
            self.skins.iter().map(|(id, s)| (*id, s.clock)).collect();
        for (id, skin) in &mut self.skins {
            let Some(steed) = combat.combatant(*id).and_then(|c| c.mount) else {
                continue;
            };
            if !skin.clip.starts_with("Ride_") {
                continue;
            }
            if let Some(&clock) = clocks.get(&steed) {
                skin.clock = clock;
                if let Some(b) = world.get_mut::<SkinnedBody>(skin.body) {
                    b.time = clock;
                }
            }
        }
    }

    /// Give the fighters their bodies and shapes back (Stop), the bodies made
    /// for them gone with their sockets and gear.
    pub fn end(self, world: &mut World) {
        for (_, skin) in self.skins {
            skin.despawn(world);
        }
        for (e, body) in self.taken {
            let _ = world.insert_component(e, body);
        }
        for (e, had) in self.hidden {
            match had {
                Some(flags) => {
                    let _ = world.insert_component(e, flags);
                }
                None => {
                    let _ = world.remove_component::<EditorFlags>(e);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_combat::world::CombatantDesc;

    fn fighter() -> (CombatWorld, CombatantId) {
        let mut w = CombatWorld::new(somnium_combat::CombatRules::default());
        let id = w.spawn(CombatantDesc::new(
            "Caelan",
            somnium_combat::TeamId(0),
            Vec3::ZERO,
            somnium_combat::weapon::sword(),
        ));
        (w, id)
    }

    /// Standing, walking, jogging and backing off pick their clips and rates;
    /// with the blade out it stands on guard.
    #[test]
    fn locomotion_by_speed_and_way() {
        let (mut w, id) = fighter();
        let c = w.combatant_mut(id).unwrap();
        c.facing = Vec3::Z;
        c.velocity = Vec3::ZERO;
        let (clip, t) = pose_of(c, None, Some(Mode::Ground), 10.0, false, None);
        assert!(clip.starts_with("Idle_Neutral_Loop"));
        assert_eq!(t, Timing::Loop(1.0));
        let (clip, _) = pose_of(c, None, Some(Mode::Ground), 10.0, true, None);
        assert!(
            clip.starts_with("Sword_Aether_Idle_Loop"),
            "on guard with the blade out"
        );
        c.velocity = Vec3::new(0.0, 0.0, 1.0);
        let (clip, t) = pose_of(c, None, Some(Mode::Ground), 10.0, false, None);
        assert!(clip.starts_with("Loco_Walk_F_Loop"));
        assert!(matches!(t, Timing::Loop(r) if (r - 1.0 / 0.86).abs() < 1e-4));
        c.velocity = Vec3::new(0.0, 0.0, 5.6);
        assert!(
            pose_of(c, None, Some(Mode::Ground), 10.0, false, None)
                .0
                .starts_with("Loco_Jog_F_Loop")
        );
        c.velocity = Vec3::new(0.0, 0.0, -1.0);
        assert!(
            pose_of(c, None, Some(Mode::Ground), 10.0, false, None)
                .0
                .starts_with("Loco_Walk_B_Loop")
        );
        c.velocity = Vec3::new(-1.0, 0.0, 0.0);
        assert!(
            pose_of(c, None, Some(Mode::Ground), 10.0, false, None)
                .0
                .starts_with("Loco_Walk_R_Loop"),
            "facing +Z, -X is the right"
        );
        c.velocity = Vec3::new(0.0, 0.0, 1.0);
        assert!(
            pose_of(c, None, Some(Mode::Swim), 10.0, false, None)
                .0
                .starts_with("Swim_F_Loop")
        );
        assert!(
            pose_of(c, None, Some(Mode::Air), 10.0, false, None)
                .0
                .starts_with("Jump_Loop")
        );
    }

    /// A swordsman draws at Sword_Draw's grab (clips.json's `events.draw`), the
    /// clip playing as he stands, and sheathes at Sword_Sheathe's release once
    /// the fight is long over; a move that cannot wait has the blade in hand.
    #[test]
    fn the_blade_changes_hands_at_the_grab() {
        let table = ClipTable::from_json(
            r#"{"clips": {
                "Sword_Draw": {"seconds": 0.7, "events": {"draw": 0.2}},
                "Sword_Sheathe": {"seconds": 0.9, "events": {"sheathe": 0.6}}}, "aliases": {}}"#,
        )
        .unwrap();
        let (mut combat, id) = fighter();
        let foe = combat.spawn(CombatantDesc::new(
            "Raider",
            somnium_combat::TeamId(1),
            Vec3::new(30.0, 0.0, 0.0),
            somnium_combat::weapon::sword(),
        ));
        let mut world = World::new();
        let caelan = world.spawn((
            Name::new("Caelan"),
            Transform::default(),
            WorldTransform::identity(),
            SkinnedBody::default(),
        ));
        let mut skins = Skins {
            table: Some(table),
            ..Default::default()
        };
        let mut spawned = Vec::new();
        skins.dress(
            &mut world,
            &BTreeMap::from([(id, caelan)]),
            &combat,
            &mut spawned,
        );
        let socket = world
            .iter_with::<SkinnedSocket>()
            .map(|(e, _)| e)
            .next()
            .expect("the blade's socket");
        let joint = |world: &World| world.get::<SkinnedSocket>(socket).unwrap().joint.clone();
        let body = spawned[0];
        let clip = |world: &World| world.get::<SkinnedBody>(body).unwrap().clip.clone();
        let open = somnium_combat::OpenField;
        let tick =
            |combat: &mut CombatWorld, world: &mut World, skins: &mut Skins, seconds: f32| {
                for _ in 0..(seconds * 60.0).round() as usize {
                    combat.step(1.0 / 60.0, &open, None);
                    skins.write(world, combat, &BTreeMap::new(), 1.0 / 60.0);
                }
            };
        assert_eq!(joint(&world), "sheath");
        combat.combatant_mut(id).unwrap().lock = Some(foe);
        tick(&mut combat, &mut world, &mut skins, 0.1);
        assert_eq!(joint(&world), "sheath", "the hand is not on the hilt yet");
        assert!(
            clip(&world).starts_with("Sword_Draw"),
            "drawing: {}",
            clip(&world)
        );
        tick(&mut combat, &mut world, &mut skins, 0.2);
        assert_eq!(joint(&world), "weapon_r", "drawn at the grab");
        combat.combatant_mut(id).unwrap().lock = None;
        tick(
            &mut combat,
            &mut world,
            &mut skins,
            SHEATHE_AFTER as f32 + 0.3,
        );
        assert!(
            clip(&world).starts_with("Sword_Sheathe"),
            "putting it away: {}",
            clip(&world)
        );
        assert_eq!(joint(&world), "weapon_r", "not released yet");
        tick(&mut combat, &mut world, &mut skins, 0.5);
        assert_eq!(joint(&world), "sheath", "back in the sheath");
    }

    /// The game's work clip plays while he stands idle, holding the clip's
    /// tools (clips.json `gear`); without it his own moves come back and the
    /// tools go.
    #[test]
    fn at_work_he_plays_its_clip_holding_its_tools() {
        let dir = std::env::temp_dir().join(format!("combat_work_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("characters/gear")).unwrap();
        std::fs::write(dir.join("characters/gear/axe.glb"), b"").unwrap();
        let text = r#"{"gear": {
            "Chop_Wood_Loop": [{"piece": "axe", "file": "characters/gear/axe.glb", "nodes": [0, 1], "joint": "weapon_r"}]},
            "clips": {}, "aliases": {}}"#;
        let mut skins = Skins {
            gear: gear_pieces(&dir, text),
            clip_gear: clip_gear(text),
            ..Default::default()
        };
        let (combat, id) = fighter();
        let mut world = World::new();
        let caelan = world.spawn((
            Name::new("Caelan"),
            Transform::default(),
            WorldTransform::identity(),
            SkinnedBody::default(),
        ));
        let mut spawned = Vec::new();
        skins.dress(
            &mut world,
            &BTreeMap::from([(id, caelan)]),
            &combat,
            &mut spawned,
        );
        let body = spawned[0];
        let axes = |world: &World| {
            world
                .iter_with::<Name>()
                .filter(|(_, n)| n.as_str().contains(" axe "))
                .count()
        };
        skins.work = Some((id, "Chop_Wood_Loop|Idle_Loop".into()));
        skins.write(&mut world, &combat, &BTreeMap::new(), 1.0 / 60.0);
        assert_eq!(
            world.get::<SkinnedBody>(body).unwrap().clip,
            "Chop_Wood_Loop|Idle_Loop"
        );
        assert_eq!(axes(&world), 2, "the axe's two nodes in his hand");
        skins.work = None;
        skins.write(&mut world, &combat, &BTreeMap::new(), 1.0 / 60.0);
        assert!(
            world
                .get::<SkinnedBody>(body)
                .unwrap()
                .clip
                .starts_with("Idle_Neutral_Loop")
        );
        assert_eq!(axes(&world), 0, "put down");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// In a boat he plays its clip at its rate holding the clip's oars (two
    /// pieces, both hands); stepping off puts them down.
    #[test]
    fn rowing_he_holds_the_oars() {
        let dir = std::env::temp_dir().join(format!("combat_row_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("characters/gear")).unwrap();
        std::fs::write(dir.join("characters/gear/oar.glb"), b"").unwrap();
        let text = r#"{"gear": {"Row_Loop": [
            {"piece": "oar", "file": "characters/gear/oar.glb", "nodes": [0, 1], "joint": "weapon_r"},
            {"piece": "oar_l", "file": "characters/gear/oar.glb", "nodes": [0, 1], "joint": "weapon_l"}]},
            "clips": {}, "aliases": {}}"#;
        let mut skins = Skins {
            gear: gear_pieces(&dir, text),
            clip_gear: clip_gear(text),
            ..Default::default()
        };
        let (combat, id) = fighter();
        let mut world = World::new();
        let caelan = world.spawn((
            Name::new("Caelan"),
            Transform::default(),
            WorldTransform::identity(),
            SkinnedBody::default(),
        ));
        let mut spawned = Vec::new();
        skins.dress(
            &mut world,
            &BTreeMap::from([(id, caelan)]),
            &combat,
            &mut spawned,
        );
        let body = spawned[0];
        let joints = |world: &World| {
            let mut j: Vec<String> = world
                .iter_with::<SkinnedSocket>()
                .filter(|(e, _)| world.iter_with::<Parent>().any(|(_, p)| p.entity == *e))
                .map(|(_, s)| s.joint.clone())
                .collect();
            j.sort();
            j
        };
        skins.seated = Some((id, ("Row_Loop|Sit_Idle_Loop".into(), 0.0)));
        skins.write(&mut world, &combat, &BTreeMap::new(), 1.0 / 60.0);
        assert_eq!(
            world.get::<SkinnedBody>(body).unwrap().clip,
            "Row_Loop|Sit_Idle_Loop"
        );
        assert_eq!(
            joints(&world),
            ["sheath", "weapon_l", "weapon_r"],
            "an oar in each hand, the blade sheathed"
        );
        skins.seated = None;
        skins.write(&mut world, &combat, &BTreeMap::new(), 1.0 / 60.0);
        assert_eq!(joints(&world), ["sheath"], "the oars shipped");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// clips.json's gear table gives each piece once, only where its file is
    /// there; a weapon kind names its piece and whether it is sheathed.
    /// A fighter whose encounter falls asleep leaves the fight: its body,
    /// sockets and blade go with it, and nothing is written for it again.
    #[test]
    fn a_sleeping_fighter_takes_its_body_away() {
        let (mut combat, id) = fighter();
        let mut world = World::new();
        let caelan = world.spawn((
            Name::new("Caelan"),
            Transform::default(),
            WorldTransform::identity(),
            SkinnedBody::default(),
        ));
        let mut skins = Skins::new(None);
        let mut spawned = Vec::new();
        skins.dress(
            &mut world,
            &BTreeMap::from([(id, caelan)]),
            &combat,
            &mut spawned,
        );
        assert_eq!(skins.count(), 1);
        let parts: Vec<Entity> = world
            .iter_with::<Name>()
            .filter(|(_, n)| n.as_str() != "Caelan")
            .map(|(e, _)| e)
            .collect();
        assert_eq!(parts.len(), 3, "the body, its sheath and the blade");
        combat.despawn(id);
        skins.write(&mut world, &combat, &BTreeMap::new(), 1.0 / 60.0);
        assert_eq!(skins.count(), 0);
        assert!(
            parts.iter().all(|e| world.get::<Name>(*e).is_none()),
            "all despawned"
        );
        assert!(
            world.get::<Name>(caelan).is_some(),
            "the fighter's own entity is its encounter's"
        );
        // At Stop, a body still in the fight goes the same way.
        let again = combat.spawn(CombatantDesc::new(
            "Caelan",
            somnium_combat::TeamId(0),
            Vec3::ZERO,
            somnium_combat::weapon::sword(),
        ));
        world
            .insert_component(caelan, SkinnedBody::default())
            .unwrap();
        skins.dress(
            &mut world,
            &BTreeMap::from([(again, caelan)]),
            &combat,
            &mut spawned,
        );
        assert_eq!(world.iter_with::<Name>().count(), 4);
        skins.end(&mut world);
        assert_eq!(
            world.iter_with::<Name>().count(),
            1,
            "only the fighter is left"
        );
    }

    /// A rider plays the ride loop of its mount's gait at its mount's rate;
    /// the mount's own clips follow the same lines.
    #[test]
    fn a_rider_keeps_its_mounts_gait() {
        let (mut w, id) = fighter();
        let elk = w.spawn(somnium_combat::mount::MountKind::FrostElk.desc(
            "Elk",
            somnium_combat::TeamId(0),
            Vec3::X,
        ));
        w.combatant_mut(id).unwrap().mount = Some(elk);
        w.combatant_mut(elk).unwrap().velocity = Vec3::new(0.0, 0.0, 3.0);
        let steed = w.combatant(elk).unwrap().clone();
        let c = w.combatant(id).unwrap();
        let (clip, t) = pose_of(c, Some(&steed), Some(Mode::Ground), 10.0, false, None);
        assert!(clip.starts_with("Ride_Trot_Loop"), "{clip}");
        let (horse, ht) = pose_of(&steed, None, Some(Mode::Ground), 10.0, false, None);
        assert!(horse.starts_with("Trot_F_Loop"));
        assert_eq!(t, ht, "one rate");
        assert!(
            pose_of(c, None, Some(Mode::Ground), 10.0, false, None)
                .0
                .starts_with("Ride_Idle_Loop"),
            "its own pace is none"
        );
    }

    /// A glTF binary's mesh nodes are read from its JSON chunk.
    #[test]
    fn a_scabbards_meshes_are_its_mesh_nodes() {
        let json =
            br#"{"nodes":[{"name":"root"},{"name":"scabbard","mesh":0},{"name":"iron","mesh":1}]}"#;
        let mut glb = b"glTF".to_vec();
        glb.extend(2u32.to_le_bytes());
        glb.extend(((20 + json.len()) as u32).to_le_bytes());
        glb.extend((json.len() as u32).to_le_bytes());
        glb.extend(b"JSON");
        glb.extend(json);
        let path = std::env::temp_dir().join(format!("scabbard_{}.glb", std::process::id()));
        std::fs::write(&path, &glb).unwrap();
        assert_eq!(mesh_nodes(&path), vec![1, 2]);
        assert_eq!(
            mesh_node_names(&path)[1],
            (2, "iron".to_string()),
            "and their names (a boat's shipped oars)"
        );
        let _ = std::fs::remove_file(path);
    }

    /// The watch: a spearman stands sentry, and on guard (drawn or guarding)
    /// holds the spear's guard; a sword still blocks.
    #[test]
    fn spearmen_stand_sentry_and_guard() {
        let mut w = CombatWorld::new(somnium_combat::CombatRules::default());
        let id = w.spawn(CombatantDesc::new(
            "Watchman",
            somnium_combat::TeamId(0),
            Vec3::ZERO,
            somnium_combat::weapon::spear(),
        ));
        let c = w.combatant_mut(id).unwrap();
        c.velocity = Vec3::ZERO;
        assert!(
            pose_of(c, None, Some(Mode::Ground), 10.0, false, None)
                .0
                .starts_with("Spear_Sentry_Loop")
        );
        assert!(
            pose_of(c, None, Some(Mode::Ground), 10.0, true, None)
                .0
                .starts_with("Spear_Guard_Loop")
        );
        c.action = Action::Guard { t: 0.1 };
        assert_eq!(
            pose_of(c, None, Some(Mode::Ground), 10.0, false, None),
            (
                "Spear_Guard_Loop|Idle_Neutral_Loop|Idle_Loop".into(),
                Timing::Loop(1.0)
            )
        );
        let (mut w, id) = fighter();
        let c = w.combatant_mut(id).unwrap();
        c.action = Action::Guard { t: 0.1 };
        assert!(
            pose_of(c, None, Some(Mode::Ground), 10.0, false, None)
                .0
                .starts_with("Sword_Aether_Rimeguard_Block")
        );
    }

    #[test]
    fn weapons_are_ws3_gear_by_kind() {
        let dir = std::env::temp_dir().join(format!("combat_gear_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("characters/gear")).unwrap();
        std::fs::write(dir.join("characters/gear/axe.glb"), b"").unwrap();
        let text = r#"{"gear": {
            "Axe_Cut_1": [{"piece": "axe", "file": "characters/gear/axe.glb", "nodes": [0, 1], "joint": "weapon_r"}],
            "Axe_Cut_2": [{"piece": "axe", "file": "characters/gear/axe.glb", "nodes": [0, 1], "joint": "weapon_r"}],
            "Bow_Draw_Release": [{"piece": "bow", "file": "characters/gear/bow.glb", "nodes": [0], "joint": "weapon_l"}]},
            "clips": {}, "aliases": {}}"#;
        let gear = gear_pieces(&dir, text);
        assert_eq!(gear.len(), 1, "the bow's file is not there");
        assert_eq!(gear["axe"].nodes, vec![0, 1]);
        assert_eq!(gear["axe"].joint, "weapon_r");
        assert_eq!(piece_for(WeaponKind::Axe), Some(("axe", false)));
        assert_eq!(piece_for(WeaponKind::Sword), Some(("sword", true)));
        assert_eq!(piece_for(WeaponKind::LanceGlaive), Some(("spear", false)));
        assert_eq!(piece_for(WeaponKind::Unarmed), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A move plays its aliased clip warped to its designed timings; a death holds; a dodge back plays Dodge_B.
    #[test]
    fn moves_dodges_and_death() {
        let table = ClipTable::from_json(
            r#"{"aliases": {"sword_cut_1": "Sword_Aether_Combo_1", "wolf_bite": "rime_wolf/Attack_Bite"},
                "clips": {"Sword_Aether_Combo_1": {"seconds": 1.2, "combat_events": {"hit_on": 0.4, "hit_off": 0.5, "recover": 0.9}},
                          "Dodge_B": {"seconds": 0.8}}}"#,
        )
        .unwrap();
        assert_eq!(resolve(Some(&table), "wolf_bite"), "Attack_Bite");
        assert_eq!(resolve(Some(&table), "unknown_move"), "unknown_move");
        let (mut w, id) = fighter();
        let c = w.combatant_mut(id).unwrap();
        let def = somnium_combat::weapon::MoveDef {
            animation: "sword_cut_1".into(),
            windup: 0.2,
            active: 0.1,
            recovery: 0.3,
            ..c.weapon.light[0].clone()
        };
        c.action = Action::Attack(Box::new(somnium_combat::world::AttackState {
            source: somnium_combat::world::MoveSource::Light(0),
            def,
            phase: Phase::Active,
            t: 0.0,
            hit: Vec::new(),
            flags: Default::default(),
            scale: 1.0,
            chain: false,
            dir: Vec3::Z,
            travelled: 0.0,
        }));
        let (clip, t) = pose_of(c, None, Some(Mode::Ground), 1.0, true, Some(&table));
        assert_eq!(clip, "Sword_Aether_Combo_1");
        assert!(
            matches!(t, Timing::At(s) if (s - 0.4).abs() < 1e-4),
            "the blow lands on the clip's hit_on: {t:?}"
        );
        c.facing = Vec3::Z;
        c.action = Action::Dodge(somnium_combat::world::DodgeState {
            t: 0.2,
            duration: 0.4,
            dir: -Vec3::Z,
            distance: 3.0,
            iframes: [0.0, 0.2],
            trail: None,
            from: Vec3::ZERO,
        });
        let (clip, t) = pose_of(c, None, Some(Mode::Ground), 1.0, true, Some(&table));
        assert!(clip.starts_with("Dodge_B"));
        assert!(
            matches!(t, Timing::At(s) if (s - 0.4).abs() < 1e-4),
            "halfway through: {t:?}"
        );
        c.action = Action::Guard { t: 0.1 };
        c.riposte_until = 2.0;
        assert!(
            pose_of(c, None, Some(Mode::Ground), 1.0, true, Some(&table))
                .0
                .starts_with("Sword_Meridian_Parry_Mid")
        );
        c.riposte_until = f64::NEG_INFINITY;
        assert!(
            pose_of(c, None, Some(Mode::Ground), 1.0, true, Some(&table))
                .0
                .starts_with("Sword_Aether_Rimeguard_Block")
        );
        c.action = Action::Dead;
        assert_eq!(
            pose_of(c, None, Some(Mode::Ground), 1.0, true, Some(&table)),
            ("Death_F|Death".into(), Timing::Once)
        );
    }
}
