//! # Somnium Voxel
//!
//! Chunked voxel world for the Somnium Engine (Phase 14).
//!
//! Pipeline (one chunk):
//!
//! ```text
//! desired set (camera)          edit overlay (set_voxel)
//!         │                              │
//!         ▼                              ▼
//! VoxelWorld::update ──► somnium_jobs job: sample terrain + edits (34³ padded)
//!         ▲                              │ downsample for LOD (18³ / 10³)
//!         │                              │ block_mesh::visible_block_faces
//!         │                              ▼
//!     JobHandle ◄───────────── ChunkMeshData { Vertex[], u32[] }
//!         │
//!         ▼
//! caller uploads to GeometryPool ──► DrawCommand ──► Visibility Buffer
//! ```
//!
//! Chunks are **not** ECS entities — the game submits one `DrawCommand` per
//! visible chunk each frame, so the editor outliner and undo stack are not
//! flooded with hundreds of transient entities.
//!
//! ## Reference Architecture
//!
//! The chunk/meshing/LOD design ports patterns from **bevy_voxel_world**
//! (`example_repo/bevy-plugins/bevy_voxel_world-main/`, MIT/Apache-2.0,
//! © bevy_voxel_world authors — see ATTRIBUTION.md §13.10):
//!
//! - `src/chunk.rs` — 32³ chunks padded to 34³ (1-voxel border so face
//!   culling works across chunk boundaries without seams)
//! - `src/meshing.rs` — `block_mesh::visible_block_faces()` with
//!   `RIGHT_HANDED_Y_UP_CONFIG`; LOD via nearest-neighbour voxel resampling
//!   before meshing (padded bounds kept aligned so outer voxels survive)
//! - `src/chunk.rs::ChunkThread` — async chunk task pattern, adapted from
//!   Bevy task pools to the engine's one `somnium_jobs` scheduler: a typed
//!   `JobHandle` per chunk, at `JobPriority::Visible`, cancelled when the
//!   chunk despawns (DOOM-H)
//! - `NeedsRemesh` marker components — adapted to internal dirty flags +
//!   a version counter that discards stale in-flight results
//!
//! No source code is copied; the patterns are re-implemented against
//! Somnium's `Vertex` type and renderer.

#![warn(clippy::all)]

pub mod chunk;
pub mod mesh;
pub mod terrain;
pub mod voxel;
pub mod world;

pub use chunk::{CHUNK_SIZE, CHUNK_SIZE_F, ChunkCoord, PADDED_CHUNK_SIZE, chunk_origin};
pub use mesh::ChunkMeshData;
pub use terrain::TerrainConfig;
pub use voxel::{PALETTE_SIZE, Voxel};
pub use world::{ReadyChunk, VoxelWorld, VoxelWorldConfig, VoxelWorldUpdate};
