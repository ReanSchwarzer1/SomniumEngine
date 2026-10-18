//! Dialogue graph assets (`*.somdialogue`): lines, choices, branches, actions, calls, jumps and ends.
//!
//! The asset is the source of truth, in plain JSON a writer can diff and an agent can write; the editor's graph is a
//! view of it (`somnium_ui::graph::dialogue`) and saves back to it. Node ids are authored and stable, because they
//! are what saves remember (`visited`), what string keys are made from and what other dialogues jump to.
//!
//! # How a conversation moves
//!
//! Every node has a `next` list. Following it:
//!
//! - **all targets are options**: a menu. The options are offered in list order (the editor orders them top to
//!   bottom as drawn), each hidden, shown locked or offered by its condition and check;
//! - **otherwise**: the first target whose condition holds is entered (a line's `when`; every other kind of node
//!   always qualifies, and a branch decides by its own condition once entered). This is how a line varies with
//!   the world without a branch node;
//! - **empty**, or nothing qualifies: the dialogue ends (or returns to its caller).
//!
//! Lines and menus are the only places a conversation waits for the player; actions may also wait for the game
//! (`wait`, for a camera move or an animation). Everything else runs straight through.

use crate::expr::{Condition, Context, Effects, ExprError};
use crate::problem::{Problem, has_errors};
use crate::text::LocalText;
use crate::value::{Value, valid_id};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The dialogue format this build reads.
pub const DIALOGUE_VERSION: u32 = 1;

fn is_false(value: &bool) -> bool {
    !*value
}

/// A spoken line.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineDef {
    pub id: String,
    /// A cast id. Empty: the narrator (no name plate, no portrait).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub speaker: String,
    /// The portrait expression. Empty: the speaker's default. Inline `[expr=angry]` changes it mid-line.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub expression: String,
    pub text: LocalText,
    /// A voice clip, project-relative. Empty: text only.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub voice: String,
    /// The language spoken. Empty: the speaker's. The game's fluency transform reads it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub language: String,
    /// Entered only when this holds (see the module docs).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub when: String,
    /// Applied as the line is shown.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub effects: String,
    /// Free tags for the game and the presenter (`whisper`, `shout`, `thought`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next: Vec<String>,
    /// Editor position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// A roll on a check: success when `fact + bonus + d(sides) >= difficulty`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollDef {
    #[serde(default = "twenty")]
    pub sides: u32,
    pub difficulty: i64,
    #[serde(default)]
    pub bonus: i64,
}

fn twenty() -> u32 {
    20
}

/// A skill, language or knowledge check on an option.
///
/// Without `roll` it is passive: it passes when `fact >= at_least` (numbers, or enums by declared order). An option
/// with `fail` targets may always be chosen and branches on the result; one without is gated: shown locked, with its
/// label, until the check passes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckDef {
    /// What kind of check, for the presenter (`skill`, `language`, `memory`, `book`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub category: String,
    /// What the player reads (`Mending 3`). Empty: made from the fact and threshold.
    #[serde(default, skip_serializing_if = "LocalText::is_empty")]
    pub label: LocalText,
    pub fact: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_least: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roll: Option<RollDef>,
}

