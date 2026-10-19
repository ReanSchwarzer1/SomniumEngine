//! Read-only world cells streamed by distance, for open worlds built offline (Rimeborn's Tyffir).
//!
//! A `somnium.CellStreaming` names a manifest of cells. Each cell is a plain schema scene (a composer's output, never
//! edited in place) with the XZ bounds of what it holds. Cells within reach of the view are read and parsed on worker
//! threads, installed a few per frame into the live world with their meshes restored from the same imported-mesh
//! cache a scene open uses, and despawned once the view is past reach plus a margin. Streamed entities carry
//! [`Streamed`], which is not a registered component: saves, Play checkpoints and undo snapshots leave them out, so
//! a saved scene never fills up with its cells.
//!
//! Not the actor World Partition (`world_partition.rs`): that one keeps one editable file per actor and writes cells
//! back on unload. This one only reads, streams whole hierarchies, and costs nothing for a scene without the
//! component.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use somnium_ecs::component_schema;
use somnium_ecs::reflect::TypeRegistry;
use somnium_ecs::{Component, Entity};

/// Streams the cells of a manifest around the view.
#[derive(Debug, Clone, PartialEq)]
pub struct CellStreamingComponent {
    /// Off: every streamed cell unloads.
    pub enabled: bool,
    /// The cell manifest, relative to the project.
    pub manifest: String,
    /// Cells whose bounds come within this distance of the view load (a cell's own `reach` wins).
    pub load_radius: f32,
    /// Loaded cells unload once the view is this much further than their reach.
    pub unload_margin: f32,
    /// Most cells installed in one frame (each read and parsed off the main thread first).
    pub cells_per_frame: u32,
    /// Live: cells in the world now.
    pub loaded_cells: u32,
    /// Live: cells being read.
    pub pending_cells: u32,
    /// Live: entities the cells hold now.
    pub streamed_entities: u32,
    /// Live: what the streamer is doing.
    pub status: String,
}

impl Component for CellStreamingComponent {}

impl Default for CellStreamingComponent {
    fn default() -> Self {
        Self {
            enabled: true,
            manifest: String::new(),
            load_radius: 600.0,
            unload_margin: 100.0,
            cells_per_frame: 2,
            loaded_cells: 0,
            pending_cells: 0,
            streamed_entities: 0,
            status: String::new(),
        }
    }
}

pub(crate) fn register(registry: &mut TypeRegistry) {
    registry.register(component_schema! {
        TerrainStreamingComponent as "somnium.TerrainStreaming", display "Terrain Streaming", version 1,
        fields {
            geometry_distance { min: 0.0, soft_max: 8192.0, unit: "m", group: "Streaming", doc: "An open world's terrain window: its geometry is resident only while the camera is within this distance of the terrain's edge. 0 keeps it resident. Read when the terrain is created." },
            own_area { group: "Streaming", doc: "The part of the window that is its own where neighbouring windows overlap: x0, z0, x1, z1 in terrain-local metres. Generated ground cover grows only inside it, so an overlap is not covered twice. All zero: the whole terrain." },
        }
    });
    registry.register(component_schema! {
        CellStreamingComponent as "somnium.CellStreaming", display "Cell Streaming", version 1,
        fields {
            enabled { group: "Cells", doc: "Off: every streamed cell unloads." },
            manifest { group: "Cells", doc: "The cell manifest (JSON), relative to the project: {\"version\": 1, \"cells\": [{\"file\", \"min\": [x, z], \"max\": [x, z], \"reach\"?}]}, files relative to the manifest." },
            load_radius { min: 0.0, soft_max: 4096.0, unit: "m", group: "Cells", doc: "Cells whose bounds come this near the view load, unless a cell names its own reach." },
            unload_margin { min: 0.0, soft_max: 1024.0, unit: "m", group: "Cells", doc: "A loaded cell unloads once the view is this much past its reach." },
            cells_per_frame { min: 1, soft_max: 16, group: "Cells", doc: "Most cells installed in one frame." },
            loaded_cells { read_only: true, group: "Diagnostics", flags: FieldFlags::EDIT.union(FieldFlags::SCRIPT_READ) },
            pending_cells { read_only: true, group: "Diagnostics", flags: FieldFlags::EDIT.union(FieldFlags::SCRIPT_READ) },
            streamed_entities { read_only: true, group: "Diagnostics", flags: FieldFlags::EDIT.union(FieldFlags::SCRIPT_READ) },
            status { read_only: true, group: "Diagnostics", flags: FieldFlags::EDIT.union(FieldFlags::SCRIPT_READ) },
        }
    });
}

