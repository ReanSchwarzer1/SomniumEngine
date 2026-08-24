# MORROWIND-A — the license audit

`phase_MORROWIND.md` §6.6 names three references that are not permissive and
adds a fourth open question: *"Flax's license is source-available and must be
checked by MORROWIND-A before §6.2's surface work begins, since this plan leans
on Flax heavily and 'widely read' is not the same as 'permissive'."*

Checked 2026-08-24 by reading the license file at the root of each reference
tree in `C:\Users\adhir\Downloads\GE\example_repo\`. The path and the first
lines are quoted so the finding can be re-run and disagreed with.

## The Flax question, resolved

**`FlaxEngine-master/LICENSE.md`, in full:**

> Use of the Flax, provided source code and binary files is governed by the
> terms of the Flax Engine End User License Agreement, which can be found at
> https://flaxengine.com/licensing

**Verdict: proprietary, source-available. Flax joins Unreal in §6.6's strict
tier, and the plan's table is wrong to imply otherwise.**

This matters because §6.5 and §6.2 lean on Flax specifically:

- **§6.5, Seam 4a** takes Flax's `UICanvas` model — a UI root that declares its
  space — as the reference for MORROWIND-E.
- **§6.2** cites `FlaxEngine/Source/Editor/Surface/` as a reference for the one
  graph surface (Seam 8a, MORROWIND-K).

Neither is blocked, because **neither is a transcription**. "A UI tree's root
declares whether it is screen-space, world-space or overlay" is an architectural
idea, not an expression of one; Unity's `Canvas.renderMode`, Godot's
`CanvasLayer`/`SubViewport` split and Unreal's `WidgetComponent` all express the
same idea, and the last two are permissive. The rule that follows is the same
one §6.6 states for Unreal, applied to Flax as well:

> Read for architecture only. Do not reproduce identifiers, file structure,
> comments, constants or shader code. Describe the *technique* and implement
> from a permissive reference or the public literature wherever one exists.
> Cite the technique, not the file, in shipped code comments.

**Two concrete consequences for later sub-phases:**

1. **MORROWIND-E must not name `UICanvas`, `CanvasScaler` or Flax's render-mode
   enum variants in shipped code.** Seam 4a's own names —
   `Canvas::Screen { scaler }` / `World { .. }` / `Overlay { camera }` — are
   already different, and that is the state to keep.
2. **MORROWIND-K should read Godot's `GraphEdit`/`GraphNode` (MIT) and Fyrox's
   `absm/` (MIT) as its *primary* surface references**, with Flax's
   `Source/Editor/Surface/` demoted to a secondary read. §6.2 has this the other
   way round and it should not.

`ATTRIBUTION.md` §13G already cites Flax for CONTROL-E's typed drag payloads and
long-operation jobs. That citation is architectural and stays; this audit does
not require revisiting a shipped phase.

## The full tier table, as measured

| Reference | License file | License | Tier | Rule |
|---|---|---|---|---|
| `UnrealEngine-release` | Epic EULA | **Proprietary** | **Strict** | §6.6 as written. Architecture only; implement from public literature. |
| `FlaxEngine-master` | `LICENSE.md` | **Proprietary (Flax EULA)** | **Strict — added by this audit** | As Unreal. See above. |
| `Daemon-master` | `LICENSE.txt` (BSD-3-Clause) **but see below** | **GPL-2.0-or-later for the file that matters** | **Strict** | The root license is BSD-3-Clause; `src/engine/renderer/gl_shader.cpp:1–21` carries its own GPL-2.0-or-later header, and `GPL.txt` sits beside `COPYING.txt` at the root. **The root file does not govern the file MORROWIND-C wants to read.** Read only; `terra-main/rshader` (Apache-2.0) is the primary reference instead. |
| `luanti-master` | `LICENSE.txt` | **LGPL-2.1+** (code; assets vary) | **Strict** | Relevant to Track 4's block emerge and streaming. Read only. |
| `fyrox/Fyrox-master` | `LICENSE.md` | **MIT** | Permissive | Patterns, cited. The MORROWIND-A diff is built on this. |
| `Esoterica-main` | `LICENSE.md` | **MIT** (Bobby Anguelov, 2022–2024) | Permissive | Track 5's animation node list (§6.3). |
| `stride-master` | `LICENSE.md` | **MIT** (.NET Foundation) | Permissive | GHOSTFENCE's golden-image model (§10). |
| `WickedEngine-master` | `LICENSE.txt` | **MIT** | Permissive | |
| `godot-4.7.1-stable` | `LICENSE.txt` | **MIT** | Permissive | Editor conventions; the graph surface, per this audit. |
| `o3de-development` | `LICENSE.txt` | **Apache-2.0 or MIT**, at the user's option | Permissive | Seam 1's deadline/priority contract; Track 4's streaming. |
| `terra-main` | `LICENSE` | **Apache-2.0** | Permissive | `rshader/` — MORROWIND-C's primary reference. |
| `bevy/bevy-main` | `LICENSE-APACHE` + `LICENSE-MIT` | **Apache-2.0 or MIT** | Permissive | `bevy_tasks` for Seam 1's Rust shape; pipeline specialisation for Seam 3. |
| `korge-main` | `LICENSE` | **Mixed, per-library; typically MIT / Apache-2.0 / public domain** | Permissive **with a caveat** | §6.9.3's out-of-process play-in-editor. The root file says "Each library has its own licenses"; **MORROWIND-N must check the specific subtree it reads**, not this file. |

## The three rules a sub-phase needs to remember

1. **Five references are strict, not three.** Unreal, Flax, Daemon, Luanti —
   and Korge is per-subtree, so treat it as strict until the subtree is checked.
   Two of those five were found by *not* trusting the root license file: Flax's
   root file is a pointer to a EULA, and Daemon's root file is BSD over a tree
   whose relevant file is GPL. **Check the header of the file you are actually
   reading**, which is the only rule in this document that generalises.
2. **A strict reference never appears in a shipped code comment as a file
   path.** Cite the technique and a permissive or public source for it. The
   existing convention in `crates/somnium_ui/src/widgets/canvas.rs:1`
   (`// Port of: example_repo/fyrox/...`) is correct *because Fyrox is MIT*; the
   same line naming a Flax or Unreal path would not be.
3. **`ATTRIBUTION.md` §13H is where MORROWIND's citations go.** §13E and §13F
   belong to Phase 27 and §13G to Phase CONTROL; none of them is edited by this
   phase.
