//! Tests for the shader system's outer layer.
//!
//! `compose`, `cache` and `watch` test themselves. What is left here is the
//! part where they meet, and in particular **the reload contract**, which
//! Appendix A.7 names as the specific way this sub-phase gets faked:
//!
//! > *0 — shaders. The cheat: composition works, hot reload silently falls back
//! > on error. The check: introduce a deliberate WGSL syntax error; a toast must
//! > show naga's diagnostic and the **old pipeline must stay bound** — not a
//! > black screen, not a silent revert with no message.*
//!
//! Naga is not linked here — the crate depends on wgpu, and validation happens
//! at the device. The tests supply a validator closure instead, which is why
//! [`ShaderSystem::apply_reload`] takes one: a contract about failure handling
//! should be testable without a GPU, and making the validator an argument is
//! what buys that.

use super::*;

const SKINNED: u32 = 0;

fn system() -> ShaderSystem {
    let mut s = ShaderSystem::new();
    s.register_define(SKINNED, "SKINNED");
    s
}

/// A validator that accepts everything, for tests not about failure.
fn accept(_module: &str, _source: &str) -> Result<(), String> {
    Ok(())
}

#[test]
fn a_variant_is_resolved_once_and_then_hit() {
    let mut s = system();
    s.register("brdf.wgsl", "fn brdf() {}\n");
    let root = s.register("shading.wgsl", "//!include \"brdf.wgsl\"\nfn shade() {}\n");

    let key = ShaderKey::new(root);
    assert_eq!(s.source(key).unwrap(), "fn brdf() {}\nfn shade() {}\n");
    assert_eq!(s.variants().len(), 1);
    s.source(key).unwrap();
    assert_eq!(
        s.variants().record(key).unwrap().hits,
        2,
        "both resolutions count as uses; only the first did any work"
    );
}

#[test]
fn defines_produce_separate_variants() {
    let mut s = system();
    s.register("skin.wgsl", "fn skin() {}\n");
    let root = s.register(
        "shading.wgsl",
        "//!if SKINNED\n//!include \"skin.wgsl\"\n//!endif\nfn shade() {}\n",
    );

    let plain = ShaderKey::new(root);
    let skinned = ShaderKey::new(root).with(Defines::bit(SKINNED));
    assert!(!s.source(plain).unwrap().contains("fn skin"));
    assert!(s.source(skinned).unwrap().contains("fn skin"));
    assert_eq!(s.variants().len(), 2);
}

#[test]
fn an_unregistered_module_is_named_rather_than_panicking() {
    let mut s = system();
    let error = s.source(ShaderKey::new(ModuleId(7))).unwrap_err();
    assert!(matches!(error, ShaderError::UnknownModule(ModuleId(7))));
}

// ---------------------------------------------------------------------------
// The reload contract — Appendix A.7's named check
// ---------------------------------------------------------------------------

#[test]
fn a_good_reload_swaps_the_source_and_lists_what_changed() {
    let mut s = system();
    let brdf = s.register("brdf.wgsl", "fn brdf() -> f32 { return 1.0; }\n");
    let root = s.register("shading.wgsl", "//!include \"brdf.wgsl\"\nfn shade() {}\n");
    let key = ShaderKey::new(root);
    s.source(key).unwrap();

    let outcome = s.apply_reload(
        vec![(brdf, "fn brdf() -> f32 { return 2.0; }\n".to_string())],
        accept,
    );

    assert_eq!(outcome.reloaded, vec!["brdf.wgsl"]);
    assert_eq!(outcome.invalidated, vec![key]);
    assert!(outcome.failures.is_empty());
    assert!(s.source(key).unwrap().contains("return 2.0"));
    assert!(outcome.summary().starts_with("Reloaded 1 shader module(s)"));
}

