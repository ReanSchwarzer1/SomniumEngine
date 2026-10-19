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
//! `sheath` socket when not fighting. A rider sits on the mount's `saddle`.

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

/// What a fighter's state calls for: a clip (alternatives `A|B`) and how it runs.
#[must_use]
pub fn pose_of(
    c: &Combatant,
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
        let clip = match speed {
            s if s < 0.3 => "Ride_Idle_Loop|Sit_Idle_Loop",
            s if s < 2.5 => "Ride_Walk_Loop|Ride_Idle_Loop|Sit_Idle_Loop",
            s if s < 5.5 => "Ride_Trot_Loop|Ride_Idle_Loop|Sit_Idle_Loop",
            s if s < 8.0 => "Ride_Canter_Loop|Ride_Gallop_Loop|Sit_Idle_Loop",
            _ => "Ride_Gallop_Loop|Ride_Canter_Loop|Sit_Idle_Loop",
        };
        return (clip.into(), Timing::Loop(1.0));
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
        let (clip, rate) = match speed {
            s if s < 0.3 && grazing => ("Graze_Loop|Idle_Loop", 1.0),
            s if s < 0.3 => ("Idle_Loop", 1.0),
            s if s < 2.5 => ("Walk_F_Loop", s / 1.6),
            s if s < 5.5 => ("Trot_F_Loop|Walk_F_Loop", s / 3.5),
            s if s < 8.0 => ("Canter_F_Loop|Gallop_F_Loop|Trot_F_Loop", s / 6.0),
            s => ("Gallop_F_Loop|Canter_F_Loop|Trot_F_Loop", s / 9.0),
        };
        return (clip.into(), Timing::Loop(rate.clamp(0.6, 1.6)));
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
            format!("{clip}|Loco_Jog_F_Loop|Trot_F_Loop"),
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
    /// A rider's pelvis socket (made when it first mounts), the point that sits on the saddle.
    pelvis: Option<Entity>,
    clip: String,
    clock: f32,
    /// When it last fought (the blade comes out, and stays out a while).
    fought: f64,
}

impl Skin {
    /// The body and everything placed on it (despawn does not reach children).
    fn despawn(self, world: &mut World) {
        let mut parts = vec![self.body];
        parts.extend(self.saddle);
        parts.extend(self.pelvis);
        if let Some((socket, _, _)) = self.weapon {
            parts.extend(
                world
                    .iter_with::<Parent>()
                    .filter(|(_, p)| p.entity == socket)
                    .map(|(e, _)| e)
                    .collect::<Vec<_>>(),
            );
            parts.push(socket);
        }
        for e in parts {
            world.despawn(e);
        }
    }
}

/// Where a body's pelvis is in the body's own frame, as last posed (its socket
/// is placed with the pose); nothing until the socket has been placed.
fn pelvis_offset(world: &World, body: Entity, pelvis: Entity) -> Vec3 {
    let (Some(b), Some(p)) = (
        world.get::<WorldTransform>(body),
        world.get::<WorldTransform>(pelvis),
    ) else {
        return Vec3::ZERO;
    };
    if p.0 == Mat4::IDENTITY {
        return Vec3::ZERO;
    }
    let offset = b.0.inverse().transform_point3(p.0.w_axis.truncate());
    if offset.length() < 2.0 {
        offset
    } else {
        Vec3::ZERO
    }
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
    hidden: Vec<(Entity, Option<EditorFlags>)>,
}

