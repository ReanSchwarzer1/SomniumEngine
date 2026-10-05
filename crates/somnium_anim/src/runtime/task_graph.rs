//! A compiled frame is a dependency DAG. Only ready work enters the shared job
//! pool; workers never wait for other workers. Polling installs data on the caller.
use super::*;
use somnium_jobs::{JobDesc, JobError, JobHandle, JobPriority, JobSystem};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub enum PoseTaskError {
    Graph(AnimGraphError),
    Job(JobError),
    Cancelled,
    AlreadyTaken,
}
impl From<AnimGraphError> for PoseTaskError {
    fn from(e: AnimGraphError) -> Self {
        Self::Graph(e)
    }
}

#[derive(Clone)]
enum Operation {
    Sample { clip: ClipId, time: f32 },
    Blend(f32),
    BlendThree([f32; 3]),
    Layer(Vec<(f32, Option<BoneMask>)>),
    Copy,
}
struct Task {
    operation: Operation,
    dependencies: Vec<usize>,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Request {
    node: AnimNodeId,
    phase: Option<(String, u32)>,
}
impl Request {
    fn phase(&self) -> Option<(&str, f32)> {
        self.phase
            .as_ref()
            .map(|(s, p)| (s.as_str(), f32::from_bits(*p)))
    }
    fn new(node: AnimNodeId, phase: Option<(&str, f32)>) -> Self {
        Self {
            node,
            phase: phase.map(|(name, phase)| (name.to_string(), phase.to_bits())),
        }
    }
}

/// Per-frame compiled work, retaining immutable assets by Arc. Construction is
/// iterative and deduplicates by node AND forced sync phase; one cached node may
/// legitimately be reached by two differently synchronized branches.
pub struct PoseTaskGraph {
    graph: Arc<AnimGraphAsset>,
    skeleton: Arc<Skeleton>,
    tasks: Vec<Task>,
    output: usize,
}

impl AnimGraphAsset {
    pub fn pose_tasks(
        self: &Arc<Self>,
        skeleton: Arc<Skeleton>,
        parameters: &ParameterSet,
        elapsed: f32,
    ) -> Result<PoseTaskGraph, AnimGraphError> {
        self.pose_tasks_for_node(self.output, skeleton, parameters, elapsed)
    }

