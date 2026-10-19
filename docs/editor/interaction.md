# Interaction in an isometric game: doors, talk, take, sit, read, use

Rimeborn's player walks into buildings, opens their doors and uses what is inside. It does this with the engine's
**Interaction** component (`somnium.Interactable`), the same one TSF's first-person hands use, driven by an
isometric layer, `somnium_core::interact::Interaction`. A door is an Interactable of kind **Door** whose entity sits
on the hinge.

The layer runs only when a game calls it. TSF's interactions behave exactly as before, and the fields below that are
marked *new* are read by nothing else.

## Picking what to use

An isometric view has no screen-centre ray to aim with. The player uses **the interactable nearest in front of
them**:
- within its **Reach** of the player's eye;
- inside its **Cone** either side of the player's facing;
- of those, the best mix of near and straight ahead.

A small prompt floats over it: the key, then the **Prompt** ("E  Open"). An empty prompt says the kind (Open, Take,
Inspect, Use), and an open door says "Close". The interact key (E by default) uses it.

- Within 0.45 m along the ground, the facing no longer matters, so a player standing in a doorway can still close
  the door.
- The current target holds until another is clearly better, so two doors side by side do not flicker.
- The pick looks only at the interactables filed near the player (4 m squares), and at the cast members, so a
  frame costs the same however many a city holds (see **What it costs**).
- Interactables are filed where they are when they arrive. One that moves later (cargo on a cart) is still found
  where it arrived. Put things that walk on a cast member: cast members are looked at where they stand each frame.

## Using it

A use runs the interactable's own interaction session, the same timing TSF's hands follow:
1. the reach takes half of **Duration**;
2. then comes the one commit event, *contact*;
3. then the hand returns.

At the commit, the game gets the use back, with the interactable's **Trigger** as its name for what the use means.
A door swings at the commit.

If the player stops being an actor mid-reach (a dialogue opens, Play stops), the reach is let go without
committing.

**Locked**, or a **Requires** condition that is false, refuses the use at once: the prompt reads "Locked" and
nothing moves. **Requires** is a world-state condition evaluated by the game. In Rimeborn it is a `somnium_story`
fact or expression, such as `has_church_key` or `chapter >= 2 and not curfew`.

| Field | Used here for |
|---|---|
| Kind | What using it does: see **The verbs** below. 5 Talk and 6 Sit are new; 0 to 4 are TSF's |
| Prompt, Trigger, Enabled, Locked | As above |
| Reach | From the player's eye to the anchor, in metres (2.0) |
| Duration | The reach and return. A door's is 0.4 s, so it moves 0.2 s after the key |
| Anchor | The point reached, in this entity's frame. A door's is the middle of its leaf at hand height |
| Open Angle | How far a door swings, in degrees; positive turns anticlockwise seen from above |
| *Cone* (new) | How far either side of the player's facing it can be picked, in degrees (75). 180: any direction |
| *Requires* (new) | The world-state condition above. Empty: only Locked locks |
| *Refused Prompt* (new) | The prompt while refused ("Barred for the night"). Empty says "Locked" |
| *Gone When* (new) | A world-state condition. While it holds, the thing is gone: it is removed as it appears |
| *Swings* (new) | An operated thing (a lever, a gate) swings like a door when used, blocking nothing |

## The verbs

| Kind | Prompt | What happens at the commit |
|---|---|---|
| 0 Door | Open, Close | It swings itself (below) |
| 1 Collect | Take | The game gets the use, then the thing and everything under it are removed at the start of the next frame |
| 2 Inspect | Read | The game opens the note or sign that **Trigger** names |
| 3 Operate | Use | The game acts on **Trigger** (a well, a Glyphcraft device). With **Swings**, it also swings like a door (a lever) |
| 4 Equip Light | Take | The game's |
| 5 Talk | Talk | The game starts the conversation for **Trigger** |
| 6 Sit | Sit | The game seats the player at `interact::seat(world, entity)` |

`somnium_core::interact::interactable(kind, trigger)` gives an Interactable with this layer's defaults:
- an empty prompt, so it says the verb;
- a short reach, so the commit comes 0.15 s after the key;
- for Talk, a 3 m reach.

TSF's default prompt is "Operate", so set the prompt empty, or to your own words.

**Talk needs no authoring for the story's people.** A `somnium.CastMember` with a speaker and no Interactable of its
own is a talk target: its chest, 1.4 m above its origin, within 3 m of the player's eye; its speaker id is the
trigger. Give a cast member its own Interactable to change that (a shopkeeper's "Barter", a different trigger).

**Take for good.** A take removes the thing at once, but its cell brings it back when it streams in again. To stop
that, set **Gone When** to the fact your game sets when the thing is taken (for example `taken.purse_01`). A thing
whose condition holds is removed the moment it appears.

**Sit.** `seat` gives the anchor in the world, where the hips go, and the way the sitter faces: the seat's +Z, along
the ground. While the player sits, the seat stays the nearest target. Use on it again is the game's cue to stand.

**Use with Swings.** A lever is an Operate whose children, or **Leaves**, turn by **Open Angle** about its origin's
vertical. It springs back after **Close After**, or stays pulled when that is 0. Its **Open** says whether it is
pulled. It has no box.

## Doors

A door's entity *is* its hinge:
- its origin sits on the hinge's axis, at the bottom of the leaf;
- its **+X** runs along the closed leaf to the free edge;
- **+Y** is up.

What swings:
- the door's **children**;
- the entities under the door's **parent** whose names start with **Leaves**. This is for imported parts: a WS2
  building's `Door / <material>` parts, wherever they sit under the building (under `LOD0`, or inside a cutaway
  group).

