//! Designer-facing navigation volumes, agents, obstacles and sensor previews.
use super::NavigationCells;
use crate::{
    MeshComponent, MeshKind, Parent, TerrainComponent, Transform, WorldTransform,
    world_partition::CellCoord,
};
use glam::{Mat4, Vec3};
use somnium_ai::{
    navigation::{BakeInput, BakeSettings, Bounds, NavAgent, Neighbor, Triangle},
    perception::{Observer, StimulusMemory, occluded},
};
use somnium_ecs::{
    Component, Entity, World, component_schema,
    reflect::{FieldFlags, TypeRegistry},
};
use somnium_jobs::JobSystem;
use somnium_renderer::{SomniumRenderer, pass::light_gizmo::LineVertex};
use std::collections::BTreeMap;

/// Editable bake volume. Position it with the ordinary transform gizmo.
#[derive(Clone, Debug, PartialEq)]
pub struct NavigationProfile {
    /// Axis-aligned world-space bake volume dimensions, centered on Transform.
    pub bounds_size: Vec3,
    /// Width/depth of each independently cooked tile, in metres.
    pub tile_size: f32,
    /// Navigation raster resolution, in metres.
    pub voxel_size: f32,
    /// Required standing clearance.
    pub agent_height: f32,
    /// Horizontal obstacle clearance.
    pub agent_radius: f32,
    /// Maximum walkable step between spans.
    pub max_step: f32,
    /// Maximum walkable surface slope.
    pub max_slope_degrees: f32,
    /// Draw the volume, walkable contours and agent paths.
    pub show_mesh: bool,
    /// Latest cook/status diagnostic; not serialized.
    pub status: String,
    /// Installed convex polygon count; not serialized.
    pub polygon_count: u32,
    /// Outstanding tile jobs; not serialized.
    pub pending_cells: u32,
}
impl Default for NavigationProfile {
    fn default() -> Self {
        Self {
            bounds_size: Vec3::new(32.0, 16.0, 32.0),
            tile_size: 16.0,
            voxel_size: 0.5,
            agent_height: 1.8,
            agent_radius: 0.3,
            max_step: 0.4,
            max_slope_degrees: 45.0,
            show_mesh: true,
            status: "Choose Bake Navigation to build this volume".into(),
            polygon_count: 0,
            pending_cells: 0,
        }
    }
}
impl Component for NavigationProfile {}
impl NavigationProfile {
    fn settings(&self) -> BakeSettings {
        BakeSettings {
            voxel_size: self.voxel_size,
            agent_height: self.agent_height,
            agent_radius: self.agent_radius,
            max_step: self.max_step,
            max_slope_degrees: self.max_slope_degrees,
        }
    }
    fn bounds(&self, center: Vec3) -> Result<Bounds, String> {
        if !self.bounds_size.is_finite()
            || self.bounds_size.min_element() <= 0.0
            || self.bounds_size.max_element() > 4096.0
            || !self.tile_size.is_finite()
            || self.tile_size <= 0.0
            || !self.voxel_size.is_finite()
            || self.voxel_size <= 0.0
            || self.tile_size / self.voxel_size > 512.0
        {
            return Err("Use positive finite bounds/tile/voxel sizes; tiles support at most 512 voxels per edge".into());
        }
        Ok(Bounds {
            min: (center - self.bounds_size * 0.5).to_array(),
            max: (center + self.bounds_size * 0.5).to_array(),
        })
    }
}
/// An actor follows a designer-authored world-space destination while playing.
#[derive(Clone, Debug)]
pub struct NavigationAgent {
    /// Enable navigation-driven movement during Play.
    pub enabled: bool,
    /// Destination in world space.
    pub destination: Vec3,
    /// Maximum speed in metres per second.
    pub speed: f32,
    /// Separation radius; bake using at least this radius.
    pub radius: f32,
    /// Live movement diagnostic; not serialized.
    pub status: String,
    /// True when the destination has been reached.
    pub arrived: bool,
    /// Link currently awaiting traversal; unset otherwise. Runtime-only.
    pub pending_link: Entity,
    /// Expected traversal exit in world space. Runtime-only.
    pub link_exit: Vec3,
    /// One-shot acknowledgement, accepted only after reaching the link exit.
    pub acknowledge_link: bool,
}
impl Default for NavigationAgent {
    fn default() -> Self {
        Self {
            enabled: true,
            destination: Vec3::ZERO,
            speed: 3.0,
            radius: 0.3,
            status: "Bake a Navigation Profile before Play".into(),
            arrived: false,
            pending_link: Entity::DANGLING,
            link_exit: Vec3::ZERO,
            acknowledge_link: false,
        }
    }
}
impl Component for NavigationAgent {}
struct AgentExecution {
    agent: NavAgent,
    target: Vec3,
    speed: f32,
    radius: f32,
    pending: Option<PendingNavigationLink>,
}
impl Component for AgentExecution {}
/// The physical traversal an editor-owned agent is waiting for. Gameplay moves
/// the actor, then acknowledges this exact link after reaching `exit`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PendingNavigationLink {
    /// Authored Navigation Link entity, including its generation.
    pub link: Entity,
    /// World-space entry selected by the route.
    pub entry: Vec3,
    /// World-space exit selected by the route.
    pub exit: Vec3,
    /// Maximum accepted distance from the exit, in metres.
    pub exit_tolerance: f32,
}
/// Inspect the pending physical traversal without exposing transient agent state.
pub fn pending_navigation_link(world: &World, actor: Entity) -> Option<PendingNavigationLink> {
    world.get::<AgentExecution>(actor)?.pending
}
fn current_position(world: &World, entity: Entity) -> Result<Vec3, String> {
    let mut matrix = Mat4::IDENTITY;
    let mut cursor = entity;
    let mut visited = std::collections::HashSet::new();
    for _ in 0..128 {
        if !world.is_alive(cursor) || !visited.insert(cursor) {
            return Err("Invalid actor/link hierarchy".into());
        }
        matrix = world
            .get::<Transform>(cursor)
            .map_or(Mat4::IDENTITY, Transform::to_matrix)
            * matrix;
        let Some(parent) = world.get::<Parent>(cursor) else {
            let position = matrix.transform_point3(Vec3::ZERO);
            return position
                .is_finite()
                .then_some(position)
                .ok_or_else(|| "Actor/link transform is not finite".into());
        };
        cursor = parent.entity;
    }
    Err("Actor/link hierarchy exceeds 128 levels".into())
}
/// Resume an editor-owned route after gameplay or a designer moved the actor
/// to the expected link exit. Never teleports; rejects stale, changed, disabled
/// or mismatched links and positions farther than 0.1 m from that exit.
pub fn acknowledge_navigation_link(
    world: &mut World,
    actor: Entity,
    expected_link: Entity,
) -> Result<(), String> {
    let pending = pending_navigation_link(world, actor).ok_or("No link is awaiting traversal")?;
    if pending.link != expected_link {
        return Err("The pending link changed; inspect the new traversal first".into());
    }
    let link = world
        .get::<NavigationLink>(expected_link)
        .ok_or("Navigation link was removed")?;
    if !link.enabled {
        return Err("Navigation link is disabled".into());
    }
    let start = current_position(world, expected_link)?;
    let near =
        |a: Vec3, b: Vec3| a.is_finite() && b.is_finite() && a.distance_squared(b) <= 0.000001;
    if !(near(start, pending.entry) && near(link.destination, pending.exit))
        && !(link.bidirectional
            && near(start, pending.exit)
            && near(link.destination, pending.entry))
    {
        return Err("Navigation link endpoints or direction changed; wait for replanning".into());
    }
    let controls = world
        .get::<NavigationAgent>(actor)
        .ok_or("Actor no longer has Navigation Agent")?;
    let execution = world
        .get::<AgentExecution>(actor)
        .ok_or("Navigation execution was reset")?;
    if controls.destination != execution.target
        || controls.speed != execution.speed
        || controls.radius != execution.radius
    {
        return Err("Agent controls changed; wait for replanning".into());
    }
    let distance = current_position(world, actor)?.distance(pending.exit);
    if distance > pending.exit_tolerance {
        return Err(format!(
            "Move to Link Exit first ({distance:.2} m away; allowed {:.2} m)",
            pending.exit_tolerance
        ));
    }
    let execution = world
        .get_mut::<AgentExecution>(actor)
        .ok_or("Navigation execution was reset")?;
    if !execution.agent.complete_link(entity_key(expected_link)) {
        return Err("The route no longer waits for this link".into());
    }
    execution.pending = None;
    if let Some(agent) = world.get_mut::<NavigationAgent>(actor) {
        agent.pending_link = Entity::DANGLING;
        agent.link_exit = Vec3::ZERO;
        agent.acknowledge_link = false;
        agent.status = "Link traversal acknowledged; continuing route".into();
    }
    Ok(())
}
struct NavigationRuntimeOwner(u64);
impl Component for NavigationRuntimeOwner {}
static NEXT_NAVIGATION_OWNER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Reset transient movement and sensor memory at Stop/load without discarding
/// authored controls or the installed navigation bake.
pub fn reset_editor_runtime(world: &mut World) {
    let entities: Vec<_> = world.entities().collect();
    for entity in entities {
        let _ = world.remove_component::<AgentExecution>(entity);
        let _ = world.remove_component::<PerceptionExecution>(entity);
        if let Some(agent) = world.get_mut::<NavigationAgent>(entity) {
            agent.arrived = false;
            agent.pending_link = Entity::DANGLING;
            agent.link_exit = Vec3::ZERO;
            agent.acknowledge_link = false;
            agent.status = "Ready — Play to follow the destination".into();
        }
        if let Some(sensor) = world.get_mut::<PerceptionComponent>(entity) {
            sensor.confidence = 0.0;
            sensor.status = "Sensor reset".into();
        }
    }
}
/// A movable carving volume. Rebuilds only intersecting navigation cells.
#[derive(Clone, Copy, Debug)]
pub struct NavigationObstacle {
    /// Whether the obstacle blocks navigation.
    pub enabled: bool,
    /// Local-space box dimensions; the entity transform applies normally.
    pub size: Vec3,
}
impl Default for NavigationObstacle {
    fn default() -> Self {
        Self {
            enabled: true,
            size: Vec3::splat(2.0),
        }
    }
}
impl Component for NavigationObstacle {}
/// Inspect an actor's sight/hearing against an authored target entity.
#[derive(Clone, Debug)]
pub struct PerceptionComponent {
    /// Enable sensor sampling and memory decay.
    pub enabled: bool,
    /// Entity observed; use the Details entity picker.
    pub target: Entity,
    /// Eye offset above the observer's origin.
    pub eye_height: f32,
    /// Sight distance in metres.
    pub sight_range: f32,
    /// Half-angle of the sight cone in degrees.
    pub half_angle_degrees: f32,
    /// Hearing distance in metres.
    pub hearing_range: f32,
    /// Preview a continuous sound at the target. Zero disables hearing preview.
    pub target_loudness: f32,
    /// Stimulus retention duration in seconds.
    pub memory_seconds: f32,
    /// Latest sensor diagnostic; not serialized.
    pub status: String,
    /// Decayed strongest stimulus confidence; not serialized.
    pub confidence: f32,
}
impl Default for PerceptionComponent {
    fn default() -> Self {
        Self {
            enabled: true,
            target: Entity::DANGLING,
            eye_height: 1.5,
            sight_range: 20.0,
            half_angle_degrees: 60.0,
            hearing_range: 12.0,
            target_loudness: 0.0,
            memory_seconds: 5.0,
            status: "Choose a target entity".into(),
            confidence: 0.0,
        }
    }
}
impl Component for PerceptionComponent {}
struct PerceptionExecution {
    memory: StimulusMemory,
    lifetime: f32,
}
impl Component for PerceptionExecution {}

