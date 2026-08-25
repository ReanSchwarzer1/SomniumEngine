//! MORROWIND-K — Seam 8a: **one** graph surface.
//!
//! # Why one
//!
//! §6.2 is the evidence and §9.2 is the conclusion: this and MORROWIND-D are
//! the two enabling primitives, each unblocking five or more sub-phases, and
//! the two places where a *"just enough for this one feature"* implementation
//! has to be rewritten. Flax ships **eight tools on one surface**; Babylon
//! ships six node editors on one substrate. Somnium is about to want a material
//! graph, an animation graph, a behaviour tree, a VFX graph and a scattering
//! graph, and building five is building four too many.
//!
//! # Archetype-driven, which is what "one surface" actually requires
//!
//! The surface knows nothing about materials, animation or particles. A feature
//! contributes a [`Catalogue`] of [`NodeArchetype`]s — *"a Multiply node has two
//! float inputs and one float output"* — and the surface can then create,
//! connect, validate, lay out, serialise and draw it without a line of
//! feature-specific code.
//!
//! That is the difference between a framework and a tool, and it is testable:
//! `A.7`'s check for this track is **"a second catalogue exists"**. Two ship
//! here, in [`catalogues`], and one of them is not a material graph.
//!
//! # What is here
//!
//! The model, the rules and the geometry: nodes, typed pins, connection
//! validity, cycle rejection, selection, box selection, pan and zoom,
//! reroutes, comments, groups, copy and paste, and versioned serialisation —
//! plus the wire geometry, which is a cubic bezier through
//! [`Path::wire`](crate::path::Path::wire), the primitive MORROWIND-D built for
//! exactly this.
//!
//! Everything in this module is testable without a GPU, which for a system
//! whose bugs are *"the wire connected to the wrong pin"* is the half that
//! matters.

pub mod archetype;
pub mod catalogues;
pub mod geometry;
pub mod material;
pub mod serial;
pub mod surface;
pub mod widget;

pub use archetype::{
    Catalogue, GroupArchetype, NodeArchetype, NodeElementArchetype, PinArchetype, PinDirection,
    PinType,
};
pub use geometry::{GraphView, NodeLayout, PinLayout};
pub use material::{CompiledMaterialGraph, MaterialGraphError};
pub use surface::{GraphHistory, GraphSurface};
pub use widget::{GraphEditor, GraphEditorBuilder, GraphEditorMessage};

use glam::Vec2;
use std::collections::{HashMap, HashSet};

/// A node's identity within one graph.
///
/// Stable across a save and reload, which is what makes a connection something
/// that can be written to a file. Allocated monotonically and never reused, so
/// deleting a node cannot make a stale reference point at a new one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

/// One end of a connection: a node, and which of its pins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PinRef {
    pub node: NodeId,
    /// Index into the archetype's inputs or outputs, per [`Self::direction`].
    pub index: u16,
    pub direction: PinDirection,
}

impl PinRef {
    #[must_use]
    pub fn input(node: NodeId, index: u16) -> Self {
        Self {
            node,
            index,
            direction: PinDirection::Input,
        }
    }

    #[must_use]
    pub fn output(node: NodeId, index: u16) -> Self {
        Self {
            node,
            index,
            direction: PinDirection::Output,
        }
    }
}

/// A wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Connection {
    pub from: PinRef,
    pub to: PinRef,
}

/// One node in a graph.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: NodeId,
    /// Which archetype in the catalogue this is.
    pub archetype: String,
    /// Top-left, in graph space. Graph space is unbounded and unscaled; the
    /// view's transform is what turns it into pixels.
    pub position: Vec2,
    /// Author-editable title. Empty means "use the archetype's".
    pub title: String,
    /// Constant values for unconnected inputs, by pin index.
    ///
    /// A `String` because the surface does not know what a float is — the
    /// catalogue's owner parses it. The alternative, a typed value enum here,
    /// would put every feature's value types in the surface, which is exactly
    /// the coupling archetypes exist to avoid.
    pub literals: HashMap<u16, String>,
    /// Comment and group nodes carry a size; ordinary nodes are sized by their
    /// pins.
    pub size: Option<Vec2>,
    /// Optional containing group. Selection and expansion remain view state;
    /// membership is authored graph data and therefore serialises.
    pub group: Option<NodeId>,
}

