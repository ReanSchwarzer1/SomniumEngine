//! MORROWIND-X/Y engine adapters: cell ownership, job installation and native
//! cooked navigation payloads. All mutations happen on the caller's thread.
#[path = "ai_editor.rs"]
mod editor;
pub use editor::*;

use crate::world_partition::CellCoord;
use somnium_ai::navigation::{BakeInput, Bounds, NavTile, NavWorld, submit_bake};
use somnium_asset::cook::{CookKind, LoadedNativeAsset};
use somnium_jobs::{JobDesc, JobError, JobHandle, JobPriority, JobSystem};
use std::collections::BTreeMap;
use std::time::Duration;

/// Navigation coordinator uses the same coordinates as actor world partition.
/// An unloaded cell cancels work and removes its tile; a replacement request
/// cannot install an obsolete result after a geometry or obstacle edit.
#[derive(Default)]
pub struct NavigationCells {
    /// Installed tiles and their shared query graph.
    pub world: NavWorld,
    inputs: BTreeMap<CellCoord, BakeInput>,
    pending: BTreeMap<CellCoord, JobHandle<NavTile>>,
    obstacles: BTreeMap<u64, Bounds>,
}
impl NavigationCells {
    /// Schedule a visible cell bake, superseding older work only after enqueue.
    pub fn request(
        &mut self,
        jobs: &mut JobSystem,
        cell: CellCoord,
        mut input: BakeInput,
    ) -> Result<(), JobError> {
        input.cell = [cell.x, cell.y, cell.z];
        let authored_input = input.clone();
        input.obstacles.extend(self.obstacles.values().copied());
        // Enqueue before superseding the previous request. Queue exhaustion
        // leaves the previous known-good tile/request intact.
        let handle = submit_bake(
            jobs,
            input.clone(),
            JobDesc::new("navigation.bake")
                .priority(JobPriority::Visible)
                .within(Duration::from_secs(30)),
        )?;
        if let Some(old) = self.pending.insert(cell, handle) {
            old.cancel();
        }
        self.inputs.insert(cell, authored_input);
        Ok(())
    }
    /// Cancel pending work and remove all navigation owned by this cell.
    pub fn unload(&mut self, cell: CellCoord) {
        if let Some(job) = self.pending.remove(&cell) {
            job.cancel();
        }
        self.inputs.remove(&cell);
        self.world.unload([cell.x, cell.y, cell.z]);
    }
    /// Drain data produced on workers. Errors are returned to the caller for
    /// editor diagnostics; a failed bake keeps the previous usable tile.
    pub fn poll(&mut self) -> Vec<(CellCoord, Result<(), JobError>)> {
        let ready: Vec<_> = self
            .pending
            .iter()
            .filter_map(|(cell, job)| job.try_take().map(|result| (*cell, result)))
            .collect();
        let mut results = Vec::new();
        for (cell, result) in ready {
            self.pending.remove(&cell);
            results.push((cell, result.map(|tile| self.world.install(tile))));
        }
        results
    }
    /// Number of cell results awaiting main-thread installation.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
    /// Carve / uncarve only cells overlapping the old or new obstacle. Geometry
    /// is retained for rebaking and obstacles are applied consistently on reload.
    pub fn set_obstacle(
        &mut self,
        jobs: &mut JobSystem,
        id: u64,
        bounds: Option<Bounds>,
    ) -> Result<Vec<CellCoord>, JobError> {
        if bounds.is_some_and(|b| !b.valid()) {
            return Err(JobError::Failed("invalid obstacle bounds".into()));
        }
        let old = self.obstacles.get(&id).copied();
        if let Some(b) = bounds {
            self.obstacles.insert(id, b);
        } else {
            self.obstacles.remove(&id);
        }
        let affected: Vec<_> = self
            .inputs
            .iter()
            .filter_map(|(cell, input)| {
                let mut tile = input.bounds;
                let radius = input.settings.agent_radius + input.settings.voxel_size;
                for axis in [0, 2] {
                    tile.min[axis] -= radius;
                    tile.max[axis] += radius;
                }
                [old, bounds]
                    .into_iter()
                    .flatten()
                    .any(|b| tile.intersects(b))
                    .then_some(*cell)
            })
            .collect();
        // Errors are observable and leave retained inputs for retry; successful
        // requests continue. Do not silently mark every cell clean on queue full.
        for &cell in &affected {
            self.request(jobs, cell, self.inputs[&cell].clone())?;
        }
        Ok(affected)
    }
    /// Install a source-free native artifact through the common cook envelope.
    pub fn install_cooked(&mut self, asset: &LoadedNativeAsset) -> Result<CellCoord, String> {
        let tile = decode_navigation_asset(asset)?;
        let [x, y, z] = tile.cell();
        self.world.install(tile);
        Ok(CellCoord { x, y, z })
    }
}
/// Decode the checked native family envelope produced by MORROWIND-Q.
pub fn decode_navigation_asset(asset: &LoadedNativeAsset) -> Result<NavTile, String> {
    if asset.kind != CookKind::Navigation
        || asset.payload.len() < 20
        || &asset.payload[..8] != b"SOMNAV\0\0"
    {
        return Err("not a cooked navigation asset".into());
    }
    let version = u32::from_le_bytes(asset.payload[8..12].try_into().unwrap());
    let length = u64::from_le_bytes(asset.payload[12..20].try_into().unwrap());
    if version != 1 || length != ((asset.payload.len() - 20) as u64) {
        return Err("invalid navigation payload envelope".into());
    }
    NavTile::decode(&asset.payload[20..])
}