    pub fn pose_tasks_for_node(
        self: &Arc<Self>,
        node: AnimNodeId,
        skeleton: Arc<Skeleton>,
        parameters: &ParameterSet,
        elapsed: f32,
    ) -> Result<PoseTaskGraph, AnimGraphError> {
        if skeleton.id() != self.skeleton {
            return Err(AnimGraphError::SkeletonMismatch);
        }
        if parameters.schema != self.parameters.id {
            return Err(AnimGraphError::ParameterSchemaMismatch);
        }
        if !elapsed.is_finite() {
            return Err(AnimGraphError::NonFiniteTime);
        }
        let first = Request::new(node, None);
        let mut indices = HashMap::from([(first.clone(), 0usize)]);
        let mut requests = vec![first];
        let mut tasks = Vec::new();
        let mut cursor = 0;
        while cursor < requests.len() {
            let request = requests[cursor].clone();
            let item = self
                .nodes
                .get(request.node.0 as usize)
                .ok_or(AnimGraphError::UnknownNode)?;
            let mut dependencies = Vec::new();
            let mut child = |node, phase| {
                let key = Request::new(node, phase);
                let next = indices.len();
                let index = *indices.entry(key.clone()).or_insert_with(|| {
                    requests.push(key);
                    next
                });
                dependencies.push(index);
            };
            let operation = match item {
                AnimNode::Clip { clip, playback } => {
                    let asset = self.clip(*clip)?;
                    let time = if let Some((name, phase)) = request.phase() {
                        asset
                            .sync_track(name)
                            .ok_or_else(|| AnimGraphError::MissingSyncTrack(name.to_string()))?
                            .time_at_phase(phase)
                            .map_err(|e| AnimGraphError::Clip(ClipError::Sync(e)))?
                    } else {
                        asset.local_time(elapsed, *playback)?
                    };
                    Operation::Sample { clip: *clip, time }
                }
                AnimNode::Blend1D {
                    parameter,
                    samples,
                    sync_track,
                    sync_leader,
                } => {
                    let value = parameter_float(parameters, parameter)?;
                    let phase = self.resolve_phase(
                        request.phase(),
                        sync_track.as_deref(),
                        samples[*sync_leader].node,
                        elapsed,
                    )?;
                    let upper = samples.partition_point(|s| s.position <= value);
                    if upper == 0 || upper == samples.len() {
                        child(samples[upper.saturating_sub(1)].node, phase);
                        Operation::Copy
                    } else {
                        let (a, b) = (&samples[upper - 1], &samples[upper]);
                        child(a.node, phase);
                        child(b.node, phase);
                        Operation::Blend((value - a.position) / (b.position - a.position))
                    }
                }
                AnimNode::Blend2D {
                    parameter_x,
                    parameter_y,
                    samples,
                    sync_track,
                    sync_leader,
                    ..
                } => {
                    let point = Vec2::new(
                        parameter_float(parameters, parameter_x)?,
                        parameter_float(parameters, parameter_y)?,
                    );
                    let (selected, weights) = self
                        .blend2d
                        .get(&request.node)
                        .ok_or(AnimGraphError::InvalidBlend)?
                        .weights(point)
                        .map_err(AnimGraphError::InvalidTriangulation)?;
                    let phase = self.resolve_phase(
                        request.phase(),
                        sync_track.as_deref(),
                        samples[*sync_leader].node,
                        elapsed,
                    )?;
                    for i in selected {
                        child(samples[i].node, phase);
                    }
                    Operation::BlendThree(weights)
                }
                AnimNode::Layer { base, layers } => {
                    child(*base, request.phase());
                    let mut settings = Vec::with_capacity(layers.len());
                    for layer in layers {
                        child(layer.node, request.phase());
                        let weight = match &layer.weight {
                            LayerWeight::Constant(w) => *w,
                            LayerWeight::Parameter(p) => parameter_float(parameters, p)?,
                        };
                        settings.push((weight, layer.mask.clone()));
                    }
                    Operation::Layer(settings)
                }
                AnimNode::Cache { source } => {
                    child(*source, request.phase());
                    Operation::Copy
                }
            };
            tasks.push(Task {
                operation,
                dependencies,
            });
            cursor += 1;
        }
        Ok(PoseTaskGraph {
            graph: self.clone(),
            skeleton,
            tasks,
            output: 0,
        })
    }
}

impl PoseTaskGraph {
    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }
    /// At most max_in_flight jobs from this pose occupy the shared pool. A full
    /// queue is retried by poll; cancellation/drop cancels outstanding handles.
    pub fn start(self, max_in_flight: usize) -> PoseEvaluation {
        let count = self.tasks.len();
        PoseEvaluation {
            plan: self,
            results: vec![None; count],
            running: (0..count).map(|_| None).collect(),
            max_in_flight: max_in_flight.max(1),
            cancelled: false,
            taken: false,
            submitted: 0,
        }
    }

    fn blend(mut self, mut other: Self, weight: f32) -> Self {
        let offset = self.tasks.len();
        for task in &mut other.tasks {
            for dependency in &mut task.dependencies {
                *dependency += offset;
            }
        }
        let dependencies = vec![self.output, other.output + offset];
        self.tasks.extend(other.tasks);
        self.output = self.tasks.len();
        self.tasks.push(Task {
            operation: Operation::Blend(weight),
            dependencies,
        });
        self
    }
}

impl StateMachinePlayer {
    /// Schedule both transition lanes as independent pose DAGs followed by a
    /// blend dependency. Sync-adjusted target time matches ordinary sampling.
    pub fn pose_tasks(
        &self,
        machine: &StateMachine,
        graph: &Arc<AnimGraphAsset>,
        skeleton: Arc<Skeleton>,
        parameters: &ParameterSet,
    ) -> Result<PoseTaskGraph, StateMachineError> {
        self.validate_binding(machine, graph)?;
        if parameters.schema != graph.parameters.id {
            return Err(StateMachineError::ParameterSchemaMismatch);
        }
        let Some(active) = &self.active else {
            return graph
                .pose_tasks_for_node(
                    machine.state(self.current)?.node,
                    skeleton,
                    parameters,
                    self.state_time,
                )
                .map_err(StateMachineError::Graph);
        };
        let transition = machine
            .transitions
            .get(active.index)
            .ok_or(StateMachineError::VersionMismatch)?;
        let source = machine.state(transition.from)?;
        let target = machine.state(transition.to)?;
        let target_time = if let Some(name) = &transition.sync_track {
            let phase = graph.sync_phase(source.node, active.source_time, name)?;
            graph.elapsed_at_phase(target.node, phase, name, active.target_time)?
        } else {
            active.target_time
        };
        let a = graph.pose_tasks_for_node(
            source.node,
            skeleton.clone(),
            parameters,
            active.source_time,
        )?;
        let b = graph.pose_tasks_for_node(target.node, skeleton, parameters, target_time)?;
        Ok(a.blend(
            b,
            (active.elapsed / transition.blend_seconds).clamp(0.0, 1.0),
        ))
    }
}

