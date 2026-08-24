# Phase MORROWIND — NetImmerse

> *You can walk from Seyda Neen to Dagoth Ur without a loading screen, and
> almost none of what makes that walk worth taking is in the renderer.*

> **Codename:** MORROWIND (Bethesda, NetImmerse 4.x, 2002). Chosen because the
> point is load-bearing rather than decorative: **Morrowind's renderer was bought
> off the shelf.** NetImmerse was middleware — a scene graph, a rasteriser, a
> skinning path. Everything anyone remembers about Morrowind is the part
> Bethesda had to write on top of it: a province that streams as you walk, an
> interior set kit-bashed from modular prefabs, a Construction Set shipped in
> the box, a journal-and-dialogue UI that *is* the game, AI packages, save
> games, and a data format the whole thing is authored in.
>
> Somnium is in the mirror-image position. It has spent twenty-odd phases
> building the part Bethesda bought, and has almost none of the part Bethesda
> built. **This phase is that part.**
>
> **Status:** **PLAN — nothing in tree.** No sub-phase started. Written
> 2026-08-23 against the tree at `7c0b66f` on `dev`. Every count in §4 was
> measured on that date by the commands quoted beside it; where a figure
> disagrees with an earlier document the difference is stated, not silently
> corrected.
>
> **Position in the roadmap:** this phase supersedes `context.md` §17.6's
> numbered list from **Phase 30 onward**, and absorbs the never-started parts of
> the §17.6 entries for 27 (animation), 28 (asset pipeline), 31 (particles) and
> 33 (input/localization/video). See §1.3 — **the §17.6 numbering has collided
> with reality and this phase is where that gets resolved.**
>
> **Predecessor:** Phase 26 (Metaphor) + 26-Zeta (Nocturne Atelier) built the
> editor's information architecture and token layer. Phase 27 (Hades) rebuilt
> its paint layer. **Phase CONTROL (Northlight) builds the editor's *reach* —
> the property seam, the asset seam, drag and drop, viewport control,
> preferences, scene lifecycle, curve and gradient editing, time of day, clouds
> and weather.** MORROWIND is gated on CONTROL for six of its eight tracks and
> says so in §9. **Decision, taken 2026-08-23 and recorded in both documents:
> Phase CONTROL runs first and completes in full — Tracks 2 and 3 included —
> and Phase MORROWIND is next.** See [`phase_CONTROL.md`](phase_CONTROL.md)
> §9.1, which states the same decision from the other side and closes CONTROL's
> "reasonable stopping point after J" as an exit. This is stronger than the
> "CONTROL-B and CONTROL-C in tree" floor this plan was originally drafted
> against, and it removes §12.8's risk entirely: CONTROL's six seams will be
> settled code rather than a moving plan before anything here is built. It also
> secures a dependency the weaker floor did not — **CONTROL-K's curve and
> gradient editors**, which MORROWIND-H, MORROWIND-L and MORROWIND-AG all
> require and which sit in CONTROL's Track 2, past the old stopping point.
> Phase PORTAL (Source) is the engineering-health phase and is orthogonal;
> §9.4 states how they interleave.
>
> **Record:** this file. Evidence folder `dev records/phase MORROWIND/` is
> created by **MORROWIND-A**, not before. **Do not invent PNGs.**
>
> **Do not copy source** from Unreal, O3DE, Flax, Wicked, Esoterica, Stride,
> Fyrox, Bevy, Godot, Unity, Daemon, Luanti, Ren'Py, Panda3D, jMonkeyEngine,
> Korge, terra, or any other repository named here. Patterns only, cited in
> `ATTRIBUTION.md` **§13H** — note the letter: §13E and §13F belong to Phase 27,
> §13G to Phase CONTROL. Two of the references in §6 are **copyleft** (Daemon is
> GPL, Luanti is LGPL) and one is **proprietary** (Unreal, under its EULA);
> §6.6 states the rule for those three specifically and it is stricter than the
> rule for the permissive references.

**Frozen by this phase** — a MORROWIND sub-phase that changes any of these has
gone wrong, and the containment gate in §10 (**GHOSTFENCE**) is the mechanism
that catches it:

- The Hades paint contract (phase 27 §6): the 100-byte `Primitive` instance
  layout, the colour contract, `draw_over` ordering, block-origin text snapping.
  **MORROWIND-D *extends* this by adding a second instance stream; it does not
  edit the existing one.** See §7, Seam 4.
- The Nocturne token sheet and its certified contrast pairs (Zeta §8A.3); the
  five bundled type cuts and the `FontRole`/`TextRole` split.
- CONTROL's six seams, once landed. MORROWIND adds seams; it does not
  renegotiate CONTROL's.
- The XV 32-layer terrain contract and `GpuTerrainMaterial` layout; Great Lakes
  water numbers; foliage LOD and cull distances.
- DOOM's measured defaults: dynamic resolution opt-in, tile binning and aerial
  terrain default off, hex/POM default off.
- The visibility-buffer pipeline's existing pass order and its GPU timings.
  **Every rendering sub-phase in Track 7 must show a `.somtime` row proving the
  frame did not regress on the two shipped maps.**
- rustc 1.88, **wgpu 30**, winit 0.30. **MORROWIND-A2 landed the bump on
  2026-08-24** (wgpu 29.0.3 -> 30.0.1); the line above is the post-A2 state and
  the pre-A2 line read wgpu 29. The freeze is a rule against *unannounced*
  change, not against moving, and it is now enforced rather than asserted:
  `FROZEN_TOOLCHAIN` in `tools/ghostfence/run.py` is the single source and the
  gate's `toolchain` row fails when any manifest disagrees with it. A later
  sub-phase that bumps anything edits that dict in the same commit.

**The rule this phase is judged by, stated once — the runtime rule:**

> A system that only the editor can drive is not an engine system. Every
> sub-phase below closes with a named artefact that a **game** — not the editor
> — can use: a public API in a `somnium_*` crate, a Luau binding, and a line in
> `examples/`. A sub-phase that adds an editor panel without adding the runtime
> capability underneath it has failed, however good the panel looks; and a
> sub-phase that adds a runtime capability the editor cannot author has failed
> CONTROL's reachability rule, which this phase inherits and does not repeal.

**And its sibling — the second-example rule:**

> `examples/` contains exactly one program, `hello_engine`, at 2,646 lines, and
> it is simultaneously the demo, the test harness, the input layer and the game.
> **Every track in this phase adds to a second example** — a small playable
> vertical slice, `examples/vvardenfell` — built only from public crate APIs. If
> a track cannot be exercised from that example without reaching into engine
> internals, the track's API is wrong. This is the only mechanism in the plan
> that reliably catches an engine/game boundary that does not exist, and §4.7
> shows that Somnium's boundary does not currently exist.

---

## 0. How to use this document (handoff)

This file is long because the phase is eight tracks wide and because a later
session must be able to start cold. Read in this order before writing code:

1. **This file**, all of it. Especially §4 (the measured audit that motivates
   the phase), §5 (what the 2026 bar actually is), §7 (the eight seams — do not
   relitigate them), §8 (sub-phases), §9 (sequencing — **this phase is not meant
   to be executed front to back**), §10 (GHOSTFENCE), §15 (start checklist).
2. [`phase_CONTROL.md`](phase_CONTROL.md) **in full.** MORROWIND is built on
   CONTROL's six seams and duplicates none of its sub-phases. §6.7 of this file
   is the explicit non-overlap table; if you find yourself planning a curve
   editor, a gradient editor, a preferences window, a keybinding editor, time of
   day, volumetric clouds or weather, **stop — that is CONTROL, not this.**
3. [`phase_27.md`](phase_27.md) §6 (the render contract) and §12
   (must-not-break). Track 1 extends that contract and must not break it.
4. [`phase_DOOM.md`](phase_DOOM.md) §15 — the measured defaults and the
   `.somtime` harness. Track 7 lives or dies by that harness.
5. [`context.md`](../context.md) §6 (the visibility-buffer pipeline), §8
   (`somnium_ui`), §11 (`somnium_asset`), §12 (frame execution order), §17.6
   (the roadmap this phase supersedes), §18 (known issues).
6. `crates/somnium_ui/src/primitive.rs` and `crates/somnium_ui/src/pass.rs`,
   **end to end**. Track 1 is unreadable without them.
7. `crates/somnium_renderer/src/geometry.rs`, `meshlet.rs`, `culling.rs` and
   the visibility pass — Track 5's skinning integration lands there and nowhere
   else.