/// Why a connection was refused.
///
/// Every variant is a real mistake somebody makes with a node editor, and
/// naming them is what lets the UI say *why* the wire would not attach rather
/// than silently dropping it — which is the single most common complaint about
/// every node editor ever shipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectError {
    /// One of the two nodes is not in the graph.
    UnknownNode,
    /// A pin index past the archetype's pin count.
    UnknownPin,
    /// Output to output, or input to input.
    SameDirection,
    /// A node connected to itself.
    SelfConnection,
    /// The types do not convert.
    TypeMismatch { from: PinType, to: PinType },
    /// Would make a loop.
    ///
    /// Refused for every graph kind here. A behaviour tree and a material graph
    /// are both acyclic; a state machine has cycles by construction and is
    /// **not** a graph in this sense — MORROWIND-V's transitions are edges in a
    /// different model, which is stated here so a later sub-phase does not try
    /// to relax this rule and break the other four consumers.
    WouldCycle,
    /// The target input is already connected.
    ///
    /// Not an error the surface *decides*: an input takes one wire, an output
    /// fans out. Reported rather than silently replacing, so the UI can choose
    /// to replace and say so.
    InputOccupied,
}

/// Why a copied graph fragment could not be pasted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteError {
    /// A node names an archetype the active catalogue does not provide.
    UnknownArchetype,
    /// A literal addresses a pin that is not an input.
    UnknownLiteral,
    /// One of the fragment's internal connections is not valid in this
    /// catalogue.
    InvalidConnection(ConnectError),
}

/// A node graph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    nodes: Vec<Node>,
    connections: Vec<Connection>,
    next_id: u32,
    context: Vec<String>,
}

impl Graph {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a node of `archetype` at `position`.
    ///
    /// Returns `None` when the catalogue has no such archetype — a graph
    /// containing a node nobody can evaluate is worse than a refused paste.
    pub fn add(
        &mut self,
        catalogue: &Catalogue,
        archetype: &str,
        position: Vec2,
    ) -> Option<NodeId> {
        catalogue.get(archetype)?;
        let next_id = self.next_id.checked_add(1)?;
        let id = NodeId(self.next_id);
        self.next_id = next_id;
        self.nodes.push(Node {
            id,
            archetype: archetype.to_string(),
            position,
            title: String::new(),
            literals: HashMap::new(),
            size: None,
            group: None,
        });
        Some(id)
    }

    /// Remove a node and every wire touching it.
    ///
    /// Returns the connections that were removed, so an undo command can put
    /// them back — CONTROL's command registry needs the inverse, not just the
    /// action.
    pub fn remove(&mut self, id: NodeId) -> Vec<Connection> {
        let removed: Vec<Connection> = self
            .connections
            .iter()
            .copied()
            .filter(|c| c.from.node == id || c.to.node == id)
            .collect();
        self.connections
            .retain(|c| c.from.node != id && c.to.node != id);
        self.nodes.retain(|n| n.id != id);
        for node in &mut self.nodes {
            if node.group == Some(id) {
                node.group = None;
            }
        }
        removed
    }