/// Authored graph intent; the shared schema owns its persistence and Details.
#[derive(Clone, Debug, PartialEq)]
pub struct BehaviorComponent {
    /// Whether this entity ticks its authored tree during Play.
    pub enabled: bool,
    /// Versioned MORROWIND-K authoring document, including catalogue identity.
    pub graph_json: String,
    /// Friendly author label displayed beside the enabled control.
    pub label: String,
    /// Live execution outcome; never serialized into authored scenes.
    pub status: String,
    /// Latest compile or task binding diagnostic; never serialized.
    pub last_error: String,
}
impl Default for BehaviorComponent {
    fn default() -> Self {
        let surface = somnium_ui::graph::behavior::default_surface();
        let graph_json = somnium_ui::graph::serial::to_json(&surface.graph, &surface.catalogue)
            .unwrap_or_default();
        Self {
            enabled: true,
            graph_json,
            label: "Behavior Tree".into(),
            status: "Ready — press Play to run".into(),
            last_error: String::new(),
        }
    }
}
impl somnium_ecs::Component for BehaviorComponent {}
struct BehaviorExecution {
    source: String,
    instance: Option<somnium_ai::behavior::BehaviorInstance>,
    error: String,
}
impl somnium_ecs::Component for BehaviorExecution {}
pub(crate) fn invalidate_behavior_execution(
    world: &mut somnium_ecs::World,
    entity: somnium_ecs::Entity,
) {
    let _ = world.remove_component::<BehaviorExecution>(entity);
}

