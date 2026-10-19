# Switching between two protagonists

Rimeborn's second half has two played characters, Caelan and his son Rhun (`games/Rimeborn/design/DUAL_PROTAGONISTS.md`).
Switching between them takes 3 to 6 seconds:
- the camera lifts off the one being played, up to the Lid's height;
- it crosses the world while the other man's cells stream in underneath;
- it falls onto him;
- as the fall begins, control passes to him.

The same lift serves the story's "meanwhile" cuts. Three engine pieces do it.

## The lift (`somnium_core::camera_lift`)

`Lift::new(rig, settings, to, LiftShape)` builds a lift from where the isometric rig looks now to where it will look
when it follows `to`. `update(dt, ready)` returns the camera pose and the phase: Rising, Crossing, Holding, Falling,
Done.

- **No jumps.** It is drawn with the rig's own pose, so it leaves the rig's view of the start and lands exactly on the
  rig's view of the destination.
- **The top stretches.** If the destination isn't `ready` when the lift gets there, it holds high above it until it
  is. Waiting doesn't shorten the fall.
- **Shape.** `LiftShape` sets the height (900 m by default), how steeply the view looks down at the top (80°), and
  the legs: rise 1.3 s, fall 1.4 s, and a crossing at 4 km/s kept between 0.6 and 2.5 s. With the destination ready,
  a switch takes 3.3 to 5.2 s.

## Holding a second place's cells (`somnium_core::cell_streaming`)

Streaming normally follows the view. `cell_streaming::hold(world, key, at)` keeps the cells around another point
loaded as if the view were there too. Uses: the switch's destination while the camera lifts, and both men's places
during a joint or parallel mission.

- `hold_ready(world, key)` says when every cell around the point is in. A cell that failed to load counts as in, so
  a broken file never blocks a switch.
- Calling `hold` again with the same key moves the hold. A moved hold isn't ready until its new cells are.
- `release(world, key)` lets the cells go once nothing else keeps them.

The streamer plans from the view and every hold (`plan_many`). With no holds it is exactly what it was.

## The switch (`somnium_core::protagonist_switch`)

```rust
// Tab, and the game's rules (DUAL_PROTAGONISTS 2) allow it; `to` is where the other man stands now.
let mut switch = Switch::begin(ctx.world, &rig, &settings, to);
// Each frame while it runs: the rig yields the camera to the switch.
let frame = switch.update(ctx.world, dt);
camera = frame.camera;
if frame.hand_over {
    // The game's part: the other fighter becomes the played one (input, lock-on, the HUD's subject), and the one
    // left goes to the life sim on his schedule.
}
if frame.done {
    rig.focus = switch.destination(); rig.focus_goal = rig.focus; rig.snap();   // and follow the arrived man
}
```

- **When it's ready.** The destination is ready when the scene streams no cells, when its hold's cells are all in, or
  after 8 s at the top (`MAX_WAIT`), so the camera is never stranded.
- **Hand-over.** `hand_over` is true on exactly one frame, as the fall begins (the design's step 4). The arrival
  vignette (step 5) covers the last of the streaming.
- **Cancelling.** `cancel(world)` gives up a switch (an attack as it began, a cut scene) and releases the hold.

## What is the game's

Each item below names its owner:
- **The rules:** when a switch is allowed, and switching *to* a man in a story lock (DUAL_PROTAGONISTS 2).
- **The hand-over:** which fighter is played, which belongs to the combat host.
- **The off-screen man:** where he is (his schedule, from `somnium_life`).
- **The interface:** the switch wheel (rb-ui).
- **Arrival:** the vignette played on arrival (`play_dialogue`).
