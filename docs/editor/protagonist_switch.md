# Switching between two protagonists

Rimeborn's second half has two played characters, Caelan and his son Rhun (`games/Rimeborn/design/DUAL_PROTAGONISTS.md`).
A switch takes 3 to 6 seconds:
- the player holds **Left Alt** (the `SwitchProtagonist` action, rebindable) and picks the other man on the wheel;
- the camera lifts off the one being played, up to the Lid's height, and at the top the game crossfades to its
  world map;
- the camera crosses while the other man's cells stream in;
- it falls onto him, and as the fall begins, control is his.

The same lift serves the story's "meanwhile" cuts. Three engine pieces do it.

## The lift (`somnium_core::camera_lift`)

`Lift::new(rig, settings, to, LiftShape)` builds a lift from where the isometric rig looks now to where it will look
when it follows `to`. `update(dt, ready)` returns the camera pose and the phase: Rising, Crossing, Holding, Falling,
Done.

- **No jumps.** It is drawn with the rig's own pose, so it leaves the rig's view of the start and lands exactly on the
  rig's view of the destination.
- **The top waits.** If the destination isn't `ready` when the lift gets there, it holds high above it until it is.
  Waiting doesn't shorten the fall.
- **`altitude()`** runs 0 (the rig's view) to 1 (the top). At the top the camera sees far past the cells streamed
  around either place. The game crossfades to its world map from about 0.6 up, so the crossing pans the map, and fades
  back out below 0.6 on the fall. The lift keeps moving under the map, so the right world is there when it fades out.
- **Shape.** `LiftShape` sets the height (900 m by default), how steeply the view looks down at the top (80°), and
  the legs: rise 1.3 s, fall 1.4 s, and a crossing at 4 km/s kept between 0.6 and 2.5 s. With the destination ready,
  a switch takes 3.3 to 5.2 s.

## Holding a second place's cells (`somnium_core::cell_streaming`)

Streaming normally follows the view (the camera).

- **`hold(world, key, at)`** keeps the cells within 250 m (`HOLD_RADIUS`) of another point loaded. `hold_within`
  takes another radius. Uses: the switch's destination while the camera lifts, and both men's places during a joint
  or parallel mission.
  - A hold ignores landmark cells' long reach; that reach is the view's.
  - At most **two** holds (`MAX_HOLDS`). A third is refused (`hold` returns false and logs once): each disc of cells
    costs GPU memory, and a region's first fills already run close to it.
  - Holding a key again moves it. A hold moved along the ground isn't ready until its new cells are.
- **`hold_ready(world, key)`** says when every cell within the hold's radius is in. A cell that failed to load counts
  as in, so a broken file never blocks a switch.
- **`release(world, key)`** lets the cells go once nothing else keeps them.
- **`held_points(world)`** gives the held points to the systems that think by distance. Pass them to combat's
  `set_focus` and to `LifeWorld::step`'s focus as well, so the far man's half of a joint mission thinks, not only
  streams.
- **`pin_view(world, Some(at))`** streams from `at` instead of the camera, until `pin_view(world, None)`. The switch
  pins the view at the place it leaves while the camera is up, so the crossing starts no readers for the land
  beneath it.

The streamer plans from the view and every hold (`plan_many`). With no holds and no pin, it is exactly what it was.

## The switch (`somnium_core::protagonist_switch`)

```rust
// Once: the switch key alongside the game's other maps (hold Left Alt; rebindable).
input.add_map(protagonist_switch::switch_map());
// When the wheel picks the other man (the game's rules, DUAL_PROTAGONISTS 2, allow it); `to` is where he stands now.
let mut switch = Switch::begin(ctx.world, &rig, &settings, to);
// In a joint mission the game already holds both men: switch on his hold instead.
let mut switch = Switch::begin_on(ctx.world, &rig, &settings, to, "rhun");
// Each frame while it runs: the rig yields the camera to the switch.
let frame = switch.update(ctx.world, dt);
camera = frame.camera;
map_fade = smoothstep(0.5, 0.7, frame.altitude);   // the world map over the top of the lift
if frame.held_for > 1.0 { /* the veil: a card on the map until the destination is in */ }
if frame.hand_over {
    // The game's part: the other fighter becomes the played one (input, lock-on, the HUD's subject), and the one
    // left goes to the life sim on his schedule.
}
if frame.done {
    rig.focus = switch.destination(); rig.focus_goal = rig.focus; rig.snap();   // and follow the arrived man
}
```

- **Ready.** The destination is ready when the scene streams no cells, or when its hold's cells are all in. There is
  no time cap: a slow destination keeps the camera high, behind the game's veil (`held_for`), rather than landing on
  missing ground.
- **Hand-over.** `hand_over` is true on exactly one frame, as the fall begins (the design's step 4), and the view is
  unpinned then. The arrival vignette (step 5) covers the last of it.
- **Cancelling.** `cancel(world)` gives up a switch (an attack as it began, a cut scene). It unpins the view and
  releases the switch's own hold. A hold the game passed to `begin_on` stays held.
- **Reduced motion.** The game's comfort option skips the lift and fades to black instead, waiting on the same
  `hold_ready`.

## What is the game's

Each item below names its owner:
- **The rules:** when a switch is allowed, and switching *to* a man in a story lock (DUAL_PROTAGONISTS 2).
- **The wheel:** shown while `SwitchProtagonist` is held (rb-ui).
- **The map crossfade and the veil** (rb-ui).
- **The hand-over:** which fighter is played, which belongs to the combat host.
- **The off-screen man:** where he is (his schedule, from `somnium_life`).
- **Arrival:** the vignette played on arrival (`play_dialogue`).
