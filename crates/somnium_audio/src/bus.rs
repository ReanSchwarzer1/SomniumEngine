//! Mixer buses (MORROWIND-AG).
//!
//! **This file used to be one line: `// Bus stub`.**
//!
//! A bus is a named volume a group of sounds routes through: Master, Music,
//! SFX, Dialogue, UI. A game needs them for the one reason every options screen
//! demonstrates — a player who wants the music quieter does not want the
//! dialogue quieter — and an engine without them makes that slider impossible
//! to build.
//!
//! # The gain graph is resolved here, not by Kira
//!
//! Kira has tracks and can nest them, so routing *could* be pushed down. It is
//! not, for one reason: **solo**. Solo is not a property of a bus, it is a
//! property of the whole mixer — soloing one bus mutes every bus that is not
//! soloed — and expressing that through per-track volumes means recomputing
//! every track whenever any bus changes. Resolving the graph here makes that one
//! function with one test, and Kira receives the answer.

use std::collections::BTreeMap;

/// A named mixer bus.
#[derive(Clone, Debug, PartialEq)]
pub struct Bus {
    /// The bus this one feeds into. `None` for the master bus.
    ///
    /// A tree rather than a flat list because "quieter dialogue" and "quieter
    /// everything" are different sliders and the second has to affect the
    /// first. A flat list makes master volume a multiplication every caller has
    /// to remember.
    pub parent: Option<String>,
    /// Linear gain, `0.0..`. 1.0 is unity.
    pub volume: f32,
    /// Silenced by the player.
    pub muted: bool,
    /// Soloed. Any solo anywhere silences every bus not on a soloed path.
    pub soloed: bool,
}

impl Default for Bus {
    fn default() -> Self {
        Self {
            parent: Some(Mixer::MASTER.to_string()),
            volume: 1.0,
            muted: false,
            soloed: false,
        }
    }
}

/// The bus graph.
#[derive(Clone, Debug)]
pub struct Mixer {
    buses: BTreeMap<String, Bus>,
}

impl Default for Mixer {
    /// The buses a game actually ships with.
    ///
    /// Not an empty mixer: an options screen needs these five names to exist
    /// before anything plays, and a game that has to declare them itself will
    /// declare four of them and discover the fifth in a bug report.
    fn default() -> Self {
        let mut mixer = Self {
            buses: BTreeMap::new(),
        };
        mixer.buses.insert(
            Self::MASTER.to_string(),
            Bus {
                parent: None,
                ..Default::default()
            },
        );
        for name in [Self::MUSIC, Self::SFX, Self::DIALOGUE, Self::UI] {
            mixer.buses.insert(name.to_string(), Bus::default());
        }
        mixer
    }
}

impl Mixer {
    /// Everything routes here.
    pub const MASTER: &'static str = "master";
    /// Music and ambience.
    pub const MUSIC: &'static str = "music";
    /// World sounds.
    pub const SFX: &'static str = "sfx";
    /// Speech. Its own bus because it is the one players raise rather than
    /// lower, and because it is what a subtitle setting is an alternative to.
    pub const DIALOGUE: &'static str = "dialogue";
    /// Interface sounds, which must stay audible when the world is ducked.
    pub const UI: &'static str = "ui";

    /// An empty mixer with only a master bus.
    #[must_use]
    pub fn empty() -> Self {
        let mut buses = BTreeMap::new();
        buses.insert(
            Self::MASTER.to_string(),
            Bus {
                parent: None,
                ..Default::default()
            },
        );
        Self { buses }
    }

    /// Add or replace a bus.
    pub fn insert(&mut self, name: impl Into<String>, bus: Bus) {
        self.buses.insert(name.into(), bus);
    }

