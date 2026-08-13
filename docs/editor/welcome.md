# Welcome

Somnium is a from-scratch 3D engine: a visibility-buffer renderer, an archetype ECS, and a native editor drawn with wgpu.

This window stays inside the editor. Press **F1** or the **?** button to open it, **Esc** or a click outside to close. Use the list on the left to jump between topics.

## What you can do here

- Fly the camera and pick objects in the centre viewport.
- Create lights, meshes, terrain, water, and post-fx from the **Create** menu.
  Water reflections (SSR / RT Reflect / Reflect Debug) live on the water body;
  **RT Reflections** and **RT Refraction** (off by default) are on the Post Processing entity.
- Import glTF/GLB with **File → Import Model**.
- Browse project files in the **Content Drawer** along the bottom.
- Sculpt terrain from the left **Sculpt** strip once a terrain is selected (F6).
- Play, pause, and stop the simulation from the toolbar. The button beside Play fills the screen; **Esc** restores the editor. Overlays hide while you play.

Nothing in Help is a web page. If a control looks like a button, it is a button — click it, it highlights, and it does the thing.

Help will gain topics as the engine does. Missing a page for a new panel usually means the chrome for that feature is still being written.
