//! Validated immutable behavior assets and agent-owned execution state.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Bool(bool),
    Number(f64),
    Text(String),
    Position([f32; 3]),
    Entity(u64),
}
impl Value {
    fn valid(&self) -> bool {
        match self {
            Self::Number(n) => n.is_finite(),
            Self::Position(p) => p.iter().all(|n| n.is_finite()),
            _ => true,
        }
    }
}
pub type Blackboard = BTreeMap<String, Value>;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Success,
    Failure,
    Running,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Task {
    Wait {
        seconds: f32,
    },
    Set {
        key: String,
        value: Value,
    },
    Check {
        key: String,
        value: Value,
    },
    /// Named native task resolved by the simulation owner.
    Native(String),
    /// Named script attachment resolved through the existing ScriptBackend.
    Script(String),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Node {
    Sequence(Vec<usize>),
    Selector(Vec<usize>),
    Parallel {
        children: Vec<usize>,
        successes: usize,
    },
    Invert(usize),
    Repeat {
        child: usize,
        count: u32,
    },
    Timeout {
        child: usize,
        seconds: f32,
    },
    Task(Task),
}
impl Node {
    fn children(&self) -> &[usize] {
        match self {
            Self::Sequence(c) | Self::Selector(c) | Self::Parallel { children: c, .. } => c,
            Self::Invert(c) | Self::Repeat { child: c, .. } | Self::Timeout { child: c, .. } => {
                std::slice::from_ref(c)
            }
            Self::Task(_) => &[],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BehaviorTree {
    version: u32,
    root: usize,
    nodes: Vec<Node>,
}
impl BehaviorTree {
    pub fn new(root: usize, nodes: Vec<Node>) -> Result<Self, String> {
        let tree = Self {
            version: 1,
            root,
            nodes,
        };
        tree.validate()?;
        Ok(tree)
    }
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }
    pub fn root(&self) -> usize {
        self.root
    }
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(self).map_err(|e| e.to_string())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("behavior asset exceeds limit".into());
        }
        let tree: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        tree.validate()?;
        Ok(tree)
    }
    fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || self.nodes.is_empty()
            || self.nodes.len() > 4096
            || self.root >= self.nodes.len()
        {
            return Err("invalid behavior tree header".into());
        }
        let mut parents = vec![0; self.nodes.len()];
        for node in &self.nodes {
            match node {
                Node::Sequence(c) | Node::Selector(c) if c.is_empty() => {
                    return Err("empty behavior composite".into());
                }
                Node::Parallel {
                    children,
                    successes,
                } if *successes == 0 || *successes > children.len() => {
                    return Err("invalid parallel threshold".into());
                }
                Node::Timeout { seconds, .. } | Node::Task(Task::Wait { seconds })
                    if !seconds.is_finite() || *seconds < 0.0 =>
                {
                    return Err("invalid behavior duration".into());
                }
                Node::Repeat { count, .. } if *count == 0 => {
                    return Err("zero behavior repeat count".into());
                }
                Node::Task(Task::Set { key, value } | Task::Check { key, value })
                    if key.is_empty() || !value.valid() =>
                {
                    return Err("invalid blackboard literal".into());
                }
                Node::Task(Task::Native(name) | Task::Script(name)) if name.is_empty() => {
                    return Err("empty task name".into());
                }
                _ => {}
            }
            for &c in node.children() {
                let Some(count) = parents.get_mut(c) else {
                    return Err("missing behavior child".into());
                };
                *count += 1;
                if *count > 1 {
                    return Err("behavior nodes cannot have multiple parents".into());
                }
            }
        }
        if parents[self.root] != 0 {
            return Err("behavior root has a parent".into());
        }
        let mut seen = BTreeSet::new();
        let mut stack = vec![(self.root, 0)];
        while let Some((i, depth)) = stack.pop() {
            if depth > 128 || !seen.insert(i) {
                return Err("behavior cycle or excessive depth".into());
            }
            stack.extend(self.nodes[i].children().iter().map(|&c| (c, depth + 1)));
        }
        if seen.len() != self.nodes.len() {
            return Err("unreachable behavior nodes".into());
        }
        Ok(())
    }
}

