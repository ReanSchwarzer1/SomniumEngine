# CONTROL-D — material authoring verification

Date: 2026-08-23

## Implemented

- `.sommat` is the canonical authored material document. Its versioned header
  carries an embedded PNG preview, while its body preserves base colour and
  opacity, metallic, roughness, emissive colour and intensity, transmission,
  alpha mode and cutoff, double-sided/foliage flags, and five texture
  `AssetId` slots.
- `MaterialComponent` stores the authored material `AssetId`; renderer pool and
  bindless texture indices are derived session state and are omitted from scene
  serialization. Version-1 numeric material references migrate honestly to an
  unset asset rather than masquerading as stable authored identity.
- Material fields use the schema-generated Details panel and generic field
  command/gesture/undo path. The panel's generic optional preview cell displays
  the shared sphere thumbnail, and `OnPropertyChange` edits invalidate it
  immediately.
- All five texture references are resolved through the asset database and
  bounded worker jobs, then uploaded on the main thread and reflected into the
  shared GPU material slot.
- New Material is a registry command available from both Create and the Content
  context surface. Multi-entity assignment is one undo step; Make Unique copies
  to a collision-safe sibling, assigns the new `AssetId`, and undo restores the
  assignment without deleting authored content.
- glTF import writes collision-safe editable `.sommat` siblings and materializes
  embedded textures. Imported render nodes retain the authored material
  association while runtime slots remain derived.
- The polished-metal proof round-trips roughness `0.2` and metallic `1.0`
  through reflected material editing, scene serialization, material reload,
  and the 80-byte GPU payload.

## Deterministic verification

```text
cargo test -p somnium_asset --lib -j1       20 passed
cargo test -p somnium_core --lib -j1       141 passed
cargo test -p somnium_renderer --lib -j1   315 passed
cargo test -p somnium_ui --lib -j1         228 passed
                                             704 passed total
```

Focused proofs include the complete `.sommat`/five-texture/header round-trip,
glTF sibling and embedded-texture output, polished GPU reconstruction, material
scene-schema identity versus runtime-state omission, a 16-row generated
material panel with five Asset pickers and no unsupported fields, thumbnail
invalidation, registry command reachability, vector assignment as exactly one
undo step, and Make Unique undo semantics.

The workspace all-target check is warning-free. Generated reachability output,
the opt-in CONTROL-B gates, package format checks, and `git diff --check` are
also green.

## CONTROL-J boundary

CONTROL-D's deterministic tests prove material and scene documents can be
written, reloaded, and reconstructed without persisting renderer pool IDs. This
record does **not** claim a literal editor Save -> quit -> reopen visual exit:
`EditorEvent::LoadScene` still routes to the version-2 map-recipe loader instead
of dispatching `scene.somnium` through `scene_schema::load_scene_schema`, and
the editor must then rebuild GPU resources from the loaded authored
components. That routing and reconstruction work is explicitly owned by
CONTROL-J. Once CONTROL-J lands, the manual quit/reopen viewport proof must be
run; it is not fabricated here.