/// One choice in a menu.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionDef {
    pub id: String,
    #[serde(default)]
    pub text: LocalText,
    /// Offered only when this holds.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub when: String,
    /// When the condition or a gating check fails: shown locked rather than hidden. Defaults to shown for checked
    /// options and hidden otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_locked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<CheckDef>,
    /// Gone once chosen.
    #[serde(default, skip_serializing_if = "is_false")]
    pub once: bool,
    /// A timed menu: this hidden option is taken when nobody chooses within this many milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Applied when chosen.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub effects: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next: Vec<String>,
    /// Followed instead of `next` when the check fails.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fail: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// A fork on a condition.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BranchDef {
    pub id: String,
    pub when: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next: Vec<String>,
    #[serde(default, rename = "else", skip_serializing_if = "Vec::is_empty")]
    pub otherwise: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// Effects with no line: set facts, start a quest, tell the game to cut the camera.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionDef {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub effects: String,
    /// Hold the conversation until the game says go (`Conversation::resume`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub wait: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// Play another dialogue, then come back and follow `next`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallDef {
    pub id: String,
    pub dialogue: String,
    /// A node id or named entry in it. Empty: its entry.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub entry: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// Go to another node (`node`, or `dialogue/node` to hand the conversation to another dialogue for good).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JumpDef {
    pub id: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// The end, with an optional result the game can read (`accepted`, `refused`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndDef {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub result: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// One node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeDef {
    Line(LineDef),
    Option(OptionDef),
    Branch(BranchDef),
    Action(ActionDef),
    Call(CallDef),
    Jump(JumpDef),
    End(EndDef),
}

impl NodeDef {
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            NodeDef::Line(n) => &n.id,
            NodeDef::Option(n) => &n.id,
            NodeDef::Branch(n) => &n.id,
            NodeDef::Action(n) => &n.id,
            NodeDef::Call(n) => &n.id,
            NodeDef::Jump(n) => &n.id,
            NodeDef::End(n) => &n.id,
        }
    }

    pub fn id_mut(&mut self) -> &mut String {
        match self {
            NodeDef::Line(n) => &mut n.id,
            NodeDef::Option(n) => &mut n.id,
            NodeDef::Branch(n) => &mut n.id,
            NodeDef::Action(n) => &mut n.id,
            NodeDef::Call(n) => &mut n.id,
            NodeDef::Jump(n) => &mut n.id,
            NodeDef::End(n) => &mut n.id,
        }
    }

    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            NodeDef::Line(_) => "line",
            NodeDef::Option(_) => "option",
            NodeDef::Branch(_) => "branch",
            NodeDef::Action(_) => "action",
            NodeDef::Call(_) => "call",
            NodeDef::Jump(_) => "jump",
            NodeDef::End(_) => "end",
        }
    }

    #[must_use]
    pub fn pos(&self) -> Option<[f32; 2]> {
        match self {
            NodeDef::Line(n) => n.pos,
            NodeDef::Option(n) => n.pos,
            NodeDef::Branch(n) => n.pos,
            NodeDef::Action(n) => n.pos,
            NodeDef::Call(n) => n.pos,
            NodeDef::Jump(n) => n.pos,
            NodeDef::End(n) => n.pos,
        }
    }

    pub fn set_pos(&mut self, pos: Option<[f32; 2]>) {
        match self {
            NodeDef::Line(n) => n.pos = pos,
            NodeDef::Option(n) => n.pos = pos,
            NodeDef::Branch(n) => n.pos = pos,
            NodeDef::Action(n) => n.pos = pos,
            NodeDef::Call(n) => n.pos = pos,
            NodeDef::Jump(n) => n.pos = pos,
            NodeDef::End(n) => n.pos = pos,
        }
    }

    /// Every outgoing link, by role: `("next", targets)`, `("else", ...)`, `("fail", ...)`, `("target", ...)`.
    #[must_use]
    pub fn links(&self) -> Vec<(&'static str, &[String])> {
        match self {
            NodeDef::Line(n) => vec![("next", n.next.as_slice())],
            NodeDef::Option(n) => vec![("next", n.next.as_slice()), ("fail", n.fail.as_slice())],
            NodeDef::Branch(n) => vec![
                ("next", n.next.as_slice()),
                ("else", n.otherwise.as_slice()),
            ],
            NodeDef::Action(n) => vec![("next", n.next.as_slice())],
            NodeDef::Call(n) => vec![("next", n.next.as_slice())],
            NodeDef::Jump(n) => vec![("target", std::slice::from_ref(&n.target))],
            NodeDef::End(_) => vec![],
        }
    }

    /// Mutable outgoing link lists (the jump target is not a list and is not included).
    pub fn link_lists_mut(&mut self) -> Vec<&mut Vec<String>> {
        match self {
            NodeDef::Line(n) => vec![&mut n.next],
            NodeDef::Option(n) => vec![&mut n.next, &mut n.fail],
            NodeDef::Branch(n) => vec![&mut n.next, &mut n.otherwise],
            NodeDef::Action(n) => vec![&mut n.next],
            NodeDef::Call(n) => vec![&mut n.next],
            NodeDef::Jump(_) | NodeDef::End(_) => vec![],
        }
    }
}

