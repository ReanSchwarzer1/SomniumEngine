//! Async chunk lifecycle manager.
//!
//! Adapts bevy_voxel_world's `ChunkThread` task pattern and `NeedsRemesh` /
//! `NeedsDespawn` marker components (ATTRIBUTION.md §13.10) to Somnium:
//! tasks run on the rayon global pool and report back over an `mpsc` channel;
//! remesh/despawn markers become per-chunk dirty flags plus a version counter
//! that lets the main thread discard results that were obsoleted by an edit
//! while the worker was still meshing.

use crate::chunk::{chunk_origin, chunks_touching_voxel, ChunkCoord, CHUNK_SIZE, PADDED_CHUNK_SIZE};
use crate::mesh::{mesh_chunk, ChunkMeshData, MAX_LOD};
use crate::terrain::TerrainConfig;
use crate::voxel::Voxel;
use glam::{IVec3, Vec3};
use ndshape::{RuntimeShape, Shape};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};

/// Tuning knobs for the chunk streaming system.
#[derive(Debug, Clone)]
pub struct VoxelWorldConfig {
    /// Terrain generator parameters.
    pub terrain: TerrainConfig,
    /// Horizontal load radius around the camera, in chunks.
    pub radius_chunks: i32,
    /// Lowest vertical chunk loaded (inclusive).
    pub min_chunk_y: i32,
    /// Highest vertical chunk loaded (inclusive).
    pub max_chunk_y: i32,
    /// Horizontal chunk distance at which LOD drops to half resolution…
    pub lod1_distance: f32,
    /// …and to quarter resolution.
    pub lod2_distance: f32,
    /// Maximum number of generation tasks in flight at once.
    pub max_in_flight: usize,
    /// Extra chunks beyond `radius_chunks` kept alive before despawning
    /// (hysteresis so chunks don't thrash at the load boundary).
    pub keep_margin: i32,
}

impl Default for VoxelWorldConfig {
    fn default() -> Self {
        Self {
            terrain: TerrainConfig::default(),
            radius_chunks: 5,
            min_chunk_y: -1,
            max_chunk_y: 0,
            lod1_distance: 2.5,
            lod2_distance: 4.0,
            max_in_flight: 16,
            keep_margin: 1,
        }
    }
}

/// A finished chunk mesh handed back to the integration layer.
pub struct ReadyChunk {
    pub coord: ChunkCoord,
    /// LOD level the mesh was built at (0 = full resolution).
    pub lod: u8,
    /// World-space position of the chunk's minimum corner.
    pub origin: Vec3,
    /// `None` when the chunk has no visible faces (all air / fully buried) —
    /// the caller should free any previous GPU allocation and draw nothing.
    pub mesh: Option<ChunkMeshData>,
}

/// Everything that changed during one `VoxelWorld::update` call.
#[derive(Default)]
pub struct VoxelWorldUpdate {
    /// Chunks with freshly meshed data, ready for GPU upload.
    pub ready: Vec<ReadyChunk>,
    /// Chunks that left the load radius; the caller frees their GPU memory.
    pub despawned: Vec<ChunkCoord>,
}

struct ChunkState {
    /// LOD currently uploaded (or being handed to the caller). `None` until
    /// the first mesh result lands.
    uploaded_lod: Option<u8>,
    /// `(version, lod)` of the in-flight generation task, if any.
    pending: Option<(u64, u8)>,
    /// Bumped on every edit; stale task results are discarded by comparison.
    version: u64,
    /// Set by `set_voxel`; cleared when a remesh task is queued.
    dirty: bool,
}

struct TaskResult {
    coord: ChunkCoord,
    lod: u8,
    version: u64,
    mesh: Option<ChunkMeshData>,
}

/// Streaming voxel world: owns chunk lifecycle state and the edit overlay,
/// but no GPU resources — the caller uploads meshes and submits draws.
pub struct VoxelWorld {
    config: VoxelWorldConfig,
    chunks: HashMap<ChunkCoord, ChunkState>,
    /// Sparse player edits keyed by world voxel coordinate. Generation samples
    /// this overlay on top of the deterministic terrain function, so chunks
    /// never need their voxel arrays persisted.
    edits: HashMap<IVec3, Voxel>,
    tx: Sender<TaskResult>,
    rx: Receiver<TaskResult>,
    in_flight: usize,
}

impl VoxelWorld {
    pub fn new(config: VoxelWorldConfig) -> Self {
        let (tx, rx) = channel();
        Self {
            config,
            chunks: HashMap::new(),
            edits: HashMap::new(),
            tx,
            rx,
            in_flight: 0,
        }
    }

