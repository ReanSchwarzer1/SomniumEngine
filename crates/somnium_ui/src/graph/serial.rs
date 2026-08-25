//! Versioned, deterministic graph-asset serialisation.

use super::{Catalogue, ConnectError, Connection, Graph, Node, NodeId, PinDirection, PinRef};
use glam::Vec2;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt};

/// Current `.somgraph` schema version.
pub const GRAPH_ASSET_VERSION: u32 = 1;

/// A rejected graph asset.
#[derive(Debug)]
pub enum GraphAssetError {
    Json(serde_json::Error),
    FutureVersion(u32),
    CatalogueMismatch { expected: String, found: String },
    DuplicateNode(NodeId),
    UnknownArchetype(String),
    InvalidPosition(NodeId),
    InvalidSize(NodeId),
    InvalidLiteral { node: NodeId, pin: u16 },
    InvalidGroup { node: NodeId, group: NodeId },
    InvalidNextId,
    InvalidConnection(ConnectError),
}

impl fmt::Display for GraphAssetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(formatter, "invalid graph JSON: {error}"),
            Self::FutureVersion(version) => write!(
                formatter,
                "graph version {version} is newer than supported version {GRAPH_ASSET_VERSION}"
            ),
            Self::CatalogueMismatch { expected, found } => {
                write!(formatter, "graph catalogue is {found}, expected {expected}")
            }
            Self::DuplicateNode(id) => write!(formatter, "duplicate graph node {}", id.0),
            Self::UnknownArchetype(id) => write!(formatter, "unknown node archetype {id}"),
            Self::InvalidPosition(id) => {
                write!(formatter, "node {} has a non-finite position", id.0)
            }
            Self::InvalidSize(id) => write!(formatter, "node {} has an invalid size", id.0),
            Self::InvalidLiteral { node, pin } => {
                write!(formatter, "node {} has no input pin {pin}", node.0)
            }
            Self::InvalidGroup { node, group } => write!(
                formatter,
                "node {} has invalid group membership in {}",
                node.0, group.0
            ),
            Self::InvalidNextId => formatter.write_str("next node id would reuse an existing id"),
            Self::InvalidConnection(error) => {
                write!(formatter, "invalid graph connection: {error:?}")
            }
        }
    }
}

impl std::error::Error for GraphAssetError {}