/// **The check A.7 names.** A broken edit shows the diagnostic and changes
/// nothing.
///
/// The three ways this is usually faked, each of which this asserts against:
/// the diagnostic is swallowed and replaced with "compilation failed"; the
/// broken source is installed anyway and the next frame is black; or the old
/// source is silently kept with no message, so the author sits there editing a
/// file that is doing nothing.
#[test]
fn a_broken_reload_reports_naga_and_keeps_the_old_source() {
    let mut s = system();
    let brdf = s.register("brdf.wgsl", "fn brdf() -> f32 { return 1.0; }\n");
    let root = s.register("shading.wgsl", "//!include \"brdf.wgsl\"\nfn shade() {}\n");
    let key = ShaderKey::new(root);
    let before = s.source(key).unwrap().to_string();

    let outcome = s.apply_reload(
        vec![(brdf, "fn brdf( -> f32 { retunr 1.0 }\n".to_string())],
        |_module, source| {
            if source.contains("retunr") {
                // Standing in for naga, with the shape of message that matters:
                // a location and a reason.
                Err("expected ')', found '-' at line 1:11".to_string())
            } else {
                Ok(())
            }
        },
    );

    assert!(outcome.reloaded.is_empty(), "nothing was installed");
    assert!(outcome.invalidated.is_empty(), "nothing was invalidated");
    assert_eq!(outcome.failures.len(), 1);

    // 1. The diagnostic survives to the surface, verbatim.
    let message = outcome.summary();
    assert!(
        message.contains("expected ')'") && message.contains("1:11"),
        "the diagnostic is the only useful thing in a shader failure: {message}"
    );
    assert!(message.starts_with("Shader reload failed"));

    // 2. The old source is still what a pipeline would be built from.
    assert_eq!(
        s.source(key).unwrap(),
        before,
        "a typo mid-edit must cost a toast, never the frame"
    );
    assert!(!s.source(key).unwrap().contains("retunr"));

    // 3. The cache still holds the variant, so nothing downstream is told to
    //    swap and the previously built pipeline stays bound.
    assert!(s.variants().record(key).is_some());
}

/// A reload that breaks *composition* rather than syntax behaves identically.
///
/// Adding `//!include "typo.wgsl"` is at least as common as a syntax error, and
/// it fails at a different layer. Both must reach the same non-destructive
/// path, or one of them is a black screen.
#[test]
fn a_reload_that_breaks_composition_is_also_non_destructive() {
    let mut s = system();
    let brdf = s.register("brdf.wgsl", "fn brdf() {}\n");
    let root = s.register("shading.wgsl", "//!include \"brdf.wgsl\"\nfn shade() {}\n");
    let key = ShaderKey::new(root);
    let before = s.source(key).unwrap().to_string();

    let outcome = s.apply_reload(
        vec![(brdf, "//!include \"nope.wgsl\"\nfn brdf() {}\n".to_string())],
        accept,
    );

    assert_eq!(outcome.failures.len(), 1);
    assert!(matches!(
        outcome.failures[0],
        ShaderError::Compose(ComposeError::UnknownModule { .. })
    ));
    assert_eq!(s.source(key).unwrap(), before);
    assert!(outcome.summary().contains("nope.wgsl"));
}

#[test]
fn a_reload_only_touches_variants_that_used_the_changed_module() {
    let mut s = system();
    let brdf = s.register("brdf.wgsl", "fn brdf() {}\n");
    s.register("water_only.wgsl", "fn wat() {}\n");
    let shading = s.register("shading.wgsl", "//!include \"brdf.wgsl\"\nfn shade() {}\n");
    let water = s.register("water.wgsl", "//!include \"water_only.wgsl\"\nfn water() {}\n");
    s.source(ShaderKey::new(shading)).unwrap();
    s.source(ShaderKey::new(water)).unwrap();

    let outcome = s.apply_reload(vec![(brdf, "fn brdf() { let x = 1; }\n".to_string())], accept);
    assert_eq!(
        outcome.invalidated,
        vec![ShaderKey::new(shading)],
        "clearing the cache would work and would rebuild every pipeline in the \
         engine, turning \"edit a file and see it\" into \"edit a file and wait\""
    );
}

#[test]
fn an_empty_reload_is_empty() {
    let mut s = system();
    let outcome = s.apply_reload(Vec::new(), accept);
    assert!(outcome.is_empty());
}

// ---------------------------------------------------------------------------
// The budget
// ---------------------------------------------------------------------------

#[test]
fn the_budget_names_modules_and_counts_their_space() {
    let mut s = system();
    s.register_define(1, "ALPHA_CUTOUT");
    let root = s.register(
        "shading.wgsl",
        "//!if SKINNED\nlet a = 1;\n//!endif\nfn shade() {}\n",
    );
    s.source(ShaderKey::new(root)).unwrap();
    s.source(ShaderKey::new(root).with(Defines::bit(SKINNED))).unwrap();

    let rows = s.budget();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].module, "shading.wgsl");
    assert_eq!(rows[0].defines_used, 1);
    assert_eq!(rows[0].possible, 2);
    assert_eq!(rows[0].compiled, 2);
    assert_eq!(
        rows[0].unused, 0,
        "both were looked up, so neither is a startup stall nobody asked for"
    );
    assert!(s.budget_table().contains("shading.wgsl"));
}
