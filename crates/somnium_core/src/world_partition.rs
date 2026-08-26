//! MORROWIND-S world partition and cell-owned entity lifecycle.
//!
//! Want-state is derived only from streaming sources and explicit pins. Disk
//! I/O runs through `somnium_jobs`; ECS mutation remains on the main thread.

use serde::{Deserialize, Serialize};
use somnium_asset::database::AssetId;
use somnium_ecs::{Component, Entity, PersistentId, World};
use somnium_jobs::{JobDesc, JobError, JobHandle, JobPriority, JobSystem};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};

/// Integer coordinate in the partition's spatial hash.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CellCoord {
    /// X cell.
    pub x: i64,
    /// Y cell.
    pub y: i64,
    /// Z cell.
    pub z: i64,
}

impl CellCoord {
    /// Map a double-precision world position to a cell.
    #[must_use]
    pub fn from_position(position: [f64; 3], cell_size: f64) -> Self {
        assert!(cell_size.is_finite() && cell_size > 0.0);
        Self {
            x: (position[0] / cell_size).floor() as i64,
            y: (position[1] / cell_size).floor() as i64,
            z: (position[2] / cell_size).floor() as i64,
        }
    }

    fn center(self, cell_size: f64) -> [f64; 3] {
        [
            (self.x as f64 + 0.5) * cell_size,
            (self.y as f64 + 0.5) * cell_size,
            (self.z as f64 + 0.5) * cell_size,
        ]
    }
}

/// Serialized actor owned by exactly one cell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActorRecord {
    /// Durable cross-cell reference target.
    #[serde(with = "persistent_id_serde")]
    pub id: PersistentId,
    /// Double-precision authored position.
    pub position: [f64; 3],
    /// Real schema-driven one-entity scene document.
    pub document: serde_json::Value,
}

/// Runtime ECS representation of a streamed actor.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamedActor {
    /// Owning cell.
    pub cell: CellCoord,
    /// Double-precision authored position.
    pub position: [f64; 3],
}

impl Component for StreamedActor {}

/// Camera, player or explicit-volume source of cell want-state.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamingSource {
    /// Caller-stable source id.
    pub id: u64,
    /// World-space centre.
    pub position: [f64; 3],
    /// Wanted radius in world units.
    pub radius: f64,
    /// Scheduling priority; larger values win.
    pub priority: u8,
    /// Declared requester role and shape.
    pub kind: StreamingSourceKind,
}

/// Why and in what shape a source requests cells.
#[derive(Clone, Debug, PartialEq)]
pub enum StreamingSourceKind {
    /// View-dependent residency.
    Camera,
    /// Gameplay-critical residency.
    Player,
    /// Explicit axis-aligned volume around the source position.
    Volume {
        /// Half-size of the requested axis-aligned box.
        half_extents: [f64; 3],
    },
}

/// Observable cell lifecycle for the editor overlay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CellLoadState {
    /// No actors resident.
    #[default]
    Unloaded,
    /// A load job is in flight.
    Loading,
    /// Actors are live in the ECS.
    Loaded,
    /// An unload persistence job is in flight.
    Unloading,
}

/// One row/cell in the editor's load-state overlay model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellDiagnostic {
    /// Cell coordinate.
    pub coord: CellCoord,
    /// Current lifecycle state.
    pub state: CellLoadState,
    /// Highest source priority wanting the cell.
    pub priority: Option<u8>,
    /// Whether an editor pin forces residency.
    pub pinned: bool,
    /// Active actors owned by the cell.
    pub actor_count: usize,
}

/// World-aligned line consumed by the editor gizmo pass for the cell overlay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellOverlayLine {
    /// Segment start in double-precision world coordinates.
    pub from: [f64; 3],
    /// Segment end in double-precision world coordinates.
    pub to: [f64; 3],
    /// Cell state selecting the overlay colour.
    pub state: CellLoadState,
    /// Pinned cells receive the authored-override accent.
    pub pinned: bool,
}

