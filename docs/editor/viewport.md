# Viewport & camera

The large centre pane is the 3D view. Chrome around it does not steal fly-cam input.

## Camera

- **Right mouse + WASD / QE** — fly. Hold **Shift** to go faster.
- **RMB + scroll wheel** — camera speed. The same value lives on the viewport toolbar slider (shown as m/s).
- The top-right of the title bar shows frame rate.
- Select the **Camera** entity in the Outliner for **Frustum Cull** (default on): terrain chunks whose AABB misses the camera never reach the draw queue. Off-screen ground can still cast into view (cascade volumes, not the camera). Hold **RMB** and look at empty sky so the tile is behind you — profiler `cpu-cull` rises and `vis` drops; flying with WASD while still looking at the coast will not, because the 1 km tile fits a 45° frustum. If the row says `[off]` or `[forced-off]`, the CPU test is not running (`SOMNIUM_CPU_FRUSTUM=0` or the checkbox). **F10** is the GPU 15B A/B and is independent. Physical Camera (aperture / shutter / ISO) lives on **Post Processing**, not here.
- **Play** possesses the Outliner **Camera** (its world transform; local `-Z` is look). The editor fly-cam is restored on **Stop**. Parent that Camera under a player later and Play will follow it. Move the Camera with the gizmo in edit mode to choose where Play starts. Loading a map from the Content Drawer reseeds this Camera (and the fly-cam) from the map recipe.

## Picking and gizmos

- **Left click** an object to select it. The Outliner and Details panel follow the selection.
- **T / R / S** — translate, rotate, scale gizmos (W / E / R when you are not flying).
- **L** — light gizmos.
- Drag a gizmo axis to edit. **Ctrl+Z / Ctrl+Y** undo and redo transform and light edits.

## Viewport toolbar

Play / Pause / Stop sit on the main toolbar. The button beside Play fills the monitor with the 3D view (Esc restores the editor). The **Profiler** toggle on the viewport bar shows GPU timings, a pass-order **Graph**, and CPU zones over the scene (including Water prepass / reflection / shade). Camera speed is the slider next to the m/s readout. **Resolution** caps the 3D target (Native, 2560×1440, 1920×1080, 1600×900, 1280×720) while the window and UI stay at display pixels — pick **1920×1080** for fullscreen on a 2K panel. **FSR** (Post Processing, default on) temporally reconstructs that internal target to the window; it replaces TAA and the bilinear blit while enabled. Frame generation is not in the engine. `SOMNIUM_FSR=0` kills it at startup. Water and other transparents have no reactive mask yet, so they can ghost under camera motion.

### When the bar runs out of room

The bar holds four groups — camera speed, **Resolution**, the snapping cluster, and the day-cycle scrub — plus a reserved right-hand end that always keeps the overflow chevron and **Float**. When the viewport is too narrow for all four, whole groups are hidden in a fixed order:

1. **Snapping** goes first. The chevron that appears in its place opens the same controls.
2. **Camera speed** goes next. **RMB + scroll wheel** in the viewport still sets it.
3. **Resolution** and the **day cycle** go last, because neither has a second route.

A group is never half-hidden, and a label is never separated from the control it names. If you have seen "Resoluti" with its dropdown missing, that was the previous rule slicing the bar at its cell edge; it is now measured every layout, per group, including the bar's own 12 px inset on each side.

Widening the window, or floating the Outliner and Details panels out of the dock, brings the groups back in reverse order. The day-cycle scrub is the exception: it stays hidden on a scene with no Environment however much room there is, because there is no clock for it to scrub.

## Dynamic resolution

Off by default, and deliberately. It is the only control in the engine that
trades image quality for frame rate, so it is something you switch on with the
quality floor in front of you rather than something the engine starts doing
when a frame gets expensive.

Select the **Camera** entity in the Outliner and find the **Dynamic Resolution**
group in Details:

- **Dynamic Resolution** — on/off.
- **Dynamic Target Ms** — the frame time it aims at. 16.67 is 60 Hz.
- **Dynamic Floor** — the lowest scale it may choose, as a fraction of the
  **Resolution** preset above. 0.67 renders about 45% of the pixels; it will
  never go below this, so a busy view drops frames rather than going soft.

It only ever scales the internal 3D target. The window, the UI, gizmos and text
stay at display pixels. The controller has a ±10% dead band around the target
(a frame's own standard deviation is a few percent, and without a dead band a
controller chases its own noise), waits longer to raise the scale than to lower
it, and settles — once it has found a scale it stops changing anything.

The step grid is sixteenths, which is coarse on purpose: a change reallocates
every scene-sized render target. The alternative — rendering into a sub-rect of
a fixed target — needs every pass to become viewport-aware and is not in the
engine.

Lighting extras (world cache, scene specular, path tracer): see Help → **Lighting**. World cache is off by default; it adds bounce light, not frame-rate.

Water reflections: see Help → **Water**. Short version — Details on a water body has **SSR**, **RT Reflect**, and **Reflect Debug**; the Post Processing entity has **RT Reflections** and **RT Refraction** (off by default). `SOMNIUM_RT_REFLECT=0` restores SSR + sky cube.
