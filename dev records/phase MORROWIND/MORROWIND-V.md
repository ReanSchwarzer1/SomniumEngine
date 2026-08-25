# MORROWIND-V — clips, blend trees and state machines

**Complete, 2026-08-25.** Track 5 (DWEMER), after MORROWIND-U and MORROWIND-K.

This sub-phase adds the animation runtime above U's skeleton/pose seam and
authors it through K's one graph surface. The renderer contract is unchanged:
the animation crate produces a `Pose`; U alone converts poses to palettes and
posed vertex buffers.

## Runtime contract

`somnium_anim` now supplies:

- validated transform tracks and clip sampling, including looping, negative or
  scaled playback, rest-channel fallback and finite-time rejection;
- typed bool, float, integer and trigger parameters bound to a versioned schema;
- ordered `Blend1D` samples and authored triangulated `Blend2D` samples. Outside
  a 2D hull, sampling projects to its nearest boundary edge instead of blending
  distant poses. Invalid, crossing, non-manifold, disconnected or overlapping
  triangulations are rejected at construction;
- constant or parameter-driven layers with optional per-bone masks;
- state machines over arbitrary compiled pose nodes, typed conditions,
  deterministic transition order, blend times and definition/graph hot-reload
  guards;
- named cyclic sync tracks. A stable authored leader supplies phase, compatible
  markers are checked across every recursively reached clip, and phase-aligned
  target time survives transition completion;
- cache nodes keyed by caller generation, fixed evaluation lane, graph id,
  graph version and node id. Evaluation automatically removes older generations
  in the same lane and stale hot-reload versions. State source/target lanes do
  not alias any future caller generation.

`Blend2D` topology is validated and retained once in `AnimGraphAsset`; the hot
evaluation path computes barycentric weights without reconstructing the
triangulation.

## Authoring through MORROWIND-K

The animation catalogue contains clip, two- and three-sample one-dimensional
blends, single- and multi-triangle two-dimensional blends, layer, cache,
reroute, output and state archetypes. Blend nodes carry an authored sync leader;
layer nodes accept either a constant or parameter weight and an authored bone
mask. `compile_animation` follows the animation output and every authored state
pose, preserves the caller-owned definition revision, and produces both the
UI-neutral `AnimGraphAsset` and the durable authored-node to runtime-node map
used by games and state compilation.

State-machine layout uses the same `GraphSurface` selection, pan/zoom, grouping
and history substrate. Cyclic transitions are overlay records rather than
ordinary pose wires, preserving K's acyclic data-flow guarantee. A versioned,
deterministic `AnimationStateMachineDocument` owns the surface, initial state,
transition overlays and bounded undo/redo history. Its compiler resolves typed
pose pins through the authored/runtime map into `StateMachine`, where the
runtime performs the final graph, schema, condition and sync validation.

This is a shipped authoring path, not only a compiler API. The registered
Animation workspace hosts a production `GraphEditor`; game/editor code opens a
document with `UiManager::edit_animation_state_machine`. Node bodies draw their
catalogue-declared pin labels and literal controls with ranges, units, tooltips,
validation and one undo step per commit. Alt-click selects the initial state,
Shift-drag creates a cyclic transition, and selecting its labelled overlay opens
the in-canvas Blend Time, Sync Track and Conditions fields plus syntax help and
Delete. Overlay edits use the document's own bounded undo/redo history while
the one shared graph surface continues to own pose edits.

## Game and Luau boundaries

`examples/vvardenfell` builds one-joint idle/walk/run clips, compatible
foot-contact sync tracks and a K-authored three-sample speed blend. It compiles
and evaluates the graph through public `somnium_ui` and `somnium_anim` APIs,
attaches a real Luau script that changes the speed parameter, and retains the
sampled root displacement as headless evidence.

Luau callbacks have typed `setAnimationBool`, `setAnimationFloat`,
`setAnimationInt` and `triggerAnimation` calls. They emit a neutral deferred
`SetAnimationParameter` command; `ScriptHost::set_animation_parameter_router`
connects that command to the game-owned animation instance at the existing
phase safe point. Luau never receives an engine pointer, and schema validation
remains in the animation runtime.

## Verification

- `somnium_anim`: **46 unit tests passed**; strict clippy passed.
- `somnium_ui`: **561 unit tests, 6 shader integration tests and 1 doc test
  passed**, including routed literal/state edits, real mouse/keyboard transition
  inspection, deletion and the shipped Animation workspace instance.
- Luau vertical slice: **25 tests passed**, including all four typed animation
  commands crossing the real VM boundary.
- Core deferred command bridge: **17 focused tests passed**, plus **1** concrete
  `ParameterSet` schema/application test.
- `cargo check -p vvardenfell` passed through public APIs.
- GHOSTFENCE: **7/7 rows passed**, including **1,808 tests passed, 0 failed**
  against the repository floor of 945. The golden-image row compared all 3
  registered images within threshold.

No PNG is invented for this record. The runtime tests prove sync-on/off produces
different locomotion sampling, but a visual foot-slide comparison needs a
windowed, skinned character capture after tonemapping. U's thousand-character
performance comparison likewise remains open until the repository contains a
rendered crowd scene; this sub-phase does not relabel a headless pose test as a
GPU measurement.

## References

Esoterica's MIT animation runtime and Fyrox's MIT ABSM editor were read for
permissive patterns. The clean-room distinctions are in `ATTRIBUTION.md`
§13H.18; Flax remains excluded under §13H.17.
