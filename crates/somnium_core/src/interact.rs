//! Isometric interaction over `somnium.Interactable` (Rimeborn, 19 Oct 2026; contract `docs/editor/interaction.md`).
//!
//! An isometric view has no screen-centre ray, so each frame an [`Interactor`] picks the interactable nearest in front
//! of the player: within its `reach` of the player's eye, inside its `cone` either side of the player's facing, the
//! best of near and straight ahead. A small prompt floats over it, and the interact key ([`interact_map`], `E`,
//! rebindable through `somnium_input::rebind`) uses it.
//!
//! A use runs the interactable's own [`InteractionSession`]: the reach, then the one commit event
//! (`InteractionEvent::Contact`), which the interactor hands back as [`Used`] for the game (and the doors,
//! [`crate::door::Doors`]) to act on. `locked`, or a `requires` world-state condition the game evaluates (Rimeborn:
//! `somnium_story`'s `Story::check`) that is false, refuses it at once, and the prompt says "Locked".
//!
//! [`Interaction`] bundles the interactor, the doors and the prompt for a game to drive in four calls. Nothing here
//! runs unless a game calls it, so TSF's own use of `somnium.Interactable` (first-person hands) is untouched.

use glam::{Mat4, Vec2, Vec3};
use somnium_ecs::{ComponentId, ComponentSet, Entity, World};
use somnium_input::{Action, ActionMap, Binding, ControlPath, InputSystem};

use crate::door::{DoorEvent, Doors};
use crate::interaction::{Interactable, InteractionEvent, InteractionSession};
use crate::{EngineContext, EngineEvent, InputState, SimulationState, WorldTransform};

/// `Interactable::kind` for a door.
pub const KIND_DOOR: u32 = 0;

/// The prompt's verb when an interactable's own prompt is empty, by `kind`.
#[must_use]
pub fn kind_word(kind: u32) -> &'static str {
    match kind {
        0 => "Open",
        1 | 4 => "Take",
        2 => "Inspect",
        _ => "Use",
    }
}

/// Who is using things this frame: their eye (reach is measured from it) and the way they face along the ground
/// (zero: any).
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(missing_docs)] // the type's doc says it
pub struct Actor {
    pub eye: Vec3,
    pub forward: Vec3,
}

/// The thing the actor would use now.
#[derive(Clone, Debug, PartialEq)]
#[allow(missing_docs)] // as the interactable says
pub struct Target {
    pub entity: Entity,
    pub kind: u32,
    /// The anchor in the world, for the prompt.
    pub at: Vec3,
    pub prompt: String,
    pub refused: bool,
}

/// A use: committed at the session's contact, or refused at once (locked).
#[derive(Clone, Debug, PartialEq)]
#[allow(missing_docs)] // as the target said
pub struct Used {
    pub entity: Entity,
    pub kind: u32,
    pub refused: bool,
    /// The interactable's `trigger`: the game's name for what using it means.
    pub trigger: String,
}

/// The interact map's name.
pub const MAP: &str = "interact";
/// The interact map's action.
pub const ACTION: &str = "Interact";

/// The interact key: `E` and the gamepad's west button.
#[must_use]
pub fn interact_map() -> ActionMap {
    ActionMap::new(MAP).action(
        Action::digital(ACTION)
            .bind(Binding::single(ControlPath::keyboard("e")))
            .bind(Binding::single(ControlPath::gamepad("buttonwest"))),
    )
}

/// Nearer than this along the ground the facing no longer matters (standing in a doorway), metres.
const CLOSE: f32 = 0.45;
/// How much better another thing must score before the target changes (no flicker between two doors).
const STICKY: f32 = 0.08;

/// The interactable `actor` would use: within reach of the eye, inside its cone, the best of distance (as a share of
/// reach) and facing. `current` keeps the last target unless another is clearly better. One pass over the
/// interactables' own columns.
#[must_use]
pub fn pick(world: &World, actor: Actor, current: Option<Entity>) -> Option<(Entity, Vec3)> {
    let forward = Vec3::new(actor.forward.x, 0.0, actor.forward.z).normalize_or_zero();
    let id = ComponentId::of::<Interactable>();
    let required = ComponentSet::from_ids(vec![id, ComponentId::of::<WorldTransform>()]);
    let mut best: Option<(Entity, Vec3, f32)> = None;
    let mut kept: Option<(Entity, Vec3, f32)> = None;
    for archetype in world.query_archetypes(&required, &ComponentSet::empty()) {
        let col = archetype.column_index(id).expect("queried for it");
        let world_col = archetype.column_index(ComponentId::of::<WorldTransform>()).expect("queried for it");
        for row in 0..archetype.len() {
            // SAFETY: each column is the one `column_index` found for that component's id; `row` is this archetype's.
            let thing = unsafe { archetype.column(col).get::<Interactable>(row) };
            if !thing.enabled {
                continue;
            }
            let frame = unsafe { archetype.column(world_col).get::<WorldTransform>(row) }.0;
            let at = frame.transform_point3(thing.anchor);
            let to = at - actor.eye;
            if to.length() > thing.reach {
                continue;
            }
            let along = Vec2::new(to.x, to.z);
            let distance = along.length();
            let facing = if distance < CLOSE || forward == Vec3::ZERO {
                1.0
            } else {
                (forward.x * along.x + forward.z * along.y) / distance
            };
            if facing < thing.cone.to_radians().cos() {
                continue;
            }
            let score = to.length() / thing.reach.max(1e-3) + 0.5 * (1.0 - facing);
            let entity = archetype.entities()[row];
            if Some(entity) == current {
                kept = Some((entity, at, score));
            }
            if best.is_none_or(|(_, _, s)| score < s) {
                best = Some((entity, at, score));
            }
        }
    }
    match (kept, best) {
        (Some(kept), Some(best)) if best.2 > kept.2 - STICKY => Some((kept.0, kept.1)),
        (_, best) => best.map(|(entity, at, _)| (entity, at)),
    }
}