impl From<serde_json::Error> for GraphAssetError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GraphAsset {
    #[serde(default)]
    version: u32,
    catalogue: String,
    #[serde(default)]
    context: Vec<String>,
    #[serde(default)]
    next_id: Option<u32>,
    #[serde(default)]
    nodes: Vec<AssetNode>,
    #[serde(default)]
    connections: Vec<AssetConnection>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AssetNode {
    id: u32,
    archetype: String,
    position: [f32; 2],
    #[serde(default, skip_serializing_if = "String::is_empty")]
    title: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    literals: BTreeMap<u16, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    size: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    group: Option<u32>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct AssetConnection {
    from: AssetPin,
    to: AssetPin,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct AssetPin {
    node: u32,
    index: u16,
    direction: AssetDirection,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum AssetDirection {
    Input,
    Output,
}

/// Serialise a graph deterministically. Nodes and connections are sorted by
/// durable ids and literal maps use a `BTreeMap`, so identical graph content
/// produces byte-identical output regardless of edit history.
pub fn to_json(graph: &Graph, catalogue: &Catalogue) -> Result<String, GraphAssetError> {
    let mut nodes: Vec<_> = graph.nodes().iter().map(AssetNode::from).collect();
    nodes.sort_by_key(|node| node.id);
    let mut connections: Vec<_> = graph
        .connections()
        .iter()
        .copied()
        .map(AssetConnection::from)
        .collect();
    connections.sort_by_key(|connection| {
        (
            connection.from.node,
            direction_key(connection.from.direction),
            connection.from.index,
            connection.to.node,
            direction_key(connection.to.direction),
            connection.to.index,
        )
    });
    let asset = GraphAsset {
        version: GRAPH_ASSET_VERSION,
        catalogue: catalogue.id.to_string(),
        context: graph.context().to_vec(),
        next_id: Some(graph.next_id),
        nodes,
        connections,
    };
    let mut json = serde_json::to_string_pretty(&asset)?;
    json.push('\n');
    Ok(json)
}

/// Load, migrate, and validate a graph asset against its catalogue.
pub fn from_json(json: &str, catalogue: &Catalogue) -> Result<Graph, GraphAssetError> {
    let asset: GraphAsset = serde_json::from_str(json)?;
    let asset = migrate(asset)?;
    if asset.catalogue != catalogue.id {
        return Err(GraphAssetError::CatalogueMismatch {
            expected: catalogue.id.to_string(),
            found: asset.catalogue,
        });
    }

    let mut graph = Graph::new();
    let mut nodes = asset.nodes;
    nodes.sort_by_key(|node| node.id);
    for node in nodes {
        let id = NodeId(node.id);
        if graph.nodes.iter().any(|existing| existing.id == id) {
            return Err(GraphAssetError::DuplicateNode(id));
        }
        let archetype = catalogue
            .get(&node.archetype)
            .ok_or_else(|| GraphAssetError::UnknownArchetype(node.archetype.clone()))?;
        let position = Vec2::from_array(node.position);
        if !position.is_finite() {
            return Err(GraphAssetError::InvalidPosition(id));
        }
        for &pin in node.literals.keys() {
            if archetype.inputs.get(pin as usize).is_none() {
                return Err(GraphAssetError::InvalidLiteral { node: id, pin });
            }
        }
        let size = node.size.map(Vec2::from_array);
        if size.is_some_and(|size| !size.is_finite() || size.x <= 0.0 || size.y <= 0.0) {
            return Err(GraphAssetError::InvalidSize(id));
        }
        graph.nodes.push(Node {
            id,
            archetype: node.archetype,
            position,
            title: node.title,
            literals: node.literals.into_iter().collect(),
            size,
            group: node.group.map(NodeId),
        });
    }

    let groups: Vec<_> = graph
        .nodes
        .iter()
        .filter_map(|node| node.group.map(|group| (node.id, group)))
        .collect();
    for (node, group) in groups {
        if graph.node(group).is_none() || !graph.set_group(node, Some(group)) {
            return Err(GraphAssetError::InvalidGroup { node, group });
        }
    }

    let derived_next = graph
        .nodes
        .iter()
        .map(|node| node.id.0)
        .max()
        .map_or(0, |id| id.saturating_add(1));
    let next_id = asset.next_id.unwrap_or(derived_next);
    if next_id < derived_next
        || (next_id == u32::MAX && graph.nodes.iter().any(|n| n.id.0 == u32::MAX))
    {
        return Err(GraphAssetError::InvalidNextId);
    }
    graph.next_id = next_id;
    graph.context = asset
        .context
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect();

    for connection in asset.connections {
        graph
            .connect(catalogue, connection.from.into(), connection.to.into())
            .map_err(GraphAssetError::InvalidConnection)?;
    }
    Ok(graph)
}

fn migrate(mut asset: GraphAsset) -> Result<GraphAsset, GraphAssetError> {
    match asset.version {
        // Version zero is the unversioned development format. Its node and
        // connection shape is identical; only the monotonic id cursor was not
        // persisted yet and is reconstructed by the loader.
        0 => {
            asset.version = GRAPH_ASSET_VERSION;
            Ok(asset)
        }
        GRAPH_ASSET_VERSION => Ok(asset),
        version => Err(GraphAssetError::FutureVersion(version)),
    }
}

fn direction_key(direction: AssetDirection) -> u8 {
    match direction {
        AssetDirection::Input => 0,
        AssetDirection::Output => 1,
    }
}

impl From<&Node> for AssetNode {
    fn from(node: &Node) -> Self {
        Self {
            id: node.id.0,
            archetype: node.archetype.clone(),
            position: node.position.to_array(),
            title: node.title.clone(),
            literals: node
                .literals
                .iter()
                .map(|(&pin, value)| (pin, value.clone()))
                .collect(),
            size: node.size.map(|size| size.to_array()),
            group: node.group.map(|id| id.0),
        }
    }
}

impl From<Connection> for AssetConnection {
    fn from(connection: Connection) -> Self {
        Self {
            from: connection.from.into(),
            to: connection.to.into(),
        }
    }
}

impl From<PinRef> for AssetPin {
    fn from(pin: PinRef) -> Self {
        Self {
            node: pin.node.0,
            index: pin.index,
            direction: match pin.direction {
                PinDirection::Input => AssetDirection::Input,
                PinDirection::Output => AssetDirection::Output,
            },
        }
    }
}

impl From<AssetPin> for PinRef {
    fn from(pin: AssetPin) -> Self {
        Self {
            node: NodeId(pin.node),
            index: pin.index,
            direction: match pin.direction {
                AssetDirection::Input => PinDirection::Input,
                AssetDirection::Output => PinDirection::Output,
            },
        }
    }
}
