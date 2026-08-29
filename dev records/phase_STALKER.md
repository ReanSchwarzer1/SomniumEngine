# Phase STALKER — X-Ray

> *The editor is where a game is made. It is not where a game is shipped.*

> **Codename:** STALKER (GSC Game World, X-Ray / OpenXRay). The name is doing
> work. S.T.A.L.K.E.R. survived because its engine treats shipped resources,
> user data and mods as different layers *and* because its world keeps moving
> beyond the player's immediate bubble. OpenXRay then made compatibility with
> those resources a first-order contract instead of an accident. This phase is
> the same transition for Somnium: from a repository that can run engine demos
> to a product that can build and ship a small, lit, UI-complete, persistent
> world; patch, roll back and mod it; and do all of that without the repository.
>
> **Status:** **PLAN — nothing in tree.** Written 2026-08-29 against `dev` at
> `d19b7c1`. No section below is a claim that a player, package, updater or mod
> loader exists. The measured inventory in §4 describes the current tree; all
> output sizes and timings are deliberately left blank until STALKER-A.
>
> **Position:** the named successor to Phase KENSHI. `phase_KENSHI.md` §14.1
> says explicitly that packaging, distribution and platform targets are *the
> phase after KENSHI*. STALKER therefore starts only after KENSHI-J publishes
> `limits.md` and the fixes that report authorizes have either landed or been
> refused on evidence. MORROWIND-Q/R, MORROWIND-AF, PORTAL's gates and KENSHI's
> deterministic harness are hard prerequisites; §9 gives the exact gates.
>
> **Record:** this file. `dev records/phase STALKER/` is created by STALKER-A,
> not by this draft. Do not invent installers, package manifests, patch-size
> tables, crash reports or clean-machine screenshots before the commands that
> generate them exist.
>
> **References:** the new survey root is
> `C:/Users/adhir/Downloads/GE/example_repo/new and shiny/`. Source is studied
> for architecture, not copied. Permissive sources still require attribution;
> UPBGE is GPL, Nu is noncommercial, OpenXRay contains derivative X-Ray source,
> and Toaster has no root licence in the surveyed snapshot. Those four are
> **pattern-only or rejected**. STALKER-A opens a new `ATTRIBUTION.md` section
> before any adaptation.

**The rule this phase is judged by — the clean-machine rule:**

> A release exists only if a clean machine with no repository, Rust toolchain,
> editor cache or `SOMNIUM_*` environment variables can verify it, launch it,
> update it, survive an interrupted update, roll back, load an old save and
> explain a crash. `cargo run`, copying `target/release`, or launching from the
> asset source tree is not a release. If the proof needs the developer's
> machine, the phase has not shipped.

**Frozen by this phase:**

- The existing runtime look and measured defaults. STALKER may add the explicit
  local-lighting and sprite paths in §8, but they begin opt-in, attach after the
  visibility buffer, and may not retune unrelated passes or reopen a KENSHI
  performance decision.
- `GameApp` remains the game-facing lifecycle interface. The player may host
  it; the build system may compile a tiny bootstrap around it; neither invents
  a second application model.
- `CookedAsset` and `CookManifest` are extended by version, never silently
  reinterpreted. Every v1 artifact in the existing cook tests remains readable.
- The editor keeps a loose-file adapter. The player gains a package adapter.
  There is one content interface at one seam, not separate editor and player
  asset systems.
- Untrusted extension code remains Luau behind the existing command/capability
  boundary. This phase does **not** load arbitrary native DLLs from a mod.
- Old `.somtime` files, old save fixtures and every GHOSTFENCE row remain
  readable. A release pipeline that cannot run the existing gates is a new way
  to hide failures, not a shipping system.

---

## 0. How to use this document (handoff)

Read, in order:

1. This file, especially §4 (what exists), §6 (what the references really
   contribute), §7 (ten seams), §8 (sub-phases) and §9 (gating).
2. [`phase_KENSHI.md`](phase_KENSHI.md) §1.3 and §14.1. STALKER is the shipping
   phase KENSHI deliberately did not start.
3. [`phase_MORROWIND.md`](phase_MORROWIND.md) MORROWIND-Q/R and MORROWIND-AF,
   plus §14.9 and §14.11. The cook, residency and save state are prerequisites;
   native gameplay plugins and live type migration remain out.
4. `crates/somnium_asset/src/cook.rs`, end to end. Its opening comment already
   states that content-addressed blobs plus an atomic manifest leave room for
   live update. This phase fills that room instead of replacing the cook.
5. `crates/somnium_script/src/capability.rs`, especially the documented
   “future mod tier.” STALKER-M is that tier; it must use the existing exhaustive
   command-boundary check.
6. `crates/somnium_core/src/app.rs` (`GameApp`, `EngineContext`) and
   `examples/vvardenfell/src/main.rs`. If the package cannot run the second
   example through public interfaces, the package interface is wrong.
7. The source census in §6 before opening random files under `new and shiny`.
   Nu, Prowl and X-Ray were read below their READMEs for the additions in this
   revision; their licences still control whether only a pattern may be used.

**Authorized work:** new build/player/package/update modules and tools; bounded
local irradiance/reflection volumes; sprite/atlas assets and authoring; typed
persistent actions and scene tools; abstract/detailed simulation; factions,
inventory, trade, dialogue, quests and anomaly-field data; one integrated Zone
slice; narrow adapters in existing crates; CI/release metadata; tests and
generated evidence.

**Not authorized:** a render-graph rewrite, a second UI or scripting runtime,
networking, native mod DLLs, a store client, online crash service, weapon/combat
framework, vehicle stack or an attempt to clone S.T.A.L.K.E.R. as a game.

---

## 1. Executive decision

### 1.1 The finding

Somnium has the expensive middle of a shipping pipeline and neither end.

The tree has a deterministic native asset cook, content hashes, reverse
dependency invalidation, an atomic cooked manifest, residency budgets, a world
partition, a real `GameApp` lifecycle, a sandboxed Luau runtime and a second
public-API consumer in `examples/vvardenfell`. It does **not** have:

- a player binary separated from editor-only code;
- a target/profile model;
- a build stage graph or release manifest;
- dependency-closure export from authored roots;
- an archive/package reader;
- content patch generation, atomic apply or rollback;
- save migration across product versions;
- a local crash envelope with build and content identity;
- a mod manifest, dependency resolver or layered content mount;
- a clean-machine smoke test, SBOM or release licence bundle.

`tools/assetcook` takes four paths and a plan. It can cook assets; it cannot
answer which assets belong to a game, assemble an executable, prove the output
is complete or explain how to update it. `examples/vvardenfell` is an excellent
game-facing acceptance fixture, but it still runs as a workspace member against
source assets. `hello_engine` contains a `PlayerRuntime` name, not a product
boundary.

### 1.2 The decision

Build one release pipeline and one shippable feature slice around the
interfaces that already exist, in nine tracks:

- **Track 0 — CORDON:** draw the line between editor, player and headless host.
- **Track 1 — ROSTOK:** turn roots into a deterministic, verified release.
- **Track 2 — DEAD CITY:** patch, migrate and recover without destroying the
  last working state.
- **Track 3 — WILD TERRITORY:** layered, capability-limited data and Luau mods.
- **Track 4 — YANTAR:** finish local irradiance/reflection volumes and prove the
  baked result survives cook/package/headless paths.
- **Track 5 — 100 RADS BAR:** add a real sprite workflow, composable scene tools
  and safe inspector-wired actions, then use them for product UI.
- **Track 6 — THE ZONE:** simulate distant actors cheaply and hand them to the
  detailed ECS with deterministic, hysteretic transitions.
- **Track 7 — RED FOREST:** add the small game-framework modules needed to make
  that world legible: relations, inventory/trade, facts/dialogue/quests and
  anomaly fields.
- **Track 8 — PRIPYAT:** prove the integrated artifact on machines and paths
  that did not build it.

The phase's product is not an installer UI or a pile of feature checkboxes. It
is a set of deep modules: the five release seams plus lighting environments,
sprite assets, authored actions, simulation LOD and durable game-state
commands. Editor, player, headless verifier, mods and tests meet those same
interfaces. The Zone slice is the integration proof, not a new engine layer.

### 1.3 Why STALKER, and why now

OpenXRay's stated goal is a drop-in replacement that preserves original game
resources while adding optional engine features and better tooling for
modmakers. Its ALife implementation also makes a valuable separation between
objects simulated abstractly and objects realized near the player. Together
those are precisely the disciplines a first Somnium release needs: content
compatibility is a contract, optional layers never corrupt the base, distant
state does not require a rendered entity, and the engine can say which version
of every layer and snapshot it loaded.

The timing is equally deliberate. MORROWIND creates the engine half, PORTAL
makes its gates honest, and KENSHI measures the combined load. Shipping before
those three would freeze unknown behaviour into a public format. Shipping after
them turns their contracts into a product boundary.

### 1.4 Why not the render graph from Myth and Adria

The new survey contains two persuasive render graphs. Myth has a strict SSA
graph with dead-pass elimination and transient aliasing; Adria has automatic
barriers, transient resource pooling and asynchronous compute scheduling.

They do **not** reopen MORROWIND §14.6 or KENSHI §14.5. KENSHI defines the only
evidence that can do that: a measured row indicting pass scheduling. A shipping
phase cannot replace the frame core because a new repository made the design
look attractive. The render-graph findings are recorded in §14.4 for the phase
that receives that evidence, if it ever does.

---

## 2. Goals

1. **One standalone player.** `vvardenfell` runs without linking editor modules
   and without a source-asset directory. The generated game bootstrap is tiny;
   the reusable player implementation lives behind one interface.
2. **One deterministic build.** A `BuildRequest` plus pinned inputs produces a
   canonical release manifest and byte-identical cooked/package payloads on
   repeated runs. Any executable nondeterminism is isolated and reported rather
   than allowing the whole claim to become vague.
3. **Only reachable content ships.** Starting scenes, explicit resources,
   localisation catalogues and package declarations are roots. The cook's
   dependency graph computes their closure. Missing and orphaned assets are
   diagnostics, not runtime surprises.
4. **Every byte is attributable and verifiable.** The release manifest records
   product/build id, target triple, engine commit, toolchain, profile, package
   hashes, asset roots, configuration, licences and SBOM. Verification works
   without launching the game.
