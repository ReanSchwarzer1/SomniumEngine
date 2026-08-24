# CONTROL-B property seam evidence

Date: 2026-08-23

## Implemented

- `FieldSchema` carries hard/soft bounds, step, precision, unit, documentation,
  display/group/order metadata, advanced/read-only state, `ChangeScope`, and an
  engine-neutral asset-kind mask. `component_schema!` accepts every option and
  captures `///` field documentation.
- `TypeRegistry` has a `SchemaDecorator` hook. `AssetRef` has named raw
  conversions and both `AssetRef` and `Option<AssetRef>` implement
  `ReflectField`.
- `PostProcessComponent` (62 fields), `ParticleEmitter`, `BuoyantVessel`, and
  `CameraSettingsComponent` are registered. The built-in registry now contains
  16 component schemas. Rebuilding terrain fields declare entity scope.
- `EditingRulesRegistry` supplies tri-state editability and built-in conditional
  visibility for Post Processing, spot-light cone angles, and water spectrum
  controls.
- `PropertyEditorRegistry` covers every current `FieldType`, including entity,
  asset, recursive array, and a visible unsupported fallback.
- `generate_property_rows` consumes `schema.fields`, filters
  `FieldFlags::EDIT`, evaluates editing rules, chooses an editor, computes true
  default/revert state, and searches labels plus documentation.
- The live Details panel now consumes those generated rows every frame. Its
  durable binding is `(StableId, FieldId)`, and bool, integer/float, text,
  vector, Euler/quaternion, colour, and enum widgets all emit the generic event.
  Entity, asset, collection, and unknown shapes remain visible rather than
  disappearing; the asset row exposes the engine-neutral query/commit hook that
  CONTROL-C supplies.
- `EditorEvent::SetComponentField` and `SetFieldCmd` are live. A `GestureId`
  captures the mouse-down baseline; live writes coalesce into one committed undo
  step; no-op gestures are discarded. Field/component/entity/scene snapshots
  are selected from `ChangeScope`.
- All schema-backed numeric, boolean, colour, and enum rows use the generated
  route. `InspectorField`, `ColorField`, `PostFxToggle`, `SetInspectorValue`,
  `SetInspectorColor`, `CancelInspectorColor`, `field_bindings`, and the served
  `InspectorHandles` fields have zero source hits. Post-processing mutual
  exclusions, water tint magnitude, and light temperature invariants are
  preserved by the generic write boundary.
- Fourteen genuine non-schema controls remain intentionally narrow: eight
  renderer-owned terrain tool values use `TerrainToolField`, and six foliage
  brush values use `FoliageBrushField`. They are not represented as component
  properties and therefore do not reintroduce the deleted Details hand-wiring.

## Verification

- CONTROL-B generated inspector gate: green.
- CONTROL-B property-editor coverage gate: green.
- `somnium_ecs`: 39 unit + 15 integration + 4 doc tests green.
- `somnium_ui`: 230 tests green.
- `somnium_core --lib`: 132 tests green.
- Generic command tests cover ordinary undo/redo, live coalescing, no-op
  discard, and entity-scoped restoration.
- The schema round-trip test edits every editable serialized field through
  `SetFieldCmd`, saves, reloads, and compares the complete schema document.
- Generated reports reproduce with `python tools/reachability/generate.py
  --check`; `git diff --check` is clean.
- The preserved census records CONTROL-A at 676 identifiers and CONTROL-B at
  zero. The generated inspector covers all 160 editable fields across 16
  registered schemas.
- `editor/inspector.rs` is 410 lines versus CONTROL-A's 839;
  `editor_event.rs` is 346 lines versus CONTROL-A's 549.

## Exit

CONTROL-B's strangler migration is complete. The schema path is the production
Details path, the legacy property families and the handles they served are
deleted, both generated completeness gates are green, and the generic
round-trip/undo/coalescing coverage is green. The only controls outside that
path are tool/runtime or asset-authoring surfaces without component field
addresses; the latter is deliberately handed to CONTROL-C/CONTROL-D through the
asset editor hook rather than through legacy inspector enums.