8. **[Appendix A](#appendix-a--implementation-reference)**, at the end of this
   file — the implementation layer, added 2026-08-23 for exactly the case where
   a different session or model picks this up cold. A.1 is a reading order over
   the real tree with line counts; A.2 is a glossary of the Somnium-specific
   words a general model will guess wrongly; A.3 expands every seam into Rust
   and WGSL sketches written against the **actual** types in `reflect.rs` and
   `primitive.rs`; A.4 is a file-by-file change map; **A.5 is the three
   integrations that carry real risk**; A.6 is the one place this document and
   `phase_CONTROL.md` would otherwise collide; A.7 is how to tell a track is
   genuinely finished rather than plausibly finished.
   **If you are starting cold, read §1, §4, §7, then all of Appendix A, and only
   then §8.**

**Authorized work:** new crates (§7.9 lists the ones this phase creates), the
runtime halves of `somnium_ui`, `somnium_asset`, `somnium_audio`,
`somnium_core`, the renderer additions each Track 7 sub-phase names, and the
build tooling under `tools/`.

**Not authorized:** re-theming anything Zeta certified or Hades painted;
retuning terrain, water or foliage; a second reflection system; a second UI
framework; or a rewrite of the visibility buffer.

**Update `context.md` and `ATTRIBUTION.md` after every completed sub-task**, per
`user_profile.md`. `ATTRIBUTION.md` §13H is opened by MORROWIND-A.

---

## 1. Executive decision

### 1.1 The finding

Somnium is **113,892 lines of Rust and WGSL across eleven crates**, and
**85.1% of it is three crates**: the renderer (50,206), the UI (27,530) and
core (19,220). The remaining eight crates together are 16,936 lines. One of
them, `somnium_audio`, is **93 lines**, three of whose six files are literally
one-line stubs (`// Bus stub`, `// Listener stub`, `// Error stub` —
`crates/somnium_audio/src/bus.rs:1` and siblings).

This is not a complaint about balance for its own sake. It is the specific
observation that **Somnium can render an open world and cannot run one.** There
is no skeletal animation (zero occurrences of `bone` or `armature` in the tree),
no navigation (zero occurrences of `navmesh` or `pathfind`), no prefabs, no
input abstraction (zero occurrences of `gamepad` or `action_map`; sixteen
`KeyCode::` arms sit inline in the example app), no localization (zero
occurrences of `localiz`), no save system, no asset cook, no streaming, and no
way for a *game* built on Somnium to draw a single button.

### 1.2 The decision

**Build the engine half.** Eight tracks, thirty-six sub-phases, each of which
would be a defensible phase on its own. Three of them — Track 0 (foundations),
Track 1 (the runtime UI) and Track 4 (the cook and streaming) — are
prerequisites for most of the rest and are the recommended cut if only part of
the phase is executed. §9.3 states that cut precisely.

The phase is deliberately larger than one session, one month, or plausibly one
year at the cadence this project has run at. **That is intentional and stated
rather than hidden.** A plan that pretends thirty-six sub-phases fit in a
sprint produces a worse outcome than one that lays out the whole shape and
names its own critical path. §9 is the part to argue with; §8 is the part to
build from.

### 1.3 The roadmap collision, resolved

`context.md` §17.6 lists "Phases 26–33 — the systems Somnium does not have" and
then adds 34–38. **That numbering no longer describes reality:**

| §17.6 says | What actually shipped under that number |
|---|---|
| Phase 26 — UI framework (retained tree, layout, SDF text, world-space canvases) | Phase 26 **Metaphor** — the *editor's* information architecture. No game UI, no world-space canvas, no shaping. |
| Phase 27 — Skeletal animation | Phase 27 **Hades** — the editor's paint layer. Zero animation. |
| Phase 28 — Asset pipeline | Not started. |
| Phase 29 — Profiler | Partially shipped (GPU timestamps, CPU zones, frame-graph row). |
| Phase 30–38 | Not started; 34–38 were appended later without renumbering. |

The numbers 26 and 27 are **spent**. Re-using them for animation and the asset
pipeline now would make `dev records/` unreadable. **Decision: the §17.6
numbering is retired.** From here, systems work is planned under codenames, as
DOOM, CR, DF, VV, XV, PORTAL and CONTROL already are. MORROWIND-A updates §17.6
in `context.md` to say so and maps each old number to the track that absorbs it:

| Old | Absorbed by |
|---|---|
| 26 (runtime UI framework) | **Track 1 — VIVEC** |
| 27 (skeletal animation) | **Track 5 — DWEMER** |
| 28 (cook, hot reload, streaming) | **Track 4 — SILT STRIDER** |
| 29 (profiler) | Stays as-is; Track 7 extends it, Phase PORTAL owns the harness |
| 30 (navigation and AI) | **Track 6 — SIXTH HOUSE** |
| 31 (GPU particles and VFX) | **Track 7 — RED MOUNTAIN**, MORROWIND-AA |
| 32 (networking) | **Explicitly out of scope**, §3.1 |
| 33 (input, localization, video) | **Track 8 — ALMSIVI** |
| 34 (prefabs) | **Track 3 — HLAALU** |
| 35 (rule-driven scattering) | **Track 3 — HLAALU**, MORROWIND-P2 |
| 36 (cinematics and sequencer) | **Track 2 — CONSTRUCTION SET**, MORROWIND-L |
| 37 (cloth and hair) | Deferred; §14.3 states why |
| 38 (game framework) | **Track 8 — ALMSIVI** |

### 1.4 The counter-argument, and the answer

The honest objection is that Somnium's renderer is its differentiator and this
phase spends a year on things every engine already has. Three answers:

1. **The differentiator is unusable.** A visibility buffer with ReSTIR GI that
   can only ever draw a static scene with no animated characters, no UI, and no
   way to ship is a technology demo. The renderer's value is *realised* by this
   phase, not diluted by it.
2. **One of the eight tracks is rendering work** (Track 7) and one more is a
   rendering prerequisite (Track 0's shader permutation system, which the
   renderer needs regardless: 48 WGSL files, 12,079 lines, and a
   `MaterialSystem` that has been a 29-line stub since it was written —
   `crates/somnium_renderer/src/material/hlms.rs:14`).
3. **The remaining tracks are what makes the rendering work testable at scale.**
   There is currently no way to put ten thousand animated agents in a streamed
   world and find out whether the culling holds up, because there are no agents,
   no animation and no streaming.

---

## 2. Goals

1. **A game can have a UI.** A screen-space and a world-space canvas, anchors,
   focus and gamepad navigation, shaped and localised rich text, arbitrary
   textures, and a motion system — all reachable from Luau and from Rust,
   without touching the editor.
2. **A character can move.** Skinned meshes with GPU skinning that reaches the
   visibility buffer, clips, blend trees, a state machine with synchronised
   blending, root motion, IK and animation events.
3. **A world can be bigger than memory.** An offline cook, a content-hashed
   native format, an async job system with deadlines, a residency budget, cell
   streaming with named streaming sources, HLOD and impostors, and a floating
   origin.
4. **A world can be composed rather than typed.** Prefabs with nested instances
   and overrides; rule-driven scattering; blockout geometry; spline authoring.
5. **A world can be inhabited.** Navmesh generation from level geometry,
   pathfinding with funnel smoothing, agent avoidance, and a behaviour-tree
   runtime.
6. **The editor can author all of it.** One graph surface and one timeline
   widget, each reused by every feature that needs them; docking and multiple
   viewports; virtualised lists that survive 100k rows; play-in-editor.
7. **The engine has a boundary.** A second example program, built only from
   public crate APIs, that is a small playable slice rather than a demo harness.
8. **Nothing already shipped regresses.** GHOSTFENCE (§10) is the mechanism.

---

## 3. Non-goals

1. **No networking.** §17.6's Phase 32 stays unbuilt. This phase must not
   *preclude* it — §7's Seam 6 prefab identity model and Seam 2's asset identity
   model are both chosen to be network-compatible — but no replication,
   prediction or transport is written. Rationale in §14.1.
2. **No visual scripting language.** Somnium has Luau, complete, as of Phase 16.
   Track 2's graph surface is built and its first four consumers are materials,
   animation, VFX and behaviour trees. A fifth consumer that compiles to Luau is
   named as *possible* in §14.2 and deliberately not planned.
3. **No renderer rewrite.** No render-graph refactor of the existing pass list.
   §5.4 argues the case against one for an engine at Somnium's size, and §7's
   Seam 3 delivers most of what people want a render graph for.
4. **No re-theming, no re-painting.** Phase 26, 26-Zeta and 27 are closed
   surfaces.
5. **No cloth, hair or fur.** §14.3.
6. **No motion matching in this phase.** Track 5 stops at IK and root motion and
   states the data it would need to add motion matching later (MORROWIND-W2).
7. **No console or mobile port.** wgpu makes it plausible; nothing here assumes
   it.
8. **No second UI framework.** Track 1 grows the existing retained tree. If a
   sub-phase finds itself writing an immediate-mode layer beside it, that
   sub-phase has gone wrong.

---

## 4. The engine census, measured 2026-08-23

Every figure below was produced on the tree at `7c0b66f`. The command is quoted
so it can be re-run and disagreed with. **MORROWIND-A turns this section into a
checked-in script**, on the DOOM-A and CONTROL-A precedent, because a hand-typed
audit rots in a week.

### 4.1 The shape of the codebase

`find crates -name '*.rs' -o -name '*.wgsl' | xargs wc -l`, summed per crate:

| Crate | Lines | Share | Tests (`#[test]`) |
|---|---:|---:|---:|
| `somnium_renderer` | 50,206 | 44.1% | 328 |
| `somnium_ui` | 27,530 | 24.2% | 215 |
| `somnium_core` | 19,220 | 16.9% | 217 |
| `somnium_script` | 4,815 | 4.2% | 55 |
| `somnium_script_luau` | 4,457 | 3.9% | 58 |
| `somnium_ecs` | 4,018 | 3.5% | 54 |
| `somnium_asset` | 1,639 | 1.4% | 6 |
| `somnium_voxel` | 1,000 | 0.9% | 11 |
| `somnium_physics` | 580 | 0.5% | 1 |
| `somnium_physics_sys` | 334 | 0.3% | 0 |
| `somnium_audio` | **93** | **0.08%** | **0** |
| **Total** | **113,892** | | **945** |

Plus `examples/hello_engine/src/main.rs` at **2,646 lines** — the only example
in the repository.

Read the last four rows together. **Physics, physics FFI and audio are 1,007
lines and one test between them.** Somnium links Jolt, one of the most capable
physics libraries in existence, through a 236-line `world.rs`
(`crates/somnium_physics/src/world.rs`) — which means character controllers,
vehicles, ragdolls, soft bodies and constraints are all *already paid for* and
none is exposed. That is the cheapest capability in this entire document and it
is called out again in Track 5 and Track 8.

### 4.2 `somnium_audio` is a stub, in the literal sense

```
crates/somnium_audio/src/bus.rs        1 line:  // Bus stub
crates/somnium_audio/src/error.rs      1 line:  // Error stub
crates/somnium_audio/src/listener.rs   1 line:  // Listener stub
crates/somnium_audio/src/engine.rs    49 lines
crates/somnium_audio/src/sound.rs     36 lines
crates/somnium_audio/src/lib.rs        5 lines
```

`AudioEngine::play` (`crates/somnium_audio/src/engine.rs:35`) loads a file from
disk, on the calling thread, every time it is called, and returns a handle. It
builds a `StaticSoundSettings` with the requested volume into a variable named
`_kira_settings` and **then does not use it** (`engine.rs:36`) — the volume
argument is silently discarded. There is no bus, no listener, no
spatialisation, no attenuation, no reverb, no streaming, no mixer, and no
cache. A file played twice is read twice.

For an open-world RPG this is the single most conspicuous hole after animation,
and it is also the smallest: Kira already provides tracks, effects and spatial
scenes. Track 8 (MORROWIND-AG) is mostly *exposure*, not implementation.

### 4.3 The renderer has no shader system

48 WGSL files, **12,079 lines**, from `present.wgsl` at 30 lines to
`shading.wgsl` at 1,750. There is no permutation system, no variant key, no
hot reload and no shared preprocessor. `crates/somnium_renderer/src/material/hlms.rs`
is **29 lines**, of which the substantive part is:

```rust
pub struct MaterialSystem {
    /// Cached pipelines mapped by their configuration hash.
    _pipeline_cache: HashMap<u64, wgpu::RenderPipeline>,
}
```

— an underscore-prefixed field that no code reads, under a doc comment
describing Ogre-Next's HLMS and a trailing comment beginning *"In a full
implementation, this would…"* (`hlms.rs:26`). The reference architecture was
documented and never built.

This matters far beyond tidiness. **Skinning is a permutation.** So is
instancing, so is alpha cutout, so is two-sided shading, so is every
lighting-model variant Track 7 wants to add. Track 5 cannot integrate skinned
meshes into `shading.wgsl` without either a permutation system or a fifth
uber-shader branch in a file that is already 1,750 lines. **MORROWIND-C is
therefore a hard prerequisite for Track 5 and most of Track 7**, and is the
reason Track 0 exists at all.

### 4.4 There is no job system

`crates/somnium_renderer/src/jobs.rs` is **75 lines**. It contains one function,
`for_each_mut`, which calls `rayon::par_iter_mut` when a slice exceeds
`PARALLEL_THRESHOLD = 512` (`jobs.rs:14`) and iterates serially otherwise
(`jobs.rs:18`). Its own doc comment is candid about scope: *"Parallel work is
CPU-side only… Record still happens on the render thread"* (`jobs.rs:3`).

There is exactly **one** `std::thread::spawn` in the crates directory. There is
no thread pool with priorities, no deadline, no cancellation, no completion
queue drained on the main thread, and no way to express "this work may take
250 ms and must not stall the frame."

Every remaining track needs that: thumbnail decode (CONTROL-C measured a
232–260 ms zlib inflate on the largest terrain PNG — `phase_CONTROL.md` §4.2),
asset cooking, cell streaming, navmesh baking, lightmap baking, mesh import,
and shader compilation. **MORROWIND-B is the other reason Track 0 exists.**

### 4.5 The UI cannot draw a game

`crates/somnium_ui/src/primitive.rs:63` defines the sole instance type, asserted
at 100 bytes (`primitive.rs:89`) across twelve vertex attributes
(`primitive.rs:92`). Its fields are `rect`, `uv`, `radii`, `shadow`,
`grad_axis`, `border_width`, `expand`, four packed colours and `flags`.

What follows from that layout is the whole of Track 1's problem statement:

- **There is no transform.** Every primitive is an axis-aligned rectangle in
  screen pixels. No rotation, no scale, no skew, no arbitrary matrix. A rotated
  health bar, a radial menu, a zoomable node graph, a curve editor's bezier
  handles and a world-space quest marker are all, today, *inexpressible*.
- **There is no stroke.** `border_width` is an inset band on a rounded box.
  There is no line, no polyline, no bezier, no join, no cap, no dash. A node
  graph's wires and a curve editor's splines cannot be drawn.
- **Clipping is a rectangle.** `push_clip_rect` (`draw.rs:78`) takes a `Rect`.
  Circular avatars, rounded panel clipping and any masked reveal are out.
- **Gradients are one linear axis** (`grad_axis`, two floats). No radial, no
  angular, no multi-stop.
- **There is no render-to-texture.** A sub-scene, a minimap, a material preview
  in a graph node, or a picture-in-picture viewport cannot be composed.

And `crates/somnium_ui/src/pass.rs` binds **exactly three textures**: the font
atlas (`pass.rs:226`), the icon atlas (`pass.rs:242`) and the thumbnail atlas
(`pass.rs:259`). **There is no path by which a game supplies its own texture to
the UI.** A sprite, a portrait, an item icon, an inventory grid or a video panel
cannot be drawn at all — not slowly, not at all.

`push_nine_slice` exists (`draw.rs:360`) and takes a `texture_id`, so the
*intent* is present; there is simply no texture for it to reference.

### 4.6 What is absent, by grep

`grep -ril <term> crates --include=*.rs --include=*.wgsl`, on 2026-08-23:

| Term | Files | Reading |
|---|---:|---|
| `bone`, `armature` | **0**, **0** | No skeletal animation of any kind. |
| `skin` | 8 | All false positives (`asking`, `masking`) **except one**: `material/hlms.rs:8` names skinning as a hypothetical permutation key. |
| `navmesh`, `pathfind` | **0**, **0** | No navigation. |
| `gamepad`, `action_map` | **0**, **0** | No input abstraction. Sixteen `KeyCode::` arms inline in `examples/hello_engine/src/main.rs`; fifty-four more in `somnium_core/src/script_input.rs`. |
| `localiz` | **0** | No localization. |
| `state_machine` | **0** | No animation or AI state machines. |
| `prefab` | 2 | Both are comments in the *scripting* crate about a hypothetical self-spawning prefab (`somnium_script/src/runtime.rs:39`). No prefab system. |
| `dock` | 5 | An unused `IconId::Dock` (`icons.rs:58`) and a comment calling the fixed content drawer "docked" (`editor/shell.rs:927`). No docking system. |
| `accessib` | 1 | A doc comment about *script*-accessible fields. No accessibility. |
| `ninepatch` / `nine_slice` | 0 / 1 | The draw call exists; nothing can feed it. |

### 4.7 The engine/game boundary does not exist

`examples/hello_engine/src/main.rs` is 2,646 lines. It is simultaneously the
demo, the manual test harness, the input layer, the camera controller and the
scene setup. It is also the *only* consumer of the public API, which means the
public API has never been tested against a second use.

Two symptoms:

- **The workspace declares `egui`, `egui-wgpu` and `egui-winit`**
  (`Cargo.toml:83–85`) and `grep -rn egui crates --include=*.rs` returns
  **nothing**. A dead dependency triple survives because nothing forces the
  dependency list to justify itself. (Phase PORTAL's CI gates would catch this;
  noted here as evidence, and left to PORTAL to fix.)
- **`somnium_audio` is a workspace member with zero tests and one caller**, and
  the fact that `AudioEngine::play` discards its volume argument (§4.2) has
  survived because no second program has ever asked it to be quieter.

The second-example rule in the preamble exists because of this section.

### 4.8 Component schemas: twelve

`crates/somnium_core/src/reflect_registry.rs:342–353` registers exactly twelve
component schemas: foliage, light, material, mesh, mesh-kind, name, parent, one
further schema at `:349`, terrain, transform, voxel-terrain and water.
CONTROL-B generates inspector rows from these. **Every system this phase adds
must ship its schema in the same sub-phase**, or CONTROL's reachability rule is
violated on arrival. §11 makes that an acceptance row rather than an aspiration.

### 4.9 `SOMNIUM_*` variables: 96 (or 97)

`grep -rhoE 'SOMNIUM_[A-Z0-9_]+' crates examples --include=*.rs | sort -u | wc -l`
returns **96** on 2026-08-23. `phase_CONTROL.md` reports **97** as of
2026-08-22. The difference is method, not tree: the counts include different
directories. **CONTROL-A's generated table is authoritative**; this row exists
only so the two numbers do not look like a regression to a later reader.

---

## 5. What "an engine" means in 2026, and where Somnium stands

### 5.1 The market bar, honestly stated

Somnium's renderer is genuinely competitive. Feature-for-feature against Flax,
Wicked or Stride it holds — a visibility buffer with meshlet culling, ReSTIR
DI and GI, an FFT ocean, a 32-layer terrain material and FSR 3 is a better
rendering feature set than most open-source engines ship. `context.md` §17.6
reached that conclusion and it still holds.

The bar it fails is different, and it is not a rendering bar. It is the one a
person hits in the first hour:

| Ask | Unity | Godot | Flax | Somnium |
|---|---|---|---|---|
| Put a button on screen from game code | yes | yes | yes | **no** |
| Play a walk cycle on a character | yes | yes | yes | **no** |
| Drop the same rock in 50 places and edit it once | yes | yes | yes | **no** |
| Rebind jump to a gamepad button | yes | yes | yes | **no** |
| Save and reload a game | yes | yes | yes | **no** |
| Make an NPC walk around a wall | yes | yes | yes | **no** |
| Ship a build without re-parsing 101 MB of source assets | yes | yes | yes | **no** |
| Position a sound in the world | yes | yes | yes | **no** |
| Two floating panels side by side in the editor | yes | yes | yes | **no** |

Nine rows, nine noes. Every one is a track in §8.

### 5.2 The rendering bar, where Somnium is genuinely behind

Track 7 is not a catch-up list; most of the catch-up is done. Five specific
gaps remain, and each is named with the reference that solves it:

1. **Shadows do not scale.** Cascaded shadow maps with PCF are a 2012 answer.
   An open world with many shadow-casting lights needs a page-cached virtual
   shadow map. Reference:
   `UnrealEngine-release/Engine/Source/Runtime/Renderer/Private/VirtualShadowMaps/`
   — nine files, of which `VirtualShadowMapCacheManager.h` (the invalidation
   model) and `VirtualShadowMapClipmap.h` (the directional-light case) are the
   two that decide the design. **Proprietary — pattern only, §6.6.**
2. **Particles are a CPU list.** `Renderer::set_particles` takes a
   `Vec<GpuParticle>` and re-uploads it every frame
   (`crates/somnium_renderer/src/renderer.rs:1296`, consumed at `:3660`). No
   compute simulation, no sorting, no depth collision, no ribbons, no mesh
   particles, no authoring. References: `WickedEngine-master/WickedEngine/wiEmittedParticle.cpp`,
   and O3DE's `Gems/OpenParticleSystem` for the data model.
3. **There is no GI tier below ray query.** ReSTIR GI is excellent and requires
   hardware ray query. There is nothing for hardware that lacks it and nothing
   baked for static scenes. References:
   `FlaxEngine-master/Source/Engine/Renderer/GI/DynamicDiffuseGlobalIllumination.cpp`
   and `GlobalSurfaceAtlasPass.cpp` — the two files are a complete, readable
   probe-GI-plus-surface-cache pair, and Flax is the only engine in the tree
   that ships both beside each other.
4. **Transparency is unordered and AA has one spatial mode.** No OIT; FXAA is
   the only non-temporal AA. `FlaxEngine-master/Source/Engine/Renderer/AntiAliasing/SMAA.cpp`
   sits next to `FXAA.cpp` and `TAA.cpp` in the same directory — the smallest
   possible diff to study.
5. **Textures are resident or absent.** No virtual texturing, so the 32-layer
   terrain material's memory is a hard ceiling rather than a budget.

### 5.3 The editor bar, after CONTROL

CONTROL closes the property, asset, drag, settings, input-modifier and command
seams, and adds curve and gradient editing, time of day, clouds and weather.
What it does **not** close, and MORROWIND does:

- **Panels are fixed.** One layout, one viewport, no floating windows, no
  user-arranged workspace. Reference:
  `FlaxEngine-master/Source/Editor/GUI/Docking/` — six files
  (`MasterDockPanel.cs`, `DockPanel.cs`, `DockPanelProxy.cs`, `DockWindow.cs`,
  `FloatWindowDockPanel.cs`, `WindowDragHelper.cs`), a from-scratch docking
  implementation in a retained UI, not an ImGui wrapper. And
  `fyrox/Fyrox-master/fyrox-ui/src/dock/` — **the same feature, in the exact
  widget architecture Somnium forked.** See §6.1, which is the most important
  finding in this document.
- **There is no graph editor**, so materials, animation, VFX and AI have no
  authoring surface.
- **There is no timeline**, so nothing keyframed can be sequenced.
- **Lists are not virtualised**, so the content drawer and the outliner have a
  size ceiling nobody has measured.
- **There is no play-in-editor**, so the editor and the game are the same
  process in the same state with no separation to revert to.

### 5.4 On render graphs, decided once

Several references in §6 have one (Flax; Stride's `RenderFeature`/`RenderStage`
model; Godot's rendering-device graph), and two go further: Ogre-Next's
compositor is a **script-authored** frame graph
(`ogre-next-master/ogre-next-master/OgreMain/src/Compositor/` —
`OgreCompositorNodeDef.cpp`, `OgreCompositorWorkspaceDef.cpp`,
`OgreTextureDefinition.cpp`, and shadow nodes as a first-class kind), and
Babylon ships a graphical editor for one
(`Babylon.js-master/packages/tools/nodeRenderGraphEditor`). The recurring
temptation is to build one.

**Decision: no.** A render graph buys automatic barrier insertion, resource
aliasing and pass reordering. wgpu already does barrier tracking; Somnium's pass
list is explicit, ordered, and small enough to read; and DOOM proved the frame
is shading-bound rather than bandwidth-bound on both shipped maps. The measured
cost of a graph refactor is a rewrite of `renderer.rs` (4,383 lines) against a
benefit the profiler cannot currently see.

**What people actually want from a render graph, Somnium gets from MORROWIND-C
instead**: named resources, declared dependencies, and the ability to add a pass
without editing a 4,383-line function. Seam 3 states that as an explicit design
constraint, so the door stays open without the rewrite.

---

## 6. Repository and literature audit

Every path below was verified by listing or reading it on **2026-08-23**.
Where a claim is inference rather than reading, it says so.

### 6.1 The single most efficient finding: Fyrox already built half of Track 1 and Track 2

`somnium_ui` is a fork of Fyrox's widget architecture — the generational pool,
the message bus and the widget/draw split are cited in `ATTRIBUTION.md` §13.13
through §13.18. Somnium took roughly twenty widgets.

`fyrox/Fyrox-master/fyrox-ui/src/` contains **sixty-odd modules**, and the ones
Somnium did *not* take map almost exactly onto this phase's editor and UI
tracks:

| Fyrox module | What it is | Somnium track |
|---|---|---|
| `dock/` | Docking: tiles, splitters, floating windows | **MORROWIND-J** |
| `absm/` | An **animation state machine editor** | **MORROWIND-K/V** |
| `curve/` | Curve editor | CONTROL-K (already planned there) |
| `bbcode.rs` + `formatted_text/` | **Rich text with markup tags** | **MORROWIND-G** |
| `vector_image.rs` | **Vector path rendering in the widget tree** | **MORROWIND-D** |
| `navigation.rs` | **Focus navigation** between widgets | **MORROWIND-F** |
| `nine_patch.rs` | Nine-slice widget (Somnium has the draw call, no widget) | **MORROWIND-D** |
| `window.rs`, `screen.rs` | Floating windows; screen-space root | **MORROWIND-J/E** |
| `list_view.rs`, `tree.rs` | List and tree | **MORROWIND-M** |
| `inspector/` | Reflection-driven inspector with per-type property editors | CONTROL-B |
| `animation.rs`, `key.rs` | UI animation and hotkeys | **MORROWIND-H** |
| `file_browser/`, `path.rs`, `messagebox.rs`, `expander.rs`, `dropdown_list.rs`, `progress_bar.rs`, `range.rs`, `selector.rs`, `toggle.rs`, `thumb.rs`, `matrix.rs` | Widget library depth | Track 1/2 as they come up |
| `style/` | A style/theme resource system | Zeta owns this; do not fork |

And `fyrox/Fyrox-master/editor/src/` contains `absm/`, `animation/`, `asset/`,
`audio/`, `command/`, `export/`, `interaction/`, `plugins/`, `scene_viewer/`,
`settings/`, `ui_scene/`, `world/` — including a **`ui_scene`**, i.e. Fyrox uses
its own UI framework to author *game* UI in the editor, which is precisely the
Track 1 + Track 2 combination this phase proposes.

**This is the highest-leverage reference in the tree, by a wide margin**, because
it is the only one where the surrounding architecture is not merely similar to
Somnium's but is Somnium's actual ancestor, in Rust, under a permissive license.
MORROWIND-A's first deliverable is a systematic diff of `fyrox-ui/src/` against
`somnium_ui/src/`, module by module, with a keep/adapt/refuse verdict on each.

**Caveat, stated so it is not discovered later:** Somnium's fork has diverged
substantially — Phase 27 replaced the paint layer entirely, Phase 26-Zeta
replaced the style layer, and Fyrox has moved on since the fork. These are
*patterns to read*, not patches to apply. §6.6's copy rule applies to Fyrox
exactly as it applies to everything else.

### 6.2 The enabling-primitive finding: one surface, one timeline

Two directory listings make the same argument.

**`FlaxEngine-master/Source/Editor/Surface/`** contains `VisjectSurface.cs`
(1,121 lines) plus `VisjectSurface.Input.cs` (1,229), `.Draw.cs`,
`.DragDrop.cs`, `.CopyPaste.cs`, `.Formatting.cs`, `.Parameters.cs` (99),
`.Serialization.cs` (87), `VisjectSurfaceContext.cs` (464),
`VisjectSurfaceContext.Serialization.cs` (780) and `VisjectSurfaceWindow.cs`
(1,355) — **7,842 lines across the files measured** — and then, in the same
directory, these thin specialisations:

```
AnimGraphSurface.cs
BehaviorTreeSurface.cs
MaterialSurface.cs
MaterialFunctionSurface.cs
ParticleEmitterSurface.cs
ParticleEmitterFunctionSurface.cs
AnimationGraphFunctionSurface.cs
VisualScriptSurface.cs
```

**One graph implementation; eight authoring tools.** The generic parts are
`NodeArchetype.cs`, `NodeElementArchetype.cs`, `GroupArchetype.cs` and
`NodeFactory.cs` — nodes are *data*, and each tool contributes a node catalogue
rather than a widget.

**`FlaxEngine-master/Source/Editor/GUI/Timeline/`** repeats the shape:
`Timeline.cs` + `Timeline.Data.cs` + `Timeline.UI.cs` + `Track.cs` +
`TrackArchetype.cs` + `Media.cs` + `Undo/`, specialised by
`AnimationTimeline.cs`, `ParticleSystemTimeline.cs` and
`SceneAnimationTimeline.cs`.

**This is the organising principle of Track 2.** MORROWIND-K builds one surface;
MORROWIND-L builds one timeline; between them they unlock the material graph,
the animation graph, the behaviour tree, the VFX graph, the sequencer, the
animation editor and the audio track view. Building any one of those bespoke
would cost most of what the shared primitive costs and would unlock exactly one.

### 6.3 Animation: Esoterica is the reference, and its node list is the spec

`Esoterica-main/Code/Engine/Animation/` contains, at the top level:
`AnimationBlender`, `AnimationBoneMask`, `AnimationClip`, `AnimationEvent`,
`AnimationFloatChannels`, `AnimationFrameTime`, `AnimationPose`,
`AnimationRootMotion`, `AnimationSkeleton`, **`AnimationSyncTrack`**,
`AnimationTarget`, plus `Components/`, `Debug/`, `Events/`, `Graph/`, `IK/`,
`ResourceLoaders/`, `Systems/` and **`TaskSystem/`**.

Two of those deserve naming because most hobby animation systems lack them and
then discover why they were needed:

- **`AnimationSyncTrack`** — synchronised blending. Blending a walk into a run
  without it produces foot-sliding, because the two clips' phases are unrelated.
  Sync tracks are the difference between a blend tree that demos well and one
  that ships.
- **`TaskSystem/`** — pose evaluation is scheduled as a task graph rather than
  a recursive tree walk, which is what makes multi-character evaluation
  parallelisable. This is a direct customer of MORROWIND-B.

`Esoterica-main/Code/Engine/Animation/Graph/Nodes/` enumerates the runtime node
types, and the list is effectively Track 5's feature spec: `AnimationClip`,
`Blend1D`, `Blend2D`, `BoneMasks`, `Bools`, `CachedValues`, `ConstValues`,
`Events`, `ExternalPose`, `Floats`, **`FootIK`**, `IDs`, `Layers`,
**`OrientationWarp`**, `Parameters`, and more beyond the thirty files sampled.
MORROWIND-V and MORROWIND-W take that list as their scope and cut from it,
rather than inventing a smaller one and discovering the gaps.

Second and third opinions where they differ usefully:

- `o3de-development/Gems/EMotionFX` for production breadth, and
  `Gems/MotionMatching/Code/Source/BlendTreeMotionMatchNode.cpp` for the
  motion-matching node — deferred (MORROWIND-W2), but the sibling files
  (`EventData.cpp`, `CsvSerializers.cpp`) name the data a future phase would
  have to cook.
- `FlaxEngine-master/Source/Engine/Animations` for the AnimGraph runtime that
  pairs with the `AnimGraphSurface` in §6.2 — the clearest available example of
  the runtime and the authoring tool being designed together.
- `bevy/bevy-main/crates/bevy_animation/` for the Rust-idiomatic tier.

### 6.4 Streaming, prefabs and scattering: O3DE and Unreal

- **Prefabs.** `o3de-development/Code/Framework/AzToolsFramework/AzToolsFramework/Prefab/`
  contains `Instance/`, `Link/`, `Overrides/`, `PrefabDomTypes.h`,
  `PrefabDomUtils.cpp`, `PrefabFocusHandler.cpp` and
  `DocumentPropertyEditor/`. The model is template + instance + **JSON patch**,
  with links carrying the patch and a focus handler deciding which prefab an
  edit lands in. It is the most rigorous prefab implementation available in
  open source and its patch-based override model is the one that survives
  nesting. Second opinion: `FlaxEngine-master/Source/Engine/Level/Prefabs/`
  for a simpler tier closer to Somnium's scale. Both are read by MORROWIND-O
  before it picks.
- **Streaming.** `o3de-development/Code/Framework/AzCore/AzCore/IO/Streamer/`
  contains `BlockCache`, `DedicatedCache`, `FileRange`, `FileRequest` and a
  scheduler — a prioritised async I/O stack with deadlines, which is exactly the
  contract MORROWIND-B needs to expose and MORROWIND-R needs to consume.
- **World partition.** `UnrealEngine-release/Engine/Source/Runtime/Engine/Private/WorldPartition/`
  contains `RuntimeSpatialHash/`, `RuntimeHashSet/`, `HLOD/`, `DataLayer/`,
  `LoaderAdapter/`, `ContentBundle/`, `Cook/`, `LevelInstance/`,
  `PackedLevelActor/`, `NavigationData/` and `ActorDescContainer*`. Two things
  to take and one to refuse are in MORROWIND-S. **Proprietary — pattern only,
  §6.6.**
- **Scattering.** `o3de-development/Gems/Vegetation/Code/Source/` with
  `AreaSystemComponent`, `Components/`, `Debugger/`, alongside
  `Gems/GradientSignal/Code/Source/` (`GradientSampler.cpp`, `Components/`) and
  `Gems/SurfaceData`, authored through `Gems/LandscapeCanvas`. Gradients produce
  values, surface tags classify ground, filters cut, and a node graph composes
  them — and the node graph is `Gems/GraphCanvas` + `Gems/GraphModel`, i.e.
  **the same reusable graph framework**, which is the §6.2 argument arriving
  from a second direction.
- **Rust prior art.** `terra-main/src/` — `cache/`, `stream.rs`, `mapfile.rs`,
  `gpu_state.rs`, `compute_shader.rs`, **`billboards.rs`** (impostors) and a
  sibling `rshader/` crate (shader preprocessing and hot reload). A
  planet-scale streaming terrain **in Rust**. It is the closest thing in the
  tree to a working model of MORROWIND-Q/R/S/T in Somnium's own language.

### 6.5 Runtime UI, and the four questions Track 1 must answer

| Question | Reference to read | Note |
|---|---|---|
| Anchors and layout | `Unity3D/uGUI-main` (RectTransform anchors); Godot's `Control` anchors and size flags; `stride-master/sources/engine/Stride.UI` (WPF-style measure/arrange) | Somnium already has a measure/arrange core from Fyrox — Seam 4 recommends **anchors layered on top of it**, not a replacement. |
| World-space canvas | `FlaxEngine-master/Source/Engine/UI/UICanvas.cpp` — screen and world space in one class; `Babylon.js-master/packages/dev/gui` for the 3D GUI variant | The question is render-target-then-quad versus direct 3D submission. MORROWIND-E picks and records why. |
| Focus and gamepad navigation | `fyrox-ui/src/navigation.rs`; Godot's explicit focus-neighbour links; Unity's geometric search | MORROWIND-F takes **both**: explicit links with geometric fallback. |
| Rich text and sprites | `fyrox-ui/src/bbcode.rs` and `formatted_text/`; `WickedEngine-master/WickedEngine/wiSpriteFont.cpp`; `o3de-development/Gems/LyShine/Code/Source/` with its `Animation/` subdirectory (`AnimNode.cpp`, `AnimSequence.cpp`, `AnimSplineTrack.h`, `2DSpline.h`) and `Gems/TextureAtlas` | LyShine is the only reference here that ships **UI animation as tracks on a timeline** — which is MORROWIND-L's fifth consumer. |

### 6.6 Provenance rules, and the three references that are not permissive

The standing rule — patterns only, never source, cited in `ATTRIBUTION.md` —
applies to everything. Three references need a **stricter** rule and it is
stated here once so no sub-phase has to re-derive it:

| Reference | License | Rule |
|---|---|---|
| `UnrealEngine-release` | Epic EULA, **proprietary** | Read for architecture only. Do not reproduce identifiers, file structure, comments, constants or shader code. Describe the *technique* in the plan and implement from the public literature (papers, GDC talks) wherever one exists. Cite the technique, not the file, in shipped code comments. |
| `Daemon-master` | **GPL** | Same. Its `src/engine/renderer/gl_shader.cpp` is a mature shader-permutation manager and is worth *reading* for MORROWIND-C; nothing from it may be transcribed. |
| `luanti-master` | **LGPL** | Same. Relevant to Track 4 for its block emerge and streaming pipeline. |

Everything else named in this document is MIT, Apache-2.0, BSD or comparable
(Fyrox, Bevy, Godot, Wicked, Stride, O3DE, terra). **Flax's license is
source-available and must be checked by MORROWIND-A before §6.2's surface work
begins**, since this plan leans on Flax heavily and "widely read" is not the
same as "permissive". The rule for the permissive set is unchanged: patterns,
cited, never source.

### 6.7 Non-overlap with Phase CONTROL

Stated as a table so a later session cannot drift into duplicating it:

| Capability | Owner | MORROWIND's relationship |
|---|---|---|
| Reflection-driven Details / property seam | **CONTROL-B** | Consumes it. Every new component ships a schema. |
| Asset database, thumbnails, preview jobs | **CONTROL-C** | Consumes it; MORROWIND-Q replaces the *source* files it indexes with cooked ones and must not break its `AssetId`. |
| Material authoring (property-based, `.sommat`) | **CONTROL-D** | MORROWIND-K adds a *graph* authoring mode **on top of** `.sommat`, and only after CONTROL-D ships. It does not replace it. |
| Drag and drop | **CONTROL-E** | Consumes it; adds payload kinds. |
| Outliner, selection, clipboard | **CONTROL-F** | MORROWIND-O extends it with prefab-instance display rules. |
| Viewport control (camera, gizmos, overlays) | **CONTROL-G** | MORROWIND-J adds *multiple* viewports; it does not re-implement one. |
| Preferences, keybindings, project settings | **CONTROL-H** | Consumes it; MORROWIND-AE's action maps are stored through it. |
| Scene lifecycle, save and load | **CONTROL-J** | MORROWIND-O extends the format with prefab links; MORROWIND-AF adds *save games*, which are a different thing. |
| **Curve and gradient editing** | **CONTROL-K** | **MORROWIND does not build a curve editor.** MORROWIND-L's timeline *embeds* CONTROL-K's. |
| **Time of day, clouds, weather** | **CONTROL-L/M/N** | **MORROWIND does not build any of these.** |
| Deferred decals | CONTROL-O (**stretch, optional**) | If CONTROL drops it, MORROWIND-AC picks it up; if CONTROL ships it, MORROWIND-AC drops it. Whoever starts first announces it. |

---

### 6.8 The second reconnaissance pass — the engines nobody had read

§6.1–6.5 lean on the eight engines this project has mined before. `example_repo`
holds roughly forty repositories and most had never been opened for any purpose.
They were surveyed on **2026-08-23**, specifically for things the mainstream
references do *not* have. **Ten of them changed a decision in §8 and the change
is noted inline there**; the rest are recorded so nobody re-checks.

#### 6.8.1 The seven that changed this plan

**(a) Stride ships golden-image regression testing for a renderer.**
`stride-master/sources/engine/Stride.Graphics.Regression/` contains
`ImageTester.cs`, `ImageThreshold.cs`, `TestResultImage.cs`, `GameTestBase.cs`,
`FrameGameSystem.cs`, `RegressionTestAttribute.cs` and `FpsTestCamera.cs`,
beside a sibling `Stride.Games.AutoTesting` project.

Somnium's entire visual-evidence process is a human taking a screenshot and
looking at it. Every phase record in `dev records/` says "capture after
tonemapping" and none of it is checked by anything. A renderer with 328 tests
and **zero image assertions** cannot detect that a shader edit shifted the
terrain a shade greener.

**Adopted into GHOSTFENCE (§10)** as a golden-image row: a fixed camera, a fixed
frame, a perceptual threshold, and a stored reference PNG per scene. This turns
the phase's most-repeated instruction — "prove the paint contract did not move"
— from a promise into a test. It is also the single cheapest quality mechanism
found in this entire survey.

**(b) Panda3D solved multi-threaded rendering with pipeline cycling, and it is
the reference for the thing MORROWIND-B deliberately does not do.**
`panda3d-master/panda/src/pipeline/` contains `pipelineCycler.h`,
`pipelineCyclerTrueImpl.cxx`, `cycleData.cxx` and `cyclerHolder.cxx`. Every
piece of scene state is stored in a cycler with one copy per pipeline stage, so
the App, Cull and Draw threads each read a *different* consistent snapshot of
the same graph without locks and without copying the scene.

Somnium's `jobs.rs:3` states its own limit plainly: *"Record still happens on
the render thread."* Pipeline cycling is the principled fix, and it is also a
change to every piece of scene state in the engine.

**Recorded, not adopted.** MORROWIND-B builds a job system for *background*
work; multi-threaded recording is out of this phase (§14.8). The value of the
reference is that it names the design Somnium would have to adopt if recording
ever becomes the bottleneck, so a later phase does not invent a worse one.

**(c) Babylon.js runs six node-graph editors on one substrate — a stronger
version of §6.2's finding, with two graph kinds this plan had not listed.**
`Babylon.js-master/packages/tools/` contains `nodeEditor` (shaders),
`nodeGeometryEditor` (**procedural geometry**), `nodeParticleEditor`
(**particles**), `nodeRenderGraphEditor` (**the frame graph itself**),
`flowGraphEditor` (logic) and `smartFiltersEditor`, plus
`packages/dev/sharedUiComponents/`.

Flax proved one surface serves eight tools. Babylon proves the same and adds
*node geometry* — a Blender-geometry-nodes-style procedural mesh graph — which is
a plausible seventh catalogue for MORROWIND-K and is named in §14.2 rather than
planned.

**(d) Babylon and Fyrox both ship a GUI *layout editor*, which is the missing
half of Track 1.** `Babylon.js-master/packages/tools/guiEditor/` and
`fyrox/Fyrox-master/editor/src/ui_scene/`. Track 1 as originally written gives a
game a UI framework and leaves authoring to code.

**Adopted as MORROWIND-M2** (§8, Track 2): the editor authors `.somui` documents
in the same widget tree the editor itself is built from. Two independent engines
concluded that a UI framework without a layout editor is half-shipped, and
Somnium is uniquely well placed to do it because its editor *is* the framework.

**(e) `terra` and `bevy_terrain` both solve large-world precision, in Rust, and
they disagree — which is exactly what MORROWIND-T needed.**
`terra-main/src/shaders/softdouble.glsl` emulates double precision **inside the
shader**; `bevy-plugins/bevy_terrain-main/src/big_space.rs` uses a hierarchical
integer-grid-plus-float-offset space on the CPU.

**Adopted into MORROWIND-T** as the two references its decision is made against,
replacing a paragraph that previously reasoned from first principles. `big_space`
is the closer fit for an engine whose renderer is already camera-relative-capable;
`softdouble` is what a planet-scale variant would need later.

**(f) `terra/rshader` already splits shaders the way MORROWIND-C proposes.**
`terra-main/rshader/src/` is three files: `lib.rs`, **`dynamic_shaders.rs`** and
**`static_shaders.rs`** — hot-reloading sources in development, baked variants in
release, behind one interface. That is MORROWIND-C items 3 and 5, in Rust,
proven small. `bevy-plugins/bevy_mod_outline-master/src/pipeline_key.rs` supplies
the other half — a permutation key as an idiomatic Rust type.

**Adopted into MORROWIND-C** as its two Rust precedents, displacing Daemon's
`gl_shader.cpp` from primary to secondary (Daemon is GPL and read-only anyway,
§6.6).

**(g) Luanti answers the streaming question this plan had left implicit: what
happens to living entities when their cell unloads?**
`luanti-master/src/staticobject.cpp` / `.h`, beside `activeobjectmgr.h` and
`serverenvironment.cpp`. Luanti's answer is that an active object is *serialised
into the block* when the block unloads and re-instantiated when it loads — so an
entity's identity is owned by the cell, not by a global list.

**Adopted into MORROWIND-S.** Without a rule like this, streaming either leaks
entities or destroys them. Luanti is **LGPL — pattern only, §6.6.**

#### 6.8.2 The rest of the survey, and what each is good for

**Ren'Py** (`renpy-master/renpy/`) is the most unusual engine in the tree and
the one whose ideas travel furthest from their origin:

- **`rollback.py` (1,217 lines) with `revertable.py`** — deterministic rewind of
  the entire game state. Every mutable container is a revertable subclass that
  logs its mutations; the engine can step backwards arbitrarily. This is
  relevant twice over: **MORROWIND-N** needs exactly "snapshot world state on
  entering play mode, restore on exit", and **MORROWIND-AF** needs a save model
  that is not a scene serialiser. Noted in both.
- **`atl.py` (2,368 lines)** — a declarative animation and transformation
  language, the most complete precedent for MORROWIND-H's tween spec.
- **`sl2/` and `display/screen.py`** — a declarative UI re-evaluated each frame
  and diffed against the retained tree. The closest thing in the tree to a
  modern reactive UI. **Refused for Somnium** (§3.8 forbids a second UI model),
  recorded because it is the strongest argument against the refusal.
- **`translation/`** — string extraction with round-trip, already cited by
  MORROWIND-AH. **`lint.py`** — a *content* linter, which nothing else here has.

**Defold** (`defold-dev/defold-dev/engine/`) is a lesson in cookers:
`atlasc`, `modelc`, `shaderc`, `texc` are four standalone compilers, one per
asset kind, exactly the shape MORROWIND-Q proposes. It also has **`liveupdate`**
— post-ship patchable content — which nothing else in the tree ships and which
MORROWIND-Q's content-hash design should not preclude. `rig` is its
skinning/animation runtime and `ddf` its schema-driven data format.

**Daemon** (**GPL — pattern only**) carries one genuinely novel architecture:
`src/engine/framework/VirtualMachine.cpp` with `src/common/IPC/`
(`Channel.h`, `CommandBuffer.cpp`, `Primitives.h`) runs **game logic in a
sandboxed out-of-process VM** talking to the engine over a command buffer, with
`CommonVMServices.cpp` brokering syscalls. Crash isolation, hot reload and mod
safety fall out of it. Somnium gets a weaker version of the same benefit from
Luau. **Out of scope, recorded in §14.9** because it is the right answer if
native gameplay plugins are ever wanted.

**Panda3D**, beyond §6.8.1(b): `panda/src/pgraph/` has `renderState.cxx`,
`renderAttrib.cxx`, `renderAttribRegistry.cxx`, `stateMunger.cxx` and
`cullBinManager.cxx` — render state as **interned, composed, cached** objects
with sortable bins, which is a materially different answer to material state
than a permutation key and worth reading beside MORROWIND-C. And
`panda/src/pstatclient/` is a **networked** profiler: the engine is a thin client
streaming to a separate GUI application, which is how you profile a build you
cannot attach a debugger to.

**Ogre-Next** — the citation deleted from the first draft is restored, the path
was nested twice: `ogre-next-master/ogre-next-master/OgreMain/src/Compositor/`
contains `OgreCompositorManager2.cpp`, `OgreCompositorNode.cpp`,
`OgreCompositorNodeDef.cpp`, `OgreCompositorWorkspace.cpp`,
`OgreCompositorWorkspaceDef.cpp`, `OgreCompositorShadowNodeDef.cpp` and
`OgreTextureDefinition.cpp`. A **script-authored** frame graph — nodes,
workspaces, texture definitions, and shadow nodes as a first-class kind. It is
the most refined data-driven frame graph available and **§5.4 still declines to
build one**; the reference makes the refusal informed rather than uninformed,
and Babylon's `nodeRenderGraphEditor` shows where that road ends.

**jMonkeyEngine** (`jme3-core/src/main/java/com/jme3/`) has `anim/` and
`animation/` side by side — the modern armature rewrite next to the legacy
system, which is a rare chance to read a migration rather than a design. Its
`material/` `.j3md` definition-plus-technique-plus-define model remains a clean
small-scale answer to MORROWIND-C, and `export/` (its `Savable` binary format)
and `cinematic/` are worth a look for MORROWIND-Q and MORROWIND-L respectively.

**The bevy plugin set** — sixteen Rust/wgpu implementations, surveyed for what
each proves is possible in Somnium's own stack:

| Plugin | The transferable part | Track |
|---|---|---|
| `bevy_terrain-main` | `big_space.rs` (§6.8.1e), `preprocess/`, `terrain_data/`, `terrain_view.rs` — tile streaming and virtual texturing | 4 |
| `bevy_mod_outline-master` | `pipeline_key.rs` (permutation key), `flood/` (jump-flood outlines) | 0, 7 |
| `bevy_aabb_instancing-main` | `vertex_pulling/` — geometry pulled from a storage buffer rather than a vertex buffer, which is the shape MORROWIND-U's skin-to-buffer design wants | 5 |
| `bevy_trenchbroom-main` | `brush.rs`, `qmap/`, `bsp/`, `fgd.rs`, `class/` — Quake-format brush geometry **and an entity-definition schema**. A complete blockout format with typed entities, in Rust | 3 |
| `bevy-hikari-main` | `mesh_material/`, `prepass.rs` — a Rust/wgpu RT-GI implementation; Somnium's ReSTIR is ahead, so this is a second opinion, not a source | 7 |
| `bevy_voxel_world-main` | `chunk_map.rs`, `mesh_cache.rs`, `voxel_traversal.rs` — chunk residency and meshing cache | 4 |
| `bevy_vello-main` | `render/`, `integrations/`, `picking.rs` — compute vector graphics in a wgpu render graph, and **hit-testing against vector shapes**, which MORROWIND-F needs | 1 |
| `bevy_enoki-master`, `bevy-vfx-bag-main` | Particle material model; a post-FX collection | 7 |
| `bevy_water-main`, `bevy_triplanar_splatting-main`, `bevy_wind_waker_shader-main` | Somnium is ahead on all three | — |
| `bevy_light_2d-main`, `bevy_ecs_tiled-main`, `bevy_aseprite_ultra-master`, `bevy_vox-master` | 2D and format-specific; not applicable | — |

**Recorded as checked and not useful, so nobody repeats the work:**
`swiftshader-master` (a software Vulkan implementation; already cited in
`ATTRIBUTION.md` §7 for semantics and has nothing further),
`DirectXShaderCompiler-main` (already §6), `CDLOD-master` and
`GodotOceanWaves-main` (both fully mined by Phases XV and IV),
`haxe-development` (a language compiler; its macro system is not a model for a
Rust `macro_rules!` schema), `Crafty-develop` (a 2D JS framework with nothing
Somnium lacks), `ebitenui-master` (a small Go retained UI — a useful widget
taxonomy for Track 1 and nothing structural), `SolersEngine-main` (confirmed a
Godot 4.7.1 fork; read Godot instead), and `Unity3D/PostProcessing-2` (a
complete effect list — AO, auto-exposure, bloom, chromatic aberration, colour
grading, DOF, dithering, FXAA, fog, grain, lens distortion, motion blur,
multi-scale VO, SSR, **SMAA**, TAA, vignette — of which Somnium already has
every entry except SMAA and dithering, which is a useful confirmation that
Track 7's list is the right list).

**Not opened, and honest about it:** `sbox-public-master`, `LuminaEngine-main`,
`VoxelHex-main` beyond its module list (`boxtree/`, `raytracing/`, `spatial/`,
`object_pool.rs` — a Rust sparse-voxel structure with GPU traversal, already
cited in `ATTRIBUTION.md` §8), `korge-main` beyond confirming that
`korge-reload-agent/` and `korge-ipc/` exist (hot-reloading a *running* game, and
out-of-process editor/game IPC — both interesting for MORROWIND-N and unread),
`raylib-master`, `Unity3D/UnityCsReference-master`, `Unity3D/ml-agents-develop`,
`FalcoEngine`, `Overload-main`, `NeoAxisEngine-master` and `rbfx-master` beyond
what Phase CONTROL already recorded. A later pass should take `korge-ipc` and
`sbox` first; they are the two most likely to hold something.

### 6.9 The third pass — Korge, s&box, and the web research that had failed

§6.8 closed with two repositories named as unread and a web pass recorded as
never having run. Both were done on **2026-08-23**. **The web half changed a
frozen constraint**, which is the most consequential single finding in this
document.

#### 6.9.1 wgpu 30 shipped on 2026-07-01, and Somnium is on 29

Per the wgpu changelog (`raw.githubusercontent.com/gfx-rs/wgpu/trunk/CHANGELOG.md`,
fetched 2026-08-23), **v30.0.0 released 2026-07-01**. What it adds is not
incidental to this plan:

| v30 addition | Why it matters here |
|---|---|
| **`EXPERIMENTAL_MESH_SHADER`** — fully supported on Vulkan; Metal and DX12 via passthrough | Somnium's visibility buffer is **already meshlet-based**. Mesh shaders are the native expression of what `meshlet.rs` and `cull.wgsl` currently emulate with compute plus indirect draws. |
| **`MULTI_DRAW_INDIRECT_COUNT`** | GPU-driven submission without a CPU round-trip for the draw count. |
| **`ACCELERATION_STRUCTURE_BINDING_ARRAY`** | Bindless acceleration structures — relevant to ReSTIR and RT water once geometry becomes dynamic (Track 5). |
| `enable wgpu_binding_array;`, `enable primitive_index;`, `enable wgpu_int16;` (`SHADER_I16`) | WGSL `enable` extensions — MORROWIND-C's module composition must model them, because an `enable` is file-scoped and composing two modules that disagree is a compile error. |
| **`immediate` address space replaces `push_constant`** | A **breaking WGSL change** across 48 shader files. |
| `subgroup_min_size` / `subgroup_max_size` moved from `Limits` to `AdapterInfo` | A breaking Rust-side change. |
| Storage-texture binding arrays on Metal | Portability of the bindless path. |
| `EXPERIMENTAL_RAY_QUERY` absorbs the former separate acceleration-structure feature | A flag rename Somnium's ray-query paths must follow. |

`EXPERIMENTAL_RAY_QUERY` remains the only ray-tracing surface — **naga supports
ray *queries*, not ray-tracing pipelines** — which confirms Somnium's existing
choice was and remains correct, and closes that question.

**Adopted as MORROWIND-A2** (§8, Track 0), a sub-phase that does nothing but the
bump, exactly as §12.4 requires. It is not optional: three Track 7 sub-phases and
Track 5's skinning integration are written against capabilities that exist in 30
and not in 29, and doing the bump *inside* a feature sub-phase is how a
two-week debugging session starts.

**Caveat this document must not overstate:** the changelog was read; the
features were not exercised. `EXPERIMENTAL_` is in two of those names for a
reason, and "fully supported on Vulkan" is a changelog's claim, not a
measurement. MORROWIND-A2 probes each flag on the actual target hardware and
records what it finds. §12.4's rule stands unchanged.

#### 6.9.2 The Rust ecosystem answers four of this plan's open questions

Each of these replaces a paragraph that previously said "the sub-phase decides
against Rust crate reality" with an actual name. **None was verified by reading
the crate**; they are leads with a URL, and each owning sub-phase confirms.

- **Navmesh (MORROWIND-X).** `oxidized_navigation` is a *pure-Rust* runtime
  tiled Recast port; `polyanya` implements any-angle path planning with
  overlapping navmeshes, one-way layers and per-layer traversal costs;
  `landmass` / `bevy_landmass` supplies agents and avoidance; `recast_navigation`
  is an FFI wrapper for the C++ original. **The important caveat, and it is
  Somnium-specific:** `oxidized_navigation` consumes **parry3d** colliders
  through an `OxidizedCollider` trait, with integrations for Rapier and Avian.
  **Somnium uses Jolt.** So the pure-Rust path costs a collider-extraction
  adapter that none of the published integrations provide, and MORROWIND-X's
  FFI-versus-pure-Rust decision is really "write an adapter" versus "bind the
  C++", with the in-tree Jolt FFI precedent (`somnium_physics_sys`) favouring
  the latter. That is a materially better-framed decision than the plan had.
- **Text shaping (MORROWIND-G).** `cosmic-text` now shapes via **`harfrust`** (a
  Rust HarfBuzz) and rasterises via `swash`, which has absorbed parts of Google's
  font stack (`skrifa`); `parley` is the Linebender alternative and requires
  Rust 1.85+, which Somnium's 1.88 satisfies. Phase 27 deferred `cosmic-text`
  once; the ecosystem has moved since, and MORROWIND-G now has two credible
  candidates rather than one deferred one.
- **Video (MORROWIND-AH).** `vk-video` decodes through **Vulkan Video directly
  into a `wgpu::Texture`**, so frames never leave GPU memory. That is a
  qualitatively better answer than the FFmpeg-decode-then-upload path the plan
  assumed, and it removes video's main objection. Fallback remains
  `ffmpeg-next`; `dav1d` covers AV1.
- **Accessibility (MORROWIND-I).** **Godot 4.5 shipped AccessKit screen-reader
  support.** A mainstream engine has done the exact thing MORROWIND-I proposes,
  in a self-rendered UI, which converts that sub-phase from speculative to
  precedented. `bevy_a11y` remains the Rust integration shape (§8, Track 1).

#### 6.9.3 Korge: out-of-process play-in-editor, in six files

`korge-main/korge-ipc/src/main/kotlin/korlibs/korge/ipc/` is the smallest
complete answer to MORROWIND-N found anywhere:

- `KorgeFrameBuffer.kt` — a **memory-mapped file** as a shared framebuffer.
  `FileChannel.open(path, READ, WRITE, CREATE)` then
  `channel.map(READ_WRITE, 0, 32 + width * height * 4)`: a 32-byte header
  followed by pixels, remapped on resize (`ensureSize`).
- `KorgeIPCSocket.kt` / `KorgeUnixSocket.kt` / `KorgeIPCQueue.kt` — events back
  the other way over a socket with a packet ring buffer (there is a
  `KorgePacketRingBufferTest`).
- `KorgeIPC.kt` — the pair is discovered from one path: `"$path.frame"` and
  `"$path.socket"`, with `path` taken from the `korge.ipc` system property or
  the `KORGE_IPC` environment variable, defaulting to a temp path keyed by pid.
- `KorgeIPCJPanel.kt` — the editor side: the game's framebuffer displayed in a
  host UI panel.

The game runs as a **separate process**; the editor sees its pixels through
shared memory and sends it input over a socket. Crash isolation, a real
play/stop boundary, and no state contamination between edit and play all fall
out of process separation rather than out of a snapshot mechanism.

**Recorded as MORROWIND-N's second option**, against Ren'Py-style state
snapshotting (§6.8.2). It is not obviously the right answer for Somnium — a
shared framebuffer costs a full-resolution copy per frame and the editor's
viewport is not a passive image, it is a gizmo surface — but it is a genuinely
different design and MORROWIND-N is now choosing between two rather than
implementing one.

`korge-reload-agent/` is two files (`KorgeReloadAgent.kt`, `CommonWatcher.kt`) —
a JVM agent that watches classes and hot-swaps a running game. The JVM does the
hard part; there is no Rust equivalent to lift, and it is noted only so nobody
goes looking again.

#### 6.9.4 s&box: hot reload is a *state migration* problem, and its UI is Razor

`sbox-public-master/engine/` is thirty-odd C# projects. Two are worth the read:

**`Sandbox.Hotload/`** — `Hotload.cs`, **`InstanceUpgrader.cs`**, `Upgraders/`,
`ILHotload/`, `CecilHelpers.cs`, **`UpdateReferences.cs`**, `ReferenceComparer.cs`,
`StructArrayConverter.cs`, `Watch.cs`. The naming tells the story: swapping the
assembly is the easy part; the work is **walking every live object graph,
upgrading each instance to its new type, and rewriting every reference to it**.
Everyone who attempts hot reload discovers this second; s&box named a whole
project after it.

The lesson transfers to Somnium unchanged even though the language does not:
**MORROWIND-R's hot reload of cooked assets, and MORROWIND-C's hot reload of
shaders, are both easy precisely because their live state is a handle table.**
Anything whose reload requires migrating live instances — a component whose
layout changed, a script whose type changed — is a different and much harder
feature, and §14.11 now says so rather than letting a later session assume "hot
reload works" generalises.

**`Sandbox.Razor/`** with `Microsoft.AspNetCore.Components` — s&box's runtime UI
is **Razor components with CSS-like styling**. Together with Unity's UI Toolkit
(UXML/USS) that is two commercial engines choosing a markup-plus-stylesheet
model for game UI. Somnium's §3.8 refuses a second UI model and this does not
change that; it is recorded beside Ren'Py's screen language in §14.10 as the
second data point that the refusal has a real cost.

Also present and worth knowing: **`Sandbox.CodeUpgrader/`** — automatic
migration of *user* code across engine API changes, which is the thing every
engine wishes it had by version three; `Sandbox.Tools/` with `Animgraph/`,
`ControlWidget/`, `MapEditor/`, `MeshEditor/`, `ModelEditor/`, `CodeEditor/`,
`Inspector/`, `GameData/`, `EditorShortcuts.cs` and an `Mcp/` directory; and
`Sandbox.Mounting/` for mounting other games' content.

#### 6.9.5 Public literature for the one technique that must not be copied

MORROWIND-Z implements virtual shadow maps and its only in-tree reference is
Unreal, which is proprietary and pattern-only (§6.6). §6.6 says to implement
from published literature "wherever one exists" — one does:
**ktstephano.github.io/rendering/stratusgfx/svsm**, a public write-up of sparse
virtual shadow maps covering GPU-side page management with shader atomics and
frame markers, alongside the hardware sparse-allocation support in Vulkan.
**MORROWIND-Z implements from that and from the original papers, and cites the
technique rather than the file.** A targeted search for a Rust/wgpu VSM
implementation found none; if MORROWIND-Z is ever started, search again first.

---

## 7. The eight seams

A seam is a contract other sub-phases are written against. Argue with them in
§7; do not renegotiate them in §8.

### Seam 1 — Background work is a job with a deadline, and the frame has a budget

```rust
pub struct JobDesc { pub priority: Priority, pub deadline: Option<Duration>, pub cancel: CancelToken }
pub fn submit<T: Send + 'static>(desc: JobDesc, f: impl FnOnce(&Ctx) -> T + Send + 'static) -> JobHandle<T>;
pub fn drain_completions(budget: Duration);   // once per frame, on the main thread
```

Three properties, all load-bearing:

- **Priority and deadline are declared, not inferred.** A visible thumbnail
  outranks an off-screen one; a streaming cell the camera is about to enter has
  a deadline measured in frames. O3DE's `Streamer` (§6.4) is the model.
- **Cancellation is first-class.** The camera turns around; the cell is no
  longer wanted; the work stops. Without this, streaming thrash is unbounded.
- **Completion is drained on the main thread inside a time budget.** This is
  what stops CONTROL-C's 232–260 ms decodes from becoming a frame spike even
  when they finish off-thread. `drain_completions` returning early with work
  outstanding is *correct behaviour*, not a bug.

**Everything long-running in this phase goes through this and nothing bypasses
it.** MORROWIND-B builds it; §11 tests it by asserting no sub-phase introduces a
second thread pool.

### Seam 2 — Every asset is an `AssetId` with a residency state

CONTROL-C introduces `AssetId`. MORROWIND extends it with residency:

```rust
enum Residency { Absent, Requested { since: Instant }, Partial { lod: u8 }, Resident, Evicting }
```

Rules: nothing loads synchronously on the main thread; a request returns
immediately with a placeholder; residency is budgeted in bytes with an eviction
policy; and **the same `AssetId` addresses a source file in the editor and a
cooked blob in a build** (MORROWIND-Q). Runtime code never learns which.

### Seam 3 — A shader is a source plus a permutation key

```rust
struct ShaderKey { module: ModuleId, defines: BitSet }          // hashed
fn pipeline(&mut self, key: ShaderKey, layout: &PipelineLayoutRef) -> &wgpu::RenderPipeline;
```

- WGSL modules compose by named include, resolved by the system, not by
  `include_str!` at the call site.
- A variant is requested by key and compiled once, cached by hash — the thing
  `hlms.rs:14` describes and does not do.
- Compilation is a **job** (Seam 1), so a cache miss stalls one draw, not the
  frame.
- In debug builds a file watcher invalidates by module and recompiles dependent
  variants. Hot shader reload is the highest-value developer feature in the
  entire phase per line of code, and it falls out of this seam for free.
- **The pass list stays explicit** (§5.4). This seam is what lets a pass be
  added without editing `renderer.rs`; it is not a render graph and must not
  grow into one.

References: Daemon's `gl_shader.cpp` (**GPL, read-only**), Ogre-Next HLMS
(already in `ATTRIBUTION.md` §5), jMonkeyEngine's material-definition technique
and define model, Bevy's pipeline specialisation, `terra-main/rshader`.

### Seam 4 — A UI tree has a root that declares its space; primitives gain a second stream

Two parts, and the second is the one that must not break Phase 27.

**(a) Canvas roots.** A UI tree's root is a `Canvas` with a mode:
`Screen { scaler }`, `World { transform, size, billboard }`, or
`Overlay { camera }`. Widgets below it are unaware. This is Flax's `UICanvas`
model (§6.5) and it is what makes the editor's own chrome and a game's HUD the
same code path.

**(b) The primitive extension.** The 100-byte `Primitive` (§4.5) is **frozen**.
MORROWIND-D adds a *second* instance stream with its own pipeline, drawn in the
same pass, ordered by the existing `draw_over` rule:

```rust
enum UiInstance {
    Quad(Primitive),          // unchanged, 100 bytes, existing pipeline
    Shaped(ShapedInstance),   // transform + stroke + path + mask + texture slot
}
```

`ShapedInstance` carries a 2x3 affine transform, a stroke description (width,
join, cap, dash), a path or arc reference, a clip-mask slot, and a bindless
texture index. **The existing pipeline is untouched; existing widgets emit
exactly the bytes they emit today; GHOSTFENCE asserts the 646-instance
composition Phase 27 measured is byte-identical after MORROWIND-D lands.**

Textures become a bindless array rather than three fixed bindings
(`pass.rs:226/242/259`), with the font, icon and thumbnail atlases as the first
three entries — so the existing bind group's *semantics* survive the change.

### Seam 5 — Input is a stream of actions

```rust
struct ActionMap { name: String, actions: Vec<Action> }
struct Action  { name: String, kind: ActionKind, bindings: Vec<Binding> }  // Digital | Analog1D | Analog2D
struct Binding { path: ControlPath, processors: Vec<Processor>, interaction: Option<Interaction> }
```

Keycodes appear in exactly one place: the device layer that resolves a
`ControlPath` to a hardware control. Game code, script and UI see actions.
Rebinding is a runtime operation over the same data. Reference:
`Unity3D/InputSystem-develop/Packages/com.unity.inputsystem/InputSystem/Runtime/Actions/`
— `InputAction.cs`, `InputActionMap.cs`, `InputBinding.cs`. This seam **extends
rather than replaces** CONTROL's Seam 5 (modifier state on `WidgetMessage`):
CONTROL's seam is about the editor's widgets, this one is about the game's
verbs, and they meet at the device layer.

### Seam 6 — A scene is prefab instances plus patches

```rust
struct PrefabInstance { template: AssetId, root: StableId, patches: Vec<Patch> }
struct Patch { target: (StableId, FieldId), value: ReflectValue }
```

- A flat scene is the degenerate case: zero instances.
- Patches are expressed in CONTROL's Seam 1 vocabulary — `(StableId, FieldId,
  ReflectValue)` — so **the prefab override system and the inspector are the
  same mechanism**, and a property edit inside an instance is a patch by
  construction rather than by special case.
- Nesting is a template referencing templates; a patch addresses a path through
  the nesting.
- CONTROL's `ChangeScope` (adopted from rbfx's `AttributeScopeHint`) already
  tells `SetFieldCmd` how far a write ripples; **a patch inherits the same
  scope**, which is what stops an override on a rebuilding field from silently
  corrupting an instance.

Reference: O3DE's template, instance and JSON-patch model (§6.4).

### Seam 7 — A pose is data; the renderer does not know what produced it

```rust
struct Pose            { skeleton: SkeletonId, local: Vec<Transform> }   // authored space
struct SkinningPalette { entity: StableId, matrices: GpuRange }          // world/bind space, GPU
```

The animation graph produces a `Pose`. A single system converts poses to
palettes and uploads them. The visibility-buffer path consumes a palette index
and a skinning permutation key (Seam 3). **Nothing in the renderer references
the animation crate**, which keeps the two testable apart, and means ragdoll,
IK-only rigs, procedural animation and network-replicated poses are all the same
customer.

### Seam 8 — One graph surface, one timeline

The editor gets exactly one node-graph implementation and exactly one
track-timeline implementation, both **data-driven by archetypes**:

```rust
struct NodeArchetype  { id, title, category, inputs: Vec<PinSpec>, outputs: Vec<PinSpec>, body: Vec<ElementSpec> }
struct TrackArchetype { id, title, lanes: Vec<LaneSpec>, media: MediaKind }
```

A feature contributes a *catalogue*, never a widget. §6.2 is the evidence that
this scales to eight tools. **A sub-phase that writes a second graph or a second
timeline has failed.**

### 7.9 Crates this phase creates

New crates, so the dependency graph stays legible and `somnium_core` does not
absorb another 20,000 lines:

| Crate | Track | Contents |
|---|---|---|
| `somnium_jobs` | 0 | Seam 1. No dependency on anything else in the workspace. |
| `somnium_shader` | 0 | Seam 3. Depends on wgpu only. |
| `somnium_anim` | 5 | Seam 7's producer side: skeletons, clips, graphs, IK. No renderer dependency. |
| `somnium_nav` | 6 | Navmesh build and query. |
| `somnium_ai` | 6 | Behaviour trees, perception, steering. |
| `somnium_input` | 8 | Seam 5. |
| `somnium_i18n` | 8 | String tables, plural rules, locale switching. |

`somnium_ui` grows a `runtime/` module (Track 1) beside its existing `editor/`;
`somnium_asset` grows the cook and residency (Track 4); `somnium_audio` grows
from 93 lines to a real crate (Track 8); prefabs land in `somnium_core` beside
`scene_schema.rs` because they are a scene concern.

---

## 8. Sub-phases

Eight tracks, thirty-six sub-phases. Every sub-phase closes with **five**
things, and one missing any of them is not finished:

1. **Runtime artefact** — the public API a game uses, per the runtime rule.
2. **Reached** — the editor controls that author it, per CONTROL's reachability
   rule, including its component schema (§4.8).
3. **Slice** — what it adds to `examples/vvardenfell`, per the second-example
   rule.
4. **Evidence** — captures, `.somtime` rows for anything touching the frame, and
   a GHOSTFENCE run (§10).
5. **Attribution** — its `ATTRIBUTION.md` §13H entries.

Sub-phase names are Vvardenfell places, on the Phase 27 precedent of naming
sub-phases after the rivers of the underworld.

---

### Track 0 — BALMORA (foundations)

*Small, unglamorous, and everything else is gated on it.*

#### MORROWIND-A — The engine census

The DOOM-A and CONTROL-A analogue. **No code in any other sub-phase is written
until this exists.**

1. **The census script**, checked in beside its output, regenerating §4's tables:
   lines and tests per crate, absent-system greps, WGSL inventory, schema count,
   public-API surface per crate, and a dependency-justification list that
   catches dead dependencies like the `egui` triple (§4.7).
2. **The Fyrox diff** (§6.1): every `fyrox-ui/src/` module against
   `somnium_ui/src/`, with keep / adapt / refuse and a one-line reason. This is
   the input to Tracks 1 and 2 and it is worth a day.
3. **The license audit** (§6.6), including the Flax question, resolved before
   any sub-phase leans on Flax.
4. Creates `dev records/phase MORROWIND/`, opens `ATTRIBUTION.md` §13H, and
   updates `context.md` §17.6 with §1.3's mapping.
5. Creates `examples/vvardenfell` as an empty program that opens a window and
   draws nothing, so every later sub-phase has somewhere to land.

**Exit:** the census command reproduces §4 without a human editing a table.

#### MORROWIND-A2 — The wgpu 30 bump *(added by the 2026-08-23 web pass)*

**This sub-phase adds no feature.** It exists because §12.4 requires a toolchain
bump to be taken alone, and because §6.9.1 found that wgpu **30.0.0 released
2026-07-01** while Somnium sits on 29 — and three Track 7 sub-phases plus
Track 5's skinning are written against capabilities that exist in 30 and not 29.

1. **Bump and compile.** The two known breaking changes, both mechanical and
   both wide: WGSL's `push_constant` address space becomes **`immediate`**
   across 48 shader files, and `subgroup_min_size` / `subgroup_max_size` move
   from `Limits` to `AdapterInfo`. `EXPERIMENTAL_RAY_QUERY` also absorbs the
   former separate acceleration-structure feature, so Somnium's ray-query paths
   follow the rename.
2. **Probe, do not trust.** A capability report, printed at startup and checked
   in, for `EXPERIMENTAL_MESH_SHADER`, `MULTI_DRAW_INDIRECT_COUNT`,
   `ACCELERATION_STRUCTURE_BINDING_ARRAY`, `SHADER_I16`, storage-texture binding
   arrays, and the subgroup sizes — **on the actual target hardware.** Two of
   those names contain `EXPERIMENTAL_` and the changelog's "fully supported on
   Vulkan" is a claim, not a measurement (§6.9.1's caveat).
3. **`.somtime` parity on both shipped maps.** A version bump that changes the
   frame is a regression until explained. This is the whole acceptance test.
4. **Record what 30 unlocks, and build none of it here.** Mesh shaders are the
   native expression of what `meshlet.rs` plus `cull.wgsl` currently emulate,
   and that is a Track 7 investigation with its own measurement — not a thing to
   start while the bump is still settling.
5. Updates the frozen toolchain line in this document's preamble and in
   `context.md`. **wgpu 29 is frozen until this sub-phase, and wgpu 30 after
   it** — the freeze is a rule about unannounced changes, not about staying still.

**Exit:** the engine runs on wgpu 30, the capability report is checked in, and
the frame time on both maps is unchanged within measurement noise.

#### MORROWIND-B — The job system (Seam 1)

New crate `somnium_jobs`. A worker pool sized to `available_parallelism() - 1`,
a priority queue with deadlines, cancellation tokens, a completion queue drained
under a frame budget, and an optional `#[cfg]` single-threaded mode so tests
stay deterministic. **Full API sketch in Appendix A.3.1.**

> **Read this before writing a line of it.** CONTROL Seam 2 already introduces a
> `JobRegistry` in `somnium_core` — bounded queue, worker pool, cancellation,
> progress — and CONTROL ships first. **MORROWIND-B promotes that into
> `somnium_jobs` and extends it; it does not write a second one.** §10's "one
> job system" row and §11's row 12 both forbid the alternative. Appendix A.6
> states the reconciliation and what CONTROL should do to make the move a rename
> rather than a rewrite.

- **First customers, in this sub-phase**, so the API is tested by three unlike
  users before twenty depend on it: CONTROL-C's thumbnail decode (232–260 ms,
  §4.4), glTF import, and BC7 terrain encoding.
- **Instrumentation is not optional**: every job reports to the Phase 29
  profiler as a CPU zone with its priority and its queue wait. A job system
  without visibility is a source of mystery hitches.
- **Scope, stated so it is not quietly widened:** this is a system for
  *background* work. It does **not** make rendering multi-threaded —
  `jobs.rs:3` already admits "Record still happens on the render thread" and
  that stays true. The principled fix for *that* is Panda3D's pipeline cycling
  (`panda3d-master/panda/src/pipeline/pipelineCyclerTrueImpl.cxx`,
  `cycleData.cxx`), where every piece of scene state holds one copy per pipeline
  stage so App, Cull and Draw read different consistent snapshots without locks.
  It is also a change to every piece of scene state in the engine, so it is
  **out of this phase and recorded in §14.8** rather than half-started here.
- Reference: O3DE `AzCore/IO/Streamer/` for the deadline and priority contract;
  `bevy/bevy-main/crates/bevy_tasks/` for the Rust shape.

**Exit:** opening `assets/terrain/` (60 PNGs, 1.17 GB) never drops a frame, and
the profiler shows why.

#### MORROWIND-C — The shader system (Seam 3)

New crate `somnium_shader`. Retires `material/hlms.rs`.

1. **Composition**: a WGSL module registry with named includes, cycle detection
   and a resolved-source cache. The 48 existing shaders migrate; `shading.wgsl`
   (1,750 lines) is the acceptance case.
2. **Permutation**: `ShaderKey`, variant compilation as a job, hash-keyed
   pipeline cache, and a compile-time-registered define set so a typo is a build
   error rather than a silent miss.
3. **Hot reload**: debug-build file watcher, module-granular invalidation,
   recompile on a job, atomic pipeline swap, and a visible toast on failure with
   the naga diagnostic — never a silent revert to the old pipeline.
4. **Variant budget**: a build-time report of variants per module. A key that
   generates thousands is a design error, and the report is how it gets caught
   before it is a startup stall.
5. **Ahead-of-time**: a `tools/` cooker that compiles the shipped variant set at
   build time so a release build has no first-use hitch.

**Rust precedents, read first (§6.8.1f).** `terra-main/rshader/src/` is three
files — `lib.rs`, `dynamic_shaders.rs`, `static_shaders.rs` — and is items 3 and
5 above already working: hot-reloading sources in development, baked variants in
release, behind one interface. `bevy-plugins/bevy_mod_outline-master/src/pipeline_key.rs`
is item 2's key as an idiomatic Rust type. Both are small enough to read in an
hour and they displace Daemon's `gl_shader.cpp` (**GPL, read-only**) to a
secondary reference. Panda3D's `pgraph/renderState.cxx` +
`renderAttribRegistry.cxx` + `stateMunger.cxx` is worth reading as the
*alternative* answer — interned, composed, cached state objects instead of a
permutation key — before committing to the key.

**Exit:** editing `brdf.wgsl` updates the running editor in under a second;
adding a `SKINNED` define adds a variant without editing `renderer.rs`;
`hlms.rs` is deleted.

---

### Track 1 — VIVEC (the runtime UI)

*Vivec is a city of thirteen cantons laid out to one plan, which is the only
reason a city that size is navigable. The runtime UI is the same argument.*

**This is the track with the most value per line**, because §4.5 shows the gap
is not "some widgets are missing" — it is that the rasteriser cannot express the
shapes a game UI is made of.

#### MORROWIND-D — The paint layer, part two (Seam 4b)

Extends, never edits, the Hades contract.

1. **The second instance stream**: `ShapedInstance` with a 2x3 affine transform,
   stroke (width, join, cap, dash), path or arc reference, mask slot, bindless
   texture index. Its own pipeline; same pass; existing `draw_over` ordering.
2. **Paths and strokes**: line, polyline, quadratic and cubic bezier, arc, with
   joins and caps. Tessellated on the CPU into the shaped stream. This single
   item unblocks the node graph's wires (MORROWIND-K), the timeline's curves
   (MORROWIND-L), the spline editor (MORROWIND-P) and every radial or rotated
   game widget.
3. **Arbitrary textures**: the three fixed bindings (`pass.rs:226/242/259`)
   become a bindless array with those three as entries 0–2. A game registers a
   texture and gets a slot. `push_nine_slice` (`draw.rs:360`) finally has
   something to reference.
4. **Masking**: clip to a path or an alpha texture, not only a `Rect`.
5. **Render-to-texture**: a subtree renders to an offscreen target, consumed as
   a texture — which is what makes a graph node's material preview, a minimap
   and a world-space canvas one mechanism instead of three.
6. **Gradients**: multi-stop, plus radial and angular.

Reference: `fyrox-ui/src/vector_image.rs` for the in-architecture precedent;
`bevy-plugins/bevy_vello-main` read to decide explicitly **against** a
compute-based vector rasteriser if tessellated paths suffice — and to record
why, so the question is not reopened annually.

**Runtime artefact:** `DrawingContext::push_path`, `push_stroke`,
`push_transformed`, `register_texture`, `push_mask`, `begin_layer`.
**GHOSTFENCE:** the 646-instance / 56-rounded / 29-washed / 21-lifted /
5-recessed / 17-stroked composition Phase 27 measured on the 1920x1080 shell is
**byte-identical** after this lands.

#### MORROWIND-E — The canvas (Seam 4a)

1. `Canvas` roots: `Screen { scaler }`, `World { transform, size, billboard }`,
   `Overlay { camera }`.
2. **Anchors** layered on the existing measure/arrange core: min/max anchor,
   offsets, pivot, stretch — the RectTransform vocabulary, without discarding
   Fyrox's arrange pass (Seam 4's rationale).
