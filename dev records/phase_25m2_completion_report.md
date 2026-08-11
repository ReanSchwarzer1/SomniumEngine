# Somnium Engine — Phase 25M-2 Comprehensive Handover, Audit Guide & Task Brief

> **Target Audience:** Incoming AI Assistant / LLM / Lead Graphics Engineer  
> **Repository Workspace:** `C:\Users\adhir\OneDrive\Documents\GitHub\SomniumEngine`  
> **Reference Repositories:** `C:\Users\adhir\Downloads\GE\example_repo` and `C:\Users\adhir\Downloads\GE\example_repo\New_Engines`  
> **Primary Context File to Mimic:** `C:\Users\adhir\Downloads\READ THIS FIRST.md`  
> **Date:** August 11, 2026  

---

> [!IMPORTANT]
> ## 0. Context Gathering & Mandatory First Step (MUST READ)
> 
> Before writing any code, modifying shaders, or diagnosing bugs, **YOU MUST EXPLICITLY BUILD YOUR CONTEXT BY READING THE FOLLOWING FILES IN FULL**:
> 
> 1. **Primary Context Brief:** Read `C:\Users\adhir\Downloads\READ THIS FIRST.md` to understand the engine architecture, toolchain (Rust 1.85, wgpu 29), strict approach rules, and living documentation requirements.
> 2. **Living Documentation:** Read `context.md`, `ATTRIBUTION.md`, `m2.md`, and `m25.md` in the workspace root to understand the history of rendering phases up to Phase 25.
> 3. **Core Shader & Pass Code:** Read the authoritative source files:
>    - [`crates/somnium_renderer/src/shaders/shading.wgsl`](file:///C:/Users/adhir/OneDrive/Documents/GitHub/SomniumEngine/crates/somnium_renderer/src/shaders/shading.wgsl) (Visibility buffer deferred PBR shading, directional light, moonlight, foliage transmission, and IBL)
>    - [`crates/somnium_renderer/src/shaders/atmosphere.wgsl`](file:///C:/Users/adhir/OneDrive/Documents/GitHub/SomniumEngine/crates/somnium_renderer/src/shaders/atmosphere.wgsl) (Hillaire sky raymarching, 3x3x3 starfield, procedural moon disc)
>    - [`crates/somnium_renderer/src/shaders/ibl_gen.wgsl`](file:///C:/Users/adhir/OneDrive/Documents/GitHub/SomniumEngine/crates/somnium_renderer/src/shaders/ibl_gen.wgsl) (Environment map capture & prefiltering)
>    - [`crates/somnium_core/src/sun.rs`](file:///C:/Users/adhir/OneDrive/Documents/GitHub/SomniumEngine/crates/somnium_core/src/sun.rs) (CPU transmittance integration, horizon elevation clamping, and lunar orbit position calculation)
> 4. **Reference Repositories:** Inspect reference code in `C:\Users\adhir\Downloads\GE\example_repo\UnrealEngine-release`. **Never guess how Unreal Engine 5 or reference engines implement features — inspect the C++ and USF source files first.**

---

> [!CAUTION]
> ## 1. Mandatory Code Audit Warning (Previous Code Edits May Be Flawed!)
> 
> **Do not assume the existing code edits written in Phase 25M-2 are 100% correct or optimal.**  
> The previous AI assistant made multiple modifications across `shading.wgsl`, `atmosphere.wgsl`, `ibl_gen.wgsl`, and `sun.rs`. While the code compiles cleanly and passes basic unit tests, **some of these proposed changes may be incomplete, mathematically flawed, or partially incorrect**.
> 
> **YOUR MANDATORY FIRST TASK:** Before attempting to fix the remaining open problems, **audit and review all recent code edits in the workspace**. Verify:
> - Are normal vectors, geometric normals (`geo_normal`), and view vectors formatted correctly in WGSL?
> - Does moonlight evaluation conserve PBR energy and respect material properties?
> - Are contact shadow guards (`if light_dir.y <= -0.02`) creating unintended edge cases during twilight transitions?
> - Is `specular_occlusion` (Lagarde & de Rousiers) or `evaluate_ibl` miscalculating foliage ambient reflections?

---

## 2. Phase 25M-2 Task Status & Overview

Phase 25M-2 covers night sky photorealism, directional moonlight, procedural lunar disc rendering, and terrain/foliage shading refinements. Below is the detailed breakdown of all sub-tasks and their current status:

| Sub-Task | Feature | Status | Summary |
| :--- | :--- | :--- | :--- |
| **25M-2A** | Dusk Over-Orange & Transmittance Clamping | **Partially Verified** | Ported UE5 `MIN_ELEVATION_COS = -0.026` (-1.5°) into `sun.rs` to stop hyper-extinction at horizon. |
| **25M-2B** | Low-Sun Terrain Shadows & Contact Shadow Noise | **Partially Verified** | Added horizon guard in `contact_shadow()` to stop downward depth buffer raymarching at night. |
| **25M-2C** | Starfield 3x3x3 Neighborhood & Exponential Magnitude | **Implemented** | Replaced 1-cell lookup with 27-cell neighborhood evaluation and exponential magnitude ($0.005 e^{4\text{hash}}$). |
| **25M-2D** | Procedural Moon Disc, Phase & Earthshine | **Implemented** | Enlarged moon radius (`MOON_COS_RADIUS = 0.9996`), added 3D FBM lunar maria/craters, smooth phase terminator, and earthshine. |
| **25M-2E** | **Foliage Night Shading & Shifting Green Glow** | **[UNFIXED / OPEN]** | **CRITICAL ISSUE:** Grass/lower foliage remains pitch-black at night under moonlight, while upper tree leaves exhibit an shifting yellow-green glow. |

---

## 3. Deep Technical Breakdown of the Unfixed Foliage Problem (Task 25M-2E)

### Visual Symptoms (From Latest Engine Capture):
- **Symptom 1 (Pitch-Black Grass):** Ground grass tufts and lower foliage receive zero moonlight illumination even when the moon is high in the sky and moonlight intensity is increased. The ground terrain is illuminated, but grass blades remain pitch black silhouette cutouts.
- **Symptom 2 (Shifting Yellow-Green Glow):** Upper tree canopy leaves receive a bright, shifting yellowish-green glow at night. The glow changes intensity and location as the camera moves.

### Suspected Root Causes & Directives for Investigation:

#### A. Foliage Curved Normals & Geometric Normal Branching ([`shading.wgsl:L764-L790`](file:///C:/Users/adhir/OneDrive/Documents/GitHub/SomniumEngine/crates/somnium_renderer/src/shaders/shading.wgsl#L764-L790))
- **Code Reference:**
  ```wgsl
  if (material.flags & 2u) != 0u {
      let curve = clamp((uv.x - 0.5) * 2.0943951, -1.5707963, 1.5707963);
      let axis = normalize(cross(surface.normal, tangent));
      let c = cos(curve);
      let s = sin(curve);
      surface.normal = normalize(
          surface.normal * c + cross(axis, surface.normal) * s + axis * dot(axis, surface.normal) * (1.0 - c)
      );
  }
  ```
- **Diagnostic Directive:** Investigate whether rotating `surface.normal` by $\pm 60^\circ$ across foliage cards forces the dot product $\vec{n} \cdot \vec{d}_\text{moon}$ to negative values for ground grass, causing $N\cdot L$ evaluation to return 0. Verify if double-sided lighting (`abs(dot(n, moon_dir))`) or wrapped diffuse ($N\cdot L_\text{wrap} = \frac{N\cdot L + w}{1 + w}$) is needed for thin foliage quad cards.

#### B. ReSTIR GI & Temporal Reservoir Bleed
- **Code Reference:** [`shading.wgsl:L1028-L1032`](file:///C:/Users/adhir/OneDrive/Documents/GitHub/SomniumEngine/crates/somnium_renderer/src/shaders/shading.wgsl#L1028-L1032)
  ```wgsl
  let gi_texel = textureLoad(restir_gi, vec2<i32>(in.clip_pos.xy), 0);
  let ambient = evaluate_ibl(surface, gi_texel);
  ```
- **Diagnostic Directive:** ReSTIR GI (`restir_gi.wgsl`) temporally accumulates and spatially reuses indirect GI reservoirs across frames. During daytime, foliage bounces intense green light into nearby pixels. When the time of day transitions to night, temporal reservoirs may retain high-weight green irradiance samples, causing a "shifting green halo" on upper foliage silhouettes as camera motion reuses old spatial reservoirs. Verify how `restir_gi` handles light intensity changes when the sun sets.

#### C. Transmitted Light & Sunlight Residuals
- **Code Reference:** [`shading.wgsl:L75-L96`](file:///C:/Users/adhir/OneDrive/Documents/GitHub/SomniumEngine/crates/somnium_renderer/src/shaders/shading.wgsl#L75-L96)
  ```wgsl
  const AMBIENT: f32 = 0.15;
  return light_color * (lobe + AMBIENT) * transmission * surface.albedo;
  ```
- **Diagnostic Directive:** `transmitted_light` includes an `AMBIENT = 0.15` term multiplied by `surface.albedo` (which is olive green `(0.2, 0.4, 0.1)`). If `light_color` (sun light) contains non-zero ambient values at dusk/night, `(lobe + AMBIENT) * surface.albedo` will produce a bright green glow on transmissive foliage cards!

---

## 4. Exact UE5 & Reference Source Code Locations (`example_repo`)

When researching patterns to resolve these issues, refer to the following authoritative source code files inside `C:\Users\adhir\Downloads\GE\example_repo\UnrealEngine-release`:

1. **UE5 SkyAtmosphere & Directional Light Coupling:**
   - `Engine/Source/Runtime/Engine/Private/Atmosphere/SkyAtmosphereComponent.cpp`
   - `Engine/Shaders/Private/SkyAtmosphereCommon.ush`
   - `Engine/Shaders/Private/SkyAtmosphere.usf`
2. **UE5 Foliage & Two-Sided Subsurface Shading:**
   - `Engine/Shaders/Private/SubsurfaceHeader.ush` (Subsurface profile & two-sided foliage transmission)
   - `Engine/Shaders/Private/ShadingModels.ush` (Two-Sided Foliage BRDF formulation)
   - `Engine/Shaders/Private/DeferredLightingCommon.ush` (GetDynamicLighting evaluation for directional light 0 & 1)
3. **UE5 Shadow Cascades & Slope Depth Bias:**
   - `Engine/Source/Runtime/Renderer/Private/ShadowRendering.cpp`
   - `Engine/Shaders/Private/ShadowFilteringCommon.ush`

---

## 5. Summary of Files & Expected Action Plan for Incoming LLM

### Step 1: Audit Recent Edits
Review the code changes made in:
- [`crates/somnium_renderer/src/shaders/shading.wgsl`](file:///C:/Users/adhir/OneDrive/Documents/GitHub/SomniumEngine/crates/somnium_renderer/src/shaders/shading.wgsl)
- [`crates/somnium_renderer/src/shaders/atmosphere.wgsl`](file:///C:/Users/adhir/OneDrive/Documents/GitHub/SomniumEngine/crates/somnium_renderer/src/shaders/atmosphere.wgsl)
- [`crates/somnium_core/src/sun.rs`](file:///C:/Users/adhir/OneDrive/Documents/GitHub/SomniumEngine/crates/somnium_core/src/sun.rs)

### Step 2: Fix Foliage Night Shading & Glow
1. Inspect foliage material flags `(material.flags & 2u) != 0u` in `shading.wgsl`.
2. Implement UE5-style Two-Sided Foliage transmission and double-sided moonlight NdotL for grass and trees.
3. Clamp or clear ReSTIR GI reservoirs when sun illuminance drops to zero to stop daytime green GI bleed.

### Step 3: Validation
Run validation commands to ensure no regressions:
```bash
cargo check --bin hello_engine
cargo test
```
