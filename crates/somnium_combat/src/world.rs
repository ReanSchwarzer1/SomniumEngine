//! The combat simulation: one fixed step at a time, in a fixed order.
//!
//! ```text
//! step(dt)
//!  1. focus (slow time) drains; world and focuser time scales
//!  2. the ground: environment tick, standing in water / fire / on ice
//!  3. bodies: statuses, burn and bleed, regeneration, wards, kindling
//!  4. brains think (AI writes commands)
//!  5. commands become actions (attack, guard, dodge, aim, cast, toggles)
//!  6. actions advance (phases, shots, spell releases, charges)
//!  7. melee volumes test hurt volumes
//!  8. projectiles fly
//!  9. walls, waves and blooms act
//! 10. queued hits resolve in order (dodge, ward, parry, block, harm, poise, death)
//! 11. movement intents (and, if the host asks, integration)
//! ```
//!
//! The host calls [`CombatWorld::set_pose`] before a step with where its
//! character controller put each body, [`CombatWorld::command`] with the
//! player's input, and reads [`CombatWorld::intent`] and
//! [`CombatWorld::drain_events`] after.
mod cast;
mod effects;
mod melee;
mod projectile;
mod resolve;

pub use cast::{CastPhase, CastState, Shaping};
pub use effects::{Bloom, Wall, Ward, Wave};
pub use projectile::Projectile;

use crate::damage::{Hit, HitFlags};
use crate::environment::{EnvironmentField, Surface, SurfaceChange};
use crate::geometry::{Capsule, Obb, flat, planar_dir};
use crate::schools::Swordsmanship;
use crate::spell::{CastSpeed, Element, Magic, Shape, SpellAsset, SpellError, SpellRequest};
use crate::status::{StatusChange, StatusKind, StatusSet};
use crate::weapon::{ProjectileKind, WeaponDef};
use crate::{
    CombatQuery, CombatantId, DamageType, Relations, Resistances, Rng, TeamId, Vitals, WorldHit, ai,
};
use glam::Vec3;
use std::collections::{BTreeMap, HashMap};

/// A body's size, for hurt volumes and movement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    /// Capsule radius, metres.
    pub radius: f32,
    /// Standing height, metres.
    pub height: f32,
    /// Mass in kilograms; knockback divides by it relative to 80.
    pub mass: f32,
    /// Walking speed, metres per second.
    pub speed: f32,
}

impl Default for Body {
    fn default() -> Self {
        Self {
            radius: 0.35,
            height: 1.8,
            mass: 80.0,
            speed: 3.6,
        }
    }
}

/// A breakable or weak part of a large body (a harrowcat's tail, a giant's
/// knee), in the body's own frame: x right, y up, z forward.
#[derive(Clone, Debug, PartialEq)]
pub struct HurtPart {
    /// Name, for events.
    pub name: String,
    /// One end of the part's core segment.
    pub a: Vec3,
    /// The other end.
    pub b: Vec3,
    /// Radius.
    pub radius: f32,
    /// Damage multiplier for strikes here.
    pub damage_scale: f32,
    /// Hit points before the part breaks; `None` never breaks.
    pub health: Option<f32>,
}

/// One digital control across a step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Press {
    /// Went down since the last step.
    pub pressed: bool,
    /// Is down.
    pub held: bool,
    /// Came up since the last step.
    pub released: bool,
}

impl Press {
    /// A press edge that is also held.
    #[must_use]
    pub fn tap() -> Self {
        Self {
            pressed: true,
            held: true,
            released: false,
        }
    }

    /// Down and up within one frame: a click.
    #[must_use]
    pub fn click() -> Self {
        Self {
            pressed: true,
            held: false,
            released: true,
        }
    }

    /// Held, no edge.
    #[must_use]
    pub fn hold() -> Self {
        Self {
            pressed: false,
            held: true,
            released: false,
        }
    }

    /// A release edge.
    #[must_use]
    pub fn release() -> Self {
        Self {
            pressed: false,
            held: false,
            released: true,
        }
    }
}

/// What a combatant wants this step: the player's input or an AI's choice.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Command {
    /// Planar movement, length at most one.
    pub move_dir: Vec3,
    /// Run.
    pub sprint: bool,
    /// The point being aimed at (the cursor on the ground, or a target).
    pub aim: Option<Vec3>,
    /// Attack button.
    pub attack: Press,
    /// Guard / parry button.
    pub guard: Press,
    /// Dodge pressed.
    pub dodge: bool,
    /// School art (or throw) pressed.
    pub art: bool,
    /// Toggle the Kindled Body.
    pub kindle: bool,
    /// Toggle lock-on.
    pub lock: bool,
    /// Toggle slow-time focus.
    pub focus: bool,
    /// Climb into the nearest saddle, or out of this one.
    pub mount: bool,
    /// Spell keys 1 to 0.
    pub spells: [Press; 10],
    /// Mouse wheel notches while shaping.
    pub measure_steps: f32,
    /// Cycle which measure the wheel edits.
    pub measure_cycle: bool,
    /// A finished request to cast at once (AI, scripts, quick-cast).
    pub cast: Option<SpellRequest>,
    /// Hold the current channel (AI; the player holds the spell key).
    pub channel: bool,
    /// Vault or climb what the body runs into (the AI on an off-mesh link).
    pub climb: bool,
    /// Whistle for a mount ([`crate::mount::whistle`]).
    pub whistle: bool,
}

impl Command {
    fn consume_edges(&mut self) {
        for p in std::iter::once(&mut self.attack)
            .chain(std::iter::once(&mut self.guard))
            .chain(self.spells.iter_mut())
        {
            p.pressed = false;
            p.released = false;
        }
        self.dodge = false;
        self.art = false;
        self.kindle = false;
        self.lock = false;
        self.focus = false;
        self.mount = false;
        self.whistle = false;
        self.measure_steps = 0.0;
        self.measure_cycle = false;
        self.cast = None;
    }
}

/// Which move an attack is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveSource {
    /// A light move by chain index.
    Light(u8),
    /// The heavy.
    Heavy,
    /// The running charge.
    Charge,
}

/// The three phases of a move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Winding up: the telegraph.
    Windup,
    /// The blow is live.
    Active,
    /// Recovering.
    Recovery,
}

/// A move in progress.
#[derive(Clone, Debug, PartialEq)]
pub struct AttackState {
    /// Which move.
    pub source: MoveSource,
    /// The move, resolved (school and rank applied).
    pub def: crate::weapon::MoveDef,
    /// Phase.
    pub phase: Phase,
    /// Seconds in this phase.
    pub t: f32,
    /// Who this activation has already hit.
    pub hit: Vec<CombatantId>,
    /// Flags every hit of this move carries.
    pub flags: HitFlags,
    /// Damage multiplier (riposte, rank, kindling).
    pub scale: f32,
    /// The next chain move was asked for.
    pub chain: bool,
    /// Planar direction of the blow.
    pub dir: Vec3,
    /// Metres travelled (charges).
    pub travelled: f32,
}

/// A dodge in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DodgeState {
    /// Seconds in.
    pub t: f32,
    /// Seconds long.
    pub duration: f32,
    /// Planar direction.
    pub dir: Vec3,
    /// Metres covered over the dodge.
    pub distance: f32,
    /// Invulnerable from and to, seconds.
    pub iframes: [f32; 2],
    /// Element left along the path (Frost Step's ice), if any.
    pub trail: Option<Element>,
    /// Where it started (for the trail).
    pub from: Vec3,
}

/// What a combatant is doing.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Free.
    Idle,
    /// Swinging.
    Attack(Box<AttackState>),
    /// Guard raised for `t` seconds.
    Guard {
        /// Seconds since raised.
        t: f32,
    },
    /// Dodging.
    Dodge(DodgeState),
    /// Drawing a bow or steadying a rifle.
    Aim {
        /// Seconds drawn.
        t: f32,
    },
    /// Reloading.
    Reload {
        /// Seconds left.
        left: f32,
    },
    /// Casting.
    Cast(Box<CastState>),
    /// Busy for `left` seconds (after a shot, after an art).
    Recover {
        /// Seconds left.
        left: f32,
    },
    /// Dead.
    Dead,
}

