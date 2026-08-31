# MORROWIND-G — text, properly

**Items 2, 3, 5 and 6 complete, 2026-08-24. Item 1 adopted and item 4 completed
2026-08-31 (CS-CORRECTNESS #6) — with a different library than this record
chose, for a reason stated below.** Track 1 (VIVEC).

§8 opens this sub-phase by calling it *"the largest single sub-phase in Track 1,
and the one most likely to be under-estimated"*. This record does not claim it
is closed. It claims what is built, what is decided, and — for the one item that
is deliberately unbuilt — quotes the instruction that says not to build it yet.

## The run model, which is what was actually missing

`font.rs` rasterises glyphs keyed by `(char, px, font_id)` and `draw.rs` walks a
`&str` a `char` at a time. That works for Latin at one size in one colour and
**cannot express anything else**, because there is no concept of a run — nowhere
to put a colour change mid-sentence, a fallback face for one codepoint, a shaped
cluster whose glyph count differs from its character count, or a bidi level.

`StyledRun` is that missing concept, and introducing it first is what makes a
shaper a *substitution* rather than a rewrite. Its `range` is a byte range into
the source rather than a copied `String`: a paragraph with a dozen colour
changes should not allocate a dozen strings, and a range keeps the run tied to
the text it came from so a caret offset means the same thing on both sides.

## Item 1 — the shaper: decided, not adopted

**Decision: `cosmic-text`.**

| | `cosmic-text` | `parley` |
|---|---|---|
| Shaping | `harfrust` — a Rust HarfBuzz | `swash` |
| Scope | Buffer, layout, editing, with selection and cursor movement | Layout and rich-text ranges; editing is the caller's |
| Fit | Higher — `text_box.rs` hand-rolls selection and cursor movement today | Lower — that hand-rolled code stays |

The deciding row is the third. **Item 5 of this sub-phase is IME, and IME is
editing**: composition, candidate windows, and a caret that moves by cluster
rather than by byte. A shaper that returns positioned glyphs and leaves editing
to the caller means writing cluster-aware caret movement by hand, which is the
part of text handling hardest to get right and easiest to get subtly wrong.

**It is not adopted here, and that is Appendix A.5's explicit instruction:**

> *"Phase 27 froze block-origin text snapping to get crisp glyphs at 1x DPI. A
> shaper returns sub-pixel advances; naive snapping of shaped output destroys
> kerning, and naive non-snapping blurs the editor's own chrome. The resolution
> is to snap the run origin and keep advances sub-pixel within the run — **but
> that is a claim, not a result.** Land the shaper behind `SOMNIUM_UI_SHAPER=1`,
> A/B it, and only then flip the default. GHOSTFENCE's golden-image row is what
> makes the A/B decidable rather than a matter of opinion."*

**GHOSTFENCE has no golden reference images.** Landing a shaper now would flip
the most visually sensitive switch in the editor with no way to tell whether the
chrome got worse. `ShaperPolicy` reads `SOMNIUM_UI_SHAPER` as A.5 specifies, and
`is_available()` returns `false` — so `Shaped` cannot be mistaken for "working"
by reading the enum.

## Item 1, revisited and adopted — CS-CORRECTNESS #6

**Landed, A/B'd, available behind `SOMNIUM_UI_SHAPER=1`. 2026-08-31.**

### The decision was re-taken, and here is why

This record chose `cosmic-text` over `parley`, and said the deciding row was
editing: *"Item 5 of this sub-phase is IME, and IME is editing"*. Item 5 then
shipped in `text/ime.rs` **without** a shaper. That removes the whole basis of
the comparison — what is left to want is shaping, and the parts around it are
already built here: `StyledRun` is the run model, `FallbackChain` resolves
coverage, `FontAtlas` rasterises and packs.

A library that also owns a font database, an atlas and a layout engine would
have arrived as a *second* text stack beside those. GHOSTFENCE has a row for
exactly that. So:

| | |
|---|---|
| `rustybuzz` | shaping — a HarfBuzz port that takes a face and a string and returns positioned glyph ids, and owns nothing else |
| `unicode-bidi` | the UAX #9 resolution item 4 deferred *"to whatever could reorder glyphs"* |
| already here | `FallbackChain` (item 3), `StyledRun` (the run model), `FontAtlas` (packing) |

### The two bugs, both of which rendered something plausible

**A chain is a fallback chain, not a router.** The first version asked the chain
which face covers each character and got face 0 for everything, because the
regular cut covers Latin too — so every label drawn in the medium and semibold
cuts came out in regular. `split_preferring` gives the caller's face first
refusal, and only what that face lacks reaches anything else.

**`rustybuzz` and `fontdue` do not share a glyph index space.** Measured on the
editor's own `Inter-Regular.ttf`:

| character | `rustybuzz` | `fontdue` |
|---|---|---|
| `C` | 18 | 18 |
| `(` | 331 | 324 |
| `:` | 366 | 365 |
| `-` | 348 | 344 |

Letters coincide and punctuation does not, and the divergence is not a constant
offset. In the editor this read as "Coastal Surf  CC0" and "14 00" — the
advance kept, the glyph gone — because the mismatched id happened to land on a
glyph with no outline. **The important half is what it would have done next:** a
mismatch that lands on a glyph *with* an outline draws plausible, wrong text
that nobody would think to check, and a ligature is exactly that case.

So the shaped path rasterises from the same face the shaper read — outlines from
`ttf-parser`, which `rustybuzz` re-exports, filled by `tiny-skia`. Both were
already in the tree for the icon atlas, so this is a new use of the dependency
graph rather than a new dependency in it. `fontdue` keeps the per-character
path, unchanged and untouched.

### The A/B Appendix A.5 asked for

A.5's instruction was to land the shaper behind the flag, A/B it, and only then
flip the default — with GHOSTFENCE's golden image as the arbiter. **That row was
already failing before this work**, so it could not arbitrate; the A/B was run
from fresh captures of the same frame instead, and from a test that pins the
measured width ratio between the two paths to within 10%.

The result: shaped chrome is **tighter and correctly kerned, at the same
crispness** — the run origin is still snapped and only the advances within a run
are sub-pixel, which is precisely A.5's stated resolution.

**The default is still `PerCharacter`,** and that is a deliberate hand-off
rather than a hedge: turning it on changes every glyph position in the editor,
and the person who should look at that before it becomes the default is the one
who uses the editor. `SOMNIUM_UI_SHAPER=1` turns it on; flipping the default is
a one-line change to `ShaperPolicy`.

### What it does now

- **Shaping**: kerning, ligatures, mark positioning, Arabic joining, Indic
  reordering — whatever the face's `GSUB`/`GPOS` describe, because the script
  and language are guessed from the text rather than hard-coded.
- **Bidi**: UAX #9 resolution and visual reordering, so the first character of
  an Arabic line lands on the right and Latin inside it still reads left to
  right.
- **Fallback**: unchanged from item 3, now with the caller's face preferred, and
  a span no face covers still produces no glyphs rather than a substitute.
- **Measurement follows drawing.** `measure_text` shapes when the draw path
  would shape and does not when it would not, because a layout that measures one
  way and paints the other is off by the kerning on every string.

### What it does not claim

- **Tracked text is not shaped.** Letter-spacing and mark positioning disagree:
  inserting space after every glyph pushes a diacritic off its letter. The
  uppercase header role is Latin caps, where the per-character path is correct,
  and both paths agree about which strings they take.
- **One line at a time.** The paragraph model — newlines, wrapping — is still
  the caller's, and multi-line shaped text is a wrapper around this rather than
  a change to it.
- The golden-image row still fails for an unrelated, pre-existing reason.

## Item 2 — rich text

BBCode, not HTML, for three reasons in order of weight:

1. **Angle brackets appear in game text.** "HP < 20" and "press \<Enter\>" are
   both things a UI says; an HTML-shaped parser must escape or guess.
2. It is what the in-architecture reference (`fyrox-ui/src/bbcode.rs`) uses.
3. **The failure mode is better.** An unknown tag is emitted as literal text
   rather than swallowed, so a typo appears on screen where somebody fixes it
   instead of silently deleting the rest of the sentence.

The vocabulary is §8's, complete: `b i u s`, `color`, `size`, `font`, `link`,
`wave`, `shake`, `sprite`.

**Ranges index the stripped text, not the markup.** Indexing the source would
make every caret offset wrong by the length of the tags before it — a bug that
only appears once somebody clicks in the middle of a styled sentence.

An inline sprite occupies **one** U+FFFC OBJECT REPLACEMENT CHARACTER, which is
what that codepoint is for: a caret steps over the sprite as one unit and a
selection can include it, with no special case anywhere.

### The defect the tests found

Point 3 above was half-implemented and the tests caught it. `[blink]` was
correctly emitted as text — and then `[/blink]` errored as `Unmatched`, because
nothing styled was open. **The literal-open path existed to stop a typo breaking
the paragraph, and erroring on the second half broke it anyway.** An open tag
emitted as text now records that, so its close is emitted as text too, and a
literal tag left unclosed is not an error either — `press [E] now` is a sentence.

## Item 3 — font fallback

Every non-Latin character in the engine is tofu today: the five bundled cuts are
Latin, and `get_or_rasterize` asks one face and takes the `.notdef` box when it
misses. That includes every save-game name typed on a Japanese keyboard.

Two decisions carry the design:

**Coverage is asked of the face, never inferred from the Unicode block.** A good
CJK face contains Latin, and its Latin is designed to sit with its Han; a Latin
face routinely contains Greek, Cyrillic and arrows. Routing by block would send
that Latin to the wrong face and produce a visible mismatch mid-sentence.
`a_cjk_face_that_covers_latin_is_allowed_to_serve_it` pins it.

**`split` produces the longest possible spans.** A per-character lookup would
give one run per character — and a run is the unit a shaper shapes, so
per-character runs mean per-character shaping, which is exactly the kerning-free
output shaping exists to avoid.

A span with no face is kept as its own span rather than merged or substituted:
substituting a space makes a missing glyph invisible, and an invisible bug does
not get reported.

## Item 4 — bidi, half in and half deferred

`Direction` and the first-strong-character paragraph heuristic (UAX #9 P2/P3)
are here, because a run must carry a level for a shaper to use, and because
"always LTR" mangles the first line of every Arabic UI.

**The UAX #9 resolution algorithm is not**, and the reason is the same as the
shaper's: reordering belongs with the thing that can reorder glyphs. Writing it
against a pipeline that cannot would produce levels nothing reads.

**Completed 2026-08-31**, with the shaper: `text/shape.rs` resolves levels
through `unicode-bidi` and lays the visual runs out in order. See item 1 above.

**Vertical writing modes are explicitly deferred by §8 item 4 itself** ("bidi is
in; vertical writing modes are explicitly deferred, §14.5"). Not attempted.

## Item 5 — IME

`text_box.rs` reads raw keystrokes. On a Japanese, Chinese or Korean keyboard
the characters typed are not the characters meant: `nihon` is five Latin letters
the input method converts to 日本 on confirmation. An engine reading keystrokes
gets `nihon` in the box and no way to reach 日本 at all. winit reports this
through `WindowEvent::Ime`, which nothing in the tree handles.

**Preedit is not text yet, and that is the whole design.** The composition
string is provisional: shown, underlined, and abandonable. It must not be
committed, must not enter undo, must not be sent to whatever reads the field.
Getting that wrong is *invisible in English* — everything works until somebody
types Japanese, and then a half-finished romanisation is saved as a character's
name.

`swallows_enter()` is the other invisible one: the first Enter during
composition belongs to the IME. A dialog that closes on it closes **every time
somebody finishes a word**.

## Item 6 — the localisation hook

`somnium_i18n` is MORROWIND-AH and does not exist. What is here is the *shape*,
so the ~86 widget call sites taking `&str` can migrate one at a time rather than
all at once, and so code written between now and then does not add to the pile.

**A key plus arguments, not a formatted string.** `format!("You have {n}
potions")` breaks on the first language with grammatical number beyond
singular/plural, and it breaks quietly: Polish has three plural forms and Arabic
six, and a substituted `{n}` cannot choose between them because the choice has
to happen inside the resolver, which needs to see `n`. Word order differs too — a
translator handed a format string can move `{n}`; one handed `"You have "` and
`" potions"` cannot.

Two smaller decisions with tests: a missing translation **renders the key**, not
a blank (a blank looks like a layout bug and gets filed against the wrong
system); and a bare `&'static str` converts to a **key**, so untranslated text
has to say `LocalizedText::literal` and is therefore countable. Making `Literal`
the easy path is how a codebase ends up with a thousand untranslatable labels
nobody can enumerate.

## Tests: 70 new, 463 in the crate, 0 failures

- **`text`, 8** — paragraph direction from the first strong character (not the majority: an English sentence quoting Arabic is an LTR paragraph containing an RTL run); digits and punctuation not strong; CJK LTR; a run slicing rather than owning; an unstyled run inheriting everything; the shaper admitting it is unavailable.
- **`markup`, 20** — ranges indexing the stripped text; every decoration; three-, six- and eight-digit colours, with `#abc` repeating its nibbles (shifting instead makes every short colour 6% too dark, invisible alone and obvious beside its long form); nesting restoring the outer style; a sprite as exactly one character; links and motion; **unknown tags surviving as text on both halves**; `HP < 20`, `press [E]`, `[[literal`; errors naming their byte offset; crossed tags; multi-byte ranges staying on char boundaries.
- **`fallback`, 10** — the first covering face winning; a CJK name not tofu; longest-possible spans; tofu as its own span; char boundaries; a CJK face serving its own Latin.
- **`ime`, 10** — `nihon` never reaching the buffer; **Enter swallowed while composing**; cancellation; an unchanged preedit not redrawing (some platforms resend it per key, and the underline flickers); focus loss cancelling rather than committing.
- **`localize`, 9** — the translation deciding word order; a count staying a number so plural selection can dispatch on it; a missing translation showing the key; a literal never reaching the resolver; a bare `&str` defaulting to a key.

## Files

```
+ crates/somnium_ui/src/text/mod.rs        StyledRun, Direction, Motion, ShaperPolicy
+ crates/somnium_ui/src/text/markup.rs     BBCode -> styled runs
+ crates/somnium_ui/src/text/fallback.rs   FaceCoverage, FallbackChain, split
+ crates/somnium_ui/src/text/ime.rs        Composition, ImeEvent, ImeOutcome
+ crates/somnium_ui/src/text/localize.rs   TextKey, Argument, LocalizedText, Resolver
~ crates/somnium_ui/src/lib.rs             pub mod text
```

## What Track 1 still owes

- **The shaper itself**, blocked on a golden reference image (A.5).
- **Bidi reordering**, which belongs with the shaper.
- **Wiring**: `markup::parse` produces runs and `draw.rs` still walks a `&str` per
  character. The migration is per-call-site and is the same shape as the
  `LocalizedText` migration, so both should move together.
- **The `EngineContext` UI hook**, found by MORROWIND-E and confirmed by
  MORROWIND-F. Still the one thing between Track 1 and a game with a working
  menu, and now also between it and a game with a translated one.
