//! Compile MORROWIND-K animation catalogues into MORROWIND-V runtime assets.

use std::collections::{HashMap, HashSet};

use glam::Vec2;
use serde::{Deserialize, Serialize};
use somnium_anim::{
    AnimGraphAsset, AnimGraphError, AnimNode, AnimNodeId, AnimationClip, AnimationState, BoneMask,
    CompareOp, Condition, GraphId, LayerWeight, MachineId, NodeBlendSample1D, NodeBlendSample2D,
    NodeLayer, ParameterSchema, Playback, Skeleton, StateId, StateMachine, StateMachineError,
    StateTransition,
};

use super::{Catalogue, Graph, GraphSurface, Node, NodeId, PinRef};

/// Why an authored animation graph could not become a runtime asset.
#[derive(Clone, Debug, PartialEq)]
pub enum AnimationGraphCompileError {
    MissingRoot,
    MultipleRoots,
    Cycle,
    MissingPoseInput { node: NodeId, pin: u16 },
    MissingLiteral { node: NodeId, pin: u16 },
    InvalidLiteral { node: NodeId, pin: u16 },
    UnsupportedNode(String),
    Runtime(AnimGraphError),
}

impl From<AnimGraphError> for AnimationGraphCompileError {
    fn from(value: AnimGraphError) -> Self {
        Self::Runtime(value)
    }
}

/// A runtime animation graph plus the durable authored-node mapping needed by
/// state-machine overlays. Runtime node indices are deliberately not authored.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledAnimationGraph {
    asset: AnimGraphAsset,
    authored_nodes: HashMap<NodeId, AnimNodeId>,
}

impl CompiledAnimationGraph {
    #[must_use]
    pub fn asset(&self) -> &AnimGraphAsset {
        &self.asset
    }

    #[must_use]
    pub fn into_asset(self) -> AnimGraphAsset {
        self.asset
    }

    #[must_use]
    pub fn runtime_node(&self, authored: NodeId) -> Option<AnimNodeId> {
        self.authored_nodes.get(&authored).copied()
    }
}

/// A transition drawn between two `animation.state` nodes on the shared graph
/// surface. The surface owns layout, selection and node history; this record
/// owns the cyclic edge because pose-graph wires deliberately remain acyclic.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthoredStateTransition {
    pub from: NodeId,
    pub to: NodeId,
    pub conditions: Vec<Condition>,
    pub blend_seconds: f32,
    pub sync_track: Option<String>,
}

/// Current version of the animation state-machine overlay document.
pub const ANIMATION_STATE_DOCUMENT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq)]
struct StateDocumentSnapshot {
    initial: Option<NodeId>,
    transitions: Vec<AuthoredStateTransition>,
}

#[derive(Clone, Debug)]
struct StateDocumentHistoryEntry {
    label: &'static str,
    before: StateDocumentSnapshot,
    after: StateDocumentSnapshot,
}

/// Durable state layout and cyclic transition overlay over one K graph surface.
/// Pose/state node edits use `GraphSurface` history; initial-state and
/// transition edits use the bounded overlay history here.
#[derive(Clone)]
pub struct AnimationStateMachineDocument {
    surface: GraphSurface,
    initial: Option<NodeId>,
    transitions: Vec<AuthoredStateTransition>,
    history: Vec<StateDocumentHistoryEntry>,
    cursor: usize,
}

impl AnimationStateMachineDocument {
    #[must_use]
    pub fn new(surface: GraphSurface) -> Self {
        Self {
            surface,
            initial: None,
            transitions: Vec::new(),
            history: Vec::new(),
            cursor: 0,
        }
    }

    #[must_use]
    pub fn surface(&self) -> &GraphSurface {
        &self.surface
    }

    pub fn surface_mut(&mut self) -> &mut GraphSurface {
        &mut self.surface
    }

    #[must_use]
    pub fn initial(&self) -> Option<NodeId> {
        self.initial
    }