/// A dialogue asset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DialogueAsset {
    pub version: u32,
    pub id: String,
    /// What the editor and lint call it. Not shown to players.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// The node a conversation starts at. Empty: the first node.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub entry: String,
    /// More ways in, by name (`revisit`, `after_the_fire`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub entries: BTreeMap<String, String>,
    /// Where a conversation goes when the game interrupts it (the player walked off). Empty: it just ends.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub on_interrupt: String,
    /// Bark-style: plays without taking the player's input (chatter, a radio). The presenter auto-advances.
    #[serde(default, skip_serializing_if = "is_false")]
    pub ambient: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub nodes: Vec<NodeDef>,
}

impl DialogueAsset {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            version: DIALOGUE_VERSION,
            id: id.into(),
            title: String::new(),
            entry: String::new(),
            entries: BTreeMap::new(),
            on_interrupt: String::new(),
            ambient: false,
            note: String::new(),
            nodes: Vec::new(),
        }
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }

    /// Pretty JSON with a trailing newline, stable for diffs.
    pub fn to_json(&self) -> Result<String, String> {
        let mut text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        text.push('\n');
        Ok(text)
    }

    #[must_use]
    pub fn node(&self, id: &str) -> Option<&NodeDef> {
        self.nodes.iter().find(|n| n.id() == id)
    }

    /// The node a named entry (or the default entry, for `""`) leads to.
    #[must_use]
    pub fn entry_node(&self, entry: &str) -> Option<&str> {
        let id = if entry.is_empty() {
            if self.entry.is_empty() {
                return self.nodes.first().map(NodeDef::id);
            }
            self.entry.as_str()
        } else {
            self.entries.get(entry).map_or(entry, String::as_str)
        };
        self.node(id).map(NodeDef::id)
    }

    /// A fresh node id: `n<k>` above every numbered one.
    #[must_use]
    pub fn fresh_id(&self) -> String {
        let highest = self
            .nodes
            .iter()
            .filter_map(|n| n.id().strip_prefix('n')?.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        format!("n{}", highest + 1)
    }

    /// Structural problems that need no manifest: ids, links, menus, expression syntax. The full check (facts,
    /// speakers, other dialogues) is `StoryDatabase`'s.
    #[must_use]
    pub fn structural_problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        let mut warnings = Vec::new();
        let mut push = |at: &str, message: String| problems.push(Problem::error(at, message));
        if self.version != DIALOGUE_VERSION {
            push(
                "",
                format!(
                    "dialogue version {} is not {DIALOGUE_VERSION}",
                    self.version
                ),
            );
        }
        if !valid_id(&self.id) {
            push("", format!("dialogue id \"{}\" is not a valid id", self.id));
        }
        if self.nodes.is_empty() {
            push("", "a dialogue needs at least one node".into());
        }
        let mut ids = BTreeSet::new();
        for node in &self.nodes {
            let id = node.id();
            if !valid_id(id) || id.contains('.') {
                push(
                    id,
                    format!("node id \"{id}\" must be letters, digits and _"),
                );
            }
            if !ids.insert(id) {
                push(id, format!("node id {id} is used twice"));
            }
        }
        let exists = |target: &str| target.contains('/') || ids.contains(target);
        if !self.entry.is_empty() && !exists(&self.entry) {
            push("", format!("entry {} is not a node", self.entry));
        }
        for (name, target) in &self.entries {
            if !exists(target) || target.contains('/') {
                push(
                    "",
                    format!("entry {name} leads to {target}, which is not a node here"),
                );
            }
        }
        if !self.on_interrupt.is_empty() && !ids.contains(self.on_interrupt.as_str()) {
            push(
                "",
                format!("on_interrupt {} is not a node", self.on_interrupt),
            );
        }
        let kind_of = |target: &str| self.node(target).map(NodeDef::kind);
        for node in &self.nodes {
            let id = node.id();
            for (role, targets) in node.links() {
                let mut seen = BTreeSet::new();
                for target in targets {
                    if target.is_empty() {
                        push(id, format!("{role} has an empty target"));
                    } else if !exists(target) {
                        push(id, format!("{role} leads to {target}, which is not a node"));
                    } else if !seen.insert(target) {
                        push(id, format!("{role} lists {target} twice"));
                    }
                }
                let options = targets
                    .iter()
                    .filter(|t| kind_of(t) == Some("option"))
                    .count();
                if options > 0 && options < targets.len() {
                    push(
                        id,
                        format!("{role} mixes options with other nodes; a menu is options only"),
                    );
                }
                if targets.len() > 1 && options == 0 && role != "target" {
                    for target in &targets[..targets.len() - 1] {
                        if let Some(n) = self.node(target)
                            && !matches!(n, NodeDef::Line(l) if !l.when.trim().is_empty())
                        {
                            warnings.push(Problem::warning(
                                id,
                                format!(
                                    "{role}: {target} always qualifies, so nothing after it is ever reached"
                                ),
                            ));
                        }
                    }
                }
                if options > 0 {
                    let timeouts = targets
                        .iter()
                        .filter(|t| matches!(self.node(t), Some(NodeDef::Option(o)) if o.timeout_ms.is_some()))
                        .count();
                    if timeouts > 1 {
                        push(
                            id,
                            format!("{role}: a menu takes at most one timeout option"),
                        );
                    }
                }
            }
            if let NodeDef::Jump(jump) = node
                && jump.target == jump.id
            {
                push(id, "a jump to itself never ends".into());
            }
            match node {
                NodeDef::Line(line) => {
                    if line.text.is_empty() {
                        push(id, "a line with no text".into());
                    }
                    syntax(&mut push, id, "when", &line.when, false);
                    syntax(&mut push, id, "effects", &line.effects, true);
                    for problem in line.text.problems() {
                        push(id, problem);
                    }
                }
                NodeDef::Option(option) => {
                    if option.text.is_empty() && option.timeout_ms.is_none() {
                        push(id, "an option with no text".into());
                    }
                    if option.timeout_ms == Some(0) {
                        push(id, "timeout_ms must be above zero".into());
                    }
                    if !option.fail.is_empty() && option.check.is_none() {
                        push(id, "fail targets need a check".into());
                    }
                    if let Some(check) = &option.check {
                        if check.at_least.is_none() && check.roll.is_none() {
                            push(id, "a check needs at_least or roll".into());
                        }
                        if check.roll.as_ref().is_some_and(|r| r.sides == 0) {
                            push(id, "a roll needs at least one side".into());
                        }
                        if check.roll.is_some() && option.fail.is_empty() {
                            push(
                                id,
                                "a roll needs fail targets: where does a failed roll go?".into(),
                            );
                        }
                    }
                    syntax(&mut push, id, "when", &option.when, false);
                    syntax(&mut push, id, "effects", &option.effects, true);
                }
                NodeDef::Branch(branch) => {
                    if branch.when.trim().is_empty() {
                        push(id, "a branch needs a condition".into());
                    }
                    syntax(&mut push, id, "when", &branch.when, false);
                }
                NodeDef::Action(action) => syntax(&mut push, id, "effects", &action.effects, true),
                NodeDef::Call(call) => {
                    if !valid_id(&call.dialogue) {
                        push(id, format!("call names dialogue \"{}\"", call.dialogue));
                    }
                }
                NodeDef::Jump(jump) => {
                    if jump.target.is_empty() {
                        push(id, "a jump needs a target".into());
                    }
                }
                NodeDef::End(_) => {}
            }
        }
        // Options are entered from menus only: one with no menu leading to it is lost.
        let mut menu_members = BTreeSet::new();
        for node in &self.nodes {
            for (_, targets) in node.links() {
                for target in targets {
                    menu_members.insert(target.as_str());
                }
            }
        }
        for node in &self.nodes {
            if let NodeDef::Option(option) = node
                && !menu_members.contains(option.id.as_str())
            {
                push(&option.id, "no menu offers this option".into());
            }
        }
        if !self.nodes.is_empty() && problems.is_empty() {
            for unreachable in self.unreachable() {
                warnings.push(Problem::warning(
                    unreachable.clone(),
                    format!("{unreachable} can never be reached"),
                ));
            }
        }
        problems.extend(warnings);
        problems
    }

    /// Nodes no entry, interruption or link reaches.
    #[must_use]
    pub fn unreachable(&self) -> Vec<String> {
        let mut seen = BTreeSet::new();
        let mut stack: Vec<&str> = Vec::new();
        if let Some(entry) = self.entry_node("") {
            stack.push(entry);
        }
        stack.extend(self.entries.values().map(String::as_str));
        if !self.on_interrupt.is_empty() {
            stack.push(&self.on_interrupt);
        }
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            if let Some(node) = self.node(id) {
                for (_, targets) in node.links() {
                    stack.extend(targets.iter().map(String::as_str));
                }
            }
        }
        self.nodes
            .iter()
            .map(NodeDef::id)
            .filter(|id| !seen.contains(id))
            .map(str::to_owned)
            .collect()
    }

    /// Text search over ids, speakers, text, conditions, effects and notes, for the editor's search box and agents.
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<String> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        self.nodes
            .iter()
            .filter(|node| {
                serde_json::to_string(node)
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains(&query)
            })
            .map(|node| node.id().to_owned())
            .collect()
    }
}