5. **Updates are transactions.** Download/stage, verify, switch one pointer,
   retain the previous manifest, recover after interruption. A failed update
   never modifies the only working copy.
6. **Saves migrate or fail safely.** Every durable save has a schema version and
   build/content identity. Migrations are ordered, deterministic, tested with
   fixtures and never overwrite the source save before success.
7. **Crashes are explainable locally.** A bounded crash envelope records build
   id, content/mod manifests, adapter/capability report, last log/profiler
   breadcrumbs and the failing phase without silently uploading user data.
8. **Mods are layers, not writes into the install.** A manifest names identity,
   version, dependencies, conflicts, content roots and requested script
   capabilities. Resolution is deterministic; untrusted code stays in Luau.
9. **Two targets are real.** Windows x86-64 is the clean-machine release target.
   Linux x86-64 must at least compile and run the headless packaged smoke in CI;
   it becomes a graphical tier only when a real machine proves it.
10. **Release evidence is generated.** Build report, closure report, package
    map, patch report, migration matrix, crash fixture, licence/SBOM output and
    clean-machine transcript come from commands, not prose.
11. **Local lighting is authorable and portable.** Bounded volumes provide
    diffuse SH irradiance and prefiltered specular environment data with
    deterministic overlap/blend rules. Captures are cooked assets, not hidden
    GPU/editor state, and the fallback path is explicit.
12. **Sprites are assets, not UV folklore.** A texture may produce stable sprite
    sub-assets with rect, pivot, pixels-per-unit, border and optional tight
    geometry. The same asset feeds runtime UI, world sprites and billboard VFX.
13. **UI events are data with a safe boundary.** Authors can wire button,
    trigger and dialogue-choice events to registered commands with typed
    arguments, undo, validation and capability checks—never arbitrary Rust
    reflection or a serialized function pointer.
14. **The world has two simulation costs.** Distant actors use deterministic,
    budgeted abstract updates; near actors use the existing detailed ECS/AI.
    Promotion and demotion preserve identity and use hysteresis so the boundary
    cannot flap.
15. **The shipped slice is coherent.** Relations influence trade and dialogue;
    inventory and quest facts survive save/load and simulation LOD; an authored
    anomaly field drives graphics, audio, detector UI and an item reward. The
    slice ships through the same packages and mod resolver as the engine.

---

## 3. Non-goals

1. **No Steam, Epic, console or mobile SDK integration.** The phase produces a
   portable release directory and deterministic archives. Store wrappers and
   platform certification are separate projects with external credentials.
2. **No Web/WASM target.** Myth and Laya prove the architectural possibility;
   Somnium's Jolt, native dialogs, filesystem assumptions, ray-query tiers and
   thread model need their own audit. A fake target card is worse than none.
3. **No networking, launcher account system, CDN or cloud saves.** Patches are
   files applied from a local source. Transport is deliberately outside the
   update transaction.
4. **No arbitrary native gameplay plugins.** Prowl supports managed/native
   plugins; Somnium does not adopt that risk. MORROWIND §14.9 already chose
   Luau for isolation, and this phase extends its package/capability model.
5. **No DRM, anti-cheat, encryption-as-security or secret key embedded in the
   player.** Package hashes establish integrity, not secrecy.
6. **No installer artwork or auto-updater GUI before the transaction is proven.**
   CLI and a minimal editor surface are enough. The UI work in this phase is
   authoring and game UI; it does not decorate an unsafe updater.
7. **No feature work outside the named slice.** Local lighting volumes, sprite
   assets, authored actions, scene tools, simulation LOD and the RED FOREST
   modules are authorized because they meet in the packaged proof. 3D Gaussian
   splats, XR, vehicles, a renderer rewrite, new audio effects, terrain holes,
   morph targets and broad material-lobe expansion remain out.
8. **No binary delta algorithm in v1.** Content-addressed package replacement
   already makes unchanged blobs free. A bsdiff-like layer is allowed only
   after STALKER-I measures package replacement as the real patch-size wall.
9. **No live migration of Rust object graphs.** Save migration is bytes-at-rest.
   MORROWIND §14.11's running type migration remains a different problem.

---

## 4. The audit — measured 2026-08-29

Measured on `dev` at `d19b7c1`. “Absent” below means no source module, manifest
or tool in the Somnium tree matched the capability after direct inspection and
the graphify query; it does not mean the underlying libraries are incapable.

### 4.1 What can be reused

| Existing module | What is already true | STALKER use |
|---|---|---|
| `somnium_asset::cook` | `CookedAsset` has magic, format and cooker versions, sorted dependency hashes, payload hash validation, bounded decode and deterministic manifest output. | Package payload and release closure; do not invent a second artifact format. |
| `AssetDependencyGraph` | Validates missing nodes/cycles, computes a topological order and reverse affected closure. | Add forward root closure and generated reachability evidence. |
| `somnium_asset::residency` | Stable handles, state transitions, a single budget owner and snapshots. | Package-backed assets enter through the same handles; install/update never reaches renderer internals. |
| `GameApp` / `EngineContext` | A small game lifecycle already used by two examples. | Standalone player hosts this interface; editor and player remain adapters. |
| `somnium_script` | Snapshot-in/commands-out, stable asset/instance ids and deterministic command order. | Packaged scripts use the same runtime and identity. |
| `Capabilities` | Unknown bits are dropped; every command names its requirement; `SANDBOXED` is documented as the future mod default. | STALKER-M serializes and enforces that existing model per mod package. |
| GHOSTFENCE + `.somtime` | Existing generated gates and measurement format. | Build/release invokes them and archives results; no new green path. |
| `vvardenfell` | Second public-API consumer exercises UI, cook/residency, streaming, animation and scripts. | Becomes the packaged acceptance product. |

### 4.2 What is absent

| Capability | Current state | Consequence |
|---|---|---|
| Player/editor dependency boundary | No `somnium_player` crate or dependency rule. | An example binary is not proof that editor-only code cannot leak into a build. |
| Build target/profile | No `BuildTarget`, `BuildProfile`, target registry or build settings document. | Platform conditionals will spread across UI and scripts. |
| Build orchestration | No staged build graph, cancellation, structured diagnostics or partial-output policy. | A failed build leaves no durable explanation of what ran or what is safe to keep. |
| Rooted asset closure | Cook plans list assets directly; there is no starting-scene/resources root policy. | Either unused content ships or a runtime reference fails. |
| Package mount | Cook writes per-asset files and a manifest; there is no archive reader or layered source. | Player still depends on a directory shaped like a cache. |
| Release identity | No product/build/content id binds executable, packages, saves, logs and crash data. | A report cannot establish which bytes produced it. |
| Patch/rollback | Explicitly left for Defold-style live update by `cook.rs`; not implemented. | Updating requires replacing files in place with no recovery contract. |
| Save migration | MORROWIND-AF is still open in the current README. | First public save format would become accidentally permanent. |
| Crash envelope | Logs and profiler data exist, but no bounded local incident bundle. | “It crashed” loses the build/content/mod context needed to reproduce it. |
| Mod layer | No package manifest/resolver/mount order despite the script capability hook. | Mods would become manual file replacement, making verification and rollback impossible. |
| Release compliance | No generated SBOM, third-party notice bundle or shipped-file licence gate. | A technically valid build may be legally undistributable. |
| Local lighting environments | Phase 24Q's old 4×4×4 probe controls now map to MORROWIND-AB's portable DDGI volume; there is no separate bounded diffuse/specular environment asset, overlap rule, capture cook or per-object selection. | Indoor/outdoor transitions and localized reflections cannot be authored or reproduced in a package. |
| Sprite workflow | UI can draw images/inline sprites, but there is no stable sprite sub-asset, atlas slicing, pivot/border authoring, tight mesh or world-sprite component. | UI skins, icons, billboards and 2D content duplicate UV rectangles and cannot share one import contract. |
| Authored actions / scene tools | CONTROL supplies inspectors, commands and fixed viewport tooling; there is no serialized typed event binding or component-scoped scene-tool seam. | Product UI and new authoring modes either hard-code callbacks or deepen editor/UI god nodes. |
| Simulation LOD | World partition streams space, but no abstract actor state, scheduled budget or realized/abstract handoff exists. | Every living actor must be fully spawned or cease to exist outside the loaded bubble. |
| Game-framework slice | No faction/relation ledger, gameplay inventory/equipment/trade, durable story facts/dialogue/quests or reusable anomaly-field model was found. | Runtime UI and save/mod systems have no integrated product workload to prove them. |

### 4.3 The architectural debt this phase must not create

The tempting design is a large `export_game()` function that shells out to
Cargo, copies a directory, zips assets and reports progress to the editor. That
would be a shallow module: its interface would inherit every platform,
filesystem, process and asset-policy detail, and tests would have to mock the
world.

STALKER instead places ten seams (§7). The build planner returns operations;
the executor performs them. Content consumers read one interface; directory,
package and layered adapters satisfy it. Update and migration are pure plans
before they touch disk. The editor is a caller, never the implementation.

---

## 5. What “shipped” means

There are four distinct artifacts. Conflating them is how release systems
become impossible to reason about.

1. **Cooked asset:** one versioned, integrity-checked build representation.
   Already exists.
2. **Content package:** a deterministic set of cooked assets with an index.
   It knows nothing about executables or install locations.
3. **Release:** player + packages + configuration + notices + SBOM, bound by a
   release manifest and build id.
4. **Patch:** a transaction from one release manifest to another. It contains
   new blobs and a switch plan; it is not a second release format.

The player trusts no filename by itself. It opens the release manifest, checks
its supported format, verifies package indexes and hashes, resolves optional
mod layers, then constructs the existing asset resolver. Development may turn
verification down for iteration; release builds may not.

### 5.1 Identity

Three ids, each answering one question:

- **Product id:** which game is this? Stable across releases.
- **Build id:** which executable/toolchain/configuration produced this release?
  Hash of canonical build inputs, not a timestamp.
- **Content id:** which ordered base package manifests are mounted? Hash of
  package indexes. Mods are recorded separately so a base install remains
  identifiable.

Saves and crash envelopes carry all three where applicable. A human-readable
version is presentation; ids are the comparison keys.