    /// Whether a connection would be legal, and why not if it would not.
    ///
    /// Separate from [`Self::connect`] so the UI can colour a wire while it is
    /// still being dragged — a node editor that only tells you the connection
    /// was wrong *after* you let go is a node editor people fight.
    pub fn can_connect(
        &self,
        catalogue: &Catalogue,
        from: PinRef,
        to: PinRef,
    ) -> Result<(), ConnectError> {
        if from.direction == to.direction {
            return Err(ConnectError::SameDirection);
        }
        // Normalise so `from` is always the output.
        let (from, to) = if from.direction == PinDirection::Output {
            (from, to)
        } else {
            (to, from)
        };
        if from.node == to.node {
            return Err(ConnectError::SelfConnection);
        }

        let source = self.node(from.node).ok_or(ConnectError::UnknownNode)?;
        let target = self.node(to.node).ok_or(ConnectError::UnknownNode)?;
        let source_arch = catalogue
            .get(&source.archetype)
            .ok_or(ConnectError::UnknownNode)?;
        let target_arch = catalogue
            .get(&target.archetype)
            .ok_or(ConnectError::UnknownNode)?;

        let out_pin = source_arch
            .outputs
            .get(from.index as usize)
            .ok_or(ConnectError::UnknownPin)?;
        let in_pin = target_arch
            .inputs
            .get(to.index as usize)
            .ok_or(ConnectError::UnknownPin)?;

        if !out_pin.ty.connects_to(in_pin.ty) {
            return Err(ConnectError::TypeMismatch {
                from: out_pin.ty,
                to: in_pin.ty,
            });
        }
        if self.connections.iter().any(|c| c.to == to) {
            return Err(ConnectError::InputOccupied);
        }
        if self.reaches(to.node, from.node) {
            return Err(ConnectError::WouldCycle);
        }
        Ok(())
    }

    /// Connect two pins.
    ///
    /// On [`ConnectError::InputOccupied`] the caller decides: `connect` refuses,
    /// [`Self::reconnect`] replaces.
    pub fn connect(
        &mut self,
        catalogue: &Catalogue,
        from: PinRef,
        to: PinRef,
    ) -> Result<Connection, ConnectError> {
        self.can_connect(catalogue, from, to)?;
        let (from, to) = if from.direction == PinDirection::Output {
            (from, to)
        } else {
            (to, from)
        };
        let connection = Connection { from, to };
        self.connections.push(connection);
        Ok(connection)
    }

    /// Connect, replacing whatever was on the input.
    ///
    /// Returns the connection made and the one displaced, because an undo needs
    /// both — this is the operation a user actually performs when they drag a
    /// new wire onto an occupied input, and modelling it as
    /// disconnect-then-connect would make undo two steps for one gesture.
    pub fn reconnect(
        &mut self,
        catalogue: &Catalogue,
        from: PinRef,
        to: PinRef,
    ) -> Result<(Connection, Option<Connection>), ConnectError> {
        if from.direction == to.direction {
            return Err(ConnectError::SameDirection);
        }
        let (out_pin, in_pin) = if from.direction == PinDirection::Output {
            (from, to)
        } else {
            (to, from)
        };
        let displaced_index = self.connections.iter().position(|c| c.to == in_pin);
        // Take it out first, so the occupancy check in `can_connect` passes and
        // every *other* rule still applies.
        let displaced = displaced_index.map(|index| self.connections.remove(index));
        match self.can_connect(catalogue, out_pin, in_pin) {
            Ok(()) => {
                let connection = Connection {
                    from: out_pin,
                    to: in_pin,
                };
                self.connections.push(connection);
                Ok((connection, displaced))
            }
            Err(error) => {
                // Put it back: a refused reconnect must not have destroyed the
                // wire that was already there or changed deterministic order.
                if let (Some(index), Some(previous)) = (displaced_index, displaced) {
                    self.connections.insert(index, previous);
                }
                Err(error)
            }
        }
    }

    /// Remove a connection. Returns whether one was there.
    pub fn disconnect(&mut self, connection: Connection) -> bool {
        let before = self.connections.len();
        self.connections.retain(|c| *c != connection);
        self.connections.len() != before
    }

    /// Whether `from` can reach `to` by following wires forward.
    ///
    /// Iterative rather than recursive: a deep graph is a graph somebody built,
    /// and a stack overflow in the editor is a lost session.
    #[must_use]
    pub fn reaches(&self, from: NodeId, to: NodeId) -> bool {
        if from == to {
            return true;
        }
        let mut seen = HashSet::new();
        let mut stack = vec![from];
        while let Some(node) = stack.pop() {
            if !seen.insert(node) {
                continue;
            }
            for connection in self.connections.iter().filter(|c| c.from.node == node) {
                if connection.to.node == to {
                    return true;
                }
                stack.push(connection.to.node);
            }
        }
        false
    }