/// Undoable editor pin/unpin operation for one cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellPinCommand {
    /// Target cell.
    pub coord: CellCoord,
    /// Pin state before execution.
    pub before: bool,
    /// Pin state after execution.
    pub after: bool,
}

impl CellPinCommand {
    /// Apply the authored override.
    pub fn execute(self, partition: &mut WorldPartition) {
        partition.set_pin(self.coord, self.after);
    }

    /// Restore the prior authored override.
    pub fn undo(self, partition: &mut WorldPartition) {
        partition.set_pin(self.coord, self.before);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CellIndex {
    version: u32,
    coord: CellCoord,
    #[serde(with = "persistent_ids_serde")]
    actors: Vec<PersistentId>,
    derived: Vec<AssetId>,
}

/// Filesystem layout with a separate JSON document per actor.
#[derive(Clone, Debug)]
pub struct PartitionStore {
    root: PathBuf,
}

impl PartitionStore {
    /// Open a partition storage root.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn cell_root(&self, coord: CellCoord) -> PathBuf {
        self.root
            .join(format!("{}_{}_{}", coord.x, coord.y, coord.z))
    }

    /// Persist a cell index and one deterministic file per actor.
    pub fn save_cell(&self, coord: CellCoord, records: &[ActorRecord]) -> Result<(), String> {
        self.save_cell_with_derived(coord, records, &[])
    }

    /// Persist authored actors plus sorted cook-derived content ids.
    pub fn save_cell_with_derived(
        &self,
        coord: CellCoord,
        records: &[ActorRecord],
        derived: &[AssetId],
    ) -> Result<(), String> {
        let cell_root = self.cell_root(coord);
        let actors_root = cell_root.join("actors");
        fs::create_dir_all(&actors_root).map_err(|error| error.to_string())?;
        let mut records = records.to_vec();
        records.sort_by_key(|record| record.id);
        let ids: BTreeSet<_> = records.iter().map(|record| record.id).collect();
        if ids.len() != records.len() {
            return Err("cell contains duplicate stable actor ids".into());
        }
        for record in &records {
            let bytes = canonical_json(record)?;
            write_atomic(&actors_root.join(format!("{}.json", record.id)), &bytes)?;
        }
        for entry in fs::read_dir(&actors_root).map_err(|error| error.to_string())? {
            let path = entry.map_err(|error| error.to_string())?.path();
            let keep = path
                .file_stem()
                .and_then(std::ffi::OsStr::to_str)
                .and_then(PersistentId::parse_hex)
                .is_some_and(|id| ids.contains(&id));
            if path.extension().and_then(std::ffi::OsStr::to_str) == Some("json") && !keep {
                fs::remove_file(&path).map_err(|error| error.to_string())?;
            }
        }
        let mut derived = derived.to_vec();
        derived.sort_unstable();
        derived.dedup();
        let index = CellIndex {
            version: 1,
            coord,
            actors: records.iter().map(|record| record.id).collect(),
            derived,
        };
        write_atomic(&cell_root.join("cell.json"), &canonical_json(&index)?)
    }

    /// Load and validate a cell's actor documents.
    pub fn load_cell(&self, coord: CellCoord) -> Result<Vec<ActorRecord>, String> {
        let cell_root = self.cell_root(coord);
        let index_bytes = match fs::read(cell_root.join("cell.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.to_string()),
        };
        let index: CellIndex =
            serde_json::from_slice(&index_bytes).map_err(|error| error.to_string())?;
        if index.version != 1 || index.coord != coord {
            return Err("cell index version or coordinate mismatch".into());
        }
        if !index.actors.windows(2).all(|pair| pair[0] < pair[1]) {
            return Err("cell actor ids are not unique and sorted".into());
        }
        index
            .actors
            .into_iter()
            .map(|id| {
                let path = cell_root.join("actors").join(format!("{id}.json"));
                let record: ActorRecord = serde_json::from_slice(
                    &fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?,
                )
                .map_err(|error| error.to_string())?;
                if record.id != id {
                    return Err("actor document identity or owning cell mismatch".into());
                }
                Ok(record)
            })
            .collect()
    }

    /// Cook-derived assets attached to a cell, sorted and deduplicated.
    pub fn load_derived(&self, coord: CellCoord) -> Result<Vec<AssetId>, String> {
        let index: CellIndex = serde_json::from_slice(
            &fs::read(self.cell_root(coord).join("cell.json"))
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        Ok(index.derived)
    }
}

enum FlightKind {
    Load,
    Unload,
}

struct CellFlight {
    kind: FlightKind,
    handle: JobHandle<Vec<ActorRecord>>,
}

/// Main-thread world-partition coordinator.
pub struct WorldPartition {
    cell_size: f64,
    store: PartitionStore,
    sources: BTreeMap<u64, StreamingSource>,
    pins: BTreeSet<CellCoord>,
    wanted: BTreeMap<CellCoord, u8>,
    states: BTreeMap<CellCoord, CellLoadState>,
    active: BTreeMap<PersistentId, (CellCoord, Entity)>,
    flights: BTreeMap<CellCoord, CellFlight>,
}

impl WorldPartition {
    /// Create a partition with a configurable spatial-hash cell size.
    #[must_use]
    pub fn new(store: PartitionStore, cell_size: f64) -> Self {
        assert!(cell_size.is_finite() && cell_size > 0.0);
        Self {
            cell_size,
            store,
            sources: BTreeMap::new(),
            pins: BTreeSet::new(),
            wanted: BTreeMap::new(),
            states: BTreeMap::new(),
            active: BTreeMap::new(),
            flights: BTreeMap::new(),
        }
    }

    /// Insert or replace a source, then recompute want-state.
    pub fn set_source(&mut self, source: StreamingSource) {
        self.sources.insert(source.id, source);
        self.recompute_wanted();
        self.cancel_obsolete_flights();
    }

    /// Remove a source and cancel loads it no longer justifies.
    pub fn remove_source(&mut self, id: u64) {
        self.sources.remove(&id);
        self.recompute_wanted();
        self.cancel_obsolete_flights();
    }

    /// Force a cell resident from the editor.
    pub fn pin(&mut self, coord: CellCoord) {
        self.pins.insert(coord);
        self.recompute_wanted();
    }

    /// Return a cell to source-derived residency.
    pub fn unpin(&mut self, coord: CellCoord) {
        self.pins.remove(&coord);
        self.recompute_wanted();
    }

    /// Query an editor pin.
    #[must_use]
    pub fn is_pinned(&self, coord: CellCoord) -> bool {
        self.pins.contains(&coord)
    }

    fn set_pin(&mut self, coord: CellCoord, pinned: bool) {
        if pinned {
            self.pin(coord);
        } else {
            self.unpin(coord);
        }
    }

    /// Install authored actors directly, used by editor creation and tests.
    pub fn activate(
        &mut self,
        world: &mut World,
        coord: CellCoord,
        records: &[ActorRecord],
    ) -> Result<(), String> {
        let registry = crate::reflect_registry::component_registry();
        for record in records {
            if self.active.contains_key(&record.id)
                || world.entity_by_persistent_id(record.id).is_some()
            {
                return Err(format!("persistent actor {} is already live", record.id));
            }
            let report = crate::scene_schema::scene_from_json(world, &registry, &record.document)
                .map_err(|error| error.to_string())?;
            if report.entities.len() != 1 {
                return Err("actor file must contain exactly one entity".into());
            }
            let entity = report.entities[0];
            if world.persistent_id(entity) != Some(record.id) {
                return Err("actor file identity disagrees with its filename".into());
            }
            world
                .insert_component(
                    entity,
                    StreamedActor {
                        cell: coord,
                        position: record.position,
                    },
                )
                .map_err(|error| error.to_string())?;
            self.active.insert(record.id, (coord, entity));
        }
        self.states.insert(coord, CellLoadState::Loaded);
        Ok(())
    }

    /// Poll completions and schedule every required load/unload with deadlines.
    pub fn update(
        &mut self,
        world: &mut World,
        jobs: &mut JobSystem,
        deadline: Instant,
    ) -> Result<(), JobError> {
        self.apply_completions(world);
        let known: BTreeSet<_> = self
            .states
            .keys()
            .chain(self.wanted.keys())
            .copied()
            .collect();
        for coord in known {
            let wanted = self.wanted.contains_key(&coord);
            let state = self.states.get(&coord).copied().unwrap_or_default();
            if wanted && state == CellLoadState::Unloaded && !self.flights.contains_key(&coord) {
                self.schedule_load(jobs, coord, deadline)?;
            } else if !wanted
                && state == CellLoadState::Loaded
                && !self.flights.contains_key(&coord)
            {
                self.schedule_unload(world, jobs, coord, deadline)?;
            }
        }
        self.apply_completions(world);
        Ok(())
    }

    /// Stable editor overlay model, sorted by coordinate.
    #[must_use]
    pub fn diagnostics(&self) -> Vec<CellDiagnostic> {
        let coords: BTreeSet<_> = self
            .states
            .keys()
            .chain(self.wanted.keys())
            .chain(self.pins.iter())
            .copied()
            .collect();
        coords
            .into_iter()
            .map(|coord| CellDiagnostic {
                coord,
                state: self.states.get(&coord).copied().unwrap_or_default(),
                priority: self.wanted.get(&coord).copied(),
                pinned: self.pins.contains(&coord),
                actor_count: self
                    .active
                    .values()
                    .filter(|(owner, _)| *owner == coord)
                    .count(),
            })
            .collect()
    }

    /// World-aligned XZ cell-grid segments for the existing gizmo line pass.
    #[must_use]
    pub fn overlay_lines(&self, y: f64) -> Vec<CellOverlayLine> {
        self.diagnostics()
            .into_iter()
            .flat_map(|cell| {
                let x0 = cell.coord.x as f64 * self.cell_size;
                let z0 = cell.coord.z as f64 * self.cell_size;
                let x1 = x0 + self.cell_size;
                let z1 = z0 + self.cell_size;
                [
                    ([x0, y, z0], [x1, y, z0]),
                    ([x1, y, z0], [x1, y, z1]),
                    ([x1, y, z1], [x0, y, z1]),
                    ([x0, y, z1], [x0, y, z0]),
                ]
                .map(|(from, to)| CellOverlayLine {
                    from,
                    to,
                    state: cell.state,
                    pinned: cell.pinned,
                })
            })
            .collect()
    }

    fn recompute_wanted(&mut self) {
        self.wanted.clear();
        for coord in &self.pins {
            self.wanted.insert(*coord, u8::MAX);
        }
        for source in self.sources.values() {
            let radius = source.radius.max(0.0);
            let extents = match source.kind {
                StreamingSourceKind::Volume { half_extents } => half_extents,
                StreamingSourceKind::Camera | StreamingSourceKind::Player => [radius; 3],
            };
            let min = CellCoord::from_position(
                [
                    source.position[0] - extents[0],
                    source.position[1] - extents[1],
                    source.position[2] - extents[2],
                ],
                self.cell_size,
            );
            let max = CellCoord::from_position(
                [
                    source.position[0] + extents[0],
                    source.position[1] + extents[1],
                    source.position[2] + extents[2],
                ],
                self.cell_size,
            );
            for z in min.z..=max.z {
                for y in min.y..=max.y {
                    for x in min.x..=max.x {
                        let coord = CellCoord { x, y, z };
                        let centre = coord.center(self.cell_size);
                        let distance_squared = centre
                            .iter()
                            .zip(source.position)
                            .map(|(cell, source)| (cell - source).powi(2))
                            .sum::<f64>();
                        let allowance = radius + self.cell_size * 3.0_f64.sqrt() * 0.5;
                        if matches!(source.kind, StreamingSourceKind::Volume { .. })
                            || distance_squared <= allowance * allowance
                        {
                            self.wanted
                                .entry(coord)
                                .and_modify(|value| *value = (*value).max(source.priority))
                                .or_insert(source.priority);
                        }
                    }
                }
            }
        }
    }

    fn cancel_obsolete_flights(&self) {
        for (coord, flight) in &self.flights {
            let wanted = self.wanted.contains_key(coord);
            if (!wanted && matches!(flight.kind, FlightKind::Load))
                || (wanted && matches!(flight.kind, FlightKind::Unload))
            {
                flight.handle.cancel();
            }
        }
    }

    fn schedule_load(
        &mut self,
        jobs: &mut JobSystem,
        coord: CellCoord,
        deadline: Instant,
    ) -> Result<(), JobError> {
        let store = self.store.clone();
        let priority = priority(self.wanted.get(&coord).copied().unwrap_or_default());
        let handle = jobs.submit_with(
            JobDesc::new("world.cell.load")
                .priority(priority)
                .deadline(deadline),
            move |context| {
                context
                    .check_cancelled()
                    .map_err(|error| format!("{error:?}"))?;
                store.load_cell(coord)
            },
        )?;
        self.states.insert(coord, CellLoadState::Loading);
        self.flights.insert(
            coord,
            CellFlight {
                kind: FlightKind::Load,
                handle,
            },
        );
        Ok(())
    }

    fn schedule_unload(
        &mut self,
        world: &mut World,
        jobs: &mut JobSystem,
        coord: CellCoord,
        deadline: Instant,
    ) -> Result<(), JobError> {
        let ids: Vec<_> = self
            .active
            .iter()
            .filter_map(|(id, (owner, _))| (*owner == coord).then_some(*id))
            .collect();
        // Empty spatial cells are a normal part of a camera radius. They have
        // no authored state to persist and must not create thousands of empty
        // directories as the camera moves.
        if ids.is_empty() {
            self.states.insert(coord, CellLoadState::Unloaded);
            return Ok(());
        }
        let registry = crate::reflect_registry::component_registry();
        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            let (_, entity) = self.active[&id];
            let position = world
                .get::<StreamedActor>(entity)
                .map_or([0.0; 3], |actor| actor.position);
            let document = crate::scene_schema::entities_to_json(world, &registry, &[entity])
                .map_err(|error| JobError::Failed(error.to_string()))?;
            records.push(ActorRecord {
                id,
                position,
                document,
            });
        }
        let store = self.store.clone();
        let worker_records = records;
        let handle = jobs.submit_with(
            JobDesc::new("world.cell.unload")
                .priority(JobPriority::Normal)
                .deadline(deadline),
            move |context| {
                context
                    .check_cancelled()
                    .map_err(|error| format!("{error:?}"))?;
                store.save_cell(coord, &worker_records)?;
                Ok(worker_records)
            },
        )?;
        self.states.insert(coord, CellLoadState::Unloading);
        self.flights.insert(
            coord,
            CellFlight {
                kind: FlightKind::Unload,
                handle,
            },
        );
        Ok(())
    }

    fn apply_completions(&mut self, world: &mut World) {
        let ready: Vec<_> = self
            .flights
            .iter()
            .filter_map(|(coord, flight)| flight.handle.try_take().map(|result| (*coord, result)))
            .collect();
        for (coord, result) in ready {
            let Some(flight) = self.flights.remove(&coord) else {
                continue;
            };
            match (flight.kind, result) {
                (FlightKind::Load, Ok(records)) if self.wanted.contains_key(&coord) => {
                    let _ = self.activate(world, coord, &records);
                }
                (FlightKind::Load, _) => {
                    self.states.insert(coord, CellLoadState::Unloaded);
                }
                (FlightKind::Unload, Ok(_)) if !self.wanted.contains_key(&coord) => {
                    let ids: Vec<_> = self
                        .active
                        .iter()
                        .filter_map(|(id, (owner, _))| (*owner == coord).then_some(*id))
                        .collect();
                    for id in ids {
                        if let Some((_, entity)) = self.active.remove(&id) {
                            world.despawn(entity);
                        }
                    }
                    self.states.insert(coord, CellLoadState::Unloaded);
                }
                (FlightKind::Unload, _) => {
                    self.states.insert(coord, CellLoadState::Loaded);
                }
            }
        }
    }
}

fn priority(value: u8) -> JobPriority {
    if value >= 192 {
        JobPriority::User
    } else if value >= 64 {
        JobPriority::Visible
    } else {
        JobPriority::Normal
    }
}

fn canonical_json(value: &impl Serialize) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("partition output has no parent")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("tmp");
    let mut file = fs::File::create(&temporary).map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())
}