### 5.2 Determinism claim

The phase makes a narrow, testable claim:

- cooked assets, package indexes, package payloads, release manifest, notices
  and SBOM are byte-identical for the same normalized inputs;
- file ordering, JSON field ordering, timestamps and path separators are
  canonical;
- the executable is compared separately. If Rust/linker output is not
  reproducible on the pinned host, the build report names differing sections
  and the release manifest still binds the exact executable hash.

“Mostly reproducible” without separating these classes is not accepted.

---

## 6. What the new-engine survey contributes

Surveyed locally on 2026-08-29. This table is both a source map and a refusal
map: it records why a shiny feature did or did not enter the plan.

| Reference | Licence posture | Adopted lesson | Deliberately not adopted |
|---|---|---|---|
| **Prowl** | MIT | Primary build and authoring reference. Its build pipeline separates plan from execution and computes deterministic content chunks. `SpriteEditorWindow.cs` treats slices as stable sub-assets with rect/pivot/border/PPU/tight-mesh data and undo; `ProwlAction.cs` plus its property editor expose persistent calls; `SceneTool.cs`/`SceneToolManager.cs` isolate component-scoped handles, overlays, settings and dropped assets. | C# assemblies, Zip as the durable package contract, CLR reflection invocation, and arbitrary managed/native plugins. Somnium uses registered typed commands rather than copying member-name reflection. |
| **OpenXRay / X-Ray** | OpenXRay changes are MIT; the tree is derivative X-Ray source, so pattern-only unless STALKER-A proves a file independently safe. | Primary product/mod/living-world reference. Filesystem/archive code separates logical lookup from physical files. `alife_switch_manager` uses distinct online/offline distances, `alife_schedule_registry` budgets abstract updates, and the relation/inventory/trade/dialog/info/task/anomaly modules prove that product systems can remain data-addressable across UI and simulation boundaries. | Code, legacy formats, singleton registries, GameSpy/network stack, exact gameplay balance and class hierarchy. All living-world work is an independently designed pattern implementation. |
| **LayaAir 3.4** | MIT | A target is a publish profile, not scattered conditionals. Root README claims Web, native desktop/mobile and mini-game targets; `src/layaAir/platforms/` demonstrates platform adapters around common engine modules. | “One click” as an acceptance claim, platform count as a success metric, AIGC, WebXR and mobile in this phase. |
| **MethaneKit** | Apache-2.0 | A Null adapter is a real adapter, not a boolean headless mode: `Modules/Graphics/RHI/Null/` mirrors the graphics interface. Its platform modules and CI matrix separate compile targets from host-run targets. STALKER-C applies the pattern one layer higher to a player host that can validate packages without a GPU. | A second RHI beneath wgpu and explicit API resource-barrier abstractions. |
| **Myth** | MIT OR Apache-2.0 | `docs/advanced/headless-rendering.md` and the Python renderer mode show that offscreen/readback use cases deserve a first-class host, while Cargo/WebGPU targets keep platform features explicit. The headless packaged smoke is taken; the render graph is only recorded. | Python bindings, Web/WASM, 3D Gaussian splatting, SSA render graph, SSGI/SSSS and material lobes. |
| **Adria** | MIT | `Source` contains structured GPU debug, capture/profiler and crash-tool integrations; the release crash envelope should carry adapter/capability identity and last GPU breadcrumb, and optional tooling must never be required to run. | D3D12/Metal/Vulkan backends beside wgpu, DLSS/XeSS/FSR integrations, render graph and renderer effects already present in Somnium. |
| **UPBGE** | GPL | Pattern-only confirmation that authoring application and game player/distribution concerns must be separable, and that bundled licences are part of the artifact. | Any source adaptation, Blender's plugin model, Python runtime and GPL code. |
| **Nu** | Noncommercial licence | Pattern-only graphics reference. `Renderer3d.fs` makes local lighting environments explicit values: bounded probes/light maps carry ambient terms, diffuse irradiance and filtered environment data, are distance-sorted, and feed deferred/forward work. Its sprite batch/billboard messages also reinforce one sprite asset serving several render consumers. | All code adaptation; its licence is incompatible with Somnium's intended distributable engine. Algorithms and formats must be independently designed from public graphics literature and Somnium's existing renderer. |
| **Toaster 2.0** | No root licence found in surveyed snapshot | Its `tstb new` SDK workflow confirms that project scaffolding can be a tiny CLI surface. Prowl and Cargo already provide safer concrete references. | Source, SDK install layout and native-plugin assumptions. No code is consulted unless provenance is resolved. |

### 6.1 The Prowl lessons worth keeping

Prowl's build pipeline is valuable because its interface is deeper than its
editor window. `BuildPipeline` produces a stage graph and operations;
`BuildExecutor` owns resource limits, cancellation and structured issues. The
desktop pipeline declares dependencies between stages, and the asset collector
can ship only the closure of starting scenes. That separation lets a CLI and UI
share behaviour without the UI becoming the build system.

Somnium should take that shape, not those names wholesale. The smallest useful
external interface is `plan(request) -> BuildPlan`, with execution and UI
observation behind it.

Prowl's authoring code supplies a second, equally useful shape. A sprite is a
named sub-asset whose identity survives edits; the source texture owns slicing
metadata; the editor provides selection, handles, automatic slicing, preview,
undo and explicit save/reimport plus revert. Its scene tools coexist through a shared handle
arbitrator instead of each owning viewport input. Its persistent action UI is
friendly, but CLR member-name reflection is the wrong trust boundary for
Somnium. STALKER keeps the authoring experience and substitutes stable command
ids, typed value schemas and the existing Luau capability boundary.

### 6.2 The OpenXRay lessons worth keeping

The transferable idea is not its old archive code. It is the lookup model:
logical resource identity survives whether bytes come from a base archive, a
loose override or a mod. Resolution order is explicit, and compatibility with
old content is treated as a product constraint.

Somnium already has the stronger identity primitive—`AssetId` from normalized
source path—and the stronger payload primitive—content-addressed, hashed cooked
artifacts. STALKER combines those with a layered `ContentSource`; it does not
port X-Ray's filesystem.

The living-world lesson is the same separation applied to entities. X-Ray does
not require a distant object to remain a full rendered actor: online and
offline distances differ, so transitions have hysteresis, and scheduled
abstract work has an objects-per-update budget. Somnium will use neither those
classes nor their global registries. It will define a small serializable
`AbstractActor`, a deterministic scheduler, and explicit realize/abstract
adapters at the world-partition boundary. Factions, inventory and story facts
are durable data read by both representations; animation, physics and behavior
trees remain detailed-only.

### 6.3 The Nu graphics lesson worth keeping

Nu's useful idea is not another renderer architecture. It is that local diffuse
and specular environment lighting is authored as bounded scene data, selected
for deferred work near the eye and for forward work against each surface's
bounds, then submitted alongside ordinary render work. Somnium already has
global IBL, MORROWIND-AB's portable DDGI volume and visibility-buffer shading;
24Q's old probe controls map to DDGI rather than forming a second static-probe
system. STALKER completes the missing product seam: immutable cooked environment
captures, deterministic overlap/blend/priority, a per-object selection budget,
and editor/debug visualization. This fills the adjacent local-environment gap
without reviving 24Q as a competing probe system or replacing the frame graph
or GI stack.

### 6.4 Why the other shiny features stay out

The survey still contains enough unrelated work for several phases: Myth's SSA
render graph and 3DGS, Adria's reconstruction backends, Prowl's blend shapes,
progressive lightmapper, vehicles and audio effects, Laya's XR/video and
UPBGE's DCC integration. STALKER takes only capabilities exercised by the Zone
slice and its package. The rest stay recorded in §14.

---

## 7. The ten seams

Names here describe roles. Final Rust names may change during STALKER-A, but a
change must preserve the interface depth and ownership.

### Seam 1 — player host

```rust
pub struct PlayerConfig {
    pub release_manifest: PathBuf,
    pub mode: PlayerMode,
}

pub fn run_player<G: GameApp>(config: PlayerConfig) -> Result<(), PlayerError>;
```

The player owns window/headless host setup, manifest verification, content
mounts, settings/log paths and engine launch. The game owns `GameApp`. The
generated product bootstrap should be boring enough to audit in one screen.

**Adapters:** editor host, windowed player, headless verifier. Three callers
make this a real seam. Editor-only crates are forbidden in the player
dependency closure by a generated gate.

### Seam 2 — build target and plan

```rust
pub trait BuildTarget: Send + Sync {
    fn id(&self) -> TargetId;
    fn plan(&self, request: &BuildRequest) -> Result<BuildPlan, BuildError>;
}

pub struct BuildPlan {
    pub stages: Vec<BuildStage>,
    pub expected_outputs: Vec<ArtifactSpec>,
}
```

The interface returns data. It does not copy files, spawn Cargo or update a
progress bar. The executor validates dependencies, schedules bounded work,
records structured diagnostics and publishes into a staging directory. CLI,
editor and tests see the same plan.

**Adapters:** Windows x86-64 and Linux x86-64. Headless is a profile, not a fake
operating system.

### Seam 3 — content source

```rust
pub trait ContentSource: Send + Sync {
    fn describe(&self) -> ContentLayer;
    fn locate(&self, asset: AssetId) -> Result<Option<ContentEntry>, ContentError>;
    fn read(&self, entry: &ContentEntry) -> Result<Vec<u8>, ContentError>;
}
```

**Adapters:** loose cooked directory, indexed package, layered base/patch/mod
source. The caller asks for an `AssetId` and receives verified bytes plus
provenance. It never learns archive offsets or override search rules.

The package index maps sorted `AssetId` values to blob hash, offset, length,
kind and dependency hashes. Payload blobs may be grouped for IO, but identity
does not depend on group placement.

### Seam 4 — durable migration

```rust
pub trait Migration: Send + Sync {
    fn kind(&self) -> DurableKind;
    fn from(&self) -> SchemaVersion;
    fn to(&self) -> SchemaVersion;
    fn migrate(&self, input: &[u8]) -> Result<Vec<u8>, MigrationError>;
}
```

One registry plans an unbroken, acyclic path before running any step. A
migration is pure bytes-in/bytes-out; validation and atomic replace happen
outside it. Save games are the first consumer. Release/mod manifests may use
the same mechanism only if a second version actually exists—no hypothetical
generic framework before two adapters.

