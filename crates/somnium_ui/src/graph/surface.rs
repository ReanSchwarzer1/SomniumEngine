//! Stateful editing surface over the feature-neutral graph model.

use glam::Vec2;

use super::{
    Catalogue, ConnectError, Connection, Graph, GraphFragment, GraphView, NodeId, PasteError,
    PinRef,
    geometry::{self, Alignment, GraphSelection},
};

#[derive(Clone)]
struct HistoryEntry {
    label: &'static str,
    before: Graph,
    after: Graph,
}

/// Bounded graph-local history driven by CONTROL's Undo/Redo commands.
#[derive(Clone, Default)]
pub struct GraphHistory {
    entries: Vec<HistoryEntry>,
    cursor: usize,
    capacity: usize,
}

impl GraphHistory {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            ..Self::default()
        }
    }

    pub fn apply(
        &mut self,
        graph: &mut Graph,
        label: &'static str,
        edit: impl FnOnce(&mut Graph) -> bool,
    ) -> bool {
        let before = graph.clone();
        if !edit(graph) || *graph == before {
            *graph = before;
            return false;
        }
        self.entries.truncate(self.cursor);
        self.entries.push(HistoryEntry {
            label,
            before,
            after: graph.clone(),
        });
        if self.entries.len() > self.capacity {
            self.entries.remove(0);
        }
        self.cursor = self.entries.len();
        true
    }

    /// Record an edit that was applied live during a pointer gesture.
    pub fn record(&mut self, before: Graph, graph: &Graph, label: &'static str) -> bool {
        if before == *graph {
            return false;
        }
        self.entries.truncate(self.cursor);
        self.entries.push(HistoryEntry {
            label,
            before,
            after: graph.clone(),
        });
        if self.entries.len() > self.capacity {
            self.entries.remove(0);
        }
        self.cursor = self.entries.len();
        true
    }

    pub fn undo(&mut self, graph: &mut Graph) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        *graph = self.entries[self.cursor].before.clone();
        true
    }

    pub fn redo(&mut self, graph: &mut Graph) -> bool {
        let Some(entry) = self.entries.get(self.cursor) else {
            return false;
        };
        *graph = entry.after.clone();
        self.cursor += 1;
        true
    }

    #[must_use]
    pub fn labels(&self) -> Vec<&'static str> {
        self.entries.iter().map(|entry| entry.label).collect()
    }

    #[must_use]
    pub fn position(&self) -> usize {
        self.cursor
    }
}

/// One reusable graph editing surface. Consumers contribute only a catalogue.
#[derive(Clone)]
pub struct GraphSurface {
    pub graph: Graph,
    pub catalogue: Catalogue,
    pub view: GraphView,
    pub selection: GraphSelection,
    history: GraphHistory,
    clipboard: GraphFragment,
}

impl GraphSurface {
    #[must_use]
    pub fn new(catalogue: Catalogue) -> Self {
        Self {
            graph: Graph::new(),
            catalogue,
            view: GraphView::default(),
            selection: GraphSelection::default(),
            history: GraphHistory::new(128),
            clipboard: GraphFragment::default(),
        }
    }

    #[must_use]
    pub fn palette(&self, query: &str) -> Vec<&super::NodeArchetype> {
        self.catalogue.search(query)
    }

    pub fn add(&mut self, archetype: &str, at: Vec2) -> Option<NodeId> {
        let mut created = None;
        self.history
            .apply(&mut self.graph, "Add Graph Node", |graph| {
                created = graph.add(&self.catalogue, archetype, at);
                created.is_some()
            });
        if let Some(id) = created {
            self.selection.select_only(id);
        }
        created
    }

    /// Commit one unconnected input's authored value as a single undo step.
    ///
    /// Literal text deliberately remains feature-neutral here: the catalogue
    /// owner validates it when compiling the graph. Connected inputs cannot be
    /// edited because their value is supplied by the wire.
    pub fn set_literal(&mut self, node: NodeId, pin: u16, value: impl Into<String>) -> bool {
        let value = value.into();
        let input = self
            .graph
            .node(node)
            .and_then(|node| self.catalogue.get(&node.archetype))
            .and_then(|archetype| archetype.inputs.get(pin as usize));
        let valid = input.is_some()
            && input.is_none_or(|input| {
                input.range.is_none_or(|(min, max)| {
                    let Ok(value) = value.trim().parse::<f64>() else {
                        return false;
                    };
                    let (Ok(min), Ok(max)) = (min.parse::<f64>(), max.parse::<f64>()) else {
                        return false;
                    };
                    value.is_finite() && value >= min && value <= max
                })
            })
            && self.graph.input_source(PinRef::input(node, pin)).is_none();
        if !valid {
            return false;
        }
        self.history
            .apply(&mut self.graph, "Edit Graph Literal", |graph| {
                let Some(node) = graph.node_mut(node) else {
                    return false;
                };
                if node.literals.get(&pin) == Some(&value) {
                    return false;
                }
                node.literals.insert(pin, value);
                true
            })
    }

    pub fn add_comment(&mut self, at: Vec2, size: Vec2, text: impl Into<String>) -> Option<NodeId> {
        let text = text.into();
        let mut created = None;
        self.history
            .apply(&mut self.graph, "Add Graph Comment", |graph| {
                let Some(id) = graph.add(&self.catalogue, "graph.comment", at) else {
                    return false;
                };
                let node = graph.node_mut(id).expect("just added");
                node.size = Some(size.max(Vec2::splat(1.0)));
                node.title.clone_from(&text);
                created = Some(id);
                true
            });
        if let Some(id) = created {
            self.selection.select_only(id);
        }
        created
    }

