# Phase CONTROL — Northlight

> *Control's interface is the building talking to you. Every door, every
> department sign, every case file is authored in the same language as the
> world it describes — which is the only reason a place that rearranges itself
> is navigable at all.*

> **Codename:** CONTROL (Remedy, Northlight, 2019). Chosen for two reasons and
> both are load-bearing: it is the game whose interface design is inseparable
> from its systems, and it is the game that made ray-traced reflections
> mainstream. This phase is a UI phase with a rendering half, in that order.
>
> **Status:** **COMPLETE — CONTROL-A through CONTROL-O in tree 2026-08-23.**
> Every track is finished, including the two §9 called a reasonable stopping
> point and the one §8 marked an explicit stretch. What is **owed** is the
> visual and timing evidence for Track 2 and Track 3: captures and `.somtime`
> rows need a windowed GPU run, and each sub-phase's record says exactly
> which ones. The cloud pass, the weather driver and decals therefore ship
> **default off**, which is the arrangement §12 asks for until the numbers
> exist. CONTROL-A's
> regenerable audit, red gates, two-width surface captures, and terrain timing are in tree;
> CONTROL-A1's input seam and CONTROL-A2's command registry are complete.
> CONTROL-B's schema-generated production Details path covers all 160 editable
> fields across 16 schemas; its legacy hand-wiring census is zero. CONTROL-D
> ships native `.sommat` authoring, generated material Details, live shared
> sphere previews, authored material references, worker texture resolution,
> glTF sibling materialization, Make Unique and vector assignment. **Next:
> CONTROL-E.** Plan written 2026-08-17;
> **re-audited and expanded 2026-08-22** against the
> tree at `209fd07`, after Phase 27 landed. Every count in §4 was re-measured
> on that date; the 2026-08-17 figures are superseded and the differences are
> stated rather than quietly corrected.
>
> **Predecessor:** Phase 26 (Metaphor) + 26-Zeta (Nocturne Atelier) built the
> *information architecture* and the *token layer*. **Phase 27 (Hades) rebuilt
> the paint layer and most of the first-impression surfaces** — 27-A through
> 27-G are in tree as of 2026-08-18. This phase builds the *reach*: what the
> editor can actually touch, and how it feels to touch it. Do not restart at
> 26-A. Do not re-theme anything Zeta certified. Do not repaint anything Hades
> painted. **Read §4.7 before writing a line, because Phase 27 already did
> several things this plan originally listed as missing.**
>
> **Record:** this file. Evidence folder `dev records/phase CONTROL/` is created
> by **CONTROL-A**, not before. Do not invent PNGs.
>
> **Successor, decided 2026-08-23:** **this phase runs first and completes in
> full; [Phase MORROWIND](phase_MORROWIND.md) (NetImmerse) is next.** Six of
> MORROWIND's eight tracks consume the seams this phase builds, and **CONTROL-K's
> curve and gradient editors are a hard dependency of three MORROWIND
> sub-phases** — so the "reasonable stopping point after J" in §9 is a pause, not
> an exit. §9.1 states what that means for how this phase is executed, including
> the tiebreak to use when a seam decision here is otherwise balanced.
>
> **Do not copy source** from Fyrox, Flax, Godot, O3DE, Unreal, Stride, Defold,
> rbfx, NeoAxis, Overload, Wicked, Esoterica or Remedy. Patterns only, cited in
> `ATTRIBUTION.md` §13G. **Note the letter: §13E and §13F are already taken by
> Phase 27**, so CONTROL's section is **G**, not E as the 2026-08-17 draft said.
> The section is opened by this plan's own reconnaissance (the files §6 cites
> were read on 2026-08-22) and expanded by CONTROL-A.

**Frozen by this phase** (a CONTROL sub-phase that changes any of these has gone
wrong): the Nocturne token sheet and its certified contrast pairs (Zeta §8A.3);
the Hades paint contract (phase 27 §6 — the primitive-quad instance layout, the
colour contract, `draw_over` ordering, the block-origin text snapping rule); the
five bundled type cuts and the `FontRole`/`TextRole` split; the 68 px pre-scene
budget and the floating context bar; the XV 32-layer terrain contract and
`GpuTerrainMaterial` layout; Great Lakes water numbers; foliage LOD and cull
distances; DOOM's measured defaults (dynamic resolution opt-in, tile binning and
aerial terrain default off, hex/POM default off); rustc 1.88; wgpu 29.

**The rule this phase is judged by, stated once — the reachability rule:**

> A feature that the engine can do and the editor cannot reach is not shipped,
> it is hidden. Every sub-phase below closes with a named list of knobs that
> moved from "environment variable, recompile, or nothing at all" to "a control
> in the editor with a label, a range, a unit, a tooltip, an undo step and a
> Help line." A sub-phase that adds a *renderer* capability without adding its
> authoring surface in the same sub-phase has failed, however good the pixels
> look.

**And its sibling, added by the 2026-08-22 expansion — the craft rule:**

> A control that exists but behaves worse than the equivalent control in Godot,
> Unreal, Unity or Blender is a control this phase has not finished. §5.2 lists
> the eleven places where that is true today, each against a named, verified
> convention in a shipping tool. Closing them is not polish appended to the end
> of the phase; each is assigned to the sub-phase that touches its widget.

---

## 0. How to use this document (handoff)

This file is long because the phase is large and because a later session must be
able to start cold. Read in this order before writing code:

1. **This file**, all of it. Especially §4 (the measured audit that motivates
   the phase), §5.2 (the craft defects, which are the "make it feel
   professional" half of the ask), §7 (the six seams — do not relitigate them),
   §8 (sub-phases), §10 (must-not-break), §15 (start checklist).
2. [`phase_27.md`](phase_27.md) §6 (the render contract), §12 (must-not-break),
   §18 (the implementation ledger — what actually landed, in what order, and the
   three defects that only a human looking at a screenshot caught).
3. [`phase_26_Zeta.md`](phase_26_Zeta.md) §5 identity, §6 token architecture,
   §8A.2 redlines, §8A.3 certified contrast pairs, §8A.4 the four-cue state
   grammar.
4. [`phase_26.md`](phase_26.md) §3 constraints, §14 must-not-break.
5. [`context.md`](../context.md) §8 (`somnium_ui`), §16, §17.6, §17.18–17.20,
   §18, and the `NEXT:` line at the top — which this phase closes.
6. `crates/somnium_ecs/src/reflect.rs` and
   `crates/somnium_core/src/reflect_registry.rs`, **end to end**. The phase is
   unreadable without them.
7. `crates/somnium_ui/src/editor/inspector.rs`, `crates/somnium_ui/src/editor_event.rs`,
   and `InspectorHandles` at `crates/somnium_ui/src/lib.rs:131`. These are the
   three things CONTROL-B shrinks, and the third is the one nobody has counted.
8. **[Appendix A](#appendix-a--implementation-reference)**, at the end of this
   file — the implementation layer, added 2026-08-23 for exactly the case where
   a different session or model picks this up cold. A.1 is a code-reading order
   over the real tree with line counts (§0 above is the *context* order; A.1 is
   the *code* order and they are different lists). **A.2 walks one field —
   `WaterComponent::roughness` — all the way from its `component_schema!`
   declaration through the generated panel, the single `EditorEvent`, the one
   `app.rs` handler, the scope-aware undo entry, and out to disk**, which is the
   fastest way to understand Seam 1. A.3 populates the property-editor registry;
   A.4 shows `EditingRules` as real Rust; A.5 is a file-by-file change map;
   **A.6 is two forward-compatibility notes for Phase MORROWIND that cost
   nothing now and a rewrite later**; A.7 is how to tell a sub-phase is genuinely
   finished rather than plausibly finished.
   **If you are starting cold, read §1, §4, §7, then A.1 and A.2, and only then
   §8.**

**Authorized work:** the editor surface and the seams beneath it —
`somnium_ui`, the `EditorEvent` boundary (which this phase *does* change, unlike
Phase 27), `somnium_core`'s editor commands and reflection registry,
`somnium_asset`, and the renderer additions Track 3 names. Component schemas are
authorized everywhere.

**Not authorized:** retuning the look of anything the renderer already draws
(§3), forking the token sheet, or adding a second reflection system.

**Update `context.md` and `ATTRIBUTION.md` after every completed sub-task**, per
`user_profile.md`. `ATTRIBUTION.md` §13G is opened by this plan's reconnaissance,
expanded by CONTROL-A, and gains
file-level citations from every sub-phase before that sub-phase closes.

---

## 1. Executive decision

Somnium's problem is no longer that the engine is thin, and after Phase 27 it is
no longer that the editor is ugly. It is that **the editor cannot reach the
engine**, and that the controls it does have are, one by one, a notch below what
a person arriving from Godot or Unreal expects a control to do.

Two numbers frame the phase. Both were measured on 2026-08-22 against `209fd07`.

**The reach number.** The property surface of this editor is maintained by hand,
and the hand-maintenance costs **675 identifiers**:

| Where the property surface is hand-written | Count |
|---|---:|
| `InspectorField` variants (`editor_event.rs:44–210`) | **106** |
| `ColorField` variants | **9** |
| `PostFxToggle` variants | **27** |
| `InspectorHandles` fields (`lib.rs:131–376`, 245 lines) | **226** |
| `field_bindings` rows pairing a handle to an `InspectorField` (`lib.rs:3922`) | **106** |
| `IF::` occurrences in `app.rs`'s write path | **201** |
| **Total hand-maintained call sites for "the Details panel"** | **675** |

Adding one number to the inspector costs, today, five edits across three crates:
a variant in `editor_event.rs`, a handle in `InspectorHandles`, a row in
`field_bindings`, a builder call in `editor/inspector.rs`, and at least one arm
in `app.rs`. That is why the table in §4.1 looks the way it does.

**The gap number.** Against that, the engine exposes **97** distinct `SOMNIUM_*`
environment variables and has **12** registered component schemas driving
**zero** generated inspector rows.

| Measured in the tree, 2026-08-22 | Count |
|---|---:|
| Distinct `SOMNIUM_*` environment variables | **97** |
| …with a control anywhere in the editor | **~18** |
| ECS component types in `somnium_core` | **11** |
| …with a `component_schema!` registration | **12** schemas, covering 11 components + `Parent`/`Name`/`MeshKind` |
| …whose inspector rows are generated from that schema | **0** |
| `PostProcessComponent` fields | **44** |
| …reachable from a schema | **0** (it has none) |
| Editable properties of a material, from the editor | **1** (base colour) |
| Hand-written `EditorCommand` types | **13** |
| …that are generic over a property | **1** (`SetScriptPropertyCmd`) |
| Command-palette entries | **15**, dispatched by array index |
| Editor preference files | **0** |
| Persisted editor settings | **4 floats** (`layout_persist.rs`) |

You can build a coastal landscape with 32 painted terrain layers, a spectral FFT
ocean with ray-traced reflections, ReSTIR GI and a scripted first-person
character — and you cannot change a mesh's roughness, drag anything anywhere,
select two things at once, or reopen the scene you just saved.

**Phase CONTROL's thesis, unchanged and now better evidenced: the editor's
problem is not missing panels, it is missing seams.** Phase 16-A already solved
the hardest one and nobody noticed.
`crates/somnium_core/src/reflect_registry.rs` opens with:

> *"Adding a component to the engine means adding one `component_schema!` block
> here. Nothing else needs editing to make it saveable, inspectable and
> script-visible."*

That promise is kept for the scene serializer (`scene_schema.rs` reads
`FieldFlags::SERIALIZE`), kept for the script bridge (`script_bridge.rs` and
`script_decls.rs` read `SCRIPT_READ` / `SCRIPT_WRITE`), and **broken for the
editor**. `FieldFlags::EDIT` is defined at `somnium_ecs/src/reflect.rs:318`,
documented as *"Shown in the inspector"*, and — re-verified on 2026-08-22 —
**referenced by zero lines of code outside its own definition.** Meanwhile
`somnium_ui` already depends on `somnium_ecs`, so the seam needs no new
dependency edge. It needs a consumer.

The proof that the design works is already shipping, in the one place it was
used. §17.19 records that every declared field of a Luau script — `walkSpeed`,
`jumpSpeed`, `mouseSensitivity`, the invert flags — *"appears in the Details
panel automatically, because the panel is generated from the schema."* And
`editor_commands.rs` has exactly one generic command out of thirteen:
`SetScriptPropertyCmd`. **A Luau script written by a user gets a better
inspector, and a better undo story, than `PostProcessComponent` does with its 44
fields and 27 hand-written toggle variants.**

So the order of this phase is not negotiable: **the seams first, the surfaces
after.** A material editor built before the property seam is 800 more lines of
`editor/inspector.rs`, 20 more `InspectorField` variants and 20 more
`InspectorHandles` fields. Built after it, it is a schema block and a
texture-slot editor.

**What the 2026-08-22 expansion adds to that thesis.** Three things the original
plan did not contain, all of them found by measuring rather than recollecting:

1. **There are six seams, not four.** Modifier keys never reach a widget
   (`WidgetMessage::MouseDown` carries `pos` and `button` and nothing else), and
   the editor's actions live in four unconnected hand-written lists. Drag and
   drop, multi-select and snapping are all blocked on the first; preferences,
   keybindings and a useful palette are all blocked on the second. Both are
   cheap. Both are named as seams in §7 so that no sub-phase discovers them the
   hard way.
2. **The thumbnail budget is wrong by two orders of magnitude, and it is
   measurable today.** `thumbnail.rs` states that "a 4096² source downscales in
   single-digit milliseconds" and decodes two assets per frame on the UI thread.
   `assets/terrain/` holds 60 PNGs totalling 1.17 GB, most of them 4096². The
   zlib inflate *alone* — a strict lower bound on decode — measures **232 ms and
   260 ms** on the two largest. Opening that one folder is a multi-second freeze
   with the code that is in tree right now. §4.2 has the numbers; CONTROL-C has
   the fix, and the fix is Fyrox's, which the current comment explicitly
   rejected.
3. **"Professional" is a measurable claim and this phase makes it.** §5.2 lists
   eleven specific behaviours where a Somnium control does less than the
   equivalent control in a shipping tool, each against a convention read out of
   that tool's source or its documentation. They are assigned to sub-phases, not
   deferred to a polish pass.

The rendering half of the phase exists to prove the rule rather than to decorate
it. Volumetric clouds, a time-of-day driver and weather are chosen because they
are (a) genuinely absent — `cloud`, `time_of_day`, `puddle` and `lightning`
return **zero** hits across `crates/**/*.rs` and `crates/**/*.wgsl`, and
`weather` and `decal` return one hit each, both in comments — (b) not claimed by
any existing phase, and (c) useless without authoring surfaces, which makes them
the honest test of whether the seams work. If clouds ship with a weather-map
painter and a preset list built from a schema block, the thesis held. If they
ship with three more `SOMNIUM_*` variables, it did not.

---

## 2. Goals

1. **`FieldFlags::EDIT` becomes true.** The Details panel is generated from
   `ComponentSchema` for every registered component, and every engine component
   is registered. Adding a component to Somnium adds its inspector for free, and
   the 675 hand-maintained call sites of §1 collapse to a schema block per
   component.
2. **Every widget the editor already has behaves the way its counterpart in a
   shipping tool behaves.** The eleven defects in §5.2 are closed, each in the
   sub-phase that owns its widget, and each with a test or a capture.
3. **A material is an authored asset.** Create it, name it, edit every property
   `GpuMaterial` actually carries, assign it by dragging, see it as a rendered
   sphere, and save it in a scene that reloads.
4. **The Content Drawer shows what things are, without stalling.** Real
   thumbnails, generated off the UI thread, capped by generated-per-frame rather
   than requested-per-frame, cached on disk by content hash, with the terrain
   folder — the worst case that exists in this repository — as the acceptance
   test.
5. **Direct manipulation exists.** Drag from drawer to viewport, drawer to
   entity, outliner to outliner, OS to drawer. Every drop is exactly one undo
   step, and a cancelled drag leaves nothing behind.
6. **Modifiers reach widgets.** `Ctrl`, `Shift` and `Alt` are carried on the
   input message, so add-to-selection, range-select, precision scrub and
   snap-invert are expressible at all.
7. **Every editor action is one registered command.** Menus, the toolbar, the
   palette, context menus, keybindings and the Help index are six views of one
   registry, not six hand-written lists.
8. **The 97 environment variables become ~97 settings**, searchable, persisted,
   revertable, layered, with the env var demoted to an override that says so in
   the UI.
9. **A scene saved from the editor opens from the editor.** The `NEXT:` line at
   the top of `context.md` is closed inside this phase, not around it.
10. **Clouds, a day cycle and weather exist, are authored from the editor, and
    are measured.** Default states are decided by the profiler, not by taste.
11. **No regression in the Zeta redlines or the Hades paint contract.** Same
    tokens, same contrast pairs, same 68 px, same focus order, same primitive
    pipeline. New surfaces join the grammar; they do not fork it.

---

## 3. Non-goals

- **A node-graph material editor.** Flax's `MaterialSurface` and Unreal's
  Material Editor are Visject/BP graphs over a shader compiler. Somnium's
  `GpuMaterial` is 48 fixed bytes consumed by one `shading.wgsl`; a graph would
  author nothing it can express. Parameter materials with texture slots, and a
  separate future phase if a shader-graph target ever appears.
- **A full docking platform.** Zeta's "Out of scope" already refused arbitrary
  multi-window docking, and named workspaces (`workspace.rs`, seven of them)
  cover the actual need. Godot 4.6 unified its docks and made them floatable;
  Somnium does not have to, and Godot's own release notes admit not every dock
  survived the transition to both orientations.
- **Replacing `somnium_ui`.** Phase 27 §3 evaluated Tauri v2, GPUI, Slint and
  Iced/libcosmic, rejected all four, and recorded why. That decision is closed;
  see phase 27 §3.6 for what would reopen it.
- **Prefabs.** Phase 34 owns nested instancing and overrides. This phase ships
  copy/paste/duplicate-with-hierarchy, which is QoL, and stops there. If a
  CONTROL sub-phase starts growing override propagation, it has drifted.
- **Skeletal animation, navmesh, networking, cooking.** Phases 27-anim, 30, 32,
  28 respectively.
- **Text shaping (`cosmic-text`).** Zeta's and Hades' shared open item. Large,
  orthogonal, and it blocks nothing in §8. It stays open; §14 says so explicitly
  rather than quietly folding it in.
- **AccessKit.** Same treatment, with one change from the 2026-08-17 plan: §14
  now records *why* it is large, and what Godot's experience cost, so the next
  session sizing it has a number instead of an intuition.
- **A visual scripting graph.** O3DE `ScriptCanvas` is a phase, not a sub-phase.
- **Turning clouds, weather or decals on by default before the profiler says
  so.** The engine is GPU-bound and shading-dominated (CR-A, DOOM-B). A 2 ms
  cloud pass on a 19.9 ms frame is a 10% tax and must be argued, not assumed.
- **Retuning the look.** Same clause as Phase DOOM §3 and Phase 27 §12: if a
  CONTROL stage changes what the renderer draws in an existing scene, it has
  failed, with the sole exceptions the phase declares in advance (clouds,
  weather and mesh wetness, all new and all default off).
- **A second reflection system.** Not `bevy_reflect`, not `serde` as the editor's
  description of a component. §7 Seam 1 states why.
---

## 4. The reachability audit, measured 2026-08-22

Assembled by reading and measuring the tree at `209fd07`, not by recollection.
CONTROL-A turns this section into a mechanically generated, checked-in table
that later sub-phases update; what follows is the measurement that justified the
phase, with the commands that produced each number so a later session can
reproduce them.

**Deltas from the 2026-08-17 draft of this file**, stated rather than silently
corrected: environment variables 96 → **97**; `EditorEvent` variants 48 → **58**;
ECS components "~20" → **11** real component types in `somnium_core` (the earlier
count swept in ECS test fixtures); Content Drawer thumbnails 0 → **images only,
delivered by Phase 27-G**. Everything else held.

### 4.1 The property surface

`crates/somnium_ui/src/editor/inspector.rs` is **839 lines** that hand-build
every row. Behind it:

| Artefact | Location | Size |
|---|---|---:|
| `CreateKind` | `editor_event.rs:3` | 13 variants |
| `InspectorField` | `editor_event.rs:44` | **106 variants**, 167 lines |
| `ColorField` | `editor_event.rs:214` | 9 variants |
| `PostFxToggle` | `editor_event.rs:228` | 27 variants |
| `ScriptFieldKind` | `editor_event.rs:297` | 3 variants |
| `EditorEvent` | `editor_event.rs:354` | 58 variants |
| `InspectorHandles` | `lib.rs:131` | **226 fields**, 245 lines |
| `field_bindings` | `lib.rs:3922` | 106 rows |
| `IF::` in the write path | `app.rs` | 201 occurrences |

`InspectorHandles` is the artefact nobody had counted, and it is the clearest
statement of the problem: **a 245-line struct whose sole content is 223 bare
`NodeHandle`s, two `[NodeHandle; 32]` arrays and one `Vec`** — one name per
widget the Details panel can contain, written out by hand, in a struct that must
be threaded through every function that touches the panel.

The cost of one new number in the inspector is five edits in three crates. That
is why the following components have no inspector at all, or a partial one:

| Component | Fields | Schema? | Inspector today |
|---|---:|---|---|
| `PostProcessComponent` | **44** | **no** | ~40 hand-written rows, 27 `PostFxToggle` variants |
| `ParticleEmitter` | — | **no** | two colour swatches; rate, lifetime, velocity, size unreachable |
| `BuoyantVessel` | — | **no** | six hand-written rows |
| `CameraSettingsComponent` | — | **no** | two rows (dynamic-resolution target and floor) |
| `RigidBodyComponent` | — | yes (`character.rs:125`) | **none** — velocity is script-only |
| `MaterialComponent` | — | yes | **one** colour swatch |
| `MeshComponent` | — | yes | none |
| `FoliageComponent` | — | yes | partial |
| `LightComponent` | — | yes | hand-written |
| `VoxelTerrainComponent` | — | yes | partial |
| `WaterComponent` | — | yes | 24 hand-written rows |
| `TerrainComponent` | — | yes | 9 hand-written rows + a **debug-view integer** |

The last one is the phase in miniature. `InspectorField::TerrainDebugView` is
documented as *"Debug visualisation code (same numbers as
`SOMNIUM_SHADOW_DEBUG`)"*. A user of this editor is expected to type `24` into a
numeric field to see the cluster heatmap. The views exist, the renderer is
finished, and the interface is a magic number.

The write side is worse than the read side. `editor_commands.rs` has thirteen
command types — `SetTransformCmd`, `SetNameCmd`, `SetLightCmd`,
`CreateEntityCmd`, `CreateLandscapeCmd`, `DeleteEntityCmd`, `ReparentCmd`,
`TerrainEditCmd`, `AttachScriptCmd`, `DetachScriptCmd`, `ReorderScriptCmd`,
`SetScriptEnabledCmd`, `SetScriptPropertyCmd` — and exactly one of them,
`SetScriptPropertyCmd`, is generic over a property. Everything else is a
bespoke undo record for a specific field. There is **no `SetFieldCmd`**, which
is why most of `PostProcessComponent` is not undoable at all.

> **Reproduce:**
> `awk` over `editor_event.rs` for variants per enum; `awk` over `lib.rs:131`
> for `InspectorHandles`; `grep -c 'IF::' crates/somnium_core/src/app.rs`.

### 4.2 The asset surface

**What Phase 27-G delivered, and what it did not.** `crates/somnium_ui/src/thumbnail.rs`
(448 lines) now owns a 1024² atlas of 64 px cells bound as texture id 2. Images
— `png`, `jpg`, `tga` and friends — are decoded and downscaled in-crate with the
`image` crate, aspect preserved on a transparent ground. Meshes, materials and
scenes are *requests*: `take_thumbnail_requests` / `deliver_thumbnail` /
`fail_thumbnail` is a narrow host API, and nothing answers it yet. Failures are
recorded so a corrupt file is never re-decoded. That split is correct and stays.

**What is wrong with it is the budget, and it is measurable today.**

`thumbnail.rs` sets `DECODE_BUDGET_PER_FRAME = 2`, decodes on the UI thread, and
justifies both in its module docs:

> *"A background thread would need the atlas behind a lock and would buy little:
> a 4096² source downscales in single-digit milliseconds, and the budget bounds
> the worst case at a predictable cost per frame rather than an unpredictable
> stall when a folder of 200 textures is opened."*

The premise is false for this repository. Measured on 2026-08-22:

| | |
|---|---|
| `assets/` total | **1.8 GB**, 305 files |
| `assets/terrain/` | **60 PNGs, 1.17 GB** |
| Largest single file | `leafy_grass_surface.png`, **53.3 MB**, 4096×4096 RGBA8 |
| Composition | 141 `.jpg`, 74 `.png`, 64 `.bc7`, 5 `.json`, 4 `.luau`, 4 `.gltf`, 2 `.somnium`, 2 `.glb` |

And the decode cost, measured as **zlib inflate only** — a strict lower bound,
since PNG unfiltering over the same `W×H×4` bytes and then the downscale both
sit on top of it:

| File | Dimensions | Inflate alone | Raw output |
|---|---|---:|---:|
| `aerial_grass_rock_surface.png` | 4096² | **232 ms** | 67 MB |
| `leafy_grass_surface.png` | 4096² | **260 ms** | 67 MB |
| `aerial_beach_01_albedo.png` | 2048² | **54 ms** | 17 MB |

Two decodes per frame of 4096² terrain textures is therefore **≥ 500 ms per
frame**, sustained across the ~30 frames it takes to drain a 60-file folder.
Opening `assets/terrain/` in the shipped editor is a multi-second freeze, and
the atlas holds 256 cells so the whole folder fits — nothing throttles it.

Three consequences for the plan:

1. **The original acceptance test was the wrong test.** "A drawer with 2 000
   assets opens without a hitch" measures file count. This tree is 305 files
   where one file is 53 MB. **The acceptance test becomes `assets/terrain/`**,
   the worst case that actually exists here, measured with the `.somtime`
   harness.
2. **The thread argument was decided backwards, and Fyrox already shows why.**
   `editor/src/asset/preview/cache.rs` puts the *loading* on a spawned thread
   behind an `mpsc::Receiver` and keeps only *generation* — the part that needs
   the engine and the GPU — on the main thread. The atlas never needs a lock,
   because the atlas write is the part that stays on the main thread. Somnium's
   comment rejected a design nobody was proposing.
3. **The budget counts the wrong thing.** Fyrox's `throughput` counter increments
   only when a preview is genuinely *generated*; cache hits drain for free.
   Somnium's `DECODE_BUDGET_PER_FRAME` bounds work done, which is right in
   principle, but it bounds *two decodes* rather than *a millisecond budget*, so
   its worst case is unbounded in time.

**Everything else about the asset surface is still absent:**

- `metaphor::list_content` (`metaphor.rs:133`) does a synchronous `read_dir`
  per call, emits an icon and a name, and carries no metadata, no id, no size,
  no dimensions, no hash.
- `refresh_content_list` (`lib.rs:2697`) calls it and then `clear_children` on
  the whole grid, rebuilding every tile widget. It is invoked from **12** call
  sites, including the filter box — so every keystroke in the search field is a
  full directory read plus a full widget-tree rebuild.
- There is no `AssetDb`, no `AssetId`, no content hash, no filesystem watch. A
  texture edited in an external tool does not update.
- There is no material asset. Materials exist only as `GpuMaterial` rows
  uploaded from a glTF import at startup; nothing names them, nothing saves
  them, nothing can create one.
- `FieldType::Asset` exists in the reflection enum (`reflect.rs:222`) with no
  editor behind it.
- Double-click does three things: folders navigate, `.somnium` fires `LoadScene`
  (see §4.6 — it does not work), `.luau` attaches. Every `.glb`, `.png`,
  `.ktx2` and `.hdr` in the tree is inert.
- 101 MB of foliage is re-parsed on every run (§17.6, Phase 28's premise). Not
  this phase's job to cook it, but it *is* this phase's job not to make it worse.

### 4.3 Interaction, and the seam underneath it

**No drag and drop of any kind.** 26-D2 has been open since Metaphor. The token
layer is ready and unused: `style.rs:333` defines `drop_target(valid: bool) -> Paint`
with a certified hue and a 2 px border, and `style.rs:449` tests it. Nothing
constructs a drag.

**No multi-select.** `selected_entity: Option<Entity>` appears in **71** places
and is singular through `EngineContext` (`context.rs:104`), the outliner, the
gizmo, the outline pass, the inspector and the script bridge.

**No snapping.** Translate, rotate and scale are continuous; there is no grid,
no angle increment, no snap-to-surface.

**No clipboard.** `grep -ri clipboard crates/somnium_ui/src` returns **zero**
hits. Duplicate exists; copy and paste do not.

**No camera focus, no bookmarks, no orbit-around-selection, no view presets.**

**No undo history view.** `UndoStack::new(128)` and no way to see it.

**No arrow-key traversal and no scroll-into-view.** `KeyCode::Up` and
`KeyCode::Down` return zero hits in `somnium_ui`; so does `scroll_into_view`.
`Tab` moves between shell regions and the design expects arrows to move within
them, which is written down in `lib.rs` and not implemented. Without
scroll-into-view, a focused row below the fold cannot be brought into view, and
a search result cannot be revealed.

**And the reason several of these are not merely unimplemented but currently
*inexpressible*:**

```rust
pub enum WidgetMessage {
    MouseDown { pos: Vec2, button: MouseButton },
    MouseUp   { pos: Vec2, button: MouseButton },
    MouseMove { pos: Vec2 },
    MouseWheel { pos: Vec2, delta: f32 },
    KeyDown(KeyCode),
    ...
}
```

**No message carries modifier state.** `Ctrl` and `Shift` are tracked as ambient
fields on `UiManager` (`lib.rs:1482–1484`, set from `WindowEvent::ModifiersChanged`)
and read only by the shell's own shortcut match. A widget cannot ask whether
`Shift` is down. So `Ctrl`+click to add to a selection, `Shift`+click to extend a
range, `Shift` to scrub finely, `Ctrl` to snap while scrubbing and `Alt`-drag to
spawn a decal are not features waiting to be written — they are features the
message type cannot express. This is Seam 5 (§7), and it must land before
CONTROL-E, F or G.

Diagnostics carry `file:line:column` and the Output Log renders them as text
(§17.18.6). Godot shipped click-to-open in 4.6, opening the external editor at
the offending line; Somnium has the data and not the affordance.

### 4.4 Command surfaces — four unconnected lists

This is the second finding the original plan did not contain, and it is the one
that makes CONTROL-H and CONTROL-I bigger than they look.

The editor's actions are declared four times, in four incompatible shapes:

| Surface | Where | Shape | Count |
|---|---|---|---:|
| Application menus | `editor/shell.rs:197–302` | hand-built widget trees | 6 menus |
| Command palette | `editor/shell.rs:54–80` | `Vec<PaletteItem>`, **dispatched by array index** | **15** |
| Keybindings | `lib.rs:1504–1540` | a hard-coded `match` on `KeyCode` | 6 |
| Content-drawer context menu | `lib.rs:1871` | `content_menu_id::*` integer ids | ~6 |

The palette's own doc comment records the fragility:

> *"Order is load-bearing: `UiManager::run_palette_command` dispatches on
> position, so inserting or reordering an entry here silently rebinds every
> command after it. Append only, and update `UiManager::STATIC_PALETTE_COMMANDS`
> when you do."*

`STATIC_PALETTE_COMMANDS = 15` (`lib.rs:2342`), asserted with a `debug_assert_eq!`.
Fifteen commands is not a command palette; it is a shortlist. Godot's palette
indexes every registered editor shortcut plus every `EditorScript`; Unreal's
`FUICommandList` is the substrate for menus, toolbars, context menus and
keybindings alike.

There is no way to answer "what can this editor do?" from data, which is why
there is no keybinding editor, no discoverable action list, and no Help index
that stays in sync with the toolbar. This is Seam 6 (§7).

### 4.5 Settings

**97** distinct environment variables. **Exactly 18** of them have an editor
control, and that number is now mechanically derivable rather than estimated:
the 18 `PostFxToggle` variants whose name maps onto a `SOMNIUM_*` variable —
`ANALYTIC_GRAD`, `BLOOM`, `CAS`, `FSR`, `GTAO`, `LIGHT_SHAFTS`, `MESH_SDF`,
`MOTION_BLUR`, `PATH_TRACER`, `PROBES`, `RESTIR`, `RESTIR_GI`, `RT_REFLECT`,
`RT_REFRACT`, `SPECULAR_GI`, `TAA`, `VOLUMETRICS`, `WORLD_CACHE`. **Seventy-nine
have no control at all.**

Reading one requires knowing it exists, and the only index is the source. There
is **no preferences window, no keybinding editor, no project settings, no
recent-scenes list, no autosave** — a grep for `preferences`, `settings.toml`,
`editor.toml` or `project.toml` across `crates/**` returns nothing.
`layout_persist.rs` persists exactly four floats: `tools`, `viewport`, `details`
and `outliner` column widths.

### 4.6 The broken button

`context.md`'s `NEXT:` line, re-verified 2026-08-22 and still true.
`EditorEvent::LoadScene` (`app.rs:4595`) routes to `crate::load_map`, which
accepts version-2 map recipes only. `scene_schema::load_scene_schema`
(`scene_schema.rs:610`) already restores every registered component. So
`File > Save` writes a `scene.somnium` that `File > Open` and the Content
Drawer's own double-click both refuse, and the failure surfaces as
`warn!("LoadScene failed: {error}")` in the log.

This is not a missing feature; it is a control in the shipped UI that lies. It
is CONTROL-J and it is not optional.

### 4.7 What Phase 27 already did — do not redo it

The 2026-08-17 draft of this plan was written against the 26-Zeta baseline. Phase
27 (Hades) landed 27-A through 27-G on 2026-08-18 and moved several items off
this phase's list. **Read `phase_27.md` §18 before starting anything.** In
particular, the following are done and must not be rebuilt:

- **The paint layer.** One instanced primitive-quad pipeline with analytic SDF
  evaluation: radius, antialiasing, gradients, borders, real shadows, glow,
  inner shadow. All 18 widgets migrated. Any new widget this phase adds uses
  `push_paint` and the recipe layer, and adds no new pipeline.
- **`Control::draw_over`.** A post-children draw hook. Any container this phase
  writes that needs to paint above its content uses it; the ordering is pinned
  by `draw_over_paints_after_every_child`.
- **Text baseline correctness.** Block-origin snapping, not per-glyph. Do not
  reintroduce per-glyph rounding; there is a regression test that was verified
  against the bug.
- **Empty states** for Details, Outliner, Log and the Content Drawer, including
  the distinction between "empty" and "filtered to nothing".
- **Content Drawer type badges, density metrics and image thumbnails.**
- **Search Everywhere** and the viewport selection overlay.
- **`NumericField` unit suffixes** — `unit: &'static str`, rendered muted and
  right-aligned. Seam 1's `unit` metadata feeds this existing field rather than
  adding a new one.
- **`PropertyRow` grammar** — the label column, the 14 px modified gutter and
  the narrow-panel stacking rule, computed once from the redline. CONTROL-B's
  generated rows are `PropertyRow`s; they do not invent a second row grammar.
- **`ScrollViewer` content sizing that skips hidden children.** CONTROL-B and
  CONTROL-F both rely on this; it was fixed in 27's ninth pass and wiring
  panels before it would have reproduced the Details scroll bug.

**Still open from 27 and inherited here:** the project picker (deferred by
decision because it needed an `EditorEvent` addition that 27 forbade — this
phase *does* touch `EditorEvent`, so CONTROL-H may take it); the 27-D backdrop
blur (needs `COPY_SRC`, conditionally supported); 27-F's monogram, optical
ladder, `.ico` and splash; `cosmic-text`; 27-H interaction and accessibility
completion; 27-I harness and lints; 27-J sign-off. **CONTROL does not absorb
27-H/I/J.** If Phase 27 is to be closed, it is closed by Phase 27.

**And one process lesson from 27 that this phase inherits as a rule.** Two of
the three defects found in Phase 27 produced correct geometry, correct colour
and green tests, and were caught only by a human looking at a screenshot: the
scroll-edge fade drawn under its own content, and the per-glyph baseline
stagger. A third — a deleted `handle_routed_message` — passed 184 tests because
nothing exercised scroll input. **The capture sheet in §13 is not optional
polish; it is the only instrument that catches that class**, and every
sub-phase below produces captures before it closes.

---

## 5. What "professional" means, and the eleven places Somnium is not

The second half of the ask is that the editor's *existing* functionality feel
market-ready, not just that new functionality arrive. That is a real and
separate goal, and it needs a real and separate standard, because "make it feel
professional" is otherwise a taste argument that ends in another re-theme —
which §12 lists as a named risk.

The standard this phase uses: **for each behaviour, name a shipping tool, name
the convention, and cite where that convention is written down or where it was
read out of the source.** Then either match it or record why not.

### 5.1 The 2025–2026 market bar

Godot is the most useful reference here, because it is the only major engine
that has publicly reframed whole releases around editor UX rather than
rendering, and enumerated the friction it was fixing.

**Godot 4.5, September 2025** — <https://godotengine.org/releases/4.5/>
- **AccessKit screen-reader support** on `Control` nodes, shipped *experimental*,
  covering the Project Manager, standard controls and the inspector only, after
  roughly two years of work. This is the number §14 uses to size Somnium's own
  accessibility work.
- **Toggleable inspector sections** — a checkbox on the section header replaces
  an "enabled" boolean buried inside the group, so an enabled/disabled
  sub-object is readable while collapsed.
- Variant-typed export properties with a type selector that swaps the widget;
  inline colour swatches in the script editor; multi-node remote inspection;
  batch import settings; DPI-aware editor icons.

**Godot 4.6, January 2026 — "It's all about your flow"** — <https://godotengine.org/releases/4.6/>
This is the release to model, and several items map one-to-one onto sub-phases
below.
- **Clickable output paths.** Clicking a path in an error or warning opens the
  script *at the problem line*, honouring the external-editor setting. → CONTROL-I.
- **Selection and transform decoupled in the 3D viewport.** "Select" was renamed
  "Transform" and a separate select-only mode added, so picking can no longer
  accidentally drag geometry. → CONTROL-G.
- **Layer flags (collision/render) became draggable** rather than
  click-per-checkbox; array editors relaid out to use horizontal space;
  multi-node group assignment. → CONTROL-B, CONTROL-F.
- **Live previews inside the Quick Open dialog**; a tab menu listing every open
  scene and script; drag-hover over a tab switches view mid-drag. → CONTROL-C,
  CONTROL-E.
- Bottom panels became ordinary docks, draggable between sides and bottom and
  floatable as OS windows — **explicitly not adopted here** (§3), and Godot's own
  notes record that not every dock survived the transition to both orientations.
- A modern default theme: greyscale, reduced blue tint, justified as keeping
  attention on the viewport rather than fighting the user's art. Nocturne
  already made that choice; this is corroboration, not a change.

**Godot 4.7, June 2026** — <https://godotengine.org/releases/4.7/>
- **Property category copy/paste** — right-click a group header to copy or paste
  a whole group's values. → CONTROL-F's clipboard, extended to properties.
- **Searchable popup menus**, motivated explicitly by discoverability in menus
  with hundreds of entries. → CONTROL-A2's registry makes this free.
- The Asset Store moved onto background threads *specifically so browsing no
  longer hitches the editor* — the same lesson as §4.2.

**Godot 4.8 dev snapshot** — the internal fuzzy-search implementation was
promoted to a public API; tree headers became sticky when scrolling deep
hierarchies; click-and-drag across visibility flags bulk-toggles them. All three
are relevant to CONTROL-F's Outliner.

**Unreal Engine 5.6, June 2025** — the redesigned **Viewport Toolbar**, the first
structural change to that toolbar since the UE4 beta in 2013, and the docs state
four design properties worth taking verbatim
(<https://dev.epicgames.com/documentation/unreal-engine/viewport-toolbar>):
1. features in *consistent locations, ordered by logical category* — transforms,
   snapping, viewport modes — rather than by historical accretion;
2. consolidation of options previously scattered across separate dropdowns;
3. **overflow management** — quick-select elements condense into an overflow
   menu on smaller viewports;
4. extensibility through the `ToolsMenu` system, with the user choosing which
   tools appear.
   Epic also kept the old toolbar **toggleable**, which is the migration pattern
   to copy if Somnium ever restructures its own.

Two older Unreal features remain the bar for the Details panel and are adopted
here: the **Section Bar** (jump links across related property categories, to cut
scrolling) and per-type **Favorites** (pin frequently used properties to the
top). Both land in CONTROL-B. For bulk editing at scale Epic routes users to the
**Property Matrix** — a table of objects × properties, thousands of rows, cell
copy/paste, with a details tree bound to the selected rows. That is *not* adopted
(§14): CONTROL-F's multi-edit covers the actual need, and a matrix is a
sub-phase of its own.

**Unity 6** — the one idea worth stealing outright is the **Piercing menu**
(`Ctrl`+right-click in the Scene view), which lists *every* selectable object
under the cursor. It is the canonical fix for picking in dense scenes, it is
cheap, and Somnium's viewport has exactly that problem in foliage. → CONTROL-G.

**Blender 4.x** — the **Asset Shelf**: an asset browser as an in-context strip
inside the working editor, filtered to the catalogs relevant to the current
mode, rather than a separate window you switch to. Somnium's seven named
workspaces are the same instinct; CONTROL-C's type-filter chips are the cheap
version of it.

**A correction to the 2026-08-17 draft.** That draft cited "per-project setting
overrides" as a Godot 4.6 feature to copy. **That is wrong.** Per-project
*editor* setting overrides are an open Godot proposal
(godotengine/godot-proposals#1480) plus a third-party tool, Godot Launcher, which
isolates `editor_settings-<major>.<minor>.tres` per project. The need is real and
unmet upstream — which is a better argument for building it in CONTROL-H than
"Godot has it" would have been — but it must not be cited as shipped behaviour.

### 5.2 The eleven craft defects

Each of these is a control Somnium already has, behaving worse than its
counterpart in a named tool. Each is assigned to the sub-phase that owns the
widget, so none of them becomes a polish pass that never happens.

---

**C1 — The numeric field is missing six of the seven drag-scrub conventions.**
*Owner: CONTROL-B.*

Somnium's `NumericField` (`widgets/numeric_field.rs`) has a 3 px scrub
threshold, per-field `drag_step`, and — correctly — the live/commit split:
`ValueChanging` on every step, one `ValueChanged` at the end, so a 200-pixel drag
is one undo entry. **That last one it got right, and it should be recorded as
such**: it is exactly Godot's `deferred_drag_mode`.

Read out of `godot/editor/gui/editor_spin_slider.cpp:96–215`, here is what
`EditorSpinSlider` does that Somnium does not:

| Convention | Godot | Somnium |
|---|---|---|
| Threshold measured on **accumulated relative motion** scaled by the field's speed and DPI (`4 * speed * EDSCALE`) | yes | 3 raw pixels |
| Crossing the threshold **captures the pointer** (`MOUSE_MODE_CAPTURED`) so the drag is unbounded by the screen edge | yes | no |
| **`Shift` multiplies motion by 0.1** — fine precision | yes | no |
| **`Ctrl`/`Cmd` rounds the result** — snap | yes | no |
| **Right-click or `Esc` during the drag cancels**, restoring the pre-grab value | yes | no |
| On release the pointer is **warped back** to where the drag began | yes | no |
| A fine `step` must not make the drag crawl: `drag_step = max(step, default_float_step)` where the floor is an **editor setting** | yes | no |
| A press that never crosses the threshold falls through to text entry | yes | yes |

Blender's manual documents the same modifier assignment — `Ctrl` snaps to steps,
`Shift` is fine precision — plus two more worth considering: **`Ctrl`+wheel while
hovering** edits the value, and a **vertical multi-field drag** (press on the
first field, drag down across its siblings) sets X/Y/Z in one gesture. Blender
also draws the distinction this phase's Seam 1 needs: **dragging clamps to the
*soft* limit; typing may exceed the soft limit but never the hard one.**

Figma is the dissenting design and is worth naming because it is better in one
respect: the scrub hotspot is **the label, not the field**, so click-to-type is
never ambiguous, and vertical cursor position selects one of four speeds
(2×, 1×, ½, ¼) with an on-screen indicator and a changing cursor width. That is
self-labelling in a way a hidden `Shift` modifier is not.

**Decision for CONTROL-B:** adopt Godot's modifier assignment (`Shift` fine,
`Ctrl` snap) because it agrees with Blender and is what the target audience has
in its hands; adopt pointer capture rather than cursor warping, because capture
is what Figma does and warping is a documented source of multi-monitor bugs in
Blender; adopt the `max(step, floor)` rule with the floor as a preference; adopt
right-click and `Esc` cancel. Take Figma's label-hotspot **only** for the
`PropertyRow` label gutter, which already exists, and only if it does not fight
the modified-dot affordance in the same gutter.

**Blocked on Seam 5** — none of the modifier behaviours are expressible today.

---

**C2 — Every numeric value prints three decimal places.**
*Owner: CONTROL-B.*

`numeric_field.rs:126`: `format!("{:.3}", self.value)`. A scale of 1, a distance
of 12 metres, an ISO of 800 and an angle of 45° all render as `1.000`,
`12.000`, `800.000`, `45.000`. Phase 27-G added the `unit` suffix, which fixed
half of the problem — the reader now knows it is metres — and left the other
half. Fyrox's `FieldMetadata` (`fyrox-core/src/reflect/field.rs:63`) carries
`precision: Option<usize>` for exactly this. Seam 1 adds it; the schema declares
it; the field reads it.

---

**C3 — Mixed values in a multi-selection have no representation, because there
is no multi-selection.**
*Owner: CONTROL-F.*

When multi-select arrives, the convention is settled and worth following
precisely rather than inventing. Unity: matching values show the value,
differing values show a dash, and — the underrated part — **right-clicking the
property label lets you pick which object to inherit the value from**. Figma
shows `Mixed` and accepts a *relative* equation that applies per-object, which is
strictly better for transforms but costs an expression evaluator.

The structural lesson comes from Godot, and it is the one that decides
CONTROL-F's design. `editor/inspector/multi_node_edit.cpp` does **not** make the
inspector multi-aware. It builds a synthetic object, `MultiNodeEdit`, holding a
list of node paths, whose `_get_property_list` is the *intersection* of the
selected nodes' properties and whose setter fans out to all of them under one
undo entry. The inspector then inspects one object and knows nothing about
multi-selection.

The intersection rule is stricter than "same name": a property appears only if
name, type, class name, hint and hint string all match across every selected
node, tracked by a `uses` counter that must equal the selection size. Somnium's
equivalent: **same `StableId`, same `FieldId`, same `FieldType` including the
enum variant list.**

---

**C4 — Focus cannot be moved with the keyboard inside a panel, and focus cannot
scroll.**
*Owner: CONTROL-F (Outliner) and CONTROL-B (Details).*

`Tab` moves between shell regions; the design says arrows move within them; the
arrows are not implemented (`KeyCode::Up`/`KeyCode::Down`: zero hits). There is
no `scroll_into_view`, so even if focus moved, a focused row below the fold
would stay below the fold — and a search result cannot be revealed. WCAG 2.4.3
(Level A) is the floor here: sequential focus order must preserve meaning, focus
must move into new UI when it appears, and **must return to the invoking control
when that UI is dismissed**. WCAG 2.2 SC 2.4.11 adds a ≥ 3:1 contrast floor for
the focus indicator, which Zeta's focus glow already satisfies.

This is the cheap, non-AccessKit half of Zeta-H, and it is worth taking now
precisely because the expensive half is deferred.

---

**C5 — The Content Drawer rebuilds its entire widget tree on every keystroke.**
*Owner: CONTROL-C.*

`refresh_content_list` → `read_dir` → `clear_children` → rebuild. Twelve call
sites. On a 60-file folder that is 60 widget constructions per character typed,
and on a folder being previewed it also re-issues every thumbnail request. Godot
4.7 moved its Asset Store onto background threads for the same reason and said
so. The fix is an `AssetDb` that is queried, not a directory that is re-walked.

---

**C6 — The command palette is fifteen entries dispatched by array index.**
*Owner: CONTROL-A2.*

§4.4. A palette that cannot enumerate the editor's actions is a shortlist with
a search box on it. Godot's `EditorCommandPalette` keys commands by a path-like
string, ranks by fuzzy score plus recency, persists the history, and gets its
contents from `register_shortcuts_as_command()` — every registered shortcut is
automatically a command. `ED_SHORTCUT_AND_COMMAND(path, name, keycode, command)`
is one declaration that registers both.

---

**C7 — There is no way to cancel a gesture.**
*Owner: CONTROL-A1 and CONTROL-E.*

`Esc` closes the top overlay. It does not cancel a numeric scrub, a gizmo drag,
a marquee, or (when it exists) a drag-and-drop. Godot cancels a spin-slider grab
on right-click *or* `Esc` and restores the pre-grab value; Blender cancels a
transform on right-click. A modal-feeling gesture with no escape hatch is the
single most anxiety-producing thing in a direct-manipulation tool. `Esc`
precedence is already an ordered concept in `close_top_overlay`; gesture-cancel
goes at the top of that order.

---

**C8 — Nothing tells the user why a control is disabled or overridden.**
*Owner: CONTROL-H.*

When CONTROL-H lands and an environment variable overrides a setting, the
control must be disabled *and say why* — "overridden by `SOMNIUM_HEXTILE`" in the
tooltip. This is stated as an implementation detail in the original §8's
CONTROL-H, and it is listed here as a craft defect because the same rule applies
now, before preferences exist: several toggles in the Details panel are inert on
hardware that does not support the feature, and none of them explain themselves.

---

**C9 — Picking in a dense scene has no disambiguation.**
*Owner: CONTROL-G.*

Click-to-select picks the frontmost hit. In a foliage field or a stack of
overlapping props there is no way to reach the object behind. Unity 6's
Piercing menu (`Ctrl`+right-click → a list of every selectable object under the
cursor) is the fix, it is small, and the hit data already exists.

---

**C10 — Long operations are invisible.**
*Owner: CONTROL-C.*

glTF import, BC7 encode, terrain bake and thumbnail generation all block or run
silently. There is no job registry, no status-bar progress, no cancel. Flax's
`Progress/Handlers/*.cs` makes each a first-class cancellable job with a
status-bar presence; Godot 4.7's threading of the Asset Store is the same move.
Somnium's status bar has the space and the grammar for it already.

---

**C11 — The editor has no memory beyond four column widths.**
*Owner: CONTROL-H and CONTROL-J.*

No recent scenes, no last-opened folder in the drawer, no window size, no
workspace on restart, no autosave, no crash recovery. `layout_persist.rs`
persists `tools`, `viewport`, `details`, `outliner` and nothing else. Every
professional tool in §5.1 restores its state; a tool that forgets everything
between runs reads as a demo.

### 5.3 What must not be copied

Source, from any of them. Fyrox is MIT, Flax is BSD-like, Godot is MIT, Stride
is MIT, rbfx is MIT — all permit reuse, and **`ATTRIBUTION.md` §15 is stricter
than any of their licences on purpose.** Patterns, cited.
`ATTRIBUTION.md` §13G — **not §13E; that letter and §13F belong to Phase 27** —
is opened by this plan's reconnaissance, expanded by CONTROL-A with an entry per
reference in §6, and every sub-phase adds its file-level citations before it
closes.

Specifically not copied, even as patterns:

- **Godot's dock system.** §3.
- **Unreal's Property Matrix.** §14; the need is covered by CONTROL-F.
- **Any engine's node-graph material authoring.** §3.
- **Unity's `AssetPreview` polling model.** Its own documentation says the call
  *"might return null until the preview is ready"* and prescribes polling
  `IsLoadingAssetPreview`; the bounded cache that silently returns null on
  overflow is not even mentioned on the API page. Godot's
  callback-that-fires-even-on-failure is strictly better and is what CONTROL-C
  adopts.
- **Ultra Dynamic Sky's "Refresh Settings" button.** Its documentation requires
  pressing it to see curve edits. CONTROL-K/L's curves are live or they are not
  shipped.
---

## 6. Repository and literature audit

### 6.1 `example_repo` — the four primary references

These four were read directly for this plan on 2026-08-22, and the citations
below name the exact files. They are primary because Somnium's UI is a port of
one of them and its editor's problems are the ones the other three solved first.

**Fyrox** (`example_repo/fyrox/Fyrox-master`) remains the primary reference for
the same reason it was in Phase 12: `somnium_ui` is a port of its widget
architecture, so its editor's *shape* transfers without translation loss.

| Fyrox path | What it solves here | Sub-phase |
|---|---|---|
| `fyrox-core/src/reflect/field.rs:63` — `FieldMetadata` | `{ name, display_name, tag, doc, read_only, immutable_collection, min_value, max_value, step, precision }`, with `doc` populated by the derive macro **from the field's `///` comment**. This is the target shape for `FieldSchema`, and the doc-comment route is the one worth taking wholesale. | B |
| `fyrox-ui/src/inspector/editors/` — 27 files | One `PropertyEditorDefinition` per type behind a `PropertyEditorDefinitionContainer` keyed by `TypeId`, with `register_inheritable_{vec_collection, inspectable, enum, option}` helpers. The registry-of-editors-by-type pattern, and the *list* of types a mature inspector actually needs. | B |
| `editor/src/asset/preview/cache.rs` | `AssetPreviewCache`: loading on a spawned thread behind an `mpsc::Receiver`, *generation* on the main thread, `throughput = 4` counted only on generated previews, layered fallback (preview → grey-tinted kind icon → placeholder), keyed by resource UUID with a `force_update` flag. **This is the fix for §4.2**, and it is the design `thumbnail.rs` argued against without having measured it. | C |
| `editor/src/asset/selector.rs`, `item.rs` | `AssetSelectorMixin` + `AssetItem`: how a property field becomes an asset picker with search, and how one widget serves both the browser and the inspector. | C |
| `editor/src/plugins/material/editor.rs` | `MaterialFieldEditor`: text + preview image + **Edit / Locate / Make Unique**. "Make Unique" is the answer to shared-material editing and is taken wholesale as a concept. | D |
| `fyrox-ui/src/curve/` | Curve editor widget: keyframes, tangents, zoom/pan. | K |
| `editor/src/scene/clipboard.rs` | Copy/paste of a subgraph with handle remapping. | F |
| `editor/src/settings/keys.rs`, `move_mode.rs`, `rotate_mode.rs` | Keybinding storage and snap increments as *settings*, not constants. | G, H |
| `editor/src/interaction/gizmo/` | Move/rotate/scale as separate interaction objects rather than a mode integer. | G |

**Flax** (`example_repo/FlaxEngine-master/Source/Editor`) is the reference for
the surfaces Fyrox draws differently.

| Flax path | What it solves here | Sub-phase |
|---|---|---|
| `CustomEditors/` + `CustomEditors/Editors/*.cs` | Forty-odd `*Editor.cs` behind a `CustomEditorPresenter`, with `GenericEditor` as the reflecting fallback. Independent confirmation of Seam 1's design: a per-type editor table with a generic fallback, not a per-component panel. | B |
| `GUI/Drag/DragHelper.cs`, `DragHandlers.cs`, `DragAssets.cs`, `DragActors.cs`, `DragScripts.cs`, `DragNames.cs` | `DragHelper<T,U>` with a `ValidateFunction` supplied by the drop target, a **filtered** `List<T> Objects` so a drag can be partially valid, `DragHandlers : List<DragHelper>` for composition, and a `DragDropEffect` return rather than a bool. Seam 3 is rewritten around this; see §7. | E |
| `Progress/Handlers/*.cs` | `ImportAssetsProgress`, `BakeLightmapsProgress`, `CompileScriptsProgress`: long operations as first-class, cancellable, status-bar-visible jobs. Craft defect C10. | C |
| `Options/` (`EditorOptions`, `InputOptions`, `InputBinding`, `ViewportOptions`, `InterfaceOptions`) | The preferences model Seam 4 copies structurally: categories as types, bindings as data. | H |
| `History/` | Undo history as a viewable, navigable list. | J |
| `GUI/CurveEditor*.cs`, `IKeyframesEditor` | The other half of CONTROL-K, and the keyframe abstraction a future sequencer (Phase 36) shares. | K |

**Godot 4.7** (`example_repo/godot-4.7.1-stable/editor`) contributes the shape of
`file_system/`, `inspector/`, `docks/` and `settings/` as separated concerns,
plus four specific mechanisms read out of the source for this plan:

| Godot path | What it solves here | Sub-phase |
|---|---|---|
| `gui/editor_spin_slider.cpp:96–215` | The complete drag-scrub contract: relative-motion threshold scaled by speed and DPI, pointer capture on cross, `Shift`×0.1, `Ctrl` round, right-click/`Esc` cancel restoring the pre-grab value, pointer warped back on release, `drag_step = max(step, default_float_step)` with the floor as an editor setting, and `deferred_drag_mode` for one-signal-at-the-end. Craft defect C1. | A1, B |
| `inspector/editor_resource_preview.cpp` | Thread + semaphore + queue; disk cache `resthumb-<md5>.png` plus `_small.png` and a `.txt` sidecar; **two-stage invalidation** — mtime first, hash only if mtime differs, sidecar rewritten if the hash matches so a touched file does not regenerate; two sizes from one job; **the callback fires even on failure**, passing null. | C |
| `inspector/multi_node_edit.cpp:153` | Multi-select as a **synthetic object** whose property list is the intersection of the selection's, matched on name *and* type *and* hint *and* hint string with a `uses` counter equal to the selection size, and whose setter fans out under one undo entry. The inspector never learns about multi-selection. | F |
| `settings/editor_command_palette.h` | `HashMap<String, Command>` keyed by a path-like id; `ED_SHORTCUT_AND_COMMAND` registering a binding and a palette entry in one declaration; `register_shortcuts_as_command()`; fuzzy score plus persisted recency. Seam 6. | A2, H |

**Unreal** (`example_repo/UnrealEngine-release`) contributes the thumbnail
architecture and the interaction every user of this editor already has in their
hands:

| Unreal path | What it solves here | Sub-phase |
|---|---|---|
| `Editor/UnrealEd/Classes/ThumbnailRendering/ThumbnailRenderer.h` | `UThumbnailRenderer` virtuals — `CanVisualizeAsset`, `GetThumbnailSize`, `Draw`, `AllowsRealtimeThumbnails`, `GetThumbnailRenderFrequency` — and `enum class EThumbnailRenderFrequency { Realtime, OnPropertyChange, OnAssetSave, Once }`, whose own comment reads *"listed from most to least CPU demanding / frequent"*. Each asset kind declares how often its preview must be redone. | C, D |
| `ThumbnailRendering/ThumbnailManager.h` | `FThumbnailRenderingInfo` mapping class → renderer, lazy-bound **by class name** so the renderer module need not be loaded; and a set of shared preview primitives (`EditorCube`, `EditorSphere`, `EditorCylinder`, `EditorPlane`, `EditorSkySphere`) — the shared rig CONTROL-C and CONTROL-D need. | C, D |
| Content Browser drag-onto-object assignment | The gesture CONTROL-E must match, because every user of this editor has that muscle memory. | E |
| Details panel Section Bar and per-type Favorites | Jump links across property categories, and pinned frequently used properties. | B |

### 6.2 `example_repo` — the survey of the newly added engines

Fifteen engines were added to `example_repo` without ever being read for this
project. They were surveyed on 2026-08-22 specifically for editor architecture.
The full triage is below; **the four concepts worth carrying into Somnium are
called out first, because they change decisions in §7 and §8.**

#### 6.2.1 The four ideas that change this plan

**(a) rbfx's `AttributeScopeHint` — the schema declares undo granularity.**
`example_repo/rbfx-master/Source/Urho3D/Core/Attribute.h:66`. Every attribute
declares how far a change to it ripples: `Attribute`, `Serializable`, `Node`,
`Scene`. `CommonEditorActionBuilders.cpp` switches on it to choose the undo
record: a hint of `Attribute` stores a scalar diff, `Node` snapshots the node
subtree, `Scene` snapshots the scene.

This solves a bug Somnium is about to have. Seam 1's `SetFieldCmd` stores a
before/after `ReflectValue` — which is correct for `roughness` and **wrong for
`TerrainComponent::resolution`**, where the write rebuilds a heightfield, a
collider and a GPU sidecar. Today that case is safe only because
`TerrainEditCmd` is hand-written and snapshots. The moment CONTROL-B routes
every field through one generic command, a scalar-diff undo on a rebuilding
field corrupts the scene silently.

**Adopted into Seam 1** as `FieldSchema::scope: ChangeScope` with variants
`Field` (default), `Component`, `Entity`, `Scene`. `SetFieldCmd` picks its
undo strategy from it. This is a small addition and it is the difference
between CONTROL-B being safe and CONTROL-B being a scene-corruption bug.

**(b) Wicked Engine's thumbnail-in-the-file-header — deletes a subsystem.**
`example_repo/WickedEngine-master/WickedEngine/wiArchive.h/.cpp`. Wicked's
archive format is `Header | thumbnail JPEG | data`, with `thumbnail_data_size`
in the header bitfield. `Archive::SetThumbnailAndResetPos(texture)` encodes the
editor viewport on save; `Archive::PeekThumbnail(filename)` reads only
`sizeof(Header) + thumbnail_data_size` bytes. `ContentBrowserWindow.cpp:527`
populates tiles from that.

There is **no thumbnail cache, no invalidation and no throttling in Wicked, and
none is needed**, because the thumbnail cannot be stale: it is part of the file
it describes, written at the moment the file was written, using a frame the
editor had already rendered.

**Adopted for the two file kinds Somnium authors itself** — `.somnium` scenes
(CONTROL-J writes the viewport frame into the file on save) and `.sommat`
materials (CONTROL-D writes the preview sphere). Those two get free, never-stale,
zero-cost previews. `.glb`, `.png` and `.hdr` are third-party files Somnium does
not write, so they keep the Fyrox/Godot cache path. **This splits CONTROL-C's
work honestly: the cache only has to be good enough for files we did not
author.**

**(c) Esoterica's `TypeEditingRules` — conditional visibility in code, not in
the attribute DSL.** `example_repo/Esoterica-main/Code/EngineTools/PropertyGrid/PropertyGridTypeEditingRules.h`.
Esoterica's property metadata is exactly ten keys (`FriendlyName`,
`Description`, `Category`, `Hidden`, `ReadOnly`, `ShowAsStaticArray`, `Min`,
`Max`, `DisableTypePicker`, `CustomEditor`) and **conditional visibility is not
one of them**. Instead a `TypeEditingRules` subclass is registered per type via
`EE_PROPERTY_GRID_EDITING_RULES(EditedType, RulesClass)` and answers
`IsReadOnly(propertyID)` / `IsHidden(propertyID)` / `GetPropertyNameOverride(...)`
with a tri-state — `Editable` / `ReadOnly` / `Unhandled` — re-evaluated every
frame.

This is the right answer and it settles a question Seam 1 would otherwise hit
within two components. `PostProcessComponent` alone needs "`bloom_intensity` is
meaningless when `bloom_enabled` is false", "`dof_focus_distance` needs
`dof_enabled`", "the physical-camera triple is dead unless
`use_physical_camera`". Encoding that in `component_schema!` means inventing an
expression grammar in a declarative macro, which ends badly. **Adopted** as a
separate `EditingRules` registry keyed by `StableId`, with the same tri-state
return so "this rule has no opinion" is distinct from "editable".

Luanti's counter-example is worth recording beside it: its `settingtypes.txt`
does express conditionals, as a `Requires: setting_a, !setting_b` comment line —
and it works only because the grammar is restricted to conjunctions of boolean
settings. The moment a condition needs arithmetic, the text schema loses.

**(d) Esoterica confirms Fyrox on doc comments, from a different direction.**
Esoterica's reflector is a real libclang pass, not a macro expansion:
`Code/Applications/Reflector/TypeReflection/Clang/ClangVisitors_Structure.cpp`
calls `clang_Cursor_getBriefCommentText()` and **the field's doc comment becomes
its inspector tooltip**, with the `Description=` metadata key only as a
fallback. Two independent engines reached the same conclusion. Seam 1's `doc`
field is populated from `#[doc]` and there is no competing argument.

#### 6.2.2 The rest of the survey, and what each is good for

| Engine | Editor size | Reflection | Per-property metadata | The one thing |
|---|---|---|---|---|
| **Esoterica** | `Code/EngineTools/`, ~25 subsystems; PropertyGrid alone ~5,050 lines | libclang codegen, out-of-band | 10 keys + a rules class | doc-comment tooltips; `TypeEditingRules`; unknown metadata keys are **preserved, not dropped** |
| **rbfx** | `Source/Editor/`, 145 files | runtime, hand-registered macros | 4 metadata keys + mode flags + **scope hint** | `AttributeScopeHint`; `EditorAction::MergeWith` for drag coalescing; `UndoException` when the stack desyncs from editor state |
| **NeoAxis** | `NeoAxis.Core.Editor/`, **2,095 C# files** | .NET reflection + **runtime-attachable attributes** | ~15 attributes | the deepest preview pipeline here (below); `[UndoDependentProperty]`; `Reference<T>` |
| **Overload** | `Sources/OvEditor/`, **77 files** | none | call arguments | the calibration point for "minimum viable professional editor"; gatherer/provider lambda pairs |
| **Wicked** | `Editor/`, ~57k lines, ~40 hand-written `*Window.cpp` | none | none | thumbnail in the archive header |
| **Falco** | `Editor/`, 151 files / ~25.7k lines | Mono script fields only | none | wildcard drop-format strings (`"*."`, `"::Asset"`) |
| **Luanti** | no editor | text schema | `type_args` + `Requires:` | one schema, two consumers; **hard assert on an unknown type** |
| **Stride** | see §6.2.3 | — | — | — |
| **Defold** | see §6.2.3 | — | — | — |

Details worth keeping, by engine:

**NeoAxis `PreviewImagesManager.cs`** (716 lines) is the most complete thumbnail
pipeline surveyed and contributes three details CONTROL-C should take: up to ten
processors each owning an offscreen viewport; **render at 1024, auto-crop to the
actual content with a 13 px border, then downsample to 128** — so a thin mesh
does not become a speck in the middle of a tile; a submission queue capped at
100; and invalidation that *deletes* the cached PNG when the preview is not
currently loaded rather than re-queueing it. Cache path is
`<Project>/Caches/Files/<virtualPath>.preview.png`. NeoAxis also has
`[Range(min, max, ConvenientDistribution.Linear|Exponential, power)]` — the
distribution shapes the slider curve, which matters for the several Somnium
properties (light intensity, fog density, roughness) where linear is the wrong
feel. Noted for Seam 1 as `FieldSchema::curve: SliderCurve`, **deferred** to
CONTROL-B's second pass rather than the first.

**rbfx `HotkeyManager`** models a binding as a fluent builder —
`.Ctrl().Shift().Press(Key)`, with `MaybeCtrl()` for "don't care" — and renders
`ToString()` into menu labels. Seam 6's `Chord` takes that shape, and the
`ToString()` detail is the thing that keeps a menu's accelerator text from
drifting from the binding it names.

**rbfx `UndoException`** is worth naming as a rule: undo failing because the
stack desynchronized from editor state is a *typed error that surfaces*, not a
silent partial apply. CONTROL-J's history panel needs somewhere to show it.

**Overload's `GUIDrawer`** offers every property helper in two forms: a
by-reference form, and a **gatherer/provider `std::function` pair**. The second
is how a property with a side-effecting setter is edited without the UI knowing.
Somnium's `ComponentSchema` already carries getter/setter functions, so this is
mostly present — but it is the reason those functions must stay the *only* write
path, and never be bypassed by a direct field write in a generated editor.

**Falco's drop-format wildcards** (`"*."` = any node from the same tree,
`"::Asset"` = any node from the Assets tree) are a tiny grammar that saves
writing a type test per drop target. Seam 3's `DropAcceptance` covers the same
ground with types instead of strings, and types are better here — but the
*shape* of "a target declares a coarse class of acceptable sources" is worth
copying.

**Babylon.js** (`packages/dev/inspector-v2/`, `packages/dev/sharedUiComponents/src/fluent/hoc/propertyLines/`)
is the best available catalogue of *what rows a mature inspector needs* — 17
components including `syncedSliderPropertyLine`, `entitySelectorPropertyLine`,
`hexPropertyLine` and `vectorPropertyLine`. Its user-extension schema,
`IInspectable[]` with `{label, propertyName, type, min, max, step, callback,
fileCallback, options, accept}`, carries two things Esoterica and rbfx both lack:
a **`step`** and an **`accept`** file-extension filter on asset fields. Seam 1
takes both — `step` was already planned, and `accept` becomes
`FieldType::Asset(AssetKindMask)` so a texture slot cannot be handed a mesh.

**Korge** (`korge/src/commonMain/kotlin/korlibs/korge/view/property/ViewProperty.kt`)
is the closest existing analogue to a single-attribute inspector macro:
`@ViewProperty(min, max, clampMin, clampMax, decimalPlaces, groupName, order,
name, editable)`, plus `@ViewPropertyProvider` for dynamic option lists,
`@ViewPropertyFileRef(extensions)` for asset refs and `@ViewPropertySubTree` for
recursion. Two details confirm Seam 1's design from a third direction:
**`min`/`max` are separate from `clampMin`/`clampMax`** — exactly the
soft-limit/hard-limit split Blender documents and Seam 1 adopts — and there is
an explicit **`order`**, because declaration order is not always presentation
order.

**Panda3D** (`direct/src/leveleditor/`) contributes only a shape: a positional
property tuple `(PROP_TYPE, PROP_DATATYPE, PROP_FUNC, PROP_DEFAULT, PROP_RANGE,
PROP_DYNAMIC_KEY)` with UI hints as string constants. Worth ten minutes, no more.

**Not useful, recorded so nobody re-checks:** jMonkeyEngine (the SDK with the
scene composer is a separate repository and is absent here), Raylib (no editor;
`tools/rlparser/` is a header-to-JSON parser and nothing else), Ren'Py (the
launcher is written in the engine's own screen DSL — interesting as a
"tool written in the runtime" case study, irrelevant as an inspector
reference), Haxe (a compiler; no editor), mach (`editor/` is a project
generator: `init.zig`, `main.zig` and a template).

