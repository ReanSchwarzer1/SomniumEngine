//! MORROWIND-U tests for the skinning pass's CPU half.
//!
//! The GPU half needs a device; this is the half that does not, and for a
//! system whose failure mode is an out-of-bounds palette read it is the half
//! worth testing hardest.

use super::*;
use somnium_anim::{NO_PARENT, SkeletonId, SkinBinding, Transform};

fn skeleton(joints: usize) -> Skeleton {
    let parents: Vec<u16> = (0..joints)
        .map(|i| if i == 0 { NO_PARENT } else { (i - 1) as u16 })
        .collect();
    Skeleton::new(
        SkeletonId(0),
        (0..joints).map(|i| format!("j{i}")).collect(),
        parents,
        vec![Mat4::IDENTITY; joints],
        vec![Transform::IDENTITY; joints],
    )
    .expect("a valid chain")
    .0
}

fn skin(vertices: usize, joint: u16) -> Skin {
    Skin {
        skeleton: SkeletonId(0),
        bindings: vec![SkinBinding::from_influences(&[(joint, 1.0)]); vertices],
    }
}

const UNIT_BOX: ([f32; 3], [f32; 3]) = ([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]);

// ── f16 packing, which has to agree with WGSL bit for bit ───────────────────

#[test]
fn weights_survive_a_round_trip_through_f16() {
    for weights in [
        [1.0, 0.0, 0.0, 0.0],
        [0.25, 0.25, 0.25, 0.25],
        [0.5, 0.3, 0.15, 0.05],
        [0.7, 0.2, 0.07, 0.03],
    ] {
        let binding = SkinBinding {
            joints: [0, 1, 2, 3],
            weights,
        };
        let round_tripped = SkinVertex::pack(binding).unpack();
        assert_eq!(round_tripped.joints, binding.joints);
        for (index, (got, want)) in round_tripped.weights.iter().zip(weights.iter()).enumerate() {
            // f16 has an 11-bit mantissa: ~5e-4 relative on values near 1.
            assert!((got - want).abs() < 1e-3, "weight {index}: {got} != {want}");
        }
        assert!(round_tripped.is_normalised(), "{:?}", round_tripped.weights);
    }
}

#[test]
fn f16_endpoints_are_exact() {
    // 0, 0.5 and 1 are exactly representable, and a blend that lands on one of
    // them should not drift.
    for value in [0.0f32, 0.5, 1.0, 0.25, 0.75] {
        let (round_tripped, _) = unpack_f16x2(pack_f16x2(value, 0.0));
        assert_eq!(round_tripped, value, "{value} did not round-trip exactly");
    }
}

#[test]
fn a_nan_weight_stays_a_nan_rather_than_becoming_a_large_number() {
    // "Unreachable in practice" is how a NaN gets into a vertex buffer. If a
    // NaN silently became infinity the vertex would fly off rather than
    // disappearing, and the two are debugged very differently.
    let (got, _) = unpack_f16x2(pack_f16x2(f32::NAN, 0.0));
    assert!(got.is_nan(), "a NaN weight became {got}");
}

#[test]
fn joint_indices_pack_into_the_high_and_low_halves_as_the_shader_reads_them() {
    let packed = SkinVertex::pack(SkinBinding {
        joints: [1, 2, 3, 4],
        weights: [0.25; 4],
    });
    // The shader does `lo & 0xffff` then `lo >> 16`, so joint 0 is the low half.
    assert_eq!(packed.joints_01 & 0xffff, 1);
    assert_eq!(packed.joints_01 >> 16, 2);
    assert_eq!(packed.joints_23 & 0xffff, 3);
    assert_eq!(packed.joints_23 >> 16, 4);
}

#[test]
fn the_highest_joint_index_survives_packing() {
    // 255 is the palette limit, but the packing must not be the thing that
    // caps it — a future MAX_JOINTS_PER_SKELETON change should not silently
    // wrap.
    let packed = SkinVertex::pack(SkinBinding {
        joints: [65_534, 0, 0, 0],
        weights: [1.0, 0.0, 0.0, 0.0],
    });
    assert_eq!(packed.unpack().joints[0], 65_534);
}

// ── registration, which is where the out-of-bounds read gets refused ────────

#[test]
fn a_well_formed_registration_is_accepted() {
    let mut palettes = SkinningPalettes::new(SkinBudget::default());
    let skeleton = skeleton(4);
    let handle = palettes
        .register(&skeleton, &skin(100, 3), 0, 5_000, 100, UNIT_BOX)
        .expect("should register");
    assert_eq!(handle, SkinnedHandle(0));
    assert_eq!(palettes.instances().len(), 1);
    assert_eq!(palettes.palette().len(), 4, "palette space not reserved");
    assert_eq!(palettes.posed_bytes(), 100 * 32);
}