impl Skins {
    /// With the project's clip table (`<content>/animation/clips.json`), if it has one.
    #[must_use]
    pub fn new(content: Option<&std::path::Path>) -> Self {
        let text =
            content.and_then(|c| std::fs::read_to_string(c.join("animation/clips.json")).ok());
        let table = text.as_deref().and_then(|t| ClipTable::from_json(t).ok());
        let gear = match (content, text.as_deref()) {
            (Some(content), Some(text)) => gear_pieces(content, text),
            _ => BTreeMap::new(),
        };
        Self {
            table,
            gear,
            ..Default::default()
        }
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
                        for &node in &g.nodes {
                            world.spawn((
                                Name::new(&format!("{name} {piece} {node}")),
                                Transform::default(),
                                WorldTransform::identity(),
                                Parent { entity: s },
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
            let saddle = c.mount_def.is_some().then(|| socket(world, "saddle"));
            self.skins.insert(
                id,
                Skin {
                    body,
                    weapon,
                    saddle,
                    pelvis: None,
                    clip: String::new(),
                    clock: 0.0,
                    fought: f64::NEG_INFINITY,
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
            let (clip, timing) = pose_of(c, mode, now, drawn, self.table.as_ref());
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
                Some(s) => {
                    // Its pelvis on the saddle (the body's origin is its feet).
                    let body = skin.body;
                    let pelvis = *skin.pelvis.get_or_insert_with(|| {
                        world.spawn((
                            Name::new("pelvis"),
                            Transform::default(),
                            WorldTransform::identity(),
                            Parent { entity: body },
                            SkinnedSocket {
                                joint: "pelvis".into(),
                            },
                        ))
                    });
                    let (_, r, t) = s.to_scale_rotation_translation();
                    (t - r * pelvis_offset(world, body, pelvis), r)
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
            if let Some((s, held, sheathes)) = &skin.weapon {
                let joint = if drawn || !sheathes {
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
        let (clip, t) = pose_of(c, Some(Mode::Ground), 10.0, false, None);
        assert!(clip.starts_with("Idle_Neutral_Loop"));
        assert_eq!(t, Timing::Loop(1.0));
        let (clip, _) = pose_of(c, Some(Mode::Ground), 10.0, true, None);
        assert!(
            clip.starts_with("Sword_Aether_Idle_Loop"),
            "on guard with the blade out"
        );
        c.velocity = Vec3::new(0.0, 0.0, 1.0);
        let (clip, t) = pose_of(c, Some(Mode::Ground), 10.0, false, None);
        assert!(clip.starts_with("Loco_Walk_F_Loop"));
        assert!(matches!(t, Timing::Loop(r) if (r - 1.0 / 0.86).abs() < 1e-4));
        c.velocity = Vec3::new(0.0, 0.0, 5.6);
        assert!(
            pose_of(c, Some(Mode::Ground), 10.0, false, None)
                .0
                .starts_with("Loco_Jog_F_Loop")
        );
        c.velocity = Vec3::new(0.0, 0.0, -1.0);
        assert!(
            pose_of(c, Some(Mode::Ground), 10.0, false, None)
                .0
                .starts_with("Loco_Walk_B_Loop")
        );
        c.velocity = Vec3::new(-1.0, 0.0, 0.0);
        assert!(
            pose_of(c, Some(Mode::Ground), 10.0, false, None)
                .0
                .starts_with("Loco_Walk_R_Loop"),
            "facing +Z, -X is the right"
        );
        c.velocity = Vec3::new(0.0, 0.0, 1.0);
        assert!(
            pose_of(c, Some(Mode::Swim), 10.0, false, None)
                .0
                .starts_with("Swim_F_Loop")
        );
        assert!(
            pose_of(c, Some(Mode::Air), 10.0, false, None)
                .0
                .starts_with("Jump_Loop")
        );
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

    /// A rider's pelvis offset is read from its socket as posed, in the
    /// body's own frame; an unplaced socket gives none.
    #[test]
    fn a_rider_sits_by_its_pelvis() {
        let mut world = World::new();
        let turn = Quat::from_rotation_y(1.0);
        let body = world.spawn((WorldTransform(Mat4::from_rotation_translation(
            turn,
            Vec3::new(5.0, 0.0, 2.0),
        )),));
        let pelvis = world.spawn((WorldTransform::identity(),));
        assert_eq!(
            pelvis_offset(&world, body, pelvis),
            Vec3::ZERO,
            "not placed yet"
        );
        let posed = Vec3::new(5.0, 0.0, 2.0) + turn * Vec3::new(0.0, 0.92, -0.1);
        world.get_mut::<WorldTransform>(pelvis).unwrap().0 = Mat4::from_translation(posed);
        assert!(pelvis_offset(&world, body, pelvis).distance(Vec3::new(0.0, 0.92, -0.1)) < 1e-4);
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
            pose_of(c, Some(Mode::Ground), 10.0, false, None)
                .0
                .starts_with("Spear_Sentry_Loop")
        );
        assert!(
            pose_of(c, Some(Mode::Ground), 10.0, true, None)
                .0
                .starts_with("Spear_Guard_Loop")
        );
        c.action = Action::Guard { t: 0.1 };
        assert_eq!(
            pose_of(c, Some(Mode::Ground), 10.0, false, None),
            (
                "Spear_Guard_Loop|Idle_Neutral_Loop|Idle_Loop".into(),
                Timing::Loop(1.0)
            )
        );
        let (mut w, id) = fighter();
        let c = w.combatant_mut(id).unwrap();
        c.action = Action::Guard { t: 0.1 };
        assert!(
            pose_of(c, Some(Mode::Ground), 10.0, false, None)
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
        let (clip, t) = pose_of(c, Some(Mode::Ground), 1.0, true, Some(&table));
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
        let (clip, t) = pose_of(c, Some(Mode::Ground), 1.0, true, Some(&table));
        assert!(clip.starts_with("Dodge_B"));
        assert!(
            matches!(t, Timing::At(s) if (s - 0.4).abs() < 1e-4),
            "halfway through: {t:?}"
        );
        c.action = Action::Guard { t: 0.1 };
        c.riposte_until = 2.0;
        assert!(
            pose_of(c, Some(Mode::Ground), 1.0, true, Some(&table))
                .0
                .starts_with("Sword_Meridian_Parry_Mid")
        );
        c.riposte_until = f64::NEG_INFINITY;
        assert!(
            pose_of(c, Some(Mode::Ground), 1.0, true, Some(&table))
                .0
                .starts_with("Sword_Aether_Rimeguard_Block")
        );
        c.action = Action::Dead;
        assert_eq!(
            pose_of(c, Some(Mode::Ground), 1.0, true, Some(&table)),
            ("Death_F|Death".into(), Timing::Once)
        );
    }
}