impl Action {
    /// A short name for debugging and Details.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Attack(a) => match a.phase {
                Phase::Windup => "Winding up",
                Phase::Active => "Striking",
                Phase::Recovery => "Recovering",
            },
            Self::Guard { .. } => "Guarding",
            Self::Dodge(_) => "Dodging",
            Self::Aim { .. } => "Aiming",
            Self::Reload { .. } => "Reloading",
            Self::Cast(c) => match c.phase {
                CastPhase::Gather => "Casting",
                CastPhase::Channel => "Channelling",
                CastPhase::Recover => "Releasing",
            },
            Self::Recover { .. } => "Recovering",
            Self::Dead => "Dead",
        }
    }
}

/// How the host should move a body this step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveIntent {
    /// Desired velocity, metres per second (planar plus any lunge or push).
    pub velocity: Vec3,
    /// Planar facing.
    pub facing: Vec3,
    /// Running.
    pub sprinting: bool,
    /// Friction multiplier from the ground (ice).
    pub traction: f32,
    /// Playback speed for this body's animation (focus, chill).
    pub time_scale: f32,
    /// The body should not be moved by the controller (frozen, down, dead).
    pub rooted: bool,
    /// Vault or climb a ledge the body runs into ([`Command::climb`]).
    pub climb: bool,
}

impl Default for MoveIntent {
    fn default() -> Self {
        Self {
            velocity: Vec3::ZERO,
            facing: Vec3::Z,
            sprinting: false,
            traction: 1.0,
            time_scale: 1.0,
            rooted: false,
            climb: false,
        }
    }
}

/// How a death becomes a ragdoll: the host ramps the existing ragdoll blend
/// (`somnium_anim::blend_ragdoll`, `somnium_core::animation_motion::blend_jolt_ragdoll`)
/// by [`Self::weight`] and applies [`Self::impulse`] once at [`Self::point`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeathRagdoll {
    /// Seconds since death.
    pub t: f32,
    /// Seconds to blend fully into physics.
    pub blend_in: f32,
    /// Impulse of the killing blow, kilogram metres per second.
    pub impulse: Vec3,
    /// Where it struck.
    pub point: Vec3,
}

impl DeathRagdoll {
    /// Blend weight now, 0 to 1, smoothstepped.
    #[must_use]
    pub fn weight(&self) -> f32 {
        let x = if self.blend_in > 0.0 {
            (self.t / self.blend_in).clamp(0.0, 1.0)
        } else {
            1.0
        };
        x * x * (3.0 - 2.0 * x)
    }
}

/// Everything the world needs to create a combatant.
#[derive(Clone, Debug)]
pub struct CombatantDesc {
    /// Display name.
    pub name: String,
    /// Team.
    pub team: TeamId,
    /// Feet position.
    pub position: Vec3,
    /// Planar facing.
    pub facing: Vec3,
    /// Size and speed.
    pub body: Body,
    /// Pools.
    pub vitals: Vitals,
    /// Resistances and armour.
    pub resist: Resistances,
    /// Status resistances.
    pub status_resist: [f32; 7],
    /// Weapon.
    pub weapon: WeaponDef,
    /// Sword school, if any.
    pub swordsmanship: Option<Swordsmanship>,
    /// Magic.
    pub magic: Magic,
    /// AI, if not player driven.
    pub brain: Option<ai::BrainDesc>,
    /// The host's handle (an ECS entity's bits).
    pub host: u64,
    /// Weak or breakable parts (large bodies).
    pub parts: Vec<HurtPart>,
    /// A Hollowborn (Night Fang's Shadow-step).
    pub hollowborn: bool,
    /// This body is a mount that can be ridden.
    pub mount: Option<crate::mount::MountDef>,
}

impl CombatantDesc {
    /// A fighter with a weapon and ordinary pools.
    #[must_use]
    pub fn new(name: impl Into<String>, team: TeamId, position: Vec3, weapon: WeaponDef) -> Self {
        Self {
            name: name.into(),
            team,
            position,
            facing: Vec3::Z,
            body: Body::default(),
            vitals: Vitals::new(100.0, 100.0, 5.0, 40.0),
            resist: Resistances::default(),
            status_resist: [0.0; 7],
            weapon,
            swordsmanship: None,
            magic: Magic::default(),
            brain: None,
            host: 0,
            parts: Vec::new(),
            hollowborn: false,
            mount: None,
        }
    }
}

/// A combatant's runtime state.
#[derive(Clone, Debug)]
pub struct Combatant {
    /// Identity.
    pub id: CombatantId,
    /// Display name.
    pub name: String,
    /// Team.
    pub team: TeamId,
    /// The host's handle.
    pub host: u64,
    /// Feet position (from the host).
    pub position: Vec3,
    /// Planar facing.
    pub facing: Vec3,
    /// Observed velocity (from successive poses).
    pub velocity: Vec3,
    /// Size and speed.
    pub body: Body,
    /// Pools.
    pub vitals: Vitals,
    /// Resistances.
    pub resist: Resistances,
    /// Statuses.
    pub status: StatusSet,
    /// Weapon.
    pub weapon: WeaponDef,
    /// School.
    pub sword: Option<Swordsmanship>,
    /// Magic.
    pub magic: Magic,
    /// What it is doing.
    pub action: Action,
    /// Lock-on target.
    pub lock: Option<CombatantId>,
    /// Kindled Body on.
    pub kindled: bool,
    /// Shield, if warded.
    pub ward: Option<Ward>,
    /// This step's command.
    pub command: Command,
    /// Output intent.
    pub intent: MoveIntent,
    /// AI.
    pub brain: Option<ai::Brain>,
    /// Spell being shaped on a held key.
    pub shaping: Option<Shaping>,
    /// Host-reported blade (hilt, tip) now and last step.
    pub blade: Option<[Vec3; 2]>,
    prev_blade: Option<[Vec3; 2]>,
    /// Rounds or knives left before a reload.
    pub ammo: u8,
    /// Time until which the next attack is a riposte.
    pub riposte_until: f64,
    /// Time until which watchers see this combatant at reduced range.
    pub concealed_until: f64,
    /// Spell cooldowns: asset id to ready time.
    pub cooldowns: BTreeMap<String, f64>,
    /// Knockback slide velocity, decaying.
    pub knock: Vec3,
    /// Seconds the attack button has been held.
    pub attack_held: f32,
    /// Moves started, for telling one swing from the next.
    pub swings: u32,
    /// Death presentation, once dead.
    pub death: Option<DeathRagdoll>,
    /// Breakable parts.
    pub parts: Vec<(HurtPart, bool)>,
    /// A Hollowborn.
    pub hollowborn: bool,
    /// Time of the last hit taken, for AI threat.
    pub last_hurt: f64,
    /// Who last hurt it.
    pub last_attacker: Option<CombatantId>,
    /// Mounted on, if riding.
    pub mount: Option<CombatantId>,
    /// Ridden by, if a mount.
    pub rider: Option<CombatantId>,
    /// Mount stats, if this is a mount.
    pub mount_def: Option<crate::mount::MountDef>,
    /// A mount's own mind while no one rides it.
    pub herd: Option<crate::mount::Herd>,
}

impl Combatant {
    /// Hurt volume (body capsule).
    #[must_use]
    pub fn capsule(&self) -> Capsule {
        Capsule::upright(self.position, self.body.height, self.body.radius)
    }

    /// Chest height point.
    #[must_use]
    pub fn chest(&self) -> Vec3 {
        self.position + Vec3::Y * self.body.height * 0.62
    }

    /// Eye point.
    #[must_use]
    pub fn eye(&self) -> Vec3 {
        self.position + Vec3::Y * self.body.height * 0.92
    }

    /// Whether alive.
    #[must_use]
    pub fn alive(&self) -> bool {
        !matches!(self.action, Action::Dead)
    }