pub(super) fn register_editor(registry: &mut TypeRegistry) {
    let runtime = FieldFlags::EDIT.union(FieldFlags::SCRIPT_READ);
    registry.register(component_schema! {
        NavigationLink as "somnium.NavigationLink",display "Navigation Link",version 1,
        fields {
            enabled {group:"Traversal"},destination {group:"Traversal",unit:"m",step:0.5},bidirectional {group:"Traversal"},
            attachment_radius {group:"Traversal",min:0.01,max:10.0,step:0.1,unit:"m"},extra_cost {group:"Traversal",min:0.0,max:1000.0,step:0.5,unit:"m"},
        }
    });
    registry.register(component_schema! {
        NavigationProfile as "somnium.NavigationProfile",display "Navigation Profile",version 1,
        fields {
            bounds_size {group:"Bake Volume",display_name:"Bounds Size",min:0.1,step:1.0,unit:"m"},
            tile_size {group:"Bake Volume",min:1.0,max:256.0,step:1.0,unit:"m"},
            voxel_size {group:"Quality",min:0.1,max:4.0,step:0.1,unit:"m",doc:"Smaller voxels preserve narrow passages and cost more to bake."},
            agent_height {group:"Agent",min:0.1,max:20.0,step:0.1,unit:"m"},
            agent_radius {group:"Agent",min:0.0,max:10.0,step:0.05,unit:"m"},
            max_step {group:"Agent",min:0.0,max:5.0,step:0.05,unit:"m"},
            max_slope_degrees {group:"Agent",min:0.0,max:89.0,step:1.0,unit:"degrees"},
            show_mesh {group:"Preview",display_name:"Show Navigation"},
            status {group:"Bake Status",read_only:true,flags:runtime},
            polygon_count {group:"Bake Status",read_only:true,flags:runtime},
            pending_cells {group:"Bake Status",read_only:true,flags:runtime},
        }
    });
    registry.register(component_schema! {
        NavigationAgent as "somnium.NavigationAgent",display "Navigation Agent",version 1,
        fields {
            enabled {group:"Agent"},destination {group:"Movement",unit:"m",step:0.5},
            speed {group:"Movement",min:0.1,max:100.0,step:0.1,unit:"m/s"},
            radius {group:"Movement",min:0.05,max:10.0,step:0.05,unit:"m"},
            status {group:"Runtime",read_only:true,flags:runtime},arrived {group:"Runtime",read_only:true,flags:runtime},
            pending_link {group:"Link Traversal",read_only:true,flags:runtime},
            link_exit {group:"Link Traversal",read_only:true,flags:runtime,unit:"m"},
            acknowledge_link {group:"Link Traversal",flags:runtime.union(FieldFlags::SCRIPT_WRITE),doc:"Move the actor to Link Exit first, then enable this to resume. Accepted within 0.1 m; resets after every attempt. Gameplay owns jump/ladder motion."},
        }
    });
    registry.register(component_schema! {
        NavigationObstacle as "somnium.NavigationObstacle",display "Navigation Obstacle",version 1,
        fields {enabled {group:"Carving"},size {group:"Carving",min:0.01,step:0.1,unit:"m",doc:"Move this entity to rebake only affected cells."}}
    });
    registry.register(component_schema! {
        PerceptionComponent as "somnium.Perception",display "Perception Sensor",version 1,
        fields {
            enabled {group:"Sensor"},target {group:"Sensor"},eye_height {group:"Sight",min:0.0,max:10.0,step:0.1,unit:"m"},
            sight_range {group:"Sight",min:0.1,max:1000.0,step:1.0,unit:"m"},half_angle_degrees {group:"Sight",min:1.0,max:180.0,step:1.0,unit:"degrees"},
            hearing_range {group:"Hearing",min:0.1,max:1000.0,step:1.0,unit:"m"},target_loudness {group:"Hearing",min:0.0,max:10.0,step:0.1,display_name:"Target Sound Preview"},
            memory_seconds {group:"Memory",min:0.1,max:120.0,step:0.5,unit:"s"},
            status {group:"Runtime",read_only:true,flags:runtime},confidence {group:"Runtime",read_only:true,flags:runtime},
        }
    });
}

