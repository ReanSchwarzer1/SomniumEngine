# Phase 26 — Iris

> *"Iris, messenger of the gods, whose path is the rainbow."*
> Every colour the inspector already stores is currently edited as three
> anonymous floats. This phase gives each of them a face.

> **Codename:** Iris, after the Greek goddess of the rainbow — and the aperture
> of the eye that colour is judged through
> **Status:** PLANNED — IMPLEMENTATION NOT STARTED
> **Plan date:** 2026-08-13
> **Project:** Somnium Engine
> **Target:** Rust 1.85, wgpu 29, winit 0.30
> **Depends on:** Phase 12 (native UI) complete; no GPU feature dependency

The codename is thematic. No third-party source code is copied by this phase;
Unreal Engine's colour-picker UX is the controlling reference for layout and
interaction semantics, studied under the UE EULA and cited in
`ATTRIBUTION.md` as the stages land. Somnium's widget implementation is
original Rust on the existing Fyrox-inspired UI stack.

## 1. Executive decision

The inspector can already edit light colour — as three `Col R` / `Col G` /
`Col B` numeric fields — and cannot edit water colour, absorption, scattering,
particle colours, or material base colour at all. That is not a missing float;
it is a missing *widget*. Colour is a spatial quantity. Authoring it as three
independent scalars is how you get a sun that is accidentally magenta and a
lake whose deep colour nobody has ever been able to retune without opening
source.

Phase 26 adds a first-class colour property editor to `somnium_ui`, modelled on
Unreal's Details-panel pattern:

1. A compact **swatch** (`SColorBlock` equivalent) sits in the property row.
2. Clicking the swatch opens a **popup picker** (`SColorPicker` equivalent).
3. Dragging inside the picker fires **interactive** updates so the viewport
   follows the colour in real time; releasing or confirming **commits**;
   Escape / Cancel restores the colour that was present when the popup opened.
4. The same widget is reused for every authored colour in the engine — lights,
   water, particles, materials, and any later consumer — rather than inventing
   a one-off per section.

This is editor infrastructure, not a rendering phase. It unblocks authoring
that Phase IV, Phase 22, and Phase 24 already shipped data for and left
unreachable.

## 2. Goals

1. Ship one reusable colour property editor (swatch + popup) that any inspector
   section can host.
2. Replace every existing RGB triple of numeric fields with that editor where
   the quantity is actually a colour.
3. Expose every authored colour that is currently invisible in the inspector
   (water deep / shallow / edge, absorption and scattering as tinted
   coefficients, particle start / end, material base colour when a mesh with a
   material is selected).
4. Preserve Unreal's interactive-preview contract: the scene updates while the
   picker is dragged, and Cancel reverts cleanly.
5. Keep linear working colour as the source of truth; the swatch and the picker
   spectrum display in approximate sRGB so what the eye sees matches the
   viewport after tonemap.
6. Leave temperature-based light colour (Kelvin) as a sibling control, not a
   replacement — Unreal does the same for lights.

## 3. Non-goals

- A full colour-grading suite, LUT authoring, or ACES ODTs inside the picker.
- Eyedropper sampling from the 3D viewport (useful later; not required to ship
  the widget).
- HDR colour editing above 1.0 inside the spectrum widget. Lights that carry
  physical intensity keep intensity as a separate float; the picker edits the
  *tint*, clamped to the unit cube, matching Unreal's separation of colour and
  intensity on `ULightComponent`.