    pub fn connect(&mut self, from: PinRef, to: PinRef) -> Result<Connection, ConnectError> {
        let mut result = Err(ConnectError::UnknownNode);
        self.history
            .apply(&mut self.graph, "Connect Graph Pins", |graph| {
                result = graph.connect(&self.catalogue, from, to);
                result.is_ok()
            });
        result
    }

    /// Replace an occupied input as one authoring gesture.
    pub fn reconnect(
        &mut self,
        from: PinRef,
        to: PinRef,
    ) -> Result<(Connection, Option<Connection>), ConnectError> {
        let mut result = Err(ConnectError::UnknownNode);
        self.history
            .apply(&mut self.graph, "Reconnect Graph Pins", |graph| {
                result = graph.reconnect(&self.catalogue, from, to);
                result.is_ok()
            });
        result
    }

    /// Remove one wire as one authoring gesture.
    pub fn disconnect(&mut self, connection: Connection) -> bool {
        self.history
            .apply(&mut self.graph, "Disconnect Graph Pins", |graph| {
                graph.disconnect(connection)
            })
    }

    /// Split a wire with the catalogue's typed reroute node.
    pub fn insert_reroute(&mut self, connection: Connection, at: Vec2) -> Option<NodeId> {
        let mut reroute = None;
        self.history
            .apply(&mut self.graph, "Insert Graph Reroute", |graph| {
                reroute = graph.insert_reroute(&self.catalogue, connection, at);
                reroute.is_some()
            });
        if let Some(id) = reroute {
            self.selection.select_only(id);
        }
        reroute
    }

    pub fn move_selection(&mut self, delta: Vec2) -> bool {
        let ids = self.selection.ids();
        self.history
            .apply(&mut self.graph, "Move Graph Nodes", |graph| {
                if ids.is_empty() || !delta.is_finite() || delta == Vec2::ZERO {
                    return false;
                }
                graph.translate(&ids, delta);
                true
            })
    }

    pub fn align_selection(&mut self, alignment: Alignment) -> bool {
        let ids = self.selection.ids();
        self.history
            .apply(&mut self.graph, "Align Graph Nodes", |graph| {
                geometry::align_nodes(graph, &self.catalogue, &ids, alignment)
            })
    }

    pub fn delete_selection(&mut self) -> bool {
        let ids = self.selection.ids();
        let changed = self
            .history
            .apply(&mut self.graph, "Delete Graph Nodes", |graph| {
                if ids.is_empty() {
                    return false;
                }
                for id in &ids {
                    graph.remove(*id);
                }
                true
            });
        if changed {
            self.selection.clear();
        }
        changed
    }

    pub fn copy(&mut self) {
        self.clipboard = self.graph.copy(&self.selection.ids());
    }

    pub fn paste(&mut self, offset: Vec2) -> Result<Vec<NodeId>, PasteError> {
        let fragment = self.clipboard.clone();
        let mut result = Ok(Vec::new());
        self.history
            .apply(&mut self.graph, "Paste Graph Nodes", |graph| {
                result = graph.paste(&self.catalogue, &fragment, offset);
                result.as_ref().is_ok_and(|ids| !ids.is_empty())
            });
        if let Ok(ids) = &result {
            self.selection.clear();
            for id in ids {
                self.selection.toggle(*id);
            }
        }
        result
    }

    pub fn group_selection(&mut self, at: Vec2, size: Vec2) -> Option<NodeId> {
        let members = self.selection.ids();
        let mut group = None;
        self.history
            .apply(&mut self.graph, "Group Graph Nodes", |graph| {
                let Some(id) = graph.add(&self.catalogue, "graph.group", at) else {
                    return false;
                };
                graph.node_mut(id).expect("just added").size = Some(size.max(Vec2::splat(1.0)));
                if members
                    .iter()
                    .any(|member| !graph.set_group(*member, Some(id)))
                {
                    return false;
                }
                group = Some(id);
                true
            });
        if let Some(id) = group {
            self.selection.select_only(id);
        }
        group
    }

    pub fn undo(&mut self) -> bool {
        let changed = self.history.undo(&mut self.graph);
        if changed {
            self.selection.clear();
        }
        changed
    }

    pub fn redo(&mut self) -> bool {
        let changed = self.history.redo(&mut self.graph);
        if changed {
            self.selection.clear();
        }
        changed
    }

    /// Route CONTROL-A2's shared Edit commands into the active graph surface.
    ///
    /// The command registry remains the only source of shortcuts, menu labels,
    /// and palette entries; this method only supplies the active document's
    /// semantics. Unknown command ids are left for the rest of the editor.
    pub fn dispatch_command(&mut self, command: &str, paste_offset: Vec2) -> bool {
        match command {
            "editor.edit.undo" => self.undo(),
            "editor.edit.redo" => self.redo(),
            "editor.edit.copy" => {
                if self.selection.ids().is_empty() {
                    false
                } else {
                    self.copy();
                    true
                }
            }
            "editor.edit.paste" => self
                .paste(paste_offset)
                .is_ok_and(|created| !created.is_empty()),
            "editor.edit.delete" => self.delete_selection(),
            _ => false,
        }
    }

    /// Close a live graph gesture as exactly one undo entry.
    pub fn commit_gesture(&mut self, before: Graph, label: &'static str) -> bool {
        self.history.record(before, &self.graph, label)
    }

    /// Enter a named nested graph and expose it through the breadcrumb path.
    pub fn enter_context(&mut self, name: impl Into<String>) {
        self.graph.enter_context(name);
    }

    /// Leave the current nested graph.
    pub fn leave_context(&mut self) -> Option<String> {
        self.graph.leave_context()
    }

    #[must_use]
    pub fn history(&self) -> &GraphHistory {
        &self.history
    }
}