**Explicitly out of scope, unchanged:** uGUI and ebitenui are runtime-UI
references. Phase 26-G's `UiCanvas` covers the game HUD and nothing in this
phase touches it.

#### 6.2.3 Stride and Defold — the two most different answers

These two were surveyed in depth because they are the least like Unreal and
Godot, and because each solves one problem better than anything else in
`example_repo`.

**Stride** (`example_repo/stride-master/sources/`; note the lowercase `sources/`,
not `Source/`). The editor is WPF — `Stride.Assets.Presentation` 590 files,
`Stride.Core.Assets.Editor` 359, `Stride.Core.Presentation.Wpf` 263. The
Avalonia project is infrastructure only (dialogs, converters); there is no
Avalonia property grid, so do not read it expecting one.

Stride's contribution is **Quantum**, a four-layer pipeline that turns a plain
object graph into an editable UI tree, with each layer in its own assembly:

1. **Object graph** (`sources/presentation/Stride.Core.Quantum`).
   `NodeContainer` + `DefaultNodeBuilder` reflect an object into a persistent
   graph of `IObjectNode` / `IMemberNode`, each with a stable `Guid`, a type
   descriptor, and `ValueChanging`/`ValueChanged` events. Collections are
   addressed by `NodeIndex`; references between objects are first-class, so the
   graph mirrors the object topology rather than a tree. `GraphNodePath` gives
   any node a serialisable address, and `GraphNodeLinker` walks two graphs in
   parallel — which is what powers prefab/archetype base-linking.
2. **Presenters** (`Stride.Core.Presentation.Quantum/Presenters`). A presenter is
   a *mutable, decoratable* view of a node carrying `DisplayName`, `Order`,
   `IsVisible`, `IsReadOnly`, `Commands` and a typed `AttachedProperties` bag.
   After each presenter is created, the factory runs every registered
   **`INodePresenterUpdater`** over it.
3. **View models** (`.../Quantum/ViewModels`). `GraphViewModel` collapses *N*
   presenter trees into one, grouping by `CombineKey`.
4. **Templates** (`editor/Stride.Core.Assets.Editor/View`). ~30 built-in
   `NodeViewModelTemplateProvider`s, each with a `MatchNode` predicate,
   registered as XAML data.

**Three things in that pipeline are worth taking, and one is worth refusing.**

*Take: metadata is pushed, not read.* All property metadata arrives via
updaters. `NumericValueNodeUpdater` seeds min/max from the *CLR type*, forces
`DecimalPlaces = 0` for integral types, then overrides from `[DataMemberRange]`.
`DocumentationNodeUpdater` pulls tooltips from a documentation *service* backed
by XML doc comments, not from an attribute. The consequence: adding a new piece
of metadata — a unit, an expert flag — is one small registered class, and the
property grid does not change. **Somnium takes the light version of this**:
`component_schema!` remains the primary source, but the registry exposes a
`SchemaDecorator` hook so a later phase can attach editor-only metadata (a unit
table, a favourites list, a per-project override) without editing the macro or
the components. This is the escape valve that keeps Seam 1 from becoming a
macro-DSL arms race.

*Take: categories are structure, not a rendering hint.* `CategoryNodeUpdater`
does not tag a property with a category string for the renderer to group on — it
**creates a virtual category node and reparents the property into it**, ordered
by `[CategoryOrder]` on the declaring type. Seam 1's `group` field is therefore
consumed by building a real section node in the generated panel, which is what
lets a section fold, be favourited, be copied whole (Godot 4.7's group
copy/paste) and be reordered.

*Take: `IUnloadable` — and this one is a bug fix, not an improvement.* When a
Stride project's game assembly fails to load, instances deserialise as
`IUnloadable` proxies **that retain their raw YAML parsing events**, an updater
spots them, and a template renders the raw YAML in the inspector. **Data
survives a broken assembly round-trip.**

Somnium has the opposite behaviour today, and it is a silent data-loss path.
`scene_schema::scene_from_json` (`scene_schema.rs:499`) does:

```rust
let Some(schema) = registry.by_name(name) else {
    report.warnings.push(SceneWarning { .. "no component named `{name}` in this build; skipped" });
    continue;
};
```

and the same for an unknown *field* — `"has no field `{field_name}`; dropped"`.
So loading a scene authored by a build that had a component this build does not,
and then saving it, **destroys that component's data permanently**, with a
warning in a log nobody reads. The warning proves someone thought about it; the
`continue` is still the wrong verb. **CONTROL-J adopts Stride's answer**: unknown
components and unknown fields are retained verbatim as opaque JSON on the entity
and written back out on save. It costs a `HashMap<String, serde_json::Value>`
and it converts a data-loss bug into a forward-compatibility feature.

*Refuse: the four-layer assembly split.* Quantum is the right architecture for a
plugin ecosystem where third parties register updaters and templates. Somnium
has one editor and one team. The *ideas* transfer; the four assemblies do not.

Stride's other contributions, briefly:

- **Thumbnails are build-engine jobs**, pushed into the same
  `PriorityQueue<AssetBuildUnit>` as asset compilation and keyed by `AssetId`
  (not by `AssetItem`, so a rename does not duplicate the job). A re-request
  while a render is in flight stores a continuation rather than racing. Changing
  the rendering mode or colour space re-queues *every* thumbnail.
- **`TilePanelThumbnailPrioritizationBehavior`** hooks the scroll event on a
  virtualising tile panel, computes the visible index range, and calls
  `IncreaseThumbnailPriority` **only for on-screen assets.** This is the piece
  §4.2 was missing: the fix for `assets/terrain/` is not only "decode off the UI
  thread", it is "decode the eight tiles the user can see, and stop." Adopted by
  CONTROL-C, and it is what makes the budget question mostly moot.
- **`ObjectCache<ObjectId, ImageSource>(512)` keyed by content hash**, plus a
  `ComputingThumbnails` dictionary so two views awaiting the same preview share
  one decode task.
- **`IMergeableOperation`**: `ContentValueChangeOperation.CanMerge` is *same node
  + same index + same change type*, so a burst of edits to one property collapses
  to one undo entry at commit. Merging is the backstop; the *mechanism* is at the
  input layer — `NumericTextBoxDragBehavior` only validates on
  `Thumb.DragCompleted`, so the binding pushes once.
- **Every property set is a named transaction**: `$"Update property {DisplayPath}"`,
  which is what makes a history panel readable rather than a list of "Change".
- **The settings dialog *is* the property grid.** `SettingsPropertyNodeUpdater`
  turns `SettingsKey<T>` wrappers into nodes. One inspector engine, three
  surfaces: assets, editor preferences, project settings. This is Seam 4's claim,
  corroborated. `SettingsKey<T>` also carries **`FallbackDeserializers`** — a
  list of functions for reading a key whose *type* changed between versions,
  which is the migration story Somnium's `project.toml` will eventually need.

**Defold** (`example_repo/defold-dev/defold-dev/editor`; 318 `.clj*` + 57
`.java` under `editor/src`). Read `editor/README_SYSTEMS_OVERVIEW.md` first — it
is an unusually honest architecture document.

Defold models the entire editor as **one reactive dependency graph**
(`src/clj/dynamo/graph.clj`). A node has properties (state), inputs, and outputs
that are pure functions; outputs marked `:cached` memoise and invalidate
transitively. Any output can yield an `ErrorValue` that propagates downstream
until an input supplies a substitute — **so validation errors flow to the UI
through the same graph as values.**

Its contribution to this plan is two mechanisms, both better than the
alternatives in every other engine surveyed.

*(a) Property metadata as reactive functions, not attributes.* Every piece of UI
metadata is a `dynamic` — a function of other node values:

```clojure
(dynamic read-only? (g/fnk [size-mode] (= :size-mode-auto size-mode)))
(dynamic edit-type  (g/fnk [anim-ids] (properties/->choicebox anim-ids)))
(dynamic error      (g/fnk [_node-id textures ...] ...))
```