/// Register behavior intent with the engine component schema.
pub fn register(registry: &mut somnium_ecs::reflect::TypeRegistry) {
    editor::register_editor(registry);
    registry.register(somnium_ecs::component_schema! {
        BehaviorComponent as "somnium.Behavior", display "Behavior Tree", version 1,
        fields {
            enabled { group:"Behavior" },
            label { group:"Behavior",display_name:"Tree Name" },
            status { group:"Runtime",read_only:true,flags:somnium_ecs::reflect::FieldFlags::EDIT.union(somnium_ecs::reflect::FieldFlags::SCRIPT_READ) },
            last_error { group:"Runtime",read_only:true,flags:somnium_ecs::reflect::FieldFlags::EDIT.union(somnium_ecs::reflect::FieldFlags::SCRIPT_READ) },
            graph_json { group:"Advanced",advanced:true,read_only:true,display_name:"Authored Graph",doc:"MORROWIND-K behavior graph. Apply a Behavior graph from the shared graph editor." },
        }
    });
}
/// Validate before attachment, so an invalid graph never replaces live intent.
pub fn attach_behavior(
    world: &mut somnium_ecs::World,
    entity: somnium_ecs::Entity,
    json: String,
) -> Result<(), String> {
    let graph =
        somnium_ui::graph::serial::from_json(&json, &somnium_ui::graph::behavior::catalogue())
            .map_err(|e| format!("{e:?}"))?;
    somnium_ui::graph::behavior::compile(&graph)?;
    let label = world.get::<BehaviorComponent>(entity).map_or_else(
        || "Behavior Tree".into(),
        |component| component.label.clone(),
    );
    world
        .insert_component(
            entity,
            BehaviorComponent {
                enabled: true,
                graph_json: json,
                label,
                status: "Ready — press Play to run".into(),
                last_error: String::new(),
            },
        )
        .map_err(|e| format!("{e:?}"))?;
    world
        .remove_component::<BehaviorExecution>(entity)
        .map_err(|e| format!("{e:?}"))?;
    Ok(())
}
/// Play-mode default host executes the built-in blackboard/wait tasks and two
/// named native outcomes. Gameplay supplies its own TaskHost for native motion
/// or ScriptTaskHost for Luau; missing task bindings fail visibly in blackboard.
pub fn tick_behaviors(
    world: &mut somnium_ecs::World,
    dt: f32,
) -> Vec<(somnium_ecs::Entity, somnium_ai::behavior::Status)> {
    struct Host;
    impl somnium_ai::behavior::TaskHost for Host {
        fn tick(
            &mut self,
            script: bool,
            name: &str,
            board: &mut somnium_ai::behavior::Blackboard,
            _: f32,
        ) -> somnium_ai::behavior::Status {
            use somnium_ai::behavior::{Status, Value};
            if !script && name == "succeed" {
                return Status::Success;
            }
            if !script && name == "fail" {
                return Status::Failure;
            }
            board.insert(
                "task_error".into(),
                Value::Text(format!("Task {name} requires a gameplay host binding")),
            );
            Status::Failure
        }
    }
    tick_behaviors_with(world, dt, &mut Host)
}
/// Tick through a game-supplied native or budgeted scripting task host.
pub fn tick_behaviors_with(
    world: &mut somnium_ecs::World,
    dt: f32,
    host: &mut impl somnium_ai::behavior::TaskHost,
) -> Vec<(somnium_ecs::Entity, somnium_ai::behavior::Status)> {
    let entities: Vec<_> = world.entities().collect();
    entities
        .into_iter()
        .filter_map(|entity| {
            tick_behavior_with(world, entity, dt, host).map(|status| (entity, status))
        })
        .collect()
}
/// Tick one owner through its own task bindings and script snapshot. Use this
/// when different agents have different script attachment instances.
pub fn tick_behavior_with(
    world: &mut somnium_ecs::World,
    entity: somnium_ecs::Entity,
    dt: f32,
    host: &mut impl somnium_ai::behavior::TaskHost,
) -> Option<somnium_ai::behavior::Status> {
    tick_behavior_scoped(world, entity, |_, instance, reset| {
        if reset {
            instance.reset(host);
            somnium_ai::behavior::Status::Failure
        } else {
            instance.tick(dt, host)
        }
    })
}