- Theme / palette library panels (Unreal's colour themes). A small recent-
  colours strip is enough for v1.
- Replacing Fyrox-inspired layout with Slate. The *interaction model* is
  Unreal's; the *widget tree* stays Somnium's.
- Reworking post-process Temp / Tint / Lift / Gamma / Gain into full colour
  wheels. Those are scalar grading axes and stay numeric; only true RGB(A)
  properties migrate.

## 4. Repository audit (2026-08-13)

| Quantity | Component | Inspector today | Should get Iris |
|---|---|---|---|
| Light tint RGB | `LightComponent.color` | Three floats (`Col R/G/B`) | Yes — primary adoption |
| Light temperature | `LightComponent.color_temperature_k` | Kelvin float | Keep as sibling; picker shows resulting tint read-only when Kelvin > 0 |
| Water deep / shallow / edge | `WaterComponent.*_color` | Absent | Yes |
| Water absorption RGB | `WaterComponent.absorption` | Absent | Yes — labelled as coefficients, not albedo (see 5.4) |
| Water scattering RGB | `WaterComponent.scattering` | Absent | Yes — same |
| Water underwater enable | `WaterComponent.underwater_enabled` | Absent | Toggle in the same water pass (not a colour, but blocked on the same inspector work) |
| Water wave directions | `wave_dir_a/b` | Absent | Compact X/Z pair rows in the same pass |
| Particle start / end | particle colour fields | Absent from entity inspector | Yes when a particle emitter entity is selected |
| Material base colour | `MaterialComponent` / GPU material | Absent | Yes when a mesh-with-material is selected (Phase 26-E) |
| Post Temp / Tint / Lift / Gain | `PostProcessComponent` | Scalars | No — remain numeric |
| Fog colour | not authored as RGB today | N/A | Out of scope until fog gains an authored colour |

The native UI stack already has everything a picker needs as primitives:
`Button` (swatch click), `Popup` (picker host — File menu already uses one),
`NumericField` (channel readouts), `Border` / `Canvas` for the spectrum, and
`NumericFieldMessage::{ValueChanging, ValueChanged}` as the live/commit
pattern to mirror.

## 5. Design (Unreal → Somnium)

### 5.1 Controlling Unreal references

Studied under the UE EULA; pattern-level only, no source translation:

| Unreal piece | Role in Iris |
|---|---|
| `SColorBlock` | Compact rectangular swatch in the Details row; click opens the picker; shows the current linear colour converted for display |
| `SColorPicker` | Popup with HSV spectrum / value slider, RGB+HSV+Hex readouts, optional alpha, OK / Cancel |
| `FColorPickerArgs` | `InitialColor`, `OnColorCommitted`, `OnColorPickerCancelled`, `bUseAlpha`, `bOnlyRefreshOnOk`, interactive begin/end delegates |
| `FLinearColor` vs `FColor` | Linear is storage; display applies approximate sRGB encode so the swatch matches the tonemapped frame |
| Light Details customisation | Colour swatch + separate Intensity + optional Temperature; Iris keeps Kelvin beside the swatch rather than inside it |

### 5.2 Somnium widget contract

```text
ColorProperty row
├── Label ("Deep", "Color", …)
├── ColorSwatch (Button-sized; fills with display-encoded RGB)
└── optional Alpha NumericField when the property is RGBA

ColorPickerPopup (one global instance, like Unreal's static OpenColorPicker)
├── SV square (saturation × value at fixed hue)
├── Hue strip
├── Channel NumericFields: R G B  |  H S V  |  Hex
├── optional A slider
├── Recent colours (8 slots)
└── Cancel  (OK is implicit on click-outside commit, matching Unreal default
             when bOnlyRefreshOnOk = false)
```

Messages, parallel to `NumericFieldMessage`:

| Message | When | Semantics |
|---|---|---|
| `ColorChanging(LinearRgba)` | Pointer moves inside spectrum / channel drag | Live preview; engine writes the component immediately |
| `ColorChanged(LinearRgba)` | Pointer up, Enter, or popup dismiss after edits | Commit; undo stack records a single step from the colour captured at popup open |
| `ColorCancelled` | Escape or Cancel | Restore the colour captured at popup open; one write |

Only one picker popup may be open. Opening a second swatch closes the first
with a commit of its current value (Unreal closes and keeps the last
interactive colour).

### 5.3 Colour space rules

- **Storage:** linear RGB(A) in the component, matching what shaders already
  consume (`LightComponent.color`, water colours, material base colour).
- **Swatch / spectrum display:** encode with the same approximate sRGB the
  capture path uses (`pow(x, 1/2.2)`), so a mid-grey water shallow colour does
  not look darker in the inspector than in the frame.
- **Hex field:** edits sRGB bytes; convert to linear on commit.
- **Absorption / scattering:** stored as linear coefficients in inverse metres.
  The picker still edits them as an RGB triple, but the swatch is drawn
  *normalised* (divide by max channel) so a `(0.22, 0.07, 0.03)` absorption
  reads as a deep-blue tint rather than near-black. A small "× max" numeric
  beside the swatch edits the magnitude without fighting the hue.

### 5.4 Inspector field model

Replace per-channel `InspectorField::LightColorR/G/B` with a single
`InspectorField::Color(ColorTarget)` (or a flat enum of colour slots:
`LightColor`, `WaterDeep`, `WaterShallow`, …). The UI emits one colour event
carrying `[r,g,b,a]`; `app.rs` writes the whole vector. Live vs commit reuses
the existing `live: bool` on `SetInspectorValue`, or a dedicated
`SetInspectorColor { target, value, live }` if packing four floats into the
current scalar event proves awkward.

## 6. Adoption inventory

### 26-A — Widget foundation
`ColorSwatch`, `ColorPickerPopup`, messages, sRGB encode/decode helpers,
recent-colours ring. No consumer yet beyond a hidden smoke harness.

### 26-B — Lights
Directional / point / spot: replace `Col R/G/B` with one Color swatch.
Keep Intensity, Range, angles, Moon, and Kelvin. When Kelvin > 0, the swatch
shows the derived tint and is read-only until Kelvin is cleared (Unreal's
temperature override behaviour).

### 26-C — Water body
Deep, Shallow, Edge colours (RGBA). Absorption and Scattering as normalised
swatches + magnitude. Underwater enable toggle. Wave direction A/B as two
compact X/Z numeric pairs (not colours, but the same inspector gap that
blocked them).

### 26-D — Particles
Start and End colours on the particle / emitter component when selected.

### 26-E — Materials
Base colour on the selected mesh's material. Alpha respects the material's
blend mode (opaque hides A). This stage may share work with Phase 25J's
terrain material UI; Iris owns the widget, 25J owns terrain-specific fields.

## 7. Stages and acceptance

| Stage | Deliverable | Done when |
|---|---|---|
| 26-A | Swatch + picker popup in `somnium_ui` | Unit test: round-trip linear ↔ sRGB ↔ hex; interactive Changing then Cancel restores the original |
| 26-B | Light inspector uses Iris | Selecting Sun / Point / Spot shows one Color swatch; dragging updates the viewport; Cancel reverts; Kelvin still works |
| 26-C | Water inspector colour + leftover scalars | Deep/Shallow/Edge/Abs/Scatter editable; Underwater toggle; wave dirs editable; scene round-trip preserves values |
| 26-D | Particle colours | Start/End swatches on emitter selection |
| 26-E | Material base colour | Selected mesh material base colour editable without opening source |

Workspace `cargo test`, shader validation, and scene serialisation tests must
stay green after every stage. Evidence captures belong under
`dev records/phase 26/` (picker open over the light and water inspectors).

## 8. Risks

| Risk | Mitigation |
|---|---|
| Popup clipping inside the inspector ScrollViewer | Host the picker on the root canvas (same fix already used for the File menu popup; Fyrox pattern) |
| Linear vs sRGB mismatch makes authored colours look wrong | One shared encode helper; golden-unit test against a mid-grey and a saturated blue |
| Undo records every mouse move | Only push undo on `ColorChanged` / open→Cancel boundary; Changing is ephemeral |
| Absorption edited as colour blows the extinction integral | Normalised swatch + magnitude field (5.3); clamp magnitude to a sane range |
| Kelvin and RGB fight each other on lights | Kelvin > 0 locks the swatch; clearing Kelvin restores last authored RGB |

## 9. Attribution boundary

- **Unreal Engine** (`SColorBlock`, `SColorPicker`, `FColorPickerArgs`, light
  Details colour+temperature layout): interaction and information-architecture
  reference under the UE EULA. No Slate source is copied.
- **Fyrox UI** (`Popup`, `Button`, `NumericField`, canvas hosting): already
  attributed for Phase 12; Iris reuses those primitives.
- Somnium's `ColorSwatch` / `ColorPickerPopup` widgets, linear↔sRGB helpers,
  and inspector wiring are original.

A concrete ATTRIBUTION subsection is added when 26-A lands and the first
Unreal header paths are re-verified against the local `example_repo` tree.

## 10. Handoff rule

The next session should read this file, `context.md` (roadmap row 26), and
`crates/somnium_ui/src/widgets/numeric_field.rs` (the live/commit message
pattern to mirror), then begin at 26-A. Do not begin at 26-C because water is
the loudest consumer — the widget has to exist first.

This file is a plan only. No Phase 26 engine code was added as part of its
creation.