/// On a terrain: an open world's window, whose chunk geometry is resident only while the camera is within
/// `geometry_distance` metres of it (`TerrainData::stream_geometry`). Read when the terrain is created.
#[derive(Debug, Clone, PartialEq)]
pub struct TerrainStreamingComponent {
    /// Metres from the terrain's edge within which its geometry is resident; 0 keeps it resident always.
    pub geometry_distance: f32,
    /// Terrain-local x0, z0, x1, z1 of the window's own ground; zero is the whole terrain.
    pub own_area: glam::Vec4,
}

impl Component for TerrainStreamingComponent {}

impl Default for TerrainStreamingComponent {
    fn default() -> Self {
        Self { geometry_distance: 800.0, own_area: glam::Vec4::ZERO }
    }
}

/// Marks an entity a cell installed. Not registered, so never saved or checkpointed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Streamed {
    /// The cell's index in the manifest.
    pub cell: u32,
}

impl Component for Streamed {}

/// One cell of a manifest.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct CellEntry {
    /// The cell's scene, relative to the manifest.
    pub file: PathBuf,
    /// Lowest XZ corner of what the cell holds, world metres.
    pub min: [f32; 2],
    /// Highest XZ corner.
    pub max: [f32; 2],
    /// Its own load distance, for cells of big things seen from afar.
    #[serde(default)]
    pub reach: Option<f32>,
}

#[derive(serde::Deserialize)]
struct ManifestFile {
    version: u32,
    cells: Vec<CellEntry>,
}

/// Read a manifest; cell files come back resolved against its folder.
pub fn read_manifest(path: &Path) -> Result<Vec<CellEntry>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file: ManifestFile = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if file.version != 1 {
        return Err(format!("{}: manifest version {} is not 1", path.display(), file.version));
    }
    let folder = path.parent().unwrap_or(Path::new("."));
    Ok(file.cells.into_iter().map(|cell| CellEntry { file: folder.join(&cell.file), ..cell }).collect())
}

/// Distance in XZ from `point` to a cell's bounds (0 inside).
#[must_use]
pub fn distance_to(cell: &CellEntry, point: [f32; 2]) -> f32 {
    let dx = (cell.min[0] - point[0]).max(point[0] - cell.max[0]).max(0.0);
    let dz = (cell.min[1] - point[1]).max(point[1] - cell.max[1]).max(0.0);
    dx.hypot(dz)
}

/// Which cells to start loading (nearest first) and which to unload, for a view at `focus`.
#[must_use]
pub fn plan(
    cells: &[CellEntry],
    focus: [f32; 2],
    radius: f32,
    margin: f32,
    is_resident: impl Fn(usize) -> bool,
) -> (Vec<usize>, Vec<usize>) {
    let mut load = Vec::new();
    let mut unload = Vec::new();
    for (index, cell) in cells.iter().enumerate() {
        let distance = distance_to(cell, focus);
        let reach = cell.reach.unwrap_or(radius);
        if is_resident(index) {
            if distance > reach + margin {
                unload.push(index);
            }
        } else if distance <= reach {
            load.push((distance, index));
        }
    }
    load.sort_by(|a, b| a.0.total_cmp(&b.0));
    (load.into_iter().map(|(_, index)| index).collect(), unload)
}

