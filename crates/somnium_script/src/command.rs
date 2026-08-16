//! What a script asks the engine to do.
//!
//! A script never receives `&mut World`, a component pointer, or any
//! other borrow into engine storage. It reads a snapshot and emits
//! commands, which the engine validates and applies at a documented phase
//! boundary. Three things follow, and all three are load-bearing:
//!
//! * **structural changes cannot invalidate a live archetype iteration**,
//!   because nothing structural happens while scripts are running;
//! * **a stale entity handle is a typed error**, not a panic, because
//!   every target is re-validated at apply time;
//! * **the design survives parallel script workers**, because commands
//!   are already a merge point with a deterministic order.
//!
//! # Visibility rule
//!
//! One script's writes are not visible to another until the commit point.
//! This is a real constraint on how gameplay code is written and it is
//! documented for authors rather than hidden — the alternative is
//! immediate visibility, which makes execution order load-bearing in a
//! way no author can reason about.
//!
//! # Spawn results
//!
//! A spawn cannot return an entity immediately, because the entity does
//! not exist until commit. The script gets a [`SpawnToken`] straight
//! away, and the *next* snapshot carries the token-to-entity mapping.

use somnium_ecs::{Entity, ReflectObject, StableId};

use crate::ids::ScriptAssetId;
use crate::order::OrderKey;

/// A placeholder for an entity that does not exist yet.
///
/// Handed to the script when it queues a spawn; resolved to a real
/// [`Entity`] in the next snapshot's spawn results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpawnToken(pub u32);

/// How a force is applied to a physics body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForceMode {
    /// Continuous force, scaled by the step. For sustained thrust.
    Force,
    /// Instantaneous change in momentum. For hits and jumps.
    Impulse,
}

/// Severity of a script log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Author debugging.
    Debug,
    /// Ordinary progress.
    Info,
    /// Something suspicious that did not stop anything.
    Warn,
    /// Something that failed.
    Error,
}

/// One thing a script wants done.
///
/// Component reads and writes go through [`StableId`] and a reflected
/// record rather than through per-component command variants. That is
/// what stops this enum growing a case for every component the engine
/// will ever have — the failure mode visible in engines that hand-wrote
/// one binding file per subsystem.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptCommand {
    /// Write some fields of a component the entity already has.
    SetFields {
        /// Target entity.
        entity: Entity,
        /// Which component.
        component: StableId,
        /// Fields to write; absent fields keep their value.
        fields: ReflectObject,
    },
    /// Attach a component, with any fields not given left at default.
    AddComponent {
        /// Target entity.
        entity: Entity,
        /// Which component.
        component: StableId,
        /// Initial field values.
        fields: ReflectObject,
    },
    /// Detach a component.
    RemoveComponent {
        /// Target entity.
        entity: Entity,
        /// Which component.
        component: StableId,
    },
    /// Create an entity. The token is reported back in the next snapshot.
    Spawn {
        /// Placeholder the script can hold on to.
        token: SpawnToken,
        /// Components to create it with, in stable-id order.
        components: Vec<(StableId, ReflectObject)>,
    },
    /// Destroy an entity. Deferred to the safe point, so an entity may
    /// despawn itself from inside its own callback.
    Despawn {
        /// Target entity.
        entity: Entity,
    },
    /// Push a physics body.
    ApplyForce {
        /// Target entity.
        entity: Entity,
        /// World-space force or impulse.
        force: [f32; 3],
        /// Which of the two it is.
        mode: ForceMode,
    },
    /// Start a sound.
    PlayAudio {
        /// Sound asset.
        asset: ScriptAssetId,
        /// Linear gain.
        volume: f32,
    },
    /// Send a game event. Delivered to subscribers in the next phase with
    /// a monotonically increasing sequence number.
    EmitEvent {
        /// Event name.
        name: String,
        /// Payload.
        payload: ReflectObject,
    },
    /// Write a line to the output log, tagged with the emitting script.
    Log {
        /// Severity.
        level: LogLevel,
        /// Message text.
        message: String,
    },
}

/// A command together with everything needed to order it.
#[derive(Debug, Clone, PartialEq)]
pub struct QueuedCommand {
    /// Which attachment emitted it.
    pub order: OrderKey,
    /// Emission index within that attachment's callback, so a script's
    /// own commands stay in the order it wrote them.
    pub sequence: u32,
    /// The command.
    pub command: ScriptCommand,
}