3. **Scaling**: constant pixel, scale-with-resolution, constant physical size;
   plus **safe area** insets, which nothing in the tree currently models and
   every shipped game needs.
4. **World-space**: the decision is recorded in this sub-phase, between
   render-to-texture-then-quad (composites with the visibility buffer trivially,
   costs a target per canvas, resamples text) and direct 3D submission (crisp,
   needs depth and ordering integration). Flax's `UICanvas.cpp` implements both
   modes and is the reference for the trade-off.
5. **Layers and sorting**, so a tooltip is above a panel is above a HUD without
   anyone computing a z by hand.

**Slice:** `vvardenfell` gets a HUD and a floating name-plate over an object.

#### MORROWIND-F — Input routing, focus and gamepad navigation

1. Hit-testing against transformed and masked shapes (needs MORROWIND-D).
2. Focus: capture, scope, tab order, and a **focus visual** that satisfies
   Zeta's four-cue state grammar rather than inventing a fifth cue.
3. **Directional navigation**: explicit neighbour links where authored,
   geometric search where not — Godot's model and Unity's together, because each
   fails alone (explicit links are unmaintainable at scale; geometric search
   picks the wrong widget in dense layouts).
4. Pointer, touch and gamepad as one event stream; hover has no meaning on a pad
   and the API must say so rather than pretending.