    /// Whether the guard is up, and for how long.
    #[must_use]
    pub fn guard_time(&self) -> Option<f32> {
        match self.action {
            Action::Guard { t } => Some(t),
            _ => None,
        }
    }

    /// Whether invulnerable now (dodge frames).
    #[must_use]
    pub fn invulnerable(&self) -> bool {
        matches!(self.action, Action::Dodge(d) if d.t >= d.iframes[0] && d.t <= d.iframes[1])
    }

    /// Kindled Body strength: grows with capacity, with diminishing returns
    /// (MAGIC 5.1: its limit is capacity).
    #[must_use]
    pub fn kindle_strength(&self) -> f32 {
        if self.kindled {
            1.0 + 0.18 * (1.0 + self.vitals.mana.max.max(0.0) / 50.0).ln()
        } else {
            1.0
        }
    }

    /// Lamps per second the Kindled Body burns.
    #[must_use]
    pub fn kindle_drain(&self) -> f32 {
        0.4 + self.vitals.mana.max.max(0.0) * 0.0015
    }

    fn knows(&self, form: crate::schools::Form) -> bool {
        self.sword.is_some_and(|s| s.knows(form))
    }
}

/// World-wide rules.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatRules {
    /// Area spells (waves, blooms, walls, burning ground) harm allies too.
    pub friendly_fire_area: bool,
    /// Swings and bolts harm allies they touch.
    pub friendly_fire_direct: bool,
    /// The world moves bodies itself (sandboxes, tests). Off when the host
    /// has a character controller and reads intents.
    pub integrate_motion: bool,
    /// Damage multiplier for a riposte.
    pub riposte: f32,
    /// Seconds after a parry in which the next attack is a riposte.
    pub riposte_window: f32,
    /// How many may swing at one target at once.
    pub max_melee_attackers: usize,
    /// World time scale while someone focuses.
    pub focus_world_scale: f32,
    /// The focuser's own time scale while focusing.
    pub focus_self_scale: f32,
    /// Focus spent per real second while focusing.
    pub focus_drain: f32,
    /// Seed for the world's random stream.
    pub seed: u64,
}

impl Default for CombatRules {
    fn default() -> Self {
        Self {
            friendly_fire_area: true,
            friendly_fire_direct: false,
            integrate_motion: false,
            riposte: 2.5,
            riposte_window: 1.2,
            max_melee_attackers: 2,
            focus_world_scale: 0.35,
            focus_self_scale: 0.7,
            focus_drain: 12.0,
            seed: 0x5EED_C0DE,
        }
    }
}

/// Something that happened, for the host's animation, sound, VFX, decals,
/// physics and AI bookkeeping.
#[derive(Clone, Debug, PartialEq)]
pub enum CombatEvent {
    /// A move began: play `animation`.
    AttackStarted {
        /// Who.
        attacker: CombatantId,
        /// Move name.
        name: String,
        /// Clip.
        animation: String,
        /// A riposte.
        riposte: bool,
    },
    /// Harm landed.
    Damaged {
        /// Who was hurt.
        target: CombatantId,
        /// By whom.
        source: Option<CombatantId>,
        /// After mitigation.
        amount: f32,
        /// The largest kind.
        kind: DamageType,
        /// Where.
        point: Vec3,
        /// A critical.
        critical: bool,
        /// How it was delivered.
        flags: HitFlags,
    },
    /// Health restored.
    Healed {
        /// Who.
        target: CombatantId,
        /// By whom.
        source: Option<CombatantId>,
        /// How much.
        amount: f32,
    },
    /// A blow met a raised guard.
    Blocked {
        /// Defender.
        target: CombatantId,
        /// Attacker.
        source: Option<CombatantId>,
        /// Where.
        point: Vec3,
    },
    /// A guard ran out of stamina.
    GuardBroken {
        /// Defender.
        target: CombatantId,
    },
    /// A blow was turned aside in the parry window.
    Parried {
        /// Who parried.
        defender: CombatantId,
        /// Whose blow.
        attacker: CombatantId,
        /// Where.
        point: Vec3,
        /// At the instant of the strike (Meridian's Noon).
        perfect: bool,
    },
    /// A projectile was turned back (Meridian's Mirror).
    Reflected {
        /// Who turned it.
        defender: CombatantId,
        /// The projectile.
        projectile: u32,
    },
    /// A hit passed through dodge frames.
    Dodged {
        /// Who dodged.
        target: CombatantId,
        /// Whose.
        source: Option<CombatantId>,
    },
    /// A dodge began: play the roll.
    DodgeStarted {
        /// Who.
        who: CombatantId,
        /// Planar direction.
        dir: Vec3,
    },
    /// A windup was cancelled as a feint (Night Fang).
    Feinted {
        /// Who.
        attacker: CombatantId,
    },
    /// Poise broke.
    PoiseBroken {
        /// Who.
        target: CombatantId,
        /// Knocked down rather than staggered.
        knockdown: bool,
    },
    /// A status began.
    StatusApplied {
        /// Who.
        target: CombatantId,
        /// What.
        kind: StatusKind,
    },
    /// A status ended.
    StatusEnded {
        /// Who.
        target: CombatantId,
        /// What.
        kind: StatusKind,
    },
    /// A frozen body was struck apart.
    Shattered {
        /// Who.
        target: CombatantId,
    },
    /// Fire on a body went out.
    Extinguished {
        /// Who.
        target: CombatantId,
    },
    /// Blood burst.
    BleedBurst {
        /// Who.
        target: CombatantId,
        /// How much.
        amount: f32,
    },
    /// Died: start the ragdoll.
    Died {
        /// Who.
        target: CombatantId,
        /// Killer.
        killer: Option<CombatantId>,
        /// Impulse of the killing blow.
        impulse: Vec3,
        /// Where it struck.
        point: Vec3,
    },
    /// A shot or throw or bolt left.
    ShotFired {
        /// Who.
        shooter: CombatantId,
        /// Projectile id.
        projectile: u32,
        /// Kind.
        kind: ProjectileKind,
        /// From.
        from: Vec3,
        /// Velocity.
        velocity: Vec3,
    },
    /// A projectile struck something or ran out.
    ProjectileEnded {
        /// Projectile id.
        projectile: u32,
        /// Where.
        point: Vec3,
        /// Surface normal (zero when it simply ran out).
        normal: Vec3,
        /// Body struck, if any.
        target: Option<CombatantId>,
        /// Kind.
        kind: ProjectileKind,
        /// Spell element, if a spell.
        element: Option<Element>,
    },
    /// A spell began gathering.
    CastBegan {
        /// Who.
        caster: CombatantId,
        /// Asset.
        spell: String,
        /// Release.
        release: CastSpeed,
        /// Seconds of gathering.
        gather: f32,
    },
    /// A spell was released.
    SpellReleased {
        /// Who.
        caster: CombatantId,
        /// Asset.
        spell: String,
        /// Element.
        element: Element,
        /// Shape.
        shape: Shape,
        /// Where it was aimed.
        at: Vec3,
    },
    /// Gathering was broken.
    SpellInterrupted {
        /// Who.
        caster: CombatantId,
        /// Asset.
        spell: String,
    },
    /// A request was refused.
    SpellRefused {
        /// Who.
        caster: CombatantId,
        /// Asset.
        spell: String,
        /// Why.
        error: SpellError,
    },
    /// Cast on the Wheel's credit.
    Overdrawn {
        /// Who.
        caster: CombatantId,
        /// How far below zero.
        debt: f32,
    },
    /// A spell wall stands.
    WallRaised {
        /// Wall id.
        wall: u32,
        /// Element.
        element: Element,
        /// Its box.
        obb: Obb,
        /// Whether bodies are stopped by it (host: add a collider and a nav obstacle).
        blocks_movement: bool,
    },
    /// A spell wall fell or faded.
    WallFell {
        /// Wall id.
        wall: u32,
    },
    /// A ward rose on a body.
    WardRaised {
        /// Who.
        target: CombatantId,
        /// Element.
        element: Element,
        /// Absorption.
        strength: f32,
    },
    /// A ward broke or faded.
    WardBroken {
        /// Who.
        target: CombatantId,
    },
    /// A bloom burst.
    Bloomed {
        /// Centre.
        at: Vec3,
        /// Radius.
        radius: f32,
        /// Element.
        element: Element,
    },
    /// A bloom will burst here (telegraph).
    BloomGathering {
        /// Centre.
        at: Vec3,
        /// Radius.
        radius: f32,
        /// Element.
        element: Element,
        /// Seconds until it bursts.
        delay: f32,
    },
    /// A wave set out.
    WaveSent {
        /// From.
        origin: Vec3,
        /// Planar direction.
        dir: Vec3,
        /// Element.
        element: Element,
        /// Half angle, radians.
        half_angle: f32,
        /// Reach.
        range: f32,
    },
    /// A spell moved its caster.
    Stepped {
        /// Who.
        caster: CombatantId,
        /// From.
        from: Vec3,
        /// To.
        to: Vec3,
        /// Element.
        element: Element,
    },
    /// The ground changed.
    Surface(SurfaceChange),
    /// Something made a noise others may hear.
    Noise {
        /// Who.
        source: CombatantId,
        /// Where.
        position: Vec3,
        /// 0 to 1.
        loudness: f32,
    },
    /// An AI raised the alarm.
    Alerted {
        /// Who shouted.
        by: CombatantId,
        /// About whom.
        target: CombatantId,
        /// Last seen at.
        at: Vec3,
    },
    /// An AI broke and ran.
    Fled {
        /// Who.
        who: CombatantId,
    },
    /// Slow time began or ended.
    Focus {
        /// Who focuses.
        who: CombatantId,
        /// On or off.
        active: bool,
    },
    /// The Kindled Body went on or off.
    Kindled {
        /// Who.
        who: CombatantId,
        /// On or off.
        on: bool,
    },
    /// Lock-on changed.
    LockChanged {
        /// Who.
        who: CombatantId,
        /// New target.
        target: Option<CombatantId>,
    },
    /// A part broke.
    PartBroken {
        /// Whose.
        target: CombatantId,
        /// Which.
        part: String,
    },
    /// Darkness gathered (Night Fang's Lantern-out).
    Concealed {
        /// Who.
        who: CombatantId,
        /// For how long.
        seconds: f32,
    },
    /// A rider mounted or dismounted.
    Mounted {
        /// Rider.
        rider: CombatantId,
        /// Mount, or none when dismounting.
        mount: Option<CombatantId>,
    },
    /// A brain's awareness of an enemy changed (for the host's "?" and "!").
    Noticed {
        /// The brain.
        watcher: CombatantId,
        /// Of whom.
        target: CombatantId,
        /// Now.
        level: ai::Awareness,
        /// How, if it was noticed this time rather than forgotten.
        clue: Option<ai::Clue>,
    },
    /// Someone whistled for a mount.
    Whistled {
        /// Who.
        who: CombatantId,
        /// The mount that answers, if one heard.
        mount: Option<CombatantId>,
    },
    /// A fall hurt (the host reported the landing, [`CombatWorld::landed`]).
    Fell {
        /// Who.
        who: CombatantId,
        /// Landing speed, metres per second.
        speed: f32,
        /// Harm before armour.
        harm: f32,
    },
}

