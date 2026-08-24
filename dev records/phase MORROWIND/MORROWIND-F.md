# MORROWIND-F — input routing, focus and gamepad navigation

**Items 1–4 complete, 2026-08-24. Item 5 is blocked on MORROWIND-AE and the
plan says so.** Track 1 (VIVEC).

> §8, item 5: *"Consumes MORROWIND-AE's action map for navigation verbs, so a
> player's rebound 'confirm' works in menus. **This is a forward dependency and
> Track 8 must land AE before F closes** — noted in §9."*
>
> So this record does not claim F is closed. What it claims is that the four
> items that do not depend on Track 8 are done, and that the fifth has a seam
> rather than a hole.

## What already existed

Phase CONTROL-A1 shipped more of this than the plan's item list implies, and
finding that out first is what kept F from rebuilding it:

- focus state, `focused()` / `set_focus()`
- a **modal focus trap** with focus return on dismissal (WCAG 2.4.11)
- **linear** traversal — up/down/Home/End over a region's focus stops
- focus-into-view scrolling through every ancestor
- rect-based `hit_test` and `is_under`

And Phase 27 shipped the **focus visual**: `draw.rs`'s
`the_focus_ring_is_the_only_state_that_glows` pins it as the one state carrying
a glow, inside Zeta's four-cue grammar. §8 item 2 asks for a focus visual "that
satisfies Zeta's four-cue state grammar rather than inventing a fifth cue" —
**so F invents nothing and consumes what is there.** Adding a second focus cue
would have been the failure.

## What F adds

### Two-dimensional navigation (item 3)

Linear traversal is right for a Details panel and wrong for a menu: a 4x3
inventory has no meaningful linear order, and Tab through it visits cells in an
order nobody predicted. `runtime/nav.rs` adds directional navigation beside it,
without replacing it.

The design is the plan's, and both halves of its reasoning are real:

- **Explicit links alone** mean every button in a settings screen names four
  neighbours, and inserting a row means editing eight of them. Nobody keeps that
  correct, and the failure is *silent*: navigation still works, it just goes
  somewhere surprising.
- **Geometry alone** is right almost always and wrong exactly where it matters.

So an authored link wins; otherwise geometry decides. Two details carry the
weight:

**Off-axis distance is weighted ten times on-axis distance.** This is the whole
difference between "picks the nearest widget" and "picks the widget a person
meant". `alignment_beats_raw_proximity` is the test: a candidate directly right
but 200 px away beats one 50 px away and diagonally up, because pressing Right
on a d-pad should follow the row. Nearest-centre scoring gets that backwards,
and that is §8's "geometric search picks the wrong widget in dense layouts".

**An authored link to an unlisted widget is still honoured.** Second-guessing it
makes the feature useless where it is needed — a link into a collapsed panel, or
onto a widget the caller did not enumerate. The author knows something the
geometry does not; that is the only reason to author one.

Three smaller decisions, each with a test: a one-way link stays one-way (a "back
to the list" edge is genuinely asymmetric, and inferring the reverse would
overwrite an author's other choice); `forget()` drops links in *both* roles,
because a stale link into a generational pool lands focus on whatever was
allocated in the freed slot; and scoring uses `total_cmp`, because a zero-sized
widget mid-layout is real and `partial_cmp().unwrap()` on its NaN score panics.

### Hit-testing transformed shapes (item 1)

`UserInterface::hit_test` is rect-based, which was correct while every primitive
was an axis-aligned rectangle. MORROWIND-D added transforms, so a rotated widget
would be clickable in the wrong place — the classic symptom being a control that
responds when the pointer is beside it.

`ShapedBuffers::hit_test` walks in **reverse paint order**, inverts each
candidate's transform to reach local space, and tests point-in-triangle against
the geometry that is already there. No second representation to keep in step
with the drawn one, which is the property that stops the two drifting.

Two things it gets right on purpose:

- **A singular transform is skipped.** Its inverse is a NaN, every comparison is
  false, and the widget reads as present but unclickable — with nothing in the
  log.
- **The point-in-triangle test is inclusive on edges.** An exclusive test leaves
  a one-pixel dead line along the diagonal seam of every quad, which is
  maddening to diagnose and trivial to prevent.

**What it does not test: the mask texture.** A masked shape's alpha lives on the
GPU and reading it back per pointer move would cost a stall, so a circular
avatar built from a rectangle plus an alpha mask hit-tests as its rectangle.
That is a real limitation, stated in the doc comment rather than hidden, and the
fix where it matters is to build the shape as a *path* — then the geometry is
the shape and the hit test is exact.

### One event stream, and hover that admits what it is (item 4)

`InputSource` is `Pointer` / `Touch` / `Gamepad`, with `has_hover()` and
`navigates()`.

§8: *"hover has no meaning on a pad and the API must say so rather than
pretending."* The pretence is a specific bug: a UI that treats the focused
widget as hovered shows a hover highlight **and** a focus ring on one control,
so a pad user sees two cues meaning one thing while a mouse user sees them mean
two different things. Zeta's grammar has four cues and hover and focus are two
of them; collapsing them leaves three.

Touch is separate from pointer for the same kind of reason: a touchscreen has a
position only while touching, so a hover state there is one the user cannot see
themselves enter.

### The AE seam (item 5)

`NavAction` — `Move(Direction)`, `Confirm`, `Cancel`, `Next`, `Previous` — is
the vocabulary. `NavAction::from_key` is a hard-coded keyboard default and is
**deliberately a free function taking a key**, not a match buried in the widget
tree, so MORROWIND-AE replaces one call site instead of hunting for keycodes.
Seam 5's whole point is that keycodes appear in exactly one place.

`Tab` maps to `Next`, not to `Move(Down)`, and the distinction is not pedantic:
a two-column settings screen has one Tab order and two Down chains, and
collapsing them makes one of the two wrong.

## The gap E found is F's blocker too

MORROWIND-E's record ends on it and F confirms it from the other side:
**`EngineContext` has no UI hook.** A game cannot draw a HUD, and equally cannot
receive input into one — there is no widget tree in the frame for events to
route to.

Everything above is therefore verified as *logic*: navigation over candidate
rects, hit testing over buffers, source classification. None of it is verified
as *routing*, because there is no game-side tree to route into. E proposed the
fix (an owned `UiCanvas` on the app, a field on `EngineContext`, a draw after
the editor's pass) and said E and F should land it together; they did not, and
that is now the one thing standing between Track 1 and a game with a working
menu.

## Tests: 20 new, 407 in the crate, 0 failures

- **`nav`, 14** — a toolbar navigating along itself; **alignment beating raw proximity**; a 4x3 grid in both axes; an explicit link overriding geometry and one pointing outside the candidate list; one-way links staying one-way; `forget` clearing both roles; a distant off-axis widget refusing to be a neighbour; touching widgets being neighbours; a degenerate rect not panicking; `first_focus`; hover restricted to pointers; Tab distinct from Down; conventional Confirm/Cancel keys.
- **`shaped`, 6** — a point inside an untransformed shape; **hit testing following the transform**, with the untransformed-bounds corner explicitly asserted to *miss*; reverse paint order and `hit_test_all`; a singular shape not hit; the diagonal seam not a dead line; an empty frame.

## Files

```
+ crates/somnium_ui/src/runtime/nav.rs   Direction, InputSource, NavAction,
                                         NavLinks, navigate, first_focus
~ crates/somnium_ui/src/shaped.rs        ShapedBuffers::hit_test / hit_test_all
~ crates/somnium_ui/src/runtime/mod.rs   pub mod nav + re-exports
```
