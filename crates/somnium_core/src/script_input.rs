//! Phase 16-C: the input a script sees, and the world it is allowed to
//! change while Play is running.
//!
//! # Key numbers are ASCII where ASCII exists
//!
//! [`InputSnapshot`] carries `u32` key codes that are explicitly *not*
//! `winit::keyboard::KeyCode` — the whole point of the neutral crate is
//! that it does not know what a window is. Something has to choose the
//! numbering, and this is it.
//!
//! Letters and digits are their uppercase ASCII values, so a script can
//! write `ctx.input:isKeyDown(string.byte("W"))` and be right. Named keys
//! start at 256, out of ASCII's way, and are listed in
//! `docs/editor/scripting.md` — the alternative, casting `KeyCode`, would
//! bind the script-facing numbering to a `#[non_exhaustive]` upstream enum
//! that is free to renumber itself in a patch release.

use std::collections::{BTreeMap, BTreeSet};

use somnium_ecs::reflect::TypeRegistry;
use somnium_ecs::{Entity, PersistentId, ReflectObject, StableId, World};
use somnium_script::snapshot::InputSnapshot;
use winit::keyboard::KeyCode;

use crate::event::{EngineEvent, InputState};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Key numbering
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Where the named (non-ASCII) key codes start.
pub const KEY_NAMED_BASE: u32 = 256;

/// Space.
pub const KEY_SPACE: u32 = 32;
/// Escape.
pub const KEY_ESCAPE: u32 = KEY_NAMED_BASE;
/// Enter / Return.
pub const KEY_ENTER: u32 = KEY_NAMED_BASE + 1;
/// Tab.
pub const KEY_TAB: u32 = KEY_NAMED_BASE + 2;
/// Backspace.
pub const KEY_BACKSPACE: u32 = KEY_NAMED_BASE + 3;
/// Left arrow.
pub const KEY_LEFT: u32 = KEY_NAMED_BASE + 4;
/// Right arrow.
pub const KEY_RIGHT: u32 = KEY_NAMED_BASE + 5;
/// Up arrow.
pub const KEY_UP: u32 = KEY_NAMED_BASE + 6;
/// Down arrow.
pub const KEY_DOWN: u32 = KEY_NAMED_BASE + 7;
/// Either shift.
pub const KEY_SHIFT: u32 = KEY_NAMED_BASE + 8;
/// Either control.
pub const KEY_CONTROL: u32 = KEY_NAMED_BASE + 9;
/// Either alt.
pub const KEY_ALT: u32 = KEY_NAMED_BASE + 10;

/// The script-facing number for a physical key, if it has one.
///
/// Keys with no number are simply invisible to scripts; that is better
/// than inventing values that would collide with a later addition.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn script_key_code(key: KeyCode) -> Option<u32> {
    let code = match key {
        KeyCode::KeyA => u32::from(b'A'),
        KeyCode::KeyB => u32::from(b'B'),
        KeyCode::KeyC => u32::from(b'C'),
        KeyCode::KeyD => u32::from(b'D'),
        KeyCode::KeyE => u32::from(b'E'),
        KeyCode::KeyF => u32::from(b'F'),
        KeyCode::KeyG => u32::from(b'G'),
        KeyCode::KeyH => u32::from(b'H'),
        KeyCode::KeyI => u32::from(b'I'),
        KeyCode::KeyJ => u32::from(b'J'),
        KeyCode::KeyK => u32::from(b'K'),
        KeyCode::KeyL => u32::from(b'L'),
        KeyCode::KeyM => u32::from(b'M'),
        KeyCode::KeyN => u32::from(b'N'),
        KeyCode::KeyO => u32::from(b'O'),
        KeyCode::KeyP => u32::from(b'P'),
        KeyCode::KeyQ => u32::from(b'Q'),
        KeyCode::KeyR => u32::from(b'R'),
        KeyCode::KeyS => u32::from(b'S'),
        KeyCode::KeyT => u32::from(b'T'),
        KeyCode::KeyU => u32::from(b'U'),
        KeyCode::KeyV => u32::from(b'V'),
        KeyCode::KeyW => u32::from(b'W'),
        KeyCode::KeyX => u32::from(b'X'),
        KeyCode::KeyY => u32::from(b'Y'),
        KeyCode::KeyZ => u32::from(b'Z'),
        KeyCode::Digit0 => u32::from(b'0'),
        KeyCode::Digit1 => u32::from(b'1'),
        KeyCode::Digit2 => u32::from(b'2'),
        KeyCode::Digit3 => u32::from(b'3'),
        KeyCode::Digit4 => u32::from(b'4'),
        KeyCode::Digit5 => u32::from(b'5'),
        KeyCode::Digit6 => u32::from(b'6'),
        KeyCode::Digit7 => u32::from(b'7'),
        KeyCode::Digit8 => u32::from(b'8'),
        KeyCode::Digit9 => u32::from(b'9'),
        KeyCode::Space => KEY_SPACE,
        KeyCode::Escape => KEY_ESCAPE,
        KeyCode::Enter | KeyCode::NumpadEnter => KEY_ENTER,
        KeyCode::Tab => KEY_TAB,
        KeyCode::Backspace => KEY_BACKSPACE,
        KeyCode::ArrowLeft => KEY_LEFT,
        KeyCode::ArrowRight => KEY_RIGHT,
        KeyCode::ArrowUp => KEY_UP,
        KeyCode::ArrowDown => KEY_DOWN,
        KeyCode::ShiftLeft | KeyCode::ShiftRight => KEY_SHIFT,
        KeyCode::ControlLeft | KeyCode::ControlRight => KEY_CONTROL,
        KeyCode::AltLeft | KeyCode::AltRight => KEY_ALT,
        _ => return None,
    };
    Some(code)
}