/// Fastest landing that does not hurt a person, metres per second (about a
/// 3.3 m drop). Mounts have their own ([`crate::mount::MountDef::safe_fall`]).
pub const SAFE_FALL: f32 = 8.0;

/// Simulation LOD for brains: how far from the focus points
/// ([`CombatWorld::set_focus`]) they think every step, every few, or rarely.
/// With no focus set, every brain thinks every step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatLod {
    /// Within this (metres, on the ground plane), every step.
    pub near: f32,
    /// Within this, every `mid_stride` steps. Beyond it a brain with no
    /// enemy in mind thinks every `far_stride` steps (enough to hear an alarm
    /// or feel a blow); one in a fight thinks as if mid-range.
    pub far: f32,
    /// Steps between a mid-range brain's thoughts.
    pub mid_stride: u32,
    /// Steps between a far, idle brain's thoughts.
    pub far_stride: u32,
}

impl Default for CombatLod {
    fn default() -> Self {
        Self {
            near: 45.0,
            far: 140.0,
            mid_stride: 4,
            far_stride: 30,
        }
    }
}

/// What the last step's brains did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LodStats {
    /// Brains near a focus point.
    pub near: usize,
    /// Brains mid-range.
    pub mid: usize,
    /// Brains far off.
    pub far: usize,
    /// Brains that thought this step.
    pub thought: usize,
}

/// A sound for AI hearing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Noise {
    pub source: CombatantId,
    pub position: Vec3,
    pub loudness: f32,
    /// World time it was made: it hangs in the air long enough for every
    /// brain to perceive once (they perceive at ten hertz, not every step).
    pub at: f64,
}

/// Slow-time focus.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Focus {
    who: CombatantId,
}

/// The combat simulation for one area.
#[derive(Clone, Debug)]
pub struct CombatWorld {
    /// Rules.
    pub rules: CombatRules,
    /// Who fights whom.
    pub relations: Relations,
    /// Reactive ground, if the area has one.
    pub environment: Option<EnvironmentField>,
    /// How dark it is, 0 (day) to 1 (under the Lid): the host copies the
    /// scene's `TimeOfDay` eclipse here. Sight shrinks to a third in full dark
    /// ([`crate::ai::DARK_SIGHT_LOSS`]).
    pub darkness: f32,
    spells: BTreeMap<String, SpellAsset>,
    combatants: BTreeMap<CombatantId, Combatant>,
    next_id: u32,
    pub(crate) projectiles: Vec<Projectile>,
    next_projectile: u32,
    pub(crate) walls: Vec<Wall>,
    next_wall: u32,
    pub(crate) waves: Vec<Wave>,
    pub(crate) blooms: Vec<Bloom>,
    time: f64,
    pub(crate) rng: Rng,
    events: Vec<CombatEvent>,
    pending: Vec<(CombatantId, Hit, Option<String>)>,
    pending_heals: Vec<(CombatantId, Option<CombatantId>, f32)>,
    pub(crate) noises: Vec<Noise>,
    focus: Option<Focus>,
    pub(crate) tactics: ai::Tactics,
    /// Brain LOD by distance from the focus points.
    pub lod: CombatLod,
    lod_focus: Vec<Vec3>,
    lod_stats: LodStats,
    steps: u64,
}

impl CombatWorld {
    /// An empty world with the library spells registered.
    #[must_use]
    pub fn new(rules: CombatRules) -> Self {
        let mut world = Self {
            rng: Rng::new(rules.seed),
            rules,
            relations: Relations::default(),
            environment: None,
            darkness: 0.0,
            spells: BTreeMap::new(),
            combatants: BTreeMap::new(),
            next_id: 1,
            projectiles: Vec::new(),
            next_projectile: 1,
            walls: Vec::new(),
            next_wall: 1,
            waves: Vec::new(),
            blooms: Vec::new(),
            time: 0.0,
            events: Vec::new(),
            pending: Vec::new(),
            pending_heals: Vec::new(),
            noises: Vec::new(),
            focus: None,
            tactics: ai::Tactics::default(),
            lod: CombatLod::default(),
            lod_focus: Vec::new(),
            lod_stats: LodStats::default(),
            steps: 0,
        };
        for spell in crate::spell::library::all() {
            world.add_spell(spell);
        }
        world
    }

    /// Register (or replace) a spell asset.
    pub fn add_spell(&mut self, spell: SpellAsset) {
        self.spells.insert(spell.id.clone(), spell);
    }

    /// A registered spell.
    #[must_use]
    pub fn spell(&self, id: &str) -> Option<&SpellAsset> {
        self.spells.get(id)
    }

    /// Seconds simulated (world time, slowed by focus).
    #[must_use]
    pub fn time(&self) -> f64 {
        self.time
    }