/// The failure this whole registration path exists to prevent: on the GPU it is
/// a read past the palette, which is a garbage matrix on most drivers and a
/// hang on some.
#[test]
fn a_skin_naming_a_joint_past_the_skeleton_is_refused() {
    let mut palettes = SkinningPalettes::new(SkinBudget::default());
    let skeleton = skeleton(4);
    assert_eq!(
        palettes.register(&skeleton, &skin(10, 9), 0, 0, 10, UNIT_BOX),
        Err(SkinError::SkinDoesNotFitSkeleton)
    );
    assert!(
        palettes.is_empty(),
        "a refused registration left state behind"
    );
}

#[test]
fn a_binding_count_that_disagrees_with_the_vertex_count_is_refused() {
    let mut palettes = SkinningPalettes::new(SkinBudget::default());
    assert_eq!(
        palettes.register(&skeleton(2), &skin(10, 0), 0, 0, 11, UNIT_BOX),
        Err(SkinError::BindingCountMismatch {
            vertices: 11,
            bindings: 10
        })
    );
}

#[test]
fn a_skeleton_larger_than_the_palette_layout_is_refused_with_both_numbers() {
    let mut palettes = SkinningPalettes::new(SkinBudget::default());
    let big = skeleton(MAX_JOINTS_PER_SKELETON + 1);
    assert_eq!(
        palettes.register(&big, &skin(1, 0), 0, 0, 1, UNIT_BOX),
        Err(SkinError::TooManyJoints {
            joints: MAX_JOINTS_PER_SKELETON + 1,
            limit: MAX_JOINTS_PER_SKELETON,
        })
    );
}

/// The budget is the honest cost of skin-to-buffer, so it has to actually bite.
#[test]
fn the_posed_vertex_budget_refuses_rather_than_allocating_until_the_device_stops() {
    let mut palettes = SkinningPalettes::new(SkinBudget {
        max_posed_vertices: 150,
        max_instances: 16,
    });
    let skeleton = skeleton(2);
    palettes
        .register(&skeleton, &skin(100, 0), 0, 0, 100, UNIT_BOX)
        .expect("first fits");
    assert_eq!(
        palettes.register(&skeleton, &skin(100, 0), 0, 100, 100, UNIT_BOX),
        Err(SkinError::PosedVertexBudget {
            wanted: 100,
            remaining: 50
        }),
        "the budget did not bite"
    );
    // ...and something that does fit still gets in.
    assert!(
        palettes
            .register(&skeleton, &skin(50, 0), 0, 100, 50, UNIT_BOX)
            .is_ok()
    );
}

#[test]
fn the_instance_budget_bites_too() {
    let mut palettes = SkinningPalettes::new(SkinBudget {
        max_posed_vertices: 1_000_000,
        max_instances: 1,
    });
    let skeleton = skeleton(2);
    palettes
        .register(&skeleton, &skin(1, 0), 0, 0, 1, UNIT_BOX)
        .expect("first fits");
    assert_eq!(
        palettes.register(&skeleton, &skin(1, 0), 0, 1, 1, UNIT_BOX),
        Err(SkinError::InstanceBudget { limit: 1 })
    );
}

#[test]
fn instances_get_distinct_palette_regions() {
    let mut palettes = SkinningPalettes::new(SkinBudget::default());
    let a = palettes
        .register(&skeleton(4), &skin(10, 0), 0, 0, 10, UNIT_BOX)
        .unwrap();
    let b = palettes
        .register(&skeleton(7), &skin(10, 0), 100, 200, 10, UNIT_BOX)
        .unwrap();
    let regions = palettes.instances();
    assert_eq!(regions[a.0 as usize].palette_base, 0);
    assert_eq!(
        regions[b.0 as usize].palette_base, 4,
        "the second instance overlapped the first's joints"
    );
    assert_eq!(palettes.palette().len(), 11);
}

// ── palettes ────────────────────────────────────────────────────────────────

#[test]
fn a_palette_writes_only_its_own_instances_joints() {
    let mut palettes = SkinningPalettes::new(SkinBudget::default());
    let a = palettes
        .register(&skeleton(2), &skin(4, 0), 0, 0, 4, UNIT_BOX)
        .unwrap();
    let b = palettes
        .register(&skeleton(2), &skin(4, 0), 4, 8, 4, UNIT_BOX)
        .unwrap();

    let moved = Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0));
    assert!(palettes.set_palette(b, &[moved, moved]));

    assert_eq!(
        palettes.palette()[0],
        Mat4::IDENTITY,
        "instance a was written"
    );
    assert_eq!(palettes.palette()[1], Mat4::IDENTITY);
    assert_eq!(palettes.palette()[2], moved);
    assert_eq!(palettes.palette()[3], moved);
    let _ = a;
}