/// Picks the target each frame, and turns the interact key into a use through the target's session.
pub struct Interactor {
    input: InputSystem,
    target: Option<Target>,
    session: InteractionSession,
    /// The interactable the session is reaching for.
    reaching: Option<Entity>,
}

impl Default for Interactor {
    fn default() -> Self {
        let mut input = InputSystem::new();
        input.add_map(interact_map());
        Self { input, target: None, session: InteractionSession::default(), reaching: None }
    }
}

impl Interactor {
    /// Feed the interact key from engine events.
    pub fn on_event(&mut self, event: &EngineEvent) {
        let devices = self.input.devices_mut();
        match event {
            EngineEvent::KeyInput { key, state } => {
                if let Some(name) = somnium_input::device::key_name(*key) {
                    devices.set_key(name, *state == InputState::Pressed);
                }
            }
            EngineEvent::MouseButton { button, state } => {
                let name = match button {
                    winit::event::MouseButton::Left => "leftbutton",
                    winit::event::MouseButton::Right => "rightbutton",
                    winit::event::MouseButton::Middle => "middlebutton",
                    _ => return,
                };
                devices.set_mouse_button(name, *state == InputState::Pressed);
            }
            EngineEvent::WindowFocused(false) => devices.release_all(),
            _ => {}
        }
    }

    /// The bindings, to rebind the key.
    pub fn input_mut(&mut self) -> &mut InputSystem {
        &mut self.input
    }

    /// The key the prompt shows: the first keyboard (else mouse) binding's control, upper-cased.
    #[must_use]
    pub fn key_label(&self) -> String {
        let paths = self
            .input
            .maps()
            .iter()
            .filter(|map| map.name == MAP)
            .filter_map(|map| map.find(ACTION))
            .flat_map(|action| action.bindings.iter().flat_map(Binding::paths))
            .collect::<Vec<_>>();
        let pick = |kind| paths.iter().find(|p| p.device == kind);
        pick(somnium_input::DeviceKind::Keyboard)
            .or_else(|| pick(somnium_input::DeviceKind::Mouse))
            .map_or_else(String::new, |p| p.control.to_uppercase())
    }

    /// What would be used now.
    #[must_use]
    pub fn target(&self) -> Option<&Target> {
        self.target.as_ref()
    }

    /// The use in progress: the session's hand timing (its contact weight, for a reach animation) and what it
    /// reaches for.
    #[must_use]
    pub fn session(&self) -> (&InteractionSession, Option<Entity>) {
        (&self.session, self.reaching)
    }

    /// One frame: pick the target for `actor` (`None`: nobody is using things, and a reach in progress is let go)
    /// and run the use. `allowed` evaluates a `requires` condition.
    pub fn update(&mut self, world: &World, actor: Option<Actor>, allowed: &dyn Fn(&str) -> bool, dt: f32) -> Option<Used> {
        self.input.update(dt);
        let pressed = self.input.just_activated(ACTION);
        let current = self.target.as_ref().map(|t| t.entity);
        self.target = actor.and_then(|actor| pick(world, actor, current)).and_then(|(entity, at)| {
            let thing = world.get::<Interactable>(entity)?;
            let refused = thing.locked || (!thing.requires.trim().is_empty() && !allowed(&thing.requires));
            let prompt = if refused {
                "Locked".into()
            } else if thing.kind == KIND_DOOR && thing.open {
                "Close".into()
            } else if thing.prompt.is_empty() {
                kind_word(thing.kind).into()
            } else {
                thing.prompt.clone()
            };
            Some(Target { entity, kind: thing.kind, at, prompt, refused })
        });
        if actor.is_none() {
            self.session.cancel();
        }
        let mut used = None;
        if let Some(target) = self.target.as_ref().filter(|_| pressed && !self.session.busy()) {
            if target.refused {
                used = Some(Used { entity: target.entity, kind: target.kind, refused: true, trigger: String::new() });
            } else if let Some(thing) = world.get::<Interactable>(target.entity) {
                // An `open` door is begun as a door too: the session only times the reach.
                match self.session.begin(thing) {
                    Ok(()) => self.reaching = Some(target.entity),
                    Err(why) => tracing::debug!("interact: {why}"),
                }
            }
        }
        match self.session.tick(dt, false) {
            Some(InteractionEvent::Contact) => {
                let entity = self.reaching.filter(|e| world.is_alive(*e))?;
                let thing = world.get::<Interactable>(entity)?;
                used = Some(Used { entity, kind: thing.kind, refused: false, trigger: thing.trigger.clone() });
            }
            Some(InteractionEvent::Returned) => self.reaching = None,
            None => {}
        }
        used
    }
}