/// Check expression syntax without a schema: unknown facts are not this check's business.
fn syntax(push: &mut impl FnMut(&str, String), id: &str, field: &str, source: &str, effects: bool) {
    if let Err(error) = crate::expr::syntax(source, effects) {
        push(id, format!("{field}: {error}"));
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Compiled form

/// A node with its expressions compiled.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub def: NodeDef,
    pub when: Condition,
    pub effects: Effects,
}

/// A checked dialogue the runtime plays.
#[derive(Clone, Debug, PartialEq)]
pub struct Dialogue {
    pub asset: DialogueAsset,
    /// What compiled but deserves a look (unreachable nodes, shadowed continuations).
    pub warnings: Vec<Problem>,
    nodes: Vec<Node>,
    index: BTreeMap<String, usize>,
}

impl Dialogue {
    /// Compile every condition and effect against `ctx`. Problems come back as `(node, message)`.
    pub fn compile(mut asset: DialogueAsset, ctx: Context<'_>) -> Result<Self, Vec<Problem>> {
        let mut problems = asset.structural_problems();
        let dialogue_id = asset.id.clone();
        for node in &mut asset.nodes {
            let key = format!("dlg.{dialogue_id}.{}", node.id());
            match node {
                NodeDef::Line(line) => line.text.assign_key(|| key),
                NodeDef::Option(option) => {
                    option.text.assign_key(|| key.clone());
                    if let Some(check) = &mut option.check {
                        check.label.assign_key(|| format!("{key}.check"));
                    }
                }
                _ => {}
            }
        }
        let mut nodes = Vec::with_capacity(asset.nodes.len());
        let mut index = BTreeMap::new();
        for def in &asset.nodes {
            let (when, effects) = match def {
                NodeDef::Line(l) => (l.when.as_str(), l.effects.as_str()),
                NodeDef::Option(o) => (o.when.as_str(), o.effects.as_str()),
                NodeDef::Branch(b) => (b.when.as_str(), ""),
                NodeDef::Action(a) => ("", a.effects.as_str()),
                _ => ("", ""),
            };
            let mut report = |field: &str, error: ExprError| {
                problems.push(Problem::error(def.id(), format!("{field}: {error}")));
            };
            let when = Condition::compile(when, ctx).unwrap_or_else(|e| {
                report("when", e);
                Condition::always()
            });
            let effects = Effects::compile(effects, ctx).unwrap_or_else(|e| {
                report("effects", e);
                Effects::default()
            });
            if let NodeDef::Option(OptionDef {
                check: Some(check), ..
            }) = def
            {
                check_against_schema(check, ctx, &mut report);
            }
            index.insert(def.id().to_owned(), nodes.len());
            nodes.push(Node {
                def: def.clone(),
                when,
                effects,
            });
        }
        problems.sort();
        problems.dedup();
        if has_errors(&problems) {
            Err(problems)
        } else {
            Ok(Self {
                asset,
                warnings: problems,
                nodes,
                index,
            })
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.asset.id
    }

    #[must_use]
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.index.get(id).map(|&i| &self.nodes[i])
    }

    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }
}