/// Commands accumulated during one script phase.
///
/// The buffer is filled in whatever order attachments happen to run and
/// then **sorted** before it is applied, so the applied order depends on
/// authored data alone. Last write to a given field wins, and "last" is
/// well defined because [`OrderKey`] is a total order.
#[derive(Debug, Default)]
pub struct CommandBuffer {
    queued: Vec<QueuedCommand>,
    next_token: u32,
    /// Set while an attachment is emitting, so callers do not have to
    /// pass the order key with every push.
    current: Option<OrderKey>,
    sequence: u32,
    sorted: bool,
}

impl CommandBuffer {
    /// An empty buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin recording for one attachment. Every [`Self::push`] until the
    /// next `begin` is attributed to `order`.
    pub fn begin(&mut self, order: OrderKey) {
        self.current = Some(order);
        self.sequence = 0;
    }

    /// Stop attributing commands to the current attachment.
    ///
    /// Pushing after this is a programmer error and is rejected rather
    /// than silently attributed to whoever ran last.
    pub fn end(&mut self) {
        self.current = None;
    }

    /// Queue a command for the attachment named by the last
    /// [`Self::begin`].
    ///
    /// # Panics
    ///
    /// Panics if no attachment is currently recording. A command with no
    /// owner has no place in the order and no one to blame when it fails.
    pub fn push(&mut self, command: ScriptCommand) {
        let order = self
            .current
            .expect("CommandBuffer::push outside begin/end has no owner to order by");
        self.queued.push(QueuedCommand {
            order,
            sequence: self.sequence,
            command,
        });
        self.sequence += 1;
        self.sorted = false;
    }

    /// Allocate a spawn placeholder.
    pub fn new_spawn_token(&mut self) -> SpawnToken {
        let token = SpawnToken(self.next_token);
        self.next_token += 1;
        token
    }

    /// Sort into apply order and hand back the commands.
    ///
    /// Sorting here rather than at push time keeps emission cheap and
    /// makes the ordering rule one line in one place.
    pub fn drain_sorted(&mut self) -> Vec<QueuedCommand> {
        self.sort();
        self.next_token = 0;
        self.current = None;
        self.sequence = 0;
        self.sorted = false;
        std::mem::take(&mut self.queued)
    }

    /// Sort in place without draining. Mostly useful for inspection.
    pub fn sort(&mut self) {
        if !self.sorted {
            self.queued
                .sort_by(|a, b| (a.order, a.sequence).cmp(&(b.order, b.sequence)));
            self.sorted = true;
        }
    }

    /// Number of queued commands.
    #[must_use]
    pub fn len(&self) -> usize {
        self.queued.len()
    }

