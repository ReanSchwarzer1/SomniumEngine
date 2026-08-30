# MORROWIND-M — virtualisation, data tables, the localisation editor

**Status:** step 1 partially complete, 2026-08-30. The virtualising container
exists and the outliner is retrofitted with the acceptance property measured
through the real draw path.

## The three items, and where this one stops

| Item | State |
|---|---|
| 1. A virtualising container, retro-fitted to the outliner, content drawer and asset browser | Container **done**; **outliner done**; drawer and browser **not started** — they are a different shape, see below |
| 2. A data table editor — typed columns, sorting, filtering, multi-cell edit, CSV | **Model done**, with the localisation table as its first customer; the grid widget is not built |
| 3. Asset dependency view, built on MORROWIND-Q's dependency graph | Not started |

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

## Why the drawer and the browser are not done

They are a **different shape**, and it matters. `refresh_content_list` builds
`content_entries: Vec<(NodeHandle, ContentEntry)>` — one real widget per asset —
so virtualising the drawer means *recycling widgets*: creating a fixed pool of
tiles and rebinding them as the window moves, rather than windowing a loop that
already had all the data.

That is the harder half of *"recycled rows"* and it is a change to how the
drawer is populated, not to how it is drawn. Doing it badly — recycling a tile
whose thumbnail request is still in flight, or losing the drop target under a
recycled handle — would break two features that work today.

Worth noting that the drawer is not naive about the viewport already:
`request_visible_thumbnails` promotes only tiles intersecting the scroll
viewport, so the *expensive* per-tile work is already windowed. What is not
windowed is the widget count.

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

## What this step does not claim

- No frame time was measured. The property was.
- The outliner is retrofitted; the content drawer and asset browser are not.
- Nothing here touches items 2 and 3 — the data table editor and the dependency
  view — and the localisation table, which item 2 names as its first customer,
  is still edited outside the editor.
