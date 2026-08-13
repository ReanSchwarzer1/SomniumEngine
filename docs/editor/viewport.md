# Viewport & camera

The large centre pane is the 3D view. Chrome around it does not steal fly-cam input.

## Camera

- **Right mouse + WASD / QE** — fly. Hold **Shift** to go faster.
- **RMB + scroll wheel** — camera speed. The same value lives on the viewport toolbar slider (shown as m/s).
- The top-right of the title bar shows frame rate.

## Picking and gizmos

- **Left click** an object to select it. The Outliner and Details panel follow the selection.
- **T / R / S** — translate, rotate, scale gizmos (W / E / R when you are not flying).
- **L** — light gizmos.
- Drag a gizmo axis to edit. **Ctrl+Z / Ctrl+Y** undo and redo transform and light edits.

## Viewport toolbar

Play / Pause / Stop sit on the main toolbar. The button beside Play fills the monitor with the 3D view (Esc restores the editor). The **Profiler** toggle on the viewport bar shows GPU timings, a pass-order **Graph**, and CPU zones over the scene (including Water prepass / reflection / shade). Camera speed is the slider next to the m/s readout.

Lighting extras (world cache, scene specular, path tracer): see Help → **Lighting**. World cache is off by default; it adds bounce light, not frame-rate.

Water reflections: see Help → **Water**. Short version — Details on a water body has **SSR**, **RT Reflect**, and **Reflect Debug**; the Post Processing entity has **RT Reflections** and **RT Refraction** (off by default). `SOMNIUM_RT_REFLECT=0` restores SSR + sky cube.