5. Consumes MORROWIND-AE's action map for navigation verbs, so a player's
   rebound "confirm" works in menus. **This is a forward dependency and Track 8
   must land AE before F closes** — noted in §9.

#### MORROWIND-G — Text, properly

The largest single sub-phase in Track 1, and the one most likely to be
under-estimated.

1. **Shaping.** `fontdue` (workspace `Cargo.toml:96`) rasterises glyphs and does
   not shape. Arabic, Devanagari, Thai and even English ligatures and kerning
   pairs need a shaper. Phase 27 already deferred `cosmic-text` once
   (`phase_27.md` status), and **the ecosystem moved while it was deferred**
   (§6.9.2): `cosmic-text` now shapes through `harfrust` — a Rust HarfBuzz — and
   rasterises through `swash`, which has absorbed parts of Google's font stack;
   `parley` is the Linebender alternative and needs Rust 1.85+, which 1.88
   satisfies. **Two credible candidates, not one deferred one. This sub-phase
   decides and does it**, and records the measured cost to the existing text
   pipeline, because the Hades block-origin snapping rule is frozen and a shaper
   that breaks it is not acceptable.
2. **Rich text**: a tag vocabulary — colour, size, weight, style, inline sprite,
   link, and wave/shake for damage numbers. Reference: `fyrox-ui/src/bbcode.rs`
   and `formatted_text/`, in-architecture.