/// Native and script tasks share lifecycle semantics. A running task receives
/// `cancel` exactly once if a timeout, parallel completion or owner reset aborts it.
pub trait TaskHost {
    fn tick(&mut self, script: bool, name: &str, blackboard: &mut Blackboard, delta: f32)
    -> Status;
    fn cancel(&mut self, _script: bool, _name: &str, _blackboard: &mut Blackboard) {}
}
#[derive(Clone, Debug, Default)]
struct Memory {
    cursor: usize,
    elapsed: f32,
    repeats: u32,
    status: Option<Status>,
}
#[derive(Clone, Debug)]
pub struct BehaviorInstance {
    tree: BehaviorTree,
    memory: Vec<Memory>,
    pub blackboard: Blackboard,
}
impl BehaviorInstance {
    pub fn new(tree: BehaviorTree) -> Self {
        Self {
            memory: vec![Memory::default(); tree.nodes.len()],
            tree,
            blackboard: Blackboard::new(),
        }
    }
    pub fn tree(&self) -> &BehaviorTree {
        &self.tree
    }
    pub fn status(&self, node: usize) -> Option<Status> {
        self.memory.get(node).and_then(|m| m.status)
    }
    /// Terminal trees remain terminal until reset. This prevents a completed
    /// Set or script task from silently executing again on the following frame.
    pub fn tick(&mut self, delta: f32, host: &mut impl TaskHost) -> Status {
        if !delta.is_finite() || delta < 0.0 {
            return Status::Failure;
        }
        self.tick_node(self.tree.root, delta, host)
    }
    pub fn reset(&mut self, host: &mut impl TaskHost) {
        self.reset_node(self.tree.root, host);
    }
    fn reset_node(&mut self, id: usize, host: &mut impl TaskHost) {
        let node = self.tree.nodes[id].clone();
        if self.memory[id].status == Some(Status::Running) {
            match &node {
                Node::Task(Task::Native(name)) => host.cancel(false, name, &mut self.blackboard),
                Node::Task(Task::Script(name)) => host.cancel(true, name, &mut self.blackboard),
                _ => {}
            }
        }
        for &child in node.children() {
            self.reset_node(child, host);
        }
        self.memory[id] = Memory::default();
    }
    fn tick_node(&mut self, id: usize, dt: f32, host: &mut impl TaskHost) -> Status {
        if let Some(s @ (Status::Success | Status::Failure)) = self.memory[id].status {
            return s;
        }
        let node = self.tree.nodes[id].clone();
        let result = match node {
            Node::Sequence(children) | Node::Selector(children) => {
                let sequence = matches!(self.tree.nodes[id], Node::Sequence(_));
                let mut result = if sequence {
                    Status::Success
                } else {
                    Status::Failure
                };
                while self.memory[id].cursor < children.len() {
                    let child = children[self.memory[id].cursor];
                    let s = self.tick_node(child, dt, host);
                    if s == Status::Running
                        || (sequence && s == Status::Failure)
                        || (!sequence && s == Status::Success)
                    {
                        result = s;
                        break;
                    }
                    self.memory[id].cursor += 1;
                }
                result
            }
            Node::Parallel {
                children,
                successes,
            } => {
                let states: Vec<_> = children
                    .iter()
                    .map(|&c| self.tick_node(c, dt, host))
                    .collect();
                let succeeded = states.iter().filter(|&&s| s == Status::Success).count();
                let failed = states.iter().filter(|&&s| s == Status::Failure).count();
                let result = if succeeded >= successes {
                    Status::Success
                } else if failed > children.len() - successes {
                    Status::Failure
                } else {
                    Status::Running
                };
                if result != Status::Running {
                    for (i, &c) in children.iter().enumerate() {
                        if states[i] == Status::Running {
                            self.reset_node(c, host);
                        }
                    }
                }
                result
            }
            Node::Invert(child) => match self.tick_node(child, dt, host) {
                Status::Success => Status::Failure,
                Status::Failure => Status::Success,
                Status::Running => Status::Running,
            },
            Node::Repeat { child, count } => match self.tick_node(child, dt, host) {
                Status::Success => {
                    self.memory[id].repeats += 1;
                    if self.memory[id].repeats >= count {
                        Status::Success
                    } else {
                        self.reset_node(child, host);
                        Status::Running
                    }
                }
                s => s,
            },
            Node::Timeout { child, seconds } => {
                self.memory[id].elapsed += dt;
                if self.memory[id].elapsed >= seconds {
                    self.reset_node(child, host);
                    Status::Failure
                } else {
                    self.tick_node(child, dt, host)
                }
            }
            Node::Task(task) => match task {
                Task::Wait { seconds } => {
                    self.memory[id].elapsed += dt;
                    if self.memory[id].elapsed >= seconds {
                        Status::Success
                    } else {
                        Status::Running
                    }
                }
                Task::Set { key, value } => {
                    self.blackboard.insert(key, value);
                    Status::Success
                }
                Task::Check { key, value } => {
                    if self.blackboard.get(&key) == Some(&value) {
                        Status::Success
                    } else {
                        Status::Failure
                    }
                }
                Task::Native(name) => host.tick(false, &name, &mut self.blackboard, dt),
                Task::Script(name) => host.tick(true, &name, &mut self.blackboard, dt),
            },
        };
        self.memory[id].status = Some(result);
        result
    }
}