    /// Nodes in dependency order: every node after everything it reads.
    ///
    /// **The output every consumer actually wants.** A material graph compiles
    /// in this order, a behaviour tree ticks in it, a VFX graph evaluates in it.
    /// Returns `None` on a cycle — which `can_connect` prevents, so a `None`
    /// here means a graph loaded from a file that was edited by hand or written
    /// by an older version.
    #[must_use]
    pub fn topological_order(&self) -> Option<Vec<NodeId>> {
        let mut in_degree: HashMap<NodeId, usize> = self.nodes.iter().map(|n| (n.id, 0)).collect();
        for connection in &self.connections {
            *in_degree.entry(connection.to.node).or_insert(0) += 1;
        }
        // Seeded in node order rather than from a set, so the result is
        // deterministic — a compiled shader that differs between runs would
        // defeat MORROWIND-Q's content hashing before it is written.
        let mut ready: Vec<NodeId> = self
            .nodes
            .iter()
            .filter(|n| in_degree.get(&n.id) == Some(&0))
            .map(|n| n.id)
            .collect();
        ready.reverse();

        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(node) = ready.pop() {
            order.push(node);
            let mut unlocked: Vec<NodeId> = Vec::new();
            for connection in self.connections.iter().filter(|c| c.from.node == node) {
                if let Some(degree) = in_degree.get_mut(&connection.to.node) {
                    *degree -= 1;
                    if *degree == 0 {
                        unlocked.push(connection.to.node);
                    }
                }
            }
            unlocked.sort_unstable();
            for node in unlocked.into_iter().rev() {
                ready.push(node);
            }
        }
        (order.len() == self.nodes.len()).then_some(order)
    }

    /// What is wired into a node's input, if anything.
    #[must_use]
    pub fn input_source(&self, pin: PinRef) -> Option<PinRef> {
        self.connections
            .iter()
            .find(|c| c.to == pin)
            .map(|c| c.from)
    }