/// A half-written palette is a character with some joints from this frame and
/// some from the last, which reads as a limb tearing off.
#[test]
fn a_wrong_length_palette_is_refused_and_writes_nothing() {
    let mut palettes = SkinningPalettes::new(SkinBudget::default());
    let handle = palettes
        .register(&skeleton(3), &skin(4, 0), 0, 0, 4, UNIT_BOX)
        .unwrap();
    let moved = Mat4::from_translation(Vec3::X);
    assert!(!palettes.set_palette(handle, &[moved, moved]));
    assert!(
        palettes.palette().iter().all(|m| *m == Mat4::IDENTITY),
        "it wrote part of the palette anyway"
    );
    assert!(palettes.set_palette(handle, &[moved; 3]));
}

#[test]
fn an_unknown_handle_is_refused() {
    let mut palettes = SkinningPalettes::new(SkinBudget::default());
    assert!(!palettes.set_palette(SkinnedHandle(7), &[Mat4::IDENTITY]));
    assert!(palettes.posed_bounds(SkinnedHandle(7)).is_none());
}

// ── conservative bounds, which is what makes this correct rather than working ─

#[test]
fn posed_bounds_at_rest_are_the_rest_bounds() {
    let mut palettes = SkinningPalettes::new(SkinBudget::default());
    let handle = palettes
        .register(&skeleton(2), &skin(4, 0), 0, 0, 4, UNIT_BOX)
        .unwrap();
    let (min, max) = palettes.posed_bounds(handle).expect("bounds");
    for axis in 0..3 {
        assert!((min[axis] - (-1.0)).abs() < 1e-5, "{min:?}");
        assert!((max[axis] - 1.0).abs() < 1e-5, "{max:?}");
    }
}

/// The bug this exists to prevent: the pool's stored AABB was computed from the
/// rest pose, so a character that walks away from it gets culled while still on
/// screen.
#[test]
fn posed_bounds_follow_a_moving_joint() {
    let mut palettes = SkinningPalettes::new(SkinBudget::default());
    let handle = palettes
        .register(&skeleton(2), &skin(4, 0), 0, 0, 4, UNIT_BOX)
        .unwrap();
    let moved = Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0));
    assert!(palettes.set_palette(handle, &[Mat4::IDENTITY, moved]));

    let (min, max) = palettes.posed_bounds(handle).expect("bounds");
    assert!(max[0] >= 11.0 - 1e-4, "the box did not follow: {max:?}");
    // Conservative in the other direction too: the unmoved joint keeps the
    // original extent, so the box covers both.
    assert!(
        min[0] <= -1.0 + 1e-4,
        "the box lost the rest extent: {min:?}"
    );
}

#[test]
fn posed_bounds_cover_a_rotated_joint() {
    let mut palettes = SkinningPalettes::new(SkinBudget::default());
    let handle = palettes
        .register(&skeleton(1), &skin(4, 0), 0, 0, 4, UNIT_BOX)
        .unwrap();
    // 45° about z: a unit box's corners reach sqrt(2) along both axes.
    let rotated = Mat4::from_rotation_z(std::f32::consts::FRAC_PI_4);
    assert!(palettes.set_palette(handle, &[rotated]));
    let (_, max) = palettes.posed_bounds(handle).expect("bounds");
    assert!(
        max[0] >= std::f32::consts::SQRT_2 - 1e-4,
        "a rotated box was not covered: {max:?}"
    );
}

#[test]
fn a_palette_with_a_nan_in_it_reports_no_bounds_rather_than_a_nan_box() {
    let mut palettes = SkinningPalettes::new(SkinBudget::default());
    let handle = palettes
        .register(&skeleton(1), &skin(4, 0), 0, 0, 4, UNIT_BOX)
        .unwrap();
    let mut broken = Mat4::IDENTITY;
    broken.x_axis.x = f32::NAN;
    assert!(palettes.set_palette(handle, &[broken]));
    // A NaN box passes every cull test and every reject, which is worse than
    // no box at all.
    assert!(palettes.posed_bounds(handle).is_none());
}

// ── dispatch ────────────────────────────────────────────────────────────────

#[test]
fn an_empty_pass_dispatches_nothing() {
    let palettes = SkinningPalettes::new(SkinBudget::default());
    assert_eq!(palettes.dispatch(), (0, 0, 0));
    assert_eq!(palettes.dispatch_waste(), 0.0);
}

