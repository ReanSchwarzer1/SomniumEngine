# PERSONA-I — Nocturne surface finish

Date: 2026-09-09. Base: `0246ad2`. Implementation and integration checks complete; final native evidence is recorded below. This is the delivered surface pass plus selected H/J/K finishing work, not a claim that every choreography row or the whole expansion is closed.

## Design decisions

Use an ink-and-violet palette with flat dark content grounds. Keep the existing panel colour the user preferred; darken and distinguish the surrounding headers, controls and popups. Replace the automatic white-mixed header wash with the authored header gradient: mixing 4.5% white in linear space was a disproportionately bright grey lift on dark chrome. Raised controls and popup grounds no longer acquire that inferred wash.

Nocturne and Dawn token sheets are **0.5.0-persona**. The exports now include the material dither setting and all four authored gradient pairs in addition to semantic colours, metrics and motion. UI font families, density, target sizes and input geometry remain unchanged. Chrome/popup/tile radii decrease to 4/7/5 px; modal/input radii remain 10/4 px. Text contrast, active violet, muted copy, retained selection, control edges and decorative borders remain separate roles.

- **Emboss:** shipped on raised/primary controls and active tabs; a restrained inner lower-edge highlight follows the existing rounded-box SDF. It uses the idle shadow-colour slot of a fill instance, a new flag and no extra geometry. Inputs, content grounds and tree rows stay unembossed. High contrast disables it.
- **Dither:** deterministic Bayer 8×8 at no more than half a display-space 8-bit step, on gradients only. The offset is applied to authored stops before the existing single sRGB decode. No added decode/encode round trip, texture or time input. High contrast disables it. The original single-decode guard remains intact.
- **Corners:** keep circular corners. Squircle powers would add per-fragment work for very little distinction at these small radii.
- **Blur:** decline new backdrop sampling/passes in this slice. A themed scrim, clear card silhouette and layered popup shadows provide the fallback. Flat content bodies are an explicit user preference. This does not pretend to implement the older 27-D blur debt.
- **Pointer light:** decline. Header/selection hierarchy should remain stable while the pointer moves, and the user's Content hover preference rules out a lingering light effect there.
- **Accent derivation, multi-stop chrome and shadow replacement:** retain the explicit contrast-certified violet ramp, two stops and existing shadow kernel. Perceptual ramp generation, additional gradient stops and an erf kernel are deferred implementation alternatives, not required to ship this visual treatment.

## Delivered editor changes

Component headings now have a distinct raised treatment and measured section spacing. Nested property groups use quieter typography and spacing. The same schema, generated bindings, pin/reset targets and undo routes own the values. Labels stay on their measured centreline.

Command menus use explicit rounded popup surfaces with elevation; Preferences uses a rounded modal surface and the shared scrim. Surface roles change paint without changing border insets, scrolling or focus. The Preferences heading remains on the card ground so rectangular header paint cannot cover the rounded corners.

Centered dialogs now share Popup's immediate logical visibility and paint-only entry/exit lifecycle: 180 ms, scale 0.98 to 1; close remains 100 ms. Input is live from the first frame and closed paint cannot catch clicks. Tab underlines move over 160 ms while page selection changes immediately. Programmatic tab selection now works and clamps invalid indices.

A changed generated property row briefly shows a 240 ms accent wash after external model updates such as undo, reset or assignment. It is suppressed during an active gesture or text edit; values themselves never animate. The first row build is already settled. Reduced motion holds the cue, then cuts it; expiry removes its transient state.

Toasts use measured text and wrapping, fit inside the viewport, stack by their actual heights, and fade their text with their surface. Entry is 180 ms; reduced motion uses immediate/static visibility. Sticky errors remain sticky. Long Unicode paths and identifiers wrap without losing characters.

## Evidence and limits

The automated UI suite covers both theme/density/contrast combinations, token parity, real popup input and close behavior, centered-dialog and reduced-motion settling, tab interruption, row-flash timing, long path wrapping, idle motion, source shader validation and the unchanged primitive layout. Emboss adds no extra control instance; flat grounds and inputs remain undecorated. The core suite covers the editor integration.

No new dependency, parallel agent, alternate renderer, golden replacement, commit or push. The user allows at most three screenshots near completion; evidence uses the existing engine capture route. Full H/J choreography, the broader K backlog, matched GPU p95 certification and the OS/DPI/floating-window acceptance matrix remain open. Visual acceptance belongs to the user; no zero-bug or full-phase closure claim is made.


## Final verification

- `cargo test -p somnium_ui -p somnium_core --lib -j1`: **1,109 passed** (787 UI, 322 core), zero failures. This includes the final centered-dialog regression.
- `cargo test -p somnium_ui --test shaders_validate -j1`: **6 passed**, including both UI shaders with sRGB output enabled/disabled. The colour-transfer and primitive-layout guards remain intact.
- `cargo build -p hello_engine --release -j1`: passed; `target/release/hello_engine.exe` contains this visual pass.
- Three native captures, all successful: [1280 baseline](../../target/persona-finish/before-1280.png), [1280 updated shell](../../target/persona-finish/after-1280.png), [1920 Preferences](../../target/persona-finish/preferences-1920.png). Exactly three screenshots were taken. The matched shell pair uses the Terrain workspace; it has no selected scene entity, so it does not certify selected-object Details. Preferences exercises the shared component/group treatment and dialog frame. These generated images are local evidence, not checked-in goldens.
- Visual inspection: dark chrome and flat panel bodies are retained; the Preferences header, input row and grouped settings are legible, and the modal separates from the viewport. The screenshot review does not substitute for designer approval, floating-window journey checks or motion capture.
- The existing native-capture warning about an empty game-UI canvas appeared in baseline and both updated runs. No new capture error or crash occurred.
- `git diff --check`: passed. Census: **220,744 Rust/WGSL lines; 2,234 discovered tests**. The earlier Windows linker file locks were resolved by renaming only the two generated test executables inside `target/debug/deps`; no tests were skipped.


## Chrome visibility follow-up

The user reviewed the surface pass as looking "far better" and asked for the chrome/gradient to be more apparent in frequently seen areas. The shared Nocturne header wash is now `#2A2F40` to `#1E2230`, reaching the main toolbar, Outliner/Details and other header bands. The authored chrome wash is `#292E3E` to `#1E2230`. Their endpoint contrast ratios are 1.190 and 1.172; the Nocturne wash guard now permits 1.20 for this explicit design adjustment. Dawn keeps its existing 1.12 guard and palette.

Raised controls and active tabs use a 12.5% inner edge highlight (previously 7.8%). Panel bodies, material/property grounds, input wells and tree rows retain the flat treatment. No additional surface geometry, animation or layout change is introduced.

The 25 targeted UI theme tests passed, including token export parity, contrast, high-contrast material suppression and flat-ground contracts. The release executable was rebuilt for this adjustment. No additional screenshot was taken: the three images above show the preceding, subtler treatment. This positive surface feedback is not acceptance of the still-open expansion journeys.