mod persistent_id_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use somnium_ecs::PersistentId;

    pub fn serialize<S: Serializer>(id: &PersistentId, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&id.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<PersistentId, D::Error> {
        let text = String::deserialize(deserializer)?;
        PersistentId::parse_hex(&text)
            .ok_or_else(|| serde::de::Error::custom("invalid persistent id"))
    }
}

mod persistent_ids_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use somnium_ecs::PersistentId;

    pub fn serialize<S: Serializer>(
        ids: &[PersistentId],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(ids.iter().map(ToString::to_string))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<PersistentId>, D::Error> {
        Vec::<String>::deserialize(deserializer)?
            .into_iter()
            .map(|text| {
                PersistentId::parse_hex(&text)
                    .ok_or_else(|| serde::de::Error::custom("invalid persistent id"))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    static NEXT: AtomicU64 = AtomicU64::new(1);

    fn fixture() -> (PathBuf, PartitionStore) {
        let root = std::env::temp_dir().join(format!(
            "somnium_partition_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        (root.clone(), PartitionStore::new(root))
    }

    fn actor(id: u128, x: f64) -> ActorRecord {
        let id = PersistentId::from_raw(id);
        let mut world = World::new();
        let entity = world.spawn((
            id,
            crate::Name::new("streamed actor"),
            crate::Transform::from_translation(glam::Vec3::new(x as f32, 1.0, 1.0)),
        ));
        ActorRecord {
            id,
            position: [x, 1.0, 1.0],
            document: crate::scene_schema::entities_to_json(
                &mut world,
                &crate::reflect_registry::component_registry(),
                &[entity],
            )
            .unwrap(),
        }
    }

    #[test]
    fn spatial_hash_is_floor_based_across_the_origin() {
        assert_eq!(
            CellCoord::from_position([-0.1, 64.0, 127.9], 64.0),
            CellCoord { x: -1, y: 1, z: 1 }
        );
    }

    #[test]
    fn storage_is_one_file_per_actor_and_round_trips_stable_ids() {
        let (root, store) = fixture();
        let coord = CellCoord::default();
        let records = vec![actor(2, 2.0), actor(1, 1.0)];
        store.save_cell(coord, &records).unwrap();
        assert!(
            root.join("0_0_0/actors/00000000000000000000000000000001.json")
                .is_file()
        );
        assert!(
            root.join("0_0_0/actors/00000000000000000000000000000002.json")
                .is_file()
        );
        let loaded = store.load_cell(coord).unwrap();
        assert_eq!(loaded[0].id, PersistentId::from_raw(1));
        assert_eq!(loaded[1].id, PersistentId::from_raw(2));
        let derived = [AssetId::from_relative_path("cells/origin.hlod")];
        store
            .save_cell_with_derived(coord, &records, &derived)
            .unwrap();
        assert_eq!(store.load_derived(coord).unwrap(), derived);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn want_state_is_only_sources_and_pins() {
        let (root, store) = fixture();
        let mut partition = WorldPartition::new(store, 64.0);
        partition.set_source(StreamingSource {
            id: 9,
            position: [32.0; 3],
            radius: 1.0,
            priority: 77,
            kind: StreamingSourceKind::Camera,
        });
        let origin = CellCoord::default();
        assert_eq!(partition.wanted.get(&origin), Some(&77));
        partition.remove_source(9);
        assert!(!partition.wanted.contains_key(&origin));
        partition.set_source(StreamingSource {
            id: 10,
            position: [32.0; 3],
            radius: 0.0,
            priority: 12,
            kind: StreamingSourceKind::Volume {
                half_extents: [1.0; 3],
            },
        });
        assert_eq!(partition.wanted.get(&origin), Some(&12));
        partition.remove_source(10);
        let command = CellPinCommand {
            coord: origin,
            before: false,
            after: true,
        };
        command.execute(&mut partition);
        assert_eq!(partition.wanted.get(&origin), Some(&u8::MAX));
        assert!(partition.is_pinned(origin));
        assert_eq!(partition.overlay_lines(0.0).len(), 4);
        command.undo(&mut partition);
        assert!(!partition.wanted.contains_key(&origin));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn one_hundred_unload_reload_loops_do_not_leak_entities() {
        let (root, store) = fixture();
        let coord = CellCoord::default();
        store
            .save_cell(coord, &[actor(1, 1.0), actor(2, 2.0)])
            .unwrap();
        let mut partition = WorldPartition::new(store, 64.0);
        let mut world = World::new();
        let mut jobs = JobSystem::single_threaded();
        let deadline = || Instant::now() + Duration::from_secs(1);
        for _ in 0..100 {
            partition.pin(coord);
            partition.update(&mut world, &mut jobs, deadline()).unwrap();
            assert_eq!(world.entity_count(), 2);
            partition.unpin(coord);
            partition.update(&mut world, &mut jobs, deadline()).unwrap();
            assert_eq!(world.entity_count(), 0);
            jobs.prune_finished();
            assert!(jobs.active().is_empty());
        }
        partition.pin(coord);
        partition.update(&mut world, &mut jobs, deadline()).unwrap();
        assert_eq!(world.entity_count(), 2);
        let ids: BTreeSet<_> = world
            .entities()
            .filter_map(|entity| world.persistent_id(entity))
            .collect();
        assert_eq!(
            ids,
            BTreeSet::from([PersistentId::from_raw(1), PersistentId::from_raw(2)])
        );
        assert!(
            world
                .entities()
                .all(|entity| world.get::<crate::Name>(entity).is_some()
                    && world.get::<crate::Transform>(entity).is_some())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn moving_a_source_cancels_an_obsolete_load() {
        let (root, store) = fixture();
        let coord = CellCoord::default();
        store.save_cell(coord, &[actor(1, 1.0)]).unwrap();
        let mut partition = WorldPartition::new(store, 64.0);
        partition.set_source(StreamingSource {
            id: 1,
            position: [32.0; 3],
            radius: 1.0,
            priority: 10,
            kind: StreamingSourceKind::Player,
        });
        let mut world = World::new();
        let mut jobs = JobSystem::with_workers_and_capacity(1, 8);
        partition
            .update(
                &mut world,
                &mut jobs,
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap();
        partition.set_source(StreamingSource {
            id: 1,
            position: [10_000.0; 3],
            radius: 1.0,
            priority: 10,
            kind: StreamingSourceKind::Player,
        });
        let flight = partition.flights.get(&coord).unwrap();
        assert!(flight.handle.cancellation_requested());
        let _ = fs::remove_dir_all(root);
    }
}