### Seam 5 — update transaction

```text
inspect current → plan required blobs → stage → verify → write next manifest
        → atomic activate → smoke → retain previous / rollback
```

The transaction is a state machine persisted beside the install. Every state is
idempotent after process death. Transport only supplies blobs; it cannot decide
what becomes active. Tests interrupt after every filesystem mutation and resume
from disk.

### Seam 6 — local lighting environment

```rust
pub trait LightingEnvironmentSource: Send + Sync {
    fn candidates(&self, world_bounds: Aabb) -> SmallVec<[LightingEnvironmentRef; 4]>;
}

pub struct LightingEnvironmentRef {
    pub asset: AssetId,
    pub bounds: Aabb,
    pub priority: i16,
    pub blend_distance_m: f32,
}
```

The renderer asks which immutable lighting environments overlap an object's
bounds. Selection, weight normalization and maximum candidate count are one
tested policy. GPU upload, cubemap layout and SH coefficient packing remain
implementation details. A capture contains diffuse irradiance, prefiltered
specular environment and optional ambient tint/intensity; it is not named a
“light map,” because it does not bake direct light into surface UVs.

**Adapters:** global sky/IBL fallback, cooked local capture, live editor preview.
DDGI is a contributor/fallback, not a fourth competing authoring model.

### Seam 7 — sprite asset

```rust
pub struct SpriteAsset {
    pub id: AssetId,
    pub texture: AssetId,
    pub rect_px: UVec4,
    pub pivot: Vec2,
    pub pixels_per_unit: f32,
    pub border_px: UVec4,
    pub mesh: SpriteMesh,
}
```

The source texture owns named slice metadata; each slice receives a stable
sub-asset id independent of list order. Import produces canonical UVs and
optional alpha-tight geometry. Runtime consumers never read importer metadata
or invent rectangles.

**Consumers:** retained-mode UI image/9-slice, world-space sprite renderer and
billboard/particle draw data. Three real consumers justify the seam. This phase
does not add a second 2D scene graph.

### Seam 8 — authored action

```rust
pub struct AuthoredAction {
    pub target: StableEntityRef,
    pub command: CommandId,
    pub argument: TypedValue,
}

pub trait ActionRegistry {
    fn schema(&self, command: CommandId) -> Option<&CommandSchema>;
    fn invoke(&self, action: &AuthoredAction, ctx: &ActionContext)
        -> Result<ActionReceipt, ActionError>;
}
```

Buttons, triggers, dialogue choices and scene tools serialize the same action.
The inspector lists only registered commands whose schema matches the selected
target and argument. Invocation crosses the existing command/capability gate;
missing target, changed schema and denied capability are data diagnostics.
There is no Rust reflection by member name and no arbitrary function pointer.
`ActionRegistry` is a schema-bearing projection over Somnium's existing command
registry and Luau capability table, not a second command catalogue.

### Seam 9 — simulation LOD

```rust
pub trait SimulationLodAdapter {
    fn realize(&mut self, actor: &AbstractActor, world: &mut World)
        -> Result<RealizedActor, SimulationError>;
    fn abstract_back(&mut self, actor: RealizedActor, world: &World)
        -> Result<AbstractActor, SimulationError>;
}
```

`AbstractActor` carries durable identity, template, transform/cell,
schedule/job, inventory summary, faction/relation deltas and story facts
required while distant. MORROWIND-S's ownership rule remains authoritative:
the containing cell owns an abstract record, while a realized actor is owned by
the detailed ECS; a global scheduler may index stable ids and due ticks but may
not own a second mutable copy. Detailed-only component state is initialized
from templates or reduced through an explicit adapter. Promotion and demotion
are transactions: the same identity cannot exist in both owners, and failure
leaves the source intact.

**Adapters:** humanoid/NPC and simple roaming creature before this becomes a
general seam. Static props and projectiles never enter the abstract scheduler.

### Seam 10 — durable facts and decisions

```rust
pub trait StoryState {
    fn has(&self, fact: FactId) -> bool;
    fn apply(&mut self, tx: FactTransaction) -> Result<StoryDelta, StoryError>;
}
```

A fact transaction atomically adds/removes named facts and records its source.
Dialogue preconditions, quest objectives, authored actions and mods query the
same read-only view. Relations and inventory stay separate modules; they emit
typed outcomes that a story transaction may consume. This seam is deliberately
narrow so `StoryState` does not become a universal game-state map or an
`Any`-typed event bus.

---

## 8. Sub-phases

### Track 0 — CORDON (where the editor ends)

#### STALKER-A — census, licences and the release contract

1. Create `dev records/phase STALKER/` and open `ATTRIBUTION.md` §13J (or the
   next actually free section—verify, do not assume the letter).
2. Generate a census: crate dependency graph, editor-only dependency closure,
   shipped asset kinds, current cook versions, durable file formats,
   `SOMNIUM_*` release-affecting variables, native libraries and third-party
   licences.
3. Freeze `release-manifest-v1.json`, `build-report-v1.json` and the normalized
   path/ordering rules with golden fixtures before writing package code.
4. Re-run the nine-reference licence audit and record exact commit/archive
   identity for each local snapshot. Toaster stays excluded if no licence is
   found; OpenXRay stays pattern-only unless provenance is unambiguous.
5. Measure current `cargo build --release -p vvardenfell` size/time and list
   every runtime file it actually needs. These are baselines, not promises.

**Exit:** generated census committed; schemas have malformed/truncation tests;
no package or player feature claimed.

#### STALKER-B — `somnium_player` and the dependency firewall (Seam 1)

1. Extract reusable launch/lifecycle code from example-specific setup into a
   player module without moving editor behaviour into it.
2. Add the tiny generated bootstrap for `GameApp` and make `vvardenfell` the
   first product.
3. Add a dependency gate: the player closure may include runtime crates but not
   editor panels, native file dialogs, authoring importers, thumbnails or undo.
4. Separate paths: immutable install/content; per-user config, saves, logs,
   crash envelopes and shader cache. No player write is allowed under the
   install root.
5. Make missing/unsupported release manifests fail before window creation with
   one diagnostic naming the file and supported versions.

**Exit:** windowed player reaches the same first frame as the workspace example
using a staged manifest; `cargo tree`/census proves the editor firewall.

#### STALKER-C — headless package host

MethaneKit's Null RHI is the architectural prompt, not an implementation to
port. Add a headless `PlayerMode` that verifies manifests/packages, constructs
the world, runs a fixed number of `GameApp` updates and exits without creating
a surface. Rendering-dependent components report a capability absence; they do
not pretend a Null GPU rendered them.

This is the CI and clean-machine smoke host. It must exercise the real package
reader, script sandbox, save path and start scene. It may use wgpu's headless
adapter for compute/render tests separately, but package validation does not
require a GPU.

**Exit:** `vvardenfell --headless --frames 300` runs solely from staged release
content and emits a deterministic state digest plus build/content ids.

### Track 1 — ROSTOK (build and package)

#### STALKER-D — build request, targets and stage executor (Seam 2)

Define canonical `BuildRequest`, `BuildProfile`, `TargetId`, `BuildPlan`,
`BuildStage`, `BuildIssue` and `BuildReport`. Initial stages:

```text
validate → compile player ─┐
          cook closure ────┼→ compose release → verify → publish
          notices/SBOM ────┘
```

Stages declare inputs, outputs, dependencies and resource class. The executor
supports cancellation, bounded parallelism and a staging root. A failed build
never publishes partial output. Diagnostic codes are stable and tests assert
them; UI text is not the protocol.

Prowl's staged build is the primary reference. Somnium keeps the interface
smaller: target modules plan, one executor runs.

**Exit:** an in-memory target and filesystem fake prove ordering, cancellation,
resume-cleanup and failure propagation without invoking Cargo.

#### STALKER-E — rooted asset closure

Add forward closure to the existing `AssetDependencyGraph`. Roots are:

- build-profile starting scenes;
- explicitly retained resources (loading screens, dynamic lookups, fallback
  localisation and mod entry points);
- engine defaults required by the selected capability tier;
- generated shader/material dependencies.

Every dynamic lookup must declare a root rule or be rejected in a release
build. The report lists roots, reachable assets, shared dependencies, excluded
assets, missing references and why each retained asset is present. No
“Resources folder means everything” shortcut.

**Exit:** the same roots produce the same sorted closure; removing an unused
1 MiB fixture removes exactly it; deleting a transitive dependency fails during
build, not at runtime.

#### STALKER-F — deterministic packages (Seam 3)

Implement `ContentSource` and package v1:

- canonical header and sorted index;
- bounded counts/offsets/lengths with overflow-safe decode;
- SHA-256 per blob and package index;
- stable chunk classes: bootstrap, shared, per-start-scene and optional groups;
- no timestamps or host paths in canonical bytes;
- package composition is independent of compression library iteration order;
- verifier can stream-check without loading an 8 GiB package into memory.

Start uncompressed or with one deterministic codec already in the tree. A
package is not accepted because it is smaller; it is accepted because random
asset lookup, corruption localization and patch reuse are correct. Compression
is measured after correctness.

**Exit:** directory and package adapters return byte-identical cooked assets;
every byte/bit truncation corpus fails safely; two builds match byte-for-byte.

#### STALKER-G — release composer, CLI and one editor surface

Add one CLI, conceptually:

```text
somnium build --profile release.windows.toml --out dist/vvardenfell
somnium verify dist/vvardenfell
somnium run dist/vvardenfell
```

The editor Build panel edits a profile, starts the same executor, renders its
structured progress and opens the report. It does not have its own copy/export
logic. Build-and-run launches the published artifact from its own directory,
never the editor process.

`publish` writes to a sibling staging directory, verifies it, then atomically
renames into the requested output. Existing outputs move to a recoverable
backup until final verification succeeds.

**Exit:** CLI and editor produce the same release manifest and package hashes
from the same profile.

### Track 2 — DEAD CITY (patch, migrate, recover)

#### STALKER-H — layered content and provenance

Mount ordered layers: base release, optional official patch layer, then enabled
mods in resolved order. For every `AssetId`, diagnostics can answer:

- which layer won;
- which lower entries it shadows;
- whether kind/schema/dependency expectations still match;
- which build/mod manifest introduced the bytes.

Override does not mean write into base. Disabling a layer restores the previous
entry with no recook. Core shaders and native libraries are non-overridable by
untrusted packages in v1.

**Exit:** a three-layer fixture resolves deterministically, disabling the top
layer restores the middle, and provenance output names every decision.

#### STALKER-I — patch generation and atomic update (Seam 5)

Compare two release manifests. Reuse identical content-addressed blobs; include
new blobs, removed references and the next manifest. Applying a patch:

1. validates source product/content id and free-space requirement;
2. stages blobs without touching active content;
3. verifies every staged hash and the complete candidate release;
4. atomically switches `current` to the candidate manifest;
5. runs the headless smoke;
6. retains the prior manifest/blobs until explicit cleanup.

Fault-injection tests terminate after every mutation. Resume either completes
the candidate or returns to the old release. There is no state in which neither
launches.

**Exit:** base → patch → rollback reproduces both original content ids;
interruption at every step preserves one verified bootable release.

#### STALKER-J — save schema and migration (Seam 4)

Gated on MORROWIND-AF. Wrap its game-state payload in a durable envelope with
product/build/content id, schema version, checksum, timestamp for presentation
only and enabled mod identities. Then:

- register ordered single-step migrations;
- plan a complete path before running;
- migrate to a sibling temporary file;
- validate by decoding with the current schema;
- retain the original as a backup until the migrated save loads;
- refuse downgrades and missing paths without altering either file.

Migrations run headlessly and use canonical fixtures committed by schema
version. A release cannot delete a migration while a supported fixture needs it.

**Exit:** oldest supported fixture migrates through every intermediate version;
malformed and future saves fail safely; interrupted migration leaves the source
byte-identical.

#### STALKER-K — local crash envelope

On panic, device loss or fatal startup failure, write a bounded local directory
containing:

- release/build/content ids and executable hash;
- target, OS, GPU adapter and `CapabilityReport`;
- package and enabled-mod manifests;
- last bounded log records and KENSHI profiler/event breadcrumbs;
- update transaction state and most recent save schema (never save contents);
- panic/device-loss text and a generated reproduction command.

Use platform minidumps only where an existing safe crate/tool makes them
reliable; the portable envelope is mandatory. No automatic upload. Paths,
usernames, tokens and script source are redacted by policy with tests.

Adria is the prompt for GPU breadcrumbs/capture integrations. Optional PIX,
RenderDoc or Aftermath presence may add attachments; their absence cannot
change engine behaviour.

**Exit:** deliberate panic and deliberate package-verification failure each
produce a bounded, redacted, self-identifying envelope a separate command can
inspect.

### Track 3 — WILD TERRITORY (mods without install mutation)

#### STALKER-L — mod manifest and deterministic resolver

`sommod-v1` declares:

- stable id, display name, semantic version and compatible product/engine
  ranges;
- dependencies, optional dependencies, conflicts and load-order constraints;
- content package hashes and entry scripts;
- requested capabilities, with human-readable reasons;
- licence/author/source metadata and whether redistribution is allowed.

Resolution is a pure function over installed manifests plus user choices. It
returns either one total order or a structured conflict/cycle explanation. A
lock file records exact chosen versions and hashes; launch never “finds latest.”

**Exit:** permutation/property tests prove input enumeration order does not
change the lock; cycle/conflict messages name the smallest useful chain.

#### STALKER-M — sandboxed data and Luau mods

Instantiate one trust domain per resolved package or explicitly shared group.
Use `Capabilities::SANDBOXED` as the default. Requested widening is granted by
the product profile/user policy, intersected with what this engine build knows;
unknown bits remain dropped exactly as today.

Mods can add data assets, localisation, runtime UI documents and Luau scripts.
They cannot load native code, replace core shaders, read arbitrary files, open
network sockets, escape their package namespace or write under the install
root. Persistent mod data lives under a per-product/per-mod user-data path with
quota and schema version.

Extend the Phase 16 threat model with package path traversal, duplicate ids,
zip/package bombs, capability forgery, dependency confusion, namespace escape,
malformed bytecode/source and hostile save data.

**Exit:** a sample HUD/content mod loads from a package; an adversarial mod
corpus cannot write outside its data root or obtain an ungranted command.

#### STALKER-N — mod inspection and safe-mode recovery

Add one editor/player inspection surface over the same resolver:

- installed/enabled/locked identity and hash;
- dependency/conflict diagnostics;
- requested versus granted capabilities;
- per-asset provenance and shadowing;
- “launch safe mode” with all non-base layers disabled;
- export a support report without copying mod payloads.

After a startup crash, the next launch offers safe mode based on the local crash
envelope; it does not silently disable user content or rewrite the lock.

**Exit:** a deliberately crashing mod can be isolated and disabled without
altering the base install or deleting its data.

### Track 4 — YANTAR (Nu-inspired local graphics, independently designed)

Nu is noncommercial and therefore pattern-only. Every algorithm, format and
shader in this track must be independently designed from Somnium's existing
24Q/IBL/DDGI code and permissive/public graphics references recorded by
STALKER-A. No Nu source is adapted.

#### STALKER-O — local lighting-environment asset and capture contract (Seam 6)

Add a bounded local-environment asset beside MORROWIND-AB's portable DDGI,
reusing compatible SH/cubemap plumbing where justified rather than reviving
24Q's hidden legacy controls as another scene-wide toggle:

- `LightingEnvironmentVolume`: box or sphere, capture position, priority,
  blend distance, ambient tint/intensity and enabled state;
- L2 diffuse SH plus a prefiltered specular cubemap with explicit orientation,
  face order, mip convention, colour space and format version;
- optional box-projected/parallax-corrected reflection for interior volumes;
- stable `AssetId` and dependency on the scene/material/environment inputs that
  produced the capture;
- capture provenance: engine/build id, adapter, settings and source hashes;
- a global sky/IBL fallback when no local volume contributes.

The editor may preview an uncooked capture, but a release consumes only a
versioned cooked asset. Builds never silently rebake lighting: stale provenance
is a diagnostic or policy error, because an unnoticed GPU bake makes release
output and patches unknowable.

**Exit:** schema fixtures round-trip; malformed cubemap/SH payloads are bounded;
one synthetic capture has a documented orientation/roughness gold image.

#### STALKER-P — selection, blending and visibility-buffer integration

Implement one renderer-facing policy, outside `SomniumRenderer`:

- spatially query volumes overlapping each renderable's bounds;
- sort deterministically by priority, containment weight, distance and stable
  asset id; cap the submitted candidates;
- normalize blend weights across boundaries without energy spikes;
- blend diffuse SH and prefiltered specular independently;
- use box projection only inside a valid box and fall back cleanly at edges;
- keep transparent/forward and opaque/visibility-buffer paths visually
  consistent;
- report selected probe ids and weights in frame diagnostics.

Do not replace DDGI, ReSTIR GI, global IBL or the shading pass. Local captures
supply low-frequency diffuse and environment specular; existing dynamic paths
remain responsible for dynamic indirect light. A capability/fallback table
states what happens when cubemap arrays or the preferred texture format are
unavailable.

**Exit:** overlapping indoor/outdoor volumes transition monotonically; moving
and static objects select the same environments; probe selection stays within
its measured CPU/GPU budgets under the KENSHI stress scene.

#### STALKER-Q — probe authoring, scene tools and graphics proof

Add a component-scoped scene tool through the STALKER-S registry, not special
cases in `UiManager` or the editor shell:

- create/resize/move box and sphere volumes with snapping and undo;
- capture from the volume, preview diffuse/specular channels and mark stale;
- overlays for bounds, blend shell, capture origin, priority and selected
  consumers;
- debug views for dominant probe, blend weights, SH irradiance, specular mip,
  box projection and global fallback;
- batch “validate captures” with actionable missing/stale/overlap diagnostics;
- cook/package parity test and a headless verifier for capture metadata/hash.

The visual fixture is a small bar interior opening onto an outdoor yard: glossy
metal, rough plaster and a moving object cross two overlapping volumes. Capture
the transition at fixed camera positions after tonemapping and compare with the
repository's image policy; never bless an all-black or fallback-only image.

**Exit:** the fixture looks local rather than sky-lit everywhere, survives
editor loose files → package mount unchanged, and has debug evidence explaining
every selected environment.

### Track 5 — 100 RADS BAR (Prowl-inspired UI and authoring)

Prowl is MIT, but Somnium still adapts concepts to its existing retained UI,
schema inspectors and command model. It does not import PaperUI/OrigamiUI or
add a second editor shell.

#### STALKER-R — sprite sub-assets, atlas import and runtime consumers (Seam 7)

Extend the texture importer with `Single` and `Multiple` sprite modes. Multiple
mode supports manual rectangles, regular grid slicing and alpha-island slicing.
Each named slice stores a stable id, rectangle, normalized/custom pivot,
pixels-per-unit, four borders and tight-mesh settings. Rules:

- rename/reorder preserves identity; deleting/recreating does not;
- bounds, overlaps, duplicate names, zero area and border inversions diagnose;
- alpha slicing has deterministic connectivity, padding and minimum-area
  settings, with a golden island corpus;
- tight geometry is simplified under explicit tolerance and vertex limits;
- atlas UVs include extrusion/bleed policy and remain correct after cook;
- source/import changes invalidate every affected sub-asset through the current
  dependency graph.

Add three consumers: retained UI `Image`/9-slice, a world-space `SpriteRenderer`
with layer/order/material/tint/flip, and billboard/particle draw data. The last
is an asset/render input only; MORROWIND's unfinished GPU particle simulation
is not pulled into this phase.

**Exit:** all three consumers render the same slice identity from loose and
packaged content; pixel, pivot, border and tight-mesh goldens pass.

#### STALKER-S — sprite editor and composable scene-tool registry

Create one docked Sprite Editor using existing UI primitives:

- zoom/pan checkerboard canvas, texture and alpha preview;
- select/create/move/resize slices; pivot and four border handles;
- grid and alpha-island slicing preview before commit;
- searchable slice list, duplicate/rename/delete and numeric inspector;
- explicit apply/revert, dirty state and one undo transaction per gesture;
- preview at native pixels, world units and 9-slice target size.