Conditional visibility, dynamic enum contents, per-instance read-only and
per-instance validation all fall out for free, and are cached and invalidated by
the graph. Compare sbox's `[HideIf(prop, value)]`, which resolves the sibling
property **by name at runtime and logs a warning when the name is wrong** — the
failure mode of every attribute-string scheme.

Somnium cannot adopt the graph, but it can adopt the shape: Esoterica's
`TypeEditingRules` registry (§6.2.1c) is the static-language version of the same
idea — a per-component object answering `is_hidden(FieldId)` /
`is_read_only(FieldId)` / `validate(FieldId, &ReflectValue)` against the live
component, evaluated per rebuild rather than per frame. **That is the decision:
rules in Rust code, keyed by `StableId`, not conditions in the macro.**

*(b) `op-seq` — the cleanest drag-to-one-undo implementation in the survey.*
`properties_view.clj:608` installs a `MOUSE_PRESSED` filter that mints a fresh
handler; that factory does `(let [op-seq (gensym)] ...)`, and every transaction
the drag emits is tagged `(g/operation-sequence op-seq)`. `merge-or-push-undo`
folds consecutive transactions sharing a `sequence-label` into one `UndoState`.
`outline/drop!` reuses the same trick to make delete-then-paste a single "Drop"
entry.

The token is more general than the alternatives — Lumina's
`Start/Finish` edit-session callbacks, Unity's
`CollapseUndoOperations(groupIndex)`, Stride's `CanMerge` predicate — because it
needs no nesting discipline and survives async boundaries. **Somnium already has
half of it**: `live: bool` on `SetInspectorValue` and `NumericFieldMessage::
ValueChanging` mark the gesture. CONTROL-B promotes that to an explicit
`GestureId` minted at gesture start and carried on every event, so a drag that
crosses two components — a gizmo moving twelve selected entities — still folds to
one entry. And it takes Lumina's `IsNoOp()`: **if the value at gesture end equals
the value at gesture start, the entry is discarded rather than pushed.**

Defold's other contributions:

- **`:child-reqs` and the ancestor walk.** `src/clj/editor/outline.clj` — a drop
  target declares the node types it accepts as *data*, and `find-target-item`
  **walks up the parent chain until it finds an ancestor whose `:child-reqs`
  accept all dragged roots**. Dropping onto a leaf that cannot accept it still
  works, by reparenting to the nearest ancestor that can. `drop?` additionally
  rejects self-descendant drops and no-op reparents. Adopted by CONTROL-E for
  the Outliner.
- **Prefs as a JSON-Schema-shaped registry with per-node scope.**
  `src/clj/editor/prefs.clj`, explicitly modelled on VS Code's
  `contributes.configuration`. Each node may carry **`:scope` (`:global` or
  `:project`)**, propagated down the tree, and a `:scopes` map from scope to file
  path — so one logical settings tree is transparently split across a user-global
  file and a per-project file. `[:code :find :term]` is project-scoped;
  `[:code :font :size]` is global. **Seam 4 adopts this exactly**: the scope is
  declared per setting, not per file, which is what makes "which file does this
  live in?" a schema question instead of a filing decision.
- **The save-value round-trip invariant.** A resource's `:read-fn` output must
  equal its `save-value` output, with a `:sanitize-fn` for format migration.
  This is what makes dirty-detection exact and diffs clean, and it is precisely
  the property CONTROL-J's save→load→compare test asserts.
- **Defold has no thumbnails at all**, and that is a decision, not an omission:
  `grep -ri thumbnail src/` returns nothing; the asset browser uses static
  per-type icons and previews happen by *opening* the resource. Worth recording
  as the honest counter-position to §4.2 — one shipping editor concluded rendered
  tiles were not worth the machinery. Somnium's answer differs because §4.7's
  screenshot critique named the drawer as the weakest surface in the product, and
  because Phase 27-G already shipped half of it.

#### 6.2.4 The three remaining repos, briefly

**Unity** — `Unity3D/UnityCsReference-master` is the substantive one (1,496 `.cs`
under `Editor/`, 5,913 under `Modules/`); the other four Unity folders in that
directory are packages and samples, not editors. Three mechanisms worth naming:
`CollapseUndoOperations(groupIndex)` with explicit group numbers as the
drag-to-one-undo route; `SettingsProvider` with `SettingsScope { User, Project }`
where each provider's **search keywords are auto-harvested from its UI
declarations** (`GetSearchKeywordsFromSerializedObject`) rather than hand-written
— adopted by CONTROL-H, because a preferences search index that must be
maintained by hand will not be; and the **Presets** system
(`Modules/PresetsEditor/`), where any serialised object's values become an asset
that can be applied to another instance or registered as *the default for new
instances of that type*. That last is noted for CONTROL-D — a material preset
library is the same mechanism — and deferred.

**s&box** (`sbox-public-master/engine/Sandbox.Tools/`, 340 `.cs` over Qt).
Two ideas. First, **scored editor resolution**:
`ControlWidget.Create(SerializedProperty)` enumerates every `[CustomEditor]`
type, asks each for a numeric score (+1000 exact type, +500 open generic, +100
assignable base, **+10 per inheritance level** so more-derived wins, +1000 for a
matching named editor, hard −100 rejections), and takes the highest that
constructs. That beats first-match when several editors target overlapping
types. **Somnium does not need it** — `FieldType` is a closed enum, so exact
match is total — and the reason is recorded here so nobody adds a scoring system
to a table that cannot have ties. Second, the fallback chain is well shaped and
is copied: `MultiEditNotSupported` (when the property has multiple values and the
editor does not support that) → `GenericControlWidget` (reflective expansion) →
`MissingSerializedPropertyWidget`. Three distinct failure states, each visible.
s&box also cancels an undo entry when before and after are equal — the drag that
did not move — and resolves `Ctrl+Z` by **picking the undo system with the most
recent timestamp among visible, enabled widgets**, which is a neat answer to
per-dock undo without focus routing. Somnium has one undo stack and does not
need it.

**Lumina** (`LuminaEngine-main/Engine/Editor/Source`, 443 files) is C++ with
libclang codegen and an ImGui property table, and it has **the best-documented
thumbnail cache in the survey**. `ThumbnailManager` registers two kinds of
provider, matched by **walking up the class hierarchy so subclasses inherit**:
a `ThumbnailPainter` (CPU-drawn RGBA, for curves and gradients — checked first,
and **may decline and fall through**) and a `ThumbnailRenderer` (populates a
live `FThumbnailScene`). That is exactly the split Phase 27-G already chose for
Somnium — images decode in-crate, meshes are requests — and it is good
corroboration that the boundary is in the right place; what Somnium lacks is the
*decline-and-fall-through* rung and the hierarchy inheritance.

Lumina's cache is **an on-disk sidecar keyed by asset GUID under
`<EngineInstall>/Intermediates/ThumbnailCache`, validated against the asset's
content hash** — and its stated reason is the interesting part: generating a
thumbnail must never rewrite the asset file, because that bumps its mtime and
causes cook churn. That is the **counter-argument to Wicked's
thumbnail-in-the-header** (§6.2.1b), and both are right about different files:

> **The resolution this plan adopts.** Files Somnium *authors* — `.somnium`
> scenes and `.sommat` materials — carry their thumbnail in the file, Wicked's
> way, because those files are written only when a human presses Save, so the
> mtime bump is expected and the preview can never be stale. Files Somnium
> *consumes* — `.glb`, `.png`, `.hdr`, `.ktx2` — use Lumina's content-hash
> sidecar under `assets/.somnium/thumbnails/`, because rewriting a source asset
> to cache a preview of it is unacceptable.

Lumina also contributes the **edit-session callback pair** — `PropertyTable`
raises `Pre`/`Post` per value change *and* `Start`/`Finish` per edit session,
with `SetStartEditCallback → BeginTransaction` and
`SetFinishEditCallback → EndTransaction(propertyName)` plus `IsNoOp()` to drop
the drag that did not move — and **settings classes that declare their own file
in their reflection attribute** (`REFLECT(ConfigFile = ".../EditorPreferences.json",
DisplayName = "World Tool", Category = "Editor")`), rendered by the same property
table as any component. Third corroboration of Seam 4.

**Solers** (`SolersEngine-main`) is **a Godot 4.7.1 fork**, not a new engine —
`version.py` says `Solers Engine 4.7.1` and `grep -li solers editor/` hits one
unrelated file. For inspector, previews, drag, undo and settings, **read
`godot-4.7.1-stable`, which is already in the same directory.** Recorded so
nobody surveys it twice. One thing in `modules/solers_ai/` is worth a line:
`editor/solers_schema_form.cpp` generates controls from a JSON Schema, and it
keeps a **separate `presentation` dictionary** carrying `control`
(`slider`/`segmented`/`multi_select`/`multiline`), `labels`, and
presentation-level min/max overrides distinct from the schema's own
`minimum`/`maximum`. That is the clean statement of a distinction Seam 1 makes
for its own reasons: **`min`/`max` are validation and belong to the engine;
`soft_min`/`soft_max`, `step`, `precision` and the control hint are presentation
and belong to the editor.** A serializer must honour the first and may ignore
the second.


### 6.3 External literature — the rendering half

**Volumetric clouds.** The Nubis line is the model CONTROL-M implements, and the
citations below distinguish what Guerrilla actually published from what the
community reconstructed, because the 2026-08-17 draft of this file conflated
them.

- **Schneider & Vos, "The Real-Time Volumetric Cloudscapes of Horizon Zero
  Dawn", SIGGRAPH 2015** —
  <https://www.guerrilla-games.com/read/the-real-time-volumetric-cloudscapes-of-horizon-zero-dawn>.
  **Verified from Guerrilla's own abstract:** multiple cloud types across
  lighting conditions, rendered *"in under 2 milliseconds on the PlayStation
  4"*. That budget is the one this phase quotes.
