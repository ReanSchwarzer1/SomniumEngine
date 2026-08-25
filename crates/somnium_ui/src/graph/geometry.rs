//! View transforms and deterministic layout for the generic graph surface.

use super::{Catalogue, Connection, Graph, Node, NodeId, PinRef, PinType};
use crate::{path::Path, types::Rect};
use glam::Vec2;
use std::collections::BTreeSet;

const NODE_WIDTH: f32 = 180.0;
const NODE_HEADER: f32 = 28.0;
const PIN_ROW: f32 = 24.0;
const NODE_PADDING: f32 = 8.0;
const REROUTE_SIZE: Vec2 = Vec2::new(32.0, 24.0);

/// Pan and zoom from unbounded graph space into logical screen pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphView {
    pub pan: Vec2,
    pub zoom: f32,
}

impl Default for GraphView {
    fn default() -> Self {
        Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

impl GraphView {
    pub const MIN_ZOOM: f32 = 0.1;
    pub const MAX_ZOOM: f32 = 4.0;

    #[must_use]
    pub fn graph_to_screen(self, point: Vec2) -> Vec2 {
        point * self.zoom + self.pan
    }

    #[must_use]
    pub fn screen_to_graph(self, point: Vec2) -> Vec2 {
        (point - self.pan) / self.zoom
    }

    /// Pan by a screen-space pointer delta.
    pub fn pan_by(&mut self, delta: Vec2) {
        if delta.is_finite() {
            self.pan += delta;
        }
    }

    /// Zoom about a screen-space anchor without moving the graph point under
    /// the pointer.
    pub fn zoom_at(&mut self, anchor: Vec2, factor: f32) {
        if !anchor.is_finite() || !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let graph_anchor = self.screen_to_graph(anchor);
        self.zoom = (self.zoom * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        self.pan = anchor - graph_anchor * self.zoom;
    }
}

/// One laid-out pin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PinLayout {
    pub pin: PinRef,
    pub position: Vec2,
    pub ty: PinType,
}

/// Geometry for one node in graph space.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeLayout {
    pub node: NodeId,
    pub bounds: Rect,
    pub pins: Vec<PinLayout>,
}

impl NodeLayout {
    #[must_use]
    pub fn pin(&self, pin: PinRef) -> Option<&PinLayout> {
        self.pins.iter().find(|layout| layout.pin == pin)
    }
}

/// Lay out every node in stable graph order.
#[must_use]
pub fn layout_nodes(graph: &Graph, catalogue: &Catalogue) -> Vec<NodeLayout> {
    graph
        .nodes()
        .iter()
        .filter_map(|node| layout_node(node, catalogue))
        .collect()
}

/// Lay out one node from archetype data.
#[must_use]
pub fn layout_node(node: &Node, catalogue: &Catalogue) -> Option<NodeLayout> {
    let archetype = catalogue.get(&node.archetype)?;
    let rows = archetype
        .inputs
        .len()
        .max(archetype.outputs.len())
        .max(archetype.elements.len())
        .max(1);
    let default_size = if archetype.is_reroute {
        REROUTE_SIZE
    } else {
        Vec2::new(
            NODE_WIDTH,
            NODE_HEADER + NODE_PADDING * 2.0 + rows as f32 * PIN_ROW,
        )
    };
    let size = node.size.unwrap_or(default_size).max(Vec2::splat(1.0));
    let bounds = Rect::from_pos_size(node.position, size);
    let first_y = if archetype.is_reroute {
        node.position.y + size.y * 0.5
    } else {
        node.position.y + NODE_HEADER + NODE_PADDING + PIN_ROW * 0.5
    };
    let mut pins = Vec::with_capacity(archetype.inputs.len() + archetype.outputs.len());
    for (index, pin) in archetype.inputs.iter().enumerate() {
        pins.push(PinLayout {
            pin: PinRef::input(node.id, index as u16),
            position: Vec2::new(node.position.x, first_y + index as f32 * PIN_ROW),
            ty: pin.ty,
        });
    }
    for (index, pin) in archetype.outputs.iter().enumerate() {
        pins.push(PinLayout {
            pin: PinRef::output(node.id, index as u16),
            position: Vec2::new(node.position.x + size.x, first_y + index as f32 * PIN_ROW),
            ty: pin.ty,
        });
    }
    Some(NodeLayout {
        node: node.id,
        bounds,
        pins,
    })
}

/// Cubic wire geometry between two laid-out pins.
#[must_use]
pub fn wire_path(layouts: &[NodeLayout], connection: Connection) -> Option<Path> {
    let from = layouts
        .iter()
        .find_map(|layout| layout.pin(connection.from))?;
    let to = layouts
        .iter()
        .find_map(|layout| layout.pin(connection.to))?;
    Some(Path::wire(from.position, to.position))
}

/// Current graph selection. It is editor state, not graph-asset state, so it is
/// deliberately absent from serialisation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphSelection {
    selected: BTreeSet<NodeId>,
}

