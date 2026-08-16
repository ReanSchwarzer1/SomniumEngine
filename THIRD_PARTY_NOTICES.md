# Third-party notices

## Inter

Somnium's native editor bundles `Inter-Regular.ttf`, `Inter-Medium.ttf` and
`Inter-SemiBold.ttf` from the Inter typeface family, copyright The Inter
Project Authors. Inter is licensed under the SIL Open Font License 1.1. The
complete license is distributed at `crates/somnium_ui/assets/fonts/Inter-OFL.txt`.

| Field | Value |
|---|---|
| Upstream | <https://github.com/rsms/inter> |
| Version | 4.1 release (`Version 4.001`) |
| Files | `extras/ttf/Inter-{Regular,Medium,SemiBold}.ttf` |
| Modification | Subset with `fontTools.subset` to Latin-1, general punctuation, currency, arrows, math, technical and geometric-shape ranges; hinting dropped. No glyph outline was altered and the family is not renamed, so the Reserved Font Name rules are unaffected. |
| Retrieved | 2026-08-16 |

## JetBrains Mono

The editor bundles `JetBrainsMono-Regular.ttf` and `JetBrainsMono-Medium.ttf`,
copyright The JetBrains Mono Project Authors, licensed under the SIL Open Font
License 1.1. The complete license is distributed at
`crates/somnium_ui/assets/fonts/JetBrainsMono-OFL.txt`.

| Field | Value |
|---|---|
| Upstream | <https://github.com/JetBrains/JetBrainsMono> |
| Version | 2.304 |
| Files | `fonts/ttf/JetBrainsMono-{Regular,Medium}.ttf` |
| Modification | Subset as above; outlines unaltered, family not renamed. |
| Retrieved | 2026-08-16 |

JetBrains Mono carries the `mono` and `mono_strong` roles from the Nocturne
Atelier token sheet. It is used for numeric property values because `fontdue`
applies no OpenType features, so the tabular figures the token sheet requires
(`tnum`) cannot be enabled on the proportional face; a monospaced face gives
the same guarantee by construction.

## Tabler Icons

The editor's utility icon family is Tabler Icons, copyright 2020–2026 Paweł
Kuna, licensed under the MIT License. The complete license is distributed at
`crates/somnium_ui/assets/icons/tabler/LICENSE`.

| Field | Value |
|---|---|
| Upstream | <https://github.com/tabler/tabler-icons> |
| Files | 67 outline SVGs, vendored individually under `crates/somnium_ui/assets/icons/tabler/` |
| Modification | Renamed from the upstream icon name to the Somnium `IconId` it serves (for example `device-floppy.svg` → `save.svg`). Path data is unaltered. |
| Retrieved | 2026-08-16 |

Only the icons the editor actually draws are vendored; the upstream set of
6,000+ is not redistributed. Sources are compiled into the binary with
`include_str!` and rasterized at startup by `resvg` into the monochrome alpha
atlas, so the shader tints each glyph with a semantic colour.

## resvg

`resvg` (with `usvg` and `tiny-skia`) rasterizes the icon SVGs. Licensed under
MIT OR Apache-2.0; <https://github.com/linebender/resvg>. Vendored as a Cargo
dependency with default features disabled — text, system fonts and raster-image
support are off, because every icon source is a stroked path and the text
feature would pull a second font stack in beside `fontdue`.

## Brand and engine-specific icon assets

The Nocturne Atelier brand marks (`assets/brand/`) and the sixteen
engine-specific icons (`assets/icons/somnium/`) are original project assets from
the approved Phase 26-Zeta design package. They are drawn on Tabler's 24 × 24 /
2 px construction grid so the two sets are optically consistent; the grid is an
interoperability constraint, not copied artwork.
