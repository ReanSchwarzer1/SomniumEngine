//! The states a script attachment moves through, and the transitions that
//! are legal between them.
//!
//! ```text
//! Loaded → Initialized → Started → Enabled ⇄ Disabled → Destroyed
//! ```
//!
//! # Why this is a type and not a `bool` or three
//!
//! Every engine that grew scripting incrementally ended up with some
//! combination of `initialized`, `started`, `enabled` and `alive` flags,
//! and then with bugs where two of them disagreed — an instance that was
//! enabled but never started, a destroy that ran twice, an `on_disable`
//! that fired for something that had never been enabled. Making the state
//! one value with a transition table makes those unrepresentable rather
//! than merely unlikely.
//!
//! # The one-way door
//!
//! [`LifecycleState::Destroyed`] is terminal. A destroyed instance is
//! never revived; the attachment is rebuilt from authored data instead,
//! which is also what a hot reload does. That is what makes teardown
//! provable: `live_instances` going back to zero means every VM reference
//! was dropped, not that a flag was cleared.

use crate::backend::Callback;

/// Where one attachment is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LifecycleState {
    /// The VM object exists; `onInit` has not run.
    Loaded,
    /// `onInit` has run. Peers may still be initialising.
    Initialized,
    /// `onStart` has run. Every instance that existed at the start of this
    /// frame is initialised.
    Started,
    /// Receiving update phases.
    Enabled,
    /// Switched off, by the author or by quarantine. Still alive, still
    /// holding its state, not receiving update phases.
    Disabled,
    /// Torn down. Terminal.
    Destroyed,
}

impl LifecycleState {
    /// The callback that moves an instance *out* of this state along the
    /// happy path, if there is one.
    ///
    /// `Enabled` and `Disabled` have no single answer — which way they go
    /// depends on the authored `enabled` flag — so they return `None`.
    #[must_use]
    pub const fn advancing_callback(self) -> Option<Callback> {
        match self {
            Self::Loaded => Some(Callback::Init),
            Self::Initialized => Some(Callback::Start),
            Self::Started | Self::Enabled | Self::Disabled | Self::Destroyed => None,
        }
    }

    /// Whether a move to `next` is legal.
    #[must_use]
    pub const fn can_advance_to(self, next: Self) -> bool {
        // Terminal first: a destroyed instance goes nowhere, not even to
        // `Destroyed` again, so this arm has to outrank the teardown one
        // below it.
        if matches!(self, Self::Destroyed) {
            return false;
        }
        matches!(
            (self, next),
            // Teardown is reachable from anywhere alive, which is what
            // makes an entity that despawns mid-frame expressible.
            (_, Self::Destroyed)
                | (Self::Loaded, Self::Initialized)
                | (Self::Initialized, Self::Started)
                | (Self::Started, Self::Enabled | Self::Disabled)
                | (Self::Enabled, Self::Disabled)
                | (Self::Disabled, Self::Enabled)
        )
    }

    /// Whether this state receives `onUpdate`, `onFixedUpdate` and
    /// `onEvent`.
    #[must_use]
    pub const fn receives_updates(self) -> bool {
        matches!(self, Self::Enabled)
    }

    /// Whether the VM object still exists.
    #[must_use]
    pub const fn is_live(self) -> bool {
        !matches!(self, Self::Destroyed)
    }

    /// Whether the instance has been through `onStart`.
    #[must_use]
    pub const fn has_started(self) -> bool {
        matches!(self, Self::Started | Self::Enabled | Self::Disabled)
    }

    /// Short name, for logs and the editor's status column.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Loaded => "loaded",
            Self::Initialized => "initialized",
            Self::Started => "started",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::Destroyed => "destroyed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [LifecycleState; 6] = [
        LifecycleState::Loaded,
        LifecycleState::Initialized,
        LifecycleState::Started,
        LifecycleState::Enabled,
        LifecycleState::Disabled,
        LifecycleState::Destroyed,
    ];

    #[test]
    fn the_happy_path_walks_the_documented_diagram() {
        let mut state = LifecycleState::Loaded;
        for next in [
            LifecycleState::Initialized,
            LifecycleState::Started,
            LifecycleState::Enabled,
            LifecycleState::Disabled,
            LifecycleState::Destroyed,
        ] {
            assert!(state.can_advance_to(next), "{state:?} → {next:?}");
            state = next;
        }
    }

    #[test]
    fn destroyed_is_terminal() {
        for next in ALL {
            assert!(
                !LifecycleState::Destroyed.can_advance_to(next),
                "a destroyed instance must never be revived, not even into {next:?}"
            );
        }
    }

    #[test]
    fn teardown_is_reachable_from_every_live_state() {
        for state in ALL {
            if state == LifecycleState::Destroyed {
                continue;
            }
            assert!(
                state.can_advance_to(LifecycleState::Destroyed),
                "{state:?} must be able to tear down — an entity may despawn at any moment"
            );
        }
    }

    #[test]
    fn a_stage_cannot_be_skipped() {
        assert!(!LifecycleState::Loaded.can_advance_to(LifecycleState::Started));
        assert!(!LifecycleState::Loaded.can_advance_to(LifecycleState::Enabled));
        assert!(!LifecycleState::Initialized.can_advance_to(LifecycleState::Enabled));
    }

    #[test]
    fn enable_and_disable_are_the_only_reversible_pair() {
        assert!(LifecycleState::Enabled.can_advance_to(LifecycleState::Disabled));
        assert!(LifecycleState::Disabled.can_advance_to(LifecycleState::Enabled));
        assert!(!LifecycleState::Started.can_advance_to(LifecycleState::Initialized));
        assert!(!LifecycleState::Initialized.can_advance_to(LifecycleState::Loaded));
    }

    #[test]
    fn only_enabled_receives_update_phases() {
        for state in ALL {
            assert_eq!(
                state.receives_updates(),
                state == LifecycleState::Enabled,
                "{state:?}"
            );
        }
    }

    #[test]
    fn the_advancing_callback_matches_the_state_it_leaves() {
        assert_eq!(
            LifecycleState::Loaded.advancing_callback(),
            Some(Callback::Init)
        );
        assert_eq!(
            LifecycleState::Initialized.advancing_callback(),
            Some(Callback::Start)
        );
        assert_eq!(LifecycleState::Started.advancing_callback(), None);
        assert_eq!(LifecycleState::Enabled.advancing_callback(), None);
    }
}