/// [`plan`] for several points: the view and every point held ([`hold`]). A cell loads when it is within reach of
/// any of them (nearest first), and unloads only once it is past reach and margin from all of them.
#[must_use]
pub fn plan_many(
    cells: &[CellEntry],
    foci: &[[f32; 2]],
    radius: f32,
    margin: f32,
    is_resident: impl Fn(usize) -> bool,
) -> (Vec<usize>, Vec<usize>) {
    let mut load = Vec::new();
    let mut unload = Vec::new();
    for (index, cell) in cells.iter().enumerate() {
        let distance = foci.iter().map(|&focus| distance_to(cell, focus)).fold(f32::INFINITY, f32::min);
        let reach = cell.reach.unwrap_or(radius);
        if is_resident(index) {
            if distance > reach + margin {
                unload.push(index);
            }
        } else if distance <= reach {
            load.push((distance, index));
        }
    }
    load.sort_by(|a, b| a.0.total_cmp(&b.0));
    (load.into_iter().map(|(_, index)| index).collect(), unload)
}

/// Points other than the view whose cells stay loaded, by key, kept with the world (DUAL_PROTAGONISTS DP-2: the
/// switch's destination streams while the camera lifts; both men's cells stay in during a joint mission).
#[derive(Default)]
pub(crate) struct Holds(BTreeMap<String, Hold>);

#[derive(Clone, Copy)]
struct Hold {
    at: [f32; 2],
    ready: bool,
}

impl Holds {
    pub(crate) fn points(&self) -> impl Iterator<Item = [f32; 2]> + '_ {
        self.0.values().map(|hold| hold.at)
    }

    /// After a frame's planning: a hold is ready when every cell within reach of it is loaded.
    pub(crate) fn mark_ready(&mut self, cells: &[CellEntry], radius: f32, loaded: impl Fn(usize) -> bool) {
        for hold in self.0.values_mut() {
            hold.ready = cells
                .iter()
                .enumerate()
                .filter(|(_, cell)| distance_to(cell, hold.at) <= cell.reach.unwrap_or(radius))
                .all(|(index, _)| loaded(index));
        }
    }
}

/// Keep the cells around `at` loaded under `key` (the switch's destination, the other protagonist), as if the view
/// were there too, until [`release`]. Holding a key again moves it; a moved hold is not ready until its new cells are.
pub fn hold(world: &mut somnium_ecs::World, key: &str, at: glam::Vec3) {
    let mut holds: Holds = world.take_cache();
    let at = [at.x, at.z];
    let entry = holds.0.entry(key.to_owned()).or_insert(Hold { at, ready: false });
    if entry.at != at {
        *entry = Hold { at, ready: false };
    }
    world.put_cache(holds);
}

/// Stop holding `key`: its cells go once the view and the other holds are far from them.
pub fn release(world: &mut somnium_ecs::World, key: &str) {
    let mut holds: Holds = world.take_cache();
    holds.0.remove(key);
    world.put_cache(holds);
}

/// Whether every cell around `key`'s point is loaded (as of the streamer's last frame). False for a key not held.
pub fn hold_ready(world: &mut somnium_ecs::World, key: &str) -> bool {
    let holds: Holds = world.take_cache();
    let ready = holds.0.get(key).is_some_and(|hold| hold.ready);
    world.put_cache(holds);
    ready
}

/// What a cell's reader sends, in order: every source it needs that was not uploaded when it started, decoded and
/// prepared off the main thread, then the cell itself. The channel holds one piece, so a reader is never more than
/// one decoded source ahead of the uploads.
pub(crate) enum Piece {
    Source {
        /// As `ImportedMesh.source` resolves in the project (not canonical).
        path: PathBuf,
        parts: HashSet<usize>,
        scene: somnium_asset::LoadedScene,
        prepared: somnium_renderer::PreparedScene,
    },
    Cell(Result<serde_json::Value, String>),
}

