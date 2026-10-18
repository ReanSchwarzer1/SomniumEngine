//! Move timings from the animators' clip table, so a blow lands when the
//! blade does.
//!
//! A character team publishes each clip's length and the moments that matter
//! (Rimeborn's `assets/animation/clips.json`, WS3's contract): per clip its
//! `seconds`, its `combat_events` under this crate's names (`hit_on`,
//! `hit_off`, `recover`, `release`), and its authored `events` (`combo` as
//! `[from, to]` seconds, `cancel` in seconds); `aliases` maps a move's
//! `animation` name to a clip. [`ClipTable::retime_weapon`] fits a weapon's
//! moves to their clips: the windup lasts until `hit_on`, the hit volume is
//! live until `hit_off`, recovery runs to `recover` (else the clip's end), and
//! the combo window and dodge cancel become fractions of that recovery. A move
//! whose clip is unknown, or has no hit window, keeps its own timings.
//!
//! Or the other way about, which keeps a game's balance: [`ClipTable::warp`]
//! leaves the move's designed timings alone and says how fast to play each
//! part of its clip so the clip's hit lands on the designed frame
//! ([`ClipWarp::clip_time`] maps time in the move to time in the clip). A
//! part played at under half or over twice its speed is worth an animator's
//! look ([`ClipWarp::stretched`]).

use crate::weapon::{MoveDef, WeaponDef};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

/// The clip table.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ClipTable {
    /// Move animation names to clip names.
    #[serde(default)]
    pub aliases: HashMap<String, String>,
    /// Clips by name.
    #[serde(default)]
    pub clips: HashMap<String, ClipTiming>,
}

/// One clip's timing.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ClipTiming {
    /// Length.
    pub seconds: f32,
    /// Moments under this crate's names, in seconds.
    #[serde(default)]
    pub combat_events: HashMap<String, f32>,
    /// The animators' own events (`combo`, `cancel`, ...), in seconds.
    #[serde(default)]
    pub events: HashMap<String, Value>,
}

impl ClipTable {
    /// Read a clip table.
    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }

    /// The clip a move's `animation` plays (through an alias, or by its own
    /// name).
    #[must_use]
    pub fn clip(&self, animation: &str) -> Option<&ClipTiming> {
        self.clips.get(
            self.aliases
                .get(animation)
                .map_or(animation, String::as_str),
        )
    }

    /// Fit one move to its clip; false (the move untouched) when the clip is
    /// unknown or has no hit window.
    pub fn retime(&self, m: &mut MoveDef) -> bool {
        let Some(clip) = self.clip(&m.animation) else {
            return false;
        };
        let event = |name: &str| clip.combat_events.get(name).copied();
        let (Some(on), Some(off)) = (event("hit_on"), event("hit_off")) else {
            return false;
        };
        if !(on >= 0.0 && off > on) {
            return false;
        }
        let end = event("recover").unwrap_or(clip.seconds).max(off);
        let recovery = (end - off).max(1e-3);
        let fraction = |t: f64| ((t as f32 - off) / recovery).clamp(0.0, 1.0);
        m.windup = on;
        m.active = off - on;
        m.recovery = recovery;
        if let Some([from, to]) = clip.events.get("combo").and_then(pair) {
            m.combo_window = [fraction(from), fraction(to)];
        }
        if let Some(cancel) = clip.events.get("cancel").and_then(Value::as_f64) {
            m.dodge_cancel = fraction(cancel);
        }
        true
    }

    /// How to play a move's clip to its designed timings; `None` when the
    /// clip is unknown or has no hit window.
    #[must_use]
    pub fn warp(&self, m: &MoveDef) -> Option<ClipWarp> {
        let clip = self.clip(&m.animation)?;
        let event = |name: &str| clip.combat_events.get(name).copied();
        let (on, off) = (event("hit_on")?, event("hit_off")?);
        if !(on >= 0.0 && off > on) {
            return None;
        }
        let end = event("recover").unwrap_or(clip.seconds).max(off);
        Some(ClipWarp {
            clip: [on, off, end],
            moves: [m.windup, m.active, m.recovery],
        })
    }

    /// Fit every melee move of a weapon (its light chain, heavy and charge);
    /// returns how many were fitted.
    pub fn retime_weapon(&self, weapon: &mut WeaponDef) -> usize {
        let moves = weapon
            .light
            .iter_mut()
            .chain(weapon.heavy.as_mut())
            .chain(weapon.charge.as_mut());
        moves.map(|m| usize::from(self.retime(m))).sum()
    }
}

/// How to play a move's clip so its hit lands on the move's designed frame:
/// each part (windup, the live hit, recovery) at its own rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipWarp {
    /// Clip seconds at `hit_on`, `hit_off` and the end of recovery.
    pub clip: [f32; 3],
    /// The move's windup, active and recovery seconds.
    pub moves: [f32; 3],
}

impl ClipWarp {
    /// Playback rate of each part (clip seconds a move second).
    #[must_use]
    pub fn rates(&self) -> [f32; 3] {
        let [on, off, end] = self.clip;
        let clip = [on, off - on, end - off];
        std::array::from_fn(|i| clip[i] / self.moves[i].max(1e-3))
    }

    /// Whether some part plays at under half or over twice its speed.
    #[must_use]
    pub fn stretched(&self) -> bool {
        self.rates().iter().any(|r| !(0.5..=2.0).contains(r))
    }

    /// Where in the clip to be `t` seconds into the move.
    #[must_use]
    pub fn clip_time(&self, t: f32) -> f32 {
        let [on, off, end] = self.clip;
        let [windup, active, recovery] = self.moves;
        let part = |t: f32, len: f32, from: f32, to: f32| {
            from + (to - from) * (t / len.max(1e-3)).clamp(0.0, 1.0)
        };
        if t < windup {
            part(t, windup, 0.0, on)
        } else if t < windup + active {
            part(t - windup, active, on, off)
        } else {
            part(t - windup - active, recovery, off, end)
        }
    }
}

fn pair(v: &Value) -> Option<[f64; 2]> {
    match v.as_array()?.as_slice() {
        [a, b] => Some([a.as_f64()?, b.as_f64()?]),
        _ => None,
    }
}