    /// Whether nothing is queued.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queued.is_empty()
    }

    /// Discard everything without applying it — the error path when an
    /// attachment faults partway through a callback.
    pub fn clear(&mut self) {
        self.queued.clear();
        self.next_token = 0;
        self.current = None;
        self.sequence = 0;
        self.sorted = false;
    }

    /// Drop every command emitted by one attachment, keeping the rest.
    ///
    /// This is error quarantine: an attachment that faults has its whole
    /// batch discarded so the world never sees half of what it intended,
    /// and every other attachment's work still lands.
    pub fn discard_from(&mut self, order: OrderKey) {
        self.queued.retain(|q| q.order != order);
    }

    /// Read the queued commands without draining.
    #[must_use]
    pub fn queued(&self) -> &[QueuedCommand] {
        &self.queued
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_ecs::PersistentId;

    use crate::ids::InstanceUuid;

    fn order(exec: i32, entity: u128, instance: u128) -> OrderKey {
        OrderKey::new(
            exec,
            PersistentId::from_raw(entity),
            InstanceUuid::from_raw(instance),
        )
    }

    fn log(message: &str) -> ScriptCommand {
        ScriptCommand::Log {
            level: LogLevel::Info,
            message: message.to_owned(),
        }
    }

    fn messages(commands: &[QueuedCommand]) -> Vec<String> {
        commands
            .iter()
            .map(|q| match &q.command {
                ScriptCommand::Log { message, .. } => message.clone(),
                other => format!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_scripts_own_commands_keep_their_emission_order() {
        let mut buffer = CommandBuffer::new();
        buffer.begin(order(0, 1, 1));
        buffer.push(log("first"));
        buffer.push(log("second"));
        buffer.push(log("third"));
        buffer.end();

        assert_eq!(
            messages(&buffer.drain_sorted()),
            vec!["first", "second", "third"]
        );
    }

    #[test]
    fn apply_order_follows_the_order_key_not_the_run_order() {
        let mut buffer = CommandBuffer::new();
        // Deliberately run the later attachment first.
        buffer.begin(order(10, 1, 1));
        buffer.push(log("late"));
        buffer.end();
        buffer.begin(order(-1, 9, 9));
        buffer.push(log("early"));
        buffer.end();

        assert_eq!(messages(&buffer.drain_sorted()), vec!["early", "late"]);
    }

    #[test]
    fn shuffling_emission_never_changes_the_applied_order() {
        // The same four attachments, emitted in every rotation, must
        // produce byte-identical apply order.
        let keys = [
            order(0, 3, 1),
            order(0, 1, 7),
            order(-2, 8, 8),
            order(0, 1, 2),
        ];
        let mut expected: Option<Vec<String>> = None;
        for rotation in 0..keys.len() {
            let mut buffer = CommandBuffer::new();
            for offset in 0..keys.len() {
                let key = keys[(rotation + offset) % keys.len()];
                buffer.begin(key);
                buffer.push(log(&format!("{}:{}", key.execution_order, key.instance)));
                buffer.end();
            }
            let applied = messages(&buffer.drain_sorted());
            match &expected {
                None => expected = Some(applied),
                Some(first) => assert_eq!(&applied, first, "rotation {rotation} diverged"),
            }
        }
    }

    #[test]
    fn the_last_write_to_a_field_is_the_one_from_the_later_attachment() {
        let mut buffer = CommandBuffer::new();
        let late = order(5, 1, 1);
        let early = order(1, 1, 1);

        buffer.begin(late);
        buffer.push(log("loser"));
        buffer.end();
        buffer.begin(early);
        buffer.push(log("winner-is-applied-first"));
        buffer.end();

        let applied = buffer.drain_sorted();
        assert_eq!(
            messages(&applied),
            vec!["winner-is-applied-first", "loser"],
            "conflicts resolve by apply order, and apply order is the key"
        );
    }

    #[test]
    fn quarantine_drops_one_attachments_whole_batch() {
        let mut buffer = CommandBuffer::new();
        let good = order(0, 1, 1);
        let bad = order(0, 1, 2);

        buffer.begin(good);
        buffer.push(log("keep-1"));
        buffer.end();
        buffer.begin(bad);
        buffer.push(log("drop-1"));
        buffer.push(log("drop-2"));
        buffer.end();
        buffer.begin(good);
        buffer.push(log("keep-2"));
        buffer.end();

        buffer.discard_from(bad);
        assert_eq!(messages(&buffer.drain_sorted()), vec!["keep-1", "keep-2"]);
    }

    #[test]
    fn spawn_tokens_are_unique_within_a_phase_and_reset_after_it() {
        let mut buffer = CommandBuffer::new();
        let a = buffer.new_spawn_token();
        let b = buffer.new_spawn_token();
        assert_ne!(a, b);

        buffer.begin(order(0, 1, 1));
        buffer.push(ScriptCommand::Spawn {
            token: a,
            components: Vec::new(),
        });
        buffer.end();
        let _ = buffer.drain_sorted();

        assert_eq!(buffer.new_spawn_token(), SpawnToken(0), "tokens reset");
    }

    #[test]
    #[should_panic(expected = "has no owner to order by")]
    fn pushing_without_an_owner_is_rejected() {
        let mut buffer = CommandBuffer::new();
        buffer.push(log("orphan"));
    }

    #[test]
    fn clear_empties_everything() {
        let mut buffer = CommandBuffer::new();
        buffer.begin(order(0, 1, 1));
        buffer.push(log("gone"));
        buffer.end();
        assert!(!buffer.is_empty());
        buffer.clear();
        assert!(buffer.is_empty());
        assert_eq!(buffer.len(), 0);
    }
}
