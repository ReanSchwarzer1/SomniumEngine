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
//! The verbs, by `kind`:
//! - 0 **Door** opens and shuts itself ([`crate::door`]).
//! - 1 **Collect** (take): at the commit the thing, and everything under it, is removed. `gone_when` keeps it gone:
//!   a thing whose condition holds is removed as it appears (a cell streams in).
//! - 2 **Inspect** (read: notes, signs), 3 **Operate** (use: wells, levers, devices; with `swings` it swings like a
//!   door), 4 Equip Light: the game acts on the [`Used`], by its `trigger`.
//! - 5 **Talk**: the game starts the conversation named by `trigger`. A `somnium.CastMember` with a speaker and no
//!   interactable of its own is a talk target too, its speaker the trigger: rb-story's people talk unauthored.
//! - 6 **Sit** (benches, stools): the game seats the player at [`seat`].
//!
//! [`Interaction`] bundles the interactor, the doors and the prompt for a game to drive in four calls. Nothing here
//! runs unless a game calls it, so TSF's own use of `somnium.Interactable` (first-person hands) is untouched.

use std::collections::HashMap;

use glam::{Mat4, Vec2, Vec3};
use somnium_ecs::{ComponentId, ComponentSet, Entity, World};
use somnium_input::{Action, ActionMap, Binding, ControlPath, InputSystem};

use crate::door::{DoorEvent, Doors};
use crate::interaction::{Interactable, InteractionEvent, InteractionSession};
use crate::story_actor::CastMemberComponent;
use crate::{EngineContext, EngineEvent, InputState, Parent, SimulationState, Transform, WorldTransform};

/// `Interactable::kind`s.
pub const KIND_DOOR: u32 = 0;
#[allow(missing_docs)]
pub const KIND_COLLECT: u32 = 1;
#[allow(missing_docs)]
pub const KIND_INSPECT: u32 = 2;
#[allow(missing_docs)]
pub const KIND_OPERATE: u32 = 3;
#[allow(missing_docs)]
pub const KIND_EQUIP_LIGHT: u32 = 4;
#[allow(missing_docs)]
pub const KIND_TALK: u32 = 5;
#[allow(missing_docs)]
pub const KIND_SIT: u32 = 6;

/// How far a cast member can be talked to from, eye to chest, metres (Rimeborn's talk reach).
pub const CAST_REACH: f32 = 3.0;
/// Where a cast member is talked to: their chest, this far above their origin (the feet), metres.
pub const CAST_HEIGHT: f32 = 1.4;

/// The prompt's verb when an interactable's own prompt is empty, by `kind`.
#[must_use]
pub fn kind_word(kind: u32) -> &'static str {
    match kind {
        KIND_DOOR => "Open",
        KIND_COLLECT | KIND_EQUIP_LIGHT => "Take",
        KIND_INSPECT => "Read",
        KIND_TALK => "Talk",
        KIND_SIT => "Sit",
        _ => "Use",
    }
}

/// An interactable for `kind` with the layer's defaults: an empty prompt (it says the verb) and `trigger`. Talk
/// reaches as far as a cast member does; a door comes from [`crate::door::door_at_hinge`].
#[must_use]
pub fn interactable(kind: u32, trigger: &str) -> Interactable {
    Interactable {
        kind,
        prompt: String::new(),
        trigger: trigger.into(),
        reach: if kind == KIND_TALK { CAST_REACH } else { Interactable::default().reach },
        // A short reach: the commit comes 0.15 s after the key.
        duration: 0.3,
        ..Interactable::default()
    }
}

/// What a cast member with no interactable of its own is used as: talk, by their speaker id.
fn cast_talk(speaker: &str) -> Interactable {
    Interactable { anchor: Vec3::Y * CAST_HEIGHT, ..interactable(KIND_TALK, speaker) }
}

/// What `entity` is used as: its interactable, or a cast member's talk.
fn config(world: &World, entity: Entity) -> Option<Interactable> {
    world.get::<Interactable>(entity).cloned().or_else(|| {
        let speaker = &world.get::<CastMemberComponent>(entity)?.speaker;
        (!speaker.is_empty()).then(|| cast_talk(speaker))
    })
}

