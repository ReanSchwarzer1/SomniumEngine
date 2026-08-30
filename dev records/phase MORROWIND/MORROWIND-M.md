# MORROWIND-M — virtualisation, data tables, the localisation editor

**Status:** items 1 and 3 complete, 2026-08-31. The virtualising container exists; the
outliner virtualises its draw and the content drawer virtualises its *widgets*,
both with the acceptance property measured rather than asserted.

## The three items, and where this one stops

| Item | State |
|---|---|
| 1. A virtualising container, retro-fitted to the outliner, content drawer and asset browser | Container **done**; **outliner done**; **content drawer done** — a different shape, see below. The asset browser is the drawer under another name and inherits it |
| 2. A data table editor — typed columns, sorting, filtering, multi-cell edit, CSV | **Model done**, with the localisation table as its first customer; the grid widget is not built |
| 3. Asset dependency view, built on MORROWIND-Q's dependency graph | **Done** — a project-wide reference graph and a References panel |

## The ceiling nobody had measured

The plan says as much: *"Acceptance is 100,000 rows at 60 fps; nobody has
measured the current ceiling and MORROWIND-A does."*

The good news first. `TreeView` is **one widget**, not one widget per row — it
owns `items: Vec<TreeItem>` and paints rows itself, so the widget count was
already O(1) and none of the usual retained-mode explosion applied.

The ceiling was in the draw loop:

```rust
for (i, item) in self.items.iter().enumerate() {   // every item
    …
    let selected = primary || self.selected_set.contains(&item.id);   // linear scan
    …                                                                 // shape the label
}
```

A widget inside a scroll viewer is **as tall as its content**, so its own bounds
say nothing about what can be seen. A hundred thousand entities meant a hundred
thousand rows laid out, shaped and painted every frame to show the thirty that
fit — and `Vec::contains` per row made the selection wash O(rows × selected) on
top. That is invisible at ten rows and hopeless at a hundred thousand, and no
amount of GPU makes it up: the work is on the CPU, before a primitive exists.

## What was built

`crates/somnium_ui/src/virtual_list.rs`, answering one question — *given a clip
rectangle, which rows can be seen?*

The seam is the **clip**, not the scroll offset. `DrawingContext` already
carried the clip a widget is drawing inside; it just had no accessor. Consulting
it means this module never learns what scrolling is and therefore cannot
disagree with the scroll viewer about it — the scroll offset arrives already
baked into the widget's screen bounds.

Three bugs the three panels would otherwise each write for themselves, and each
is now a test:

- **The row straddling the bottom edge.** A clip 30.5 rows tall must paint 31,
  or the last row is a gap where its top half should be.
- **Content scrolled above its clip.** A scroll viewer places content at a
  negative offset, and `as usize` on a negative float is a trap that springs
  exactly there.
- **The overscan row.** A scroll offset lands mid-row far more often than not,
  so the row above the clip is partly visible and has to be painted. Zero
  overscan is correct and flickers.

`KeySelection` is the other half. *"Stable selection across scroll"* means the
selection is held **by key, never by index** — an index is a position that
filtering, sorting, expanding a parent and scrolling all renumber, and storing
one is why a selection appears to jump when a list changes underneath it. Sorted,
so membership is a binary search rather than the scan it replaced.

## The acceptance property, measured

Not asserted about the arithmetic — measured through `TreeView::draw` itself,
counting the primitives it emits into a 660 px viewport:

```rust
#[test]
fn drawing_a_hundred_thousand_rows_costs_what_thirty_rows_cost() {
    let small = primitives_for(100);
    let huge  = primitives_for(100_000);
    assert_eq!(small, huge);
}
```

**A hundred rows and a hundred thousand rows emit the identical number of
primitives.** Before virtualisation the second was a thousand times the first,
and every one of those primitives carried a shaped label. A companion test
scrolls a thousand rows into the list and shows the cost is the same to within
one row — the one row being the top overscan row, which does not exist at scroll
zero.

**This is the property that makes 60 fps at 100,000 rows reachable; it is not
the frame time.** A `.somtime` run with a hundred thousand entities in the
outliner is the remaining half of the acceptance criterion, and it needs the
content drawer done too or the drawer becomes the new ceiling.

## The drawer is a different shape, and that is the whole story

The outliner virtualises its **draw**, because a `TreeView` is one widget that
paints rows itself. The drawer cannot: a tile is a real `Button` with a real
`Image` and a real `Text` inside it, and it is a drop target, a drag source and
a double-click target *by being one*. Windowing the paint would save nothing —
the widgets would still exist.

So in the drawer the window decides **which widgets exist**:

```
  ScrollViewer  ── clips, owns the scroll offset
      │
      └── Canvas ── explicit height = every row in the folder
               │      (so the scrollbar is honest about a folder
               │       whose widgets do not exist)
               │
               ├── tile 1078  placed at row 98, column 0
               ├── tile 1079  placed at row 98, column 1
               └── …          ~40 of them, never more
```

`refresh_content_list` now does one thing — ask the asset database — and stores
the answer in `content_all`. `sync_content_tiles` does the other: compute a
`GridWindow`, and if it differs from the one already built, build it.

Three consequences worth naming, because each was a decision:

- **A wrap panel could not do this.** A flow layout works out where the fourth
  tile goes by having been given the first three, so it needs all 40,000 to
  place the ones you can see. The container is a `Canvas` and every tile is
  placed by `place_node` at the rectangle its index *in the whole folder*
  earns. Nothing downstream knows the difference.
- **The canvas must not clip to its own bounds.** An empty folder is nought
  rows tall, and a canvas that cropped to its bounds would build the "this
  folder is empty" panel perfectly and then crop it out of existence — the
  blank grey rectangle that panel exists to replace. The scroll viewer above
  still clips, which is the clip that matters, and hit-testing consults the
  clip too, so the overscan row above the viewport is drawn nowhere and
  clickable nowhere.
- **Rebuilding is not recycling, and here it is the better answer.** Rebinding
  a pool would mean a tile whose thumbnail request is in flight being pointed at
  a different asset, and a drop target whose handle outlives the entry under it.
  The cost that matters is *bounded work per frame*, and building forty tiles
  when the window moves is bounded. The one thing that genuinely cannot be
  rebuilt under the user is an inline rename — a text box parented to a tile —
  so a rename holds the window still until it lands.

`request_visible_thumbnails` was already windowing the *expensive* per-tile
work against the viewport; it now runs over a list that is a screenful long
rather than a folder long, and is left otherwise alone.

### What the tests hold down

`GridWindow` is `RowWindow`'s grid, and it inherits the three bugs already paid
for there. Its own are the ones a grid adds:

- **The short last row.** Five assets in a four-column grid is two rows, and a
  window that returned eight indices would panic the caller on the three that
  do not exist.
- **The height is every row, not the visible ones.** The scrollbar is the only
  thing that tells the user 40,000 assets are down there.
- **A window scrolled past the end is still sliceable.** Walk into a huge
  folder, scroll to the bottom, walk into a small one: the scroll offset does
  not reset, so the window is asked about content that is gone. `first` past
  the end with a count of zero is still an out-of-bounds slice, and the drawer
  slices with exactly this range.

Two more are integration facts that `GridWindow` cannot see, and they are
tested through the real editor layout: a canvas given an explicit height makes
the scroll viewer scrollable to it *with only a screenful of children*, and the
canvas's screen `y` moves by exactly the scroll — which is the number the next
window is computed from. Get the second wrong and the drawer builds the right
number of tiles for the wrong part of the folder.

## Item 2 — the data table model

`somnium_ui::data_table` is what a table *is*: typed columns, keyed rows,
sorting, filtering, rectangular edits and CSV. The grid widget that draws it is
a projection, the same way `somui_editor` is one for the layout editor.

Separating it is not ceremony. A table editor's bugs are almost never in the
drawing — they are a sort that loses the selection, a filter that hides a row
you then edit by index, a paste that runs off the end, a CSV round trip that
eats a comma. Each is a property of the model, and each is a test.

Four decisions worth stating:

- **Rows are addressed by `RowId`, never by position.** Sorting and filtering
  renumber positions, and an edit applied to the wrong row because the view was
  re-sorted between the click and the commit is *the* classic data-grid bug.
  `visible_rows` returns ids for exactly this reason.
- **`Cell::Empty` is not `Text("")`.** A locale with no translation is not a
  locale translated to nothing, and an editor that cannot tell them apart cannot
  show you what is missing. The `only_incomplete` filter is the whole reason a
  translator opens the table.
- **Reversing a sort really reverses it, blanks included.** Pinning empties to
  the bottom in both directions is a common choice and it makes "show me what is
  missing" impossible in a long table.
- **A range edit is all or nothing.** A paste that writes four columns and fails
  the fifth must write none: half a paste is worse than none, because the undo
  the user reaches for no longer matches what happened.

Its first customer is live. `somnium_core::i18n::catalog_to_table` projects a
`Catalog` as keys down and locales across — the join lives beside
`CatalogResolver` for the same reason that does, so neither `somnium_ui` nor
`somnium_i18n` learns about the other. Keys come from the **union** of every
locale rather than from the default one, because a key that exists only in a
translation is usually a mistake and it is one nobody can see in a table that
lists only what the default locale has. `Table::keys` was added to
`somnium_i18n` to make that askable at all.

## Item 3 — the dependency view

