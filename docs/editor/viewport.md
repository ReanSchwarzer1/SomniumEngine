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

Lighting extras (world cache, scene specular, path tracer): see Help → **Lighting**. World cache is off by default; it adds bounce light, not frame-rate.

Water reflections: see Help → **Water**. Short version — Details on a water body has **SSR**, **RT Reflect**, and **Reflect Debug**; the Post Processing entity has **RT Reflections** and **RT Refraction** (off by default). `SOMNIUM_RT_REFLECT=0` restores SSR + sky cube.