/// The interaction layer for a game: the interactor, the doors and the prompt. Per frame: [`Self::on_event`] for
/// each event, [`Self::update`] while playing, [`Self::set_view`] after the camera is set, [`Self::draw`] in the UI
/// pass.
#[derive(Default)]
pub struct Interaction {
    /// The picker and the key (rebind through it).
    pub interactor: Interactor,
    /// The doors (walkers open them with [`crate::door::request_pass`]).
    pub doors: Doors,
    /// What the doors did this frame (creaks, thuds, a locked rattle), for the game's sounds.
    pub door_events: Vec<DoorEvent>,
    prompt: Option<Prompt>,
    view_proj: Option<Mat4>,
}

struct Prompt {
    canvas: somnium_ui::UiCanvas,
    node: somnium_ui::message::NodeHandle,
    laid: Vec2,
}

impl Interaction {
    /// Feed every engine event (only the interact key's matter).
    pub fn on_event(&mut self, event: &EngineEvent) {
        self.interactor.on_event(event);
    }

    /// One frame. Outside Play everything stops: no target, the doors' boxes go (Stop restores their leaves).
    /// Returns what was used, for the kinds the game handles (a door's use is the door's own business too).
    pub fn update(&mut self, ctx: &mut EngineContext, actor: Option<Actor>, allowed: &dyn Fn(&str) -> bool) -> Option<Used> {
        match ctx.simulation.state {
            SimulationState::Editing => {
                self.door_events.clear();
                self.doors.stop(ctx.physics);
                crate::door::clear_passes(ctx.world);
                let interactor = &mut self.interactor;
                (interactor.target, interactor.reaching) = (None, None);
                interactor.session = InteractionSession::default();
                None
            }
            SimulationState::Paused => None,
            SimulationState::Playing => {
                let dt = ctx.dt();
                let used = self.interactor.update(ctx.world, actor, allowed, dt);
                let near = actor.map(|a| a.eye);
                self.door_events = self.doors.update(ctx.world, ctx.physics, used.as_ref(), near.as_slice(), dt);
                used
            }
        }
    }

    /// The camera this frame (world to clip), for the prompt.
    pub fn set_view(&mut self, view_proj: Mat4) {
        self.view_proj = Some(view_proj);
    }

    /// Draw the prompt over the target.
    pub fn draw(&mut self, frame: &mut somnium_ui::GameUiFrame) {
        use somnium_ui::message::{MessageDirection, UiMessage};
        use somnium_ui::widgets::interact_prompt::{InteractPromptBuilder, InteractPromptMessage};
        let message = match (self.interactor.target(), self.view_proj) {
            (Some(target), Some(view_proj)) => InteractPromptMessage::Show {
                view_proj,
                at: target.at,
                key: self.interactor.key_label(),
                text: target.prompt.clone(),
                refused: target.refused,
            },
            _ if self.prompt.is_none() => return,
            _ => InteractPromptMessage::Hide,
        };
        let prompt = self.prompt.get_or_insert_with(|| {
            let mut canvas = somnium_ui::UiCanvas::new(1280.0, 720.0);
            let font = canvas.font_id();
            let root = canvas.ui().root();
            let node = InteractPromptBuilder::new(somnium_ui::widget::WidgetBuilder::new()).with_font(font).build();
            let node = canvas.ui_mut().add_node(node, root);
            Prompt { canvas, node, laid: Vec2::ZERO }
        });
        let window = frame.window();
        let (size, scale) = (window.inner_size(), window.scale_factor() as f32);
        let size = Vec2::new(size.width.max(1) as f32, size.height.max(1) as f32) / scale;
        if size != prompt.laid {
            prompt.canvas.place_node(prompt.node, somnium_ui::types::Rect::new(0.0, 0.0, size.x, size.y));
            prompt.laid = size;
        }
        prompt.canvas.ui_mut().send(UiMessage::new(prompt.node, MessageDirection::ToWidget, message));
        frame.draw(&mut prompt.canvas);
    }
}

#[cfg(test)]
#[path = "interact_tests.rs"]
mod tests;