    #[must_use]
    pub fn transitions(&self) -> &[AuthoredStateTransition] {
        &self.transitions
    }

    pub fn set_initial(&mut self, state: NodeId) -> bool {
        if !self.is_state(state) {
            return false;
        }
        self.edit_overlay("Set Initial Animation State", |snapshot| {
            snapshot.initial = Some(state);
        })
    }

    pub fn add_transition(&mut self, transition: AuthoredStateTransition) -> bool {
        if !self.valid_transition(&transition) {
            return false;
        }
        self.edit_overlay("Add Animation Transition", |snapshot| {
            snapshot.transitions.push(transition);
        })
    }

    /// Replace every authored transition knob as one overlay undo step.
    pub fn set_transition(&mut self, index: usize, transition: AuthoredStateTransition) -> bool {
        if self.transitions.get(index).is_none() || !self.valid_transition(&transition) {
            return false;
        }
        self.edit_overlay("Edit Animation Transition", |snapshot| {
            snapshot.transitions[index] = transition;
        })
    }

    /// Remove one transition as one overlay undo step.
    pub fn remove_transition(&mut self, index: usize) -> bool {
        if self.transitions.get(index).is_none() {
            return false;
        }
        self.edit_overlay("Delete Animation Transition", |snapshot| {
            snapshot.transitions.remove(index);
        })
    }

    pub fn undo_overlay(&mut self) -> Option<&'static str> {
        if self.cursor == 0 {
            return None;
        }
        self.cursor -= 1;
        let entry = &self.history[self.cursor];
        self.initial = entry.before.initial;
        self.transitions.clone_from(&entry.before.transitions);
        Some(entry.label)
    }

    pub fn redo_overlay(&mut self) -> Option<&'static str> {
        let entry = self.history.get(self.cursor)?;
        self.initial = entry.after.initial;
        self.transitions.clone_from(&entry.after.transitions);
        self.cursor += 1;
        Some(entry.label)
    }

    pub fn to_json(&self) -> Result<String, AnimationStateDocumentError> {
        let graph = super::serial::to_json(&self.surface.graph, &self.surface.catalogue)
            .map_err(AnimationStateDocumentError::Graph)?;
        let graph = serde_json::from_str(&graph).map_err(AnimationStateDocumentError::Json)?;
        serde_json::to_string_pretty(&StateDocumentAsset {
            version: ANIMATION_STATE_DOCUMENT_VERSION,
            graph,
            initial: self.initial.map(|node| node.0),
            transitions: self.transitions.iter().map(AssetTransition::from).collect(),
        })
        .map_err(AnimationStateDocumentError::Json)
    }

    pub fn from_json(
        text: &str,
        catalogue: Catalogue,
    ) -> Result<Self, AnimationStateDocumentError> {
        let asset: StateDocumentAsset =
            serde_json::from_str(text).map_err(AnimationStateDocumentError::Json)?;
        if asset.version > ANIMATION_STATE_DOCUMENT_VERSION {
            return Err(AnimationStateDocumentError::FutureVersion(asset.version));
        }
        let graph_text =
            serde_json::to_string(&asset.graph).map_err(AnimationStateDocumentError::Json)?;
        let graph = super::serial::from_json(&graph_text, &catalogue)
            .map_err(AnimationStateDocumentError::Graph)?;
        let mut surface = GraphSurface::new(catalogue);
        surface.graph = graph;
        let mut document = Self::new(surface);
        document.initial = asset.initial.map(NodeId);
        document.transitions = asset
            .transitions
            .into_iter()
            .map(AuthoredStateTransition::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        if document
            .initial
            .is_some_and(|node| !document.is_state(node))
            || document
                .transitions
                .iter()
                .any(|edge| !document.is_state(edge.from) || !document.is_state(edge.to))
        {
            return Err(AnimationStateDocumentError::UnknownStateNode);
        }
        Ok(document)
    }

    fn is_state(&self, node: NodeId) -> bool {
        self.surface
            .graph
            .node(node)
            .is_some_and(|node| node.archetype == "animation.state")
    }

    fn valid_transition(&self, transition: &AuthoredStateTransition) -> bool {
        self.is_state(transition.from)
            && self.is_state(transition.to)
            && transition.blend_seconds.is_finite()
            && transition.blend_seconds >= 0.0
    }

    fn snapshot(&self) -> StateDocumentSnapshot {
        StateDocumentSnapshot {
            initial: self.initial,
            transitions: self.transitions.clone(),
        }
    }

    fn edit_overlay(
        &mut self,
        label: &'static str,
        edit: impl FnOnce(&mut StateDocumentSnapshot),
    ) -> bool {
        let before = self.snapshot();
        let mut after = before.clone();
        edit(&mut after);
        if before == after {
            return false;
        }
        self.history.truncate(self.cursor);
        self.history.push(StateDocumentHistoryEntry {
            label,
            before,
            after: after.clone(),
        });
        if self.history.len() > 128 {
            self.history.remove(0);
        }
        self.cursor = self.history.len();
        self.initial = after.initial;
        self.transitions = after.transitions;
        true
    }
}