/// The streamer's state, owned by the app.
#[derive(Default)]
pub struct CellStreamer {
    /// The component's entity and the manifest it named: a change of either starts over.
    pub(crate) key: Option<(Entity, PathBuf)>,
    pub(crate) cells: Vec<CellEntry>,
    /// Cell index -> the entities it installed.
    pub(crate) loaded: HashMap<usize, Vec<Entity>>,
    pub(crate) reading: HashMap<usize, Receiver<Piece>>,
    /// Cells that failed to read or install: not retried until the manifest changes.
    pub(crate) failed: HashMap<usize, String>,
    /// The first fill (the view's cells taken in whole) has happened for this manifest.
    pub(crate) filled: bool,
    /// Canonical sources each loaded cell draws, and how many loaded cells draw each source.
    pub(crate) cell_sources: HashMap<usize, Vec<PathBuf>>,
    pub(crate) source_users: HashMap<PathBuf, u32>,
    /// Sources no loaded cell draws, since when: released once they have been cold for `COLD_SECONDS`.
    pub(crate) cold: HashMap<PathBuf, std::time::Instant>,
    /// When cold sources were last released (batched: each release waits for the GPU).
    pub(crate) last_eviction: Option<std::time::Instant>,
}

/// How long a streamed mesh no loaded cell uses stays uploaded (coming back soon costs nothing).
pub(crate) const COLD_SECONDS: u64 = 30;

/// Most cells read at once.
pub(crate) const READERS: usize = 4;

/// Each source an entity list draws (`somnium.ImportedMesh` source -> node ordinals), from a scene document.
#[must_use]
pub fn sources_of(document: &serde_json::Value) -> BTreeMap<String, HashSet<usize>> {
    let mut sources: BTreeMap<String, HashSet<usize>> = BTreeMap::new();
    for entity in document.get("entities").and_then(serde_json::Value::as_array).into_iter().flatten() {
        let Some(fields) = entity.pointer("/components/somnium.ImportedMesh/fields") else {
            continue;
        };
        if let (Some(source), Some(node)) =
            (fields.get("source").and_then(serde_json::Value::as_str), fields.get("node").and_then(serde_json::Value::as_u64))
        {
            sources.entry(source.to_owned()).or_default().insert(node as usize);
        }
    }
    sources
}

