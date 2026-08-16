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

## Brand and icon assets

The Nocturne Atelier brand and engine-specific SVG files are original project
assets. No Tabler SVG is currently vendored; the custom icons only use the
documented 24×24 / 2 px construction grid as an interoperability constraint.