/// Where a seat (an interactable of kind Sit) puts whoever sits on it: its anchor in the world (the hips), and the
/// way they face along the ground (the seat's +Z).
#[must_use]
pub fn seat(world: &World, entity: Entity) -> Option<(Vec3, Vec3)> {
    let thing = world.get::<Interactable>(entity)?;
    let frame = world.get::<WorldTransform>(entity)?.0;
    let facing = frame.transform_vector3(Vec3::Z);
    Some((frame.transform_point3(thing.anchor), Vec3::new(facing.x, 0.0, facing.z).normalize_or(Vec3::Z)))
}

/// Remove `roots` and everything under them (a taken thing). One pass over the parented entities.
pub fn remove_with_children(world: &mut World, roots: &[Entity]) {
    if roots.is_empty() {
        return;
    }
    let mut children: HashMap<Entity, Vec<Entity>> = HashMap::new();
    for (entity, parent) in world.iter_with::<Parent>() {
        children.entry(parent.entity).or_default().push(entity);
    }
    let mut stack = roots.to_vec();
    while let Some(entity) = stack.pop() {
        stack.extend(children.remove(&entity).unwrap_or_default());
        world.despawn(entity);
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
    /// The interactable's `trigger`: the game's name for what using it means (a cast member's: their speaker).
    pub trigger: String,
    /// The anchor in the world when it was used (for a sound, a flourish).
    pub at: Vec3,
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

/// A cast member's facing cone for talk, degrees (the interactable default).
const CAST_CONE: f32 = 75.0;
/// The squares interactables are filed in, metres.
const PLACE_CELL: f32 = 4.0;

fn place_cell(at: Vec3) -> (i32, i32) {
    ((at.x / PLACE_CELL).floor() as i32, (at.z / PLACE_CELL).floor() as i32)
}

/// An entity's world frame from its local transforms: right on the frame it arrives, before propagation. An entity
/// with no `Transform` keeps its `WorldTransform`.
fn local_frame(world: &World, entity: Entity) -> Mat4 {
    if world.get::<Transform>(entity).is_none() {
        return world.get::<WorldTransform>(entity).map_or(Mat4::IDENTITY, |w| w.0);
    }
    let (mut frame, mut at) = (Mat4::IDENTITY, entity);
    for _ in 0..64 {
        frame = world.get::<Transform>(at).map_or(Mat4::IDENTITY, Transform::to_matrix) * frame;
        match world.get::<Parent>(at).map(|p| p.entity).filter(|p| *p != Entity::DANGLING) {
            Some(parent) => at = parent,
            None => break,
        }
    }
    frame
}

/// A set of entities kept as one slot per entity index (its generation + 1): membership in O(1), no hashing. A
/// despawned entity's slot reads as absent once its index is reused.
#[derive(Default)]
pub(crate) struct Marks(Vec<u32>);

impl Marks {
    pub(crate) fn contains(&self, entity: Entity) -> bool {
        self.0.get(entity.index() as usize) == Some(&entity.generation().wrapping_add(1))
    }

    pub(crate) fn insert(&mut self, entity: Entity) {
        let i = entity.index() as usize;
        if i >= self.0.len() {
            self.0.resize(i + 1, 0);
        }
        self.0[i] = entity.generation().wrapping_add(1);
    }
}

/// The interactables that stay put (all but cast members, who walk), filed by where their anchors are when they
/// arrive: a frame's pick looks at the few squares around the player, whatever a city holds. Filing happens only
/// when the world's population of interactables moves (a cell came or went). An interactable that moves after it
/// arrives (on a cart) is found where it arrived; put moving ones on a cast member.
#[derive(Default)]
struct Places {
    cells: HashMap<(i32, i32), Vec<(Entity, Vec3)>>,
    filed: Marks,
    /// Entries filed, gone ones included until a sweep.
    entries: usize,
    population: u64,
    /// The longest reach filed: how far around the player a pick looks.
    reach: f32,
}

impl Places {
    fn update(&mut self, world: &World) {
        let id = ComponentId::of::<Interactable>();
        let population = world.population_tick(id);
        if population == self.population {
            return;
        }
        self.population = population;
        let mut live = 0;
        for (entity, thing) in world.iter_with::<Interactable>() {
            live += 1;
            if self.filed.contains(entity) || world.get::<CastMemberComponent>(entity).is_some() {
                continue;
            }
            let at = local_frame(world, entity).transform_point3(thing.anchor);
            self.cells.entry(place_cell(at)).or_default().push((entity, at));
            self.filed.insert(entity);
            self.entries += 1;
            self.reach = self.reach.max(thing.reach);
        }
        // Gone ones are passed over by the pick; once they outnumber the living, they are swept.
        if self.entries > 2 * live + 1024 {
            for list in self.cells.values_mut() {
                list.retain(|&(e, _)| world.has_component(e, id));
            }
            self.cells.retain(|_, list| !list.is_empty());
            self.entries = self.cells.values().map(Vec::len).sum();
        }
    }
}

/// The interactable `actor` would use: within reach of the eye, inside its cone, the best of distance (as a share of
/// reach) and facing. `current` keeps the last target unless another is clearly better. Files the world's
/// interactables first; an [`Interactor`] keeps its filing from frame to frame.
#[must_use]
pub fn pick(world: &World, actor: Actor, current: Option<Entity>) -> Option<(Entity, Vec3)> {
    let mut places = Places::default();
    places.update(world);
    pick_in(world, &places, actor, current)
}

/// [`pick`] among filed places: the squares within the longest reach of the eye, and the cast members' own columns
/// (they move, so they are not filed; a cast member with no interactable is talk).
fn pick_in(world: &World, places: &Places, actor: Actor, current: Option<Entity>) -> Option<(Entity, Vec3)> {
    let forward = Vec3::new(actor.forward.x, 0.0, actor.forward.z).normalize_or_zero();
    let id = ComponentId::of::<Interactable>();
    let cast_id = ComponentId::of::<CastMemberComponent>();
    let required = ComponentSet::from_ids(vec![cast_id, ComponentId::of::<WorldTransform>()]);
    let mut best: Option<(Entity, Vec3, f32)> = None;
    let mut kept: Option<(Entity, Vec3, f32)> = None;
    let mut consider = |entity: Entity, at: Vec3, reach: f32, cone: f32| {
        let to = at - actor.eye;
        if to.length() > reach {
            return;
        }
        let along = Vec2::new(to.x, to.z);
        let distance = along.length();
        let facing =
            if distance < CLOSE || forward == Vec3::ZERO { 1.0 } else { (forward.x * along.x + forward.z * along.y) / distance };
        if facing < cone.to_radians().cos() {
            return;
        }
        let score = to.length() / reach.max(1e-3) + 0.5 * (1.0 - facing);
        if Some(entity) == current {
            kept = Some((entity, at, score));
        }
        if best.is_none_or(|(_, _, s)| score < s) {
            best = Some((entity, at, score));
        }
    };
    let (lo, hi) = (place_cell(actor.eye - Vec3::splat(places.reach)), place_cell(actor.eye + Vec3::splat(places.reach)));
    for x in lo.0..=hi.0 {
        for z in lo.1..=hi.1 {
            for &(entity, at) in places.cells.get(&(x, z)).into_iter().flatten() {
                if let Some(thing) = world.get::<Interactable>(entity).filter(|t| t.enabled) {
                    consider(entity, at, thing.reach, thing.cone);
                }
            }
        }
    }
    for archetype in world.query_archetypes(&required, &ComponentSet::empty()) {
        let cast_col = archetype.column_index(cast_id).expect("queried for it");
        let world_col = archetype.column_index(ComponentId::of::<WorldTransform>()).expect("queried for it");
        let own_col = archetype.column_index(id);
        for (row, &entity) in archetype.entities().iter().enumerate() {
            // SAFETY: each column is the one `column_index` found for that component's id; `row` is this archetype's.
            let frame = unsafe { archetype.column(world_col).get::<WorldTransform>(row) }.0;
            let own = own_col.map(|c| unsafe { archetype.column(c).get::<Interactable>(row) });
            match own {
                Some(thing) if thing.enabled => consider(entity, frame.transform_point3(thing.anchor), thing.reach, thing.cone),
                Some(_) => {}
                None => {
                    if !unsafe { archetype.column(cast_col).get::<CastMemberComponent>(row) }.speaker.is_empty() {
                        consider(entity, frame.transform_point3(Vec3::Y * CAST_HEIGHT), CAST_REACH, CAST_CONE);
                    }
                }
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
    places: Places,
}

impl Default for Interactor {
    fn default() -> Self {
        let mut input = InputSystem::new();
        input.add_map(interact_map());
        Self { input, target: None, session: InteractionSession::default(), reaching: None, places: Places::default() }
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
        self.places.update(world);
        let places = &self.places;
        self.target = actor.and_then(|actor| pick_in(world, places, actor, current)).and_then(|(entity, at)| {
            let thing = config(world, entity)?;
            let refused = thing.locked || (!thing.requires.trim().is_empty() && !allowed(&thing.requires));
            let prompt = if refused {
                if thing.refused_prompt.is_empty() { "Locked".into() } else { thing.refused_prompt.clone() }
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
                let (entity, kind, at) = (target.entity, target.kind, target.at);
                used = Some(Used { entity, kind, refused: true, trigger: String::new(), at });
            } else if let Some(thing) = config(world, target.entity) {
                // An `open` door is begun as a door too: the session only times the reach.
                match self.session.begin(&thing) {
                    Ok(()) => self.reaching = Some(target.entity),
                    Err(why) => tracing::debug!("interact: {why}"),
                }
            }
        }
        match self.session.tick(dt, false) {
            Some(InteractionEvent::Contact) => {
                let entity = self.reaching.filter(|e| world.is_alive(*e))?;
                let thing = config(world, entity)?;
                let at = world.get::<WorldTransform>(entity).map_or(Vec3::ZERO, |w| w.0.transform_point3(thing.anchor));
                used = Some(Used { entity, kind: thing.kind, refused: false, trigger: thing.trigger, at });
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
    /// Taken last frame: removed at the start of the next update, so the game can still read what it took.
    taken: Vec<Entity>,
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
                self.taken.clear();
                self.doors.stop(ctx.physics);
                crate::door::clear_passes(ctx.world);
                let interactor = &mut self.interactor;
                (interactor.target, interactor.reaching) = (None, None);
                interactor.session = InteractionSession::default();
                interactor.places = Places::default();
                None
            }
            SimulationState::Paused => None,
            SimulationState::Playing => {
                let dt = ctx.dt();
                self.play(ctx.world, ctx.physics, actor, allowed, dt)
            }
        }
    }

    /// One frame of play, as [`Self::update`] runs it while playing.
    pub fn play(
        &mut self,
        world: &mut World,
        physics: &mut somnium_physics::world::PhysicsWorld,
        actor: Option<Actor>,
        allowed: &dyn Fn(&str) -> bool,
        dt: f32,
    ) -> Option<Used> {
        remove_with_children(world, &std::mem::take(&mut self.taken));
        let used = self.interactor.update(world, actor, allowed, dt);
        let near = actor.map(|a| a.eye);
        self.door_events = self.doors.update(world, physics, used.as_ref(), near.as_slice(), dt);
        // Taken now (gone next update), and things already taken as they appear (a cell streamed in).
        self.taken.extend(used.as_ref().filter(|u| u.kind == KIND_COLLECT && !u.refused).map(|u| u.entity));
        let gone: Vec<Entity> = self
            .doors
            .arrived()
            .iter()
            .copied()
            .filter(|&e| world.get::<Interactable>(e).is_some_and(|t| !t.gone_when.trim().is_empty() && allowed(&t.gone_when)))
            .collect();
        remove_with_children(world, &gone);
        used
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