    /// Everything an output feeds.
    #[must_use]
    pub fn output_targets(&self, pin: PinRef) -> Vec<PinRef> {
        self.connections
            .iter()
            .filter(|c| c.from == pin)
            .map(|c| c.to)
            .collect()
    }

    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    #[must_use]
    pub fn connections(&self) -> &[Connection] {
        &self.connections
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Move a set of nodes together.
    ///
    /// Takes a slice rather than one id because dragging a selection is the
    /// operation, and applying it per node would make undo N commands.
    pub fn translate(&mut self, ids: &[NodeId], delta: Vec2) {
        let mut moving: HashSet<NodeId> = ids.iter().copied().collect();
        loop {
            let before = moving.len();
            for node in &self.nodes {
                if node.group.is_some_and(|group| moving.contains(&group)) {
                    moving.insert(node.id);
                }
            }
            if moving.len() == before {
                break;
            }
        }
        for node in self.nodes.iter_mut().filter(|n| moving.contains(&n.id)) {
            node.position += delta;
        }
    }

    /// Put a node in a group. Nested groups are supported; cycles are refused.
    pub fn set_group(&mut self, node: NodeId, group: Option<NodeId>) -> bool {
        if self.node(node).is_none() || group.is_some_and(|id| self.node(id).is_none()) {
            return false;
        }
        let mut cursor = group;
        while let Some(id) = cursor {
            if id == node {
                return false;
            }
            cursor = self.node(id).and_then(|candidate| candidate.group);
        }
        self.node_mut(node).expect("validated above").group = group;
        true
    }

    /// Copy a set of nodes and the wires **between** them.
    ///
    /// Wires to nodes outside the set are dropped, which is what a user means
    /// by copying part of a graph: the fragment has to stand alone or the paste
    /// would connect to whatever happened to have the same id.
    #[must_use]
    pub fn copy(&self, ids: &[NodeId]) -> GraphFragment {
        let set: HashSet<NodeId> = ids.iter().copied().collect();
        GraphFragment {
            nodes: self
                .nodes
                .iter()
                .filter(|n| set.contains(&n.id))
                .cloned()
                .collect(),
            connections: self
                .connections
                .iter()
                .copied()
                .filter(|c| set.contains(&c.from.node) && set.contains(&c.to.node))
                .collect(),
        }
    }

    /// Paste a fragment, offset by `offset`, with fresh ids.
    ///
    /// Returns the new ids in the fragment's order, so the caller can select
    /// what it just pasted — which is what every editor does and what makes
    /// paste-then-drag one gesture.
    pub fn paste(
        &mut self,
        catalogue: &Catalogue,
        fragment: &GraphFragment,
        offset: Vec2,
    ) -> Result<Vec<NodeId>, PasteError> {
        let mut candidate = self.clone();
        let mut remap: HashMap<NodeId, NodeId> = HashMap::new();
        let mut created = Vec::with_capacity(fragment.nodes.len());
        for node in &fragment.nodes {
            let archetype = catalogue
                .get(&node.archetype)
                .ok_or(PasteError::UnknownArchetype)?;
            if node
                .literals
                .keys()
                .any(|index| archetype.inputs.get(*index as usize).is_none())
            {
                return Err(PasteError::UnknownLiteral);
            }
            let id = candidate
                .add(catalogue, &node.archetype, node.position + offset)
                .ok_or(PasteError::UnknownArchetype)?;
            remap.insert(node.id, id);
            created.push(id);
            let created_node = candidate.node_mut(id).expect("the node was just added");
            created_node.title.clone_from(&node.title);
            created_node.literals.clone_from(&node.literals);
            created_node.size = node.size;
        }
        for connection in &fragment.connections {
            let (Some(&from), Some(&to)) = (
                remap.get(&connection.from.node),
                remap.get(&connection.to.node),
            ) else {
                continue;
            };
            candidate
                .connect(
                    catalogue,
                    PinRef {
                        node: from,
                        ..connection.from
                    },
                    PinRef {
                        node: to,
                        ..connection.to
                    },
                )
                .map_err(PasteError::InvalidConnection)?;
        }
        for node in &fragment.nodes {
            if let (Some(&created_id), Some(group)) = (remap.get(&node.id), node.group) {
                let remapped_group = remap.get(&group).copied();
                candidate
                    .node_mut(created_id)
                    .expect("the node was just added")
                    .group = remapped_group;
            }
        }
        *self = candidate;
        Ok(created)
    }

    /// Insert a reroute node on an existing wire.
    ///
    /// One wire becomes two through a pass-through node, which is how a user
    /// routes a long connection around a block of nodes. Returns the reroute's
    /// id, or `None` when the catalogue has no reroute archetype — a catalogue
    /// may legitimately not want them.
    pub fn insert_reroute(
        &mut self,
        catalogue: &Catalogue,
        connection: Connection,
        at: Vec2,
    ) -> Option<NodeId> {
        let archetype = catalogue.reroute()?;
        if !self.connections.contains(&connection) {
            return None;
        }
        let reroute = catalogue.get(archetype)?;
        if reroute.inputs.len() != 1
            || reroute.outputs.len() != 1
            || !reroute.inputs[0].ty.connects_to(reroute.outputs[0].ty)
        {
            return None;
        }

        // Build transactionally: a malformed catalogue must leave the original
        // wire and graph ordering untouched.
        let mut candidate = self.clone();
        candidate.disconnect(connection);
        let id = candidate.add(catalogue, archetype, at)?;
        candidate
            .connect(catalogue, connection.from, PinRef::input(id, 0))
            .ok()?;
        candidate
            .connect(catalogue, PinRef::output(id, 0), connection.to)
            .ok()?;
        *self = candidate;
        Some(id)
    }

    /// Enter a named sub-graph. The path is durable and serialised; the view
    /// decides how to present it as breadcrumbs.
    pub fn enter_context(&mut self, name: impl Into<String>) {
        let name = name.into();
        if !name.trim().is_empty() {
            self.context.push(name);
        }
    }

    /// Leave the current sub-graph and return its name.
    pub fn leave_context(&mut self) -> Option<String> {
        self.context.pop()
    }

    /// Current root-to-leaf sub-graph path.
    #[must_use]
    pub fn context(&self) -> &[String] {
        &self.context
    }
}

/// A detached piece of a graph: what copy produces and paste consumes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphFragment {
    pub nodes: Vec<Node>,
    pub connections: Vec<Connection>,
}

#[cfg(test)]
mod tests;