// Temporarily take execution out of the component while callbacks borrow the
// world read-only. No ECS borrow or mutable world pointer crosses into script.
pub(crate) fn tick_behavior_scoped(
    world: &mut somnium_ecs::World,
    entity: somnium_ecs::Entity,
    mut invoke: impl FnMut(
        &somnium_ecs::World,
        &mut somnium_ai::behavior::BehaviorInstance,
        bool,
    ) -> somnium_ai::behavior::Status,
) -> Option<somnium_ai::behavior::Status> {
    use somnium_ai::behavior::{BehaviorInstance, Status};
    let authored = world.get::<BehaviorComponent>(entity)?;
    if !authored.enabled {
        let instance = world
            .get_mut::<BehaviorExecution>(entity)
            .and_then(|execution| execution.instance.take());
        if let Some(mut instance) = instance {
            invoke(world, &mut instance, true);
            if let Some(execution) = world.get_mut::<BehaviorExecution>(entity) {
                execution.instance = Some(instance);
            }
        }
        if let Some(authored) = world.get_mut::<BehaviorComponent>(entity) {
            authored.status = "Disabled".into();
            authored.last_error.clear();
        }
        return None;
    }
    if world
        .get::<BehaviorExecution>(entity)
        .is_none_or(|e| e.source != authored.graph_json)
    {
        let source = authored.graph_json.clone();
        let result = somnium_ui::graph::serial::from_json(
            &source,
            &somnium_ui::graph::behavior::catalogue(),
        )
        .map_err(|e| format!("{e:?}"))
        .and_then(|g| somnium_ui::graph::behavior::compile(&g));
        let (instance, error) = match result {
            Ok(tree) => (Some(BehaviorInstance::new(tree)), String::new()),
            Err(error) => {
                tracing::warn!(?entity,%error,"Invalid behavior graph");
                (None, error)
            }
        };
        // Cancel the previous running task before replacing its execution.
        let old = world
            .get_mut::<BehaviorExecution>(entity)
            .and_then(|execution| execution.instance.take());
        if let Some(mut old) = old {
            invoke(world, &mut old, true);
        }
        world
            .insert_component(
                entity,
                BehaviorExecution {
                    source,
                    instance,
                    error,
                },
            )
            .ok()?;
    }
    let mut instance = world.get_mut::<BehaviorExecution>(entity)?.instance.take();
    let status = instance
        .as_mut()
        .map_or(Status::Failure, |instance| invoke(world, instance, false));
    let execution = world.get_mut::<BehaviorExecution>(entity)?;
    execution.instance = instance;
    let error = execution
        .instance
        .as_ref()
        .and_then(|i| i.blackboard.get("task_error"))
        .and_then(|v| match v {
            somnium_ai::behavior::Value::Text(t) => Some(t.clone()),
            _ => None,
        })
        .unwrap_or_else(|| execution.error.clone());
    if let Some(component) = world.get_mut::<BehaviorComponent>(entity) {
        component.status = format!("{status:?}");
        component.last_error = error;
    }
    Some(status)
}
/// Reopen the selected entity's authored tree in the shared graph editor.
pub fn behavior_document(
    world: &somnium_ecs::World,
    entity: somnium_ecs::Entity,
) -> Result<String, String> {
    world
        .get::<BehaviorComponent>(entity)
        .map(|c| c.graph_json.clone())
        .ok_or_else(|| "Selected entity has no Behavior Tree; apply a graph first".into())
}
/// Inspect transient decision state without making it authored scene data.
pub fn behavior_blackboard(
    world: &somnium_ecs::World,
    entity: somnium_ecs::Entity,
) -> Option<&somnium_ai::behavior::Blackboard> {
    world
        .get::<BehaviorExecution>(entity)?
        .instance
        .as_ref()
        .map(|i| &i.blackboard)
}
/// Clear transient execution at Stop without altering authored scene data.
pub fn reset_behaviors(world: &mut somnium_ecs::World) {
    let entities: Vec<_> = world.entities().collect();
    for entity in entities {
        let _ = world.remove_component::<BehaviorExecution>(entity);
    }
}
