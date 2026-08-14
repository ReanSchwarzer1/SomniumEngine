# CR-A — occupancy (CPU vs GPU vs Task Manager)

**Status: MEASURED from DF-A + Task Manager observation** (2026-08-14).
This session did not re-run a maximized Native capture. GPU numbers are
[`phase DF/DF-A_timings.md`](../phase%20DF/DF-A_timings.md) at **2560×1392**,
release `hello_engine`, RTX 5080 Laptop, Vulkan. CPU % is the editor
screenshot that started the phase (Task Manager on `hello_engine`).

**Do not read 1.5% CPU as “the machine is unused.”**

## Occupancy truth

| Source | What it shows | Reading |
|---|---|---|
| Task Manager CPU | **~1.5%** on `hello_engine` | One thread waiting on GPU / present. Other cores idle because work is serial. |
| Task Manager RAM | **~553 MB** working set (screenshot) | Clipmap off is the default; clipmap on is ~96 MiB extra GPU. Do not allocate RAM to raise this number. |
| DF-A GPU, Native max, clipmap **off** | Overview shading **50.838 ms** / frame **71.376 ms** | GPU-bound. Shading dominates. |
| DF-A GPU, Native max, walk, clipmap **off** | Shading **49.677 ms** / frame **65.364 ms** | Same. |
| DF-A GPU, Native max, clipmap **on** | Overview shading **2.435 ms**; walk **10.652 ms** | Still GPU work, not a CPU bubble. Clipmap stays default **off** (DF-E). |
| Profiler CPU zones (after CR) | `Terrain`, `Instances`, `Cluster cull` | Cheap vs 50 ms shading. 256 AABB tests are microseconds. |

## What this decides

| Question | Answer |
|---|---|
| Is the GPU idle? | **No.** ~50 ms shading at maximized Native with clipmap off. |
| Should we raise Task Manager CPU %? | **Only if frame time drops.** Inflating % on 256 chunks is a failure. |
| Is `select_lods` / queue build on the critical path? | **No** at 256 chunks. CR-D therefore uses rayon only at **512+** items. |
| Are Shadows the reason to skip CR-E? | DF-A did not isolate the Shadows GPU row. Cascade-volume caster cull is still the right contract (never camera-only) and is cheap. |
| RAM? | Keep persistent `Vec` capacity (CR-F). Do not grow clipmap/caches. |

## How to remeasure live

1. Release `hello_engine`, maximize, **Native**, Profiler on.
2. Read GPU `Shading` / `Shadows` / frame vs CPU `Terrain` / `Instances`.
3. Profiler **terrain chunks** row is `vis / cpu-cull` after CR-B.
4. A/B Camera **Frustum Cull** and F10 (GPU 15B) separately.
5. Task Manager CPU % with the toggle on should stay low; **frame time** is the score.

Clipmap on/off is a DF measurement, not a CR goal.