At the same time, extract a narrow scene-tool registry inspired by Prowl's
coexisting tools. A tool may declare availability, register hit-tested handles,
draw world/overlay primitives, contribute a toolbar/settings surface and handle
an asset drop. The viewport arbitrates controls once; tools do not read global
mouse state or own selection. Tool state is per viewport, while user settings
are versioned and persistent. Initial real adapters are probe volume, anomaly
volume and patrol route; sprite slicing uses the same handle vocabulary on its
2D canvas. Patrol geometry has one owner: if MORROWIND-P splines have landed,
the adapter annotates that asset; otherwise STALKER-V's minimal directed route
becomes the route seam MORROWIND-P must later consume, not a parallel spline.

**Exit:** tools coexist without stealing the transform gizmo; selection change,
undo/redo, viewport close and malformed saved settings are covered; graphify no
longer requires a new branch in the existing UI/editor god nodes per tool.

#### STALKER-T — typed authored actions and the product UI (Seam 8)

Implement an inspector field for an ordered list of `AuthoredAction` values.
For each call, authors pick a stable target, registered command and schema-
checked argument. The UI offers searchable grouped commands, inline argument
editing, reorder/duplicate/remove, missing-target repair and an invocation
preview. Validation runs at author time, cook time and load time. Commands from
mods are namespaced and still require granted Luau capabilities.

Use this seam and the sprite workflow to build actual game UI, not another
widget showcase:

- inventory/equipment with keyboard, mouse and controller transfer paths;
- two-party trade with quote, offer, accept/cancel and insufficient-funds
  states;
- dialogue choices with disabled-reason presentation;
- journal with active/completed/failed objective states;
- faction/reputation summary and anomaly detector signal;
- HUD prompts and notifications generated from localized keys.

All screens use the existing runtime UI, input actions, localisation and focus
model. Test 1280×720, 1920×1080 and 3840×2160; 100%/150% UI scale; long and
pseudo-localized strings; keyboard-only/controller-only navigation; explicit
focus order; no critical meaning encoded by colour alone. Large item/journal
lists must use MORROWIND's virtualization seam if available; STALKER does not
build a competing list widget.

**Exit:** the complete product UI can be operated without a mouse, every click
crosses a typed command receipt, save/reload preserves the visible domain state,
and package/mod provenance is inspectable from the screen that uses it.

### Track 6 — THE ZONE (X-Ray-inspired living-world simulation)

X-Ray source is pattern-only. This track copies no ALife classes, formats or
registries. It independently implements two costs of simulation on Somnium's
ECS, world partition, save and job systems.

#### STALKER-U — abstract actors and transactional realization (Seam 9)

Define a versioned `AbstractActor` record stored under MORROWIND-S's existing
cell-owned entity rule. The scheduler keeps a stable-id/due-tick index, not an
authoritative second actor registry. The minimum durable state is
identity/template, cell/transform, simulation clock,
schedule/job, faction, relation deltas, inventory summary, story-relevant facts
and deterministic RNG stream. Explicitly list which detailed components reduce
into the record and which are reconstructed from the template.

Interest comes from named sources (player/camera, quest pin, conversation,
editor inspection), each with enter and leave distances. Enter distance must be
smaller than leave distance. A transaction promotes one abstract record into a
detailed ECS entity or demotes it back; commit changes ownership only after the
adapter succeeds. Teleport, cell unload, death/removal, save during transition,
missing template and mod removal have named recovery results.

**Exit:** two adapters—humanoid and simple creature—round-trip identity and
declared durable state 1,000 times; fault injection never duplicates or loses
an actor; boundary oscillation does not cause promotion thrash.

#### STALKER-V — deterministic scheduler, jobs and patrols

Run abstract simulation in fixed world-time quanta. The priority key is stable
`(due_tick, actor_id)`; a configurable count/cost budget limits each engine
frame, backlog age is observable, and catch-up is bounded after load or clock
jump. Abstract code cannot touch renderer, physics bodies, UI widgets or detailed
behavior-tree instances.

Add data assets for:

- daily schedule entries with time band, activity tag and destination/job
  query;
- job anchors with tags, capacity and deterministic reservation;
- patrol graphs with named points, directed edges, waits and route policy;
- off-screen travel between cells with explicit duration and arrival event;
- debug time scrub, actor timeline, reservation table and backlog heat map.

Detailed AI may consume a resolved job/patrol when realized; it does not own the
schedule. The abstract simulator advances coarse travel/work/idle outcomes and
emits typed facts/events needed by RED FOREST.

STALKER-A resolves patrol-asset ownership against MORROWIND-P before code starts:
reuse landed splines with traversal annotations, or make this minimal directed
graph the shared route asset and update MORROWIND-P's open plan accordingly.

**Exit:** the same seed, input log and frame-time pattern produce the same
abstract digest; changing the per-frame budget changes latency but not logical
outcomes; 10,000 abstract actors meet the limit set by KENSHI evidence.

### Track 7 — RED FOREST (small game-framework modules)

These are reusable product modules, not a monolithic `GameManager`. Each owns
its data, commands, invariants, save schema and Luau capability. UI observes
snapshots and submits commands; it never mutates domain collections directly.

#### STALKER-W — factions, relations and reputation

Add stable faction assets, a base directed faction-to-faction attitude matrix,
and sparse per-actor/per-faction goodwill deltas. A policy maps the total to
`Friendly`, `Neutral` or `Hostile` with visible thresholds. Rank/reputation are
separate named values; they may affect dialogue/trade but do not secretly alter
the base matrix. Commands change goodwill with source/reason and emit a receipt.

Abstract and detailed actors query the same immutable relation snapshot.
Friendly/hostile changes may alter schedules or detailed AI on the next safe
tick; they do not mutate behavior trees from inside the ledger. Save/mod merge
rules, missing faction recovery and deterministic conflict order are explicit.

**Exit:** asymmetric relations, threshold crossings, modded factions and save
migration pass; trade/dialogue fixtures consume the same computed attitude.

#### STALKER-X — item instances, inventory, equipment and trade

Define immutable item templates and stable item instances. The minimum instance
state is template id, instance id, stack count, condition and namespaced custom
data under strict byte limits. Inventory is a container with weight/capacity;
equipment slots accept template tags. Transfers are atomic commands covering
world ↔ container, container ↔ container, stack split/merge, equip/unequip and
drop. A failed command changes nothing and explains why.

Trade creates an immutable quote over two inventories, money balances, item
conditions, faction/reputation modifiers and a trader policy. Accept revalidates
the quote then commits money/items as one transaction; stale, missing, locked
or unaffordable offers fail without partial transfer. This phase does not model
a regional economy, crafting, loot generation or weapon mechanics.

Abstract actors retain inventory instance identity and summary; detailed
realization restores containers/equipment. Packages/mods may add templates but
cannot redefine a base instance's template unnoticed.

**Exit:** property tests preserve item and currency totals through arbitrary
valid transfers/trades; save/load and abstract/detail round-trips retain every
declared field; product UI drives only command APIs.

#### STALKER-Y — facts, dialogue, quests and journal (Seam 10)

Add versioned data assets with stable node/objective ids:

- fact definitions and atomic add/remove transactions;
- dialogue graphs with speaker, localized line key, choice edges, fact/relation/
  item preconditions and authored actions;
- quests with localized title/body, ordered or parallel objectives, success/
  failure conditions and rewards;
- initial objective kinds: acquire item, reach volume, set fact and complete
  dialogue node;
- journal projection derived from quest state, never a second mutable copy.

Graph validation rejects dangling ids, impossible start nodes, action-schema
mismatches and accidental unconditional cycles; intentional repeatable loops
must be marked. Preconditions are pure queries. Actions run after a choice/
objective transaction commits and yield receipts. Luau may provide registered
condition/action commands under capability and budget limits, but authored data
remains inspectable when the script is missing.

**Exit:** one branching trader conversation starts a quest, relation changes
alter a choice and price, an inventory acquisition completes an objective, and
the journal/save/mod overlays all show the same state.

#### STALKER-Z — anomaly fields, detector and artifact integration

Create a reusable authored field volume with `Dormant`, `Warmup`, `Active` and
`Cooldown` states, deterministic timing/RNG, inner/outer falloff, accepted
target tags and an ordered list of typed effects. The fixture effects are
physics impulse, exposure accumulation and an authored action; a general health
or combat system is not introduced. Rendering and audio observe state/events
to drive existing particles, decals, lights and buses without living inside the
simulation component.

An anomaly can expose a detector signal independent of visibility. The product
UI presents direction/strength with accessible audio/visual feedback. A spawn
table may materialize an artifact item at a stable spawn point after a named
condition; collection uses STALKER-X and advances STALKER-Y. The scene tool
edits shape/falloff/spawn points and previews the signal field.

The integrated Zone slice contains the YANTAR bar/yard lighting fixture, one
trader, two factions, several scheduled abstract actors, one patrol, a dialogue
quest, trade, an anomaly, detector and artifact return. It runs for ten minutes,
saves while actors are abstract, reloads, applies a data-only mod that adds one
item/dialogue branch, and completes from packaged content.

**Exit:** deterministic replay digest plus visual/UI captures prove the whole
slice; removing the optional mod yields a clear orphaned-content diagnostic and
safe recovery rather than corrupting the base save.

### Track 8 — PRIPYAT (integrated release proof)

#### STALKER-AA — Windows x86-64 portable release

Produce the first supported graphical artifact on the development platform:

- release-profile player and native dependencies;
- verified base packages, Zone slice and release manifest;
- default config outside source control assumptions;
- `THIRD_PARTY_NOTICES` and machine-readable SBOM;
- verifier, local crash inspector and uninstaller manifest;
- no absolute build paths or editor caches.

Test on a clean Windows user/profile or disposable VM with no Rust, Git, repo
checkout or pre-existing Somnium data. GPU capability failure must be a clear
startup report, not a panic after window creation. Complete the integrated
slice with keyboard/mouse and with controller-only input.

**Exit:** clean-machine transcript, UI/graphics captures after tonemapping,
first-frame and ten-minute `.somtime`, domain-state digest, file manifest and
uninstall verification committed as generated evidence.

#### STALKER-AB — Linux x86-64 compile and headless packaged smoke