3. **Font fallback chains**, so a CJK glyph in an English UI renders rather than
   showing tofu.
4. **Bidi and vertical text**, scoped honestly: bidi is in; vertical writing
   modes are explicitly deferred (§14.5).
5. **IME**: composition strings, candidate windows, and the winit plumbing.
   Without it the engine cannot accept a Japanese character in a text box.
6. **Localisation hook**: text is a key plus arguments, resolved through
   `somnium_i18n` (MORROWIND-AH), never a baked literal.

#### MORROWIND-H — UI motion

Phase 27 shipped `motion.rs` (524 lines) for editor chrome. This generalises it
to a runtime system: a tween and transition API with easing curves that come
from **CONTROL-K's curve editor**, state transitions, staggering, and a spring
model for the cases where duration is the wrong parameterisation. LyShine's
`Animation/` (`AnimNode`, `AnimSequence`, `AnimSplineTrack`) is the reference for
the track-based variant, which is also MORROWIND-L's fifth consumer.

#### MORROWIND-I — Accessibility

The row nobody plans and everybody eventually needs. An accessibility tree
mirroring the widget tree, exposed to platform screen readers; focus and role
announcements; a respect-reduced-motion setting wired to MORROWIND-H; and a
contrast mode that reuses Zeta's certified pairs rather than inventing a second
palette. Reference: `bevy/bevy-main/crates/bevy_a11y/` for the Rust integration
shape — and, decisively, **Godot 4.5 shipped AccessKit screen-reader support**
(§6.9.2). A mainstream engine has now done this in a self-rendered UI, which
moves the sub-phase from speculative to precedented and means the integration
questions have public answers.

**Scoped honestly:** this sub-phase delivers the tree and the reader
integration. It does not deliver a conformance claim (§14.5).

---

### Track 2 — THE CONSTRUCTION SET (editor reach beyond CONTROL)

*Morrowind shipped its editor in the box. That is the standard.*

#### MORROWIND-J — Docking, floating windows, multiple viewports

1. A dock tree (tiles, splitters, tabs) replacing the fixed five-region shell,
   with the current arrangement as the **default layout** so nothing looks
   different on first run.
2. Floating windows — real OS windows via winit, not in-app fakes, because the
   second monitor is the whole point.
3. **Multiple viewports**, each with its own camera, view mode and overlays,
   including a quad split. This is where the renderer learns to render more than
   one view per frame, and that is a real cost: MORROWIND-J carries a `.somtime`
   row for the four-viewport case and a documented default of one.
4. Layout persistence extending `layout_persist.rs` (181 lines), plus named
   workspaces and a reset.

References: `FlaxEngine-master/Source/Editor/GUI/Docking/` (six files, a
from-scratch retained-mode implementation) and `fyrox-ui/src/dock/` (the same
idea in Somnium's ancestor).

#### MORROWIND-K — The graph surface (Seam 8a)

The highest-leverage sub-phase in Track 2. **One** implementation:

1. Nodes, pins, typed connections with validity rules, bezier wires (needs
   MORROWIND-D), box selection, zoom and pan (needs transforms), comments,
   reroutes, groups, alignment, copy and paste, undo through CONTROL's command
   registry, a searchable node palette, and sub-graph contexts.
2. **Archetype-driven**: `NodeArchetype` / `NodeElementArchetype` /
   `GroupArchetype` as data. A feature contributes a catalogue.
3. Serialisation to a graph asset, versioned.
4. **First consumer in this sub-phase: the material graph**, layered on
   CONTROL-D's `.sommat` — the graph *compiles to* material parameters and a
   generated WGSL variant through MORROWIND-C, so a graph material and a
   property material are the same runtime object.

Later consumers, each cheap once this exists: the animation graph
(MORROWIND-V), the behaviour tree (MORROWIND-Y), the VFX graph (MORROWIND-AA)
and the scattering graph (MORROWIND-P2).

Reference: `FlaxEngine-master/Source/Editor/Surface/` (§6.2 — eight tools, one
surface) and `o3de-development/Gems/GraphCanvas` plus `Gems/GraphModel` for the
framework-not-tool framing.

#### MORROWIND-L — The timeline (Seam 8b)

**One** track-and-media timeline: tracks, groups, media clips, keyframes,
playhead, scrubbing, zoom, snapping, markers, and an embedded **CONTROL-K curve
editor** for a selected channel. Archetype-driven like MORROWIND-K.

Consumers: the animation editor (Track 5), the sequencer and cinematics, the VFX
timeline, the audio track view, and MORROWIND-H's UI animation. Reference:
`FlaxEngine-master/Source/Editor/GUI/Timeline/` and O3DE `Gems/Maestro`
(`Cinematics/`).

#### MORROWIND-M — Virtualisation, data tables, and the localisation editor

1. A **virtualising container** — recycled rows, windowed hit-testing, stable
   selection across scroll — retro-fitted to the outliner, the content drawer
   and the asset browser. Acceptance is 100,000 rows at 60 fps; nobody has
   measured the current ceiling and MORROWIND-A does.
2. A **data table editor** — typed columns, sorting, filtering, multi-cell edit,
   CSV import and export. Its first customer is the localisation table, its
   second is any game's item or dialogue data.
3. **Asset dependency view**: what references this, what this references, what
   breaks if it is deleted. Built on MORROWIND-Q's dependency graph.

#### MORROWIND-M2 — The GUI layout editor *(added by the 2026-08-23 survey)*

Track 1 as first written gives a *game* a UI framework and leaves authoring to
code. **Two independent engines say that is half-shipped**:
`Babylon.js-master/packages/tools/guiEditor/` and
`fyrox/Fyrox-master/editor/src/ui_scene/` both author game UI inside the editor,
in the same widget system the editor itself is built from (§6.8.1d).

Somnium is unusually well placed to do this, because its editor **is** the
framework — the same retained tree, the same paint layer, the same measure and
arrange. The work is therefore mostly plumbing rather than a second system:

1. A `.somui` document asset: a widget tree with anchors (MORROWIND-E),
   properties driven by CONTROL-B's schema seam, and versioned serialisation.
2. A canvas-mode viewport that edits one, with drag-to-place, anchor handles,
   a resolution/aspect preview, and a safe-area overlay.
3. The widget palette generated from the registered widget types — not a second
   hand-written list, per CONTROL-A2's command registry precedent.
4. Load and instantiate from Rust and from Luau, so a `.somui` is a runtime
   asset and not an editor-only convenience.
5. **Uses MORROWIND-O's prefabs for reuse** rather than inventing a UI-specific
   nesting model. A UI panel is a prefab; that is the whole feature.

**This sub-phase is the proof of Track 1.** If a `.somui` authored in the editor
cannot be loaded by `examples/vvardenfell` and driven by script, Track 1 built a
framework nobody can reach.

#### MORROWIND-N — Play-in-editor

The editor and the game become distinguishable: a play/pause/step control, a
snapshot of world state on enter and a restore on exit, separate input focus, a
runtime-versus-editor flag visible to script, and an error path that returns to
edit mode rather than taking the editor down. Modest in code, large in what it
makes possible — every subsequent track becomes testable without a restart.

**The snapshot-and-restore is the hard half**, and the reference is an unlikely
one: `renpy-master/renpy/rollback.py` (1,217 lines) with `revertable.py`, where
every mutable container logs its mutations so the whole game state can be
stepped backwards arbitrarily (§6.8.2). Somnium does not need arbitrary rewind —
one snapshot depth is enough — but the *discipline* is the transferable part:
state that can be restored has to be built from containers that know they were
written to, and retro-fitting that is far more expensive than choosing it. This
**There is a third option and it is structurally different: run the game in a
separate process.** Korge does it in six files (§6.9.3) — a memory-mapped file
as a shared framebuffer (`KorgeFrameBuffer.kt`: `channel.map(READ_WRITE, 0,
32 + width * height * 4)`, a 32-byte header then pixels, remapped on resize), a
socket carrying input events back (`KorgeIPCSocket.kt`, with a packet ring
buffer), the pair discovered from one env-var path as `"$path.frame"` and
`"$path.socket"` (`KorgeIPC.kt`), and a host panel displaying it
(`KorgeIPCJPanel.kt`). Crash isolation and a real play/stop boundary then fall
out of process separation instead of out of a snapshot mechanism. It is not
obviously right for Somnium — a shared framebuffer costs a full-resolution copy
per frame, and the editor viewport is a gizmo surface rather than a passive
image — but it is a real alternative and cheap to prototype.

This sub-phase therefore chooses between **three** designs and records why: a
full-world snapshot (simple, memory-hungry, merely slow at Somnium's current
scale); mutation logging in Ren'Py's `revertable.py` style (cheap per frame,
invasive, must be chosen early or not at all); and out-of-process (isolation for
free, a copy per frame, a viewport-integration problem). Ren'Py's model is also
the second half of MORROWIND-AF's argument that a save is not a scene
serialisation.

---

### Track 3 — HLAALU (composition)

*Every interior in Morrowind is kit-bashed from a modular set. Somnium cannot
place the same rock twice and edit it once.*

#### MORROWIND-O — Prefabs (Seam 6)

1. `.somprefab` asset: a template scene fragment with stable ids.
2. Instances with **patch-based overrides**, expressed in CONTROL's
   `(StableId, FieldId, ReflectValue)` vocabulary, inheriting `ChangeScope`.
3. **Nesting**: templates referencing templates; patches addressing a path.
4. Editor surface: create-from-selection, enter and exit instance editing,
   propagate, revert, break-link, and an outliner that shows override state per
   field — the last is what makes overrides comprehensible rather than
   mysterious.
5. Scene format migration with a version bump and a round-trip test.

Reference: O3DE's `Prefab/` (`Instance/`, `Link/`, `Overrides/`, `PrefabDom*`),
with Flax's simpler model read as the alternative and the choice recorded.

#### MORROWIND-P — Splines and blockout

1. **Splines** as a first-class component: control points, tangents, closed
   loops, arc-length parameterisation, and a viewport gizmo (needs MORROWIND-D
   for the curve rendering). Customers: roads, rivers, patrol routes, camera
   rails, scatter guides.
2. **Blockout geometry** — boxes, ramps, stairs, cylinders — editable in the
   viewport and convertible to a mesh asset. Level design currently requires
   leaving the editor entirely. References: O3DE `Gems/WhiteBox`, and — **in
   Rust, which is why it is worth reading first** —
   `bevy-plugins/bevy_trenchbroom-main/src/` with `brush.rs`, `qmap/`, `bsp/`
   and `class/`. Its `fgd.rs` is the part this plan had not anticipated: an
   **entity-definition schema** that gives blockout brushes typed, authorable
   entity classes rather than anonymous geometry. That is the same idea as
   Somnium's component schemas (§4.8) arriving from level-design instead of from
   reflection, and MORROWIND-P should make them one mechanism rather than two.

#### MORROWIND-P2 — Rule-driven scattering

Replaces the paint-brush-only model (Phase 17A/17F) with the O3DE composition:
gradient sources (noise, image, slope, altitude, distance, shape), surface tags
classifying ground, filters and exclusion volumes, distribution and density —
**authored in MORROWIND-K's graph surface**, which is why this sub-phase is
cheap and would not otherwise have been.

Reference: `Gems/Vegetation` + `Gems/GradientSignal` + `Gems/SurfaceData`,
authored through `Gems/LandscapeCanvas`.

---

### Track 4 — SILT STRIDER (the cook and the stream)

*Nothing in this track is visible, and every open world is made of it.*

#### MORROWIND-Q — The cook

1. **A native format** per asset kind — meshes, textures, audio, scenes,
   prefabs, shaders — written by a cooker under `tools/`, with a version and a
   content hash.
2. **A dependency graph**, so a changed texture recooks its material and nothing
   else.
3. **Incremental and cached**, keyed by content hash plus cooker version.
4. **Deterministic**: same input, same bytes. This is what makes a build cache
   shareable and a diff meaningful.
5. The editor loads source in development and cooked data in a build, behind the
   *same* `AssetId` (Seam 2), so no runtime code branches on it.

Today: 101 MB of foliage is re-parsed on every launch, and glTF import is slow
enough that Phase 17H had to cache *failed* imports to stop the paint brush
stalling on a retry (`context.md` §17.6).

**References, with Defold promoted to primary (§6.8.2).**
`defold-dev/defold-dev/engine/` ships **four standalone cookers, one per asset
kind** — `atlasc`, `modelc`, `shaderc`, `texc` — beside `resource/`, `rig/` and
a schema-driven data format in `ddf/`. That is item 1's shape, already
decomposed, in a shipping engine famous for small deterministic builds. It also
has **`liveupdate`** — post-ship patchable content, which nothing else in the
tree ships. **MORROWIND-Q does not build live update, and its content-hash and
manifest design must not preclude it**; one sentence in the format spec now is
free, and retrofitting it is not. Secondary: Flax's
`Source/Engine/ContentImporters` + `Content` + `Streaming`; Stride's asset
compiler; jMonkeyEngine's `jme3-core/.../export/` `Savable` binary format;
`bgfx-master/tools/` for the standalone-cooker shape.

#### MORROWIND-R — Residency and hot reload

1. A **byte budget** with an eviction policy and a per-frame upload budget, so a
   burst of residency does not become a stall.
2. **LOD residency**: a mesh may be resident at LOD 2 and absent at LOD 0. This
   is what makes the budget a budget rather than a limit.
3. **Placeholders**: a request returns immediately with a stand-in; the swap is
   atomic.
4. **Hot reload** for every cooked kind, on the MORROWIND-B watcher and
   MORROWIND-C's shader precedent.
5. A residency panel — what is loaded, why, how big, who asked. Somnium
   currently cannot answer "why is this 900 MB" for any build.

#### MORROWIND-S — World partition

1. **Cells**: a spatial hash over the world, with authored and derived contents.
2. **Streaming sources**: cameras, players and explicit volumes, each with a
   radius and a priority. A cell's want-state is a function of sources and
   nothing else — which is what keeps streaming debuggable.
3. **Async load and unload as jobs with deadlines** (Seam 1), cancelled when a
   source moves away.
4. **One-file-per-actor-style storage**, so a large world is not one file two
   people cannot edit at once. Even for a single developer, it is what makes the
   scene file diffable.
5. **Entity ownership across an unload, which is the question everyone forgets
   until it leaks.** Luanti's answer (§6.8.1g) is adopted: an active entity is
   **serialised into its cell when the cell unloads** and re-instantiated when
   it loads, so identity is owned by the cell rather than by a global list —
   `luanti-master/src/staticobject.cpp` beside `activeobjectmgr.h` and
   `serverenvironment.cpp` (**LGPL, pattern only**). The corollary this
   sub-phase must state explicitly: an entity referenced from *outside* its cell
   needs a stable id that survives the round trip, which is Seam 6's `StableId`
   and the reason it is already in the plan.
6. Editor: cell grid overlay, load-state visualisation, manual pin and unpin.

Reference: UE's `WorldPartition/RuntimeSpatialHash/` and `LoaderAdapter/`
(**proprietary, pattern only**); Luanti's block emerge pipeline as the
battle-tested counterpart (**LGPL, pattern only**).

**Refuse:** UE's data layers and content bundles. They solve a
multi-team, multi-DLC problem Somnium does not have, and they are most of the
complexity.

#### MORROWIND-T — HLOD, impostors, floating origin

1. **HLOD**: cells bake a merged proxy mesh with a merged material for distant
   display, generated by the cook.
2. **Impostors**: octahedral billboards for distant foliage and props, baked
   offline. Reference: `terra-main/src/billboards.rs` — **in Rust, in the tree.**
3. **Floating origin.** f32 world coordinates fail past a few kilometres, in
   ways that look like shadow acne and z-fighting rather than like a precision
   bug, which is why it is worth deciding before it is observed. **Two Rust
   references disagree, and the disagreement is the decision (§6.8.1e):**
   `bevy-plugins/bevy_terrain-main/src/big_space.rs` uses a hierarchical
   integer-grid-plus-float-offset space on the CPU, and
   `terra-main/src/shaders/softdouble.glsl` emulates double precision **inside
   the shader**. **MORROWIND-T decides and records the reasoning**; the
   provisional recommendation is the `big_space` shape — camera-relative
   rendering plus periodic origin rebasing — because it is reversible, it is
   contained on the CPU side, and `softdouble` is what a planet-scale variant
   would need later rather than now.

---

### Track 5 — DWEMER (animation)

*The Dwemer left behind machines that still move. Somnium's characters do not
move at all.*

#### MORROWIND-U — Skinned meshes and GPU skinning

1. Skeleton, skin binding and vertex weights imported from glTF (the `gltf`
   crate is already a workspace dependency, `Cargo.toml:90`).
2. `SkinningPalette` upload (Seam 7).
3. **The hard part: skinning inside a visibility buffer.** Somnium's pipeline
   culls meshlets against a static geometry pool (`geometry.rs`, `meshlet.rs`,
   `culling.rs`). Skinned geometry moves per frame, which invalidates meshlet
   bounds and the Hi-Z occlusion assumption. Two candidate designs, and
   MORROWIND-U picks with a measurement rather than a preference:
   - **Skin-to-buffer**: a compute pass writes posed vertices into a transient
     pool slice; the existing culling and visibility path is unchanged, at the
     cost of bandwidth and memory proportional to posed vertices.
   - **Skin-in-shader**: the visibility pass applies the palette during
     rasterisation; no extra memory, but meshlet bounds must be conservatively
     expanded and every consumer of the geometry pool must be taught about it —
     including ray tracing, which `geometry.rs:122` notes reads positions
     straight out of the shared pool.
   The second interacts badly with ReSTIR and with RT water reflections, which
   is the argument for the first; the sub-phase measures both on a
   thousand-character scene before choosing.
4. Needs MORROWIND-C: `SKINNED` is a permutation, not a branch in a 1,750-line
   shader (§4.3).

#### MORROWIND-V — Clips, blend trees, state machines

Scope taken from Esoterica's runtime node list (§6.3): clip sampling with
looping and time scaling; `Blend1D` and `Blend2D`; bone masks and layers; a
state machine with conditions, transitions and blend times; parameters;
**sync tracks**, without which blended locomotion foot-slides; and pose caching.

Authored in **MORROWIND-K's graph surface** — an `AnimGraphSurface` in Flax's
sense — with `fyrox-ui/src/absm/` as the in-architecture precedent for the state
machine editor specifically.

#### MORROWIND-W — Root motion, IK and events

1. **Root motion** extraction and application, with the collide-and-slide
   interaction against the character controller decided here rather than
   discovered later.
2. **IK**: two-bone for limbs, foot IK with ground adaptation (Esoterica's
   `FootIK` node), look-at. Esoterica's `OrientationWarp` is worth reading and is
   explicitly optional.
3. **Events**: footsteps, sounds, gameplay hooks — sampled deterministically
   across a frame boundary, including when the frame is long, which is the case
   everyone gets wrong once.
4. **Ragdoll**: already paid for by Jolt (§4.1), so blending animation into and
   out of a ragdoll is exposure rather than implementation.

#### MORROWIND-W2 — Compression and the pose task graph

Clip compression (curve fitting against an error budget), and pose evaluation as
a job graph rather than a recursive walk — Esoterica's `TaskSystem/`, and a
direct customer of MORROWIND-B. Deferred within the track if time is short, and
listed separately so that deferral is deliberate.

**Motion matching is out of this phase** (§3.6). The data it would need —
per-frame feature vectors cooked from clips — is named here so MORROWIND-Q's
clip format leaves room for it.

---

### Track 6 — SIXTH HOUSE (navigation and AI)

#### MORROWIND-X — Navmesh and pathfinding

1. **Bake** from level geometry: voxelise, region-grow, contour, triangulate —
   the Recast pipeline. As a cook step (MORROWIND-Q), per cell (MORROWIND-S), on
   jobs (MORROWIND-B).
2. **Query**: A* with funnel string-pull smoothing, raycast, nearest-point.
3. **Agents**: steering, local avoidance, off-mesh links (ladders, jumps).
4. **Dynamic obstacles**: carving, with a partial rebake.
5. **Rust reality check — re-framed by §6.9.2, because the crates exist.**
   `oxidized_navigation` is a pure-Rust runtime tiled Recast port;
   `polyanya` does any-angle planning over overlapping navmeshes with one-way
   and cost-weighted layers; `landmass` supplies agents and avoidance;
   `recast_navigation` is an FFI wrapper for the C++ original. **The
   Somnium-specific catch: `oxidized_navigation` consumes `parry3d` colliders
   through an `OxidizedCollider` trait, with published integrations for Rapier
   and Avian — and Somnium uses Jolt.** So the real decision is *"write a
   Jolt-to-parry collider adapter"* versus *"bind Detour through FFI"*, and the
   in-tree Jolt FFI precedent (`somnium_physics_sys`) favours the latter.
   `polyanya` is worth taking regardless of which side wins, because its layer
   model is better than Detour's for a world with bridges and multiple floors.
   **None of these crates was read; confirm before committing.**

References: `Daemon-master/src/engine/botlib/` (a shipping Recast/Detour bot
stack, **GPL, pattern only**), O3DE `Gems/RecastNavigation`, Esoterica's navmesh
tooling.

#### MORROWIND-Y — Behaviour trees and perception

A behaviour-tree runtime (composites, decorators, tasks, a blackboard), authored
in **MORROWIND-K's surface** — Flax's `BehaviorTreeSurface.cs` is the eighth tool
on the one surface and the proof the pattern holds. Plus a perception system:
sight cones with occlusion queries, hearing with falloff, and a memory of
stimuli. Plus Luau task nodes, so gameplay logic stays in script and the tree
stays a structure rather than a language.

---

### Track 7 — RED MOUNTAIN (rendering)

*Every sub-phase here carries a `.somtime` row on both shipped maps, and a
sub-phase that cannot show one has not finished.*

#### MORROWIND-Z — Virtual shadow maps

A page table over a virtual shadow resolution, page allocation driven by
screen-space demand, a cache with invalidation on light or geometry change, and
clipmap pages for the directional case. **Highest visual-quality-per-frame-cost
item in the track**, and the one that makes many shadow-casting lights possible
at all. Ships alongside the existing CSM path with a per-light choice and a
measured default.

**References, and the order matters because the primary one may not be copied.**
UE's `VirtualShadowMaps/` (`VirtualShadowMapCacheManager.h` for invalidation,
`VirtualShadowMapClipmap.h` for the directional case) is **proprietary, pattern
only (§6.6)** — read for architecture, implement from published literature. The
published literature exists and §6.9.5 names it: the sparse-virtual-shadow-map
write-up at `ktstephano.github.io/rendering/stratusgfx/svsm` covers GPU-side page
management with shader atomics and frame markers, and is the document this
sub-phase actually implements from. **A search on 2026-08-23 found no Rust/wgpu
VSM implementation to read; search again before starting.** Note also that
wgpu 30's `MULTI_DRAW_INDIRECT_COUNT` and mesh shaders (MORROWIND-A2) change
what the page-rendering pass can do, which is one reason A2 is a prerequisite.