    /// Add a combatant.
    pub fn spawn(&mut self, desc: CombatantDesc) -> CombatantId {
        let position = desc.position;
        let id = CombatantId(self.next_id);
        self.next_id += 1;
        let mut status = StatusSet::default();
        status.resist = desc.status_resist;
        let ammo = desc.weapon.ranged.as_ref().map_or(0, |r| r.magazine);
        let brain = desc.brain.map(|d| ai::Brain::new(d, id));
        self.combatants.insert(
            id,
            Combatant {
                id,
                name: desc.name,
                team: desc.team,
                host: desc.host,
                position: desc.position,
                facing: planar_dir(desc.facing, Vec3::Z),
                velocity: Vec3::ZERO,
                body: desc.body,
                vitals: desc.vitals,
                resist: desc.resist,
                status,
                weapon: desc.weapon,
                sword: desc.swordsmanship,
                magic: desc.magic,
                action: Action::Idle,
                lock: None,
                kindled: false,
                ward: None,
                command: Command::default(),
                intent: MoveIntent {
                    facing: planar_dir(desc.facing, Vec3::Z),
                    ..MoveIntent::default()
                },
                brain,
                shaping: None,
                blade: None,
                prev_blade: None,
                ammo,
                riposte_until: f64::NEG_INFINITY,
                concealed_until: f64::NEG_INFINITY,
                cooldowns: BTreeMap::new(),
                knock: Vec3::ZERO,
                attack_held: 0.0,
                swings: 0,
                death: None,
                parts: desc.parts.into_iter().map(|p| (p, false)).collect(),
                hollowborn: desc.hollowborn,
                last_hurt: f64::NEG_INFINITY,
                last_attacker: None,
                mount: None,
                rider: None,
                herd: desc.mount.map(|_| crate::mount::Herd::new(position)),
                mount_def: desc.mount,
            },
        );
        id
    }

    /// Remove a combatant (despawned by the host).
    pub fn despawn(&mut self, id: CombatantId) -> Option<Combatant> {
        self.relations.forgive(id);
        self.tactics.forget(id);
        self.combatants.remove(&id)
    }

    /// A combatant.
    #[must_use]
    pub fn combatant(&self, id: CombatantId) -> Option<&Combatant> {
        self.combatants.get(&id)
    }

    /// A combatant, mutably (for the host to equip, heal, teach).
    pub fn combatant_mut(&mut self, id: CombatantId) -> Option<&mut Combatant> {
        self.combatants.get_mut(&id)
    }

    /// Every combatant, in id order.
    pub fn combatants(&self) -> impl Iterator<Item = &Combatant> {
        self.combatants.values()
    }

    /// Live projectiles.
    #[must_use]
    pub fn projectiles(&self) -> &[Projectile] {
        &self.projectiles
    }

    /// Standing walls.
    #[must_use]
    pub fn walls(&self) -> &[Wall] {
        &self.walls
    }

    /// Travelling waves.
    #[must_use]
    pub fn waves(&self) -> &[Wave] {
        &self.waves
    }

    /// Gathering blooms.
    #[must_use]
    pub fn blooms(&self) -> &[Bloom] {
        &self.blooms
    }

    /// Who is focusing, if anyone.
    #[must_use]
    pub fn focusing(&self) -> Option<CombatantId> {
        self.focus.map(|f| f.who)
    }

    /// Time scale of everything but the focuser.
    #[must_use]
    pub fn world_scale(&self) -> f32 {
        if self.focus.is_some() {
            self.rules.focus_world_scale
        } else {
            1.0
        }
    }

    /// Time scale a body runs at now (for animation playback).
    #[must_use]
    pub fn time_scale_for(&self, id: CombatantId) -> f32 {
        match self.focus {
            Some(f) if f.who == id => self.rules.focus_self_scale,
            Some(_) => self.rules.focus_world_scale,
            None => 1.0,
        }
    }

    /// The host reports where its controller put a body.
    pub fn set_pose(&mut self, id: CombatantId, position: Vec3, facing: Vec3, dt: f32) {
        if let Some(c) = self.combatants.get_mut(&id) {
            if !position.is_finite() {
                return;
            }
            if dt > 0.0 && dt.is_finite() {
                c.velocity = (position - c.position) / dt;
            }
            c.position = position;
            if !matches!(c.action, Action::Attack(_)) {
                c.facing = planar_dir(facing, c.facing);
            }
        }
    }

    /// The points that matter for brain LOD: the player, the camera. Brains
    /// far from all of them with no enemy in mind think every
    /// [`CombatLod::far_stride`] steps; mid-range ones every
    /// [`CombatLod::mid_stride`]. An empty list (the default)
    /// keeps every brain thinking every step.
    pub fn set_focus(&mut self, points: &[Vec3]) {
        self.lod_focus.clear();
        self.lod_focus
            .extend(points.iter().filter(|p| p.is_finite()));
    }

    /// What the last step's brains did.
    #[must_use]
    pub fn lod_stats(&self) -> LodStats {
        self.lod_stats
    }

    /// Take out the brains that do not think this step; the caller puts them
    /// back after `ai::think`. Their last command stands meanwhile.
    fn park_brains(&mut self) -> Vec<(CombatantId, ai::Brain)> {
        let mut stats = LodStats::default();
        let mut parked = Vec::new();
        if self.lod_focus.is_empty() {
            stats.near = self
                .combatants
                .values()
                .filter(|c| c.brain.is_some())
                .count();
            stats.thought = stats.near;
            self.lod_stats = stats;
            return parked;
        }
        let (lod, steps) = (self.lod, self.steps);
        let focus = &self.lod_focus;
        for c in self.combatants.values_mut() {
            let Some(brain) = &c.brain else {
                continue;
            };
            let d2 = focus
                .iter()
                .map(|f| flat(*f - c.position).length_squared())
                .fold(f32::INFINITY, f32::min);
            let engaged = brain.mind.target.is_some() || brain.mind.lead.is_some();
            let stride = if d2 <= lod.near * lod.near {
                stats.near += 1;
                1
            } else if d2 <= lod.far * lod.far {
                stats.mid += 1;
                lod.mid_stride
            } else {
                stats.far += 1;
                if engaged {
                    lod.mid_stride
                } else {
                    lod.far_stride
                }
            };
            let stride = u64::from(stride.max(1));
            if (steps + u64::from(c.id.0)) % stride != 0 {
                parked.push((c.id, c.brain.take().expect("checked")));
                continue;
            }
            stats.thought += 1;
        }
        self.lod_stats = stats;
        parked
    }

    /// The host's controller set a body down after a fall at `speed` metres a
    /// second. Past a safe speed it hurts, and floors past a hard one; a mount
    /// that lands hard throws its rider, who takes the fall too.
    pub fn landed(&mut self, id: CombatantId, speed: f32) {
        let Some(c) = self.combatants.get(&id) else {
            return;
        };
        let safe = c.mount_def.map_or(SAFE_FALL, |m| m.safe_fall);
        let over = speed - safe;
        if !c.alive() || !over.is_finite() || over <= 0.0 {
            return;
        }
        let harm = 6.0 * over + 3.0 * over * over;
        let mut hit = Hit::new(
            None,
            vec![crate::damage::DamagePart::new(DamageType::Blunt, harm)],
            25.0 * over,
        );
        hit.flags = HitFlags::ENVIRONMENT | HitFlags::UNBLOCKABLE | HitFlags::UNPARRYABLE;
        if over > 3.0 {
            hit.flags |= HitFlags::HEAVY;
        }
        hit.point = c.position;
        hit.direction = Vec3::NEG_Y;
        let rider = c.rider;
        self.queue_hit(id, hit, None);
        self.emit(CombatEvent::Fell {
            who: id,
            speed,
            harm,
        });
        if over > 3.0
            && let Some(rider) = rider
        {
            crate::mount::dismount(self, rider);
            self.landed(rider, speed);
        }
    }

