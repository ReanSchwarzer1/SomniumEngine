# Water & reflections

Select a **Water** entity. Details shows the Water Body section. The Great Lakes
defaults (level, depth, speed) are frozen authoring; reflections are the knobs
that change how the surface shows the world.

## Reflections

The surface tries a short screen-space march first. Where that misses, it uses
a ray-traced hit. Where the ray misses too, it uses the sky cube.

- **SSR** — how much the screen-space march contributes (0–1).
- **RT Reflect** — how much the traced hit contributes (0–1). Great Lakes default is 1.
- **Reflect Debug** — `0` off, `1` SSR hit (green) / miss (red), `2` colours the mix source (SSR blue, RT yellow, environment magenta).

Select the **Post Processing** entity and tick **RT Reflections** for the
scene-wide traced path. It sits with **RT Direct Light** and **RT Indirect (GI)**.

`SOMNIUM_RT_REFLECT=0` forces the previous SSR + sky-cube look, even if the
checkbox is on. Hardware without ray tracing does the same.

Water and transparent meshes are not in the ray-tracing scene, so you will not
see water reflecting water. That is expected.

## Profiler

With the viewport Profiler on, water splits into **Water prepass**, **Water
reflection**, and **Water shade**.