#### MORROWIND-AA — GPU particles and the VFX graph

Retires `set_particles(Vec<GpuParticle>)` (`renderer.rs:1296`).

1. Compute simulation with persistent buffers and indirect dispatch.
2. Sorting; depth-buffer collision; ribbons and trails; mesh particles.
3. **Authored as a graph** (MORROWIND-K) with a **timeline** (MORROWIND-L) —
   the two Track 2 primitives paying for themselves.
4. Reference: `WickedEngine-master/WickedEngine/wiEmittedParticle.cpp`; O3DE
   `Gems/OpenParticleSystem` for the data model;
   `bevy-plugins/bevy_enoki-master` for the Rust/wgpu shape.

#### MORROWIND-AB — The GI tier below ray query

Somnium has ReSTIR GI and nothing beneath it (§5.2). **Pick one, in this
sub-phase, with a measurement:** DDGI (probe volumes; robust, cheap) or a baked
lightmapper (the tier that runs on anything, and the right answer for static
architecture). Reference:
`FlaxEngine-master/Source/Engine/Renderer/GI/DynamicDiffuseGlobalIllumination.cpp`
and `GlobalSurfaceAtlasPass.cpp` — the only pair in the tree that ships both a
probe volume and a surface cache side by side.

#### MORROWIND-AC — Transparency, AA and the materials backlog

OIT (technique chosen against wgpu's actual feature set, with per-pixel linked
lists the likely answer where the required atomics are available and
weighted-blended the portable fallback); **SMAA**
(`FlaxEngine-master/Source/Engine/Renderer/AntiAliasing/SMAA.cpp`, sitting
beside the `FXAA.cpp` Somnium already ported); subsurface scattering; contact
shadows; and **deferred decals if and only if CONTROL-O drops them** (§6.7).

#### MORROWIND-AD — Virtual texturing

Streaming virtual textures with a feedback pass and a page cache, plus a runtime
virtual texture for terrain composition, so the 32-layer material's memory
becomes a budget rather than a ceiling. **The most expensive item in the track
and the most deferrable**; listed last for that reason.

---

### Track 8 — ALMSIVI (the game framework)

*Three things that hold everything else up.*

#### MORROWIND-AE — Input actions (Seam 5)

New crate `somnium_input`. Action maps, actions, bindings, control paths,
processors (dead zone, invert, scale, normalise), interactions (hold, tap,
multi-tap), composite bindings (WASD as a 2D vector), device layouts, hot-plug,
multi-device pairing, and **runtime rebinding with conflict detection**. Stored
through CONTROL-H's settings. The sixteen inline `KeyCode::` arms in
`hello_engine` and the fifty-four in `script_input.rs` migrate.

Reference: `Unity3D/InputSystem-develop/…/Runtime/Actions/InputAction.cs`,
`InputActionMap.cs`, `InputBinding.cs`.

#### MORROWIND-AF — Save games and game state

A save is **not** a scene serialisation, and the distinction is the sub-phase: a
scene records what an author built; a save records what a player changed, and
must survive the author shipping a patch that moves things. Content-versioned
slots, a migration path, partial saves scoped to streamed cells (Seam 2 and
MORROWIND-S), metadata and a screenshot, plus a game-state stack (menu, loading,
playing, paused). Reference: O3DE `Gems/SaveData` and `Gems/GameState`.

#### MORROWIND-AG — Audio, from 93 lines to a crate

The cheapest large win in the phase (§4.2).

1. **Buses** with volume, mute, solo and DSP chains — the file that currently
   reads `// Bus stub`.
2. **Spatialisation**: 3D positioning, attenuation curves (from CONTROL-K),
   Doppler, cones, and a **listener** — the file that currently reads
   `// Listener stub`.
3. **Reverb zones** and occlusion, queried against the physics world Somnium
   already has.
4. **Streaming** for music, and a cache, so a sound played twice is read once
   (`engine.rs:35` reads it twice).
5. **Fix the discarded volume argument** (`engine.rs:36`) and add the test that
   would have caught it — a one-line fix and a permanent lesson about the
   second-example rule.
6. A mixer panel, and audio tracks in MORROWIND-L's timeline.

Kira already provides most of the machinery; this is exposure and design, not
DSP.

#### MORROWIND-AH — Localisation, video, and the boundary

1. **`somnium_i18n`**: string tables keyed by identifier, plural and gender
   rules, runtime locale switching, font fallback coordination with MORROWIND-G,
   and an extraction tool that finds every user-visible string. Ren'Py's
   `renpy/translation/` is the reference for extraction and round-trip, and it is
   genuinely better than the engine references at this specific task.
2. **Video**: a decoder to a wgpu texture. §6.9.2 found a better answer than the
   assumed FFmpeg-decode-then-upload: **`vk-video` decodes through Vulkan Video
   directly into a `wgpu::Texture`**, so frames never leave GPU memory. That
   removes video's main objection. Fallback `ffmpeg-next`; `dav1d` for AV1.
   Cutscenes and in-world screens. Still lowest priority in the track, but no
   longer expensive enough to justify deferring on cost alone.
3. **The boundary**: `examples/vvardenfell` becomes a small playable slice — a
   character that walks with animation, a HUD, an NPC that paths around an
   obstacle, positional audio, a save and a reload, in a streamed world — built
   **only** from public crate APIs. If any of it requires reaching into engine
   internals, the API that forced it is fixed before this sub-phase closes.

**This sub-phase is the phase's acceptance test.** Everything above is
scaffolding for it.

---

## 9. Sequencing

### 9.1 Hard prerequisites

```
MORROWIND-A                  --> everything
MORROWIND-A2 (wgpu 30)       --> Z, AA, AC, AD, and U's skinning integration
MORROWIND-B  (jobs)          --> C, Q, R, S, T, U, W2, X, AA, AB, AD, and CONTROL-C's fix
MORROWIND-C  (shaders)       --> U (SKINNED), Z, AA, AB, AC, AD, and K's material compile
MORROWIND-D  (paint)         --> E, F, G, H, J, K, L, M, P
MORROWIND-K  (surface)       --> V (anim graph), Y (behaviour tree), AA (VFX), P2 (scatter)
MORROWIND-L  (timeline)      --> animation editing, sequencer, AA, H's track mode
MORROWIND-Q  (cook)          --> R --> S --> T
MORROWIND-U  (skinning)      --> V --> W --> W2
MORROWIND-AE (input)         --> F closes; hello_engine's inline keycodes migrate
MORROWIND-E + O              --> M2 (the GUI layout editor needs anchors and prefabs)
Phase CONTROL, complete      --> this entire phase
CONTROL-K    (curves)        --> H, L, and AG's attenuation curves
```

### 9.2 The two enabling primitives, restated

MORROWIND-D (the paint layer) and MORROWIND-K (the graph surface) each unblock
five or more sub-phases. They are the two places where under-investing costs the
most later, and the two places where a "just enough for this one feature"
implementation will have to be rewritten. §6.2 is the evidence.

### 9.3 If only part of the phase is executed

The recommended cut, in order, and the reasoning:

1. **Track 0 (A, B, C)** — three sub-phases that make everything else possible
   and that the renderer wants on its own merits. Hot shader reload alone
   changes the daily experience of working on this engine.
2. **MORROWIND-D + E + G** — the paint layer, the canvas and text. After these,
   a game can have a UI, which is the largest single capability gap (§5.1).
3. **MORROWIND-U + V** — skinned meshes and blend trees. After these, a game can
   have a character.
4. **MORROWIND-Q + R** — the cook and residency. After these, a game can ship.
5. **MORROWIND-AE + AG** — input and audio. Both small, both conspicuous.

That is eleven sub-phases and it closes seven of §5.1's nine rows. Everything
else in this document is the difference between an engine that can run a game
and an engine that can run an *open world*, which is a real difference and a
later one.

### 9.4 Interleaving with PORTAL and CONTROL

- **CONTROL first, in full — decided, not merely recommended.** Six of eight
  tracks consume CONTROL's seams. MORROWIND-A's first act is to reconcile §7
  against the seams CONTROL actually shipped (§12.8).
- **PORTAL is orthogonal and should land before Track 4.** PORTAL's CI gates,
  lint policy and `.somtime` parity harness are what make a thirty-six
  sub-phase plan survivable; without them GHOSTFENCE (§10) is a manual process,
  and manual processes at this scale do not hold.
- **Track 7 can interleave with everything**, because it touches the renderer
  and almost nothing else touches the renderer. If the phase stalls elsewhere,
  Track 7 is where progress is still possible.

---

## 10. GHOSTFENCE — the must-not-break matrix

*The Ghostfence held back the one thing that would have ended everything. This
is the same idea and it should be as boring as it sounds.*

Every sub-phase runs GHOSTFENCE before it closes. It is a script, not a habit,
and MORROWIND-A writes it.

**The 2026-08-23 survey upgraded this section, and the upgrade is the single
cheapest quality mechanism in the phase.** Somnium has 945 tests and **zero
image assertions**; every visual claim in every phase record rests on a human
looking at a screenshot. Stride ships the fix —
`stride-master/sources/engine/Stride.Graphics.Regression/` with `ImageTester.cs`,
`ImageThreshold.cs`, `TestResultImage.cs`, `GameTestBase.cs`,
`FrameGameSystem.cs`, `RegressionTestAttribute.cs` and `FpsTestCamera.cs`,
beside a sibling `Stride.Games.AutoTesting` (§6.8.1a). A fixed camera, a fixed
frame index, a stored reference PNG, a perceptual threshold, and a failure that
writes the diff. **MORROWIND-A builds it as the first GHOSTFENCE row**, because
every subsequent row in this table is a promise until something can fail.

| Invariant | Check | Owner |
|---|---|---|
| **Golden images** | A fixed camera and frame on each shipped scene matches a stored reference within a perceptual threshold; failures write a diff image | **MORROWIND-A builds it; every visual sub-phase runs it** |
| The Hades paint contract | The 1920x1080 shell emits a byte-identical primitive stream: 646 instances, 56 rounded, 29 washed, 21 lifted, 5 recessed, 17 stroked | MORROWIND-D and every Track 1/2 sub-phase |
| Nocturne tokens and contrast pairs | Zeta §8A.3 pairs re-certified | Track 1, Track 2 |
| Frame time on both shipped maps | `.somtime` row, stddev reported, no regression beyond noise | **Every Track 7 sub-phase**, plus J (multi-viewport) and U (skinning) |
| DOOM defaults | Dynamic resolution opt-in; tile binning, aerial terrain, hex/POM default off | Track 7 |
| XV terrain contract | 32 layers, `GpuTerrainMaterial` size, sidecar version | Track 4, Track 7 |
| Water and foliage numbers | Great Lakes constants, foliage LOD and cull distances | Track 4, Track 7 |
| CONTROL's six seams | Signatures unchanged; MORROWIND adds, never edits | All |
| Test suite | 945 tests green, and the count only goes up | All |
| One job system | No second thread pool, no bare `thread::spawn` outside `somnium_jobs` | All, from B onward |
| One graph, one timeline | No second implementation of either | Track 2 onward |
| Scene round-trip | Every scene in the repository loads, saves and reloads byte-identically | Track 3, Track 4 |
| Toolchain | rustc 1.88, wgpu 29, winit 0.30 unless a sub-phase says otherwise and stands alone | All |

---

## 11. Acceptance matrix

A sub-phase is done when its row is true. These are the rows that are easy to
skip and expensive to skip.

| # | Row | Applies to |
|---|---|---|
| 1 | A public API exists in a `somnium_*` crate, documented, with tests | All |
| 2 | A Luau binding exists where the system is script-relevant | All gameplay-facing |
| 3 | `examples/vvardenfell` exercises it through public APIs only | All |
| 4 | A component schema is registered (§4.8) and CONTROL-B generates its rows | All with components |
| 5 | Every knob is reachable in the editor with a label, range, unit, tooltip, undo step and Help line | All |
| 6 | Long operations go through `somnium_jobs` with a priority and a deadline | All from B onward |
| 7 | A `.somtime` row exists and shows no regression | Track 7, plus J and U |
| 8 | GHOSTFENCE passes | All |
| 9 | `ATTRIBUTION.md` §13H entries are written, with §6.6's license rule observed | All |
| 10 | `context.md` is updated | All |
| 11 | Evidence captured after tonemapping, in `dev records/phase MORROWIND/` | All visual |
| 12 | No second thread pool, graph surface, timeline, UI framework or reflection system was created | All |

---

## 12. Risks and controls

**12.1 The phase is too large to finish.** Very likely. Control: §9.3's eleven
sub-phase cut is the real plan; the rest is the map. Each track is independently
valuable and independently abandonable, and the tracks are ordered so that
stopping after any of them leaves the engine better rather than half-migrated.

**12.2 The enabling primitives get built minimally and then rewritten.**
Control: MORROWIND-D and MORROWIND-K each name their downstream consumers in
their own sub-phase text, and their exit criteria are stated in terms of those
consumers rather than their first one.