    /// The host reports the blade from the skeleton (hilt, tip), for moves
    /// with [`crate::weapon::HitShape::Blade`].
    pub fn set_blade(&mut self, id: CombatantId, hilt: Vec3, tip: Vec3) {
        if let Some(c) = self.combatants.get_mut(&id)
            && hilt.is_finite()
            && tip.is_finite()
        {
            c.blade = Some([hilt, tip]);
        }
    }

    /// The player's (or a script's) command for the coming step(s). Press
    /// edges are consumed by the first step that reads them.
    pub fn command(&mut self, id: CombatantId, command: Command) {
        if let Some(c) = self.combatants.get_mut(&id) {
            c.command = command;
        }
    }

    /// An animation event from the clip a body is playing. Names:
    /// `hit_on` / `hit_off` open and close the blow, `recover` ends the move,
    /// `release` lets a gathered spell go now.
    pub fn animation_event(&mut self, id: CombatantId, event: &somnium_anim::AnimationEvent) {
        melee::animation_event(self, id, event);
    }

    /// Cast a finished request now (scripts, quick-casts, tests). The player's
    /// hotbar goes through [`Command::spells`] instead.
    pub fn cast(&mut self, id: CombatantId, request: SpellRequest) -> Result<(), SpellError> {
        cast::begin_cast(self, id, request, None)
    }

    /// Movement intent for the host's controller.
    #[must_use]
    pub fn intent(&self, id: CombatantId) -> Option<MoveIntent> {
        self.combatants.get(&id).map(|c| c.intent)
    }

    /// Events since the last drain.
    pub fn drain_events(&mut self) -> Vec<CombatEvent> {
        std::mem::take(&mut self.events)
    }

    /// Events so far this step and unread (for inspection).
    #[must_use]
    pub fn events(&self) -> &[CombatEvent] {
        &self.events
    }

    pub(crate) fn emit(&mut self, event: CombatEvent) {
        self.events.push(event);
    }

    pub(crate) fn noise(&mut self, source: CombatantId, position: Vec3, loudness: f32) {
        if loudness > 0.0 {
            self.noises.push(Noise {
                source,
                position,
                loudness,
                at: self.time,
            });
            self.events.push(CombatEvent::Noise {
                source,
                position,
                loudness,
            });
        }
    }

    pub(crate) fn queue_hit(&mut self, target: CombatantId, hit: Hit, part: Option<String>) {
        self.pending.push((target, hit, part));
    }

    pub(crate) fn queue_heal(
        &mut self,
        target: CombatantId,
        source: Option<CombatantId>,
        amount: f32,
    ) {
        self.pending_heals.push((target, source, amount));
    }

    pub(crate) fn ids(&self) -> Vec<CombatantId> {
        self.combatants.keys().copied().collect()
    }

    pub(crate) fn get(&self, id: CombatantId) -> Option<&Combatant> {
        self.combatants.get(&id)
    }

    pub(crate) fn get_mut(&mut self, id: CombatantId) -> Option<&mut Combatant> {
        self.combatants.get_mut(&id)
    }

    pub(crate) fn next_projectile_id(&mut self) -> u32 {
        let id = self.next_projectile;
        self.next_projectile += 1;
        id
    }

    pub(crate) fn next_wall_id(&mut self) -> u32 {
        let id = self.next_wall;
        self.next_wall += 1;
        id
    }

    /// Whether `a` fights `b`.
    #[must_use]
    pub fn hostile(&self, a: CombatantId, b: CombatantId) -> bool {
        match (self.combatants.get(&a), self.combatants.get(&b)) {
            (Some(x), Some(y)) => self.relations.hostile((x.id, x.team), (y.id, y.team)),
            _ => false,
        }
    }

    /// Whether `a` counts `b` a friend.
    #[must_use]
    pub fn allied(&self, a: CombatantId, b: CombatantId) -> bool {
        match (self.combatants.get(&a), self.combatants.get(&b)) {
            (Some(x), Some(y)) => self.relations.allied((x.id, x.team), (y.id, y.team)),
            _ => false,
        }
    }

    /// Whether a direct hit from `source` may touch `target`.
    pub(crate) fn may_hit(
        &self,
        source: Option<CombatantId>,
        target: CombatantId,
        area: bool,
    ) -> bool {
        let Some(source) = source else {
            return true;
        };
        if source == target {
            return false;
        }
        if self.hostile(source, target) {
            return true;
        }
        if self.allied(source, target) {
            return if area {
                self.rules.friendly_fire_area
            } else {
                self.rules.friendly_fire_direct
            };
        }
        // Neutral: a direct blow provokes; areas catch bystanders.
        true
    }

    /// Advance one fixed step.
    pub fn step(
        &mut self,
        dt: f32,
        query: &impl CombatQuery,
        nav: Option<&somnium_ai::navigation::NavWorld>,
    ) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        // 1. Focus.
        self.step_focus(dt);
        let world_dt = dt * self.world_scale();
        self.time += f64::from(world_dt);

        // 2. The ground.
        self.step_ground(world_dt);

        // 3. Bodies.
        for id in self.ids() {
            let body_dt = dt * self.time_scale_for(id);
            self.step_body(id, body_dt);
        }

        // 4. Brains (far ones asleep, mid-range ones on their stride).
        self.steps += 1;
        let parked = self.park_brains();
        ai::think(self, world_dt, query, nav);
        for (id, brain) in parked {
            if let Some(c) = self.combatants.get_mut(&id) {
                c.brain = Some(brain);
            }
        }

        // 5-6. Commands and actions (a ridden mount takes its rider's reins
        // first; one left alone keeps its own counsel).
        crate::mount::herd(self, query);
        crate::mount::drive(self);
        for id in self.ids() {
            let body_dt = dt * self.time_scale_for(id);
            melee::handle_command(self, id, body_dt, query);
            melee::advance(self, id, body_dt, query);
        }

        // 7. Melee volumes.
        melee::collect_hits(self);

        // 8. Projectiles.
        projectile::step(self, world_dt, query);

        // 9. Walls, waves, blooms, wards.
        effects::step(self, world_dt);

        // 10. Resolution.
        resolve::resolve_pending(self);