#[derive(Debug)]
pub enum AnimationStateDocumentError {
    Json(serde_json::Error),
    Graph(super::serial::GraphAssetError),
    FutureVersion(u32),
    UnknownStateNode,
    InvalidCondition,
}

#[derive(Serialize, Deserialize)]
struct StateDocumentAsset {
    version: u32,
    graph: serde_json::Value,
    initial: Option<u32>,
    transitions: Vec<AssetTransition>,
}

#[derive(Serialize, Deserialize)]
struct AssetTransition {
    from: u32,
    to: u32,
    conditions: Vec<AssetCondition>,
    blend_seconds: f32,
    sync_track: Option<String>,
}

impl From<&AuthoredStateTransition> for AssetTransition {
    fn from(value: &AuthoredStateTransition) -> Self {
        Self {
            from: value.from.0,
            to: value.to.0,
            conditions: value.conditions.iter().map(AssetCondition::from).collect(),
            blend_seconds: value.blend_seconds,
            sync_track: value.sync_track.clone(),
        }
    }
}

impl TryFrom<AssetTransition> for AuthoredStateTransition {
    type Error = AnimationStateDocumentError;

    fn try_from(value: AssetTransition) -> Result<Self, Self::Error> {
        Ok(Self {
            from: NodeId(value.from),
            to: NodeId(value.to),
            conditions: value
                .conditions
                .into_iter()
                .map(Condition::try_from)
                .collect::<Result<Vec<_>, _>>()?,
            blend_seconds: value.blend_seconds,
            sync_track: value.sync_track,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum AssetCondition {
    Bool {
        parameter: String,
        value: bool,
    },
    Float {
        parameter: String,
        op: String,
        value: f32,
    },
    Int {
        parameter: String,
        op: String,
        value: i64,
    },
    Trigger {
        parameter: String,
    },
}

impl From<&Condition> for AssetCondition {
    fn from(value: &Condition) -> Self {
        match value {
            Condition::Bool { parameter, value } => Self::Bool {
                parameter: parameter.clone(),
                value: *value,
            },
            Condition::Float {
                parameter,
                op,
                value,
            } => Self::Float {
                parameter: parameter.clone(),
                op: compare_op_name(*op).into(),
                value: *value,
            },
            Condition::Int {
                parameter,
                op,
                value,
            } => Self::Int {
                parameter: parameter.clone(),
                op: compare_op_name(*op).into(),
                value: *value,
            },
            Condition::Trigger { parameter } => Self::Trigger {
                parameter: parameter.clone(),
            },
        }
    }
}

impl TryFrom<AssetCondition> for Condition {
    type Error = AnimationStateDocumentError;

    fn try_from(value: AssetCondition) -> Result<Self, Self::Error> {
        Ok(match value {
            AssetCondition::Bool { parameter, value } => Self::Bool { parameter, value },
            AssetCondition::Float {
                parameter,
                op,
                value,
            } => Self::Float {
                parameter,
                op: parse_compare_op(&op)?,
                value,
            },
            AssetCondition::Int {
                parameter,
                op,
                value,
            } => Self::Int {
                parameter,
                op: parse_compare_op(&op)?,
                value,
            },
            AssetCondition::Trigger { parameter } => Self::Trigger { parameter },
        })
    }
}

fn compare_op_name(op: CompareOp) -> &'static str {
    match op {
        CompareOp::Equal => "equal",
        CompareOp::NotEqual => "not_equal",
        CompareOp::Less => "less",
        CompareOp::LessEqual => "less_equal",
        CompareOp::Greater => "greater",
        CompareOp::GreaterEqual => "greater_equal",
    }
}

fn parse_compare_op(text: &str) -> Result<CompareOp, AnimationStateDocumentError> {
    match text {
        "equal" => Ok(CompareOp::Equal),
        "not_equal" => Ok(CompareOp::NotEqual),
        "less" => Ok(CompareOp::Less),
        "less_equal" => Ok(CompareOp::LessEqual),
        "greater" => Ok(CompareOp::Greater),
        "greater_equal" => Ok(CompareOp::GreaterEqual),
        _ => Err(AnimationStateDocumentError::InvalidCondition),
    }
}

/// Why a state-machine layout on the shared graph surface could not compile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnimationStateMachineCompileError {
    NoInitialState,
    UnknownStateNode(NodeId),
    DuplicateStateId(StateId),
    InvalidLiteral { node: NodeId, pin: u16 },
    Runtime(StateMachineError),
}

impl From<StateMachineError> for AnimationStateMachineCompileError {
    fn from(value: StateMachineError) -> Self {
        Self::Runtime(value)
    }
}

/// Compile the state nodes laid out on MORROWIND-K's reusable surface into the
/// UI-neutral runtime machine. State transitions are allowed to cycle, so they
/// are authored as overlay edges instead of weakening the pose graph's DAG
/// invariant.
pub fn compile_state_machine(
    document: &AnimationStateMachineDocument,
    graph: &CompiledAnimationGraph,
    id: MachineId,
    definition_version: u32,
) -> Result<StateMachine, AnimationStateMachineCompileError> {
    let surface = document.surface();
    let mut node_to_state = HashMap::new();
    let mut states = Vec::new();
    for node in surface
        .graph
        .nodes()
        .iter()
        .filter(|node| node.archetype == "animation.state")
    {
        let state_id = StateId(parse::<u16>(node, &surface.catalogue, 1).map_err(|_| {
            AnimationStateMachineCompileError::InvalidLiteral {
                node: node.id,
                pin: 1,
            }
        })?);
        if node_to_state.values().any(|existing| *existing == state_id) {
            return Err(AnimationStateMachineCompileError::DuplicateStateId(
                state_id,
            ));
        }
        let pose_source = surface
            .graph
            .input_source(PinRef::input(node.id, 0))
            .ok_or(AnimationStateMachineCompileError::InvalidLiteral {
                node: node.id,
                pin: 0,
            })?;
        let runtime_node = graph.runtime_node(pose_source.node).ok_or(
            AnimationStateMachineCompileError::InvalidLiteral {
                node: node.id,
                pin: 0,
            },
        )?;
        node_to_state.insert(node.id, state_id);
        states.push(AnimationState {
            id: state_id,
            name: if node.title.trim().is_empty() {
                format!("State {}", state_id.0)
            } else {
                node.title.clone()
            },
            node: runtime_node,
        });
    }
    let initial_node = document
        .initial()
        .ok_or(AnimationStateMachineCompileError::NoInitialState)?;
    let initial = *node_to_state
        .get(&initial_node)
        .ok_or(AnimationStateMachineCompileError::NoInitialState)?;
    let transitions = document
        .transitions()
        .iter()
        .map(|transition| {
            Ok(StateTransition {
                from: *node_to_state.get(&transition.from).ok_or(
                    AnimationStateMachineCompileError::UnknownStateNode(transition.from),
                )?,
                to: *node_to_state.get(&transition.to).ok_or(
                    AnimationStateMachineCompileError::UnknownStateNode(transition.to),
                )?,
                conditions: transition.conditions.clone(),
                blend_seconds: transition.blend_seconds,
                sync_track: transition.sync_track.clone(),
            })
        })
        .collect::<Result<Vec<_>, AnimationStateMachineCompileError>>()?;
    StateMachine::new(
        id,
        definition_version,
        graph.asset(),
        states,
        transitions,
        initial,
    )
    .map_err(Into::into)
}

/// Compile the output and state-reachable pose branches into a runtime asset.
/// `definition_version` is a caller-owned content revision, not the `.somgraph`
/// schema version; hot reload must increment it.
#[allow(clippy::too_many_arguments)]
pub fn compile_animation(
    graph: &Graph,
    catalogue: &Catalogue,
    id: GraphId,
    definition_version: u32,
    skeleton: &Skeleton,
    clips: Vec<AnimationClip>,
    parameters: ParameterSchema,
) -> Result<AnimGraphAsset, AnimationGraphCompileError> {
    compile_animation_document(
        graph,
        catalogue,
        id,
        definition_version,
        skeleton,
        clips,
        parameters,
    )
    .map(CompiledAnimationGraph::into_asset)
}

/// Compile while retaining the authored-to-runtime node map needed by a state
/// machine document on the same shared graph surface.
#[allow(clippy::too_many_arguments)]
pub fn compile_animation_document(
    graph: &Graph,
    catalogue: &Catalogue,
    id: GraphId,
    definition_version: u32,
    skeleton: &Skeleton,
    clips: Vec<AnimationClip>,
    parameters: ParameterSchema,
) -> Result<CompiledAnimationGraph, AnimationGraphCompileError> {
    let root_kind = catalogue
        .root()
        .ok_or(AnimationGraphCompileError::MissingRoot)?;
    let roots: Vec<_> = graph
        .nodes()
        .iter()
        .filter(|node| node.archetype == root_kind)
        .map(|node| node.id)
        .collect();
    let root = match roots.as_slice() {
        [] => return Err(AnimationGraphCompileError::MissingRoot),
        [root] => *root,
        _ => return Err(AnimationGraphCompileError::MultipleRoots),
    };
    let source = graph
        .input_source(PinRef::input(root, 0))
        .ok_or(AnimationGraphCompileError::MissingPoseInput { node: root, pin: 0 })?;
    let reachable = reachable_from(graph, catalogue, root);
    let order = graph
        .topological_order()
        .ok_or(AnimationGraphCompileError::Cycle)?;
    let mut compiled = Vec::new();
    let mut remap = HashMap::new();

    for node_id in order.into_iter().filter(|node| reachable.contains(node)) {
        let node = graph
            .node(node_id)
            .expect("topological order contains only graph nodes");
        let runtime = match node.archetype.as_str() {
            "animation.clip" => Some(AnimNode::Clip {
                clip: somnium_anim::ClipId(parse::<u64>(node, catalogue, 0)?),
                playback: Playback::new(
                    parse::<bool>(node, catalogue, 2)?,
                    parse::<f32>(node, catalogue, 1)?,
                )
                .map_err(|_| AnimationGraphCompileError::InvalidLiteral {
                    node: node.id,
                    pin: 1,
                })?,
            }),
            "animation.blend1d" => Some(AnimNode::Blend1D {
                parameter: literal(node, catalogue, 4)?.to_string(),
                samples: vec![
                    NodeBlendSample1D {
                        position: parse(node, catalogue, 2)?,
                        node: pose_input(graph, &remap, node.id, 0)?,
                    },
                    NodeBlendSample1D {
                        position: parse(node, catalogue, 3)?,
                        node: pose_input(graph, &remap, node.id, 1)?,
                    },
                ],
                sync_track: optional_literal(node, catalogue, 5)?,
                sync_leader: parse(node, catalogue, 6)?,
            }),
            "animation.blend1d3" => Some(AnimNode::Blend1D {
                parameter: literal(node, catalogue, 6)?.to_string(),
                samples: (0..3)
                    .map(|pin| {
                        Ok(NodeBlendSample1D {
                            position: parse(node, catalogue, pin + 3)?,
                            node: pose_input(graph, &remap, node.id, pin)?,
                        })
                    })
                    .collect::<Result<Vec<_>, AnimationGraphCompileError>>()?,
                sync_track: optional_literal(node, catalogue, 7)?,
                sync_leader: parse(node, catalogue, 8)?,
            }),
            "animation.blend2d" => Some(AnimNode::Blend2D {
                parameter_x: literal(node, catalogue, 9)?.to_string(),
                parameter_y: literal(node, catalogue, 10)?.to_string(),
                samples: vec![
                    NodeBlendSample2D {
                        position: Vec2::new(parse(node, catalogue, 3)?, parse(node, catalogue, 4)?),
                        node: pose_input(graph, &remap, node.id, 0)?,
                    },
                    NodeBlendSample2D {
                        position: Vec2::new(parse(node, catalogue, 5)?, parse(node, catalogue, 6)?),
                        node: pose_input(graph, &remap, node.id, 1)?,
                    },
                    NodeBlendSample2D {
                        position: Vec2::new(parse(node, catalogue, 7)?, parse(node, catalogue, 8)?),
                        node: pose_input(graph, &remap, node.id, 2)?,
                    },
                ],
                triangles: vec![[0, 1, 2]],
                sync_track: optional_literal(node, catalogue, 11)?,
                sync_leader: parse(node, catalogue, 12)?,
            }),
            "animation.blend2d4" => Some(AnimNode::Blend2D {
                parameter_x: literal(node, catalogue, 12)?.to_string(),
                parameter_y: literal(node, catalogue, 13)?.to_string(),
                samples: (0..4)
                    .map(|pin| {
                        Ok(NodeBlendSample2D {
                            position: Vec2::new(
                                parse(node, catalogue, 4 + pin * 2)?,
                                parse(node, catalogue, 5 + pin * 2)?,
                            ),
                            node: pose_input(graph, &remap, node.id, pin)?,
                        })
                    })
                    .collect::<Result<Vec<_>, AnimationGraphCompileError>>()?,
                triangles: parse_triangles(node, catalogue, 14)?,
                sync_track: optional_literal(node, catalogue, 15)?,
                sync_leader: parse(node, catalogue, 16)?,
            }),
            "animation.layer" => Some(AnimNode::Layer {
                base: pose_input(graph, &remap, node.id, 0)?,
                layers: vec![NodeLayer {
                    node: pose_input(graph, &remap, node.id, 1)?,
                    weight: optional_literal(node, catalogue, 3)?.map_or_else(
                        || parse(node, catalogue, 2).map(LayerWeight::Constant),
                        |parameter| Ok(LayerWeight::Parameter(parameter)),
                    )?,
                    mask: parse_mask(node, catalogue, skeleton, 4)?,
                }],
            }),
            "animation.cache" => Some(AnimNode::Cache {
                source: pose_input(graph, &remap, node.id, 0)?,
            }),
            "animation.reroute.pose" => {
                let source = pose_input(graph, &remap, node.id, 0)?;
                remap.insert(node.id, source);
                None
            }
            "animation.output" | "animation.state" | "graph.comment" | "graph.group" => None,
            other => {
                return Err(AnimationGraphCompileError::UnsupportedNode(
                    other.to_string(),
                ));
            }
        };
        if let Some(runtime) = runtime {
            let id = AnimNodeId(compiled.len() as u32);
            compiled.push(runtime);
            remap.insert(node.id, id);
        }
    }

    let output = remap
        .get(&source.node)
        .copied()
        .ok_or(AnimationGraphCompileError::MissingPoseInput { node: root, pin: 0 })?;
    let asset = AnimGraphAsset::new(
        id,
        definition_version,
        skeleton,
        clips,
        compiled,
        parameters,
        output,
    )?;
    Ok(CompiledAnimationGraph {
        asset,
        authored_nodes: remap,
    })
}

fn pose_input(
    graph: &Graph,
    remap: &HashMap<NodeId, AnimNodeId>,
    node: NodeId,
    pin: u16,
) -> Result<AnimNodeId, AnimationGraphCompileError> {
    let source = graph
        .input_source(PinRef::input(node, pin))
        .ok_or(AnimationGraphCompileError::MissingPoseInput { node, pin })?;
    remap
        .get(&source.node)
        .copied()
        .ok_or(AnimationGraphCompileError::MissingPoseInput { node, pin })
}

fn reachable_from(graph: &Graph, catalogue: &Catalogue, root: NodeId) -> HashSet<NodeId> {
    let mut reachable = HashSet::new();
    let mut stack = vec![root];
    stack.extend(
        graph
            .nodes()
            .iter()
            .filter(|node| node.archetype == "animation.state")
            .map(|node| node.id),
    );
    while let Some(node) = stack.pop() {
        if !reachable.insert(node) {
            continue;
        }
        let Some(archetype) = graph
            .node(node)
            .and_then(|node| catalogue.get(&node.archetype))
        else {
            continue;
        };
        for pin in 0..archetype.inputs.len() as u16 {
            if let Some(source) = graph.input_source(PinRef::input(node, pin)) {
                stack.push(source.node);
            }
        }
    }
    reachable
}

fn literal<'a>(
    node: &'a Node,
    catalogue: &'a Catalogue,
    pin: u16,
) -> Result<&'a str, AnimationGraphCompileError> {
    node.literals
        .get(&pin)
        .map(String::as_str)
        .or_else(|| {
            catalogue
                .get(&node.archetype)
                .and_then(|archetype| archetype.inputs.get(pin as usize))
                .and_then(|input| input.default)
        })
        .ok_or(AnimationGraphCompileError::MissingLiteral { node: node.id, pin })
}