impl GraphSelection {
    #[must_use]
    pub fn contains(&self, id: NodeId) -> bool {
        self.selected.contains(&id)
    }

    #[must_use]
    pub fn ids(&self) -> Vec<NodeId> {
        self.selected.iter().copied().collect()
    }

    pub fn clear(&mut self) {
        self.selected.clear();
    }

    pub fn select_only(&mut self, id: NodeId) {
        self.selected.clear();
        self.selected.insert(id);
    }

    pub fn toggle(&mut self, id: NodeId) {
        if !self.selected.remove(&id) {
            self.selected.insert(id);
        }
    }

    /// Replace the selection with nodes whose bounds intersect a graph-space
    /// drag rectangle. Dragging in any direction is accepted.
    pub fn select_box(&mut self, layouts: &[NodeLayout], rect: Rect) {
        let rect = normalise_rect(rect);
        self.selected = layouts
            .iter()
            .filter(|node| overlaps(node.bounds, rect))
            .map(|node| node.node)
            .collect();
    }
}

/// Node alignment operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    Left,
    CentreX,
    Right,
    Top,
    CentreY,
    Bottom,
}

/// Align nodes to the first selected node. Returns false when fewer than two
/// valid nodes were supplied.
pub fn align_nodes(
    graph: &mut Graph,
    catalogue: &Catalogue,
    ids: &[NodeId],
    alignment: Alignment,
) -> bool {
    let layouts = layout_nodes(graph, catalogue);
    let selected: Vec<_> = ids
        .iter()
        .filter_map(|id| layouts.iter().find(|layout| layout.node == *id))
        .collect();
    let Some(anchor) = selected.first() else {
        return false;
    };
    if selected.len() < 2 {
        return false;
    }
    let target = edge(anchor.bounds, alignment);
    let moves: Vec<_> = selected
        .iter()
        .skip(1)
        .map(|layout| (layout.node, target - edge(layout.bounds, alignment)))
        .collect();
    for (id, delta) in moves {
        let Some(node) = graph.node_mut(id) else {
            continue;
        };
        match alignment {
            Alignment::Left | Alignment::CentreX | Alignment::Right => node.position.x += delta,
            Alignment::Top | Alignment::CentreY | Alignment::Bottom => node.position.y += delta,
        }
    }
    true
}

fn edge(rect: Rect, alignment: Alignment) -> f32 {
    match alignment {
        Alignment::Left => rect.x,
        Alignment::CentreX => rect.x + rect.w * 0.5,
        Alignment::Right => rect.x + rect.w,
        Alignment::Top => rect.y,
        Alignment::CentreY => rect.y + rect.h * 0.5,
        Alignment::Bottom => rect.y + rect.h,
    }
}

fn normalise_rect(rect: Rect) -> Rect {
    let (x, w) = if rect.w < 0.0 {
        (rect.x + rect.w, -rect.w)
    } else {
        (rect.x, rect.w)
    };
    let (y, h) = if rect.h < 0.0 {
        (rect.y + rect.h, -rect.h)
    } else {
        (rect.y, rect.h)
    };
    Rect::new(x, y, w, h)
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x <= b.x + b.w && a.x + a.w >= b.x && a.y <= b.y + b.h && a.y + a.h >= b.y
}