/// The script-facing number for a mouse button: 0 left, 1 right,
/// 2 middle. Anything else is invisible.
#[must_use]
pub fn script_mouse_button(button: winit::event::MouseButton) -> Option<u8> {
    match button {
        winit::event::MouseButton::Left => Some(0),
        winit::event::MouseButton::Right => Some(1),
        winit::event::MouseButton::Middle => Some(2),
        _ => None,
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// The tracker
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Held keys and buttons, accumulated between fixed steps.
///
/// `BTreeSet` rather than `HashSet` because [`InputSnapshot`] promises a
/// sorted key list — its lookups are binary searches, and iterating it has
/// to be the same on every run.
#[derive(Debug, Default, Clone)]
pub struct ScriptInputTracker {
    down: BTreeSet<u32>,
    pressed: BTreeSet<u32>,
    mouse: BTreeSet<u8>,
    delta: [f32; 2],
}

impl ScriptInputTracker {
    /// A tracker with nothing held.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one engine event in.
    pub fn observe(&mut self, event: &EngineEvent) {
        match event {
            EngineEvent::KeyInput { key, state } => {
                if let Some(code) = script_key_code(*key) {
                    match state {
                        InputState::Pressed => {
                            // `pressed` is edge-triggered: the OS repeats a
                            // held key, and a script asking "did this go
                            // down this step" must not get a yes every
                            // frame the key is resting on the desk.
                            if self.down.insert(code) {
                                self.pressed.insert(code);
                            }
                        }
                        InputState::Released => {
                            self.down.remove(&code);
                        }
                    }
                }
            }
            EngineEvent::MouseButton { button, state } => {
                if let Some(index) = script_mouse_button(*button) {
                    match state {
                        InputState::Pressed => {
                            self.mouse.insert(index);
                        }
                        InputState::Released => {
                            self.mouse.remove(&index);
                        }
                    }
                }
            }
            EngineEvent::MouseMotion { delta_x, delta_y } => {
                self.delta[0] += delta_x;
                self.delta[1] += delta_y;
            }
            EngineEvent::WindowFocused(false) => {
                // Losing focus with a key held would otherwise leave it
                // held forever: the release event goes to another window.
                self.down.clear();
                self.mouse.clear();
            }
            _ => {}
        }
    }

    /// What a phase sees.
    #[must_use]
    pub fn snapshot(&self) -> InputSnapshot {
        InputSnapshot {
            keys_down: self.down.iter().copied().collect(),
            keys_pressed: self.pressed.iter().copied().collect(),
            mouse_down: self.mouse.iter().copied().collect(),
            mouse_delta: self.delta,
        }
    }

    /// Clear the edge-triggered half. Called once per fixed step, after
    /// the phase has run.
    pub fn end_step(&mut self) {
        self.pressed.clear();
        self.delta = [0.0, 0.0];
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Play / stop world separation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Every registered component of every entity, as it was when Play was
/// pressed.
///
/// # Why this and not a scene file
///
/// Round-tripping through `.somnium` would be the obvious answer and is
/// the wrong one: loading an entity dump needs GPU-side reconstruction
/// (meshes from `MeshKind`, terrain sidecars, renderer uploads), and none
/// of that is what Stop is for. Stop has to undo what *scripts* did, and
/// scripts can only touch what the [`TypeRegistry`] describes — so
/// capturing exactly that is both sufficient and free of the renderer.
///
/// Entities are keyed by [`PersistentId`], not by handle, because an
/// entity destroyed and restored gets a new index and generation.
#[derive(Debug, Default, Clone)]
pub struct WorldCheckpoint {
    entities: BTreeMap<PersistentId, Vec<(StableId, ReflectObject)>>,
}

impl WorldCheckpoint {
    /// Capture the world.
    ///
    /// Mints a [`PersistentId`] for anything that lacks one, since an
    /// entity with no durable name cannot be restored onto itself.
    #[must_use]
    pub fn capture(world: &mut World, registry: &TypeRegistry) -> Self {
        let all: Vec<Entity> = world.entities().collect();
        for entity in &all {
            let _ = world.ensure_persistent_id(*entity);
        }

        let mut entities = BTreeMap::new();
        for entity in world.entities().collect::<Vec<_>>() {
            let Some(id) = world.persistent_id(entity) else {
                continue;
            };
            let components = registry
                .schemas_on(world, entity)
                .iter()
                .filter_map(|schema| {
                    (schema.snapshot)(world, entity).map(|record| (schema.stable_id, record))
                })
                .collect();
            entities.insert(id, components);
        }
        Self { entities }
    }

    /// Put the world back.
    ///
    /// Three cases, and all three are real: an entity that survived has
    /// its fields written back; an entity a script destroyed is respawned
    /// from its record; an entity a script created is destroyed.
    pub fn restore(&self, world: &mut World, registry: &TypeRegistry) {
        // Anything with no captured record was created during play.
        let intruders: Vec<Entity> = world
            .entities()
            .collect::<Vec<_>>()
            .into_iter()
            .filter(|entity| {
                world
                    .persistent_id(*entity)
                    .is_none_or(|id| !self.entities.contains_key(&id))
            })
            .collect();
        for entity in intruders {
            world.despawn(entity);
        }

        for (id, components) in &self.entities {
            let entity = if let Some(entity) = world.entity_by_persistent_id(*id) {
                entity
            } else {
                // Destroyed during play. Rebuild it at its defaults; the
                // captured values are written over the top below.
                let entity = world.spawn((*id,));
                for (stable, _) in components {
                    if let Some(schema) = registry.by_stable_id(*stable) {
                        let _ = (schema.insert_default)(world, entity);
                    }
                }
                entity
            };
            for (stable, record) in components {
                let Some(schema) = registry.by_stable_id(*stable) else {
                    continue;
                };
                if (schema.apply)(world, entity, record).is_err() {
                    // The component was removed during play; put it back
                    // at its defaults and write the captured values over.
                    if (schema.insert_default)(world, entity).is_ok() {
                        let _ = (schema.apply)(world, entity, record);
                    }
                }
            }
            // A component a script *added* during play is not in the
            // record, and must go.
            let extra: Vec<StableId> = registry
                .schemas_on(world, entity)
                .iter()
                .map(|schema| schema.stable_id)
                .filter(|stable| !components.iter().any(|(captured, _)| captured == stable))
                .collect();
            for stable in extra {
                if let Some(schema) = registry.by_stable_id(stable) {
                    let _ = (schema.remove)(world, entity);
                }
            }
        }
    }

    /// How many entities were captured.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Whether nothing was captured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect_registry::component_registry;
    use crate::{Name, Transform};

    #[test]
    fn letters_are_their_ascii_values_so_string_byte_works() {
        assert_eq!(script_key_code(KeyCode::KeyW), Some(u32::from(b'W')));
        assert_eq!(script_key_code(KeyCode::Digit3), Some(u32::from(b'3')));
        assert_eq!(script_key_code(KeyCode::Space), Some(KEY_SPACE));
    }

    #[test]
    fn named_keys_stay_clear_of_ascii() {
        for code in [
            KEY_ESCAPE,
            KEY_ENTER,
            KEY_TAB,
            KEY_LEFT,
            KEY_RIGHT,
            KEY_UP,
            KEY_DOWN,
            KEY_SHIFT,
            KEY_CONTROL,
            KEY_ALT,
        ] {
            assert!(code >= KEY_NAMED_BASE, "{code} collides with ASCII");
        }
    }

    #[test]
    fn a_held_key_is_pressed_exactly_once() {
        let mut tracker = ScriptInputTracker::new();
        let down = EngineEvent::KeyInput {
            key: KeyCode::KeyW,
            state: InputState::Pressed,
        };
        tracker.observe(&down);
        assert!(tracker.snapshot().is_key_pressed(u32::from(b'W')));

        tracker.end_step();
        // The OS repeats the key while it is held.
        tracker.observe(&down);
        let snapshot = tracker.snapshot();
        assert!(snapshot.is_key_down(u32::from(b'W')), "still held");
        assert!(
            !snapshot.is_key_pressed(u32::from(b'W')),
            "but it did not go down again"
        );
    }

    #[test]
    fn releasing_clears_the_key_and_losing_focus_clears_everything() {
        let mut tracker = ScriptInputTracker::new();
        tracker.observe(&EngineEvent::KeyInput {
            key: KeyCode::KeyA,
            state: InputState::Pressed,
        });
        tracker.observe(&EngineEvent::MouseButton {
            button: winit::event::MouseButton::Left,
            state: InputState::Pressed,
        });
        assert!(tracker.snapshot().is_mouse_down(0));

        tracker.observe(&EngineEvent::KeyInput {
            key: KeyCode::KeyA,
            state: InputState::Released,
        });
        assert!(!tracker.snapshot().is_key_down(u32::from(b'A')));

        tracker.observe(&EngineEvent::WindowFocused(false));
        assert!(
            !tracker.snapshot().is_mouse_down(0),
            "a key held when focus is lost never gets its release event"
        );
    }

    #[test]
    fn mouse_delta_accumulates_within_a_step_and_resets_after_it() {
        let mut tracker = ScriptInputTracker::new();
        tracker.observe(&EngineEvent::MouseMotion {
            delta_x: 3.0,
            delta_y: -1.0,
        });
        tracker.observe(&EngineEvent::MouseMotion {
            delta_x: 1.0,
            delta_y: 0.5,
        });
        let delta = tracker.snapshot().mouse_delta;
        assert!((delta[0] - 4.0).abs() < 1.0e-6 && (delta[1] + 0.5).abs() < 1.0e-6, "{delta:?}");
        tracker.end_step();
        let cleared = tracker.snapshot().mouse_delta;
        assert!(cleared.iter().all(|d| d.abs() < f32::EPSILON), "{cleared:?}");
    }

    #[test]
    fn stop_puts_back_a_field_that_play_changed() {
        let registry = component_registry();
        let mut world = World::new();
        let entity = world.spawn((Transform::default(), Name::new("Original")));
        let checkpoint = WorldCheckpoint::capture(&mut world, &registry);

        world.get_mut::<Transform>(entity).unwrap().translation = glam::Vec3::new(9.0, 9.0, 9.0);
        checkpoint.restore(&mut world, &registry);

        assert!(
            world.get::<Transform>(entity).unwrap().translation.length() < 1.0e-6,
            "Stop must restore the authored world exactly"
        );
    }

    #[test]
    fn stop_destroys_what_play_created_and_restores_what_it_destroyed() {
        let registry = component_registry();
        let mut world = World::new();
        let keep = world.spawn((Transform::default(), Name::new("Keep")));
        let doomed = world.spawn((Transform::from_translation(glam::Vec3::X), Name::new("Doomed")));
        let checkpoint = WorldCheckpoint::capture(&mut world, &registry);
        assert_eq!(checkpoint.len(), 2);

        world.despawn(doomed);
        world.spawn((Transform::default(), Name::new("Spawned by a script")));
        assert_eq!(world.entities().count(), 2);

        checkpoint.restore(&mut world, &registry);
        assert_eq!(world.entities().count(), 2, "the intruder is gone");
        assert!(world.is_alive(keep));

        let names: Vec<String> = world
            .entities()
            .filter_map(|entity| world.get::<Name>(entity).map(|n| n.as_str().to_string()))
            .collect();
        assert!(names.contains(&"Keep".to_string()));
        assert!(
            names.contains(&"Doomed".to_string()),
            "an entity a script destroyed comes back: {names:?}"
        );
    }

    #[test]
    fn stop_removes_a_component_that_play_added() {
        let registry = component_registry();
        let mut world = World::new();
        let entity = world.spawn((Transform::default(),));
        let checkpoint = WorldCheckpoint::capture(&mut world, &registry);

        world.insert_component(entity, Name::new("Added by a script")).unwrap();
        assert!(world.get::<Name>(entity).is_some());

        checkpoint.restore(&mut world, &registry);
        assert!(
            world.get::<Name>(entity).is_none(),
            "a component a script attached during play must not survive Stop"
        );
    }
}
