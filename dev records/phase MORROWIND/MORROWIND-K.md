# MORROWIND-K — one graph surface

**Complete, 2026-08-25.** Track 2 (CONSTRUCTION SET), Seam 8a.

This sub-phase adds one retained-mode graph control and one feature-neutral
model. Material, animation, behaviour trees, VFX and scattering contribute
catalogue data; none gets to create another surface.

## The surface

`somnium_ui::graph` supplies:

- `GraphEditor`, a retained-mode `Control` that draws node bodies, typed pins,
  selection, a zooming grid and cubic wires through MORROWIND-D's shaped path
  stream. Pointer gestures select, box-select, pan, zoom, move and connect.
- `GraphSurface`, the authoring controller for comments, nested groups,
  alignment, reconnect, disconnect, typed reroutes, copy/paste and sub-graph
  breadcrumb contexts.
- `NodeArchetype`, `NodeElementArchetype`, `GroupArchetype` and `Catalogue`.
  The built-in material and animation catalogues are the two-consumer proof;
  the surface contains no material or animation branch.
- Transactional edits. A rejected reconnect, malformed reroute or invalid paste
  restores the exact prior graph. A live drag records one history entry on
  release. Undo, redo, copy, paste and delete use CONTROL-A2's registered Edit
  command ids rather than defining another shortcut or menu table.

Node identities allocate monotonically and never recycle after deletion. Group
membership, literals, sizes and the sub-graph path are authored data;
selection, pan and zoom are view state and do not leak into the asset.

## Asset contract

`graph::serial` writes deterministic JSON with an explicit version and
catalogue id. Loading validates archetypes, literal pin indices, group cycles,
pin types, input cardinality and graph cycles before returning a `Graph`.
Version 0 migrates to the current monotonic-id cursor; future versions and a
graph opened under the wrong catalogue are refused rather than guessed.

## First consumer: material graphs

`graph::material::compile` follows only nodes that reach the catalogue's
material root. It produces:

1. CONTROL-D's existing `somnium_asset::material::MaterialAsset`, so property
   authoring and graph authoring have the same runtime object; and
2. deterministic WGSL installed through MORROWIND-C's single `ShaderSystem`.

Generated shader modules receive independent anonymous `ModuleId`s. They do
not leak asset names into `'static` storage, shadow named include modules, or
replace one another. Naga validates the generated source in the graph tests.

`examples/vvardenfell` constructs a colour-to-surface graph and compiles it
through public `somnium_ui` APIs. It reaches neither renderer internals nor an
editor-only material type.

## Verification

- Focused graph tests cover the model, widget draw list, transactions,
  CONTROL command routing, deterministic round-trip, migration, catalogue
  isolation, material equivalence and Naga validation.
- `somnium_shader` tests cover independent generated module ids in addition to
  the existing composition, validation and reload contract.
- `cargo check -p vvardenfell` exercises the public second-example boundary.
- Full `somnium_ui`: **555 unit + 6 shader + 1 doc tests passed**.
- Full GHOSTFENCE, using a temporary target outside OneDrive to avoid its
  transient Windows executable locks: **7 rows passed, 0 failed, 0 skipped;
  1,775 tests passed, 0 failed**.

No PNG is invented for this record. The concrete `GraphEditor` is asserted at
the frozen primitive/shaped draw-list boundary; a dedicated material-editor
window capture belongs to the first shell window that hosts the control, not to
a synthetic raster produced outside the renderer.

## References

Godot `scene/gui/graph_edit` / `graph_node`, Fyrox's ABSM editor, and O3DE
GraphModel/GraphCanvas and Material Canvas were read for permissive patterns.
The clean-room distinctions and Flax's explicit exclusion are in
`ATTRIBUTION.md` §13H.17.