    /// The voxel at a world-space voxel coordinate (edits override terrain).
    pub fn get_voxel(&self, pos: IVec3) -> Voxel {
        self.edits
            .get(&pos)
            .copied()
            .unwrap_or_else(|| self.config.terrain.voxel(pos))
    }

    /// Set a voxel and mark every chunk whose padded volume contains it for
    /// remeshing. Chunks that aren't currently loaded simply pick the edit up
    /// from the overlay whenever they are generated.
    pub fn set_voxel(&mut self, pos: IVec3, voxel: Voxel) {
        self.edits.insert(pos, voxel);
        for coord in chunks_touching_voxel(pos) {
            if let Some(state) = self.chunks.get_mut(&coord) {
                state.version += 1;
                state.dirty = true;
            }
        }
    }

    /// Number of chunk generation tasks currently running.
    pub fn in_flight(&self) -> usize {
        self.in_flight
    }

    /// The configuration this world was created with.
    pub fn config(&self) -> &VoxelWorldConfig {
        &self.config
    }

    /// Drive the streaming system for one frame.
    ///
    /// Collects finished meshes, despawns out-of-range chunks, and queues
    /// generation tasks (nearest first) for missing, dirty, or LOD-changed
    /// chunks, up to `max_in_flight`.
    pub fn update(&mut self, camera_pos: Vec3) -> VoxelWorldUpdate {
        let mut result = VoxelWorldUpdate::default();

        // ── 1. Drain finished tasks ─────────────────────────────────────────
        while let Ok(task) = self.rx.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            let Some(state) = self.chunks.get_mut(&task.coord) else {
                continue; // chunk despawned while meshing
            };
            if state.pending == Some((task.version, task.lod)) {
                state.pending = None;
            }
            if task.version != state.version {
                continue; // edited while meshing — a remesh is already due
            }
            state.uploaded_lod = Some(task.lod);
            result.ready.push(ReadyChunk {
                coord: task.coord,
                lod: task.lod,
                origin: chunk_origin(task.coord),
                mesh: task.mesh,
            });
        }

        // ── 2. Desired chunk set around the camera ─────────────────────────
        let cam_chunk_x = (camera_pos.x / CHUNK_SIZE as f32).floor() as i32;
        let cam_chunk_z = (camera_pos.z / CHUNK_SIZE as f32).floor() as i32;
        let radius = self.config.radius_chunks;

        let mut desired: HashMap<ChunkCoord, u8> = HashMap::new();
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                let dist = ((dx * dx + dz * dz) as f32).sqrt();
                if dist > radius as f32 + 0.001 {
                    continue;
                }
                let lod = if dist <= self.config.lod1_distance {
                    0
                } else if dist <= self.config.lod2_distance {
                    1
                } else {
                    MAX_LOD
                };
                for cy in self.config.min_chunk_y..=self.config.max_chunk_y {
                    desired.insert(
                        IVec3::new(cam_chunk_x + dx, cy, cam_chunk_z + dz),
                        lod,
                    );
                }
            }
        }

        // ── 3. Despawn chunks outside radius + margin ───────────────────────
        let keep = (radius + self.config.keep_margin) as f32;
        self.chunks.retain(|coord, _| {
            let dx = (coord.x - cam_chunk_x) as f32;
            let dz = (coord.z - cam_chunk_z) as f32;
            if (dx * dx + dz * dz).sqrt() <= keep + 0.001 {
                true
            } else {
                result.despawned.push(*coord);
                false
            }
        });

        // ── 4. Queue generation tasks, nearest chunks first ─────────────────
        let mut candidates: Vec<(i64, ChunkCoord, u8)> = desired
            .iter()
            .filter_map(|(&coord, &lod)| {
                let needs_work = match self.chunks.get(&coord) {
                    None => true,
                    Some(s) => s.pending.is_none() && (s.dirty || s.uploaded_lod != Some(lod)),
                };
                if !needs_work {
                    return None;
                }
                let dx = (coord.x - cam_chunk_x) as i64;
                let dz = (coord.z - cam_chunk_z) as i64;
                Some((dx * dx + dz * dz, coord, lod))
            })
            .collect();
        candidates.sort_unstable_by_key(|&(d, ..)| d);

        for (_, coord, lod) in candidates {
            if self.in_flight >= self.config.max_in_flight {
                break;
            }
            self.spawn_task(coord, lod);
        }

        result
    }

    fn spawn_task(&mut self, coord: ChunkCoord, lod: u8) {
        let state = self.chunks.entry(coord).or_insert(ChunkState {
            uploaded_lod: None,
            pending: None,
            version: 0,
            dirty: false,
        });
        let version = state.version;
        state.pending = Some((version, lod));
        state.dirty = false;
        self.in_flight += 1;

        let terrain = self.config.terrain.clone();
        let edits = snapshot_edits(&self.edits, coord);
        let tx = self.tx.clone();

        rayon::spawn(move || {
            let voxels = generate_padded(&terrain, &edits, coord);
            let mesh = mesh_chunk(&voxels, lod);
            // Receiver gone only during engine shutdown — safe to ignore.
            let _ = tx.send(TaskResult { coord, lod, version, mesh });
        });
    }
}