Add a real target adapter and CI job. The minimum accepted tier is:

- player and verifier compile for Linux x86-64;
- package output, including sprites/probes/domain assets, is platform-neutral
  and hash-identical;
- headless Zone simulation smoke runs on Linux CI;
- paths, permissions and atomic rename semantics pass;
- platform-specific native dependencies/notices are correct.

Graphical Linux support is not claimed until a named machine runs and captures
it. Cross-compiling an executable does not prove a display/audio/input stack.

**Exit:** CI artifact plus headless transcript; README/status wording says
exactly “headless validated” unless graphical evidence exists.

#### STALKER-AC — release candidate, update drill and close-out

Build RC1 twice from a clean checkout and pinned toolchain. Run:

1. all GHOSTFENCE rows and workspace/domain tests;
2. deterministic payload/package/manifest comparison;
3. clean install and full Zone-slice launch;
4. old save migration plus abstract actor/domain-state load;
5. base → RC patch, forced interruption at a sampled stage, resume;
6. rollback;
7. one valid content/dialogue mod and the hostile mod corpus;
8. probe/sprite package parity and UI navigation/capture matrix;
9. deliberate crash and support-report inspection;
10. uninstall, proving user saves/config remain and installed files are removed.

Publish `release_report.md` generated from the manifests and command outputs.
Update `README.md`, `context.md`, the development-record index and attribution
**only here**, after the artifact exists. Earlier sub-phases do not advertise a
release or any feature as shipped.

**Exit:** the clean-machine rule at the top and every product-slice acceptance
row are true without exceptions hidden in prose.

---

## 9. Sequencing and gates

```text
MORROWIND-Q/R + MORROWIND-AF + PORTAL gates + KENSHI-J/authorized fixes
                              │
                              ▼
                         STALKER-A
                              │
                    ┌─────────┴─────────┐
                    ▼                   ▼
                 B → C               D → E
                    └─────────┬─────────┘
                              ▼
                           F → G
                              │
                              ▼
                        H → I → J → K
                              │
                              ▼
                        L → M → N
                              │
               ┌──────────────┼──────────────┐
               ▼              ▼              ▼
             O → P          R → S          U → V → W → X → Y
               │              │                        │
               └───────┬──────┘                        ▼
                       ▼                               Z
                       Q                               │
                       │                               │
                       └──────────────┬────────────────┘
                                      ▼
                                      T
                                      │
                                      ▼
                                AA → AB → AC
```

1. **KENSHI first.** STALKER does not ship a frame whose combined-load limits
   have not been published.
2. **MORROWIND-AF before STALKER-J.** STALKER owns migration/envelope and cannot
   invent the game-state model AF was supposed to define.
3. **B and D may run in parallel only after A.** Player dependency separation
   and a pure build-plan module are disjoint; both meet at F/G.
4. **No archive before closure.** F packages E's rooted set. Packaging an
   hand-written list would freeze the wrong interface.
5. **No patch before immutable packages.** I depends on F/H content identity;
   updating mutable loose files in place is explicitly refused.
6. **No mods before layered provenance.** L can define schema early, but M/N
   cannot load anything until H can explain which bytes win.
7. **Feature foundations may overlap, integration may not.** O, R and U may
   start after A and their MORROWIND prerequisites. Q waits for P/S; the final
   product UI T waits for the W–Z domain commands it presents.
8. **Abstract state before game systems depend on it.** W–Y must prove which
   fields survive U/V transitions. No subsystem stores a raw detailed ECS
   entity id in durable/offline state.
9. **One slice before release proof.** Z integrates graphics, UI, schedules,
   factions, inventory, story and mods. AA cannot replace that proof with a
   collection of per-feature demos.
10. **Windows proof before broader platform claims.** AB can compile earlier,
   but AC's release claim rests on AA's clean-machine run.

### 9.1 Reduced cut

If the full phase is too large, the smallest coherent *feature-bearing*
shipping cut is:

**A–G, J, O–Z, AA and AC.**

That defers updater/crash/mod tooling and Linux proof while retaining a verified
standalone package, migration and the complete Zone slice. If mod loading is
claimed, H/L/M/N return as a unit. Do not ship I without H, M without L/H, P
without O, Q without P/S, T without W–Z, or AA without Z. Deferred tracks remain
named, not half-implemented.

---

## 10. GHOSTFENCE — new must-not-break rows

Existing rows remain. STALKER adds generated rows:

| Row | Failure condition |
|---|---|
| `player-editor-firewall` | Player dependency closure includes an editor-only crate/module or `rfd`. |
| `release-schema-fixtures` | Current reader cannot parse all supported release/package/save/mod fixtures, or accepts a future version silently. |
| `deterministic-content-build` | Same normalized inputs produce different cook/package/manifest/notices/SBOM bytes. |
| `root-closure-complete` | A runtime-declared asset reference is outside the generated closure, or a retained asset has no reason. |
| `package-threat-model` | Malformed/truncated/oversized package corpus panics, allocates past limits or escapes paths. |
| `update-interruption` | Any fault-injection point leaves zero verified active releases. |
| `save-migration-chain` | A supported fixture has no complete path to current or migration alters the source on failure. |
| `mod-capabilities` | Unknown/ungranted capability can reach a `ScriptCommand`, or native code is accepted from an untrusted mod. |
| `release-compliance` | Shipped file has no release-manifest entry, SBOM attribution or licence policy result. |
| `headless-package-smoke` | Packaged `vvardenfell` cannot run its deterministic fixed-frame digest without source assets/GPU. |
| `lighting-environment-fixtures` | Probe schema/capture orientation changes, overlap weights do not normalize, fallback goes black, or loose/package selection differs. |
| `sprite-asset-fixtures` | Slice reorder changes id, importer output is nondeterministic, 9-slice borders invert, or atlas UV/tight mesh differs after cook. |
| `authored-action-schema` | Missing/renamed/denied command invokes anyway, argument type is unchecked, or a mod bypasses capability policy. |
| `scene-tool-arbitration` | Two live tools consume the same control, bypass undo/selection, or leave capture active after viewport/tool destruction. |
| `simulation-lod-conservation` | Realize/abstract fault duplicates or loses identity/items/facts, hysteresis flaps, or deterministic digest changes with frame budget. |
| `domain-transaction-conservation` | Failed inventory/trade/fact transaction partially applies or item/currency totals change without a named source/sink. |
| `story-graph-fixtures` | Dangling/unreachable nodes, schema-mismatched actions or accidental unconditional cycles enter a package. |
| `zone-slice-headless` | Packaged ten-minute abstract/domain replay cannot reproduce its expected digest without renderer/audio/editor. |

A platform job that cannot run reports `SKIP` with the exact missing runner or
tool. It never turns absence into green.

---

## 11. Acceptance matrix

| Claim | Evidence |
|---|---|
| Standalone player is real | Clean-machine windowed launch plus dependency-firewall report. |
| Build is deterministic | Two clean builds; classed hash comparison separating executable from canonical data. |
| Only reachable assets ship | Generated root/closure/exclusion report and missing-dependency negative test. |
| Packages are safe | Directory/package parity, streaming verifier, corruption and truncation corpus. |
| Update is atomic | Fault injection after every mutation; one verified release always launches. |
| Saves survive versions | Oldest supported fixture migrates through every version and loads in player. |
| Crash is explainable | Redacted local envelope from deliberate panic/device/package failure. |
| Mods are controlled | Deterministic lock, provenance report, capability denial and hostile corpus. |
| Windows is supported | Clean VM/user proof with no toolchain/repo/editor cache. |
| Linux claim is honest | Headless CI proof; graphical status remains unclaimed unless captured. |
| Distribution is compliant | Generated notices, SBOM and shipped-file manifest with no unknown file. |
| Local lighting is real | Indoor/outdoor overlap goldens, probe-id/weight trace, specular roughness ladder and loose/package parity. |
| Sprite workflow is one contract | Manual/grid/alpha slice goldens; the same stable slice appears in UI, world sprite and billboard consumers. |
| Authoring stays modular | Probe/anomaly/patrol tools coexist through handle arbitration; sprite/action edits are undoable and schema-valid. |
| Product UI is usable | Inventory, trade, dialogue, journal, faction and detector flows pass resolution/scale/localisation/focus/controller matrix. |
| Distant world stays alive | 10,000-actor budget/digest, hysteresis trace and transactional abstract/detail round-trip. |
| Domain state is durable | Relation, item, trade, fact, dialogue and quest fixtures survive save/load, package patch and optional-mod recovery. |
| The slice is integrated | One packaged ten-minute run completes the artifact quest across lighting, schedule, trade, dialogue, anomaly and return. |

No acceptance row is satisfied by an editor screenshot alone.

---

## 12. Risks and controls

| Risk | Control |
|---|---|
| Build system becomes an editor-owned god function | Seams 1/2; plan data and executor tests precede UI. |
| New package format duplicates the cook | Cooked assets remain payload authority; package is an index/container only. |
| Dynamic asset names escape closure analysis | Explicit retained roots or release-build error; report every reason. |
| Hash verification becomes too slow | Stream and cache verified indexes; measure before weakening integrity. |
| Update corrupts install on Windows file locking | Stage outside active tree, manifest pointer switch, fault-injection on actual Windows filesystem. |
| Saves become unrecoverable | Sibling migration, validation, retained original, fixture chain gate. |
| Mods become native-code supply chain | Luau/data only, capability intersection, no DLL loading, package namespaces. |
| “Cross-platform” becomes README fiction | Separate compile/headless/graphical tiers with evidence per target. |
| Reproducible-build claim is defeated by linker metadata | Compare canonical data separately; report executable differences and exact hash. |
| Licences from local reference trees contaminate adaptation | STALKER-A provenance matrix; GPL/noncommercial/unclear sources pattern-only. |
| Crash report leaks user data | Local-only default, bounded fields, redaction tests, no save contents or script source. |
| Probe work becomes a second GI architecture | Seam 6 supplies local environment data only; existing IBL/DDGI/shading ownership and defaults stay fixed. |
| UI work deepens existing god nodes | Sprite/action/tool registries sit behind data interfaces; `UiManager`, `Widget`, `UserInterface` and editor shell gain adapters, not per-feature branches. |
| Simulation boundary loses state | Explicit reduction table, transaction ownership, fault injection and conservation digest before any large actor count. |
| “Game framework” becomes a god manager | W–Z are separate domain modules with snapshots/commands; no universal mutable service locator or opaque event payload. |
| Feature scope consumes release proof | §3 names the exact slice, §9 keeps AA/AC gated by Z, and the reduced cut drops unrelated release extras before cutting slice integrity. |

