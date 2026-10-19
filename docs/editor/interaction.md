# Interaction in an isometric game, and doors

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
- One pass over the interactables picks the target: about 1.2 µs a frame for 400 (release build).

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
| Kind | 0 Door swings itself; the others (1 Collect, 2 Inspect, 3 Operate, 4 Equip Light) go to the game |
| Prompt, Trigger, Enabled, Locked | As above |
| Reach | From the player's eye to the anchor, in metres (2.0) |
| Duration | The reach and return. A door's is 0.4 s, so it moves 0.2 s after the key |
| Anchor | The point reached, in this entity's frame. A door's is the middle of its leaf at hand height |
| Open Angle | How far a door swings, in degrees; positive turns anticlockwise seen from above |
| *Cone* (new) | How far either side of the player's facing it can be picked, in degrees (75). 180: any direction |
| *Requires* (new) | The world-state condition above. Empty: only Locked locks |

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
- the door whose doorway lies within 1.2 m swings open, whatever its lock;
- it closes 1.5 s after it has finished opening.

## Driving it from a game

```rust
// on_event, for every event:
interaction.on_event(event);
// on_update, after the player has moved (None: nobody is using things):
let actor = Actor { eye: player_eye, forward: player_facing };
if let Some(used) = interaction.update(ctx, Some(actor), &|condition| story.check(condition).unwrap_or(false)) {
    // The commit of a collect, inspect, operate or equip: `used.trigger` says which. `used.refused`: a locked try.
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

A town cell of 400 buildings (6,000 entities), each with a door of two named parts, in a release build:

| | Cost |
|---|---|
| Finding the doors and their leaves, and making their boxes (once, as they arrive) | about 3 ms |
| Every frame while no door moves | 0.3 µs |
| Picking the target among 400 interactables, every frame | 1.2 µs |

Idle doors are never looked at one by one. Each frame compares the interactables' archetype entity lists with the
last frame's, which costs about as much as copying them. Only doors that are swinging, or waiting to close, are
touched. Nothing is written while every door rests.
