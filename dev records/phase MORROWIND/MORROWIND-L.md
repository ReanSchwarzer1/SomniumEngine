# MORROWIND-L — the timeline

**Complete, 2026-08-25.** Track 2 (CONSTRUCTION SET), after MORROWIND-K.

## One reusable timeline

`somnium_ui::timeline` is the single archetype-driven timeline surface. Its
versioned document owns nested groups, typed tracks, media clips, continuous
channels/keyframes, markers and duration. The retained editor adds playhead
scrubbing, cursor-anchored wheel zoom, middle-button pan, snapping, bounded
undo/redo and undoable move/resize/delete operations. Deleting a track removes
its dependent media in the same history entry.

`TimelineCatalogue`, `TrackArchetype` and `LaneArchetype` are consumer data,
not conditionals in the widget. The built-in catalogues prove two independent
consumers: animation clips/events and MORROWIND-H UI motion. Serialization is
deterministic, rejects future versions and unknown archetypes, preserves lane
order, validates every id/reference/range and round-trips byte-stably.

## CONTROL-K is embedded, not duplicated

The selected numeric channel is edited by the existing CONTROL-K
`CurveEditor`, installed as a real retained child of `TimelineEditor`.
Embedded curve editors keep their ordinary `FromWidget` output and also route
their committed/live values directly to the owning timeline. This closes the
retained-tree seam without shell forwarding and without a second curve model.
Continuous drags coalesce into one timeline history entry; discrete curve
operations create an ordinary undo entry.

## Shipped authoring and game boundary

The Animation workspace is one hidden vertical splitter containing the
MORROWIND-V graph and MORROWIND-L timeline as sibling panes; the curve editor
is the timeline's child. `UiManager::edit_animation_timeline` loads a document,
opens the workspace and retains changed documents for the asset/save owner.
Workspace switching toggles the shared container, so neither editor escapes
the Animation workspace.

`examples/vvardenfell` uses only public `somnium_ui::timeline` APIs to author
both an animation timeline and a non-animation UI-motion timeline. Each has a
group, track, media, marker and key, serializes and byte-stably round-trips, and
contributes to a deterministic retained evidence digest.

## Verification

- Timeline model/serialization/retained-route tests: **8/8 passed**.
- CONTROL-K curve-editor tests: **7/7 passed**.
- Full `somnium_ui` library suite: **572/572 passed**.
- Vvardenfell deterministic timeline evidence: **1/1 passed**;
  `cargo check -p vvardenfell` passed.
- The Animation shell regression proves workspace → graph/timeline parentage,
  timeline → curve parentage, hidden startup and shared visibility.
- `git diff --check` passed before the phase-record pass.

No visual capture is invented for this record. The retained draw and routed
input tests prove the production path; a cinematic screenshot would add no
behavioral evidence beyond them.

## Reference boundary

Flax Timeline and O3DE Maestro informed the framework vocabulary and consumer
split. Fyrox supplied the permissive retained-control conventions already used
by `somnium_ui`. The implementation is original Rust over Somnium's existing
controls and CONTROL-K curve data; no proprietary source was copied.