**12.3 Skinning does not fit the visibility buffer cleanly.** The real technical
risk in Track 5. Control: MORROWIND-U measures both designs before choosing, and
the fallback — a separate forward pass for skinned geometry — is ugly, cheap,
and available.

**12.4 A wgpu feature turns out to be missing.** OIT, virtual texturing and
virtual shadow maps each lean on capabilities that may not be available.
**Partly resolved by §6.9.1**: wgpu 30.0.0 (2026-07-01) adds
`EXPERIMENTAL_MESH_SHADER`, `MULTI_DRAW_INDIRECT_COUNT` and
`ACCELERATION_STRUCTURE_BINDING_ARRAY`, and **MORROWIND-A2 is the bump**, taken
alone, as this rule requires. The risk is not closed by that, only moved: two of
those flags are named `EXPERIMENTAL_` and a changelog entry is not a
measurement. Control unchanged: **A2 probes every flag on real hardware and
checks the report in, and each dependent sub-phase still opens with its own
probe and a recorded fallback** before any implementation.

**12.5 The text rewrite breaks the frozen paint contract.** MORROWIND-G replaces
the shaping layer under a snapping rule Phase 27 froze. Control: GHOSTFENCE's
first row, and MORROWIND-G lands the shaper behind a flag with an A/B before it
becomes the default.

**12.6 Streaming introduces non-determinism into the test suite.** Control:
`somnium_jobs` ships a single-threaded deterministic mode from MORROWIND-B, and
tests use it.

**12.7 Copyleft and proprietary contamination.** Three references are not
permissive (§6.6). Control: MORROWIND-A's license audit, the stricter rule for
those three, and a GHOSTFENCE row that greps shipped source for identifiers
characteristic of them.

**12.8 CONTROL moves under this phase. — RETIRED 2026-08-23.** The original
risk was that CONTROL is a plan rather than a tree, so its seams could move
under work built on them. **The decision that Phase CONTROL completes in full
first removes it.** What remains is a smaller obligation: MORROWIND-A re-reads
CONTROL's *shipped* seam signatures and updates §7 to match what was actually
built, because a completed phase and its plan are never identical, and this
document was written against the plan.

---

## 13. Evidence plan

`dev records/phase MORROWIND/` is created by MORROWIND-A. Per-track evidence:

- **Track 0**: the census output; a recording of a shader edit reaching the
  running editor; a profiler capture showing the job queue during a terrain
  folder open.
- **Track 1**: the shell before and after MORROWIND-D at 1920x1080 and at the
  redline minimum (proving byte-identity); a game HUD; a world-space canvas; a
  rich-text sample including a CJK fallback and a bidi line; a gamepad
  navigation recording.
- **Track 2**: a four-viewport layout; the material graph; the animation graph;
  the timeline; 100k rows scrolling with a frame-time trace.
- **Track 3**: a prefab edited once and updated in fifty places; a scatter graph
  and its output.
- **Track 4**: cook timings before and after; a residency panel during a walk
  across the world; cell load state overlaid on the terrain.
- **Track 5**: a walk cycle; a blend tree at three speeds with sync tracks on
  and off (the foot-slide comparison *is* the evidence); foot IK on a slope; a
  `.somtime` row for a thousand skinned characters.
- **Track 6**: navmesh overlay; an agent pathing around a dynamic obstacle.
- **Track 7**: `.somtime` rows on both maps for every sub-phase; before and
  after captures for VSM, particles, GI and AA.
- **Track 8**: a rebinding UI; a save and reload across a restart; an audio mixer
  with a positional source; `examples/vvardenfell` running.

Captures after tonemapping. The HDR target holds values far above one and a PNG
written from it directly is worthless as evidence.

---

## 14. Left open, deliberately

**14.1 Networking.** Out (§3.1). Two decisions in this phase are made
network-compatible anyway and it costs nothing: prefab instances carry stable
ids (Seam 6), and poses are data separable from their producer (Seam 7). If
networking is ever wanted, Daemon's delta-compressed snapshot model
(**GPL, pattern only**) is the reference to start from.

**14.2 Visual scripting.** Not planned (§3.2). If it is ever wanted, it is a
fifth catalogue on MORROWIND-K's surface compiling to Luau, and that is
precisely why the surface is built archetype-driven.

**14.3 Cloth, hair and fur.** Deferred. Jolt provides soft bodies, so cloth is
closer than it looks; hair is a rendering research problem
(`o3de/Gems/AtomTressFX`, `WickedEngine/wiHairParticle.cpp`) and is not what an
open-world RPG fails on first.

**14.4 Motion matching.** Out of Track 5 (MORROWIND-W2). The clip format leaves
room for the feature vectors it would need.

**14.5 Vertical text, and an accessibility conformance claim.** MORROWIND-G
delivers bidi and defers vertical writing modes. MORROWIND-I delivers a screen
reader integration and explicitly does not claim conformance to any standard;
claiming one requires an audit this project has not scoped.

**14.6 The render graph.** Refused with reasoning (§5.4), and MORROWIND-C is
designed so the refusal is reversible.

**14.7 Console and mobile.** Not assumed anywhere; not precluded anywhere.

**14.8 Multi-threaded rendering.** Out. `jobs.rs:3` already states that
recording happens on the render thread, and MORROWIND-B does not change that
(§8, Track 0). The principled fix is Panda3D's **pipeline cycling**
(`panda3d-master/panda/src/pipeline/pipelineCyclerTrueImpl.cxx`, `cycleData.cxx`)
— one copy of every piece of scene state per pipeline stage, so App, Cull and
Draw read different consistent snapshots without locks. It touches every piece
of scene state in the engine and is a phase of its own. Recorded so that a later
phase adopts a known-good design instead of inventing a worse one under
deadline. Nothing in MORROWIND should make it *harder*: Seam 7's pose/palette
split and Seam 2's residency states are both already snapshot-shaped.

**14.9 Sandboxed native gameplay code.** Out. Daemon runs game logic in an
out-of-process sandboxed VM over a command-buffer IPC
(`Daemon-master/src/engine/framework/VirtualMachine.cpp`, `src/common/IPC/`,
`CommonVMServices.cpp` — **GPL, pattern only**), which buys crash isolation, hot
reload and mod safety together. Somnium gets a weaker form of all three from
Luau, which is why this is out. It is the right reference if native gameplay
plugins are ever wanted, and `korge-main/korge-ipc/` is the lighter-weight
variant of the same idea (out-of-process editor/game IPC) that a later pass
should read first.

**14.10 A declarative/reactive UI layer.** Refused by §3.8, and the arguments
against the refusal are recorded rather than suppressed, because there are now
three: Ren'Py's screen language (`renpy-master/renpy/sl2/`,
`renpy/display/screen.py`) re-evaluates a declarative tree each frame and diffs
it against the retained one; **s&box's runtime UI is Razor components with
CSS-like styling** (`sbox-public-master/engine/Sandbox.Razor/` with
`Microsoft.AspNetCore.Components`); and Unity's UI Toolkit is UXML plus USS.
Two commercial engines and the most ergonomic UI model in the tree all chose
markup-plus-stylesheet. **Somnium will still not have two UI models.** If this
is ever revisited it is a *layer over* the retained tree, never beside it —
which is exactly what MORROWIND-M2's `.somui` document is the first step toward.

**14.11 Hot reload that requires migrating live state.** MORROWIND-C reloads
shaders and MORROWIND-R reloads cooked assets, and **both are easy for the same
reason: their live state is a handle table, so a reload swaps what a handle
points at and nothing else moves.** Do not generalise from that. s&box named a
whole project after the hard case — `Sandbox.Hotload/` with
`InstanceUpgrader.cs`, `Upgraders/` and `UpdateReferences.cs` (§6.9.4) — because
reloading code whose *types* changed means walking every live object graph,
upgrading each instance, and rewriting every reference to it. Somnium will hit
this the first time someone edits a component's fields while the editor is
running. **It is out of this phase**, and the reason it is written down is so a
later session does not schedule it as "extend the hot reload we already have."

---

## 15. Start checklist

Before MORROWIND-B, a session must have:

1. Read this file, `phase_CONTROL.md`, `phase_27.md` §6 and §12,
   `phase_DOOM.md` §15, and `context.md` §6, §8, §11, §12, §17.6 and §18.
2. Confirmed **Phase CONTROL is complete**, not merely started. If it is not,
   stop. Then reconcile §7's seams against what CONTROL actually shipped
   (§12.8) — the seams in this document were written against CONTROL's *plan*.
3. Run MORROWIND-A and read its output — the census, the Fyrox diff, the license
   audit — rather than trusting §4 and §6 of this file, which were measured on
   2026-08-23 and will have drifted.
4. Confirmed `dev records/phase MORROWIND/` exists and is empty of invented PNGs.
5. Confirmed `ATTRIBUTION.md` §13H is open and §13E/F/G are untouched.
6. Confirmed `examples/vvardenfell` exists and builds.
7. Re-read §6.6 and know which three references are not permissive.
8. Accepted §9.3's cut, or written down why a different cut is better. **Do not
   start at MORROWIND-Z because it is the interesting one.**

---

## 16. Research sources and confidence

**Verified by reading or listing on 2026-08-23** (high confidence): every
`crates/` figure and `file:line` in §4; the directory listings in §6.1
(`fyrox-ui/src/`, `fyrox/editor/src/`), §6.2 (Flax `Source/Editor/Surface/` with
its line counts, `Source/Editor/GUI/Timeline/`, `Source/Editor/GUI/Docking/`),
§6.3 (Esoterica `Code/Engine/Animation/` and `Graph/Nodes/`), §6.4 (O3DE
`Prefab/`, `AzCore/IO/Streamer/`, `Gems/Vegetation`, `Gems/GradientSignal`,
`Gems/GraphCanvas`, `Gems/LyShine/.../Animation`, `Gems/TextureAtlas`,
`Gems/Maestro`, `Gems/MotionMatching`, `Gems/SaveData`; UE
`Runtime/Renderer/Private/VirtualShadowMaps/` and
`Runtime/Engine/Private/WorldPartition/`), §6.5 (Unity InputSystem
`Runtime/Actions/`), `terra-main/src/`, `bevy/bevy-main/crates/`, Flax
`Source/Engine/Renderer/GI/` and `AntiAliasing/`, and the `WickedEngine/` file
names.

**The second pass (§6.8), 2026-08-23** — verified by listing: Stride's
`Stride.Graphics.Regression/` file list and `Stride.Games.AutoTesting`; Panda3D's
`pipeline/` (`pipelineCyclerTrueImpl`, `cycleData`, `cyclerHolder`),
`pgraph/` (`renderState`, `renderAttribRegistry`, `stateMunger`,
`cullBinManager`) and `pstatclient/`; Babylon's `packages/tools/` (six node
editors plus `guiEditor` and `accessibility`) and `packages/dev/`; Daemon's
`framework/VirtualMachine.cpp` and `common/IPC/`; Ren'Py's module list with
`atl.py` at 2,368 lines and `rollback.py` at 1,217; Defold's `engine/` module
list including the four `*c` cookers and `liveupdate`; Luanti's `src/` including
`staticobject`, `pathfinder`, `emerge` and `gettext_plural_form`;
`terra-main/rshader/src/` and `src/shaders/softdouble.glsl`; the `src/` listing
of all sixteen `bevy-plugins`; jMonkeyEngine's `jme3-core/.../jme3/` package
list; Ogre-Next's `Compositor/` (path corrected — it nests twice);
`Unity3D/PostProcessing-2/.../Effects/`; Stride's `sources/engine/` module list;
VoxelHex's `src/`.

**Named but not read in depth** (medium confidence — a sub-phase must read
before relying): the *contents* of the UE, O3DE, Flax, Panda3D, Ren'Py, Defold,
Luanti and Babylon files above, as opposed to their existence and role; Stride's
`Stride.UI`; Daemon's `gl_shader.cpp`; the bevy-plugin internals.

**Deliberately not opened, listed so the next pass knows where to start:**
`korge-main/korge-ipc/` and `korge-reload-agent/` (hot-reloading a running game;
out-of-process editor/game IPC — the two most likely to hold something),
`sbox-public-master`, `LuminaEngine-main`, `raylib-master`,
`Unity3D/UnityCsReference-master`, `Unity3D/ml-agents-develop`, and the four
engines Phase CONTROL already covered for editor patterns.

**The third pass (§6.9), 2026-08-23** — the web research that had not completed
is now done, and this paragraph replaces the one that said it never would be.

*Fetched and read* (medium-high confidence, but see the caveat):
`raw.githubusercontent.com/gfx-rs/wgpu/trunk/CHANGELOG.md` for **wgpu 30.0.0,
2026-07-01** and its feature-flag names. **The caveat is load-bearing: a
changelog was read, no feature was exercised.** Two of the names carry
`EXPERIMENTAL_`, and "fully supported on Vulkan" is the changelog's claim.
MORROWIND-A2 exists to turn this into a measurement, and §12.4 is unchanged.

*Search results, single-source, unverified* (low-to-medium confidence — **each
owning sub-phase confirms before committing**): the navmesh crates
(`oxidized_navigation`, `polyanya`, `landmass`, `recast_navigation`) and the
`parry3d`-versus-Jolt collider mismatch; `cosmic-text`'s move to `harfrust` and
`swash`/`skrifa`, and `parley`'s Rust 1.85 floor; `vk-video` decoding into a
`wgpu::Texture`; **Godot 4.5 shipping AccessKit screen-reader support**; the
sparse-virtual-shadow-map write-up at `ktstephano.github.io`. Crate *versions*
were deliberately not recorded — a wrong version number in a plan costs a day,
and these will have moved by the time anyone builds against them.

*Searched for and not found:* a Rust or wgpu virtual-shadow-map implementation
worth reading. §8's MORROWIND-Z says to search again before starting.

*Verified by reading source, 2026-08-23:* `korge-main/korge-ipc/` — the
memory-mapped framebuffer (`KorgeFrameBuffer.kt`, the `FileChannel.map` call and
the 32-byte header), the socket/queue files, and the `KORGE_IPC` path convention
in `KorgeIPC.kt`; `korge-reload-agent/`'s two files; and the file listings of
`sbox-public-master/engine/Sandbox.Hotload/`, `Sandbox.Razor/` and
`Sandbox.Tools/`. The s&box *contents* were not read — the argument in §6.9.4
rests on file names, and file names are weak evidence, which is why it is
phrased as "the naming tells the story" rather than as a description of the code.

**Still not verified anywhere in this document:** the *contents* of the UE,
O3DE, Flax, Panda3D, Ren'Py, Defold, Luanti and Babylon files cited in §6.2–6.8,
as opposed to their existence and role.

**Corrected rather than quietly fixed:** the first draft deleted an Ogre-Next
compositor citation because the path could not be verified. The path was wrong,
not the claim — several repositories in `example_repo` nest one level
(`ogre-next-master/ogre-next-master/`, `UnrealEngine-release/UnrealEngine-release/`,
`o3de-development/o3de-development/`, `fyrox/Fyrox-master/`, `bevy/bevy-main/`,
`defold-dev/defold-dev/`, `VoxelHex-main/VoxelHex-main/`,
`rbfx-master/rbfx-master/`, `SolersEngine-main/SolersEngine-main/`,
`ebitenui-master/ebitenui-master/`). The citation is restored in §5.4 and §6.8.2.
**Anyone re-running this reconnaissance should check for the double nesting
first**; it is the reason the first pass under-reported the tree.

---

# Appendix A — Implementation reference

*Added 2026-08-23. §§0–16 are the plan and the argument; this appendix is the
part a cold session needs in order to start typing. Nothing here changes a
decision above — where the two disagree, §§0–16 win and this appendix is stale.*

## A.1 Orientation: read these eleven things, in this order

A session that reads only this file will write plausible code that does not
compile against the tree. Budget roughly two hours:

| # | Path | Read for | Approx |
|---|---|---|---|
| 1 | `crates/somnium_ecs/src/reflect.rs` | `StableId`, `FieldId`, `ReflectValue`, `FieldType`, `FieldSchema`, `ComponentSchema`, `FieldFlags`. **Every seam in this phase speaks this vocabulary.** | 1,369 ln |
| 2 | `crates/somnium_core/src/reflect_registry.rs` | The twelve registered schemas and the `component_schema!` call shape (`:342–353`) | 713 ln |
| 3 | `crates/somnium_ui/src/primitive.rs` | The frozen 100-byte instance and its 12 vertex attributes. Seam 4b extends *around* this | 338 ln |
| 4 | `crates/somnium_ui/src/pass.rs` | The three fixed texture bindings (`:226`, `:242`, `:259`) MORROWIND-D replaces with an array | 757 ln |
| 5 | `crates/somnium_ui/src/draw.rs` | `DrawingContext` — every `push_*` a widget can call | 784 ln |
| 6 | `crates/somnium_ui/src/widget.rs` + `message.rs` | `Widget`, `NodeHandle`, `UiMessage`, `WidgetMessage`. Track 1 grows all four | 330 ln |
| 7 | `crates/somnium_renderer/src/geometry.rs` | `GeometryPool`, `MeshAllocation`, `upload_mesh_pooled`, `reserve_vertices`. **Track 5 lands here** | 700 ln |
| 8 | `crates/somnium_renderer/src/meshlet.rs` + `culling.rs` | What skinned geometry has to survive | 1,400 ln |
| 9 | `crates/somnium_renderer/src/material/hlms.rs` | 29 lines. Read it to understand what MORROWIND-C replaces | 29 ln |
| 10 | `crates/somnium_renderer/src/jobs.rs` | 75 lines. Same reason, for MORROWIND-B | 75 ln |
| 11 | `crates/somnium_core/src/scene_schema.rs` | Scene round-trip. Seam 6 extends the format | 1,055 ln |

Then skim `crates/somnium_renderer/src/renderer.rs` (4,383 lines) — do not read
it end to end; grep it for the pass you care about.

## A.2 Glossary — Somnium words a general LLM will guess wrongly

| Term | What it means **here** |
|---|---|
| **Visibility buffer** | Somnium's deferred pipeline writes triangle/instance ids, not G-buffer attributes; `shading.wgsl` (1,750 ln) reconstructs attributes from the id. Consequence: *anything that moves geometry per frame is a pipeline problem, not a shader problem.* |
| **Meshlet** | A ~64-triangle cluster with bounds, culled on GPU in `cull.wgsl`. Skinned geometry invalidates its bounds — the core of MORROWIND-U. |
| **`StableId`** | `pub struct StableId(&'static str)` — a durable component name like `"somnium.Water"`, written to files. **Not** an entity id. |
| **`FieldId`** | `pub struct FieldId(pub u16)` — declaration-order index within a schema. Wire-stable. |
| **`ReflectValue`** | The neutral value enum: `Nil, Bool, I64, F64, Str, Vec2, Vec3, Vec4, Quat, Entity(Option<Entity>), Asset(Option<AssetRef>), Array(..)`. Engine `f32` widens to `F64` crossing this boundary. |
| **Schema** | `ComponentSchema` — `stable_id`, `display_name`, `version`, `fields: Vec<FieldSchema>`, plus `snapshot` / `read_field` **function pointers**. Not a trait object. |
| **`NodeHandle`** | `Handle<UiNodeTag>` — a generational-pool handle into the widget tree. Fyrox-derived. |
| **Paint contract / Hades** | Phase 27's frozen rules: the 100-byte `Primitive`, `draw_over` ordering, block-origin text snapping. |
| **Token sheet / Nocturne** | Phase 26-Zeta's colour tokens and certified contrast pairs. |
| **`.somtime`** | A deterministic GPU timing run with a stddev per row (Phase DOOM). The only accepted evidence for a frame-time claim. |
| **GHOSTFENCE** | This phase's regression gate (§10). |
| **Reached** | CONTROL's word: a knob has a labelled control with range, unit, tooltip, undo step and Help line. |
| **Slice** | `examples/vvardenfell`, the second example this phase builds (preamble). |

## A.3 Seam code, expanded

Sketches, not final APIs. They compile in shape against the types in A.1; they
have not been compiled against the tree. **Types quoted from `reflect.rs` and
`primitive.rs` are exact; everything else is proposal.**

### A.3.1 Seam 1 — `somnium_jobs`

```rust
// crates/somnium_jobs/src/lib.rs
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Priority {
    /// Blocks a visible frame. Streaming cell the camera is entering.
    Critical = 0,
    /// User is waiting and can see it. Thumbnail of a visible tile.
    Interactive = 1,
    /// Speculative. Prefetch, off-screen thumbnails.
    Background = 2,
}

#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) { self.0.store(true, Ordering::Relaxed); }
    pub fn is_cancelled(&self) -> bool { self.0.load(Ordering::Relaxed) }
}

pub struct JobDesc {
    pub name: &'static str,          // profiler zone label — not optional, see below
    pub priority: Priority,
    /// Wall-clock instant after which the result is worthless. A job whose
    /// deadline has passed while queued is dropped, not run.
    pub deadline: Option<Instant>,
    pub cancel: CancelToken,
}

pub struct JobHandle<T> { /* oneshot receiver + the token */ _t: std::marker::PhantomData<T> }

pub struct JobSystem { /* worker threads, a per-priority queue, a completion queue */ }

impl JobSystem {
    /// `workers` is normally `available_parallelism() - 1`; the main thread is
    /// the one we are protecting.
    pub fn new(workers: usize) -> Self { todo!() }

    /// Deterministic mode for tests (§12.6): jobs run inline on `submit`.
    pub fn new_single_threaded() -> Self { todo!() }

    pub fn submit<T: Send + 'static>(
        &self,
        desc: JobDesc,
        f: impl FnOnce(&CancelToken) -> T + Send + 'static,
    ) -> JobHandle<T> { todo!() }

    /// Called **once per frame, on the main thread.** Applies finished work
    /// (GPU uploads, tree mutations) until `budget` is spent.
    ///
    /// Returning with work still outstanding is correct behaviour, not a bug:
    /// that is the mechanism that stops a burst of completions becoming a
    /// frame spike. See Seam 1.
    pub fn drain_completions(&self, budget: Duration) -> DrainStats { todo!() }
}

pub struct DrainStats {
    pub applied: usize,
    pub still_pending: usize,
    pub budget_exhausted: bool,
}
```

