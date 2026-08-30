//! Phase 16-F: what a script package is allowed to ask the engine for.
//!
//! # Why a manifest and not a trust level
//!
//! The threat model in the plan distinguishes *project scripts* — written
//! by whoever is building the game, and trusted about as far as the rest
//! of the project is — from a future *mod tier*, which is not trusted at
//! all. A single "is this trusted" bit answers that badly: the interesting
//! cases are in between. A mod that may play a sound and emit an event but
//! may not despawn things is a reasonable thing to want, and a boolean
//! cannot express it.
//!
//! # Where it is enforced
//!
//! At the **command boundary**, once, in the host — not in the bindings.
//! Enforcing per binding means every new host function is a new place to
//! remember, and a forgotten one is a hole. Every effect a script can have
//! on the world is a [`ScriptCommand`](crate::command::ScriptCommand), so
//! checking there is both exhaustive and impossible to forget: a new
//! command variant does not compile until it says which capability it
//! needs.
//!
//! Reads are deliberately not gated. A script that can see the world but
//! change nothing is the safe end of the range, and gating reads would
//! mean a capability check on the hottest path in the phase.

use std::fmt;

/// A set of engine capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Capabilities(u32);

impl Capabilities {
    /// Nothing at all. A script with this can compute and log nothing.
    pub const NONE: Self = Self(0);

    /// How many capability bits are defined.
    ///
    /// Named so [`Self::PROJECT`] and the test that checks it cannot drift
    /// apart from the list above without somebody noticing.
    pub const BIT_COUNT: u32 = 10;

    /// Write fields of a component an entity already has.
    pub const WRITE_FIELDS: Self = Self(1 << 0);
    /// Attach a component.
    pub const ADD_COMPONENT: Self = Self(1 << 1);
    /// Detach a component.
    pub const REMOVE_COMPONENT: Self = Self(1 << 2);
    /// Create entities.
    pub const SPAWN: Self = Self(1 << 3);
    /// Destroy entities.
    pub const DESPAWN: Self = Self(1 << 4);
    /// Push physics bodies.
    pub const PHYSICS: Self = Self(1 << 5);
    /// Start sounds.
    pub const AUDIO: Self = Self(1 << 6);
    /// Send game events.
    pub const EVENTS: Self = Self(1 << 7);
    /// Write to the output log.
    pub const LOG: Self = Self(1 << 8);
    /// Drive an authored `.somui` document.
    ///
    /// Its own capability rather than `WRITE_FIELDS`, because the authority is
    /// different in kind: a HUD is what the player reads, and a script that may
    /// set an entity's health should not automatically be able to rewrite the
    /// number shown for it. MORROWIND-M2.
    pub const UI: Self = Self(1 << 9);

    /// What a project's own scripts get: everything.
    ///
    /// Generous on purpose. These are written by the same people as the
    /// rest of the game, and a capability system that made ordinary
    /// gameplay work annoying would be turned off rather than tuned.
    pub const PROJECT: Self = Self(0x3FF);

    /// What an untrusted package gets by default: nearly nothing.
    ///
    /// It can change fields on things, log, and emit events. It cannot
    /// create or destroy anything, cannot restructure an entity, and
    /// cannot reach physics or audio. Widening this is a decision someone
    /// makes per package, in a manifest, on purpose.
    pub const SANDBOXED: Self = Self(Self::WRITE_FIELDS.0 | Self::EVENTS.0 | Self::LOG.0);

    /// Build a set from parts.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Remove capabilities.
    #[must_use]
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Whether every capability in `needed` is granted.
    ///
    /// [`Self::NONE`] is contained by everything, which is what makes a
    /// command that needs no capability always allowed.
    #[must_use]
    pub const fn allows(self, needed: Self) -> bool {
        self.0 & needed.0 == needed.0
    }

    /// The raw bits, for a manifest file.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Rebuild from a manifest file. Unknown bits are dropped rather than
    /// trusted — a manifest written by a newer engine must not grant a
    /// capability this one cannot enforce.
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits & Self::PROJECT.0)
    }

    /// The name of a single capability, for a diagnostic.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::WRITE_FIELDS => "write component fields",
            Self::ADD_COMPONENT => "add components",
            Self::REMOVE_COMPONENT => "remove components",
            Self::SPAWN => "spawn entities",
            Self::DESPAWN => "despawn entities",
            Self::PHYSICS => "apply forces",
            Self::AUDIO => "play audio",
            Self::EVENTS => "emit events",
            Self::LOG => "write to the log",
            Self::UI => "drive authored UI",
            Self::NONE => "nothing",
            _ => "several capabilities",
        }
    }
}

impl fmt::Display for Capabilities {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if *self == Self::NONE {
            return f.write_str("none");
        }
        let mut first = true;
        for bit in 0..9 {
            let single = Self(1 << bit);
            if self.allows(single) {
                if !first {
                    f.write_str(", ")?;
                }
                f.write_str(single.name())?;
                first = false;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_script_may_do_everything_a_command_can_express() {
        // The bound is `BIT_COUNT` and not a literal on purpose: written as
        // `0..9` this test kept passing when a tenth capability was added and
        // `PROJECT` was not widened to include it, which is exactly the bug it
        // exists to catch.
        for bit in 0..Capabilities::BIT_COUNT {
            let capability = Capabilities(1 << bit);
            assert!(
                Capabilities::PROJECT.allows(capability),
                "PROJECT does not grant bit {bit}"
            );
            assert_ne!(
                capability.name(),
                "several capabilities",
                "bit {bit} has no name"
            );
        }
    }

    #[test]
    fn the_sandboxed_default_cannot_create_or_destroy_anything() {
        let caps = Capabilities::SANDBOXED;
        assert!(caps.allows(Capabilities::WRITE_FIELDS));
        assert!(caps.allows(Capabilities::LOG));
        assert!(caps.allows(Capabilities::EVENTS));
        for denied in [
            Capabilities::SPAWN,
            Capabilities::DESPAWN,
            Capabilities::ADD_COMPONENT,
            Capabilities::REMOVE_COMPONENT,
            Capabilities::PHYSICS,
            Capabilities::AUDIO,
        ] {
            assert!(!caps.allows(denied), "{} must be denied", denied.name());
        }
    }

    #[test]
    fn nothing_is_always_allowed() {
        assert!(Capabilities::NONE.allows(Capabilities::NONE));
        assert!(Capabilities::SANDBOXED.allows(Capabilities::NONE));
    }

    #[test]
    fn a_manifest_cannot_grant_a_capability_this_build_does_not_know() {
        // A newer engine's manifest, with a bit we have never heard of.
        let forged = Capabilities::from_bits(0xFFFF_FFFF);
        assert_eq!(
            forged,
            Capabilities::PROJECT,
            "unknown bits are dropped, not trusted"
        );
    }

    #[test]
    fn union_and_without_compose() {
        let caps = Capabilities::SANDBOXED.union(Capabilities::AUDIO);
        assert!(caps.allows(Capabilities::AUDIO));
        assert!(
            !caps
                .without(Capabilities::AUDIO)
                .allows(Capabilities::AUDIO)
        );
        assert!(
            caps.without(Capabilities::AUDIO).allows(Capabilities::LOG),
            "removing one must not remove the rest"
        );
    }

    #[test]
    fn the_display_form_names_what_is_granted() {
        assert_eq!(Capabilities::NONE.to_string(), "none");
        let text = Capabilities::SANDBOXED.to_string();
        assert!(text.contains("write component fields"), "{text}");
        assert!(text.contains("emit events"), "{text}");
        assert!(!text.contains("despawn"), "{text}");
    }
}
