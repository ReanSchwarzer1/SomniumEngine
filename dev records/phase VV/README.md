# Phase VV evidence (Halcyon)

Place **tonemapped** captures here after a live play session. Do not invent
PNGs or copy HDR dumps — the renderer’s HDR target is blown out as a PNG.

Suggested names (after tonemap):

| Filename | What to show |
|---|---|
| `phase_VV-A_ssr_debug.png` | Reflect Debug = 1 (SSR hit green / miss red) |
| `phase_VV-G_source_debug.png` | Reflect Debug = 2 (SSR blue, RT yellow, env magenta) |
| `phase_VV-G_before.png` | `SOMNIUM_RT_REFLECT=0` (SSR + sky cube) |
| `phase_VV-G_after.png` | RT Reflections on, Great Lakes default |

Code for VV-A–H is already in the tree. This folder is only the visual record.
Plan and remaining work: [`../phase_VV.md`](../phase_VV.md) §11 and §13.
Start-here: [`../halcyon_context_handoff.md`](../halcyon_context_handoff.md).