Three questions, and the plan is right that they are one feature; they are not
one query:

```text
  what does this reference?     forward, direct   "open the texture this
                                                   material paints with"
  what references this?         reverse, direct   "who is using this?"
  what breaks if I delete it?   reverse, closed   a texture deleted breaks its
                                                   material, and every scene
                                                   that material appears in
```

The third has to be transitive, and it is the only one anybody asks with their
finger over the delete key.

### The graph MORROWIND-Q built is not this graph

`cook::AssetDependencyGraph` answers the same shape of question for a **cook
plan**, where every edge was declared by whoever wrote the plan. The editor
cannot ask that. Nobody declares that a scene uses a mesh — they drop the mesh
onto an entity, and the edge exists because a field now holds an id.

So `somnium_asset::depend` reads the project and works the edges out. One
decision carries the module:

**The scan is structural, not schema-driven.** An asset id reaches a file in one
of two spellings — the scene schema tags its references so they survive a round
trip through a generic value (`{"$asset": "…"}`), and a material declares typed
`AssetId` fields that serialise as the bare string — and both are 32 lowercase
hex digits. Walking the JSON for that shape means one scanner covers scenes,
prefabs, materials, `.somui` documents and anything added later. The
alternative, teaching this module the component schemas, would mean a new asset
field quietly missing from the dependency view until somebody remembered to
teach it twice, and the place people forget is exactly the place where "what
breaks if I delete this" starts lying.

Three things the scan refuses to guess at:

- **A file it cannot read is counted, not called empty.** A `.glb` names its own
  textures and a script names assets by path; neither is JSON. `ScanSummary`
  separates *scanned*, *opaque* and *unreadable*, and the panel says so in
  words. A dependency view that reports a mesh as referencing nothing is worse
  than no dependency view, because it is trusted.
- **A reference to something absent is dangling, not dropped.** It is the trail
  of a file deleted outside the editor, and it is the one thing here that can be
  seen *before* it becomes a mystery at runtime.
- **A cycle is walked once.** A prefab that spawns a scene that places the
  prefab is legal, and an infinite walk for any closure without a visited set.

### In the editor

A third tenant of the bottom row, beside the Content Drawer and the Output Log,
because it answers a question you ask *about* a drawer selection and want to
read next to it. Right-click any asset → **Show References**, or the References
button in the status bar, or `editor.window.references` from the Window menu and
the command palette.

Every row is a link: clicking one asks the same three questions about what it
names, so a texture two steps under a scene is two clicks away in either
direction. A row for an id with no record shows the raw id in the warning colour
and is not a link, because there is nowhere to go.

Two pieces of restraint in the panel, both about not training people to ignore
it. "Breaks if deleted" is only shown when it is *larger* than "used by" —
otherwise it is the same three rows under a scarier heading. And a folder gets a
sentence rather than three empty lists, which would read as "safe to delete"
about a folder whose contents are anything but.

**The index is built on the inventory's own job.** Not a second scan and not a
lazy build: the same background job that scans the content root then opens every
scannable file in it, and the snapshot and the graph are handed to the shell
together. A graph one scan behind the drawer would name assets the drawer cannot
show, and the bug that produces looks exactly like a broken reference.

### Verification

Seven unit tests on the model, and one that writes a real three-link project to
disk — a texture, a `.sommat` that paints with it, a framed `.somnium` scene
that places the material — scans it with `AssetDb::scan` and reads the chain
back out. That last one is the half a unit test cannot reach: a scene is a
binary header followed by its document, so the file is not JSON even though its
body is, and the summary counts prove which files were opened and which were
skipped.

## What this step does not claim

- No frame time was measured. The property was, in both panels.
- One pre-existing edge is unchanged and now easier to see: a scroll viewer
  clamps its offset on input, not on layout, so walking from a deep folder into
  a shallow one leaves the drawer scrolled past its own content until the next
  wheel event. It showed a blank drawer before this change and shows a blank
  drawer after it; the windowed build is simply the reason it is worth writing
  down.
- A freshly built tile has no bounds until the next arrange, so thumbnails for
  a row scrolled into view are requested one frame later. The thumbnail pump is
  already asynchronous and bounded per frame, so this is a frame of icon rather
  than a frame of nothing.
- Item 2's grid widget is not built. The model is done and the localisation
  table, which the plan names as its first customer, is still edited outside the
  editor.
- The reference graph reads scenes, prefabs, materials, `.somui` documents and
  plain JSON. Meshes, scripts and shaders keep their references in formats it
  does not parse, and it says so rather than reporting them as unused.
- The whole index is rebuilt whenever the inventory is. That is one pass over
  the project's JSON per real change on disk, on a background job; an
  incremental rebuild is a thing to want at a project size nobody here has.