impl CellStreamer {
    /// Entities installed by every loaded cell.
    #[must_use]
    pub fn streamed_entities(&self) -> usize {
        self.loaded.values().map(Vec::len).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(min: [f32; 2], max: [f32; 2], reach: Option<f32>) -> CellEntry {
        CellEntry { file: PathBuf::from("c.json"), min, max, reach }
    }

    #[test]
    fn near_cells_load_nearest_first_and_far_ones_unload_past_the_margin() {
        let cells = vec![
            cell([0.0, 0.0], [256.0, 256.0], None),       // the view is inside
            cell([800.0, 0.0], [1056.0, 256.0], None),    // 672 m off: beyond 600
            cell([400.0, 0.0], [656.0, 256.0], None),     // 272 m off
            cell([3000.0, 0.0], [4024.0, 1024.0], Some(3000.0)), // a big thing with its own reach
        ];
        let (load, unload) = plan(&cells, [128.0, 128.0], 600.0, 100.0, |_| false);
        assert_eq!(load, vec![0, 2, 3]);
        assert!(unload.is_empty());
        // Resident cells unload only past reach + margin: 672 m stays, the 3 km reach keeps the big cell.
        let (load, unload) = plan(&cells, [128.0, 128.0], 600.0, 100.0, |_| true);
        assert!(load.is_empty());
        assert!(unload.is_empty());
        // 650 m from the first cell keeps it (inside 700); the rest are past their reach + margin.
        let (_, unload) = plan(&cells, [-650.0, 128.0], 600.0, 100.0, |_| true);
        assert_eq!(unload, vec![1, 2, 3]);
    }

    #[test]
    fn a_held_point_streams_like_a_second_view() {
        // Two towns 5 km apart: the view in one, a hold in the other.
        let cells = vec![
            cell([0.0, 0.0], [256.0, 256.0], None),
            cell([5000.0, 0.0], [5256.0, 256.0], None),
            cell([2500.0, 0.0], [2756.0, 256.0], None), // between: near neither
        ];
        let (view, held) = ([128.0, 128.0], [5128.0, 128.0]);
        let (load, _) = plan_many(&cells, &[view], 600.0, 100.0, |_| false);
        assert_eq!(load, vec![0], "the view alone: its own town");
        let (load, _) = plan_many(&cells, &[view, held], 600.0, 100.0, |_| false);
        assert_eq!(load, vec![0, 1], "held: the other town too, not what lies between");
        let (_, unload) = plan_many(&cells, &[view, held], 600.0, 100.0, |_| true);
        assert_eq!(unload, vec![2], "a cell near either point stays");
        assert_eq!(plan_many(&cells, &[view], 600.0, 100.0, |_| false), plan(&cells, view, 600.0, 100.0, |_| false));
    }

    #[test]
    fn a_hold_is_ready_once_its_cells_are_in_and_moving_it_starts_over() {
        let mut world = somnium_ecs::World::new();
        let cells = vec![cell([5000.0, 0.0], [5256.0, 256.0], None), cell([5300.0, 0.0], [5556.0, 256.0], None)];
        assert!(!hold_ready(&mut world, "rhun"), "not held: not ready");
        hold(&mut world, "rhun", glam::Vec3::new(5128.0, 4.0, 128.0));
        assert!(!hold_ready(&mut world, "rhun"));
        // The streamer's frame: one of the two cells near it is in.
        let mark = |world: &mut somnium_ecs::World, loaded: &[usize]| {
            let mut holds: Holds = world.take_cache();
            holds.mark_ready(&cells, 600.0, |i| loaded.contains(&i));
            world.put_cache(holds);
        };
        mark(&mut world, &[0]);
        assert!(!hold_ready(&mut world, "rhun"), "half in");
        mark(&mut world, &[0, 1]);
        assert!(hold_ready(&mut world, "rhun"));
        // Held again at the same point: still ready. Moved: not until the streamer looks again.
        hold(&mut world, "rhun", glam::Vec3::new(5128.0, 9.0, 128.0));
        assert!(hold_ready(&mut world, "rhun"), "height does not move a hold");
        hold(&mut world, "rhun", glam::Vec3::new(9000.0, 0.0, 0.0));
        assert!(!hold_ready(&mut world, "rhun"));
        release(&mut world, "rhun");
        let holds: Holds = world.take_cache();
        assert_eq!(holds.points().count(), 0);
    }

    #[test]
    fn a_cell_names_each_source_once_with_the_parts_it_draws() {
        let document = serde_json::json!({"version": 4, "entities": [
            {"components": {"somnium.ImportedMesh": {"fields": {"source": "a.gltf", "node": 2}}}},
            {"components": {"somnium.ImportedMesh": {"fields": {"source": "a.gltf", "node": 5}}}},
            {"components": {"somnium.ImportedMesh": {"fields": {"source": "b.gltf", "node": 0}}}},
            {"components": {"somnium.Name": {"fields": {"value": "no mesh"}}}},
        ]});
        let sources = sources_of(&document);
        assert_eq!(sources.len(), 2);
        assert_eq!(sources["a.gltf"], HashSet::from([2, 5]));
        assert_eq!(sources["b.gltf"], HashSet::from([0]));
    }

    #[test]
    fn a_manifest_resolves_its_cells_beside_it() {
        let folder = std::env::temp_dir().join(format!("cell_streaming_{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("world.cells.json");
        std::fs::write(
            &path,
            r#"{"version": 1, "cells": [{"file": "cells/0_0.scene.json", "min": [0, 0], "max": [256, 256]}]}"#,
        )
        .unwrap();
        let cells = read_manifest(&path).unwrap();
        assert_eq!(cells[0].file, folder.join("cells/0_0.scene.json"));
        assert_eq!(cells[0].reach, None);
        std::fs::write(&path, r#"{"version": 2, "cells": []}"#).unwrap();
        assert!(read_manifest(&path).is_err());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&folder);
    }
}