#[test]
fn the_dispatch_covers_the_widest_instance_and_one_row_per_instance() {
    let mut palettes = SkinningPalettes::new(SkinBudget::default());
    let skeleton = skeleton(2);
    palettes
        .register(&skeleton, &skin(100, 0), 0, 0, 100, UNIT_BOX)
        .unwrap();
    palettes
        .register(&skeleton, &skin(200, 0), 100, 300, 200, UNIT_BOX)
        .unwrap();
    // 200 vertices at 64 per group is 4 groups (ceil), 2 instances.
    assert_eq!(palettes.dispatch(), (4, 2, 1));
}

#[test]
fn a_vertex_count_on_a_workgroup_boundary_does_not_over_dispatch() {
    let mut palettes = SkinningPalettes::new(SkinBudget::default());
    palettes
        .register(&skeleton(2), &skin(128, 0), 0, 0, 128, UNIT_BOX)
        .unwrap();
    assert_eq!(palettes.dispatch().0, 2, "128 vertices is exactly 2 groups");
}

/// The honest cost of the ragged shape, and a real finding for KENSHI rather
/// than a hypothetical.
#[test]
fn the_ragged_dispatch_reports_what_it_wastes() {
    let mut palettes = SkinningPalettes::new(SkinBudget::default());
    let skeleton = skeleton(2);
    // One hero and two crowd members: the classic bad case.
    palettes
        .register(&skeleton, &skin(6_400, 0), 0, 0, 6_400, UNIT_BOX)
        .unwrap();
    palettes
        .register(&skeleton, &skin(64, 0), 6_400, 20_000, 64, UNIT_BOX)
        .unwrap();
    palettes
        .register(&skeleton, &skin(64, 0), 6_464, 30_000, 64, UNIT_BOX)
        .unwrap();

    let waste = palettes.dispatch_waste();
    assert!(
        waste > 0.6,
        "a hero plus crowd should waste most of the dispatch, got {waste}"
    );

    // A uniform scene wastes nothing.
    let mut uniform = SkinningPalettes::new(SkinBudget::default());
    for i in 0..4 {
        uniform
            .register(
                &skeleton,
                &skin(64, 0),
                i * 64,
                10_000 + i * 64,
                64,
                UNIT_BOX,
            )
            .unwrap();
    }
    assert!(
        uniform.dispatch_waste() < 1e-6,
        "{}",
        uniform.dispatch_waste()
    );
}

// ── the shader agrees with the Rust ─────────────────────────────────────────

/// The mismatch MORROWIND-D found — a `vec4<f32>` aligning to 16 and a
/// `[f32; 4]` to 4 — is why this test exists for every struct that crosses.
#[test]
fn the_shader_and_the_rust_agree_on_layout_and_workgroup_size() {
    let source = include_str!("../shaders/skinning.wgsl");

    assert!(
        source.contains("@workgroup_size(64)"),
        "the shader's workgroup size no longer matches WORKGROUP_SIZE = {WORKGROUP_SIZE}"
    );

    // Both structs are four u32 in the same order the Rust declares them.
    for (name, fields) in [
        (
            "struct SkinVertex",
            ["joints_01", "joints_23", "weights_01", "weights_23"],
        ),
        (
            "struct SkinInstance",
            [
                "rest_offset",
                "posed_offset",
                "vertex_count",
                "palette_base",
            ],
        ),
    ] {
        let start = source
            .find(name)
            .unwrap_or_else(|| panic!("{name} missing"));
        let body = &source[start..start + source[start..].find('}').expect("unclosed struct")];
        let mut cursor = 0;
        for field in fields {
            let at = body[cursor..]
                .find(field)
                .unwrap_or_else(|| panic!("{name} has no `{field}`, or it moved"));
            cursor += at + field.len();
        }
    }

    assert_eq!(std::mem::size_of::<SkinVertex>(), 16);
    assert_eq!(std::mem::size_of::<SkinInstance>(), 16);

    // The shader unpacks weights with `unpack2x16float`, which is what
    // `pack_f16x2` has to agree with bit for bit.
    assert!(source.contains("unpack2x16float"));
}

/// The decision this sub-phase is about, asserted where somebody changing it
/// will see it: the pass writes into the pool, which is what keeps culling,
/// Hi-Z and ray tracing working unchanged.
#[test]
fn the_shader_writes_posed_vertices_into_the_shared_pool() {
    let source = include_str!("../shaders/skinning.wgsl");
    assert!(
        source.contains("var<storage, read_write> pool: array<Vertex>"),
        "skin-to-buffer means writing into the pool; this shader no longer does"
    );
    assert!(source.contains("pool[dst] = posed"));
}