/// Editor-owned live navigation. It uses the engine's existing job and gizmo
/// paths; authored controls live on ordinary reflected ECS components.
#[derive(Default)]
pub struct NavigationEditor {
    /// Cell tiles available to gameplay queries.
    pub cells: NavigationCells,
    owner: Option<Entity>,
    bounds: Option<Bounds>,
    geometry: Vec<Triangle>,
    obstacles: BTreeMap<u64, Bounds>,
    clock: f64,
    skipped_meshes: usize,
    links: BTreeMap<u64, somnium_ai::navigation::OffMeshLink>,
    agent_paths: Vec<[Vec3; 2]>,
    ownership: u64,
}
impl NavigationEditor {
    fn owns_runtime(&self, world: &World) -> bool {
        let active = world
            .entities()
            .filter_map(|e| world.get::<NavigationRuntimeOwner>(e).map(|owner| owner.0))
            .max();
        active.map_or(self.ownership == 0, |owner| owner == self.ownership)
    }
    /// Inspect the selected volume and expose its overlay before the first bake.
    pub fn inspect(&mut self, world: &mut World, selected: Entity) -> Result<String, String> {
        let profile = world
            .get::<NavigationProfile>(selected)
            .ok_or("Select a Navigation Profile")?;
        let bounds = profile.bounds(world_matrix(world, selected).transform_point3(Vec3::ZERO))?;
        if self.owner.is_some_and(|owner| owner != selected)
            && self.cells.world.cells().next().is_some()
        {
            return Err(
                "Another profile owns the current bake; choose Bake Navigation to switch".into(),
            );
        }
        let status = profile.status.clone();
        self.owner = Some(selected);
        self.bounds = Some(bounds);
        if let Some(profile) = world.get_mut::<NavigationProfile>(selected) {
            profile.show_mesh = true;
        }
        Ok(status)
    }
    /// Gather real scene geometry inside the selected profile and queue tile jobs.
    pub fn bake(
        &mut self,
        world: &mut World,
        renderer: &SomniumRenderer,
        jobs: &mut JobSystem,
        selected: Entity,
    ) -> Result<String, String> {
        let profile = world
            .get::<NavigationProfile>(selected)
            .ok_or("Select a Navigation Profile to bake; create one from the Create menu")?
            .clone();
        let bounds = profile.bounds(world_matrix(world, selected).transform_point3(Vec3::ZERO))?;
        let mut padded = bounds;
        for axis in [0, 2] {
            padded.min[axis] -= profile.agent_radius + profile.voxel_size;
            padded.max[axis] += profile.agent_radius + profile.voxel_size;
        }
        let (triangles, skipped) = collect_geometry(world, Some(renderer), padded)?;
        if triangles.is_empty() {
            return Err(
                "No supported mesh, blockout or terrain geometry intersects the navigation volume"
                    .into(),
            );
        }
        self.bake_geometry(world, jobs, selected, triangles)?;
        self.skipped_meshes = skipped;
        Ok(format!(
            "Navigation bake queued: {} tiles, {} triangles{}",
            self.cells.pending_count(),
            self.geometry.len(),
            if skipped > 0 {
                format!(", {skipped} imported GPU-only meshes skipped")
            } else {
                String::new()
            }
        ))
    }
    /// Headless equivalent used by editor acceptance tests and import tools.
    pub fn bake_geometry(
        &mut self,
        world: &mut World,
        jobs: &mut JobSystem,
        selected: Entity,
        triangles: Vec<Triangle>,
    ) -> Result<(), String> {
        let profile = world
            .get::<NavigationProfile>(selected)
            .ok_or("Selected entity has no Navigation Profile")?
            .clone();
        let bounds = profile.bounds(world_matrix(world, selected).transform_point3(Vec3::ZERO))?;
        let s = profile.tile_size;
        let x0 = (bounds.min[0] / s).floor() as i64;
        let x1 = (bounds.max[0] / s).ceil() as i64;
        let z0 = (bounds.min[2] / s).floor() as i64;
        let z1 = (bounds.max[2] / s).ceil() as i64;
        if (x1 - x0).saturating_mul(z1 - z0) > 64 {
            return Err(
                "Bake volume exceeds 64 tiles; reduce Bounds Size or increase Tile Size".into(),
            );
        }
        self.clear(world);
        self.owner = Some(selected);
        self.ownership = NEXT_NAVIGATION_OWNER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let _ = world.insert_component(selected, NavigationRuntimeOwner(self.ownership));
        self.bounds = Some(bounds);
        self.geometry = triangles;
        for z in z0..z1 {
            for x in x0..x1 {
                let tile = Bounds {
                    min: [
                        (x as f32 * s).max(bounds.min[0]),
                        bounds.min[1],
                        (z as f32 * s).max(bounds.min[2]),
                    ],
                    max: [
                        ((x + 1) as f32 * s).min(bounds.max[0]),
                        bounds.max[1],
                        ((z + 1) as f32 * s).min(bounds.max[2]),
                    ],
                };
                let mut padded = tile;
                for axis in [0, 2] {
                    padded.min[axis] -= profile.agent_radius + profile.voxel_size;
                    padded.max[axis] += profile.agent_radius + profile.voxel_size;
                }
                let triangles = self
                    .geometry
                    .iter()
                    .copied()
                    .filter(|t| triangle_bounds(*t).intersects(padded))
                    .collect();
                self.cells
                    .request(
                        jobs,
                        CellCoord { x, y: 0, z },
                        BakeInput {
                            cell: [x, 0, z],
                            bounds: tile,
                            triangles,
                            obstacles: Vec::new(),
                            settings: profile.settings(),
                        },
                    )
                    .map_err(|e| format!("{e:?}"))?;
            }
        }
        if let Some(p) = world.get_mut::<NavigationProfile>(selected) {
            p.status = "Baking navigation…".into();
            p.pending_cells = self.cells.pending_count() as u32;
        }
        Ok(())
    }
    /// Cancel pending work and clear the preview without deleting authored controls.
    pub fn clear(&mut self, world: &mut World) -> String {
        let cells: Vec<_> = self.cells.inputs.keys().copied().collect();
        for cell in cells {
            self.cells.unload(cell);
        }
        self.cells = NavigationCells::default();
        self.obstacles.clear();
        self.links.clear();
        self.agent_paths.clear();
        self.geometry.clear();
        self.bounds = None;
        if let Some(owner) = self.owner.take() {
            if world
                .get::<NavigationRuntimeOwner>(owner)
                .is_some_and(|owner| owner.0 == self.ownership)
            {
                let _ = world.remove_component::<NavigationRuntimeOwner>(owner);
            }
            if let Some(p) = world.get_mut::<NavigationProfile>(owner) {
                p.status = "Navigation cleared — Bake Navigation to rebuild".into();
                p.polygon_count = 0;
                p.pending_cells = 0;
            }
        }
        self.ownership = 0;
        reset_editor_runtime(world);
        "Navigation cleared".into()
    }
    /// Poll jobs, update obstacle carving, simulate agents during Play and draw
    /// the same retained gizmo lines used by the rest of the editor.
    pub fn update(
        &mut self,
        world: &mut World,
        renderer: &mut SomniumRenderer,
        jobs: &mut JobSystem,
        dt: f32,
        playing: bool,
    ) {
        self.update_runtime(world, jobs, dt, playing);
        let visible = self
            .owner
            .and_then(|e| world.get::<NavigationProfile>(e))
            .is_some_and(|p| p.show_mesh)
            && self.owns_runtime(world);
        if visible {
            renderer.submit_gizmo_lines(self.preview_lines());
        }
    }
    /// The update logic without a graphics device, for tools and acceptance tests.
    pub fn update_runtime(
        &mut self,
        world: &mut World,
        jobs: &mut JobSystem,
        dt: f32,
        playing: bool,
    ) {
        if self.owner.is_none() {
            let first_profile = world
                .entities()
                .find(|e| world.get::<NavigationProfile>(*e).is_some());
            if let Some(profile) = first_profile {
                let _ = self.inspect(world, profile);
            }
        }
        if let Some(owner) = self.owner {
            if let Some(profile) = world.get::<NavigationProfile>(owner) {
                if let Ok(bounds) =
                    profile.bounds(world_matrix(world, owner).transform_point3(Vec3::ZERO))
                {
                    self.bounds = Some(bounds);
                }
            }
        }
        if dt.is_finite() && dt > 0.0 {
            self.clock += f64::from(dt);
        }
        if self
            .owner
            .is_some_and(|e| world.get::<NavigationProfile>(e).is_none())
        {
            self.clear(world);
        }
        // The editor and a GameApp can each own a manager. The last explicitly
        // baked profile owns simulation; inspecting an empty manager cannot
        // invalidate its route or replace its diagnostics.
        if !self.owns_runtime(world) {
            return;
        }
        let desired: BTreeMap<_, _> = world
            .entities()
            .filter_map(|e| {
                let obstacle = world.get::<NavigationObstacle>(e)?;
                (obstacle.enabled && obstacle.size.is_finite() && obstacle.size.min_element() > 0.0)
                    .then(|| {
                        (
                            entity_key(e),
                            transformed_bounds(
                                world_matrix(world, e),
                                -obstacle.size * 0.5,
                                obstacle.size * 0.5,
                            ),
                        )
                    })
            })
            .collect();
        let changed: Vec<_> = self
            .obstacles
            .keys()
            .chain(desired.keys())
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .filter(|id| self.obstacles.get(id) != desired.get(id))
            .collect();
        for id in changed {
            match self.cells.set_obstacle(jobs, id, desired.get(&id).copied()) {
                Ok(_) => {
                    if let Some(b) = desired.get(&id) {
                        self.obstacles.insert(id, *b);
                    } else {
                        self.obstacles.remove(&id);
                    }
                }
                Err(error) => {
                    if let Some(profile) = self
                        .owner
                        .and_then(|e| world.get_mut::<NavigationProfile>(e))
                    {
                        profile.status = format!("Obstacle rebake failed: {error:?}");
                    }
                }
            }
        }
        let wanted_links: BTreeMap<_, _> = world
            .entities()
            .filter_map(|entity| {
                let link = world.get::<NavigationLink>(entity)?;
                link.enabled.then(|| {
                    (
                        entity_key(entity),
                        somnium_ai::navigation::OffMeshLink {
                            id: entity_key(entity),
                            start: world_matrix(world, entity).transform_point3(Vec3::ZERO),
                            end: link.destination,
                            bidirectional: link.bidirectional,
                            radius: link.attachment_radius,
                            cost: link.extra_cost,
                        },
                    )
                })
            })
            .collect();
        let removed: Vec<_> = self
            .links
            .keys()
            .filter(|id| !wanted_links.contains_key(id))
            .copied()
            .collect();
        for id in removed {
            self.cells.world.remove_link(id);
            self.links.remove(&id);
        }
        for (id, link) in wanted_links {
            if self.links.get(&id) != Some(&link) && self.cells.world.set_link(link).is_ok() {
                self.links.insert(id, link);
            }
        }
        let results = self.cells.poll();
        if let Some(profile) = self
            .owner
            .and_then(|e| world.get_mut::<NavigationProfile>(e))
        {
            profile.pending_cells = self.cells.pending_count() as u32;
            profile.polygon_count = self
                .cells
                .world
                .cells()
                .filter_map(|c| self.cells.world.tile(c))
                .map(|t| t.polygons().len() as u32)
                .sum();
            if let Some((_, Err(error))) = results.iter().find(|(_, r)| r.is_err()) {
                profile.status = format!("Bake failed: {error:?}");
            } else if !results.is_empty() {
                profile.status = if profile.pending_cells > 0 {
                    "Baking navigation…".into()
                } else if profile.polygon_count == 0 {
                    "No walkable floor — check volume, slope and agent clearance".into()
                } else {
                    format!(
                        "Ready — {} polygons, {} tiles{}",
                        profile.polygon_count,
                        self.cells.world.cells().count(),
                        if self.skipped_meshes > 0 {
                            format!(", {} unsupported meshes", self.skipped_meshes)
                        } else {
                            String::new()
                        }
                    )
                };
            }
        }
        if playing && self.cells.world.cells().next().is_some() {
            self.step_agents(world, dt);
        }
        self.agent_paths = world
            .entities()
            .filter_map(|e| world.get::<AgentExecution>(e))
            .filter_map(|e| e.agent.path())
            .flat_map(|p| p.points.windows(2).map(|pair| [pair[0], pair[1]]))
            .collect();
        self.update_perception(world);
    }
    fn step_agents(&mut self, world: &mut World, dt: f32) {
        let entities: Vec<_> = world
            .entities()
            .filter(|e| world.get::<NavigationAgent>(*e).is_some())
            .collect();
        let positions: Vec<_> = entities
            .iter()
            .filter_map(|&e| {
                Some((
                    e,
                    world_matrix(world, e).transform_point3(Vec3::ZERO),
                    world.get::<NavigationAgent>(e)?.radius,
                ))
            })
            .collect();
        for entity in entities {
            let controls = world.get::<NavigationAgent>(entity).unwrap().clone();
            if !controls.enabled {
                continue;
            }
            let position = world_matrix(world, entity).transform_point3(Vec3::ZERO);
            let changed = world.get::<AgentExecution>(entity).is_none_or(|e| {
                e.target != controls.destination
                    || e.speed != controls.speed
                    || e.radius != controls.radius
            });
            if changed {
                let Ok(mut agent) = NavAgent::new(controls.speed, controls.radius) else {
                    continue;
                };
                agent.set_destination(&self.cells.world, position, controls.destination);
                let _ = world.insert_component(
                    entity,
                    AgentExecution {
                        agent,
                        target: controls.destination,
                        speed: controls.speed,
                        radius: controls.radius,
                        pending: None,
                    },
                );
            }
            let mut acknowledgement_error = None;
            if controls.acknowledge_link {
                if let Some(agent) = world.get_mut::<NavigationAgent>(entity) {
                    agent.acknowledge_link = false;
                }
                let pending = pending_navigation_link(world, entity);
                let result = pending
                    .ok_or_else(|| "No link is awaiting traversal".to_owned())
                    .and_then(|pending| acknowledge_navigation_link(world, entity, pending.link));
                acknowledgement_error = result.err();
            }
            let neighbors: Vec<_> = positions
                .iter()
                .filter(|(e, _, _)| *e != entity)
                .map(|(_, p, r)| Neighbor {
                    position: *p,
                    velocity: Vec3::ZERO,
                    radius: *r,
                })
                .collect();
            let movement = world
                .get_mut::<AgentExecution>(entity)
                .unwrap()
                .agent
                .update(&self.cells.world, position, dt, &neighbors);
            let pending = movement.off_mesh.and_then(|id| {
                if let Some(pending) = pending_navigation_link(world, entity)
                    .filter(|pending| entity_key(pending.link) == id)
                {
                    return Some(pending);
                }
                let link = self.links.get(&id)?;
                let link_entity = world
                    .entities()
                    .find(|candidate| entity_key(*candidate) == id)?;
                let (entry, exit) = if position.distance_squared(link.start)
                    <= position.distance_squared(link.end)
                {
                    (link.start, link.end)
                } else {
                    (link.end, link.start)
                };
                Some(PendingNavigationLink {
                    link: link_entity,
                    entry,
                    exit,
                    exit_tolerance: 0.1,
                })
            });
            if let Some(execution) = world.get_mut::<AgentExecution>(entity) {
                execution.pending = pending;
            }
            let mut local_delta = movement.velocity * dt;
            if let Some(parent) = world.get::<Parent>(entity) {
                let inverse = world_matrix(world, parent.entity).inverse();
                if inverse.is_finite() {
                    local_delta = inverse.transform_vector3(local_delta);
                }
            }
            if let Some(transform) = world.get_mut::<Transform>(entity) {
                transform.translation += local_delta;
            }
            if let Some(c) = world.get_mut::<NavigationAgent>(entity) {
                c.arrived = movement.arrived;
                c.pending_link = pending.map_or(Entity::DANGLING, |pending| pending.link);
                c.link_exit = pending.map_or(Vec3::ZERO, |pending| pending.exit);
                c.status = if let Some(id) = movement.off_mesh {
                    format!("Waiting at link {id}: move to Link Exit, then acknowledge traversal")
                } else if movement.arrived {
                    "Arrived".into()
                } else if movement.velocity.length_squared() > 0.0 {
                    "Following path".into()
                } else {
                    "No path — check destination, bake and obstacles".into()
                };
                if let Some(error) = acknowledgement_error {
                    c.status = format!("Link acknowledgement refused: {error}");
                }
            }
        }
    }
    fn update_perception(&self, world: &mut World) {
        let entities: Vec<_> = world
            .entities()
            .filter(|e| world.get::<PerceptionComponent>(*e).is_some())
            .collect();
        for entity in entities {
            let controls = world.get::<PerceptionComponent>(entity).unwrap().clone();
            if !controls.enabled {
                continue;
            }
            if !world.is_alive(controls.target) {
                if let Some(c) = world.get_mut::<PerceptionComponent>(entity) {
                    c.status = "Choose a live target entity".into();
                    c.confidence = 0.0;
                }
                continue;
            }
            let observer_transform = world_matrix(world, entity);
            let observer = Observer {
                position: observer_transform.transform_point3(Vec3::Y * controls.eye_height),
                forward: observer_transform.transform_vector3(Vec3::NEG_Z),
                sight_range: controls.sight_range,
                half_angle_degrees: controls.half_angle_degrees,
                hearing_range: controls.hearing_range,
            };
            let target = world_matrix(world, controls.target)
                .transform_point3(Vec3::Y * controls.eye_height);
            if world
                .get::<PerceptionExecution>(entity)
                .is_none_or(|e| e.lifetime != controls.memory_seconds)
            {
                let Ok(memory) = StimulusMemory::new(64, f64::from(controls.memory_seconds)) else {
                    continue;
                };
                let _ = world.insert_component(
                    entity,
                    PerceptionExecution {
                        memory,
                        lifetime: controls.memory_seconds,
                    },
                );
            }
            let sight = observer.see(entity_key(controls.target), target, self.clock, |a, b| {
                occluded(&self.geometry, a, b)
                    || self.obstacles.iter().any(|(id, bounds)| {
                        *id != entity_key(entity)
                            && *id != entity_key(controls.target)
                            && segment_hits_bounds(a, b, *bounds)
                    })
            });
            let hearing = observer.hear(
                entity_key(controls.target),
                target,
                controls.target_loudness,
                self.clock,
            );
            let memory = &mut world.get_mut::<PerceptionExecution>(entity).unwrap().memory;
            if let Some(s) = sight {
                memory.remember(s);
            }
            if let Some(s) = hearing {
                memory.remember(s);
            }
            memory.expire(self.clock);
            let strongest = memory.strongest(self.clock);
            if let Some(c) = world.get_mut::<PerceptionComponent>(entity) {
                c.confidence = strongest.map_or(0.0, |s| s.strength);
                c.status = if sight.is_some() {
                    if self.geometry.is_empty() {
                        "Visible (bake navigation geometry to test occlusion)".into()
                    } else {
                        "Target visible".into()
                    }
                } else if hearing.is_some() {
                    "Target heard".into()
                } else if strongest.is_some() {
                    "Remembering last stimulus".into()
                } else {
                    "No stimulus".into()
                };
            }
        }
    }
    /// Green walkable contours and amber bake bounds; bounded to avoid huge previews.
    pub fn preview_lines(&self) -> Vec<LineVertex> {
        let mut lines = Vec::new();
        for cell in self.cells.world.cells() {
            if let Some(tile) = self.cells.world.tile(cell) {
                for edge in tile.contours().iter().take(32_768) {
                    for &point in edge {
                        lines.push(LineVertex {
                            position: (Vec3::from(point) + Vec3::Y * 0.03).to_array(),
                            color: [0.15, 0.95, 0.55],
                        });
                    }
                }
            }
        }
        for edge in &self.agent_paths {
            for point in edge {
                lines.push(LineVertex {
                    position: (*point + Vec3::Y * 0.05).to_array(),
                    color: [0.3, 0.6, 1.0],
                });
            }
        }
        for link in self.links.values() {
            for point in [link.start, link.end] {
                lines.push(LineVertex {
                    position: (point + Vec3::Y * 0.05).to_array(),
                    color: [0.95, 0.3, 0.95],
                });
            }
        }
        if let Some(bounds) = self.bounds {
            let a = Vec3::from(bounds.min);
            let b = Vec3::from(bounds.max);
            let corners = corners(a, b);
            for [u, v] in [
                [0, 1],
                [0, 2],
                [1, 3],
                [2, 3],
                [4, 5],
                [4, 6],
                [5, 7],
                [6, 7],
                [0, 4],
                [1, 5],
                [2, 6],
                [3, 7],
            ] {
                for point in [corners[u], corners[v]] {
                    lines.push(LineVertex {
                        position: point.to_array(),
                        color: [1.0, 0.65, 0.2],
                    });
                }
            }
        }
        lines
    }
}
fn entity_key(e: Entity) -> u64 {
    (u64::from(e.generation()) << 32) | u64::from(e.index())
}
fn world_matrix(world: &World, entity: Entity) -> Mat4 {
    if world.get::<Parent>(entity).is_some() {
        if let Some(transform) = world.get::<WorldTransform>(entity) {
            return transform.0;
        }
    }
    world
        .get::<Transform>(entity)
        .map_or(Mat4::IDENTITY, Transform::to_matrix)
}
fn corners(a: Vec3, b: Vec3) -> [Vec3; 8] {
    std::array::from_fn(|i| {
        Vec3::new(
            if i & 1 == 0 { a.x } else { b.x },
            if i & 2 == 0 { a.y } else { b.y },
            if i & 4 == 0 { a.z } else { b.z },
        )
    })
}
fn transformed_bounds(matrix: Mat4, a: Vec3, b: Vec3) -> Bounds {
    let points = corners(a, b).map(|p| matrix.transform_point3(p));
    Bounds {
        min: points
            .iter()
            .copied()
            .fold(Vec3::splat(f32::INFINITY), Vec3::min)
            .to_array(),
        max: points
            .iter()
            .copied()
            .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max)
            .to_array(),
    }
}
fn triangle_bounds(t: Triangle) -> Bounds {
    let [a, b, c] = t.map(Vec3::from);
    Bounds {
        min: a.min(b).min(c).to_array(),
        max: a.max(b).max(c).to_array(),
    }
}
/// Snapshot supported authored scene geometry. Imported meshes with GPU-only
/// storage are counted so the editor can report their omission explicitly.
pub fn collect_geometry(
    world: &World,
    renderer: Option<&SomniumRenderer>,
    bounds: Bounds,
) -> Result<(Vec<Triangle>, usize), String> {
    let mut triangles = Vec::new();
    let mut skipped = 0;
    for entity in world.entities() {
        if world.get::<NavigationObstacle>(entity).is_some()
            || world.get::<NavigationAgent>(entity).is_some()
        {
            continue;
        }
        let matrix = world_matrix(world, entity);
        if !matrix.is_finite() {
            continue;
        }
        if let Some(component) = world.get::<TerrainComponent>(entity) {
            if let Some(terrain) = renderer.and_then(|r| r.terrain(component.terrain_id)) {
                let nx = terrain.desc.total_vertices_x() as usize;
                let nz = terrain.desc.total_vertices_z() as usize;
                let size = terrain.desc.cell_size;
                let inverse = matrix.inverse();
                if !inverse.is_finite() || nx < 2 || nz < 2 || !size.is_finite() || size <= 0.0 {
                    continue;
                }
                let local = transformed_bounds(inverse, bounds.min.into(), bounds.max.into());
                let x0 = (local.min[0] / size).floor().max(0.0) as usize;
                let x1 = (local.max[0] / size).ceil().max(0.0).min((nx - 1) as f32) as usize;
                let z0 = (local.min[2] / size).floor().max(0.0) as usize;
                let z1 = (local.max[2] / size).ceil().max(0.0).min((nz - 1) as f32) as usize;
                if (x1.saturating_sub(x0)).saturating_mul(z1.saturating_sub(z0)) > 100_000 {
                    return Err("Terrain bake selection exceeds 100000 source quads; shrink the Navigation Profile".into());
                }
                let point = |x: usize, z: usize| -> Result<[f32; 3], String> {
                    let height = *terrain
                        .heightmap
                        .get(z * nx + x)
                        .ok_or("Terrain heightmap dimensions disagree")?;
                    Ok(matrix
                        .transform_point3(Vec3::new(
                            x as f32 * size,
                            height * terrain.desc.height_scale,
                            z as f32 * size,
                        ))
                        .to_array())
                };
                for z in z0..z1 {
                    for x in x0..x1 {
                        let a = point(x, z)?;
                        let b = point(x + 1, z)?;
                        let c = point(x + 1, z + 1)?;
                        let d = point(x, z + 1)?;
                        triangles.extend([[a, b, c], [a, c, d]]);
                    }
                }
            } else {
                skipped += 1;
            }
            continue;
        }
        let mesh = if let Some(blockout) = world.get::<crate::blockout::BlockoutComponent>(entity) {
            Some(blockout.mesh()?)
        } else if let Some(kind) = world.get::<MeshKind>(entity) {
            let (mut vertices, indices) = match kind {
                MeshKind::Cube => somnium_asset::generate_cube(1.0),
                MeshKind::Plane => somnium_asset::generate_plane(1.0, 1),
                MeshKind::Sphere => somnium_asset::generate_sphere(0.5, 32, 16),
                MeshKind::Cylinder => somnium_asset::generate_cylinder(0.5, 1.0, 32),
            };
            if let Some((a, b)) = world
                .get::<MeshComponent>(entity)
                .and_then(|m| renderer.and_then(|r| r.geometry.mesh_aabb(m.vertex_offset)))
            {
                let a = Vec3::from(a);
                let b = Vec3::from(b);
                let extent = b - a;
                let center = (a + b) * 0.5;
                for v in &mut vertices {
                    v.position = (Vec3::from(v.position) * extent + center).to_array();
                }
            }
            Some(somnium_asset::LoadedMesh {
                vertices,
                indices,
                skin: None,
            })
        } else {
            if world.get::<MeshComponent>(entity).is_some() {
                skipped += 1;
            }
            None
        };
        if let Some(mesh) = mesh {
            for indices in mesh.indices.chunks_exact(3) {
                let mut t = [[0.0; 3]; 3];
                for i in 0..3 {
                    let vertex = mesh
                        .vertices
                        .get(indices[i] as usize)
                        .ok_or("Invalid source mesh index")?;
                    t[i] = matrix.transform_point3(vertex.position.into()).to_array();
                }
                if triangle_bounds(t).intersects(bounds) {
                    triangles.push(t);
                }
            }
        }
        if triangles.len() > 200_000 {
            return Err(
                "Navigation geometry exceeds 200000 triangles; shrink the bake volume".into(),
            );
        }
    }
    Ok((triangles, skipped))
}