- **Schneider, "Nubis: Authoring Real-Time Volumetric Cloudscapes with the
  Decima Engine", SIGGRAPH 2017** —
  <https://www.guerrilla-games.com/read/nubis-authoring-real-time-volumetric-cloudscapes-with-the-decima-engine>.
  The productionization pass: regional-scale authoring, animation, transitions
  and atmospheric integration. **It is a paper about giving artists control of a
  volumetric system, which is why it is the right reference for this phase
  specifically.** Guerrilla also released the Houdini
  [Nubis Noise Generator](https://www.guerrilla-games.com/media/News/Files/nubis_noise_generator.zip).
- **A correction the 2026-08-17 draft needs.** That draft asserted "a 2D weather
  map sampled by world XZ carrying coverage / cloud type / precipitation" as a
  Nubis citation. **That three-channel breakdown could not be verified from
  Guerrilla's own material.** What is verifiable is a texture determining cloud
  coverage and type, and a scalar 0–1 coverage. The three-channel version is the
  widely repeated community reconstruction and is a perfectly good design — but
  CONTROL-M cites it as *our* design choice, not as Nubis's, unless somebody
  opens the 2017 PDF and confirms it. Likewise the widely quoted 128³
  Perlin–Worley base and 32³ Worley detail: those come from third-party
  reimplementations, not from a page anyone here has read.
- **Nubis Evolved, SIGGRAPH 2022** —
  <https://www.guerrilla-games.com/read/nubis-evolved>. Verified claims: the goal
  moves from skybox to *flyable*; *"performant and detailed results at 1080p
  resolution without the use of temporal upscaling"*; explicit **mitigation of
  temporal artifacts in fast-moving clouds**; a near-zero-cost internal-lighting
  method for in-cloud scattering and lightning.
- **Nubis Cubed, SIGGRAPH 2023** — <https://www.guerrilla-games.com/read/nubis-cubed>.
  True voxel clouds, SDF-accelerated ray marching, fluid-simulation-based
  modelling. **Deliberately not taken**: it trades memory and a whole authoring
  pipeline for shape control Somnium has no tool to use yet.
- **Toft & Bowles, "Optimisations for Real-Time Volumetric Cloudscapes",
  arXiv:1609.05344** — <https://ar5iv.labs.arxiv.org/html/1609.05344>. **This is
  the source of the hard numbers this phase budgets against**, because it states
  the Nubis-style baseline and then benchmarks alternatives on one machine.
  Baseline: ~128 raymarch steps × ~6 lighting steps per pixel. Their method: 8
  steps, per-pixel jittered ray-start offset, analytical transmittance
  integration, and *INSIDE*-style TAA on the low-resolution buffer. Measured,
  GTX 1080 @ 1920×1080:

  | Configuration | ms |
  |---|---:|
  | Full res, 128 steps | 297.7 |
  | Half res, 128 steps | 128.0 |
  | Half res, 8 steps | 2.3 |
  | Half res, 8 steps + jitter | 7.5 |
  | Half res, 8 steps + jitter + TAA | 7.5 |
  | **Quarter res, 8 steps + jitter + TAA** | **2.4** |

  **The 2.3 → 7.5 ms jump from adding jitter is the paper's own flagged
  surprise: jitter destroys texture-cache coherence**, and they name recovering
  it as open work. CONTROL-M's plan uses blue-noise ray-start offsets, so this is
  a direct warning: *measure with and without the offset*, because the offset may
  cost more than the step count.

**Integration pitfalls, all verified and all cheap to avoid:**

- **Clouds have no single depth to reproject from.** The standard pragmatic fix
  is a transmittance-weighted depth along the ray, emitting motion vectors from
  that position. Somnium's TAA already solved the jittered-matrix reprojection
  bugs (§18) and CONTROL-M reuses that history rather than growing a private
  one — but it must supply a depth, and "the cloud's depth" is a choice, not a
  given.
- **Unreal shipped a regression in exactly this stage in 5.6**: volumetric-cloud
  temporal artifacts when the camera moves fast and clouds are occluded by scene
  geometry, absent in 5.5, attributed by Epic to changes in volumetric
  render-target reconstruction and upsampling
  (<https://forums.unrealengine.com/t/volumetric-cloud-temporal-artifacts-from-volumetric-render-target-reconstruction/2649937>).
  The reconstruction/upsample stage of a low-resolution cloud buffer is fragile
  enough that Epic broke it in a point release. CONTROL-M budgets for that.
- **Unreal's `r.VolumetricRenderTarget` modes** name the trade-off explicitly:
  mode 0 traces at quarter res, reconstructs at half, upsamples to full (Epic's
  recommendation for fast-paced gameplay); mode 1 traces at half and
  reconstructs at full; mode 2 is full res but **loses cloud–mesh intersection
  support**. CONTROL-M takes mode 0's shape.
- **Aerial perspective and clouds fight over LUT resolution.**
  `r.VolumetricCloud.HighQualityAerialPerspective=1` switches Unreal's clouds
  from sampling low-resolution aerial-perspective LUTs to per-pixel ray tracing,
  because the LUT is too coarse for clouds near the horizon; the high-quality
  path has known flickering-horizontal-line artifacts. Somnium's
  `atmosphere_lut.wgsl` and the 24U/25I froxel volume are the integration
  points, and the correct order is **apply transmittance and inscattering to the
  scene layer and the cloud layer separately, then composite** — applying aerial
  perspective once after compositing is wrong.
- **Cloud shadows: Beer Shadow Maps, not ray-marched shadows.** Unreal contrasts
  the two: ray-marched gives sharp coloured shadows but is distance-limited;
  BSM uses cascaded shadow maps, supports far distances, is faster and less
  accurate, has no volumetric self-shadow colour, and is *"usually enough for
  clouds viewed from the ground"* — Epic's console recommendation. CONTROL-M's
  low-resolution world-XZ cloud shadow texture is the BSM shape.
- **Hillaire 2020, "A Scalable and Production Ready Sky and Atmosphere Rendering
  Technique"** (EGSR; CGF 39(4); DOI 10.1111/cgf.14050) is the reference for the
  atmosphere side, and its selling point is directly relevant to CONTROL-L: it
  avoids high-dimensional LUTs, so **atmosphere composition can change
  dynamically without a heavy LUT rebuild** — which is what a time-of-day slider
  demands.

**Time of day and weather — the authoring surfaces, surveyed.**

- **Unreal**: the SunSky actor plus the Sun Position Calculator. The authoring
  surface is **physical parameters** — latitude, longitude, north offset, date,
  time of day — not curves. `VolumetricCloud` supports up to two directional
  lights, which is how sun and moon coexist.
- **Ultra Dynamic Sky**, the de-facto Unreal standard and third-party, is a
  hybrid of all three candidate surfaces: one `Time of Day` scalar everything
  derives from; colour **curve assets**; a dedicated **Cloud Profile Authoring
  Tool** that bakes the LUT the cloud shader samples; and weather as named
  states with a transition policy. **Its documented wart is instructive**: curve
  edits require pressing "Refresh Settings" to take effect. §5.3 makes that a
  named non-goal.
- **Unity HDRP**: volume-override driven. Physically Based Sky, whose
  precomputation Unity 6 reduced *specifically so time of day can change at
  runtime at no extra cost*, plus an ozone parameter for twilight accuracy.
  Volumetric Clouds are driven by a cloud LUT (altitude/density/lighting), a
  cloud volume for the region, and a **cloud map acting as a top-down coverage
  and type map** — independent corroboration that the weather-map-as-2D-field
  design is the right one, whatever Nubis's exact channels were.
- **Godot**: no built-in time-of-day or weather system at all. The community
  standard, Sky3D, replaces `WorldEnvironment` with its own node.
- **The recurring pattern across all of them: one scalar driver, a set of named
  presets, and curves or LUTs mapping driver → parameters.** CONTROL-L adopts
  exactly that, with Unreal's physical sun geometry as the driver for light
  direction because it is cheap, immediately credible, and gives latitude and
  date for free.

**Wetness — the canonical model.** Sébastien Lagarde, "Water drop 3a/3b —
Physically based wet surfaces" (2013),
<https://seblagarde.wordpress.com/2013/03/19/water-drop-3a-physically-based-wet-surfaces/>
and `.../2013/04/14/water-drop-3b-physically-based-wet-surfaces/`. Five points,
each of which CONTROL-N must respect:

1. **Albedo darkening is non-linear and depends on the material's IOR**, with
   the largest effect in mid-range albedo. Not a flat multiply.
2. Water filling micro-air-gaps makes rough surfaces read smoother and more
   reflective.
3. **Porosity is the discriminating parameter.** Brick, stone and soil darken
   strongly; glass, plastic and metal barely change. Lagarde treats porosity as
   one authored channel driving rain, pollution and aging together — which
   suggests Somnium author it once on the material, not per-weather-system.
4. **Do not author separate wet and dry texture sets.** Tweak BRDF parameters
   from the dry state: a diffuse-darkening control, a specular-boost control,
   and an `AccumulatedWater` parameter that progressively **flattens the
   normal**. Puddles are the limit case — fully flat normals, mirror reflections.
5. **Drying is spatially varying and non-homogeneous, and specular reflectance
   disappears faster than the diffuse darkening persists** (citing Lu et al.,
   2005). A puddle stops looking wet before it stops looking dark. **One scalar
   lerping everything back together will look wrong**, and CONTROL-N's
   accumulation/drying model therefore needs two time constants, not one.

Also relevant: Brinck & Maximov, "The Technical Art of Uncharted 4", SIGGRAPH
2016, lists Wetness Shading among its rendering features
(<https://advances.realtimerendering.com/other/2016/naughty_dog/index.html>);
and arXiv:2401.15628, "A Micro-Ellipsoid Model for Wet Porous Materials
Rendering", is a more principled porous-wetting BRDF than the 2013
approximations if CONTROL-N ever needs one.

**One attribution correction:** an earlier framing of this phase associated
wetness with Remedy. There is no known Remedy wetness talk; the Remedy SIGGRAPH
2015 material is *Multi-Scale Global Illumination in Quantum Break*, which is
GI. Lagarde's production context was DONTNOD's *Remember Me*. Do not cite Remedy
for wetness.

### 6.4 Provenance rules

Source, from none of them. Fyrox is MIT, Flax is BSD-like, Godot is MIT, Stride
is MIT, rbfx is MIT, Overload is MIT — all permit reuse, and
**`ATTRIBUTION.md` §15 is stricter than any of their licences on purpose.**

`ATTRIBUTION.md` **§13G** — the next free letter; §13E and §13F are Phase 27's —
is opened by this plan's reconnaissance, expanded by CONTROL-A with one entry per
reference in §6.1–6.3, and **every sub-phase adds its file-level citations before
it closes**.
A sub-phase that ships without its §13G entries is not finished.
---

## 7. The six seams

Everything in §8 hangs off six decisions. They are stated here so no sub-phase
re-litigates them. Seams 1–4 are carried from the 2026-08-17 draft with
amendments marked; Seams 5 and 6 are new, and both were found by measurement in
§4.3 and §4.4.

### Seam 1 — Properties travel as `(StableId, FieldId, ReflectValue)`

One new editor event replaces 106 `InspectorField` variants, 9 `ColorField`
variants, 27 `PostFxToggle` variants and their arms:

```rust
EditorEvent::SetComponentField {
    entity: u32,
    component: StableId,     // "somnium.Water"
    field: FieldId,          // declaration-order index
    value: ReflectValue,
    live: bool,              // unchanged drag-scrub convention
}
```

`app.rs` gains **one** handler that validates through `FieldSchema::validate`
and writes through the registry. Undo gains **one** `SetFieldCmd`, which is what
finally makes `PostProcessComponent`'s 44 fields undoable. The legacy
`SetInspectorValue` / `SetInspectorColor` path stays alive through CONTROL-B and
is **deleted at B's exit**, not left as a second way to do the same thing.

**`FieldSchema` grows the metadata an editor needs and a serializer does not.**
Today it carries `name`, `id`, `ty`, `default`, `min`, `max`, `flags`. It gains:

| New field | Why | Precedent |
|---|---|---|
| `step: Option<f64>` | the drag increment and the arrow-key nudge | Fyrox `FieldMetadata::step` |
| `soft_min` / `soft_max: Option<f64>` | the *drag* range, distinct from the validation bounds; a drag clamps to soft, typing may exceed soft but never hard | Blender's soft/hard limit distinction |
| `precision: Option<usize>` | decimal places — closes craft defect C2 | Fyrox `FieldMetadata::precision` |
| `unit: &'static str` | `"m"`, `"°"`, `"lux"`, `"ms"` — feeds 27-G's existing `NumericField::unit` | Phase 27-G |
| `doc: &'static str` | one line; the tooltip and the Help text | Fyrox `FieldMetadata::doc`, populated from the `///` comment |
| `display_name: Option<&'static str>` | when the field name is not the label | Fyrox `FieldMetadata::display_name` |
| `group: Option<&'static str>` | section heading | Fyrox `FieldMetadata::tag` |
| `advanced: bool` | folded behind disclosure | Unreal's Advanced category |
| `read_only: bool` | derived state that is shown but not edited | Fyrox `FieldMetadata::read_only` |

All optional; **every existing `component_schema!` block compiles unchanged.**

The `doc` field deserves its own note, because Fyrox's choice here is the good
one and it is cheap: their derive macro populates `doc` **from the Rust doc
comment on the field**. A declarative macro can do the same with
`#[doc = ...]` capture. The consequence is that the tooltip a user reads and the
comment a maintainer reads are the same string and cannot drift. Somnium's
`component_schema!` should take this, and the doc-comment route should be the
*only* route — no separate `doc: "..."` argument competing with it.

These are declared in the same `component_schema!` block, so a script author's
exported fields get them too: the Luau declaration syntax gains the same
optional attributes in CONTROL-B, because **the script property panel and the
component property panel become the same code.** §17.19 already generates the
former from a schema by a different route; CONTROL-B merges the routes.

**The editor side is a table, not a match.** Following Fyrox's
`PropertyEditorDefinitionContainer` (`fyrox-ui/src/inspector/editors/`, 27 files,
one per type) and Flax's `CustomEditors/` with `GenericEditor` as the fallback:

```rust
trait PropertyEditor {
    fn accepts(&self, ty: &FieldType) -> bool;
    fn build(&self, ctx: &mut PropertyEditorCtx) -> NodeHandle;
    fn to_widget(&self, value: &ReflectValue, ctx: &mut PropertyEditorCtx);
    fn from_widget(&self, msg: &UiMessage) -> Option<ReflectValue>;
}
```

keyed by `FieldType`, with **a visible "unsupported type" row rather than a
silent omission** — an inspector that quietly drops a field it does not
understand is how a schema and a panel diverge without anyone noticing.

**Why not `bevy_reflect` or `serde`?** Because the registry exists, it is
`no_std`-shaped, it already carries defaults and ranges, three consumers already
read it, and its `StableId`/`FieldId` are wire-stable by contract. Importing a
second reflection system to drive the editor would put the engine back to the
three-descriptions-drift problem `reflect_registry.rs` was written to end. This
is closed; see §14.

### Seam 2 — Assets travel as `AssetId`, and previews are jobs

`somnium_asset` gains an `AssetDb`: a scan of `assets/`, an `AssetId` derived
from the content-relative path (the rule scripting already uses, §17.18.5), an
`AssetKind` classification by extension, and metadata (bytes, mtime, content
hash, and kind-specific facts — image dimensions, triangle count). It is
authoritative for the Content Drawer, the asset picker and the material system,
and it is **queried, not re-walked** (craft defect C5).

Previews are **jobs**, not function calls. A `JobRegistry` in `somnium_core`
owns a bounded queue, a worker pool, cancellation and a progress report; the
status bar renders whatever is running (craft defect C10). Thumbnail generation,
glTF import, BC7 encode and terrain bake all become jobs.

> **This `JobRegistry` gets promoted, so build it to be moved.** Phase MORROWIND
> (§9.1) extends it into a `somnium_jobs` crate with priorities, deadlines and a
> budgeted main-thread drain, and that phase forbids a second thread pool — so
> it is a **move, not a fork**. Two things make the move a rename rather than a
> rewrite, and both are free today: keep the public surface narrow (`submit`, a
> handle, cancellation, a progress query — everything else `pub(crate)`), and
> **give every submitted job a `&'static str` name at the call site** even
> though nothing consumes it yet. MORROWIND turns those names into Phase 29
> profiler zones; retrofitting them across call sites later is tedious and ends
> up incomplete. See Appendix A.6.

**Three amendments to the 2026-08-17 statement of this seam**, all forced by
§4.2's measurement:

1. **The split is load-on-a-thread, generate-on-the-main-thread.** Fyrox's
   `AssetPreviewCache` spawns a thread that does the loading and pushes ready
   work onto a queue; only *generation*, which needs the engine and the GPU,
   runs on the main thread. The atlas therefore never needs a lock, which
   answers `thumbnail.rs`'s stated objection to threading. For Somnium the split
   is: **decode and downscale off-thread; the 64×64 atlas write on-thread.**
2. **The budget is a millisecond budget, not a count.** `DECODE_BUDGET_PER_FRAME = 2`
   is unbounded in time when one decode is 260 ms. Fyrox's `throughput = 4`
   counts only *generated* previews, so cache hits drain free; Somnium takes both
   ideas — count generated work, and stop when the frame's preview budget in
   milliseconds is spent.
3. **Frequency is a property of the asset kind.** Unreal's
   `EThumbnailRenderFrequency { Realtime, OnPropertyChange, OnAssetSave, Once }`
   — its own comment says "listed from most to least CPU demanding / frequent" —
   lets a static mesh render once and a live material render every frame it is
   visible. Somnium's `PreviewGenerator` declares the same, so CONTROL-D's
   material sphere can re-render on edit without a texture doing the same.

**The disk cache follows Godot's `EditorResourcePreview`**, whose invalidation
rule is worth copying exactly: keep a sidecar recording size, mtime, source
hash; compare **mtime first**; only if mtime differs compute the hash; if the
hash matches, rewrite the sidecar with the new mtime and keep the image. A file
that was merely touched does not regenerate. Cache location:
`assets/.somnium/thumbnails/<hash>.png`, gitignored.

**Failure is a delivered result, not silence.** Godot's previewer calls the
callback even when it could not produce a preview, passing null. Somnium's
`fail_thumbnail` already does this and the tile settles on its type icon — 27-G
got this right and it stays. Fyrox's layering is the refinement: real preview →
kind icon, grey-tinted → generic placeholder. Never nothing, and never a retry
loop.

### Seam 3 — Drags carry typed payloads and report an effect

```rust
enum DragPayload {
    Assets(Vec<AssetId>),        // note: plural
    Entities(Vec<Entity>),       // note: plural
    TerrainLayer(u8),
    FoliageKind(u8),
}

enum DropEffect { None, Copy, Move, Link }
```

`UserInterface` gains a drag state machine (press → 4 px threshold → drag →
drop/cancel), a ghost drawn at the cursor, and a query on the drop target.
Drop *handling* stays in `UiManager`, so the widget tree still owns no engine
state — the Zeta-I split holds.

**Amendment to the 2026-08-17 statement.** That draft specified
`can_accept(payload) -> bool`. Flax's `DragHelper<T, U>`
(`Source/Editor/GUI/Drag/`) shows why that is too weak, and the reason is not
theoretical:

- A helper owns `List<T> Objects` and a `ValidateFunction: Func<T, bool>`
  supplied **by the drop target** at construction. `OnDragEnter` filters the
  payload through it and keeps only what passed.
- So a drag can be **partially valid**: drag five assets onto a target that
  accepts two, and two are dropped. A boolean cannot express that, and the
  five-assets case is the normal one once CONTROL-C ships multi-select in the
  drawer.
- `DragHandlers : List<DragHelper>` lets one target compose several helpers — it
  accepts entities *or* assets *or* scripts — without a match over payload kinds
  in the target itself.
- The query returns a `DragDropEffect`, not a bool, and **that is what drives
  the cursor**: copy, move, or refuse.

Somnium's shape, therefore:

```rust
fn accept(&self, payload: &DragPayload) -> DropAcceptance;
// DropAcceptance { effect: DropEffect, accepted: SmallVec<[usize; 8]> }
```

with the accepted indices computed once at drag-enter and reused on drop, so the
highlight, the cursor and the drop all agree. The `drop_target(valid: bool)`
paint recipe already in `style.rs:333` becomes `drop_target(effect: DropEffect)`
so a partial accept can read differently from a full one.

**Hard rules, unchanged:** `Esc` cancels a drag and takes precedence in
`close_top_overlay`; a drag never leaks into the fly-cam; a cancelled drag
leaves nothing behind; every completed drop is exactly one undo step.

### Seam 4 — Settings are data, environment variables are overrides

A `Settings` struct with a schema — **reusing Seam 1's `FieldSchema`, because
preferences are just properties of a non-entity object** — persisted to
`%APPDATA%/Somnium/editor.toml` for editor preferences and
`<project>/project.toml` for project settings. Resolution order:

```
default  →  project.toml  →  editor.toml  →  SOMNIUM_* env var  →  command line
```

Env vars keep working, unchanged, and **win** — headless capture runs, the
`.somtime` harness and every recorded repro in `dev records/` must not break.
What changes is that a human has a window instead of a grep, and that an
overridden control is disabled *and says which variable overrode it* (craft
defect C8).

Two additions to the 2026-08-17 statement:

- **The per-project layer is deliberate and is not copied from Godot.** §5.1
  records that Godot does not ship this; the third-party Godot Launcher exists
  because users want it. Somnium builds it because a project-scoped startup
  scene, content root and thumbnail budget are obviously project-scoped, not
  because an upstream engine has it.
- **`default_float_step` is a setting, not a constant.** Godot's spin slider
  reads `interface/inspector/default_float_step` as the floor under a
  property's declared `step`, precisely so a property with a very fine step is
  not agonizing to drag. That is the first customer of this seam and it lands
  with CONTROL-B, before the preferences window exists — as a `Settings` field
  with a default, reachable only from the env var until CONTROL-H draws it.

### Seam 5 — Input carries its modifiers *(new, 2026-08-22)*

`WidgetMessage`'s pointer and key variants gain a modifier set:

```rust
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers { pub ctrl: bool, pub shift: bool, pub alt: bool, pub logo: bool }

MouseDown { pos: Vec2, button: MouseButton, mods: Modifiers }
MouseUp   { pos: Vec2, button: MouseButton, mods: Modifiers }
MouseMove { pos: Vec2, mods: Modifiers }
MouseWheel{ pos: Vec2, delta: f32, mods: Modifiers }
KeyDown(KeyCode, Modifiers)
KeyUp(KeyCode, Modifiers)
```

`UserInterface` already tracks `ctrl_held` and `shift_held` from
`WindowEvent::ModifiersChanged`; this seam does not add state, it **delivers
state that is already there**. The state moves from an ambient field on
`UiManager` to a value on the message, which is the difference between "the
shell can read Ctrl" and "a widget can read Ctrl".

**Why it is a seam and not an implementation detail:** four separate sub-phases
are blocked on it, and each would otherwise invent its own workaround by
reaching back into `UiManager`.

| Consumer | Needs |
|---|---|
| CONTROL-B | `Shift` fine scrub, `Ctrl` snap scrub (craft defect C1) |
| CONTROL-E | `Alt`-drag for the decal gesture; copy-vs-move effect |
| CONTROL-F | `Ctrl`+click add to selection, `Shift`+click range select |
| CONTROL-G | `Ctrl` as hold-to-invert-snap; `Ctrl`+right-click for the piercing menu |

**Two rules that come with it.**

*Platform mapping.* Godot's `is_command_or_control_pressed()` and
`OS::prefer_meta_over_ctrl()` exist because macOS uses Cmd where Windows uses
Ctrl. Somnium is Windows-first but the abstraction costs one method:
`Modifiers::command()` returns `logo` on macOS and `ctrl` elsewhere. Every
shortcut and every modifier check uses `command()` unless it specifically means
the physical Ctrl key.

*Compatibility.* `KeyDown(KeyCode)` is matched in several places. The migration
is a two-step: add `KeyDownMods(KeyCode, Modifiers)` alongside, move call sites,
delete the old variant, in that order — the strangler-fig pattern Phase 27 used
for `push_rect_filled`, which cost nothing and broke nothing.

**Also in this seam, because it is the same plumbing:** `scroll_into_view` on
`ScrollViewer`, arrow-key traversal within a focused region, and gesture-cancel
routing so `Esc` reaches an in-progress drag or scrub before it reaches
`close_top_overlay` (craft defects C4 and C7).

### Seam 6 — Every editor action is one registered command *(new, 2026-08-22)*

```rust
pub struct Command {
    pub id: &'static str,            // "editor.scene.save" — path-like, stable
    pub label: &'static str,         // "Save Scene"
    pub category: &'static str,      // "Scene"
    pub default_binding: Option<Chord>,
    pub help: &'static str,          // one line, for F1 and the tooltip
}
```

One `CommandRegistry` becomes the single source for **six** surfaces that are
today six hand-written lists:

1. the application menus (`File`, `Edit`, `Create`, `View`, `Window`, `Help`);
2. the toolbar;
3. the command palette — which stops being 15 index-dispatched entries and
   becomes every command the editor has;
4. context menus, including the Content Drawer's and the Outliner's;
5. the keybinding table and, in CONTROL-H, the keybinding *editor*;
6. the F1 Help index.

**The precedent is Godot's, and it is worth copying in its specifics.**
`EditorCommandPalette` holds `HashMap<String, Command>` keyed by a path-like
string, each carrying a callable, a display name, a `Shortcut`, its rendered
shortcut text and a `last_used` timestamp. `ED_SHORTCUT_AND_COMMAND(path, name,
keycode, command)` is **one declaration that registers the keybinding and the
palette entry at once**, and `register_shortcuts_as_command()` sweeps every
registered shortcut into the palette. Ranking is a fuzzy path score plus
recency, and the history is persisted so the palette learns.

**Dispatch is by id, never by index.** The `debug_assert_eq!` on
`STATIC_PALETTE_COMMANDS = 15` and the "append only" comment both disappear,
because there is nothing left to keep in sync.

**Enablement is a predicate, not a flag.** `fn enabled(&self, ctx: &EditorCtx) -> bool`
so `Delete` greys out with no selection and `Redo` greys out at the top of the
stack — in the menu, the toolbar and the palette simultaneously, from one
definition. This is the mechanism craft defect C8 needs, generalised: a disabled
command carries a reason string.

**Search.** Godot 4.7 added search to popup menus for discoverability in long
lists; Godot 4.8 promoted its fuzzy matcher to a public API. Somnium's rule,
following §5.1's reading of Unity Search: **structured filters are exact or
prefix and predictable; the free-text remainder is subsequence-fuzzy with prefix
and word-boundary bonuses.** Never fuzzy-match the token vocabulary itself —
`type:light` must mean exactly one thing.

**Sizing.** This is the smallest of the six seams by code volume and the largest
by leverage: 15 palette entries plus six menus plus six keybindings is
roughly 30 declarations to convert, and it retires four parallel maintenance
surfaces. It is a Track 0 item for that reason.
---

## 8. Sub-phases

Four tracks. Track 0 is the foundation and is small. Track 1 is the phase.
Tracks 2 and 3 are gated on Track 1.

Every sub-phase closes with four things, and a sub-phase missing any of them is
not finished:

1. **Reached** — the named list of knobs that moved from "environment variable,
   recompile, or nothing at all" to a labelled control with a range, a unit, a
   tooltip, an undo step and a Help line.
2. **Craft** — which of §5.2's eleven defects it closed.
3. **Evidence** — captures at 1920×1080 and at the redline minimum, diffed
   against CONTROL-A's baselines, plus any `.somtime` rows.
4. **Attribution** — its `ATTRIBUTION.md` §13G entries.

### Track 0 — Foundations

#### CONTROL-A — The reachability audit

The DOOM-A analogue: measure before believing, and make the claim falsifiable.
**No widget is written until this exists.**

1. **The reachability table.** Every `SOMNIUM_*` variable with its file, meaning,
   default, type, and whether the editor can reach it. Generated by a script
   checked in beside it so it can be regenerated, not maintained by hand. Lands
   as `dev records/phase CONTROL/CONTROL-A_reachability.md`.
2. **The component table.** Every component type, whether it has a schema, how
   many fields, how many inspector rows, and how many `InspectorField` variants
   it consumes. Same file. §4.1's table is the seed; the generated version is the
   contract.
3. **The hand-wiring census**, so §1's 675 becomes a number that goes *down* and
   is seen to: variants per enum in `editor_event.rs`, fields in
   `InspectorHandles`, rows in `field_bindings`, `IF::` occurrences in `app.rs`.
   One line per sub-phase in the record, and CONTROL-B's exit is that it fell.
4. **A test that fails.** For every registered component, every field carrying
   `FieldFlags::EDIT` must have an inspector row. It fails on day one for all
   twelve schemas; CONTROL-B is what makes it pass; it stops the regression
   permanently. **This is the single most valuable artefact of CONTROL-A**,
   because it converts the phase's thesis into a build failure.
5. **A second test that fails**: every `FieldType` variant must have a registered
   `PropertyEditor`. It fails for `Asset`, `Entity` and `Array` on day one and is
   what stops CONTROL-B silently omitting a type.
6. **Baseline captures.** `SOMNIUM_CAPTURE_UI_PNG` of every editor surface at
   1920×1080 and at the redline minimum, into
   `dev records/phase CONTROL/CONTROL-A_baseline/`. These are the before-images
   every later sub-phase diffs against, and §4.7's lesson says they are the only
   instrument that catches the class of defect Phase 27 kept shipping.
7. **A measured thumbnail baseline.** Open `assets/terrain/` with the shipped
   code under the `.somtime` harness and record the stall. §4.2 predicts
   ≥ 500 ms/frame for ~30 frames from an inflate-only lower bound; the harness
   turns that prediction into a number CONTROL-C is measured against.
8. **`ATTRIBUTION.md` §13G** expanded from the reconnaissance stub to a full
   entry per reference in §6.

**Exit:** the tables exist and are regenerable, both tests exist and fail for
stated reasons, the baseline captures and the `.somtime` row are in the evidence
folder, and nobody has written a widget.

**Implementation record, 2026-08-23.** `tools/reachability/generate.py`
generates `CONTROL-A_reachability.md` and `CONTROL-A_census.md`; deleting and
regenerating produces an empty diff, and `--check` is the non-mutating gate.
The current tree measures 100 `SOMNIUM_*` identifiers (the 96 implementation
knobs plus four diagnostic-only CONTROL-A startup controls; the plan measured
97 at `209fd07`),
18 mechanically matched legacy controls, 12 schemas with 76 editable fields and
zero generated rows, and 676 hand-wired identifiers (the historical 675 gained
one `IF::` occurrence in `app.rs`). The two opt-in tests are red for the intended
reasons: 76 missing generated rows, and no registered editor for
`FieldType::{Entity, Asset, Array}`. Ordinary test discovery skips them.

An environment-gated startup driver now selects the exact logical size, real
existing entities and validated `assets/` folders, and routes menus, overlays,
panels and the unsaved-scene prompt through their shipped actions. It produced
14 UI-inclusive surface captures at both 1280x720 and 1920x1080; PNG headers
verify all 28 exact dimensions, and representative images were visually
inspected. Target-specific colour/combo/context popups remain an explicit
residual because opening them without an edit target would fabricate state.

`CONTROL-A_terrain_open.somtime` opens the real 60-PNG `assets/terrain/` folder
through the shipped synchronous thumbnail path. Over 89 wall-frame intervals it
records mean 157.3965 ms, standard deviation 273.1025 ms, minimum 13.8120 ms and
maximum 1085.5605 ms; GPU `Frame` averages 1.4905 ms. The gap is the UI-thread
decode stall CONTROL-C must remove. `.somtime` now records `cpu Frame wall` when
timing is enabled so that stall is observable. The pre-change UI suite was 215
green.

#### CONTROL-A1 — The input seam

Seam 5, and it is small — a message-type change and its call sites.

1. `Modifiers { ctrl, shift, alt, logo }` with `command()` for the
   platform-correct primary modifier, added to `MouseDown`, `MouseUp`,
   `MouseMove`, `MouseWheel`, `KeyDown`, `KeyUp`. Sourced from the existing
   `ModifiersChanged` tracking; **no new state**.
2. Migration by strangler fig, in the order Phase 27 used and for the same
   reason: add the new variant beside the old, move call sites, delete the old.
   Nothing changes behaviour in this step.
3. `ScrollViewer::scroll_into_view(NodeHandle)`, and `bring_focus_into_view` on
   focus change. Required by CONTROL-B's search, CONTROL-C's reveal-in-drawer and
   CONTROL-F's outliner filter.
4. **Arrow-key traversal within a focused region**: `Up`/`Down` move the focused
   row in the Outliner and the Details stack, `Left`/`Right` collapse and expand,
   `Home`/`End` jump. `Tab` keeps its region-level meaning. Focus follows into
   view via (3).
5. **Gesture cancellation**, ordered. A `GestureToken` held by
   `UserInterface` while any modal-feeling gesture is in flight — a scrub, a
   marquee, a gizmo drag, a drag-and-drop. `Esc` and right-click consult it
   *before* `close_top_overlay`, and cancelling restores the pre-gesture value.
   This is Godot's spin-slider behaviour generalised to every gesture in the
   editor.
6. **Modal focus trap and return.** Zeta-H's remaining item, taken here because
   it is the same plumbing: focus enters a modal when it opens and **returns to
   the control that invoked it** when it closes. WCAG 2.4.3.

**Reached:** nothing yet — this sub-phase adds no controls. It is the only one
in the phase exempt from the reachability rule, and it says so.
**Craft:** C4 (partially — the traversal half), C7.
**Exit:** the 215 `somnium_ui` tests still pass; new tests cover
`Shift`-held delivery to a widget, arrow traversal in a list longer than its
viewport with the focused row scrolled into view, and `Esc` cancelling a scrub
without closing an open popup.

**Risk:** touching every input call site at once. Mitigated by the two-variant
migration and by the rule that step 2 changes no behaviour — if any capture
differs after step 2, the migration was wrong.

**Implementation record, 2026-08-23.** All pointer and key messages carry one
`Modifiers` snapshot sourced by `UserInterface`; the duplicate ambient booleans
are gone. `GestureToken` cancellation restores a numeric scrub before the Esc or
right-click overlay ladder runs. `ScrollViewer` brings focused descendants into
view; Tree and Details regions implement arrow/Home/End traversal; modal scopes
trap focus and return it to the invoker. Palette-to-unsaved, menu-to-modal and
context-menu-to-modal transitions restore focus in the correct order. The UI
suite is 225/225 green, including the three named exit tests plus modal return
and precision/snap semantics.

#### CONTROL-A2 — The command registry

Seam 6. Roughly thirty declarations to convert, retiring four parallel
maintenance surfaces.

1. `CommandRegistry` with `Command { id, label, category, default_binding, help }`
   and `fn enabled(&self, ctx) -> Enablement` where `Enablement` is
   `Enabled` / `Disabled(&'static str)` — the reason string is what craft defect
   C8 needs.
2. The six existing menus, the fifteen palette entries, the six hard-coded
   keybindings and the content-drawer context menu all become registrations.
   `STATIC_PALETTE_COMMANDS` and its `debug_assert_eq!` are deleted.
3. Menus, toolbar and context menus are **built from the registry**, so an
   accelerator label in a menu is rendered from the binding rather than typed as
   a string. rbfx's `EditorHotkey::ToString()` is the precedent, and the reason
   is that hand-typed accelerator text drifts.
4. The palette gains fuzzy scoring plus recency, with the history persisted —
   Godot's `_score_path` + `last_used`. Ranking rule per §5.1: structured tokens
   exact or prefix, free text subsequence-fuzzy with prefix and word-boundary
   bonuses, and **never fuzzy-match the token vocabulary itself.**
5. The F1 Help index is generated from `Command::help`, so a command without a
   Help line does not compile.
6. `Chord` as a fluent value with a `Display` impl, following rbfx.

**Reached:** every editor action becomes discoverable — from 15 palette entries
to all of them.
**Craft:** C6, and the mechanism for C8.
**Exit:** no editor action is declared twice; the palette lists every command;
a test asserts every registered command has a non-empty `help`; a test asserts
no two commands share an `id` or a default binding.

**Sequencing note:** A2 is independent of CONTROL-B and may be taken in parallel
or deferred, **but it must precede CONTROL-H and CONTROL-I**, both of which are
much larger without it.

**Implementation record, 2026-08-23.** `somnium_ui::commands` is the single
registry: 52 stable path IDs declare label, category, binding, Help, enablement,
action and surfaces. It lives in `somnium_ui`, not the Appendix A sketch's
`somnium_core`, because core already depends on UI and reversing that edge would
create a Cargo cycle. Six menus, Create, the content context menu, toolbar,
shortcuts, palette and F1 command index derive from it. Palette dispatch is by
stable ID, free text uses subsequence scoring, structured tokens use only
exact/prefix matching, and recency persists. `STATIC_PALETTE_COMMANDS`,
`palette_commands`, `run_palette_command`, positional tests and the parallel
menu-handle dispatch are deleted. Registry uniqueness/help/coverage tests,
palette scoring tests, 225 UI tests and 128 core library tests are green.

### Track 1 — Reach

#### CONTROL-B — The property seam (26-J, finally)

The largest sub-phase, the riskiest, and the one every later surface is cheaper
because of. **Nothing else runs concurrently with B.**

1. **Extend `FieldSchema`** with `step`, `soft_min`, `soft_max`, `precision`,
   `unit`, `doc`, `display_name`, `group`, `order`, `advanced`, `read_only`, and
   `scope: ChangeScope`. All optional; existing blocks compile unchanged. `doc`
   is captured from the field's `///` comment and has no competing argument
   form (§7 Seam 1).
2. **`ChangeScope`** — `Field` (default) / `Component` / `Entity` / `Scene`,
   from rbfx's `AttributeScopeHint`. `SetFieldCmd` chooses its undo strategy from
   it. `TerrainComponent::resolution` and anything else whose write rebuilds
   derived state declares `Scope::Entity` or `Scope::Scene` **before** CONTROL-B
   routes it through the generic path. A field that rebuilds and forgets to say
   so is the scene-corruption bug this seam exists to prevent, so the audit of
   which fields rebuild is a work item, not an afterthought.
3. **`EditingRules`** registry keyed by `StableId` (Esoterica's
   `TypeEditingRules`), answering `is_hidden`, `is_read_only` and `validate`
   with a tri-state so "no opinion" differs from "editable". First customers:
   `PostProcessComponent`'s enable-gated groups, `LightComponent`'s
   cone angles under a non-spot type, `WaterComponent`'s spectrum parameters.
4. **Register the missing components**: `PostProcessComponent` (44 fields),
   `ParticleEmitter`, `BuoyantVessel`, `CameraSettingsComponent`, and the
   `MeshComponent` extensions. The largest mechanical chunk and the payoff — 40
   hand-built post-processing rows and 27 `PostFxToggle` variants become one
   schema block.
5. **Add `EditorEvent::SetComponentField`**, the single `app.rs` handler, and
   `SetFieldCmd` with a `GestureId` for coalescing and an `is_no_op` discard
   (Defold's `op-seq`, Lumina's `IsNoOp`).
6. **Build Details from `ComponentSchema`** in `editor/inspector.rs`: a
   `PropertyEditor` table keyed by `FieldType` — Bool → `CheckBox`, I64/F64 →
   `NumericField` (+ slider when both bounds exist), Color → `ColorSwatch`,
   Enum → `ComboBox`, Vec2/3/4 → grouped fields, Quat → Euler triple,
   Asset → CONTROL-C's picker, Entity → an entity picker, Array → a collection
   editor — with a **visible "unsupported type" row** rather than a silent
   omission, and s&box's three distinct failure states: multi-edit unsupported,
   generic fallback, missing editor. Rows are 27-G `PropertyRow`s; `group`
   builds a real section node (Stride's `CategoryNodeUpdater`), so a section can
   fold, be favourited and be copied whole.
7. **The numeric field gets its seven conventions** (craft defect C1): relative
   accumulated threshold scaled by speed and DPI, pointer capture on cross,
   `Shift` × 0.1, `command()` rounds, right-click and `Esc` cancel restoring the
   pre-grab value, pointer restored on release, and
   `drag_step = max(step, settings.default_float_step)`. And `precision` from
   the schema, closing C2.
8. **Details navigation**: a section bar with jump links, and per-component
   Favorites pinned to the top (Unreal). Search over property names and `doc`
   text, with `scroll_into_view` from A1.
9. **The revert dot gets true semantics.** `FieldSchema::default` is the
   component default, so the dot means what Zeta-G's design wanted, and the
   honest caveat recorded there is closed rather than restated.
10. **Unify the script property panel** onto the same code. §17.19 already
    generates it from a schema by a different route; the routes merge and the
    Luau declaration syntax gains the same optional metadata.
11. **Delete** `InspectorField`, `ColorField`, `PostFxToggle`,
    `SetInspectorValue`, `SetInspectorColor`, `CancelInspectorColor`, the
    `field_bindings` table, and the `InspectorHandles` fields they served —
    **after** the new path is green, never before.
12. **A `SchemaDecorator` hook** on the registry (Stride's updater pipeline, the
    light version) so a later phase can attach editor-only metadata without
    editing the macro.

**Reached:** every field of every registered component, including all 44 of
`PostProcessComponent` and the properties of `ParticleEmitter`, `BuoyantVessel`
and `RigidBodyComponent`, which have none today.
**Craft:** C1, C2, C4 (the Details half).
**Exit:**
- CONTROL-A's two tests pass.
- **`editor/inspector.rs` and `editor_event.rs` are smaller than at CONTROL-A**,
  and the hand-wiring census number has fallen. This is how the sub-phase proves
  it did the real thing instead of adding a parallel path.
- Every component's properties are editable, and every edit is one undo step.
- Existing scenes load and save byte-identically.
- Every undo step that worked before works.
- A property round-trip test: set every field on every component through the new
  path, save, load, compare.

**Risks and their controls:** this touches every property in the editor at once.
CONTROL-A's baseline captures; the round-trip test; the rule that the legacy path
is deleted only after the new one passes both; and the `ChangeScope` audit in (2)
done *first*, because a generic `SetFieldCmd` over a rebuilding field is the one
way this sub-phase can corrupt data rather than merely break.

#### CONTROL-C — The asset seam, previews and jobs

1. **`AssetDb`** in `somnium_asset`: scan, classify by extension into
   `AssetKind`, hash, and watch with debouncing so an external tool writing a
   texture refreshes the drawer. `AssetId` from the content-relative path
   (§17.18.5's rule). Metadata: bytes, mtime, content hash, and kind-specific
   facts (image dimensions, triangle count) for the tooltip. **Queried, not
   re-walked** — craft defect C5.
2. **`JobRegistry`** in `somnium_core`: bounded queue, cancellation, progress,
   priority. Status-bar jobs widget with a cancel affordance (Flax `Progress`).
   glTF import, BC7 encode, terrain bake and preview generation all become jobs
   — craft defect C10.
3. **`PreviewCache`, rebuilt against §4.2's measurement.** Four changes from the
   shipped `thumbnail.rs`, in order of how much they matter:
   - **Visible-first.** The drawer reports its visible tile range on scroll and
     only those are prioritised (Stride's
     `TilePanelThumbnailPrioritizationBehavior`). Opening `assets/terrain/`
     decodes the eight tiles on screen, not sixty.
   - **Decode off the UI thread**, atlas write on it (Fyrox). The atlas needs no
     lock, which answers the objection `thumbnail.rs` recorded.
   - **A millisecond budget**, not a count, and counted only on work actually
     performed (Fyrox's `throughput` semantics).
   - **A disk cache** at `assets/.somnium/thumbnails/<hash>.png`, gitignored,
     with Godot's two-stage invalidation — mtime first, hash only if mtime
     differs, sidecar rewritten if the hash still matches so a touched file does
     not regenerate.
4. **Preview generators**, registered per `AssetKind` with Lumina's
   decline-and-fall-through and Unreal's frequency declaration
   (`Once` / `OnAssetSave` / `OnPropertyChange` / `Realtime`):
   - **texture** — decode + mip, off-thread, as today but threaded and budgeted;
   - **mesh / glTF** — offscreen 96² render, three-quarter view, neutral studio
     light, one frame, through the request API 27-G already built; **auto-crop
     to actual content with a small border before downscaling** (NeoAxis), so a
     thin mesh is not a speck;
   - **material** — sphere on the same rig, `OnPropertyChange`;
   - **scene** — the in-file thumbnail (§6.2.3), written by CONTROL-J on save;
   - **script** — icon plus the first doc line as the tooltip.
   Fallback is layered and never empty: preview → kind icon, grey-tinted →
   generic placeholder (Fyrox).
5. **Content Drawer**: real thumbnails in 27-G's existing tiles, the type badges
   and density toggle 27-G already shipped, plus the metadata tooltip (kind,
   bytes, dimensions or triangle count, modified), sort, type filter chips,
   back/forward/up history, breadcrumbs, in-place rename replacing the modal, and
   multi-select. All of it against `AssetDb`, so filtering is a query.
6. **Asset picker widget**: search, thumbnails, type filter, "None", and Fyrox's
   Edit / Locate / Make Unique trio. Registered as the `FieldType::Asset` editor
   from CONTROL-B, and **filtered by an `AssetKindMask` on the field** (Babylon's
   `accept`), so a texture slot cannot be handed a mesh.

**Reached:** `SOMNIUM_IMPORT`, the thumbnail budget, the content root; every
asset in the tree gains a preview, a tooltip and a picker entry.
**Craft:** C5, C10.
**Exit:** **`assets/terrain/` — 60 PNGs, 1.17 GB, 4096² — opens with no frame
over the redline budget**, measured with `.somtime` against CONTROL-A's baseline
row; a texture edited in an external tool updates its tile; the asset picker
appears in Details wherever the schema says `Asset`; a cold second run costs
nothing because the disk cache is warm.

#### CONTROL-D — Material authoring — **COMPLETE 2026-08-23**

The headline, and cheap once B and C exist.

1. **`.sommat` asset**: serde JSON carrying every field `GpuMaterial` holds —
   base colour, metallic, roughness, emissive + intensity, transmission, alpha
   cutoff, double-sided — and the five texture slots (albedo, normal,
   metallic-roughness, occlusion, emissive) as `AssetId`s with kind masks. The
   file carries **its own preview thumbnail in the header** (§6.2.3), written on
   save.
2. `MaterialComponent` references a material asset; the pool index becomes
   runtime state derived from it, not the authored value. `scene_schema` round
   trips the reference.
3. **Details' Material section is generated from the schema.** No bespoke panel.
   If this sub-phase writes a hand-built material panel, CONTROL-B failed.
4. **Live preview**: the same offscreen rig as CONTROL-C renders the selected
   material to a sphere in the Details header, re-rendered on change
   (`OnPropertyChange`), and to the drawer tile.
5. `Create > Material` in the drawer context menu and the Create menu — both
   from CONTROL-A2's registry, so it is one declaration. Rename and duplicate
   follow §17.20.1's naming rules verbatim: no separators, no reserved names,
   nothing overwrites, no delete.
6. **glTF import writes `.sommat` siblings**, so an imported model's materials
   become editable instead of opaque. This is the difference between a material
   editor and a material *viewer*.
7. Multi-entity assignment, and **Make Unique** for the shared-material case
   (Fyrox `MaterialFieldEditor`).

**Reached:** every field of `GpuMaterial`, from one (base colour) to all of them.
**Exit:** create a material, set roughness 0.2 and metallic 1.0, assign it to a
cube by dragging, save, quit, reopen, and it is still a polished metal cube.

**Implementation evidence (2026-08-23):** `somnium_asset::material::MaterialAsset`
is the single serde/reflection/runtime-conversion representation. Its versioned
header embeds the saved 64x64 PNG; its generated panel carries the same live
thumbnail-atlas sphere used by the drawer. `MaterialComponent` schema v2 writes
only the durable `AssetId`; the renderer pool slot is runtime-only and is rebuilt
for every authored reference after load. Texture-slot decode runs through the
bounded job registry, then main-thread upload refreshes the shared pool entry.
glTF import materializes decoded/embedded textures and one non-overwriting
`.sommat` sibling per source material on the import worker. The registry declares
`editor.asset.new_material` once for both Create surfaces. Focused proofs cover
all 16 fields/five texture masks, preview header round-trip and invalidation,
polished GPU reconstruction (roughness 0.2/metallic 1), scene reference
round-trip with no pool id, generic live-gesture undo/redo, vector assignment,
Make Unique's non-deleting undo, and glTF sibling creation.

#### CONTROL-E — Drag and drop (26-D2) — **COMPLETE 2026-08-23**

Seam 3, plus its routes.

| From | Onto | Result |
|---|---|---|
| Drawer `.glb` | Viewport | Import + spawn at the terrain ray-hit, one undo step |
| Drawer `.sommat` | Viewport entity / Outliner row | Assign material |
| Drawer `.luau` | Viewport entity / Outliner row | Attach script |
| Drawer texture | A material texture slot in Details | Set the slot |
| Drawer `.somnium` | Viewport | Load scene (guarded by the unsaved modal) |
| Outliner row | Outliner row | Reparent |
| OS file | Drawer | Import into the current folder as a job |

Plus the mechanics:

- The drag ghost, the drop-target highlight in `style.rs`'s existing
  `drop_target` recipe — widened to take a `DropEffect` so a partial accept
  reads differently from a full one — and cursor states per effect.
- **`DropAcceptance` returning a filtered subset and an effect**, not a bool
  (§7 Seam 3), so dragging five assets onto a target that accepts two drops two.
- **The rejection reason is the adorner's text.** Stride's
  `CanAddChildren(children, modifiers, out message)` returns *why*, and the drop
  overlay shows it. "Can't drop here" is the fallback, not the answer.
- **The ancestor walk** (Defold's `:child-reqs`): a drop onto a row that cannot
  accept the payload retries against its parent chain until something can, so
  dropping a mesh onto a leaf reparents it under the nearest container.
  Self-descendant drops and no-op reparents are rejected before the highlight
  appears, not after the drop.
- `Esc` cancels a drag via A1's gesture token and takes precedence in
  `close_top_overlay`. A drag never leaks into the fly-cam.
- Every completed drop is exactly one undo step.

**Craft:** C7 (the drag half).
**Exit:** every row above works, each is one undo step, each shows a truthful
cursor and highlight *before* the button comes up, and a cancelled drag leaves
nothing behind. One capture per route.


**Implementation record, 2026-08-23.** Evidence:
[`CONTROL-E_drag_drop.md`](phase%20CONTROL/CONTROL-E_drag_drop.md). All seven
routes land as exactly one undo step through one semantic `DropRequest`; the
acceptance the adorner renders *is* the value `release()` returns, because both
read one cached `DropAcceptance`. Viewport picking exists for the first time
(ray/AABB in model space). The fourteen captures are **owed** — no GPU-backed
capture run was available — and the `.somnium` route's undo granularity is
bounded by scene load clearing the stack, which CONTROL-J states rather than
papers over.

#### CONTROL-F — Outliner, selection, clipboard — **COMPLETE 2026-08-23**

The second-riskiest sub-phase after B, and it is taken alone.

- **Multi-select**: `Option<Entity>` becomes an ordered `Vec<Entity>` with a
  *primary* for the gizmo pivot. This crosses `EngineContext`, the gizmo, the
  outline pass, the script bridge and Details — 71 call sites. It gets a
  primary-selection shim so single-selection call sites keep compiling unchanged,
  and its own tests.
- **Multi-edit in Details, built Godot's way** (§6.2.1, `multi_node_edit.cpp`):
  a synthetic `MultiSelectionTarget` whose field list is the intersection of the
  selection's schemas — matched on `StableId`, `FieldId` *and* `FieldType`
  including the enum variant list — and whose writes fan out under one undo
  entry. **The inspector learns nothing about multi-selection.** Mixed values
  render as `—` and are written only when touched (Unity's convention);
  right-clicking a mixed row offers "take value from…" per selected entity
  (Unity's underrated affordance).
  Metadata combination follows Stride: each metadata key decides how it merges
  across the selection rather than being dropped when it differs — a `soft_max`
  becomes the max, a `unit` must agree or the row is unitless.
- Marquee select in the viewport; `command()` adds, `Shift` extends a range —
  both now expressible thanks to A1.
- Per-row visibility and lock toggles, with **click-and-drag across the column
  to bulk-toggle** (Godot 4.8); type icons; hierarchy guides; badges for
  script-error, hidden, locked and dirty.
- **Sticky section headers** while scrolling a deep hierarchy (Godot 4.8).
- `F2` rename in place; a context menu built from CONTROL-A2's registry (Focus,
  Rename, Duplicate, Delete, Create Child, Copy, Paste); `F` to focus; typed
  filters (`type:light`, `script:`) with the §5.1 ranking rule.
- **Clipboard**: copy/paste a subtree with entity-handle remapping (Fyrox
  `scene/clipboard.rs`), across scenes within a session — and **property-group
  copy/paste** in Details (Godot 4.7), which is nearly free once `group` is a
  real section node.

**Craft:** C3, C4 (the Outliner half).
**Exit:** select twelve entities, set their roughness once, undo once. A mixed
row shows `—` and does not overwrite when untouched. Copy a subtree, paste it,
and the hierarchy and every property survive.


**Implementation record, 2026-08-23.** Evidence:
[`CONTROL-F_selection.md`](phase%20CONTROL/CONTROL-F_selection.md). `Selection`
keeps `primary` a public `Option<Entity>`, so the ~60 single-selection call
sites compiled unchanged; one reconcile point per frame keeps the set honest.
Multi-edit is a schema intersection with a `mixed` flag — Details learned
nothing. `EditorFlags` makes hide/lock a real serialised component. Found and
fixed a live bug: `EntitySnapshot::respawn` silently dropped a mesh that had no
material, losing geometry on delete-then-undo. **Three §8 bullets are
deferred with stated prerequisites**: sticky section headers (the tree never
sees the scroll offset), property-group copy/paste (`group` is not a section
node yet), and "take value from…" (Details has no context menu).

#### CONTROL-G — Viewport control — **COMPLETE 2026-08-23**

- **Snapping**: translate grid (0.1 / 0.25 / 0.5 / 1 / 5 m), rotate angle
  (1 / 5 / 15 / 45°), scale increment, snap-to-surface, with a snap cluster in
  the floating context bar and `command()` as hold-to-invert. Stored as settings
  (Seam 4), not constants (Fyrox `settings/move_mode.rs`).
- Gizmo space toggle (local/world) and pivot mode (individual/centre).
- **Select and transform decoupled** (Godot 4.6): a select-only mode so picking
  cannot accidentally drag. This is a real bug class, not a preference.
- **The piercing menu** (Unity 6): `command()`+right-click lists every selectable
  entity under the cursor. Craft defect C9, and the foliage scenes need it.
- Camera: `F` focus selection, orbit-around-selection, bookmarks
  `Ctrl+1..9` set / `1..9` recall, view presets (Top/Front/Side/Persp), and a
  corner axis widget that is also clickable.
- **The view-mode menu.** Every debug visualization in the renderer — the
  `SOMNIUM_SHADOW_DEBUG` / 24AB Dbg family, terrain layer views, cluster heat,
  overdraw, velocity, TAA delta, GTAO, pixel census, terrain wetness (view 23) —
  becomes a named menu entry with a Help line, registered as commands.
  **Zero renderer work**; the entire cost is a table and CONTROL-A2's registry.
  `InspectorField::TerrainDebugView` and its magic integer die here.
- **Toolbar overflow** (Unreal 5.6): the context bar condenses into an overflow
  menu on a narrow viewport rather than clipping. The 68 px budget makes this
  necessary, not optional.
- Statistics overlay: triangles, draw calls, instances, VRAM, resolution scale.
  The status bar's "objects, because that is what it can state honestly"
  (Zeta-G) gets its real numbers.

**Reached:** `SOMNIUM_SHADOW_DEBUG` and every debug view behind it,
`SOMNIUM_CENSUS`, `SOMNIUM_CULL_STATS`, `SOMNIUM_TAA_DEBUG`, `SOMNIUM_RT_DEBUG`,
`SOMNIUM_WATER_VIEW`, `SOMNIUM_TIME_VIEW`, `SOMNIUM_KIT_VIEW`, the camera
placement variables.
**Craft:** C9.
**Exit:** the phrase "type 24 into that field" is not required to reach any debug
view; a box moved with snapping on lands on the grid; clicking through a foliage
cluster reaches the object behind it.


**Implementation record, 2026-08-23.** Evidence:
[`CONTROL-G_viewport.md`](phase%20CONTROL/CONTROL-G_viewport.md). The view-mode
menu is **generated** from `somnium_ui::debug`'s tables, so all 34 shader debug
codes and 18 pipeline switches are named commands with Help lines and a
renamed view is a build failure. This is what closed CONTROL-H's env-route
gate. Snapping rounds the result rather than the delta and `command()` inverts
it both ways; the gizmo moves the whole selection by one delta. Camera
bookmarks, view presets and the clickable axis widget arrive on a new
`camera_pose` request beside `camera_focus`. `ViewportStats::vram_bytes` is
always zero and never drawn — wgpu exposes no allocation total on this path,
and a confident-looking estimate is exactly what Zeta-G's status bar avoids.

#### CONTROL-H — Preferences, keybindings, project settings — **COMPLETE 2026-08-23**

- **Seam 4's resolution order, implemented**, with the env var winning and
  **saying so in the UI** — "overridden by `SOMNIUM_HEXTILE`", control disabled,
  reason in the tooltip, via CONTROL-A2's `Enablement::Disabled(reason)`. This
  is the detail that keeps the `dev records/` repros honest (craft defect C8).
- **Scope is declared per setting, not per file** (Defold's `:scope`): each
  setting says `Global` or `Project`, and the writer routes it. "Which file does
  this live in" becomes a schema question.
- **A searchable Preferences window** whose rows are `PropertyRow`s built by
  CONTROL-B's `PropertyEditor` table — settings are properties (Seam 1/4), and
  three engines surveyed reached the same conclusion independently (Stride,
  Lumina, Esoterica). Categories, per-setting revert, a "modified only" filter.
- **The search index is generated from the declarations**, not maintained
  (Unity's `GetSearchKeywordsFrom*`).
- **Keybinding editor** with conflict detection and reset-to-default, over
  CONTROL-A2's registry — which is what makes it a table view rather than a
  feature.
- Project settings: startup scene, content root, autosave interval, thumbnail
  budget, external editor command, `default_float_step`.
- Recent scenes in `File`, most-recent-first, missing entries greyed (craft
  defect C11).
- **One schema, two consumers** (luanti): the settings schema also generates the
  environment-variable reference table CONTROL-A checked in by hand, so the
  documentation cannot drift from the settings.
- The 27-G **project picker**, unblocked — 27 deferred it because it needed an
  `EditorEvent` addition that phase forbade, and this phase does not forbid it.

**Reached:** all 97 environment variables, or listed in CONTROL-A's table with a
stated reason why one is deliberately capture-harness-only.
**Craft:** C8, C11 (the preferences half).
**Exit:** the table has no unexplained rows; setting a value in the window and
restarting preserves it; an env var override is visible, disabled and explained.


**Implementation record, 2026-08-23.** Evidence:
[`CONTROL-H_settings.md`](phase%20CONTROL/CONTROL-H_settings.md). Seam 4 is
implemented *as* Seam 1: two `Component`s with schemas on one entity in a
private `World`, so Preferences is the generated Details panel and a setting
costs one schema line. Files are hand-rolled flat TOML — the value space is
four scalar types and a dependency was not worth it. **The exit condition is
met and enforced**: all 106 `SOMNIUM_*` variables have a verified route (24
schema, 6 setting, 23 command, 53 harness-with-a-reason, zero unexplained), and
a `command` route naming an unregistered id fails exactly as an unclassified
variable does. `autosave_interval_s` lands here but is consumed by CONTROL-J.

#### CONTROL-I — Log, diagnostics, jump to source — **COMPLETE 2026-08-23**

- **Clickable `file:line:column`** in the Output Log → opens the external editor
  from CONTROL-H's setting **at the line**, or reveals the file if none is set.
  Closes the first bullet of §17.18.6; Godot 4.6 is the precedent and the
  at-the-line detail is the part that matters.
- Severity chips (Error / Warn / Info / Debug), category filter, search,
  timestamps, copy, clear, pin, and a "N script errors" status click-through to
  the first diagnostic.
- Toasts for job completion and failure; errors persist until dismissed.
- **The job list from CONTROL-C gets its panel here**, not just its status-bar
  chip, so a cancelled or failed import is inspectable.

**Exit:** a Luau syntax error is one click from the offending line.


**Implementation record, 2026-08-23.** Evidence:
[`CONTROL-I_log.md`](phase%20CONTROL/CONTROL-I_log.md). "One click from the
offending line" is a parsing problem, so `parse_source_refs` is pure and tested
directly: a Windows drive letter is a colon that is not a separator, a
timestamp is three numbers with no file, trailing punctuation is not path.
`OutputLog` holds the policy so it is testable without a GPU device. Error
toasts persist until dismissed, classified by the same `infer` the log uses, so
seventy existing call sites got the behaviour untouched. The external editor is
spawned but **not verified** — a program that starts and ignores its arguments
looks like success, which is the honest boundary of a shell-out.

#### CONTROL-J — Scene lifecycle — **COMPLETE 2026-08-23**

- **Fix `LoadScene`.** Route by file version: v2 recipes to `map::load_map`,
  schema scenes to `scene_schema::load_scene_schema` plus GPU-side
  reconstruction — meshes from `MeshKind`, terrain sidecars, material assets
  (CONTROL-D), renderer uploads. The Content Drawer's `.somnium` double-click and
  `File > Open` both work afterwards.
- **Stop dropping unknown data** (§6.2.3, Stride's `IUnloadable`). Today
  `scene_from_json` skips an unregistered component with a warning and drops an
  unknown field with a warning, so a load-then-save in a build missing a
  component **destroys that data permanently.** Unknown components and fields are
  retained verbatim as opaque JSON on the entity and written back on save. A test
  asserts the round trip through a registry that deliberately lacks a component.
- **The round-trip invariant** (Defold): the value read from a file must equal
  the value written back, and a `sanitize` step handles format migration
  explicitly rather than by accident. This is what makes dirty-detection exact,
  and the save → quit → reopen → compare test asserts it on the component graph.
- **A scene thumbnail in the file header** (§6.2.3, Wicked): on save, the current
  viewport frame is encoded and written into `.somnium` ahead of the data, and
  the drawer reads only the header bytes. Free, never stale, and it deletes the
  scene case from CONTROL-C's cache.
- Autosave to `assets/.somnium/autosave/`, on an interval and before Play;
  crash-recovery prompt on next launch (craft defect C11).
- **Undo history panel**: the stack as a list, current position marked, click to
  jump (Flax `History`). Entries are named after what changed —
  "Set Water · wave_height" — following Stride's
  `$"Update property {DisplayPath}"`, because a history of twenty rows reading
  "Change" is not a history. A desynchronised stack surfaces a typed error
  (rbfx's `UndoException`) rather than partially applying.
- Scene-modified state already exists; it now gets an accurate title bar.

**Exit:** save, quit, reopen, and the scene is the scene. The `NEXT:` line at the
top of `context.md` is deleted, not reworded.

**Implementation record, 2026-08-23.** Evidence:
[`CONTROL-J_scene_lifecycle.md`](phase%20CONTROL/CONTROL-J_scene_lifecycle.md).
`LoadScene` routes by format through `SceneKind`, and a schema scene rebuilds
the GPU state the file deliberately omits — primitives overwritten *in place*
so identity, parent and authored material survive, terrain sidecars, material
pool slots. `RetainedUnknowns` closes §6.2.3's silent data-loss path. The
`.somnium` container is a framed header plus body, so the drawer reads a
thumbnail with a `seek`; unframed files still load with no migration. Autosave
and crash recovery are in, offered rather than applied. The undo history is a
clickable list with the position marked. **The `NEXT:` line in `context.md` is
deleted.** Two things are owed and stated: the thumbnail's *pixels* (the
plumbing is complete, but the renderer has no non-capture readback path), and
`RetainedUnknowns` travelling through the clipboard.

**Track 1 is complete**, and so — as of 2026-08-23 — are Tracks 2 and 3,
CONTROL-O's optional stretch included. §9.1 recorded that Track 2 was not
optional because CONTROL-K is a hard dependency of MORROWIND-H, MORROWIND-L and
MORROWIND-AG; it shipped, and they are unblocked.

### Track 2 — Author

**Gate:** Track 2 does not start until CONTROL-B is in tree, because both
sub-phases below are new `FieldType`s and both would otherwise be hand-wired.

#### CONTROL-K — Curve and gradient editing — **COMPLETE 2026-08-23**

- `CurveEditor`: keyframes, linear/smooth/step tangents, zoom/pan, snap,
  presets, and a compact inline form for a `PropertyRow` (Fyrox `fyrox-ui/src/curve/`,
  Flax `GUI/CurveEditor*.cs`).
- `GradientEditor`: colour stops over the existing `ColorPicker`.
- Registered as the editors for new `FieldType::Curve` and `FieldType::Gradient`
  (Seam 1), so any component can declare one, and both round-trip through
  `scene_schema`.
- **Curve edits are live.** §5.3 names Ultra Dynamic Sky's "Refresh Settings"
  button as an explicit non-goal.
- First consumers, all existing features that currently have none:
  colour-grading response curves, particle colour-over-life (the
  `ParticleStart`/`ParticleEnd` pair becomes a real ramp), foliage LOD falloff,
  and CONTROL-L's day track.
- **`SliderCurve`** (NeoAxis's `ConvenientDistribution`) lands here rather than
  in CONTROL-B: a schema may declare that a slider is exponential, which matters
  for light intensity, fog density and roughness, where linear is the wrong feel.

**Exit:** a curve authored in the editor round-trips through `scene_schema` and
drives a shader uniform, with no restart and no refresh button.

**Implementation record, 2026-08-23.** Evidence:
[`CONTROL-K_curves.md`](phase%20CONTROL/CONTROL-K_curves.md).
`Curve`, `Gradient` and `SliderCurve` are `somnium_ecs::curve` value types, and
`FieldType::{Curve, Gradient}` / `ReflectValue::{Curve, Gradient}` put them in
Seam 1's vocabulary — so a curve gets the generated row, the scoped undo entry,
the drag-scrub coalescing, the scene round trip and the script visibility for
free, and none of those four consumers learns what a keyframe is. Both editors
are registered `PropertyEditorKind`s; the gradient reuses the existing
`ColorPicker` rather than growing a second colour surface, which collapsed three
duplicated write-back matches into one `write_color_target`. Exit met through
`PostProcessComponent::response_curve`, sampled every frame into
`PostParams.response` — there is no refresh step because the table is rebuilt
unconditionally. Owed: draggable tangent handles, and presets are keyboard-only.

#### CONTROL-L — Time of day — **COMPLETE 2026-08-23**

- `TimeOfDayComponent`: hour, day of year, latitude, longitude, timescale, with a
  schema block and therefore a free inspector.
- Drives the existing `somnium_core/src/sun.rs` sun position **analytically** from latitude,
  longitude and date — Unreal's SunSky model, chosen because it is cheap,
  immediately credible and gives the parameters for free — plus the moon, star
  fade, sky and fog. 25M's night work becomes a *cycle* instead of a
  configuration.
- Curve tracks (CONTROL-K) for sun colour, sun intensity, fog density, exposure
  compensation and cloud coverage across 24 h; presets for dawn / noon / golden
  hour / dusk / night. The pattern is the one every surveyed engine converged on
  (§6.3): **one scalar driver, named presets, curves mapping driver → parameters.**
- A scrub control in the floating context bar; Play advances time at `timescale`.
- `SOMNIUM_SUN_AZIMUTH` / `SOMNIUM_SUN_ELEVATION` become overrides of a real
  system rather than the only way to place the sun.
- **Watch the atmosphere LUT rebuild cost.** Hillaire 2020's contribution is that
  composition can change dynamically without a heavy rebuild; if Somnium's LUT
  turns out to need one per frame at scrub speed, that is a measured cost that
  goes in the record, not a surprise discovered in CONTROL-M.

**Reached:** `SOMNIUM_SUN_AZIMUTH`, `SOMNIUM_SUN_ELEVATION`, and the sky/fog
parameters currently authored only in code.
**Exit:** dragging one slider takes the Coastal scene from noon to dusk with the
existing 25M night path, captures at four times of day are in the evidence
folder, and the LUT cost per scrub frame is a number in the record.

**Implementation record, 2026-08-23.** Evidence:
[`CONTROL-L_time_of_day.md`](phase%20CONTROL/CONTROL-L_time_of_day.md).
`TimeOfDayComponent` is six scalars and five CONTROL-K tracks in one schema
block. `solar_position` is NOAA's formulation — Unreal's SunSky choice — and
the time zone is derived from longitude rather than authored, so the two cannot
disagree. The driver writes the first *directional light*'s rotation, colour
and intensity (not an entity named `SunLight`: a name is not a type), pushes fog
and exposure straight to the renderer rather than fighting `PostProcess`, and
never sets `scene_dirty`. Reachable from Details, from a `HH:MM` scrub on the
viewport context bar, and from six generated preset commands. The env vars are
now overrides of a real system and still win, per Seam 4. **Owed: the four
captures and the LUT-cost row**, both of which need a windowed run; the LUTs are
built once at startup and the driver does not force a rebuild, but that is an
observation from reading the pass, not a measurement.

### Track 3 — Sky

**Gate:** Track 3 does not start until CONTROL-B, C and G are in tree. It is the
phase's proof, not its content, and shipping it early would reproduce exactly the
failure this phase exists to fix.

#### CONTROL-M — Volumetric clouds — **COMPLETE 2026-08-23**

- **Shape.** A low-frequency Perlin–Worley base eroded by high-frequency Worley
  detail, generated once at build or first run into a cached 3D texture, plus a
  2D **weather map** sampled by world XZ. Resolutions and channel assignment are
  **Somnium's design decision, stated as such** — §6.3 records that the widely
  quoted 128³/32³ and the coverage/type/precipitation channel split could not be
  verified from Guerrilla's own material, and this plan does not launder a
  community reconstruction as a citation. Unity HDRP's cloud map is independent
  corroboration that a top-down coverage-and-type field is the right shape.
- **March.** Quarter-resolution raymarch with adaptive step size (large steps
  until density, small steps inside), early-out on transmittance, and
  reprojection into the **existing TAA history** rather than a private buffer —
  Somnium already solved the jittered-matrix reprojection bugs (§18) and a second,
  naive history would reintroduce them. Motion vectors come from a
  **transmittance-weighted depth** along the ray, because a cloud has no single
  depth (§6.3).
- **Measure the jitter, do not assume it.** Toft & Bowles measured 2.3 → 7.5 ms
  from adding a per-pixel jittered ray-start offset, and named cache incoherence
  as the cause. CONTROL-M's `.somtime` row is taken **with and without** the
  blue-noise offset, and if the offset costs more than the steps it saves, it is
  cut and the number is recorded.
- **Light.** Henyey–Greenstein forward scattering with a dual-lobe
  approximation, Beer's law with the powder term, cone-sampled shadow taps toward
  the sun, and ambient from the existing atmosphere LUT so clouds and sky share
  one sun colour.
- **Integration.** Composited **before** the froxel aerial-perspective resolve so
  distant clouds inherit 24U/25I's fog, with transmittance and inscattering
  applied to the scene layer and the cloud layer **separately, then composited**
  (§6.3 — applying aerial perspective once after compositing is wrong).
  **Cloud shadows** written to a low-resolution world-XZ texture consumed by the
  shading pass, on terrain and water alike — the Beer-Shadow-Map shape, which
  Epic recommends for ground-level viewing.
- **Reconstruction is the fragile part and is treated as such.** Epic shipped a
  regression in exactly this stage in UE 5.6 (§6.3). The quarter-res →
  reconstruct → upsample chain gets its own captures with a fast-moving camera
  and clouds occluded by geometry, because that is the case that broke there.
- **Authored, on day one.** A `SkyComponent` schema block with coverage,
  altitude, thickness, wind vector, detail strength and precipitation; a
  weather-map paint mode reusing the terrain brush infrastructure; a preset list.
  **This is the sub-phase that either proves or refutes §1.**
- **Budget and default.** Target ≤ 2 ms at 1080p quarter-res on the reference
  RTX 5080 Laptop — Guerrilla's verified "under 2 ms on PlayStation 4" is the
  bar, and Toft & Bowles' 2.4 ms for quarter-res 8-step + jitter + TAA on a GTX
  1080 says it is reachable. Measured with `.somtime` against a DOOM-A baseline.
  **Default off until that number exists.** A cloud pass costing 3 ms on a
  19.9 ms frame ships off and says so, exactly as tile binning and the aerial
  pipeline did.

**Exit:** a `.somtime` row for the cloud pass with and without jitter, four
captures (clear, scattered, overcast, storm), a fast-camera occlusion capture,
cloud shadows visible on terrain and water, and every parameter above reachable
from Details.

**Implementation record, 2026-08-23.** Evidence:
[`CONTROL-M_clouds.md`](phase%20CONTROL/CONTROL-M_clouds.md).
Twenty-one parameters, one `component_schema!` block, **zero** new environment
variables that are the only route to anything — which is the test §1 set this
sub-phase. Perlin–Worley base eroded by Worley detail, a top-down weather map,
a quarter-res adaptive march with early-out and depth occlusion, dual-lobe HG
with Beer-powder and cone-sampled shadow taps, ambient from the sky's own
multiscatter LUT. Aerial perspective is applied **inside** the march at the
cloud's transmittance-weighted depth, and the composite is a fixed-function
`One`/`SrcAlpha` blend into the HDR buffer TAA already resolves — no private
history. Cloud shadows are a world-XZ field folded into `shadow_factor`, so
terrain, water and meshes read one source. The weather map is paintable
engine-side. The resolutions and channel split are labelled **Somnium's
decision** in the shader, per §6.3's refusal to launder a community
reconstruction. **Owed: all four evidence items**, which need a windowed run —
and until the `.somtime` row exists the pass ships off, which is what §12 asks.
Also owed: the painter's viewport gesture, and a cloud debug view.

#### CONTROL-N — Weather and wetness — **COMPLETE 2026-08-23**

One sub-phase rather than three, because the chain is the point: coverage drives
precipitation, precipitation drives wetness, wetness drives the surface.

- `WeatherComponent`: precipitation type and rate, wind speed and direction,
  wetness target, temperature (the rain/snow switch).
- **Wetness** already exists as a shipped terrain uniform (`TerrainWetness`,
  XV-H, `SOMNIUM_TERRAIN_WETNESS`) with no driver, and as debug view 23 in
  `shading.wgsl`. Weather animates it and extends it to meshes, following
  Lagarde (§6.3) rather than by feel:
  - **two time constants, not one** — accumulation and drying differ, and
    specular recovery outruns diffuse recovery, so a puddle stops looking wet
    before it stops looking dark;
  - **porosity as an authored material channel**, one value driving rain,
    pollution and aging, not a wetness-only input;
  - **no separate wet texture set** — a diffuse-darkening control, a
    specular-boost control, and an accumulated-water term that progressively
    flattens the normal, with puddles as the flat-normal limit case;
  - albedo darkening non-linear in the base albedo, not a multiply.
- **Precipitation** through the existing particle emitter — camera-anchored,
  wind-sheared, occlusion-faded under cover using CONTROL-M's cloud shadow
  texture.
- **Rain ripples on water**, hooked into the existing foam/whitecap path in
  `water.wgsl`; wind speed feeds the FFT spectrum through the existing
  `WaterWindSpeed`, which is currently authored by hand and never changes.
- **Wind becomes one global vector**: foliage sway, cloud advection, ocean
  spectrum and precipitation shear all read it instead of three private
  constants.
- Weather is **named states with an explicit transition duration** (Ultra Dynamic
  Sky's model), authored as presets, not a pile of sliders.

**Reached:** `SOMNIUM_TERRAIN_WETNESS` and `WaterWindSpeed` gain drivers and
controls; mesh wetness and porosity become authored material properties.
**Exit:** one weather preset takes Coastal from clear to storm — clouds close,
light drops, rain falls, terrain and meshes darken, the sea roughens — with no
env var touched and every step undoable.

**Implementation record, 2026-08-23.** Evidence:
[`CONTROL-N_weather.md`](phase%20CONTROL/CONTROL-N_weather.md).
All four of Lagarde's findings are load-bearing: two time constants, specular
recovering before diffuse, porosity as a **material** channel (in
`GpuMaterial`'s existing padding, so the struct is unchanged), and no separate
wet texture set — non-linear albedo darkening, a specular boost, and standing
water flattening the normal. Every approach is `1 - exp(-dt/tau)`, so wetness
behaves the same at 30 Hz and 240 Hz. Snow is gated on the *kind*, not the rate.
The exit criterion's "one preset" is real: `editor.weather.storm` applies the
weather **and its sky** in one `CommandGroup` undo entry, because rain falling
out of a clear sky would meet the letter of the sub-phase and fail its point.
Wind is one vector with three consumers — cloud advection, the ocean spectrum
and precipitation shear. Rain ripples are a per-frame slope term in
`water.wgsl`; precipitation is the existing `ParticleEmitter` plus two new
generally-useful fields. **Owed: the capture sequence, and occlusion fade under
cover** — the cloud shadow is a GPU texture and the emitter is CPU-simulated, so
that wants MORROWIND-P's GPU particles rather than a readback. The plan also
names foliage sway as a wind consumer; **there is no foliage sway in this
engine**, so there was nothing to unify.

#### CONTROL-O — Deferred decals *(stretch)* — **COMPLETE 2026-08-23**

Included because it is the cheapest remaining renderer gap with an obvious
authoring surface, and marked optional because the phase is already large.

- Decal boxes clustered through the **existing** `cluster_offsets` /
  `light_indices` infrastructure from Phase 13C — the clustering is written,
  tested and shipping, and decals are the second consumer it was shaped for.
- Applied in the shading pass over base colour, normal and roughness.
- Authoring: drag a material into the viewport with `Alt` held → a decal entity
  with a box gizmo (the `Alt` is now expressible because of A1). Sorting by a
  priority field. Angle fade.

**Exit criterion if attempted:** it ships with the drag gesture, or it does not
ship. **If any earlier track slips, this is the first thing cut**, and cutting it
is not a failure of the phase.

**Implementation record, 2026-08-23.** Evidence:
[`CONTROL-O_decals.md`](phase%20CONTROL/CONTROL-O_decals.md).
It ships with the drag gesture. `cluster.rs` grew a `ClusterVolume` trait and
its counting sort became generic, so decals are genuinely the second consumer of
13C's binning rather than a copy of it — and the decal grid is binned inside
`render()` beside the light grid, with the same matrices, so the two agree about
what a froxel is. A decal is the entity's `Transform` (the box, projecting along
its own -Y, with the middle axis as projection *depth*) plus an ordinary
`MaterialComponent`, so it reuses all of CONTROL-D. Applied in `shading.wgsl`
before `f0` is derived, with an edge fade and an authored **angle fade** —
without which a projection aimed at the floor smears down every wall inside its
box. `Alt` + drag a material into the viewport creates one, oriented to the
terrain normal, as one undo step: the eighth route through CONTROL-E's seam, and
expressible at all only because of A1. **Owed: captures, mesh-surface drops (the
probe returns terrain points), a decal debug view, and emissive decals.**
---

## 9. Sequencing

```
A ──► A1 ──┬──► B ──┬──► C ──► D
           │        │         ▲
           │        ├──► E ───┘      (E needs C's AssetDb; D needs C+E to assign)
           │        ├──► F
           │        ├──► G
           │        └──► J           (J needs D for material references in scenes)
           │
           └──► A2 ──┬──► H
                     └──► I

B, C, G in tree ──► K ──► L ──► M ──► N ──► [O, optional]
```

- **A before everything.** No widget is written until the tables exist and the
  two failing tests are checked in.
- **A1 immediately after A**, and before B, E, F or G. It is the smallest
  sub-phase in the phase and four others are blocked on it. Taking it late means
  four sub-phases each invent a workaround.
- **A2 is independent** and may run in parallel with B or be deferred, **but it
  must precede H and I.** If the session budget is tight, A2 → H → I is a
  coherent slice on its own.
- **B before everything else in Track 1**, without exception. Every surface after
  it is smaller because of it, and any surface built before it is built twice.
  **Nothing runs concurrently with B**; it touches every property in the editor.
- **C before D and E.** Materials and drags are both asset consumers.
- **F, G, I are independent** of each other and can be taken in any order or in
  parallel sessions. **F is taken alone**, like B, because multi-select crosses
  71 call sites.
- **J after D**, because a scene that reloads must reload material references.
- **Track 2 after B**; **Track 3 after B, C and G.**

**A reasonable stopping point exists after J**, and a second after A2 → H → I.
A–J alone is a complete phase that closes every §4 gap, every §5.2 craft defect
except the curve-dependent ones, and every open Zeta-G item. Tracks 2 and 3 could
become their own phase if the session budget demands it. **Say so out loud if
that happens; do not silently drop them and declare CONTROL complete.**

### 9.1 What comes after — and it is decided *(added 2026-08-23)*

**Phase CONTROL runs first and completes in full. Phase MORROWIND is next.**
That is a decision, not a preference, and both documents record it so neither can
drift.

> **Status, 2026-08-23: CONTROL is complete and MORROWIND is unblocked.**
> CONTROL-K shipped, so MORROWIND-H, MORROWIND-L and MORROWIND-AG have the
> curve editor they depend on. Point 3 below asked that any seam whose shipped
> signature differs from §7 be named here, and three did:
>
> - **Seam 1 grew two `FieldType`s** — `Curve` and `Gradient` — and `FieldSchema`
>   grew a `slider: SliderCurve`. MORROWIND-O's prefab overrides travel as the
>   same `(StableId, FieldId, ReflectValue)` triple; they now have to be able to
>   carry a heap-backed value, which `ReflectValue` already does for `Str` and
>   `Array` but which is worth knowing before designing the patch format.
> - **Seam 6's `CommandAction` grew three id-carrying variants**
>   (`SetTimeOfDay`, `SetSkyPreset`, `SetWeatherPreset`) and two id/label tables
>   live in `commands.rs` with their *values* in `somnium_core`. MORROWIND-K's
>   node-graph palette inherits that split; it works, and the tests that keep
>   the two tables in step are the pattern to copy.
> - **`EntitySnapshot` is no longer `Copy`**, and neither are
>   `PostProcessComponent` or `FoliageComponent`. Anything in MORROWIND that
>   assumed a snapshot could be copied cheaply should clone deliberately.
>
> One thing CONTROL did **not** do on MORROWIND's behalf, and deliberately:
> precipitation is CPU-simulated through the existing emitter, and its
> occlusion fade is left owed rather than solved with a GPU readback, because
> MORROWIND-P owns GPU particles and solving it twice would be waste.

The successor is [`phase_MORROWIND.md`](phase_MORROWIND.md) — NetImmerse, the
engine-half phase: the runtime (game-facing) UI, skeletal animation, prefabs, the
asset cook and world streaming, navigation, GPU particles, virtual shadow maps,
input actions, save games, and an audio crate that is currently 93 lines. Eight
tracks, thirty-six sub-phases. **Six of its eight tracks consume the seams this
phase builds**, which is why the ordering is not negotiable in the other
direction.

Three consequences for how *this* phase is executed, and they are the reason this
section exists rather than being left in the successor's document:

1. **The stopping point after J is now a pause, not an exit.** MORROWIND does not
   start until CONTROL is complete — Tracks 2 and 3 included. Specifically,
   **CONTROL-K's curve and gradient editors are a hard dependency** of
   MORROWIND-H (UI motion), MORROWIND-L (the timeline, which embeds K's curve
   editor rather than growing its own) and MORROWIND-AG (audio attenuation
   curves). Dropping Track 2 and declaring CONTROL complete would silently
   remove a prerequisite from three MORROWIND sub-phases.
2. **The six seams are about to carry far more weight than this phase places on
   them.** Seam 1's `(StableId, FieldId, ReflectValue)` becomes the vocabulary of
   **prefab overrides** (MORROWIND-O); Seam 2's `AssetId` gains **residency
   states** and starts addressing cooked blobs (MORROWIND-Q/R); Seam 4's
   settings become the store for **input action maps** (MORROWIND-AE); Seam 6's
   command registry backs the **node-graph palette** (MORROWIND-K). Where a
   choice inside this phase is genuinely balanced, the tiebreak is: which
   version survives that. `ChangeScope` (§6.2.1a) is the model — it was adopted
   here for undo granularity and it is what will keep prefab patches from
   corrupting a rebuilding field later.
3. **MORROWIND was written against this phase's *plan*, not its result.**
   MORROWIND-A's first act is to reconcile its §7 against the seams CONTROL
   actually shipped. **Nothing is owed here** — CONTROL does not need to hit its
   plan exactly — but where a seam signature ends up different from §7 of this
   document, saying so in the completion record is worth more than it costs.

**What CONTROL must *not* do on MORROWIND's behalf:** build any of it early.
MORROWIND's §6.7 is an explicit non-overlap table listing what belongs to each
phase; docking, node graphs, timelines, virtualised lists, prefabs, play-in-editor
and the runtime UI are **MORROWIND's**, and a CONTROL sub-phase that starts one
of them has widened its own scope, not got ahead.

---

## 10. Must-not-break matrix

Carried forward from Phase 26 and Phase 27 and extended. **Every sub-phase
re-checks all of it before it closes.**

**Editor behaviour.** Viewport input and fly-cam · selection and gizmos
(translate / rotate / scale) · scene create / import / save · undo/redo including
the `live` drag convention · Play / Pause / Stop and the play-session checkpoint ·
immersive viewport and its `Esc` · terrain sculpt and paint, all six brushes ·
foliage paint, single and spread · post-processing toggles and the tonemapper ·
water and vessel workflows · profiler overlay and the `.somtime` harness ·
`SOMNIUM_CAPTURE_*` and every recorded repro in `dev records/` · script attach /
reorder / enable / reload and the script property panel · Content Drawer
authoring and §17.20.1's naming rules · F1 Help, Ctrl+P palette, Ctrl+Space
drawer, Tab focus ring, the `Esc` layer order · the seven named workspaces and
layout persistence · UI input capture never leaking into the viewport.

**Phase 27's paint contract**, added by this expansion. The primitive-quad
instance layout and its colour contract · `draw_over` ordering and the two tests
that pin it · block-origin text snapping (**do not reintroduce per-glyph
rounding**; the regression test was verified against the bug) · `ScrollViewer`
content sizing that skips hidden children · the four-cue state grammar · all 18
migrated widgets rendering through `push_paint`.

**Zeta's redlines.** The Nocturne token sheet · the certified contrast pairs
(§8A.3) · the 68 px pre-scene budget · the floating context bar · focus order ·
the `FontRole`/`TextRole` split and the five bundled cuts.

**Engine contracts.** XV's 32-layer terrain and `GpuTerrainMaterial` layout ·
`WaterComponent::great_lakes` · foliage LOD and cull distances · DOOM's measured
defaults · the `.somtime` baselines (**do not overwrite `DOOM-A_*`**).

**And one contract this phase adds, because CONTROL-B is the sub-phase most
likely to break it silently:** an existing `scene.somnium` must load and re-save
**byte-identically** at every point in the phase except where CONTROL-J
deliberately adds the retained-unknowns block and the header thumbnail — and
those two changes are versioned, tested, and stated in the record.

---

## 11. Acceptance matrix

| # | Gate | Evidence |
|---|---|---|
| 1 | Every `FieldFlags::EDIT` field has an inspector row | CONTROL-A's test, green |
| 2 | Every `FieldType` has a registered `PropertyEditor` | CONTROL-A's second test, green |
| 3 | `editor/inspector.rs` + `editor_event.rs` are **smaller** than at CONTROL-A, and the hand-wiring census has fallen from 675 | line counts and census in the record |
| 4 | Every engine component has a schema, including `PostProcessComponent`'s 44 fields | `reflect_registry.rs` + test |
| 5 | Property round-trip: set → save → load → compare, every field of every component | test |
| 6 | A field declaring `ChangeScope::Entity` or `Scene` undoes without corrupting derived state | test, per declaring field |
| 7 | Material created, edited, assigned, saved, reloaded | capture + test |
| 8 | **`assets/terrain/` (60 PNGs, 1.17 GB, 4096²) opens with no frame over budget** | `.somtime` row vs CONTROL-A's baseline |
| 9 | Thumbnails for mesh / texture / material / scene; a warm second run costs nothing | `.somtime` row |
| 10 | All seven drag routes work, one undo step each, with a truthful pre-drop highlight | capture per route |
| 11 | Multi-select edit of a shared property is one undo step; a mixed row shows `—` and does not overwrite untouched | test |
| 12 | Snapping, focus, bookmarks, piercing menu, view-mode menu | captures |
| 13 | All 97 env vars reachable, or listed with a stated reason | CONTROL-A table, updated |
| 14 | `file:line:col` click opens the source **at the line** | capture |
| 15 | Save → quit → open returns the same scene | test |
| 16 | A scene loaded by a build missing a component, then saved, **still contains that component's data** | test |
| 17 | Curves are live — no refresh step anywhere | **met**: `PostParams.response` is re-sampled every frame from the authored curve, so there is no refresh step to find. Capture owed |
| 18 | Clouds: pass timing **with and without jitter**, four captures, a fast-camera occlusion capture, cloud shadows, all params in Details | **params met** (21 fields, one schema block, zero new env-var-only knobs); cloud shadows implemented and folded into `shadow_factor`. **`.somtime` row and all four captures owed** — they need a windowed run, and the pass ships off until the row exists |
| 19 | Weather chain end-to-end from one preset | **met in code**: `editor.weather.storm` applies the weather *and* its sky in one undo entry, driving cloud coverage, terrain and mesh wetness, water wind and rain ripples. Capture sequence owed |
| 20 | All eleven §5.2 craft defects closed, each with the test or capture named in its sub-phase | per-sub-phase record |
| 21 | Zeta redlines unchanged: tokens, contrast pairs, 68 px, focus order | diff against A's baselines |
| 22 | Phase 27's paint contract unchanged | diff against A's baselines |
| 23 | Must-not-break matrix (§10) passes | walkthrough per sub-phase |

---

## 12. Risks and controls

| Risk | Why it is real | Control |
|---|---|---|
| **CONTROL-B touches every property at once** | 106 field variants, 226 handles, 201 write-side arms, three crates, every undo path | A's baseline captures; the round-trip test; the legacy path deleted only after the new one is green; **no other sub-phase runs concurrently with B** |
| **A generic `SetFieldCmd` corrupts a rebuilding field** | `TerrainComponent::resolution` rebuilds a heightfield, a collider and a GPU sidecar; a scalar-diff undo would leave them inconsistent, silently | `ChangeScope` declared per field **before** the generic path is routed (B step 2), and gate 6 |
| **Multi-select (F) crosses `EngineContext`** | `selected_entity: Option<Entity>` is load-bearing in 71 places | Primary-selection shim so single-selection call sites keep working unchanged; **F is taken alone**; Godot's synthetic-target design so the inspector is not touched at all |
| **Previews stall the editor** | Measured: 232–260 ms inflate for one 4096² terrain PNG, and the shipped code decodes two per frame on the UI thread | Visible-first prioritisation, decode off-thread, a millisecond budget counted on work performed, disk cache by content hash, **and `assets/terrain/` as the acceptance test rather than a synthetic 2 000-file folder** |
| **Clouds cost more than they are worth** | The frame is GPU-bound and shading-dominated; DOOM got it from 38.4 to 19.9 ms | Measured with `.somtime` against a DOOM-A baseline; **default off until the number is in the record**; quarter-res and TAA reuse from the start |
| **The jitter costs more than the steps it saves** | Toft & Bowles measured exactly that: 2.3 → 7.5 ms, and named cache incoherence | The `.somtime` row is taken with and without the offset, and the offset is cut if it loses |
| **The cloud reconstruction stage breaks with a fast camera** | Epic shipped precisely this regression in UE 5.6 | A named capture case: fast camera, clouds occluded by geometry, at every quality setting |
| **The phase becomes a re-theme** | Every UI session drifts toward colours; Phase 27 exists because of it | Zeta's tokens and Hades' paint contract are frozen in the header; gates 21 and 22 diff against A's captures; §5.2 makes "professional" a list of behaviours rather than a mood |
| **A defect that tests cannot see** | Two of Phase 27's three defects had correct geometry, correct colour and green tests, and were found by a human looking at a screenshot | Captures per sub-phase, at two widths, diffed against A's baselines. §13 is not optional |
| **Scope creep into Phases 27-H/I/J, 28, 34, 35** | Accessibility, cooking, prefabs and scattering all touch these surfaces | §3 and §14; **CONTROL does not absorb Phase 27's remaining sub-phases** |
| **The plan cites something it did not verify** | The 2026-08-17 draft did this twice — Godot per-project settings, and the Nubis weather-map channels | §16 records confidence per source; anything unverified is labelled in place |
| **"Looks the same to me"** | Same failure DOOM's fidelity rule named | Captures; a sub-phase claiming no visual change proves it |

---

## 13. Evidence plan

`dev records/phase CONTROL/` — created by CONTROL-A.

```
CONTROL-A_reachability.md              the tables; regenerable; updated by every sub-phase
CONTROL-A_census.md                    the hand-wiring count, one row per sub-phase
CONTROL-A_baseline/*.png               every editor surface, two widths, before
CONTROL-A_terrain_open.somtime         the shipped thumbnail stall, measured
CONTROL-<x>_<surface>.png              after, same viewpoints
CONTROL-C_terrain_open.somtime         the same folder, after
CONTROL-E_<route>.png                  one per drag route
CONTROL-M_clouds.somtime               cloud pass, with and without jitter, vs DOOM-A
CONTROL-M_{clear,scattered,overcast,storm}.png
CONTROL-M_fastcam_occluded.png         the UE 5.6 regression case
CONTROL-N_weather_sequence/*.png       the chain, one capture per stage
README.md                              every number, as phase DOOM/README.md does
```

Captures use `SOMNIUM_CAPTURE_UI_PNG` (the only capture that shows chrome) with
`SOMNIUM_CAPTURE_FRAME` and `SOMNIUM_CAPTURE_QUIT=1`. Scene captures use the
display path so an A/B measures the scene. All after tone mapping. Timing uses
the `.somtime` harness; **do not overwrite the `DOOM-A_*` baselines.**

**Two widths, every time**: 1920×1080 and the redline minimum. Phase 27's
overflow and stacking rules only fail at the narrow one.

---

## 14. Left open, deliberately

Stated here so a later session does not read their absence as an oversight.

**Inherited and still open:**

- **Text shaping and bidi (`cosmic-text`)** — Zeta-H's and Hades' shared item.
  Large, orthogonal, blocks nothing in §8.
- **AccessKit.** Godot shipped screen-reader support in 4.5 after roughly **two
  years of work**, and shipped it *experimental*, covering the Project Manager,
  standard controls and the inspector only. A retained-mode custom toolkit gets
  zero platform accessibility for free, so that is the honest order of
  magnitude for Somnium too. **The cheap half is taken here** — CONTROL-A1's
  arrow traversal, focus-into-view, modal focus trap and return, all of which
  WCAG 2.4.3 requires and none of which needs AccessKit. The expensive half
  stays open with a number attached instead of an intuition.
- **Zeta-I's remainder**: token/raw-literal lints, the licence and icon-manifest
  checks, the component gallery scene, golden-screenshot diffing. CONTROL-A's
  baseline captures are the manual version of the last one.
- **Phase 27's remaining sub-phases** — 27-D's backdrop blur (needs `COPY_SRC`),
  27-F's monogram, optical ladder, `.ico` and splash, and 27-H/I/J. **CONTROL
  does not absorb them.** If Phase 27 is to be closed, Phase 27 closes it.

**Considered during this expansion and deliberately not taken:**

- **Stride's four-assembly Quantum split.** The ideas transfer (§6.2.3); the
  architecture is sized for a third-party plugin ecosystem Somnium does not have.
- **Defold's reactive graph.** The right answer in a language with cheap
  memoised closures over a dependency graph. Esoterica's rules registry is the
  static-language version and is what CONTROL-B adopts.
- **s&box's scored editor resolution.** Unnecessary: `FieldType` is a closed
  enum, so exact match is total and there can be no ties. Recorded so nobody adds
  a scoring system to a table that cannot need one.
- **Unreal's Property Matrix.** CONTROL-F's multi-edit covers the actual need;
  a table of thousands of objects × properties is a sub-phase of its own.
- **Unity's Presets system.** A material preset library is the same mechanism and
  would be welcome, but it belongs after CONTROL-D has shipped one asset type.
- **NeoAxis's `Reference<T>`** — every property being *either* a literal *or* a
  path binding to another object's property, rendered in one row. It is the
  mechanism that lets NeoAxis share one inspector between hand-authored and
  node-graph-driven values. Genuinely interesting; it is also a data-model change
  and belongs to whichever phase introduces bindings, not this one.
- **A shader/material node graph.** §3.
- **A full docking platform.** §3.
- **Prefabs (34), scattering rules (35), sequencer (36), cooking (28).**
- **The 2023 voxel Nubis.** §6.3.
- **Screen-space raindrops on the lens.** Cheap, tempting, and a look change
  nobody asked for.

---

## 15. Start checklist

1. Read this file, all of it.
2. Read `phase_27.md` §6, §12 and **§18** — especially the sixth, seventh and
   tenth passes, which are the three defects a human found by looking at the
   screen, and the reason §13 exists.
3. Read `phase_26_Zeta.md` §8A and §9; `phase_26.md` §3 and §14.
4. Read `context.md` §8, §16, §17.6, §17.18–17.20, §18, and the `NEXT:` line.
5. Read `crates/somnium_ecs/src/reflect.rs` and
   `crates/somnium_core/src/reflect_registry.rs` **end to end.** The phase is
   unreadable without them.
6. Read `crates/somnium_ui/src/editor/inspector.rs`,
   `crates/somnium_ui/src/editor_event.rs`, and `InspectorHandles` at
   `crates/somnium_ui/src/lib.rs:131`. The three things CONTROL-B shrinks.
7. Read `crates/somnium_ui/src/thumbnail.rs` and then re-read §4.2. Its module
   docs state a premise this repository falsifies.
8. **Do CONTROL-A.** Do not write a widget first. Do not restart at 26-A. Do not
   repaint anything Hades painted.
9. Update `context.md` and `ATTRIBUTION.md` after every sub-task, per
   `user_profile.md`.

**A note on running tests on this machine.** `cargo test --workspace` produces
LNK1104 failures that are OneDrive file locks, not code. Use `-j 1`.

---

## 16. Research sources and confidence

Recorded because the 2026-08-17 draft of this file asserted two things it had not
verified, and both are corrected in place above. The rule going forward: **a
citation names what was read.**

**Read directly from `example_repo` for this plan (highest confidence).**
Godot `editor/gui/editor_spin_slider.cpp`,
`editor/inspector/editor_resource_preview.cpp`,
`editor/inspector/multi_node_edit.cpp`,
`editor/settings/editor_command_palette.h`,
`editor/inspector/editor_inspector.h` ·
Fyrox `editor/src/asset/preview/cache.rs`, `editor/src/asset/mod.rs`,
`fyrox-core/src/reflect/field.rs`, `fyrox-ui/src/inspector/editors/` ·
Flax `Source/Editor/GUI/Drag/DragHelper.cs`, `DragHandlers.cs` ·
Unreal `Editor/UnrealEd/Classes/ThumbnailRendering/ThumbnailRenderer.h`,
`ThumbnailManager.h`.

**Surveyed for this plan, file paths cited in §6.2 (high confidence).** Stride,
Defold, Unity (`UnityCsReference-master`), s&box, Solers, Lumina, rbfx, NeoAxis,
Overload, Wicked, Esoterica, Falco, Luanti, plus the triage of Korge, Panda3D,
jMonkeyEngine, Babylon.js, Raylib, Ren'Py, Haxe and mach.

**Measured in this repository on 2026-08-22 (highest confidence).** Every count
in §1 and §4 · the `assets/` composition · the PNG inflate timings, which are a
**lower bound** on decode because unfiltering and downscaling sit on top and were
not measured.

**Verified from a primary web source.** Godot 4.5, 4.6 and 4.7 release posts ·
Unreal's Viewport Toolbar documentation · Unity 6 "What's new" ·
Unity `AssetPreview.GetAssetPreview` docs · Godot `EditorResourcePreview` class
reference · Guerrilla's Nubis abstract pages (2015, 2017, 2022, 2023) ·
Toft & Bowles arXiv:1609.05344 · Unreal's Volumetric Cloud Component docs and
the 5.6 regression forum thread · Figma's numeric-field help page.

**Not verified — treat with care, and re-check before quoting.**

- **Blender's number-field conventions.** `docs.blender.org` refused every
  fetch. The `Ctrl`-snaps / `Shift`-precision / soft-vs-hard-limit /
  vertical-multi-field-drag description in §5.2 is assembled from search
  snippets of the official manual. It agrees with Godot's implementation, which
  *was* read, so the risk is low — but if the manual is quoted verbatim
  anywhere, open it first.
- **Lagarde's "Water drop 3a/3b".** `seblagarde.wordpress.com` refused every
  fetch. §6.3's five points come from fxguide's summary, which attributes them to
  Lagarde. The model is well known and the summary is coherent, but the original
  posts should be opened before CONTROL-N implements from them.
- **The Nubis 128³/32³ noise resolutions and the three-channel weather map.**
  Third-party reimplementations, not Guerrilla's slides — the PDFs exceeded the
  fetch limit. §6.3 says so, and CONTROL-M treats its resolutions and channels as
  **Somnium's design decision**.
- **Unreal 5.7 editor changes.** Could not be verified at all. Nothing in this
  plan depends on them; do not cite 5.7 specifics from this document.
- **Unity's multi-object-editing docs.** The current URL 404'd; the text quoted
  in §5.2 comes from an older archived copy of the same page. The convention is
  stable and corroborated by the `EditorGUI.showMixedValue` API docs.

**Corrected in place from the 2026-08-17 draft.**

1. "Godot 4.6 shipped per-project setting overrides" — **false.** It is an open
   proposal plus a third-party tool. §5.1.
2. The Nubis weather map "carrying coverage / cloud type / precipitation" cited
   as Guerrilla's — **unverified.** §6.3.
3. "~20 ECS components" — **11** real component types in `somnium_core`; the
   earlier count included ECS test fixtures. §4.
4. "96 environment variables", "48 `EditorEvent` variants" — **97** and **58**. §4.
5. "Content Drawer thumbnails: 0" — Phase 27-G shipped image thumbnails. §4.7.
6. Wetness attributed to Remedy — there is no known Remedy wetness talk; the
   canon is Lagarde, whose production context was DONTNOD. §6.3.
7. "`ATTRIBUTION.md` §13E (new section)" — **that letter was taken while this
   plan sat unstarted.** §13E is Phase 27-A/27-B and §13F is Phase 27-C/D/E.
   CONTROL's section is **§13G**, and the reconnaissance stub for it was written
   on 2026-08-22 alongside this expansion.

---

# Appendix A — Implementation reference

*Added 2026-08-23. §§0–16 are the plan and the argument; this appendix is the
part a cold session needs in order to start typing. Nothing here changes a
decision above — where the two disagree, §§0–16 win and this appendix is stale.*

## A.1 Orientation: read these nine things, in this order

§0 lists what to read for *context*. This is what to read for *code*. A session
that skips it will write a plausible `SetComponentField` handler that does not
compile. Budget about two hours.

| # | Path | Read for | Approx |
|---|---|---|---|
| 1 | `crates/somnium_ecs/src/reflect.rs` | `StableId(&'static str)`, `FieldId(pub u16)`, `ReflectValue`, `FieldType`, `FieldSchema`, `FieldFlags`, `ComponentSchema`. **Seam 1 is entirely this file's vocabulary.** | 1,369 ln |
| 2 | `crates/somnium_core/src/reflect_registry.rs` | The twelve schemas and the `component_schema!` call shape (registration at `:342–353`) | 713 ln |
| 3 | `crates/somnium_ui/src/editor_event.rs` | The `EditorEvent` enum CONTROL-B collapses | 549 ln |
| 4 | `crates/somnium_ui/src/lib.rs:131` | `InspectorHandles` — the 245-line struct of 226 bare `NodeHandle`s that CONTROL-B deletes | — |
| 5 | `crates/somnium_ui/src/editor/inspector.rs` | The hand-built Details panel being replaced | 839 ln |
| 6 | `crates/somnium_core/src/editor_commands.rs` | The undo stack `SetFieldCmd` joins | 1,146 ln |
| 7 | `crates/somnium_ui/src/message.rs` | `UiMessage { handled, destination, direction, data: Box<dyn Any + Send> }`, `WidgetMessage`. **Seam 5 changes this** | 113 ln |
| 8 | `crates/somnium_ui/src/widgets/property_row.rs` + `numeric_field.rs` | What a property editor renders into today | 1,075 ln |
| 9 | `crates/somnium_core/src/scene_schema.rs` | Round-trip, and the silent-drop path §6.2.3 found | 1,055 ln |

## A.2 The seam, worked end to end — one field, all the way through

Seam 1 is stated abstractly in §7. Here is `WaterComponent::roughness` making
the whole round trip, so a cold session can see where each piece lands.

**1. Declaration** — `reflect_registry.rs`, inside the existing macro:

```rust
component_schema! {
    stable_id: "somnium.Water",
    display_name: "Water",
    version: 3,
    fields: {
        /// Microfacet roughness of the water surface.
        /// Lower is glassier; 0.02 is a dead calm lake.
        roughness: F64 = 0.08, min: 0.0, max: 1.0,
            step: 0.005, precision: 3, soft_max: 0.35,
            unit: "", group: "Surface",
    }
}
```

The doc comment is the tooltip — §7's Fyrox rule, and **the only route**; there
is no competing `doc:` argument to drift from it.

**2. The registry** turns that into a `FieldSchema` (fields exactly as
`reflect.rs:345` declares them, plus §7's additions):

```rust
FieldSchema {
    name: "roughness",
    id: FieldId(4),                 // declaration order; wire-stable
    ty: FieldType::F64,
    default: ReflectValue::F64(0.08),
    min: Some(0.0), max: Some(1.0),
    flags: FieldFlags::EDIT | FieldFlags::SAVE,
    // --- added by CONTROL-B ---
    step: Some(0.005),
    soft_min: None, soft_max: Some(0.35),
    precision: Some(3),
    unit: "",
    doc: "Microfacet roughness of the water surface. Lower is glassier; \
          0.02 is a dead calm lake.",
    display_name: None,
    group: Some("Surface"),
    advanced: false,
    read_only: false,
    scope: ChangeScope::Field,       // §6.2.1(a), rbfx AttributeScopeHint
}
```

**3. The panel builds itself.** No `InspectorHandles` field, no `IF::` arm:

```rust
// crates/somnium_ui/src/editor/inspector_gen.rs  (new in CONTROL-B)
fn build_component(ui: &mut UserInterface, schema: &ComponentSchema,
                   values: &ReflectObject, editors: &PropertyEditorRegistry)
    -> Vec<(FieldId, NodeHandle)>
{
    let mut rows = Vec::new();
    for group in group_fields(schema) {                 // Stride's virtual category node
        let section = ui.build_section(group.name);
        for field in group.fields {
            if !field.flags.contains(FieldFlags::EDIT) { continue; }
            if rules_for(schema.stable_id).is_hidden(field.id) { continue; } // Esoterica
            let editor = editors.for_type(&field.ty);   // table lookup, not a match
            let handle = editor.build(&mut PropertyEditorCtx {
                ui, parent: section, schema: field,
                value: values.get(field.id).unwrap_or(&field.default),
            });
            rows.push((field.id, handle));
        }
    }
    rows
}
```

`rows` — a `Vec<(FieldId, NodeHandle)>` per component — is what replaces the
226-field `InspectorHandles` struct. It is built, not maintained.

**4. The user drags the slider.** The widget emits a `UiMessage`; the editor
maps it to **one** event:

```rust
EditorEvent::SetComponentField {
    entity: 17,
    component: StableId::new("somnium.Water"),
    field: FieldId(4),
    value: ReflectValue::F64(0.11),
    live: true,          // mid-drag: apply, do not push undo
}
```

**5. One handler in `app.rs`** replaces 201 `IF::` arms:

```rust
EditorEvent::SetComponentField { entity, component, field, value, live } => {
    let schema = self.registry.get(component).ok_or(Err::UnknownComponent)?;
    let fs = schema.field(field).ok_or(Err::UnknownField)?;
    fs.validate(&value)?;                       // reflect.rs:368 — already exists

    if live {
        schema.write_field(&mut self.world, entity, field, value);   // no undo entry
    } else {
        let before = (schema.read_field)(&self.world, entity, field)
            .unwrap_or(fs.default.clone());
        self.undo.push(SetFieldCmd::new(entity, component, field, before, value, fs.scope));
    }
}
```

**6. Undo, and the part that is easy to get wrong.** `SetFieldCmd` chooses its
strategy from `scope` — §6.2.1(a) is the reason this field exists:

```rust
impl SetFieldCmd {
    fn new(/* .. */, scope: ChangeScope) -> Self {
        match scope {
            // roughness: store two scalars, replay them
            ChangeScope::Field     => Self::Scalar { before, after },
            // TerrainComponent::resolution: the write rebuilds a heightfield,
            // a collider and a GPU sidecar. A scalar diff silently corrupts.
            ChangeScope::Component |
            ChangeScope::Entity    => Self::EntitySnapshot(snapshot_entity(world, entity)),
            ChangeScope::Scene     => Self::SceneSnapshot(snapshot_scene(world)),
        }
    }
}
```

**7. Save** goes through the existing `scene_schema` path unchanged, because the
value was always a `ReflectValue` and the field was always addressed by
`(StableId, FieldId)`. That is the whole point of the seam: **the editor stopped
being a special case.**

**Drag coalescing.** A drag emits `live: true` per mouse-move and one
`live: false` on release. Do not push an undo entry per move (undo becomes
unusable) and do not skip the intermediate writes (the viewport stops
responding). One entry per gesture, `before` captured on mouse-*down*.

## A.3 The property-editor registry — the table that replaces the match

§7 gives the trait. This is the population, and the two rows that matter most
are the last two:

```rust
pub struct PropertyEditorRegistry { editors: Vec<Box<dyn PropertyEditor>> }

impl PropertyEditorRegistry {
    pub fn standard() -> Self {
        let mut r = Self::default();
        r.add(BoolEditor);        // FieldType::Bool    -> CheckBox
        r.add(F64Editor);         // F64                -> NumericField (step/precision/unit)
        r.add(I64Editor);         // I64                -> NumericField, integer mode
        r.add(StrEditor);         // Str                -> TextBox
        r.add(Vec2Editor);        // Vec2               -> 2 x NumericField
        r.add(Vec3Editor);        // Vec3               -> 3 x NumericField
        r.add(Vec4Editor);
        r.add(QuatEditor);        // Quat               -> euler triple, quat under the hood
        r.add(ColorEditor);       // Color              -> swatch + ColorPicker (26-F Iris)
        r.add(EntityEditor);      // Entity             -> picker + drop target (Seam 3)
        r.add(AssetEditor);       // Asset              -> picker + drop target, kind-masked
        r.add(EnumEditor);        // Enum(&[..])        -> ComboBox
        r.add(ArrayEditor);       // Array(Box<..>)     -> add/remove/reorder, recurses
        r.add(UnsupportedEditor); // <- LAST. Renders a visible "unsupported type" row.
        r
    }

    /// First match wins, so registration order is the priority order and a
    /// custom editor is registered *before* the standard one it overrides.
    pub fn for_type(&self, ty: &FieldType) -> &dyn PropertyEditor {
        self.editors.iter().find(|e| e.accepts(ty)).unwrap().as_ref()
    }
}
```

`UnsupportedEditor` is not defensive programming, it is the mechanism that stops
a schema and a panel diverging silently — §7 says so and it is worth restating
here because it is the row a reviewer deletes as "unreachable."

`ArrayEditor` recursing into `for_type(inner)` is what makes `Array(Box<Color>)`
work with no code: a gradient's stop list is an array of colours and it should
fall out of the registry, not be special-cased.

**Custom editors** are the same trait registered earlier — Esoterica's
`CustomEditor` metadata key and Flax's `CustomEditors/` both work this way. A
curve field (CONTROL-K) registers a `CurveEditor` that `accepts` a new
`FieldType::Curve`, and every component that declares one gets it for free.

## A.4 `EditingRules` — conditional visibility, per §6.2.1(c)

Esoterica's tri-state, kept tri-state on purpose so "no opinion" and "editable"
are distinguishable:

```rust
pub enum Editability { Editable, ReadOnly, Unhandled }

pub trait EditingRules: Send + Sync {
    fn is_read_only(&self, _f: FieldId, _v: &ReflectObject) -> Editability { Editability::Unhandled }
    fn is_hidden(&self, _f: FieldId, _v: &ReflectObject) -> bool { false }
    fn name_override(&self, _f: FieldId, _v: &ReflectObject) -> Option<&'static str> { None }
}

// Registered per StableId, evaluated per frame against current values.
struct PostProcessRules;
impl EditingRules for PostProcessRules {
    fn is_hidden(&self, f: FieldId, v: &ReflectObject) -> bool {
        match field_name(f) {
            "bloom_intensity" | "bloom_threshold" => !v.bool("bloom_enabled"),
            "dof_focus_distance" | "dof_aperture" => !v.bool("dof_enabled"),
            n if n.starts_with("physical_")       => !v.bool("use_physical_camera"),
            _ => false,
        }
    }
}
```

This is **Rust, not a DSL**, and §6.2.1(c) explains why: encoding this in
`component_schema!` means inventing an expression grammar inside a declarative
macro, and Luanti's `Requires:` line only works because it is restricted to
conjunctions of booleans. The moment a condition needs arithmetic, a text schema
loses. Per-frame re-evaluation is cheap because it runs over the visible panel,
not the world.

## A.5 File-by-file change map

`+` new, `~` modified, `-` deleted.

**Track 0**
```
+ tools/reachability/                     the §4 table, generated not typed
~ crates/somnium_ui/src/message.rs        Seam 5: modifiers on WidgetMessage
~ crates/somnium_ui/src/ui.rs             populate modifier state at the winit boundary
+ crates/somnium_core/src/commands.rs     Seam 6: the one command registry
~ crates/somnium_ui/src/widgets/command_palette.rs   15 hardcoded entries -> registry
~ crates/somnium_ui/src/widgets/menu.rs + context_menu.rs   -> registry
```

**Track 1**
```
~ crates/somnium_ecs/src/reflect.rs       FieldSchema gains 10 optional fields + ChangeScope
~ crates/somnium_core/src/reflect_registry.rs   macro accepts them; doc from #[doc]
+ crates/somnium_ui/src/editor/inspector_gen.rs
+ crates/somnium_ui/src/editor/property_editors/{mod,bool,num,vec,color,asset,entity,enum,array,unsupported}.rs
+ crates/somnium_ui/src/editor/editing_rules.rs
~ crates/somnium_ui/src/editor_event.rs   +SetComponentField; -106 InspectorField,
                                          -9 ColorField, -27 PostFxToggle (at B's exit)
~ crates/somnium_ui/src/lib.rs            InspectorHandles (245 ln, 226 fields) deleted
~ crates/somnium_core/src/app.rs          201 IF:: arms -> 1 handler
~ crates/somnium_core/src/editor_commands.rs    +SetFieldCmd, scope-aware
~ crates/somnium_asset/src/lib.rs         +AssetDb, +AssetId, +JobRegistry callers
+ crates/somnium_core/src/jobs.rs         JobRegistry  ** see A.6 **
~ crates/somnium_ui/src/thumbnail.rs      448 ln, synchronous -> jobs (§4.2's 232-260 ms)
```

**Tracks 2–3**
```
+ crates/somnium_ui/src/widgets/curve_editor.rs, gradient_editor.rs   (CONTROL-K)
~ crates/somnium_ecs/src/reflect.rs       +FieldType::Curve, +FieldType::Gradient
+ crates/somnium_core/src/time_of_day.rs                              (CONTROL-L)
+ crates/somnium_renderer/src/pass/clouds.rs + shaders/clouds.wgsl    (CONTROL-M)
+ crates/somnium_renderer/src/pass/weather.rs                         (CONTROL-N)
+ crates/somnium_renderer/src/pass/decal.rs                           (CONTROL-O, optional)
```

## A.6 Two forward-compatibility notes for Phase MORROWIND

§9.1 fixes the order: CONTROL completes, MORROWIND follows. Two small choices
here make that handoff cheap, and both cost nothing now.

**1. `JobRegistry` will be promoted, so build it to be moved.** Seam 2 puts a
`JobRegistry` in `somnium_core`. MORROWIND-B extends it into a `somnium_jobs`
crate with `Priority`, `deadline` and a budgeted main-thread
`drain_completions(budget)`; MORROWIND's §10 and §11 both forbid a second thread
pool, so this is a **move, not a fork**. Two things make the move a rename
rather than a rewrite:

- Keep the public surface narrow — `submit`, a handle, cancellation, a progress
  query. Everything else `pub(crate)`.
- **Give every submitted job a `&'static str` name at the call site**, even
  though nothing consumes it yet. MORROWIND turns names into Phase 29 profiler
  zones, and retrofitting them across call sites later is tedious and always
  ends up incomplete.

**2. Seam 1's `(StableId, FieldId, ReflectValue)` becomes the prefab override
vocabulary.** MORROWIND-O's `Patch` is literally that triple plus a nesting
path, which is why a property edit inside a prefab instance is an override *by
construction*. Two implications for choices made here:

- `ChangeScope` is what will keep an override on a rebuilding field
  (`TerrainComponent::resolution`) from silently corrupting an instance. It is
  already justified for undo (§6.2.1a); it is doubly justified now.
- **`scene_from_json`'s silent drop of unknown components and fields (§6.2.3) is
  worth fixing inside CONTROL-J**, not deferring. Prefabs multiply a data-loss
  path by the instance count, and the fix is much smaller before the format
  gains nesting than after.

## A.7 How to verify a sub-phase is actually done

Beyond §11 — the specific check that catches the specific way each is usually
faked.

| Sub-phase | The cheat | The check |
|---|---|---|
| A | A table exists, typed by hand | Delete it and regenerate; the diff must be empty |
| A1 | Modifiers plumbed, no widget reads them | Ctrl+click multi-select **and** Alt-drag precision-scrub both work; §4.3 says both are inexpressible today |
| A2 | A registry exists beside the four old lists | The four lists are **deleted**. `grep` for the palette's array-index dispatch: zero hits |
| B | Generated rows for some components, hand-built for the awkward ones | **`InspectorHandles` is deleted** and `grep -c "IF::" app.rs` returns 0. Partial migration is the failure mode here |
| B | Generated rows exist but a new component still needs panel code | Add a throwaway component with one field of every `FieldType`; it must appear with no UI code written |
| C | Thumbnails are async, opening `assets/terrain/` still hitches | Open it (60 PNGs, 1.17 GB) and watch the frame graph. §4.2 measured 232–260 ms of zlib inflate on the largest file alone |
| D | A material editor that is really a material viewer | Import a glTF, edit one of its materials, save, reopen. §8's own exit criterion |
| E | Drags work, failures are silent | Drag something invalid onto something; a *reason* must be shown, not a no-op |
| F | Multi-select renders, edits apply to one | Select three entities, edit a shared field, check all three |
| G | Camera works, gizmos regressed | Golden image, if MORROWIND's GHOSTFENCE golden-image runner exists by then; otherwise a diffed capture |
| H | Settings save, environment variables still win silently | Set a `SOMNIUM_*` var **and** the setting to different values; the precedence must be stated in the UI, not just implemented |
| I | Log works, jump-to-source opens the wrong line | Click three entries from three different crates |
| J | Scene round-trips in the happy case | Round-trip a scene while a component is **missing** from the build. §6.2.3's data-loss path lives exactly here |
| K | A curve editor exists; edits need a refresh | §8 names Ultra Dynamic Sky's "Refresh Settings" button as the explicit anti-goal. Drag a keyframe and watch the viewport change |
| L/M/N | It looks good | `.somtime` on both maps, plus the reachability list — clouds with no controls is a renderer feature, not a CONTROL sub-phase |