fn check_against_schema(
    check: &CheckDef,
    ctx: Context<'_>,
    report: &mut impl FnMut(&str, ExprError),
) {
    let Some(def) = ctx.schema.get(&check.fact) else {
        report(
            "check",
            ExprError {
                at: 0,
                message: format!("`{}` is not a declared fact", check.fact),
            },
        );
        return;
    };
    use crate::value::FactType;
    let fault = match (&check.at_least, def.ty) {
        (_, FactType::Int) if check.roll.is_some() => None,
        (_, _) if check.roll.is_some() => Some("a roll needs a number fact".to_owned()),
        (Some(Value::Int(_)), FactType::Int) => None,
        (Some(Value::Text(variant)), FactType::Enum) => (!def.values.contains(variant))
            .then(|| format!("'{variant}' is not one of {}", def.values.join(", "))),
        (Some(_), _) => Some(format!(
            "at_least does not fit {}, a {:?} fact",
            def.id, def.ty
        )),
        (None, _) => None,
    };
    if let Some(message) = fault {
        report("check", ExprError { at: 0, message });
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::value::{FactDef, FactType, Schema};

    pub(crate) fn sample() -> DialogueAsset {
        DialogueAsset::from_json(
            r#"{
            "version": 1, "id": "mireille_intro", "title": "Mireille, the first evening",
            "on_interrupt": "walked_off",
            "nodes": [
                {"type": "line", "id": "n1", "speaker": "mireille", "expression": "smile",
                 "text": "You're late, little lord.", "effects": "met.mireille = true", "next": ["o1", "o2", "o3"]},
                {"type": "option", "id": "o1", "text": "I was reading.", "next": ["n2"]},
                {"type": "option", "id": "o2", "text": "[Patter] Late's a rich word from you.",
                 "check": {"category": "language", "fact": "lang.patter", "at_least": "conversant"}, "next": ["n3"]},
                {"type": "option", "id": "o3", "text": "...", "timeout_ms": 8000, "next": ["n2"]},
                {"type": "line", "id": "n2", "speaker": "mireille", "text": "Of course you were.",
                 "effects": "mireille.affection += 1", "next": ["b1"]},
                {"type": "line", "id": "n3", "speaker": "mireille", "expression": "laugh",
                 "text": "Oh, he bites.", "effects": "mireille.affection += 2", "next": ["b1"]},
                {"type": "branch", "id": "b1", "when": "mireille.affection >= 2", "next": ["n4"], "else": ["e1"]},
                {"type": "line", "id": "n4", "speaker": "mireille", "text": "Come. Walk with me.", "next": ["e1"]},
                {"type": "end", "id": "e1", "result": "met"},
                {"type": "line", "id": "walked_off", "speaker": "mireille", "text": "Rude."}
            ]}"#,
        )
        .unwrap()
    }

    pub(crate) fn schema() -> Schema {
        Schema::new([
            FactDef::new("met.mireille", FactType::Bool),
            FactDef::new("mireille.affection", FactType::Int).with_range(-10, 10),
            FactDef::new("lang.patter", FactType::Enum).with_values(&[
                "none",
                "phrases",
                "conversant",
                "fluent",
            ]),
        ])
        .unwrap()
    }

    #[test]
    fn the_sample_compiles_with_keys_assigned() {
        let schema = schema();
        let dialogue = Dialogue::compile(sample(), Context::new(&schema, &[])).unwrap();
        let NodeDef::Line(line) = &dialogue.node("n1").unwrap().def else {
            panic!()
        };
        assert_eq!(line.text.key, "dlg.mireille_intro.n1");
        assert_eq!(dialogue.asset.entry_node(""), Some("n1"));
        let json = sample().to_json().unwrap();
        assert_eq!(
            DialogueAsset::from_json(&json).unwrap(),
            sample(),
            "round trip"
        );
        assert!(json.contains("\"type\": \"line\""));
    }

    #[test]
    fn structural_mistakes_are_named_by_node() {
        let mut asset = sample();
        asset.nodes.push(NodeDef::Line(LineDef {
            id: "n1".into(),
            text: "dup".into(),
            ..LineDef::default()
        }));
        if let NodeDef::Line(line) = &mut asset.nodes[0] {
            line.next.push("n9".into());
        }
        asset.nodes.push(NodeDef::Option(OptionDef {
            id: "stray".into(),
            text: "lost".into(),
            ..OptionDef::default()
        }));
        let problems = asset.structural_problems();
        let messages: Vec<&str> = problems.iter().map(|p| p.message.as_str()).collect();
        assert!(
            messages.iter().any(|m| m.contains("used twice")),
            "{messages:?}"
        );
        assert!(
            messages
                .iter()
                .any(|m| m.contains("n9, which is not a node"))
        );
        assert!(messages.iter().any(|m| m.contains("no menu offers")));
    }

    #[test]
    fn mixed_menus_shadowed_continuations_and_bad_expressions_are_found() {
        let mut asset = DialogueAsset::new("d");
        asset.nodes = serde_json::from_str(
            r#"[{"type":"line","id":"a","text":"x","next":["o","b"], "effects": "x +="},
                {"type":"option","id":"o","text":"y"},
                {"type":"line","id":"b","text":"z","next":["c","d"]},
                {"type":"line","id":"c","text":"c"},
                {"type":"line","id":"d","text":"d", "when": "1 <"}]"#,
        )
        .unwrap();
        let problems = asset.structural_problems();
        let has = |at: &str, needle: &str, warning: bool| {
            problems
                .iter()
                .any(|p| p.at == at && p.message.contains(needle) && p.warning == warning)
        };
        assert!(has("a", "mixes options", false), "{problems:?}");
        assert!(has("b", "always qualifies", true));
        assert!(has("a", "effects:", false));
        assert!(has("d", "when:", false));
    }

    #[test]
    fn compile_checks_facts_and_checks_against_the_schema() {
        let schema = schema();
        let mut asset = sample();
        if let NodeDef::Option(option) = &mut asset.nodes[2] {
            option.check.as_mut().unwrap().at_least = Some("conversent".into());
        }
        if let NodeDef::Line(line) = &mut asset.nodes[4] {
            line.effects = "mireille.afection += 1".into();
        }
        let problems = Dialogue::compile(asset, Context::new(&schema, &[])).unwrap_err();
        assert_eq!(problems.len(), 2, "{problems:?}");
    }

    #[test]
    fn unreachable_nodes_and_search() {
        let mut asset = sample();
        asset.nodes.push(NodeDef::End(EndDef {
            id: "island".into(),
            ..EndDef::default()
        }));
        assert_eq!(asset.unreachable(), ["island"]);
        assert_eq!(asset.search("BITES"), ["n3"]);
        assert_eq!(asset.fresh_id(), "n5");
    }
}