pub struct PoseEvaluation {
    plan: PoseTaskGraph,
    results: Vec<Option<Arc<Pose>>>,
    running: Vec<Option<JobHandle<Arc<Pose>>>>,
    max_in_flight: usize,
    cancelled: bool,
    taken: bool,
    submitted: usize,
}
impl PoseEvaluation {
    pub fn submitted_tasks(&self) -> usize {
        self.submitted
    }
    pub fn cancel(&mut self) {
        self.cancelled = true;
        for handle in self.running.iter().flatten() {
            handle.cancel();
        }
    }
    /// Nonblocking progress. A complete pose is returned once, never a partial
    /// palette. The single-threaded job adapter executes the same graph inline.
    pub fn poll(&mut self, jobs: &mut JobSystem) -> Result<Option<Pose>, PoseTaskError> {
        if self.cancelled {
            return Err(PoseTaskError::Cancelled);
        }
        if self.taken {
            return Err(PoseTaskError::AlreadyTaken);
        }
        loop {
            let mut progressed = false;
            for i in 0..self.running.len() {
                if let Some(result) = self.running[i].as_ref().and_then(JobHandle::try_take) {
                    self.running[i] = None;
                    match result {
                        Ok(pose) => self.results[i] = Some(pose),
                        Err(error) => {
                            self.cancel();
                            return Err(PoseTaskError::Job(error));
                        }
                    }
                    progressed = true;
                }
            }
            if let Some(pose) = self.results[self.plan.output].take() {
                self.taken = true;
                return Ok(Some(Arc::unwrap_or_clone(pose)));
            }
            let mut running = self.running.iter().flatten().count();
            for i in (0..self.plan.tasks.len()).rev() {
                if running >= self.max_in_flight {
                    break;
                }
                if self.running[i].is_some() || self.results[i].is_some() {
                    continue;
                }
                let task = &self.plan.tasks[i];
                let Some(inputs): Option<Vec<Arc<Pose>>> = task
                    .dependencies
                    .iter()
                    .map(|i| self.results[*i].clone())
                    .collect()
                else {
                    continue;
                };
                let operation = task.operation.clone();
                let graph = self.plan.graph.clone();
                let skeleton = self.plan.skeleton.clone();
                let desc = JobDesc::new("animation.pose")
                    .priority(JobPriority::Visible)
                    .housekeeping();
                match jobs.submit_with(desc, move |context| {
                    context.check_cancelled().map_err(|e| format!("{e:?}"))?;
                    let pose = match operation {
                        Operation::Sample { clip, time } => graph.clip(clip).and_then(|c| {
                            c.sample_local(&skeleton, time)
                                .map_err(AnimGraphError::Clip)
                        }),
                        Operation::Copy => Ok((*inputs[0]).clone()),
                        Operation::Blend(weight) => Ok(blend_pose(&inputs[0], &inputs[1], weight)),
                        Operation::BlendThree(weights) => Ok(blend_three(
                            &inputs.iter().map(|p| (**p).clone()).collect::<Vec<_>>(),
                            weights,
                        )),
                        Operation::Layer(settings) => {
                            let layers: Vec<_> = settings
                                .iter()
                                .zip(&inputs[1..])
                                .map(|((weight, mask), pose)| PoseLayer {
                                    pose,
                                    weight: *weight,
                                    mask: mask.as_ref(),
                                })
                                .collect();
                            layer_poses(&inputs[0], &layers).map_err(AnimGraphError::Layer)
                        }
                    }
                    .map_err(|e| format!("{e:?}"))?;
                    context.check_cancelled().map_err(|e| format!("{e:?}"))?;
                    Ok(Arc::new(pose))
                }) {
                    Ok(handle) => {
                        self.running[i] = Some(handle);
                        running += 1;
                        self.submitted += 1;
                        progressed = true;
                    }
                    Err(JobError::QueueFull) => break,
                    Err(error) => {
                        self.cancel();
                        return Err(PoseTaskError::Job(error));
                    }
                }
            }
            if !progressed {
                return Ok(None);
            }
        }
    }
}
impl Drop for PoseEvaluation {
    fn drop(&mut self) {
        for handle in self.running.iter().flatten() {
            handle.cancel();
        }
    }
}
