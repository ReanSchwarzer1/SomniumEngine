# MORROWIND-AH — localisation, video, and the boundary

**Item 1 complete, 2026-08-24. Items 2 and 3 are open and stay open by
design.** Track 8 (ALMSIVI). §8 calls this *"the phase's acceptance test"*,
which is true of item 3 and of nothing else in the sub-phase.

**This record was written on 2026-08-25, a day after the commit.** The commit
`9dc6c09` shipped without an evidence file, without an `ATTRIBUTION.md` §13H
entry and without a `context.md` update — three of the five things §8 says a
sub-phase closes with. That is recorded here rather than quietly fixed, because
the gap is the interesting part: **AE, AG and AH all shipped code and none of
them updated the record**, and a phase that is thirty-six sub-phases long cannot
survive that habit. All three are reconciled in the same commit as this file.

## Item 1 — `somnium_i18n`

`crates/somnium_i18n`, 1,954 lines over five files, plus 183 lines in
`somnium_core::i18n` joining the crate to the trait MORROWIND-G left behind.

**The dependency list is the design.** The crate depends on `serde` and nothing
else. It does not know what a widget is, what a font is, or what a frame is —
which is possible only because **MORROWIND-G put the localisation hook in
`somnium_ui` as a `Resolver` trait** rather than as a call to a concrete
catalogue. That decision is what let item 1 land as a leaf crate instead of as a
change to the UI.

| File | Lines | What it is |
|---|---:|---|
| `lib.rs` | 593 | `Catalog`, the fallback chain (`pt-BR` -> `pt` -> `en`), runtime locale switching |
| `extract.rs` | 627 | scans source for keys **and** for the strings that never became keys |
| `plural.rs` | 372 | CLDR categories; Polish's last-two-digits rule; Arabic's six |
| `number.rs` | 204 | `1,234.5` / `1.234,5` / `1 234,5`, and `es-US` grouping like English |
| `gender.rs` | 158 | the same selection mechanism on a different variable |

### The extractor is the half that is not obvious

A translation-file check can tell you that a key is missing a translation. It
cannot tell you about a string that **never became a key** — a literal sitting
in a `format!` in a widget constructor, which is the only kind of localisation
bug that survives a green test suite. Ren'Py's `renpy/translation/` is the
reference for this specifically (`ATTRIBUTION.md` §13H.13) and it is genuinely
better at it than the engine references are.

### The fallback chain is a chain, not a default

`pt-BR` -> `pt` -> `en` rather than `pt-BR` -> `en`. The middle link is the one
that matters: a Brazilian Portuguese build with an untranslated string should
show European Portuguese before it shows English, and a two-step fallback cannot
express that.

## Tests: 57 in `somnium_i18n`, 7 in `somnium_core`, 0 failures

- **`lib`, 18** — the fallback chain including the middle link; an unknown locale
  resolving rather than panicking; runtime switching; argument substitution; a
  missing key returning the key rather than an empty string, so a bug is visible
  in the UI instead of invisible.
- **`extract`, 13** — keys found; **literals found**; a literal inside a comment
  not found; a key built by concatenation reported as unextractable rather than
  silently missed.
- **`plural`, 11** — Polish's 12/13/14 versus 22/23/24; Arabic's six categories;
  English paying for exactly two; a language with one category never branching.
- **`number`, 11** — three grouping conventions; `es-US` grouping like English
  rather than like Spanish, which is the case a locale-code prefix match gets
  wrong.
- **`gender`, 4** — selection, fallback to `other`, English costing nothing.
- **`somnium_core::i18n`, 7** — the trait joined to the data; a resolver absent
  leaving text unchanged rather than blank.

## Item 2 — video: not started, and cheaper than the plan assumed

§6.9.2 found `vk-video`, which decodes through Vulkan Video **directly into a
`wgpu::Texture`**, so frames never leave GPU memory. That removes the main
objection to video (an FFmpeg decode plus a per-frame upload). It remains the
lowest-priority item in Track 8 and it is not started. Fallback chain if
`vk-video` does not hold on the target hardware: `ffmpeg-next`, then `dav1d` for
AV1.

## Item 3 — the boundary: open, and it must stay open

Item 3 is `examples/vvardenfell` becoming a playable slice — a character that
walks with animation, a HUD, an NPC that paths around an obstacle, positional
audio, a save and a reload, in a streamed world. **Four of those six need tracks
that have not started** (5, 6, 4 and MORROWIND-AF). Closing item 3 now would
mean closing it against a slice that does not contain the things it is supposed
to prove.

**It is deliberately the last thing in the phase, and this record exists partly
to stop it being closed early.**

## Files

```
+ crates/somnium_i18n/           1,954 lines, five files, serde and nothing else
+ crates/somnium_core/src/i18n.rs  183 lines joining the Resolver trait to a Catalog
~ Cargo.toml, Cargo.lock          the workspace member
```