fn optional_literal(
    node: &Node,
    catalogue: &Catalogue,
    pin: u16,
) -> Result<Option<String>, AnimationGraphCompileError> {
    Ok(match literal(node, catalogue, pin)?.trim() {
        "" => None,
        value => Some(value.to_string()),
    })
}

fn parse<T: std::str::FromStr>(
    node: &Node,
    catalogue: &Catalogue,
    pin: u16,
) -> Result<T, AnimationGraphCompileError> {
    literal(node, catalogue, pin)?
        .trim()
        .parse()
        .map_err(|_| AnimationGraphCompileError::InvalidLiteral { node: node.id, pin })
}

fn parse_triangles(
    node: &Node,
    catalogue: &Catalogue,
    pin: u16,
) -> Result<Vec<[u16; 3]>, AnimationGraphCompileError> {
    literal(node, catalogue, pin)?
        .split(';')
        .map(|triangle| {
            let indices = triangle
                .split(',')
                .map(str::trim)
                .map(str::parse::<u16>)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| AnimationGraphCompileError::InvalidLiteral { node: node.id, pin })?;
            indices
                .try_into()
                .map_err(|_| AnimationGraphCompileError::InvalidLiteral { node: node.id, pin })
        })
        .collect()
}

fn parse_mask(
    node: &Node,
    catalogue: &Catalogue,
    skeleton: &Skeleton,
    pin: u16,
) -> Result<Option<BoneMask>, AnimationGraphCompileError> {
    let text = literal(node, catalogue, pin)?.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let weights = text
        .split(',')
        .map(str::trim)
        .map(str::parse::<f32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AnimationGraphCompileError::InvalidLiteral { node: node.id, pin })?;
    BoneMask::new(skeleton, weights)
        .map(Some)
        .map_err(|_| AnimationGraphCompileError::InvalidLiteral { node: node.id, pin })
}
