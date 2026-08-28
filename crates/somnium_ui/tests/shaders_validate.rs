//! Parse and validate the UI shaders (Phase MORROWIND, MORROWIND-C precedent).
//!
//! `somnium_renderer` has had this since Phase 25 and `somnium_ui` has not, so
//! its two shaders were only ever compiled at device-creation time — on a
//! machine with a GPU, at startup, as a first-frame crash.
//!
//! **That gap was not theoretical.** MORROWIND-C found that the renderer's
//! `naga` dev-dependency had been left on 29 while wgpu moved to 30, and that
//! this hid a `binding_array` incompatibility which would have failed on the
//! first frame. `ui_pass.wgsl` declares a binding array as of MORROWIND-D and
//! would have been in exactly the same position. One naga, checked here.

use naga::valid::{Capabilities, ValidationFlags, Validator};

const UI_PASS: &str = include_str!("../src/ui_pass.wgsl");
const UI_SHAPED: &str = include_str!("../src/ui_shaped.wgsl");

/// The substitution `UiPass` performs for a non-sRGB surface.
const SRGB_DECLARATION: &str = "const OUTPUT_IS_SRGB: bool = true;";
const NON_SRGB_DECLARATION: &str = "const OUTPUT_IS_SRGB: bool = false;";

fn check(label: &str, source: &str) {
    let module = match naga::front::wgsl::parse_str(source) {
        Ok(m) => m,
        Err(e) => panic!("{label} failed to parse:\n{}", e.emit_to_string(source)),
    };
    // Binding arrays and non-uniform indexing, both of which the UI now uses
    // and both of which `somnium_renderer::context` already requires of the
    // device this pass runs on.
    let mut validator = Validator::new(ValidationFlags::all(), Capabilities::all());
    if let Err(e) = validator.validate(&module) {
        panic!("{label} failed validation:\n{}", e.emit_to_string(source));
    }
}

#[test]
fn the_quad_shader_validates() {
    check("ui_pass", UI_PASS);
}

#[test]
fn the_shaped_shader_validates() {
    check("ui_shaped", UI_SHAPED);
}

/// Both shaders validate in the form a non-sRGB surface gets.
///
/// `UiPass` rewrites one line of source at pipeline-creation time. A shader
/// that only validates in its checked-in form is validated in the branch
/// nobody with an unusual surface format runs.
#[test]
fn both_shaders_validate_with_srgb_output_off() {
    for (label, source) in [("ui_pass", UI_PASS), ("ui_shaped", UI_SHAPED)] {
        assert!(
            source.contains(SRGB_DECLARATION),
            "{label}: UiPass substitutes this exact line; it is not there to substitute"
        );
        check(
            &format!("{label} (non-sRGB)"),
            &source.replace(SRGB_DECLARATION, NON_SRGB_DECLARATION),
        );
    }
}

/// The two pipelines agree on the bindless array's declaration.
///
/// They share one bind-group layout. A length or binding index that differs
/// between them is a validation error at pipeline creation on a GPU and
/// nothing at all in CI — which is the class of bug this whole file exists for.
#[test]
fn both_shaders_declare_the_same_texture_array() {
    let declaration = format!(
        "@group(1) @binding(4) var ui_textures: binding_array<texture_2d<f32>, {}>;",
        somnium_ui::shaped::MAX_TEXTURE_SLOTS
    );
    assert!(
        UI_PASS.contains(&declaration),
        "ui_pass.wgsl does not declare the array as `{declaration}`"
    );
    assert!(
        UI_SHAPED.contains(&declaration),
        "ui_shaped.wgsl does not declare the array as `{declaration}`"
    );
}

/// `enable wgpu_binding_array;` precedes every declaration in both shaders.
///
/// WGSL requires it, and wgpu 30 requires the directive itself for
/// `binding_array` where 29 did not. Both facts were learned the expensive way
/// in MORROWIND-C.
#[test]
fn the_binding_array_enable_comes_first() {
    for (label, source) in [("ui_pass", UI_PASS), ("ui_shaped", UI_SHAPED)] {
        let first = source
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with("//"))
            .unwrap_or_default();
        assert_eq!(
            first, "enable wgpu_binding_array;",
            "{label}: expected the enable first, found `{first}`"
        );
    }
}

/// The WGSL mirror of `ShapedInstance` agrees with the Rust struct.
///
/// naga computes the same layout the GPU will, so a member whose alignment
/// differs between the two languages fails here instead of decoding every
/// instance after the first from the wrong offset. The renderer's terrain
/// material hit exactly this with a trailing `vec3<u32>` pad, which aligns to
/// 16 in WGSL and to 4 in Rust; a 2x3 affine is the same trap, which is why the
/// shader spells it as six scalars rather than a `mat3x2`.
#[test]
fn the_shaped_instance_struct_matches_the_rust_layout() {
    let module = naga::front::wgsl::parse_str(UI_SHAPED).expect("ui_shaped parses");
    let (_, ty) = module
        .types
        .iter()
        .find(|(_, t)| t.name.as_deref() == Some("ShapedInstance"))
        .expect("ShapedInstance is declared");
    let naga::TypeInner::Struct { members, span } = &ty.inner else {
        panic!("ShapedInstance is not a struct");
    };

    assert_eq!(
        *span as usize,
        std::mem::size_of::<somnium_ui::shaped::ShapedInstance>(),
        "WGSL size disagrees with the Rust struct"
    );

    let offset = |name: &str| {
        members
            .iter()
            .find(|m| m.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("no member {name}"))
            .offset
    };
    // The six affine scalars are contiguous from zero, which is what lets the
    // Rust side keep them as one `[f32; 6]`.
    assert_eq!(offset("xform_a"), 0);
    assert_eq!(offset("xform_b"), 4);
    assert_eq!(offset("xform_c"), 8);
    assert_eq!(offset("xform_d"), 12);
    assert_eq!(offset("xform_tx"), 16);
    assert_eq!(offset("xform_ty"), 20);
    // The gradient is four scalars rather than a `vec4<f32>` precisely so it
    // lands at 24 and not at 32. The vec4 form failed this test on its first
    // run: 80 bytes in WGSL against 64 in Rust.
    assert_eq!(offset("grad_x"), 24);
    assert_eq!(offset("grad_y"), 28);
    assert_eq!(offset("grad_z"), 32);
    assert_eq!(offset("grad_w"), 36);
    assert_eq!(offset("fill_a"), 40);
    assert_eq!(offset("fill_b"), 44);
    assert_eq!(offset("texture"), 48);
    assert_eq!(offset("mask"), 52);
    assert_eq!(offset("flags"), 56);
}