    /// A bus by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Bus> {
        self.buses.get(name)
    }

    /// A bus by name, mutably.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Bus> {
        self.buses.get_mut(name)
    }

    /// Every bus name, sorted.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.buses.keys().map(String::as_str).collect()
    }

    /// Set a bus's volume. Returns whether the bus exists.
    pub fn set_volume(&mut self, name: &str, volume: f32) -> bool {
        match self.buses.get_mut(name) {
            // Clamped at zero: a negative gain inverts the waveform's phase,
            // which is inaudible alone and cancels the sound when it is mixed
            // with anything correlated. A slider dragged past zero must be
            // silence, not a phase bug.
            Some(bus) => {
                bus.volume = volume.max(0.0);
                true
            }
            None => false,
        }
    }

    /// Mute or unmute. Returns whether the bus exists.
    pub fn set_muted(&mut self, name: &str, muted: bool) -> bool {
        match self.buses.get_mut(name) {
            Some(bus) => {
                bus.muted = muted;
                true
            }
            None => false,
        }
    }

    /// Solo or unsolo. Returns whether the bus exists.
    pub fn set_soloed(&mut self, name: &str, soloed: bool) -> bool {
        match self.buses.get_mut(name) {
            Some(bus) => {
                bus.soloed = soloed;
                true
            }
            None => false,
        }
    }

    /// Whether any bus is soloed.
    #[must_use]
    pub fn any_soloed(&self) -> bool {
        self.buses.values().any(|bus| bus.soloed)
    }

    /// The gain a sound on `name` actually plays at.
    ///
    /// Multiplies down the parent chain, applies mute, and applies the solo
    /// rule. Returns 0 for a bus that does not exist: a sound routed to a typo
    /// should be silent and findable, not full volume on master.
    #[must_use]
    pub fn gain(&self, name: &str) -> f32 {
        if !self.buses.contains_key(name) {
            return 0.0;
        }
        let soloing = self.any_soloed();
        // A soloed *ancestor* keeps its children audible: soloing Music must
        // not silence a sub-bus of Music, or solo is useless on any tree deeper
        // than one level.
        let mut on_solo_path = false;
        let mut gain = 1.0f32;
        let mut current = Some(name.to_string());
        // Bounded: a cycle in the parent chain would otherwise hang the mixer,
        // and a cycle is reachable by editing a settings file.
        for _ in 0..32 {
            let Some(key) = current else { break };
            let Some(bus) = self.buses.get(&key) else { break };
            if bus.muted {
                return 0.0;
            }
            if bus.soloed {
                on_solo_path = true;
            }
            gain *= bus.volume;
            current = bus.parent.clone();
        }
        if soloing && !on_solo_path {
            return 0.0;
        }
        gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_mixer_has_the_buses_a_game_ships_with() {
        let mixer = Mixer::default();
        for name in [
            Mixer::MASTER,
            Mixer::MUSIC,
            Mixer::SFX,
            Mixer::DIALOGUE,
            Mixer::UI,
        ] {
            assert!(mixer.get(name).is_some(), "{name} is missing");
            assert_eq!(mixer.gain(name), 1.0, "{name} starts at unity");
        }
    }

    /// **Master volume affects every bus.** That is the point of a tree.
    #[test]
    fn master_volume_multiplies_down_the_chain() {
        let mut mixer = Mixer::default();
        assert!(mixer.set_volume(Mixer::MASTER, 0.5));
        assert!(mixer.set_volume(Mixer::MUSIC, 0.5));
        assert_eq!(mixer.gain(Mixer::MUSIC), 0.25);
        assert_eq!(mixer.gain(Mixer::SFX), 0.5, "sfx is only affected by master");
    }

    /// Quieter music does not mean quieter dialogue.
    #[test]
    fn buses_are_independent_of_their_siblings() {
        let mut mixer = Mixer::default();
        mixer.set_volume(Mixer::MUSIC, 0.0);
        assert_eq!(mixer.gain(Mixer::MUSIC), 0.0);
        assert_eq!(mixer.gain(Mixer::DIALOGUE), 1.0);
    }

    #[test]
    fn muting_a_parent_silences_its_children() {
        let mut mixer = Mixer::default();
        mixer.set_muted(Mixer::MASTER, true);
        assert_eq!(mixer.gain(Mixer::MUSIC), 0.0);
        assert_eq!(mixer.gain(Mixer::UI), 0.0);
    }

    /// **Solo silences everything not soloed.**
    ///
    /// This is why the graph is resolved here rather than pushed into per-track
    /// volumes: solo is a property of the mixer, not of a bus, so expressing it
    /// downstream means recomputing every track whenever any bus changes.
    #[test]
    fn soloing_one_bus_silences_the_others() {
        let mut mixer = Mixer::default();
        assert!(mixer.set_soloed(Mixer::MUSIC, true));
        assert_eq!(mixer.gain(Mixer::MUSIC), 1.0);
        assert_eq!(mixer.gain(Mixer::SFX), 0.0);
        assert_eq!(mixer.gain(Mixer::DIALOGUE), 0.0);

        mixer.set_soloed(Mixer::MUSIC, false);
        assert_eq!(mixer.gain(Mixer::SFX), 1.0, "un-soloing restores everything");
    }

    /// A soloed ancestor keeps its children audible.
    ///
    /// Otherwise solo is useless on any tree deeper than one level: soloing
    /// Music would silence Music's own sub-buses.
    #[test]
    fn soloing_a_parent_keeps_its_children() {
        let mut mixer = Mixer::default();
        mixer.insert(
            "ambience",
            Bus {
                parent: Some(Mixer::MUSIC.to_string()),
                ..Default::default()
            },
        );
        mixer.set_soloed(Mixer::MUSIC, true);
        assert_eq!(mixer.gain("ambience"), 1.0);
        assert_eq!(mixer.gain(Mixer::SFX), 0.0);
    }

    /// Muting beats soloing on the same bus.
    ///
    /// A muted bus a person also soloed is still muted — mute is the explicit
    /// "silence this" and solo is "silence the others".
    #[test]
    fn mute_wins_over_solo() {
        let mut mixer = Mixer::default();
        mixer.set_soloed(Mixer::MUSIC, true);
        mixer.set_muted(Mixer::MUSIC, true);
        assert_eq!(mixer.gain(Mixer::MUSIC), 0.0);
    }

    /// **A negative volume is silence, not a phase inversion.**
    ///
    /// A negative gain flips the waveform, which is inaudible alone and cancels
    /// the sound when it mixes with anything correlated — a bug that shows up
    /// only when two things play at once.
    #[test]
    fn a_negative_volume_clamps_to_silence() {
        let mut mixer = Mixer::default();
        mixer.set_volume(Mixer::SFX, -2.0);
        assert_eq!(mixer.gain(Mixer::SFX), 0.0);
    }

    /// A sound routed to a typo is silent and findable, not loud on master.
    #[test]
    fn an_unknown_bus_is_silent() {
        let mixer = Mixer::default();
        assert_eq!(mixer.gain("musci"), 0.0);
        assert!(!Mixer::default().set_volume("musci", 0.5));
    }

    /// A cycle in the parent chain does not hang the mixer.
    ///
    /// Reachable by editing a settings file, so it must terminate rather than
    /// spin.
    #[test]
    fn a_parent_cycle_terminates() {
        let mut mixer = Mixer::empty();
        mixer.insert(
            "a",
            Bus {
                parent: Some("b".into()),
                ..Default::default()
            },
        );
        mixer.insert(
            "b",
            Bus {
                parent: Some("a".into()),
                ..Default::default()
            },
        );
        let _ = mixer.gain("a"); // must return rather than spin
    }

    #[test]
    fn gain_above_unity_is_allowed() {
        // Boosting a quiet bus is legitimate; only negatives are refused.
        let mut mixer = Mixer::default();
        mixer.set_volume(Mixer::DIALOGUE, 1.5);
        assert_eq!(mixer.gain(Mixer::DIALOGUE), 1.5);
    }
}
