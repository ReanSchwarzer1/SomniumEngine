# Mesh-SDF import cost

Static geometry upload builds a 16³ unsigned distance brick from the first 1,024
triangles after meshlet ordering. Previously every cell walked the entire sample:
up to 4,194,304 triangle-distance calculations per imported primitive. Hundreds of
spatial foliage patches consequently caused a long synchronous scene-restoration
pause, even though their meshlets and GPU buffers were already suitably bounded.

`geometry/mesh_sdf.rs` now builds a temporary median-split triangle AABB tree over
exactly that sample. Nearest-first traversal rejects branches whose conservative
box-distance bound cannot improve the current nearest triangle. Leaves call the
existing `point_triangle_distance`; they do not substitute a bounding-box distance.
The sample cap, invalid-index handling, unsigned distances, brick extent/padding,
meshlet generation, upload allocations and missing-brick fallback are unchanged.
Non-finite triangles or query points retain a brute-force path. Degenerate triangles
keep the existing distance routine's behavior. The tree is freed after baking.

The regression test compares exact float results against the previous brute-force
index walk at face/edge/vertex and outside points, including malformed indices,
degenerates, non-finite coordinates, empty samples and the cap-before-filter rule.
It also compares all 4,096 cells of an actual multi-triangle brick. This verifies
the optimization against the current approximation; it does not make the first
1,024 triangles a complete representation of a larger mesh or introduce a sign.

Worst-case overlapping bounds can still visit every triangle. Native load-time
measurements are separate from functional equality; no startup speedup is claimed
until measured with the same scene and build profile.

A local isolated Rust diagnostic on 2026-09-23 compared a full brick over 1,024
spatially distributed synthetic triangles: 24.24 ms for construction plus BVH
queries versus 367.04 ms for the old cell-by-triangle walk (15.14× in this sample),
with identical output. This was an unoptimized test harness using the existing
glam 0.29 build, not a renderer startup benchmark or an enforced timing threshold.