/// An authored ladder/jump connection. Transform is the start; Destination is
/// the endpoint. Gameplay owns traversal animation and physical relocation.
#[derive(Clone, Copy, Debug)]
pub struct NavigationLink {
    /// Whether queries may choose this link.
    pub enabled: bool,
    /// World-space endpoint of the connection.
    pub destination: Vec3,
    /// Permit traversal in both directions.
    pub bidirectional: bool,
    /// Maximum distance from an endpoint to a walkable surface.
    pub attachment_radius: f32,
    /// Additional traversal cost measured as equivalent path metres.
    pub extra_cost: f32,
}
impl Default for NavigationLink {
    fn default() -> Self {
        Self {
            enabled: true,
            destination: Vec3::ZERO,
            bidirectional: true,
            attachment_radius: 1.0,
            extra_cost: 0.0,
        }
    }
}
impl Component for NavigationLink {}

fn segment_hits_bounds(from: Vec3, to: Vec3, bounds: Bounds) -> bool {
    let delta = to - from;
    let mut low = 0.0_f32;
    let mut high = 1.0_f32;
    for axis in 0..3 {
        if delta[axis].abs() < 0.00001 {
            if from[axis] < bounds.min[axis] || from[axis] > bounds.max[axis] {
                return false;
            }
        } else {
            let a = (bounds.min[axis] - from[axis]) / delta[axis];
            let b = (bounds.max[axis] - from[axis]) / delta[axis];
            low = low.max(a.min(b));
            high = high.min(a.max(b));
            if low > high {
                return false;
            }
        }
    }
    high > 0.0001 && low < 0.9999
}
