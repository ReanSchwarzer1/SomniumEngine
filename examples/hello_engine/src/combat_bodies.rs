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
//! A swordsman's blade rides the `weapon_r` socket while fighting and the
//! `sheath` socket otherwise; a rider sits on the mount's `saddle` socket.

use crate::skinned_crowd::{SkinnedBody, SkinnedSocket};
use glam::{Mat4, Quat, Vec3};
use somnium_character::{Controller, MantleKind, Mode};
use somnium_combat::CombatantId;
use somnium_combat::clips::ClipTable;
use somnium_combat::mount::Herding;
use somnium_combat::world::{Action, CastPhase, CombatWorld, Combatant, Phase};
use somnium_core::{EditorFlags, MeshKind, Name, Parent, Transform, WorldTransform};
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

/// One fighter's body.
struct Skin {
    body: Entity,
    /// The blade's socket (weapon_r or sheath).
    sword: Option<Entity>,
    /// A mount's saddle socket, where its rider sits.
    saddle: Option<Entity>,
    clip: String,
    clock: f32,
    /// When it last fought (the blade comes out, and stays out a while).
    fought: f64,
}

/// The fight's skinned bodies.
#[derive(Default)]
pub struct Skins {
    table: Option<ClipTable>,
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
        let table = content
            .map(|c| c.join("animation/clips.json"))
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| ClipTable::from_json(&t).ok());
        Self {
            table,
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
            let sword = c.sword.is_some().then(|| {
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
                s
            });
            let saddle = c.mount_def.is_some().then(|| socket(world, "saddle"));
            self.skins.insert(
                id,
                Skin {
                    body,
                    sword,
                    saddle,
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
            if let Some(s) = skin.sword {
                let joint = if drawn { "weapon_r" } else { "sheath" };
                if let Some(sock) = world.get_mut::<SkinnedSocket>(s)
                    && sock.joint != joint
                {
                    sock.joint = joint.into();
                }
            }
        }
    }

    /// Give the fighters their bodies and shapes back (Stop).
    pub fn end(self, world: &mut World) {
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