        // 11. Movement.
        for id in self.ids() {
            let body_dt = dt * self.time_scale_for(id);
            self.compute_intent(id, body_dt);
        }
        if self.rules.integrate_motion {
            self.integrate(dt, query, |_| true);
        }
        crate::mount::sync_riders(self);
        let heard_for = f64::from(ai::THINK_INTERVAL) * 1.5;
        let now = self.time;
        self.noises.retain(|n| now - n.at < heard_for);
        let ids = self.ids();
        self.tactics.update(self.time, &ids, |id| {
            self.combatants.get(&id).is_some_and(Combatant::alive)
        });
    }

    fn step_focus(&mut self, dt: f32) {
        let Some(focus) = self.focus else {
            return;
        };
        let drain = self.rules.focus_drain * dt;
        let ended = match self.combatants.get_mut(&focus.who) {
            Some(c) if c.alive() => !c.vitals.spend_focus(drain),
            _ => true,
        };
        if ended {
            self.focus = None;
            self.emit(CombatEvent::Focus {
                who: focus.who,
                active: false,
            });
        }
    }

    pub(crate) fn toggle_focus(&mut self, id: CombatantId) {
        match self.focus {
            Some(f) if f.who == id => {
                self.focus = None;
                self.emit(CombatEvent::Focus {
                    who: id,
                    active: false,
                });
            }
            Some(_) => {}
            None => {
                if self
                    .combatants
                    .get(&id)
                    .is_some_and(|c| c.alive() && c.vitals.focus.current >= 10.0)
                {
                    self.focus = Some(Focus { who: id });
                    self.emit(CombatEvent::Focus {
                        who: id,
                        active: true,
                    });
                }
            }
        }
    }

    fn step_ground(&mut self, dt: f32) {
        let Some(mut field) = self.environment.take() else {
            return;
        };
        field.tick(dt, &mut self.rng);
        for change in field.drain_changes() {
            self.events.push(CombatEvent::Surface(change));
        }
        let standing: Vec<(CombatantId, Surface, bool)> = self
            .combatants
            .values()
            .filter(|c| c.alive() && c.mount.is_none())
            .map(|c| (c.id, field.surface_at(c.position), field.wet_at(c.position)))
            .collect();
        self.environment = Some(field);
        for (id, surface, wet) in standing {
            match surface {
                Surface::Water => self.apply_status(id, StatusKind::Wet, 6.0),
                Surface::Burning => {
                    // Standing in fire: a fresh burn every step it is not
                    // already burning (the burn itself is the damage).
                    if self
                        .combatants
                        .get(&id)
                        .is_some_and(|c| !c.status.active(StatusKind::Burn))
                    {
                        self.apply_status(id, StatusKind::Burn, 6.0);
                    }
                }
                _ if wet => self.apply_status(id, StatusKind::Wet, 2.0),
                _ => {}
            }
        }
    }

    /// Apply a status and report what changed.
    pub fn apply_status(&mut self, id: CombatantId, kind: StatusKind, amount: f32) {
        let mut changes = Vec::new();
        let Some(c) = self.combatants.get_mut(&id) else {
            return;
        };
        if !c.alive() {
            return;
        }
        let max = c.vitals.health.max;
        c.status.apply(kind, amount, max, &mut changes);
        self.status_changes(id, changes);
    }

    pub(crate) fn status_changes(&mut self, id: CombatantId, changes: Vec<StatusChange>) {
        for change in changes {
            match change {
                StatusChange::Applied(kind) => {
                    self.emit(CombatEvent::StatusApplied { target: id, kind });
                    if matches!(
                        kind,
                        StatusKind::Freeze | StatusKind::Stagger | StatusKind::Knockdown
                    ) {
                        melee::interrupt(self, id);
                    }
                }
                StatusChange::Ended(kind) => {
                    self.emit(CombatEvent::StatusEnded { target: id, kind })
                }
                StatusChange::Extinguished | StatusChange::Steam => {
                    self.emit(CombatEvent::Extinguished { target: id })
                }
                StatusChange::BleedBurst(amount) => {
                    self.emit(CombatEvent::BleedBurst { target: id, amount });
                    let mut hit = Hit::new(None, vec![], 0.0);
                    hit.flags = HitFlags::TICK;
                    resolve::true_damage(self, id, amount, hit);
                }
            }
        }
    }

    fn step_body(&mut self, id: CombatantId, dt: f32) {
        let focusing = self.focus.is_some_and(|f| f.who == id);
        let Some(c) = self.combatants.get_mut(&id) else {
            return;
        };
        if let Some(death) = &mut c.death {
            death.t += dt;
        }
        if !c.alive() {
            return;
        }
        let mut changes = Vec::new();
        let burn = c.status.tick(dt, &mut changes);
        let exerting = c.command.sprint && c.command.move_dir.length_squared() > 0.01
            || matches!(c.action, Action::Guard { .. })
            || matches!(&c.action, Action::Cast(s) if s.phase == CastPhase::Channel);
        c.vitals.tick(dt, exerting, focusing);
        if c.command.sprint && c.command.move_dir.length_squared() > 0.01 {
            c.vitals.drain_stamina(10.0 * dt);
        }
        if c.kindled {
            let drain = c.kindle_drain() * dt;
            if c.vitals.mana.current < drain {
                c.kindled = false;
                self.events
                    .push(CombatEvent::Kindled { who: id, on: false });
            } else {
                c.vitals.mana.current -= drain;
            }
        }
        let c = self.combatants.get_mut(&id).expect("present");
        let mut ward_broke = false;
        if let Some(ward) = &mut c.ward {
            ward.time_left -= dt;
            if ward.time_left <= 0.0 {
                c.ward = None;
                ward_broke = true;
            }
        }
        if c.lock.is_some_and(|t| t == id) {
            c.lock = None;
        }
        if ward_broke {
            self.emit(CombatEvent::WardBroken { target: id });
        }
        self.status_changes(id, changes);
        if burn > 0.0 {
            let mut hit = Hit::new(
                None,
                vec![crate::DamagePart::new(DamageType::Flame, burn)],
                0.0,
            );
            hit.flags = HitFlags::TICK | HitFlags::ENVIRONMENT;
            resolve::apply_harm(self, id, &hit, None);
        }
        // A lock on the dead or the gone lets go.
        let lock = self.combatants.get(&id).and_then(|c| c.lock);
        if let Some(target) = lock
            && !self.combatants.get(&target).is_some_and(Combatant::alive)
        {
            if let Some(c) = self.combatants.get_mut(&id) {
                c.lock = None;
            }
            self.emit(CombatEvent::LockChanged {
                who: id,
                target: None,
            });
        }
    }

    fn compute_intent(&mut self, id: CombatantId, dt: f32) {
        let scale = self.time_scale_for(id);
        let traction = self
            .environment
            .as_ref()
            .zip(self.combatants.get(&id))
            .map_or(1.0, |(f, c)| {
                // A sure-footed mount (the frost-elk) ignores the ice.
                let sure = c.mount_def.map_or(0.0, |m| m.sure_footed);
                let t = f.surface_at(c.position).traction();
                t + (1.0 - t) * sure
            });
        let lock_target = self
            .combatants
            .get(&id)
            .and_then(|c| c.lock)
            .and_then(|t| self.combatants.get(&t))
            .map(|t| t.position);
        let Some(c) = self.combatants.get_mut(&id) else {
            return;
        };
        let mut intent = MoveIntent {
            facing: c.facing,
            traction,
            time_scale: scale * c.status.action_scale(),
            ..MoveIntent::default()
        };
        // A rider is carried by the mount (which reads its reins) but still
        // looks where it aims, at its lock, or the way it rides.
        if c.mount.is_some() && c.alive() && c.status.can_act() {
            let desired = lock_target
                .map(|p| p - c.position)
                .or_else(|| c.command.aim.map(|p| p - c.position))
                .or_else(|| {
                    (flat(c.command.move_dir).length_squared() > 0.01).then_some(c.command.move_dir)
                });
            if let Some(d) = desired
                && !matches!(&c.action, Action::Attack(a) if a.phase != Phase::Windup)
            {
                c.facing = turn_toward(c.facing, planar_dir(d, c.facing), 12.0 * dt);
            }
        }
        if !c.alive() || c.status.move_scale() == 0.0 || c.mount.is_some() {
            intent.rooted = true;
            intent.facing = c.facing;
            intent.velocity = c.knock;
            c.knock *= (1.0 - 8.0 * dt).max(0.0);
            c.intent = intent;
            return;
        }
        let kindle = c.kindle_strength().sqrt();
        let cmd = &c.command;
        let wants = flat(cmd.move_dir).clamp_length_max(1.0);
        let sprint =
            cmd.sprint && c.vitals.stamina.current > 0.0 && matches!(c.action, Action::Idle);
        let gallop = c.mount_def.map_or(1.7, |m| m.gallop);
        let base = c.body.speed * if sprint { gallop } else { 1.0 } * kindle;
        let action_scale = match &c.action {
            Action::Idle => 1.0,
            Action::Attack(a) => match a.phase {
                Phase::Windup | Phase::Active => 0.15,
                Phase::Recovery => 0.45,
            },
            Action::Guard { .. } => 0.45,
            Action::Aim { .. } => 0.4,
            Action::Reload { .. } => 0.6,
            Action::Cast(cast) => match (cast.phase, cast.request.release) {
                (CastPhase::Gather, CastSpeed::Incantation) => 0.25,
                (CastPhase::Gather, _) => 0.55,
                (CastPhase::Channel, _) => 0.3,
                (CastPhase::Recover, _) => 0.6,
            },
            Action::Recover { .. } => 0.5,
            Action::Dodge(_) | Action::Dead => 0.0,
        };
        let mut velocity = wants * base * action_scale * c.status.move_scale();
        // Root displacement from the action itself.
        match &c.action {
            Action::Attack(a) if matches!(a.phase, Phase::Windup | Phase::Active) => {
                let span = if a.def.kind == crate::weapon::MoveKind::Charge {
                    a.def.active.max(0.05)
                } else {
                    (a.def.windup + a.def.active).max(0.05)
                };
                let lunging =
                    a.def.kind != crate::weapon::MoveKind::Charge || a.phase == Phase::Active;
                if lunging {
                    velocity += a.dir * a.def.lunge / span * c.status.action_scale();
                }
            }
            Action::Dodge(d) => {
                velocity = d.dir * d.distance / d.duration.max(0.05);
            }
            _ => {}
        }
        velocity += c.knock;
        c.knock *= (1.0 - 8.0 * dt).max(0.0);
        // Facing: toward the aim, the lock, or the way of travel; never mid-blow.
        let desired = match &c.action {
            Action::Attack(a) if a.phase != Phase::Windup => None,
            Action::Dodge(_) => None,
            _ => lock_target
                .map(|p| p - c.position)
                .or_else(|| cmd.aim.map(|p| p - c.position))
                .or_else(|| (wants.length_squared() > 0.01).then_some(wants)),
        };
        if let Some(d) = desired {
            let rate = match c.action {
                Action::Attack(_) => 6.0,
                _ => 12.0,
            };
            c.facing = turn_toward(c.facing, planar_dir(d, c.facing), rate * dt);
        }
        intent.velocity = velocity;
        intent.facing = c.facing;
        intent.sprinting = sprint && wants.length_squared() > 0.01;
        intent.climb = cmd.climb;
        c.intent = intent;
    }

    /// How far to push a body this step so it stops overlapping the standing
    /// others: half of each overlap, on the ground plane. The world's own
    /// motion adds it; a host's controller adds it to the intent.
    #[must_use]
    pub fn separation(&self, id: CombatantId) -> Vec3 {
        let Some(me) = self.combatants.get(&id) else {
            return Vec3::ZERO;
        };
        self.combatants
            .values()
            .filter(|o| o.id != id && o.alive() && o.mount.is_none())
            .fold(Vec3::ZERO, |push, o| {
                let d = flat(me.position - o.position);
                let (len, min) = (d.length(), me.body.radius + o.body.radius);
                if len < min && len > 1e-4 {
                    push + d / len * (min - len) * 0.5
                } else {
                    push
                }
            })
    }

    /// [`CombatWorld::separation`] for many bodies at once, with the same bits
    /// (the same terms summed in the same order), but from the bodies in the
    /// neighbouring cells of a grid twice the largest radius wide instead of
    /// from everyone: a crowd's pushes cost what is near each body.
    #[must_use]
    pub fn separations(&self, ids: &[CombatantId]) -> Vec<(CombatantId, Vec3)> {
        // In id order, as `separation` folds them.
        let others: Vec<&Combatant> = self
            .combatants
            .values()
            .filter(|o| o.alive() && o.mount.is_none())
            .collect();
        // Two bodies push only when closer than their radii together.
        let cell = self
            .combatants
            .values()
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
                let Some(me) = self.combatants.get(&id) else {
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
                let push = near
                    .iter()
                    .map(|&i| others[i])
                    .filter(|o| o.id != id)
                    .fold(Vec3::ZERO, |push, o| {
                        let d = flat(me.position - o.position);
                        let (len, min) = (d.length(), me.body.radius + o.body.radius);
                        if len < min && len > 1e-4 {
                            push + d / len * (min - len) * 0.5
                        } else {
                            push
                        }
                    });
                (id, push)
            })
            .collect()
    }

    /// The first standing spell wall (stone, ice, force) that stops an upright
    /// capsule standing on `feet` (`radius`, overall `height`) moved by
    /// `delta`, for a host's controller to add to its own world. A wall raised
    /// around a body does not hold it, so it walks out.
    #[must_use]
    pub fn wall_contact(
        &self,
        feet: Vec3,
        delta: Vec3,
        radius: f32,
        height: f32,
    ) -> Option<WorldHit> {
        let len = delta.length();
        let dir = delta.try_normalize()?;
        let core = (height * 0.5 - radius).max(0.0);
        let centre = feet + Vec3::Y * (height * 0.5);
        self.walls
            .iter()
            .filter(|w| w.blocks_movement())
            .filter_map(|w| {
                // The box grown by the capsule's core (walls stand upright,
                // so their own Y is the world's): square where the capsule's
                // caps are round, which a wall does not notice.
                let grown = Obb {
                    half: w.obb.half + Vec3::Y * core,
                    ..w.obb
                };
                if grown.distance(centre) <= radius {
                    return None;
                }
                grown.ray(centre, dir, len, radius)
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(t, normal)| WorldHit {
                fraction: (t / len).clamp(0.0, 1.0),
                normal,
            })
    }

    /// Move the bodies `only` picks by their intents, sliding along spell
    /// walls and the host's world, keep bodies apart, and seat riders on
    /// their mounts. [`Self::step`] does this for everyone when
    /// [`CombatRules::integrate_motion`] is on; a host whose controller moves
    /// people can still leave mounts to it.
    pub fn integrate(
        &mut self,
        dt: f32,
        query: &impl CombatQuery,
        only: impl Fn(&Combatant) -> bool,
    ) {
        let blockers: Vec<Obb> = self
            .walls
            .iter()
            .filter(|w| w.blocks_movement())
            .map(|w| w.obb)
            .collect();
        let bodies: Vec<(CombatantId, Vec3, f32, bool)> = self
            .combatants
            .values()
            .map(|c| {
                (
                    c.id,
                    c.position,
                    c.body.radius,
                    c.alive() && c.mount.is_none(),
                )
            })
            .collect();
        // Pushes from where everyone stood before anyone moved.
        let ids: Vec<CombatantId> = bodies.iter().map(|b| b.0).collect();
        let pushes: Vec<Vec3> = self.separations(&ids).into_iter().map(|(_, push)| push).collect();
        for ((id, position, radius, alive), push) in bodies.iter().copied().zip(pushes) {
            let Some(c) = self.combatants.get(&id) else {
                continue;
            };
            if !only(c) {
                continue;
            }
            let scale = self.time_scale_for(id);
            let displacement = c.intent.velocity * dt * scale + push;
            if !alive && c.knock.length_squared() < 1e-4 {
                continue;
            }
            let height = c.body.height;
            let chest = Vec3::Y * height * 0.5;
            let applied = somnium_anim::collide_and_slide(
                position + chest,
                displacement,
                0.02,
                |from, delta| {
                    let len = delta.length();
                    let dir = delta.try_normalize()?;
                    let mut best: Option<(f32, Vec3)> = None;
                    for b in &blockers {
                        if let Some((t, n)) = b.ray(from, dir, len, radius)
                            && best.is_none_or(|x| t < x.0)
                        {
                            best = Some((t, n));
                        }
                    }
                    if let Some(hit) = query.sweep(from, from + delta, radius)
                        && best.is_none_or(|x| hit.fraction * len < x.0)
                    {
                        best = Some((hit.fraction * len, hit.normal));
                    }
                    best.map(|(t, n)| somnium_anim::MotionHit {
                        fraction: (t / len).clamp(0.0, 1.0),
                        normal: flat(n).try_normalize().unwrap_or(-dir),
                    })
                },
            );
            let applied = applied.map_or(Vec3::ZERO, |a| a.applied);
            if let Some(c) = self.combatants.get_mut(&id) {
                c.position += flat(applied);
                if dt > 0.0 {
                    c.velocity = flat(applied) / dt;
                }
            }
        }
        crate::mount::sync_riders(self);
    }
}

/// Rotate a planar unit vector toward another by at most `max` radians.
#[must_use]
pub fn turn_toward(from: Vec3, to: Vec3, max: f32) -> Vec3 {
    let a = from.z.atan2(from.x);
    let b = to.z.atan2(to.x);
    let mut d = b - a;
    while d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    }
    while d < -std::f32::consts::PI {
        d += std::f32::consts::TAU;
    }
    let step = d.clamp(-max.abs(), max.abs());
    let r = a + step;
    Vec3::new(r.cos(), 0.0, r.sin())
}
