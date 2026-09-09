//! MORROWIND-Y behavior authoring on MORROWIND-K's single graph surface.
use super::{Catalogue, Graph, GraphSurface, NodeArchetype, NodeId, PinArchetype, PinRef, PinType};
use glam::Vec2;
use somnium_ai::behavior::{BehaviorTree, Node as RuntimeNode, Task, Value};
use std::collections::{BTreeMap, BTreeSet};
const FLOW: PinType = PinType::Opaque("behavior.flow");

pub fn catalogue() -> Catalogue {
    let mut c = Catalogue::new("somnium.behavior");
    c.register(
        NodeArchetype::new("behavior.root", "Behavior Root", "Behavior")
            .with_input(PinArchetype::new("Tree", FLOW))
            .as_root(),
    );
    for (id, title) in [
        ("behavior.sequence", "Sequence"),
        ("behavior.selector", "Selector"),
        ("behavior.parallel", "Parallel (all)"),
    ] {
        let mut node = NodeArchetype::new(id, title, "Composite")
            .with_output(PinArchetype::new("Behavior", FLOW));
        for name in ["First", "Second", "Third", "Fourth"] {
            node = node.with_input(PinArchetype::new(name, FLOW));
        }
        c.register(node);
    }
    c.register(
        NodeArchetype::new("behavior.invert", "Invert", "Decorator")
            .with_input(PinArchetype::new("Child", FLOW))
            .with_output(PinArchetype::new("Behavior", FLOW)),
    );
    for (id, title, name, value) in [
        ("behavior.timeout", "Timeout", "Seconds", "5"),
        ("behavior.repeat", "Repeat", "Count", "2"),
    ] {
        c.register(
            NodeArchetype::new(id, title, "Decorator")
                .with_input(PinArchetype::new("Child", FLOW))
                .with_input(PinArchetype::new(name, PinType::Float).with_default(value))
                .with_output(PinArchetype::new("Behavior", FLOW)),
        );
    }
    c.register(
        NodeArchetype::new("behavior.wait", "Wait", "Task")
            .with_input(PinArchetype::new("Seconds", PinType::Float).with_default("1"))
            .with_output(PinArchetype::new("Behavior", FLOW)),
    );
    for (id, title) in [
        ("behavior.native", "Native Task"),
        ("behavior.script", "Luau Task"),
    ] {
        c.register(
            NodeArchetype::new(id, title, "Task")
                .with_input(
                    PinArchetype::new(
                        if id == "behavior.script" {
                            "Attached script name / path"
                        } else {
                            "Task name"
                        },
                        PinType::Opaque("behavior.name"),
                    )
                    .with_default(if id == "behavior.script" {
                        "morrowind_ai_patrol"
                    } else {
                        "succeed"
                    }),
                )
                .with_output(PinArchetype::new("Behavior", FLOW)),
        );
    }
    for (id, title) in [
        ("behavior.set", "Set Blackboard"),
        ("behavior.check", "Check Blackboard"),
    ] {
        c.register(
            NodeArchetype::new(id, title, "Blackboard")
                .with_input(
                    PinArchetype::new("Key", PinType::Opaque("behavior.name"))
                        .with_default("alert"),
                )
                .with_input(
                    PinArchetype::new("Value (JSON)", PinType::Opaque("behavior.value"))
                        .with_default("true"),
                )
                .with_output(PinArchetype::new("Behavior", FLOW)),
        );
    }
    c
}

pub fn default_surface() -> GraphSurface {
    let c = catalogue();
    let mut g = Graph::new();
    let wait = g.add(&c, "behavior.wait", Vec2::new(40.0, 80.0)).unwrap();
    let root = g.add(&c, "behavior.root", Vec2::new(350.0, 80.0)).unwrap();
    g.connect(&c, PinRef::output(wait, 0), PinRef::input(root, 0))
        .unwrap();
    let mut surface = GraphSurface::new(c);
    surface.graph = g;
    surface
}