/// Edits that fall inside the padded volume of `coord`, re-keyed for workers.
fn snapshot_edits(edits: &HashMap<IVec3, Voxel>, coord: ChunkCoord) -> HashMap<IVec3, Voxel> {
    let min = coord * CHUNK_SIZE as i32 - IVec3::ONE;
    let max = min + IVec3::splat(PADDED_CHUNK_SIZE as i32 - 1);
    edits
        .iter()
        .filter(|(p, _)| {
            p.x >= min.x && p.x <= max.x
                && p.y >= min.y && p.y <= max.y
                && p.z >= min.z && p.z <= max.z
        })
        .map(|(p, v)| (*p, *v))
        .collect()
}

/// Sample the padded 34³ voxel volume for a chunk (terrain + edit overlay).
fn generate_padded(
    terrain: &TerrainConfig,
    edits: &HashMap<IVec3, Voxel>,
    coord: ChunkCoord,
) -> Vec<Voxel> {
    let shape = RuntimeShape::<u32, 3>::new([PADDED_CHUNK_SIZE; 3]);
    let base = coord * CHUNK_SIZE as i32 - IVec3::ONE;

    let mut voxels = Vec::with_capacity(shape.size() as usize);
    for lin in 0..shape.size() {
        let [x, y, z] = shape.delinearize(lin);
        let pos = base + IVec3::new(x as i32, y as i32, z as i32);
        let v = edits
            .get(&pos)
            .copied()
            .unwrap_or_else(|| terrain.voxel(pos));
        voxels.push(v);
    }
    voxels
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn pump(world: &mut VoxelWorld, camera: Vec3) -> VoxelWorldUpdate {
        // Generation is async; poll until all in-flight tasks land.
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut total = world.update(camera);
        while world.in_flight() > 0 {
            assert!(Instant::now() < deadline, "chunk tasks timed out");
            std::thread::sleep(Duration::from_millis(10));
            let upd = world.update(camera);
            total.ready.extend(upd.ready);
            total.despawned.extend(upd.despawned);
        }
        total
    }

    #[test]
    fn streams_chunks_around_camera() {
        let mut world = VoxelWorld::new(VoxelWorldConfig {
            radius_chunks: 1,
            max_in_flight: 64,
            ..Default::default()
        });
        let upd = pump(&mut world, Vec3::ZERO);
        // radius 1 → 5 columns (diamond) × 2 vertical chunks.
        assert_eq!(upd.ready.len(), 10);
        assert!(upd.ready.iter().any(|c| c.mesh.is_some()));
    }

    #[test]
    fn moving_camera_despawns_far_chunks() {
        let mut world = VoxelWorld::new(VoxelWorldConfig {
            radius_chunks: 1,
            keep_margin: 0,
            max_in_flight: 64,
            ..Default::default()
        });
        pump(&mut world, Vec3::ZERO);
        let upd = pump(&mut world, Vec3::new(10.0 * CHUNK_SIZE as f32, 0.0, 0.0));
        assert!(!upd.despawned.is_empty());
    }

    #[test]
    fn set_voxel_triggers_remesh() {
        let mut world = VoxelWorld::new(VoxelWorldConfig {
            radius_chunks: 0,
            max_in_flight: 64,
            ..Default::default()
        });
        pump(&mut world, Vec3::ZERO);

        // Place a floating stone block well above the terrain surface.
        world.set_voxel(IVec3::new(5, 25, 5), Voxel::Stone);
        let upd = pump(&mut world, Vec3::ZERO);
        let remeshed: Vec<_> = upd.ready.iter().filter(|c| c.coord == IVec3::ZERO).collect();
        assert_eq!(remeshed.len(), 1, "edited chunk should remesh exactly once");
        assert_eq!(world.get_voxel(IVec3::new(5, 25, 5)), Voxel::Stone);
    }
}