**Three rules that are not obvious from the signatures.**

1. `name` is `&'static str` and mandatory because every job becomes a Phase 29
   CPU zone. A job system without profiler visibility converts one mystery
   (a stall) into a harder one (a stall somewhere in a thread pool).
2. The work closure takes `&CancelToken` rather than checking a global, so a
   long loop can poll it: `if cancel.is_cancelled() { return Err(Cancelled); }`.
   Cancellation that is only checked between jobs is not cancellation.
3. **Completion application is main-thread and budgeted.** The worker produces
   *data*; the main thread installs it. Nothing touches `wgpu::Queue` or the
   widget tree off-thread.

**Worked call site** — CONTROL-C's 232–260 ms thumbnail decode (§4.4):

```rust
let token = CancelToken::default();
let handle = jobs.submit(
    JobDesc {
        name: "thumbnail.decode",
        priority: if tile_visible { Priority::Interactive } else { Priority::Background },
        deadline: None,
        cancel: token.clone(),
    },
    move |cancel| decode_and_downscale(&path, 128, cancel),
);
// scrolled away before it finished:
token.cancel();
```

### A.3.2 Seam 3 — `somnium_shader`

```rust
// crates/somnium_shader/src/lib.rs
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct ModuleId(pub u32);

/// A define set as a bitset. Compile-time-registered so a typo is a build
/// error rather than a silent cache miss on a variant nobody compiled.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Default, Debug)]
pub struct Defines(u64);

pub mod define {
    use super::Defines;
    pub const SKINNED:        Defines = Defines::bit(0);
    pub const ALPHA_CUTOUT:   Defines = Defines::bit(1);
    pub const DOUBLE_SIDED:   Defines = Defines::bit(2);
    pub const INSTANCED:      Defines = Defines::bit(3);
    // ... one const per define; `Defines::bit` is const fn
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct ShaderKey { pub module: ModuleId, pub defines: Defines }

pub struct ShaderSystem { /* source registry, resolved-source cache, pipeline cache */ }

impl ShaderSystem {
    /// Register a WGSL source under a name that `#include` can reference.
    pub fn register(&mut self, name: &'static str, src: &'static str) -> ModuleId;

    /// Resolve includes, apply defines, compile, cache by `ShaderKey` hash.
    ///
    /// On a cache miss this compiles **synchronously** and stalls one draw.
    /// Prefer `request` during load so misses are paid off-frame.
    pub fn pipeline(
        &mut self,
        device: &wgpu::Device,
        key: ShaderKey,
        layout: &wgpu::PipelineLayout,
        desc: &PipelineDesc,
    ) -> &wgpu::RenderPipeline;

    /// Compile ahead of use, on a job (Seam 1).
    pub fn request(&self, jobs: &JobSystem, key: ShaderKey, desc: &PipelineDesc);

    /// Debug builds only. Returns the modules whose source changed; the caller
    /// invalidates every cached pipeline whose key transitively includes one.
    #[cfg(debug_assertions)]
    pub fn poll_file_watcher(&mut self) -> Vec<ModuleId>;
}
```

**The include convention.** WGSL has no preprocessor, so the system does the
text work before handing source to naga:

```wgsl
//!include "brdf.wgsl"
//!include "sampling.wgsl"
//!if SKINNED
//!include "skinning.wgsl"
//!endif
```

Rules that stop this becoming a second language: `//!include` only at file
scope; `//!if` / `//!endif` on whole lines, no nesting deeper than one, no
expressions beyond a single define name and optional `!`. Anything more and the
answer is a second module, not a cleverer conditional. Cycle detection is a
depth-first walk with a visited set — cheap, and the alternative is a stack
overflow inside naga with an unreadable message.

**wgpu 30 note (MORROWIND-A2):** WGSL `enable` directives are **file-scoped and
must precede all other declarations**. Composing a module that declares
`enable wgpu_binding_array;` with one that does not is fine; composing two that
declare *conflicting* extensions is not. The resolver must hoist every `enable`
from every included module to the top of the resolved source and de-duplicate.
Get this wrong and the error surfaces as a naga parse failure pointing at line 1
of a file nobody edited.

**Variant budget.** Emit at build time:

```
module              defines  variants  compiled
shading.wgsl              6        64        11
terrain_material.wgsl     3         8         8
```

A module with six independent defines has 64 possible variants and probably
compiles eleven. If `compiled` approaches `variants`, the key is too coarse and
the fix is splitting the module, not a bigger cache.

### A.3.3 Seam 4b — the shaped UI instance

The frozen instance, quoted exactly from `primitive.rs:63` so the extension can
be checked against it:

```rust
#[repr(C)]
pub struct Primitive {
    pub rect: [f32; 4], pub uv: [f32; 4], pub radii: [f32; 4], pub shadow: [f32; 4],
    pub grad_axis: [f32; 2], pub border_width: f32, pub expand: f32,
    pub fill_a: [u8; 4], pub fill_b: [u8; 4],
    pub border_color: [u8; 4], pub shadow_color: [u8; 4], pub flags: u32,
}
const _: () = assert!(std::mem::size_of::<Primitive>() == 100);
```

The addition — **a second buffer, a second pipeline, the same render pass:**

```rust
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShapedInstance {
    /// Row-major 2x3 affine: [a, b, c, d, tx, ty]. Identity = [1,0,0,1,0,0].
    pub xform: [f32; 6],
    /// Index into the shared path/geometry buffer, and how many entries.
    pub geom_offset: u32,
    pub geom_len: u32,
    /// Stroke: width, miter limit, dash period, dash phase. width == 0.0 = fill.
    pub stroke: [f32; 4],
    /// Bindless texture slot. 0 = font atlas, 1 = icons, 2 = thumbnails,
    /// 3.. = registered. u32::MAX = untextured.
    pub texture: u32,
    /// Clip-mask slot, or u32::MAX for the inherited rect clip.
    pub mask: u32,
    pub fill_a: [u8; 4],
    pub fill_b: [u8; 4],
    pub flags: u32,      // JOIN_ROUND | CAP_SQUARE | GRAD_RADIAL | ...
}
```

The draw side keeps ordering intact by tagging, not by sorting:

```rust
pub enum UiInstance { Quad(Primitive), Shaped(ShapedInstance) }

// DrawingContext keeps ONE ordered command list; each command records which
// stream it drew from and the run length, so `draw_over` ordering survives
// two pipelines. Do NOT bucket all quads then all shapes — that reorders
// the shell and GHOSTFENCE's first row catches it immediately.
```

**Why not extend `Primitive` in place?** Because `assert!(size_of == 100)` and
the 12-attribute layout are the Hades contract, 646 instances in the shipped
shell are measured against it, and a widened instance costs that memory on every
flat fill. Two streams cost one extra pipeline and one extra buffer.

**The tessellation boundary.** Bezier flattening happens on the CPU into a
shared vertex buffer, keyed by (path, tolerance) and cached across frames — a
node graph's wires do not change shape while the user pans, and re-flattening
them every frame is the obvious performance mistake. Tolerance is in *device*
pixels, so it must be recomputed when DPI changes (Phase 27 already fixed a DPI
correctness bug; do not reintroduce it).

**Sketch of the shaped fragment shader:**

```wgsl
// ui_shaped.wgsl — companion to the existing ui.wgsl, not a replacement.
struct ShapedIn {
    @location(0) local:   vec2<f32>,   // pre-transform, for stroke math
    @location(1) uv:      vec2<f32>,
    @location(2) fill_a:  vec4<f32>,
    @location(3) fill_b:  vec4<f32>,
    @location(4) @interpolate(flat) texture: u32,
    @location(5) @interpolate(flat) mask:    u32,
    @location(6) @interpolate(flat) flags:   u32,
};

@fragment
fn fs_shaped(in: ShapedIn) -> @location(0) vec4<f32> {
    var color = select_fill(in);                    // flat | linear | radial | angular
    if (in.texture != 0xffffffffu) {
        color *= textureSampleLevel(ui_textures[in.texture], ui_sampler, in.uv, 0.0);
    }
    if (in.mask != 0xffffffffu) {
        color.a *= textureSampleLevel(ui_masks[in.mask], ui_sampler, in.uv, 0.0).r;
    }
    return color;
}
```

Note `ui_textures[in.texture]` — **a binding array, which is why MORROWIND-D
depends on the bindless path and therefore benefits from MORROWIND-A2's
`enable wgpu_binding_array;`.** On a backend without binding arrays the fallback
is a texture-atlas page index plus a UV rect, which is strictly worse for large
game textures; MORROWIND-D must probe and record which it got.

### A.3.4 Seam 7 — pose and skinning

```rust
// crates/somnium_anim/src/pose.rs — no renderer dependency, by design.
#[derive(Copy, Clone)]
pub struct Transform { pub translation: Vec3, pub rotation: Quat, pub scale: Vec3 }

pub struct Skeleton {
    pub names: Vec<String>,
    /// Parent index, or u16::MAX for a root. **Invariant: parent < child.**
    /// Guaranteed at import so `to_model_space` is one forward pass.
    pub parents: Vec<u16>,
    pub inverse_bind: Vec<Mat4>,
}

pub struct Pose { pub skeleton: SkeletonId, pub local: Vec<Transform> }

impl Pose {
    /// One forward pass; correct only because of the parent < child invariant.
    pub fn to_model_space(&self, skel: &Skeleton, out: &mut [Mat4]) {
        for i in 0..self.local.len() {
            let local = Mat4::from(self.local[i]);
            let p = skel.parents[i];
            out[i] = if p == u16::MAX { local } else { out[p as usize] * local };
        }
    }
}
```

The renderer side never sees `Pose`:

```rust
// crates/somnium_renderer/src/skinning.rs
pub struct SkinningPalettes {
    buffer: wgpu::Buffer,              // one big storage buffer, all characters
    ranges: HashMap<Entity, GpuRange>, // per-entity offset+count
}
```

**The join, in one WGSL function** (which design MORROWIND-U picks decides where
it is *called* from — a compute prepass, or the visibility vertex stage):

```wgsl
// skinning.wgsl — included only when the SKINNED define is set.
@group(2) @binding(0) var<storage, read> palettes: array<mat4x4<f32>>;

fn skin_position(p: vec3<f32>, joints: vec4<u32>, weights: vec4<f32>, base: u32) -> vec3<f32> {
    var m = palettes[base + joints.x] * weights.x;
    m += palettes[base + joints.y] * weights.y;
    m += palettes[base + joints.z] * weights.z;
    m += palettes[base + joints.w] * weights.w;
    return (m * vec4<f32>(p, 1.0)).xyz;
}
```

Normals need the inverse-transpose; for rigid-ish skinning the same matrix's
upper 3x3 is close enough and every shipping engine does it. Say so in a comment
rather than leaving a reader to wonder whether it was an oversight.

### A.3.5 Seam 6 — prefab patches

Deliberately built from CONTROL's vocabulary, so the inspector and the override
system are one mechanism:

```rust
// crates/somnium_core/src/prefab.rs
pub struct PrefabInstance {
    pub template: AssetId,
    pub root: Entity,
    /// Deterministically ordered — a scene file must diff cleanly.
    pub patches: Vec<Patch>,
}

pub struct Patch {
    /// Path from the instance root through nested instances. Empty = the root.
    pub path: Vec<u16>,
    pub component: StableId,       // exactly CONTROL Seam 1's vocabulary
    pub field: FieldId,
    pub value: ReflectValue,
}

impl PrefabInstance {
    /// Instantiate the template, then replay patches in order.
    ///
    /// A patch whose component or field no longer exists in the template is
    /// **kept and reported**, never dropped. Silent dropping is exactly the
    /// data-loss path CONTROL §6.2 found in `scene_from_json`, and a prefab
    /// makes it worse because it multiplies by the instance count.
    pub fn instantiate(&self, world: &mut World, reg: &Registry) -> Vec<PatchWarning>;
}
```

**Undo interaction, and it is the subtle one.** CONTROL adopted rbfx's
`AttributeScopeHint` as `FieldSchema::scope: ChangeScope`. A patch inherits it:
patching `roughness` (`ChangeScope::Field`) replays cheaply, but patching
`TerrainComponent::resolution` (`ChangeScope::Entity` — it rebuilds a
heightfield, a collider and a GPU sidecar) cannot be replayed as a scalar write.
**MORROWIND-O must reject or specially handle patches on fields above
`ChangeScope::Field`**, and decide which in the sub-phase rather than in a
bug report.

## A.4 File-by-file change map

What each track touches. `+` new file, `~` modified, `-` deleted.

**Track 0**
```
+ crates/somnium_jobs/{Cargo.toml,src/lib.rs,src/queue.rs,src/worker.rs}
+ crates/somnium_shader/{Cargo.toml,src/lib.rs,src/compose.rs,src/cache.rs,src/watch.rs}
+ tools/census/            (§4 as a script)
+ tools/ghostfence/        (§10, incl. the golden-image runner)
+ tools/shadercook/        (AOT variants)
~ crates/somnium_renderer/src/renderer.rs      (pipeline creation -> ShaderSystem)
~ crates/somnium_renderer/src/shaders/*.wgsl   (48 files: includes; push_constant -> immediate)
- crates/somnium_renderer/src/material/hlms.rs (29 lines, replaced)
~ crates/somnium_renderer/src/jobs.rs          (keep for_each_mut; re-export the pool)
~ Cargo.toml                                    (members; wgpu 29 -> 30 in A2)
```

**Track 1**
```
+ crates/somnium_ui/src/runtime/{mod.rs,canvas.rs,anchor.rs,focus.rs,nav.rs,tween.rs}
+ crates/somnium_ui/src/shaped.rs               ShapedInstance + tessellator
+ crates/somnium_ui/src/text/{shape.rs,rich.rs,fallback.rs,ime.rs}
+ crates/somnium_ui/src/a11y.rs
+ crates/somnium_ui/src/shaders/ui_shaped.wgsl
~ crates/somnium_ui/src/pass.rs                 3 bindings -> binding array; 2nd pipeline
~ crates/somnium_ui/src/draw.rs                 push_path/stroke/transformed/mask/layer
~ crates/somnium_ui/src/font.rs                 fontdue -> shaper
~ crates/somnium_ui/src/motion.rs               generalise to a runtime tween system
  (primitive.rs is NOT modified — that is the point)
```

**Track 4/5/6/8** (abbreviated)
```
+ crates/somnium_anim/, somnium_nav/, somnium_ai/, somnium_input/, somnium_i18n/
+ crates/somnium_renderer/src/skinning.rs + shaders/skinning.wgsl
+ crates/somnium_core/src/prefab.rs
+ tools/cook/{mesh,texture,audio,scene,shader}/
~ crates/somnium_asset/src/lib.rs               AssetDb -> residency + cooked blobs
~ crates/somnium_audio/src/{bus,listener,error}.rs   (currently 1 line each)
~ crates/somnium_renderer/src/geometry.rs       skinned allocation path
~ examples/hello_engine/src/main.rs             16 KeyCode arms -> action map
+ examples/vvardenfell/
```

## A.5 The three integrations that will actually hurt

Everything else is work; these are risk. A cold session should read this section
before estimating anything.

**1. Skinning versus the visibility buffer (MORROWIND-U).** The pipeline assumes
geometry is static: `GeometryPool` hands out permanent vertex ranges,
`meshlet.rs` precomputes bounds, `cull.wgsl` tests those bounds, Hi-Z assumes
last frame's depth predicts this frame's, and **ray tracing reads positions
straight out of the shared pool** (`geometry.rs:122` says so). Skinning breaks
the second, third and fifth of those.

*Skin-to-buffer* — a compute pass writes posed vertices into a transient pool
slice each frame — keeps every downstream consumer working unchanged, including
ray tracing, at the cost of bandwidth and memory proportional to posed vertices.
*Skin-in-shader* is free in memory and requires teaching culling about
conservative bounds **and** rebuilding the BLAS for ray tracing anyway, which is
most of skin-to-buffer's cost without its simplicity.

The plan's expectation is skin-to-buffer. **MORROWIND-U measures both on a
thousand-character scene before choosing** (§8) — and if the measurement is
ambiguous, take the simple one. Fallback if both disappoint: a separate forward
pass for skinned meshes. Ugly, cheap, available, and it should be prototyped
first as the risk floor rather than last as the panic option.

**2. Text shaping versus the frozen snapping rule (MORROWIND-G).** Phase 27
froze block-origin text snapping to get crisp glyphs at 1x DPI. A shaper returns
sub-pixel advances; naive snapping of shaped output destroys kerning, and naive
non-snapping blurs the editor's own chrome. The resolution is to snap the *run
origin* and keep advances sub-pixel within the run — but that is a claim, not a
result. **Land the shaper behind `SOMNIUM_UI_SHAPER=1`, A/B it, and only then
flip the default.** GHOSTFENCE's golden-image row is what makes the A/B
decidable rather than a matter of opinion.

**3. Prefabs versus the scene format (MORROWIND-O).** `scene_schema.rs` (1,055
lines) round-trips a flat entity list. Seam 6 makes a flat scene the degenerate
case, which is a format version bump touching load, save, undo and the outliner
at once. CONTROL §6.2 already found that `scene_from_json` **drops unknown
components and fields with a warning** — a silent data-loss path. Prefabs
multiply that by the instance count. **Fix the drop before adding prefabs**, not
after; it is a smaller change and it makes the migration test meaningful.

## A.6 One reconciliation the two documents need

**CONTROL Seam 2 introduces a `JobRegistry` in `somnium_core`** — a bounded
queue, a worker pool, cancellation and a progress report — for thumbnails, glTF
import, BC7 encode and terrain bake. **MORROWIND-B proposes `somnium_jobs`**
with priorities, deadlines and a budgeted main-thread drain.

**These must not both exist.** §11 row 12 and §10's "one job system" row already
forbid a second thread pool, and CONTROL ships first, so the resolution is fixed:
**MORROWIND-B promotes CONTROL's `JobRegistry` out of `somnium_core` into
`somnium_jobs` and extends it** with `Priority`, `deadline`, and
`drain_completions(budget)`. It does not write a new one and it does not leave
the old one in place beside it.

Two consequences worth stating so CONTROL can make them cheap:

- CONTROL's `JobRegistry` should keep its **public surface small and its
  internals private**, so the promotion is a move rather than a rewrite.
- CONTROL's job callers should already pass a job *name* — the profiler-zone
  label A.3.1 makes mandatory. Retrofitting names across call sites later is
  tedious and always incomplete.

Anything else in this appendix that contradicts CONTROL once CONTROL has shipped
is this document's problem, not CONTROL's: **MORROWIND-A reconciles §7 against
the seams as built** (§12.8).

## A.7 How to verify a track is actually done

Beyond §11's matrix — the specific check that catches the specific way each
track is usually faked.

| Track | The cheat | The check |
|---|---|---|
| 0 — jobs | A pool exists, everything still calls it synchronously | `grep -rn "thread::spawn"` outside `somnium_jobs` returns nothing; the profiler shows queue wait during a terrain-folder open |
| 0 — shaders | Composition works, hot reload silently falls back on error | Introduce a deliberate WGSL syntax error; a toast must show naga's diagnostic and the **old pipeline must stay bound** — not a black screen, not a silent revert with no message |
| 1 — paint | Shapes render, the shell drifted a pixel | GHOSTFENCE row 1 byte-identical **and** the golden image passes |
| 1 — text | Latin looks fine, nothing else was tried | Render one Arabic, one Devanagari, one CJK and one bidi string in the same paragraph; fallback and shaping both fail visibly here and nowhere else |
| 2 — graph | One tool built, "reusable" asserted | **A second catalogue exists.** If only the material graph uses the surface, Seam 8 is unproven |
| 2 — timeline | Same | Two consumers, one of which is not animation |
| 3 — prefabs | Overrides work one level deep | Nest three deep, override at each level, save, reload, and diff the file |
| 4 — cook | Cooking works, determinism assumed | Cook twice from clean into different directories; `diff -r` must be empty |
| 4 — streaming | Cells load; entities leak | Walk a loop that unloads and reloads a cell with a live entity in it 100 times; entity count must return to its starting value (§6.8.1g) |
| 5 — animation | A walk cycle plays | Blend walk→run with sync tracks **off**, capture, turn them **on**, capture. If the two are identical, sync tracks are not wired |
| 6 — nav | An agent reaches a goal | Put a dynamic obstacle on the path mid-traversal |
| 7 — any | It looks better | `.somtime` on both maps, stddev reported, plus the golden image |
| 8 — audio | Sounds play | Assert the volume argument is honoured — `engine.rs:36` discards it today and nothing noticed (§4.2) |
| 8 — the slice | `vvardenfell` runs | `grep` it for `somnium_.*::internal` or any `pub(crate)` reach-through. Zero hits, or the API is wrong |