/// Compile only the connected tree. Wire input order is execution order;
/// shared children are refused by the runtime's single-parent validation.
pub fn compile(graph: &Graph) -> Result<BehaviorTree, String> {
    let roots: Vec<_> = graph
        .nodes()
        .iter()
        .filter(|n| n.archetype == "behavior.root")
        .collect();
    if roots.len() != 1 {
        return Err("behavior graph needs exactly one root".into());
    }
    let child = graph
        .connections()
        .iter()
        .find(|w| w.to == PinRef::input(roots[0].id, 0))
        .ok_or("behavior root has no tree")?
        .from
        .node;
    let c = catalogue();
    let mut nodes = Vec::new();
    let mut seen = BTreeSet::new();
    let mut indices = BTreeMap::new();
    fn visit(
        id: NodeId,
        graph: &Graph,
        c: &Catalogue,
        nodes: &mut Vec<RuntimeNode>,
        seen: &mut BTreeSet<NodeId>,
        indices: &mut BTreeMap<NodeId, usize>,
    ) -> Result<usize, String> {
        if let Some(&index) = indices.get(&id) {
            return Ok(index);
        }
        if !seen.insert(id) || seen.len() > 128 {
            return Err("behavior graph cycle or excessive size".into());
        }
        let n = graph.node(id).ok_or("missing behavior node")?;
        let mut children: Vec<_> = graph
            .connections()
            .iter()
            .filter(|w| w.to.node == id)
            .collect();
        children.sort_by_key(|w| w.to.index);
        let child_ids: Vec<_> = children
            .iter()
            .map(|w| visit(w.from.node, graph, c, nodes, seen, indices))
            .collect::<Result<_, _>>()?;
        let literal = |pin: u16| -> Result<String, String> {
            n.literals
                .get(&pin)
                .cloned()
                .or_else(|| {
                    c.get(&n.archetype)?
                        .inputs
                        .get(pin as usize)?
                        .default
                        .map(str::to_owned)
                })
                .ok_or_else(|| format!("missing literal on {} pin {pin}", n.archetype))
        };
        let child = || {
            child_ids
                .first()
                .copied()
                .ok_or_else(|| "decorator needs a child".to_string())
        };
        let number = |pin| {
            literal(pin)?
                .parse::<f32>()
                .map_err(|_| "invalid numeric behavior literal".to_string())
        };
        let node = match n.archetype.as_str() {
            "behavior.sequence" => RuntimeNode::Sequence(child_ids),
            "behavior.selector" => RuntimeNode::Selector(child_ids),
            "behavior.parallel" => RuntimeNode::Parallel {
                successes: child_ids.len(),
                children: child_ids,
            },
            "behavior.invert" => RuntimeNode::Invert(child()?),
            "behavior.timeout" => RuntimeNode::Timeout {
                child: child()?,
                seconds: number(1)?,
            },
            "behavior.repeat" => RuntimeNode::Repeat {
                child: child()?,
                count: literal(1)?
                    .parse::<u32>()
                    .map_err(|_| "repeat count must be a positive integer")?,
            },
            "behavior.wait" => RuntimeNode::Task(Task::Wait {
                seconds: number(0)?,
            }),
            "behavior.native" => RuntimeNode::Task(Task::Native(literal(0)?)),
            "behavior.script" => RuntimeNode::Task(Task::Script(literal(0)?)),
            "behavior.set" | "behavior.check" => {
                let key = literal(0)?;
                let value = parse_value(&literal(1)?)?;
                RuntimeNode::Task(if n.archetype == "behavior.set" {
                    Task::Set { key, value }
                } else {
                    Task::Check { key, value }
                })
            }
            _ => return Err(format!("unsupported behavior node {}", n.archetype)),
        };
        let index = nodes.len();
        nodes.push(node);
        indices.insert(id, index);
        seen.remove(&id);
        Ok(index)
    }
    let root = visit(child, graph, &c, &mut nodes, &mut seen, &mut indices)?;
    BehaviorTree::new(root, nodes)
}
fn parse_value(text: &str) -> Result<Value, String> {
    match serde_json::from_str::<serde_json::Value>(text).map_err(|e| e.to_string())? {
        serde_json::Value::Bool(v) => Ok(Value::Bool(v)),
        serde_json::Value::Number(v) => {
            v.as_f64().map(Value::Number).ok_or("invalid number".into())
        }
        serde_json::Value::String(v) => Ok(Value::Text(v)),
        serde_json::Value::Array(a) if a.len() == 3 => {
            let mut p = [0.0; 3];
            for (i, n) in a.iter().enumerate() {
                p[i] = n.as_f64().ok_or("position requires three numbers")? as f32;
            }
            Ok(Value::Position(p))
        }
        _ => Err(
            "blackboard value must be a boolean, number, string or three-number position".into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_ai::behavior::{BehaviorInstance, Blackboard, Status, TaskHost};
    struct Host;
    impl TaskHost for Host {
        fn tick(&mut self, _: bool, _: &str, _: &mut Blackboard, _: f32) -> Status {
            Status::Failure
        }
    }
    #[test]
    fn authored_tree_round_trips_and_executes() {
        let surface = default_surface();
        let json = super::super::serial::to_json(&surface.graph, &surface.catalogue).unwrap();
        let graph = super::super::serial::from_json(&json, &catalogue()).unwrap();
        let tree = compile(&graph).unwrap();
        let mut instance = BehaviorInstance::new(tree);
        assert_eq!(instance.tick(0.5, &mut Host), Status::Running);
        assert_eq!(instance.tick(0.5, &mut Host), Status::Success);
    }
    #[test]
    fn invalid_task_literal_cannot_compile() {
        let mut surface = default_surface();
        let id = surface.graph.nodes()[0].id;
        surface
            .graph
            .node_mut(id)
            .unwrap()
            .literals
            .insert(0, "NaN".into());
        assert!(compile(&surface.graph).is_err());
    }
}