---

## 13. Evidence plan

`dev records/phase STALKER/` is generated progressively:

- `STALKER-A_census.md`, `_licence_audit.md`, `_baseline.json`;
- versioned schema fixtures and malformed corpora;
- `STALKER-B_player_dependency.json`;
- `STALKER-C_headless_digest.txt`;
- `STALKER-D_build_plan.json`, structured failure fixtures;
- `STALKER-E_asset_closure.json` and human summary;
- `STALKER-F_package_map.json`, determinism hashes and corruption report;
- `STALKER-G_cli_editor_parity.json`;
- `STALKER-H_provenance.json`;
- `STALKER-I_patch_report.json` and interruption matrix;
- `STALKER-J_migration_matrix.json` plus save fixtures;
- `STALKER-K_crash-envelope-redacted/`;
- `STALKER-L_mod_lock.json`, resolver property-test seed corpus;
- `STALKER-M_mod_threat_model.md`;
- `STALKER-O_probe_schema.md` and capture provenance fixtures;
- `STALKER-P_probe_selection.json`, performance trace and transition goldens;
- `STALKER-Q_probe_authoring.md` plus loose/package graphics captures;
- `STALKER-R_sprite_import.json` and sprite/atlas goldens;
- `STALKER-S_scene_tool_arbitration.json` and editor gesture/undo fixtures;
- `STALKER-T_product_ui_matrix.md` and authored-action receipts;
- `STALKER-U_simulation_handoff.json` and fault-injection matrix;
- `STALKER-V_abstract_scheduler.somtime`, digest and backlog report;
- `STALKER-W_relations.json`, `STALKER-X_inventory_trade.json`;
- `STALKER-Y_story_graph.json`, save fixtures and validation report;
- `STALKER-Z_zone_slice.md`, deterministic replay and integrated captures;
- `STALKER-AA_windows_clean_machine.md` and post-tonemap/UI captures;
- `STALKER-AB_linux_headless.md`;
- `STALKER-AC_release_report.md`, generated notices and SBOM.

Binary release artifacts themselves do not belong in Git unless repository
policy changes. Manifests, small fixtures, reports and hashes do. Evidence names
the exact command, commit, toolchain, target and hardware/VM.

---

## 14. Left open, deliberately

**14.1 Store distribution and signing.** Steam/Epic/MSIX/AppImage, notarisation,
code signing and update key management require accounts, credentials and policy.
STALKER produces deterministic inputs for them and does not fake integration.

**14.2 Online transport.** Patch construction and application are local. HTTP,
CDN layout, mirrors, bandwidth scheduling and telemetry are a distribution
service, not an update transaction.

**14.3 Native plugins and out-of-process gameplay.** Prowl's native plugins are
not adopted. If native gameplay is ever required, MORROWIND §14.9's
out-of-process VM/IPC reference remains the starting point. Loading a mod DLL
into the player is not an acceptable shortcut.

**14.4 Render graph.** Myth's strict SSA graph (`docs/architecture/render-graph.md`,
`docs/articles/render-graph-design.md`) and Adria's graph are the strongest new
evidence since MORROWIND refused the feature. The refusal still stands. Reopen
only if KENSHI publishes a pass-scheduling wall; then the required design goals
are dead-pass elimination, transient aliasing, topology visualization and a
small pass-declaration interface—not a graph for its own sake.

**14.5 Web, mobile, XR and consoles.** Laya and Myth show that a common engine
can target them. Somnium needs a platform capability audit, Jolt/filesystem
adapters and real hardware. None is claimed from a cross-compile.

**14.6 Binary deltas.** Package-level content addressing comes first. Add
binary delta compression only if measured patches are still unacceptably large.

**14.7 Cloud saves, achievements, workshop and accounts.** Product services,
not engine packaging. Save envelopes and mod locks are designed so those
services could transport them without changing their formats.

**14.8 New feature finds from the survey.** Recorded for later triage, not
authorized here: Myth 3D Gaussian splatting/headless Python bindings and
advanced material lobes; Prowl blend shapes, progressive surface lightmapping,
terrain holes, vehicles and audio effect chains; Laya XR/video/full 2D scene
pipeline; Adria GPU printf/assert and vendor upscalers; UPBGE DCC/logic
integration. Sprite assets are now authorized only through R/S's three-consumer
contract; this is not permission for a parallel 2D engine.

**14.9 Combat, weapons, health, crafting and regional economy.** RED FOREST is
an integration framework, not a genre kit. Its anomaly fixture uses exposure,
impulse and authored effects; trade uses a quote/policy, not simulated supply.
Broader combat/economy design needs its own product requirements and phase.

**14.10 Dynamic probe relighting and full lightmap baking.** YANTAR captures
static local environments and blends them with existing dynamic GI. Per-frame
reflection capture, planar reflection scheduling and UV surface-lightmap baking
remain separate performance/authoring projects.

---

## 15. Start checklist

Before STALKER-A:

1. Confirm KENSHI-J's limits report exists and every authorized fix is resolved
   or explicitly refused. If KENSHI has not run, stop.
2. Confirm MORROWIND-Q/R and MORROWIND-AF are in tree. If AF is not, STALKER-J
   is blocked and the reduced cut cannot claim save compatibility.
3. Read `cook.rs`, `capability.rs`, `GameApp`/`EngineContext`, the KENSHI
   determinism contract and the complete GHOSTFENCE script.
4. Read the mandatory local guidance first:
   `C:/Users/adhir/.claude/plugins/graphify-8/AGENTS.md`, its linked Graphify
   Codex skill, `graphify-out/GRAPH_REPORT.md`, and
   `C:/Users/adhir/.codex/AGENTS.md`. Query the existing graph for the seam being
   changed before opening broad source directories. Run `graphify update .`
   only after code—not this planning Markdown—changes.
5. Re-read `context.md`, `README.md` and the relevant CONTROL/MORROWIND/PORTAL/
   KENSHI development records before trusting this draft's “absent” rows.
6. Re-run the census at current HEAD. Every count in §4 will have drifted.
7. Verify the reference licences from their local files; do not trust this
   draft's summary as legal provenance.
8. Before O, R or U, re-read the named Nu/Prowl/X-Ray source files in §16 and
   record whether each finding is pattern-only or safely adaptable. Nu and
   derivative X-Ray remain pattern-only by default.
9. Create the evidence folder and next free attribution section; no generated
   evidence before the generator exists.
10. Name the supported target tier and clean-machine environment before writing
   platform code.
11. Re-read the clean-machine rule. If a proposed shortcut relies on the repo,
   toolchain, source assets or editor cache, reject it before implementation.

---

## 16. Research sources and confidence

**Measured in Somnium at `d19b7c1`, 2026-08-29 (high confidence):** workspace
crate/tool/example list; absence of build profile/player/package/update/mod
manifests; `GameApp` and `EngineContext`; `CookedAsset`, `CookManifest`,
`AssetDependencyGraph`, `tools/assetcook`; `Capabilities::SANDBOXED`; the
`vvardenfell` public-API example. `graphify-out/GRAPH_REPORT.md` dated
2026-08-27 was read first; its god nodes (`UiManager`, `Widget`,
`SomniumRenderer`, `Engine<G>`) and existing MORROWIND seam hyperedges confirm
that release work should attach at game/content interfaces and new UI/graphics
authoring should attach through data/tool registries rather than deepen the
renderer or editor god nodes. A Graphify query against the 11,036-node graph
confirmed CONTROL's schema inspector/multi-selection and MORROWIND's runtime UI,
streaming and renderer tracks already exist, while the named local-probe,
sprite-authoring, offline-simulation and game-domain modules do not.

`context.md`, `README.md`, the development-record directory and the Graphify
report were read before the external engines. That order matters: it is why
STALKER does not duplicate CONTROL multi-select, MORROWIND runtime UI, world
partition, DDGI, localization or the unfinished GPU-particle work.

**Read in depth (high confidence for architecture, not a licence conclusion):**

- Prowl `BuildPipeline.cs`, `DesktopBuildPipeline.cs`,
  `PlatformBuildProfile.cs`, `Assets/ChunkPlanner.cs`, `BuildExecutor.cs` and
  `PluginInfo.cs`; plus `Prowl.Runtime/Utils/ProwlAction.cs`, its property
  editor, `GUI/SpriteEditor/SpriteEditorWindow.cs`, `SpriteRenderer.cs`,
  `SceneTool.cs`, `SceneToolManager.cs` and drop/shortcut registries;
- OpenXRay root README/licence and `xrCore` filesystem/archive census; plus
  `alife_switch_manager`, `alife_schedule_registry`, relation/community,
  inventory/trade, phrase/dialog/info, game-task, patrol/smart-terrain-task,
  `CustomZone`/anomaly-detector and artefact headers/implementations;
- Nu `Render/Renderer3d.fs` including lighting-environment values, bounded
  light-map selection, sky/probe/light messages and deferred/forward task
  structure; `VulkanLightMap.fs`, sprite batch/singleton, particles and
  overlayer reflection were located for implementation re-read;
- Myth README and headless/render-graph documentation;
- MethaneKit README, Apache licence, platform/RHI layout and Null module;
- Adria README/feature list and MIT licence;
- LayaAir README, MIT licence and platform-directory census.

**Listed or read at overview level (medium confidence; sub-phase must re-read
before relying):** UPBGE player/release layout and GPL notice; Nu's Vulkan
implementation below the renderer contracts; Toaster README/SDK workflow and
absence of a root licence; exact OpenXRay override ordering and gameplay data
files; Adria crash-tool implementation.

**Claims this draft intentionally does not make:** current upstream versions,
platform support beyond what the local snapshots state, patch/install
performance, release size, compression ratio, reproducible executable bytes,
or legal compatibility of derivative X-Ray source. STALKER-A turns the local
snapshot and licence facts into an auditable record before implementation.