| Field (all new, in the Door group) | Does |
|---|---|
| Leaf Width, Leaf Height, Leaf Thickness | The leaf, in metres |
| Blocks | While the door is closed, a static box fills the leaf, so a locked door stops the player |
| Leaves | The name prefix of extra parts that swing with it (`Door / `). Its children always swing |
| Swing Duration | Seconds to swing fully open or shut (0.9), eased at both ends |
| Close After | It closes by itself this long after it finishes opening, once nobody stands in its swing. 0 stays open |
| Open | Open now. Authored, it starts open; while playing, it is the door's state |

The building colliders keep the doorway open: each wall is broken at its door. The door's own box is what shuts
it. The box goes as the door starts to open and comes back once the door is fully shut.

**Open** works both ways:
- the door system writes it when the door is used;
- a game or script that sets it swings the door (a quest opening the gate, a guard shutting it at curfew).

Using a door while it swings sends it back the other way.

The hinge's frame is read once, when the door is first found during Play. Doors are for buildings that stay put.

### From a building's door record

`somnium_core::door::door_at_hinge(hinge, yaw, width, height, opens, leaves)` gives a door's Transform and
Interactable from a record in a building's frame. Its arguments:
- `yaw` is radians about +Y, from the building's +X to the leaf;
- `opens` is ±1, the side it swings to, as WS2's `doors_gltf` writes it. It becomes ±100°.

Rimeborn spawns these under each building root as the building arrives (`games/Rimeborn/src/doors.rs`).

A building composer can also write the door Interactables straight into the scene. The doors are then in the
editor, where they can be moved and tuned, and nothing reads the building file at run time.

## Villagers and other walkers

`somnium_core::door::request_pass(world, at)` says that a walker is going through the doorway at `at`. Any system
holding the world can call it, such as the life sim or the AI. On the doors' next update:
- the nearest door whose doorway lies within 2 m along the ground (and a storey up or down) swings open, whatever
  its lock;
- it closes 1.5 s after it has finished opening.

## Driving it from a game

```rust
// on_event, for every event:
interaction.on_event(event);
// on_update, after the player has moved (None: nobody is using things):
let actor = Actor { eye: player_eye, forward: player_facing };
if let Some(used) = interaction.update(ctx, Some(actor), &|condition| story.check(condition).unwrap_or(false)) {
    match used.kind {
        _ if used.refused => { /* a locked try: a rattle at used.at */ }
        interact::KIND_TALK => { /* start the conversation for used.trigger (a speaker id) */ }
        interact::KIND_COLLECT => { /* add used.trigger to the inventory; set taken.<id>; it goes next frame */ }
        interact::KIND_INSPECT => { /* open the note used.trigger */ }
        interact::KIND_SIT => { /* seat the player at interact::seat(world, used.entity) */ }
        _ => { /* a well, a device: used.trigger */ }
    }
}
for event in &interaction.door_events { /* Opening, Closing, Closed, Refused: play a sound */ }
// on_render, once the camera is set:
interaction.set_view(renderer.view_proj);
// on_render_ui:
interaction.draw(frame);
```

Outside Play, `update` drops the target, any reach in progress, and every door's box. Stop restores the leaves with
the rest of the world.

To rebind the key:

```rust
somnium_input::rebind::rebind(interaction.interactor.input_mut().maps_mut(),
    somnium_core::interact::MAP, somnium_core::interact::ACTION, somnium_input::ControlPath::keyboard("f"))?;
```

The prompt shows whichever key is bound.

## What it costs

The whole layer (`Interaction::play`) in a release build. Each building has a door of two named parts, a stall,
two pick-ups, a sign and a seat, so six interactables (`interact::tests::measure_city_density`):

| | 400 buildings, 2,400 interactables, 50 cast | 4,000 buildings, 24,000 interactables, 500 cast |
|---|---|---|
| A frame, the player beside a door | 0.7 µs | 2.2 µs |
| A frame with 20 walkers going through doorways | 5 µs | 10 µs |
| A cell of 50 buildings arriving | 0.4 ms | 1.8 ms |
| The first frame of play (everything arrives) | 1.8 ms | 20 ms |

On the city side, the frame's extra 1.5 µs is the cast members, about 3 ns each, looked at where they stand.
Interactables add nothing. On the same machine the layer used to cost 191 µs a frame at the city's density, and a
cell arriving 5.3 ms.

How it stays flat:
- **Arrivals.** The world counts, per component, the entities that gained or lost it (`World::population_tick`).
  While that and the interactables' change tick are as they were, nothing arrived, went or was written, and the
  frame reads two counters.
- **The cell-arrival walk.** When a cell does arrive, one walk of the interactables finds the new ones, at an O(1)
  mark each.
- **The leaf search.** A new door's leaves are found in one walk of the world's parented and named entities. At
  1.2 ms of the 1.8, this is most of a cell's arrival, against the 6 ms the cell's own spawning took.
- **Doors.** A frame touches only doors that are swinging or waiting to close. Nothing is written while every door
  rests.
- **Passes.** A walker's pass looks only at the doorways filed near it.
