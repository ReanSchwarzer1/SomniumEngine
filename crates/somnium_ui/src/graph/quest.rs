//! Quest authoring on the one graph surface.
//!
//! The quest asset (`*.somquest`) is the source of truth; this is its catalogue and the lossless conversion:
//!
//! - one **Quest** node holds the quest's own fields and leads to the first stage;
//! - each **Stage** has four numbered exits, `Next 1` to `Next 4`, tried in that order, each with its `When`
//!   (empty: when every required objective is done);
//! - **Objectives** and **journal lines** are their own nodes wired into the stage they belong to, ordered top to
//!   bottom as drawn;
//! - a link back to an earlier stage is drawn as a **Go To** (the surface refuses cycles) and saved as a link.

use super::{Catalogue, Graph, GraphSurface, NodeArchetype, NodeId, PinArchetype, PinRef, PinType};
use glam::Vec2;
use somnium_story::problem::Problem;
use somnium_story::quest::{
    CountDef, Expire, JournalLine, MarkerDef, ObjectiveDef, QuestAsset, StageDef, StageEnd,
    StageLink,
};
use somnium_story::text::LocalText;
use std::collections::{BTreeMap, BTreeSet};

/// The catalogue id written into a quest graph.
pub const CATALOGUE: &str = "somnium.quest";

const FLOW: PinType = PinType::Opaque("quest.flow");
const PART: PinType = PinType::Opaque("quest.part");
const TEXT: PinType = PinType::Opaque("quest.text");

pub const QUEST: &str = "quest.quest";
pub const STAGE: &str = "quest.stage";
pub const OBJECTIVE: &str = "quest.objective";
pub const JOURNAL: &str = "quest.journal";
pub const GOTO: &str = "quest.goto";

/// Exits per stage.
pub const EXITS: u16 = 4;

const WIDE: u16 = 300;
const COLUMN: f32 = 700.0;
const SATELLITE: f32 = 340.0;
const ROW: f32 = 48.0;

fn text(name: &'static str, tooltip: &'static str) -> PinArchetype {
    PinArchetype::new(name, TEXT)
        .with_default("")
        .with_tooltip(tooltip)
}

fn note() -> PinArchetype {
    text("Note", "A note for writers; never shown to players.")
        .detail()
        .multiline()
}

/// The quest catalogue.
#[must_use]
pub fn catalogue() -> Catalogue {
    let mut c = Catalogue::new(CATALOGUE);
    c.register(
        NodeArchetype::new(QUEST, "Quest", "Quest")
            .with_keywords(&["quest", "root", "job"])
            .with_input(text(
                "Id",
                "The quest's id: start_quest('id'), quest('id') and saves name it by.",
            ))
            .with_input(text("Title", "The journal heading.").multiline())
            .with_input(
                text("Summary", "One or two sentences for the journal.")
                    .multiline()
                    .detail(),
            )
            .with_input(text(
                "Category",
                "main, companion, faction, contract, event: the journal groups by it.",
            ))
            .with_input(text("Giver", "A cast id."))
            .with_input(text("Region", "Where it happens, for the journal's filters.").detail())
            .with_input(
                text(
                    "Priority",
                    "A whole number; higher sorts first and wins marker overlaps.",
                )
                .detail(),
            )
            .with_input(text(
                "Start When",
                "Starts by itself when this holds; empty: only start_quest.",
            ))
            .with_input(
                PinArchetype::new("Repeatable", TEXT)
                    .with_default("false")
                    .with_tooltip("true: may start again after it ends.")
                    .detail(),
            )
            .with_input(text(
                "Fail When",
                "Fails the quest whenever this holds while it runs.",
            ))
            .with_input(
                text("On Start", "Effects when it starts.")
                    .detail()
                    .multiline(),
            )
            .with_input(text("On Complete", "Effects when it completes: rewards.").multiline())
            .with_input(
                text("On Fail", "Effects when it fails.")
                    .detail()
                    .multiline(),
            )
            .with_input(note())
            .with_output(PinArchetype::new("First Stage", FLOW))
            .as_root()
            .with_width(WIDE),
    );
    let mut stage = NodeArchetype::new(STAGE, "Stage", "Quest")
        .with_keywords(&["stage", "step", "state"])
        .with_input(PinArchetype::new("In", FLOW).many())
        .with_input(PinArchetype::new("Journal", PART).many())
        .with_input(PinArchetype::new("Objectives", PART).many())
        .with_input(text(
            "Title",
            "A heading for this stage in the journal; optional.",
        ))
        .with_input(text("On Enter", "Effects when the stage is entered.").multiline())
        .with_input(text(
            "End",
            "complete or fail: entering this stage ends the quest. Empty: it does not.",
        ));
    for (when, tip) in [
        (
            "When 1",
            "Exit 1 is taken when this holds; empty: when every required objective is done.",
        ),
        ("When 2", "Exit 2, tried after exit 1."),
        ("When 3", "Exit 3, tried after exit 2."),
        ("When 4", "Exit 4, tried last."),
    ] {
        stage = stage.with_input(text(when, tip));
    }
    stage = stage.with_input(note());
    for exit in ["Next 1", "Next 2", "Next 3", "Next 4"] {
        stage = stage.with_output(PinArchetype::new(exit, FLOW));
    }
    c.register(stage.with_width(WIDE));
    c.register(
        NodeArchetype::new(OBJECTIVE, "Objective", "Quest")
            .with_keywords(&["objective", "task", "goal", "marker", "timer", "count"])
            .with_input(text("Text", "What the journal and HUD say.").multiline())
            .with_input(text("Done When", "Done when this holds."))
            .with_input(text("Count", "fact >= target: a counter the HUD shows as 2/3."))
            .with_input(text(
                "Marker",
                "An anchor name or (x, y, z), then r=<metres> and arrive: strongroom_door r=2 arrive",
            ))
            .with_input(
                PinArchetype::new("Optional", TEXT)
                    .with_default("false")
                    .with_tooltip("true: never holds the stage back, never fails the quest.")
                    .detail(),
            )
            .with_input(
                PinArchetype::new("Hidden", TEXT)
                    .with_default("false")
                    .with_tooltip("true: tracked but not listed.")
                    .detail(),
            )
            .with_input(text("Fail When", "Fails when this holds.").detail())
            .with_input(text("Timer (ms)", "Game time from activation.").detail())
            .with_input(
                PinArchetype::new("On Expire", TEXT)
                    .with_default("fail")
                    .with_tooltip("fail or complete: what the timer running out does.")
                    .detail(),
            )
            .with_input(text("On Done", "Effects when done.").detail().multiline())
            .with_input(text("On Fail", "Effects when failed.").detail().multiline())
            .with_output(PinArchetype::new("Stage", PART))
            .with_width(WIDE),
    );
    c.register(
        NodeArchetype::new(JOURNAL, "Journal Line", "Quest")
            .with_keywords(&["journal", "log", "entry", "tier", "lore"])
            .with_input(
                text("Text", "What the journal gains when the stage is entered.").multiline(),
            )
            .with_input(text(
                "Tier",
                "How it is known: one of the manifest's truth tiers (witnessed, retold, ...).",
            ))
            .with_output(PinArchetype::new("Stage", PART))
            .with_width(WIDE),
    );
    c.register(
        NodeArchetype::new(GOTO, "Go To Stage", "Quest")
            .with_keywords(&["jump", "goto", "loop", "back", "retry"])
            .with_input(PinArchetype::new("In", FLOW).many())
            .with_input(text("Stage", "The stage to go to; saved as a plain link.")),
    );
    c
}

fn prefix(archetype: &str) -> &'static str {
    match archetype {
        STAGE => "s",
        OBJECTIVE => "o",
        JOURNAL => "j",
        _ => "x",
    }
}

/// A fresh id for a new node of `archetype`.
#[must_use]
pub fn fresh_id(graph: &Graph, archetype: &str) -> String {
    let prefix = prefix(archetype);
    let highest = graph
        .nodes()
        .iter()
        .filter_map(|n| n.title.strip_prefix(prefix)?.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    format!("{prefix}{}", highest + 1)
}

// ---------------------------------------------------------------------------------------------------------------
// Literal formats

/// `strongroom_door r=2 arrive` or `(12, 0, -3) r=10`.
#[must_use]
pub fn format_marker(marker: &MarkerDef) -> String {
    let mut out = if marker.anchor.is_empty() {
        marker
            .position
            .map(|[x, y, z]| format!("({x}, {y}, {z})"))
            .unwrap_or_default()
    } else {
        marker.anchor.clone()
    };
    if marker.radius != 0.0 {
        out.push_str(&format!(" r={}", marker.radius));
    }
    if marker.arrive {
        out.push_str(" arrive");
    }
    out
}

/// Parse [`format_marker`]'s text. Blank is no marker.
pub fn parse_marker(text: &str) -> Result<Option<MarkerDef>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let mut marker = MarkerDef::default();
    let rest = if let Some(inner) = text.strip_prefix('(') {
        let (inside, rest) = inner.split_once(')').ok_or("a position is (x, y, z)")?;
        let parts: Vec<f32> = inside
            .split(',')
            .map(|p| p.trim().parse::<f32>())
            .collect::<Result<_, _>>()
            .map_err(|_| "a position is three numbers: (x, y, z)".to_owned())?;
        let [x, y, z] = parts.as_slice() else {
            return Err("a position is three numbers: (x, y, z)".into());
        };
        marker.position = Some([*x, *y, *z]);
        rest
    } else {
        let (anchor, rest) = text.split_once(' ').unwrap_or((text, ""));
        marker.anchor = anchor.to_owned();
        rest
    };
    for word in rest.split_whitespace() {
        if let Some(radius) = word.strip_prefix("r=") {
            marker.radius = radius
                .parse()
                .map_err(|_| format!("`{word}`: a radius is r=<metres>"))?;
        } else if word == "arrive" {
            marker.arrive = true;
        } else {
            return Err(format!("`{word}` is not r=<metres> or arrive"));
        }
    }
    Ok(Some(marker))
}

/// `fact >= target`.
pub fn parse_count(text: &str) -> Result<Option<CountDef>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let (fact, target) = text
        .split_once(">=")
        .ok_or_else(|| format!("a count reads `fact >= target`, not `{text}`"))?;
    Ok(Some(CountDef {
        fact: fact.trim().to_owned(),
        target: target
            .trim()
            .parse()
            .map_err(|_| format!("`{}` is not a whole number", target.trim()))?,
    }))
}

fn parse_bool(text: &str, field: &str) -> Result<bool, String> {
    match text.trim().to_ascii_lowercase().as_str() {
        "" | "false" | "no" | "0" => Ok(false),
        "true" | "yes" | "1" => Ok(true),
        other => Err(format!("{field} is true or false, not `{other}`")),
    }
}

/// Keys, forms and notes of texts the graph does not hold, by `(node id, pin)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extras {
    pub texts: BTreeMap<(String, u16), LocalText>,
}

impl Extras {
    fn keep(&mut self, node: &str, pin: u16, text: &LocalText) -> String {
        self.texts.insert((node.to_owned(), pin), text.clone());
        text.source.clone()
    }

    fn text(&self, node: &str, pin: u16, source: String) -> LocalText {
        match self.texts.get(&(node.to_owned(), pin)) {
            Some(original) => LocalText {
                source,
                ..original.clone()
            },
            None => LocalText {
                source,
                ..LocalText::default()
            },
        }
    }
}

const ROOT: &str = "";

// ---------------------------------------------------------------------------------------------------------------
// Asset -> graph

/// Draw a quest.
#[must_use]
pub fn to_graph(asset: &QuestAsset) -> (Graph, Extras) {
    let c = catalogue();
    let mut graph = Graph::new();
    let mut extras = Extras::default();
    let root = graph.add(&c, QUEST, Vec2::ZERO).expect("registered");
    let fields = [
        asset.id.clone(),
        extras.keep(ROOT, 1, &asset.title),
        extras.keep(ROOT, 2, &asset.summary),
        asset.category.clone(),
        asset.giver.clone(),
        asset.region.clone(),
        if asset.priority == 0 {
            String::new()
        } else {
            asset.priority.to_string()
        },
        asset.start_when.clone(),
        asset.repeatable.to_string(),
        asset.fail_when.clone(),
        asset.on_start.clone(),
        asset.on_complete.clone(),
        asset.on_fail.clone(),
        asset.note.clone(),
    ];
    for (pin, value) in fields.into_iter().enumerate() {
        set(&mut graph, root, pin as u16, &value);
    }
    let mut stages: BTreeMap<&str, NodeId> = BTreeMap::new();
    let mut satellites: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for stage in &asset.stages {
        let id = graph.add(&c, STAGE, Vec2::ZERO).expect("registered");
        graph.node_mut(id).expect("added").title = stage.id.clone();
        stages.insert(&stage.id, id);
        set(&mut graph, id, 3, &extras.keep(&stage.id, 3, &stage.title));
        set(&mut graph, id, 4, &stage.on_enter);
        set(
            &mut graph,
            id,
            5,
            match stage.end {
                Some(StageEnd::Complete) => "complete",
                Some(StageEnd::Fail) => "fail",
                None => "",
            },
        );
        for (k, link) in stage.next.iter().take(EXITS as usize).enumerate() {
            set(&mut graph, id, 6 + k as u16, &link.when);
        }
        set(&mut graph, id, 10, &stage.note);
        for line in &stage.journal {
            let node = graph.add(&c, JOURNAL, Vec2::ZERO).expect("registered");
            let title = format!("{}.{}", stage.id, line.id);
            graph.node_mut(node).expect("added").title = line.id.clone();
            set(&mut graph, node, 0, &extras.keep(&title, 0, &line.text));
            set(&mut graph, node, 1, &line.tier);
            graph
                .connect(&c, PinRef::output(node, 0), PinRef::input(id, 1))
                .expect("a journal line wires into its stage");
            satellites.entry(id).or_default().push(node);
        }
        for objective in &stage.objectives {
            let node = graph.add(&c, OBJECTIVE, Vec2::ZERO).expect("registered");
            graph.node_mut(node).expect("added").title = objective.id.clone();
            let values = [
                extras.keep(&objective.id, 0, &objective.text),
                objective.done_when.clone(),
                objective
                    .count
                    .as_ref()
                    .map(|c| format!("{} >= {}", c.fact, c.target))
                    .unwrap_or_default(),
                objective
                    .marker
                    .as_ref()
                    .map(format_marker)
                    .unwrap_or_default(),
                objective.optional.to_string(),
                objective.hidden.to_string(),
                objective.fail_when.clone(),
                objective
                    .timer_ms
                    .map(|t| t.to_string())
                    .unwrap_or_default(),
                match objective.on_expire {
                    Expire::Fail => "fail".into(),
                    Expire::Complete => "complete".into(),
                },
                objective.on_done.clone(),
                objective.on_fail.clone(),
            ];
            for (pin, value) in values.into_iter().enumerate() {
                set(&mut graph, node, pin as u16, &value);
            }
            graph
                .connect(&c, PinRef::output(node, 0), PinRef::input(id, 2))
                .expect("an objective wires into its stage");
            satellites.entry(id).or_default().push(node);
        }
    }
    // Stage links, depth first from the first stage so the links back up become Go Tos.
    let mut wires: Vec<(NodeId, u16, String)> = Vec::new();
    let mut gotos: Vec<(NodeId, u16, String)> = Vec::new();
    let mut state: BTreeMap<&str, bool> = BTreeMap::new();
    let links_of = |id: &str| -> Vec<(u16, String)> {
        asset
            .stage(id)
            .map(|s| {
                s.next
                    .iter()
                    .take(EXITS as usize)
                    .enumerate()
                    .map(|(k, l)| (k as u16, l.to.clone()))
                    .collect()
            })
            .unwrap_or_default()
    };
    let first = asset.first_stage().map(str::to_owned);
    let order: Vec<String> = first
        .iter()
        .cloned()
        .chain(asset.stages.iter().map(|s| s.id.clone()))
        .collect();
    for start in &order {
        let Some((&start, _)) = stages.get_key_value(start.as_str()) else {
            continue;
        };
        if state.contains_key(start) {
            continue;
        }
        let mut stack: super::dialogue::Walk<'_> = vec![(start, links_of(start), 0)];
        state.insert(start, false);
        while let Some(top) = stack.last_mut() {
            let node = top.0;
            let step = top.1.get(top.2).cloned();
            top.2 += 1;
            let Some((pin, link)) = step else {
                state.insert(node, true);
                stack.pop();
                continue;
            };
            let from = stages[node];
            match stages.get_key_value(link.as_str()) {
                None => gotos.push((from, pin, link)),
                Some((&to, _)) => match state.get(to) {
                    Some(false) => gotos.push((from, pin, link)),
                    Some(true) => wires.push((from, pin, link)),
                    None => {
                        wires.push((from, pin, link.clone()));
                        state.insert(to, false);
                        stack.push((to, links_of(to), 0));
                    }
                },
            }
        }
    }
    if let Some(first) = &first
        && let Some(&to) = stages.get(first.as_str())
    {
        graph
            .connect(&c, PinRef::output(root, 0), PinRef::input(to, 0))
            .expect("the quest leads to its first stage");
    }
    for (from, pin, target) in wires {
        let to = stages[target.as_str()];
        if graph
            .connect(&c, PinRef::output(from, pin), PinRef::input(to, 0))
            .is_err()
        {
            gotos.push((from, pin, target));
        }
    }
    let mut goto_nodes = Vec::new();
    for (from, pin, target) in gotos {
        let node = graph.add(&c, GOTO, Vec2::ZERO).expect("registered");
        set(&mut graph, node, 1, &target);
        graph
            .connect(&c, PinRef::output(from, pin), PinRef::input(node, 0))
            .expect("a fresh Go To takes its wire");
        goto_nodes.push((from, pin, node));
    }
    layout(
        &mut graph,
        &c,
        asset,
        root,
        &stages,
        &satellites,
        &goto_nodes,
    );
    (graph, extras)
}

fn set(graph: &mut Graph, node: NodeId, pin: u16, value: &str) {
    if let Some(node) = graph.node_mut(node) {
        node.literals.insert(pin, value.to_owned());
    }
}

/// Stages left to right by depth, each with its journal lines and objectives stacked just left of it; the quest
/// node first. Saved stage positions win (satellites follow their stage).
fn layout(
    graph: &mut Graph,
    c: &Catalogue,
    asset: &QuestAsset,
    root: NodeId,
    stages: &BTreeMap<&str, NodeId>,
    satellites: &BTreeMap<NodeId, Vec<NodeId>>,
    gotos: &[(NodeId, u16, NodeId)],
) {
    let size = |graph: &Graph, id: NodeId| {
        super::geometry::layout_node(graph.node(id).expect("node exists"), c)
            .map_or(Vec2::new(180.0, 120.0), |l| l.bounds.size())
    };
    let column_height = |graph: &Graph, stage: NodeId| {
        let parts: f32 = satellites
            .get(&stage)
            .map(|s| s.iter().map(|n| size(graph, *n).y + 16.0).sum())
            .unwrap_or(0.0);
        size(graph, stage).y.max(parts)
    };
    let mut placed: BTreeMap<NodeId, Vec2> = BTreeMap::new();
    let mut y = 0.0f32;
    let mut row_height = 0.0f32;
    let order: Vec<&str> = asset
        .first_stage()
        .into_iter()
        .chain(asset.stages.iter().map(|s| s.id.as_str()))
        .collect();
    for start in order {
        let Some(&start) = stages.get(start) else {
            continue;
        };
        let mut stack = vec![(start, 1usize)];
        while let Some((node, depth)) = stack.pop() {
            if placed.contains_key(&node) {
                continue;
            }
            placed.insert(node, Vec2::new(depth as f32 * COLUMN, y));
            row_height = row_height.max(column_height(graph, node));
            let mut kids = Vec::new();
            for pin in 0..EXITS {
                for target in graph.output_targets(PinRef::output(node, pin)) {
                    if graph
                        .node(target.node)
                        .is_some_and(|n| n.archetype == STAGE)
                        && !placed.contains_key(&target.node)
                    {
                        kids.push(target.node);
                    }
                }
            }
            if kids.is_empty() {
                y += row_height + ROW;
                row_height = 0.0;
            }
            for kid in kids.into_iter().rev() {
                stack.push((kid, depth + 1));
            }
        }
    }
    let saved: BTreeMap<NodeId, Vec2> = asset
        .stages
        .iter()
        .filter_map(|s| s.pos.map(|p| (stages[s.id.as_str()], Vec2::from_array(p))))
        .collect();
    if !saved.is_empty() {
        let bottom = saved
            .iter()
            .map(|(id, p)| p.y + column_height(graph, *id))
            .fold(f32::MIN, f32::max)
            + ROW * 3.0;
        for (id, position) in &mut placed {
            if !saved.contains_key(id) {
                position.y += bottom;
            }
        }
        placed.extend(saved.iter().map(|(id, p)| (*id, *p)));
    }
    for (stage, position) in &placed {
        if let Some(node) = graph.node_mut(*stage) {
            node.position = *position;
        }
        let mut at = *position + Vec2::new(-SATELLITE, 0.0);
        for part in satellites.get(stage).into_iter().flatten() {
            let h = size(graph, *part).y;
            if let Some(node) = graph.node_mut(*part) {
                node.position = at;
            }
            at.y += h + 16.0;
        }
    }
    let first = graph
        .output_targets(PinRef::output(root, 0))
        .first()
        .map(|p| p.node);
    let anchor = first
        .and_then(|f| graph.node(f))
        .map_or(Vec2::new(COLUMN, 0.0), |n| n.position);
    if let Some(node) = graph.node_mut(root) {
        node.position = anchor - Vec2::new(COLUMN, 0.0);
    }
    for (from, pin, goto) in gotos {
        let Some(source) = graph.node(*from).map(|n| n.position) else {
            continue;
        };
        let width = size(graph, *from).x;
        if let Some(node) = graph.node_mut(*goto) {
            node.position = source + Vec2::new(width + 60.0, 40.0 + f32::from(*pin) * 70.0);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Graph -> asset

/// Saved ids for stage, objective and journal nodes: their titles, or fresh ones.
#[must_use]
pub fn node_ids(graph: &Graph) -> BTreeMap<NodeId, String> {
    let mut taken: BTreeSet<String> = graph
        .nodes()
        .iter()
        .map(|n| n.title.trim().to_owned())
        .filter(|t| !t.is_empty())
        .collect();
    let mut ids = BTreeMap::new();
    for node in graph.nodes() {
        if !matches!(node.archetype.as_str(), STAGE | OBJECTIVE | JOURNAL) {
            continue;
        }
        let title = node.title.trim();
        let id = if title.is_empty() {
            let prefix = prefix(&node.archetype);
            let mut k = taken
                .iter()
                .filter_map(|t| t.strip_prefix(prefix)?.parse::<u32>().ok())
                .max()
                .unwrap_or(0)
                + 1;
            while taken.contains(&format!("{prefix}{k}")) {
                k += 1;
            }
            let id = format!("{prefix}{k}");
            taken.insert(id.clone());
            id
        } else {
            title.to_owned()
        };
        ids.insert(node.id, id);
    }
    ids
}

/// Convert back without extras.
pub fn from_graph(graph: &Graph) -> Result<QuestAsset, Vec<Problem>> {
    from_graph_with(graph, &Extras::default())
}

/// Convert back, restoring texts' keys and forms from `extras`.
pub fn from_graph_with(graph: &Graph, extras: &Extras) -> Result<QuestAsset, Vec<Problem>> {
    let mut problems = Vec::new();
    let roots: Vec<_> = graph
        .nodes()
        .iter()
        .filter(|n| n.archetype == QUEST)
        .collect();
    let [root] = roots.as_slice() else {
        return Err(vec![Problem::error(
            "",
            format!(
                "a quest graph has one Quest node; this one has {}",
                roots.len()
            ),
        )]);
    };
    let literal =
        |node: &super::Node, pin: u16| node.literals.get(&pin).cloned().unwrap_or_default();
    let ids = node_ids(graph);
    let target_of = |node: NodeId, pin: u16| -> Vec<String> {
        graph
            .output_targets(PinRef::output(node, pin))
            .into_iter()
            .filter_map(|p| {
                let target = graph.node(p.node)?;
                match target.archetype.as_str() {
                    GOTO => Some(literal(target, 1).trim().to_owned()),
                    _ => ids.get(&p.node).cloned(),
                }
            })
            .collect()
    };
    let parts_of = |stage: NodeId, pin: u16| -> Vec<NodeId> {
        let mut parts: Vec<NodeId> = graph
            .connections()
            .iter()
            .filter(|w| w.to == PinRef::input(stage, pin))
            .map(|w| w.from.node)
            .collect();
        parts.sort_by(|a, b| {
            let (pa, pb) = (
                graph.node(*a).map_or(Vec2::ZERO, |n| n.position),
                graph.node(*b).map_or(Vec2::ZERO, |n| n.position),
            );
            pa.y.total_cmp(&pb.y)
                .then(pa.x.total_cmp(&pb.x))
                .then(a.cmp(b))
        });
        parts
    };
    let mut asset = QuestAsset::new(literal(root, 0).trim(), "");
    asset.title = extras.text(ROOT, 1, literal(root, 1));
    asset.summary = extras.text(ROOT, 2, literal(root, 2));
    asset.category = literal(root, 3).trim().to_owned();
    asset.giver = literal(root, 4).trim().to_owned();
    asset.region = literal(root, 5).trim().to_owned();
    asset.priority = match literal(root, 6).trim() {
        "" => 0,
        other => other.parse().unwrap_or_else(|_| {
            problems.push(Problem::error(
                "",
                format!("Priority is a whole number, not `{other}`"),
            ));
            0
        }),
    };
    asset.start_when = literal(root, 7);
    asset.repeatable = parse_bool(&literal(root, 8), "Repeatable").unwrap_or_else(|e| {
        problems.push(Problem::error("", e));
        false
    });
    asset.fail_when = literal(root, 9);
    asset.on_start = literal(root, 10);
    asset.on_complete = literal(root, 11);
    asset.on_fail = literal(root, 12);
    asset.note = literal(root, 13);
    let first_stage = graph
        .nodes()
        .iter()
        .find(|n| n.archetype == STAGE)
        .and_then(|n| ids.get(&n.id));
    match target_of(root.id, 0).as_slice() {
        [] => {}
        [one] if Some(one) == first_stage => {}
        [one] => asset.first = one.clone(),
        _ => problems.push(Problem::error("", "First Stage leads to one stage")),
    }
    let mut owners: BTreeMap<NodeId, usize> = BTreeMap::new();
    for node in graph.nodes().iter().filter(|n| n.archetype == STAGE) {
        let id = ids[&node.id].clone();
        let at = format!("stage {id}");
        let mut stage = StageDef {
            id: id.clone(),
            title: extras.text(&id, 3, literal(node, 3)),
            on_enter: literal(node, 4),
            note: literal(node, 10),
            pos: Some(node.position.to_array()),
            ..StageDef::default()
        };
        stage.end = match literal(node, 5).trim() {
            "" => None,
            "complete" => Some(StageEnd::Complete),
            "fail" => Some(StageEnd::Fail),
            other => {
                problems.push(Problem::error(
                    at.clone(),
                    format!("End is complete, fail or empty, not `{other}`"),
                ));
                None
            }
        };
        for k in 0..EXITS {
            let when = literal(node, 6 + k);
            match target_of(node.id, k).as_slice() {
                [] if when.trim().is_empty() => {}
                [] => problems.push(Problem::error(
                    at.clone(),
                    format!(
                        "When {} has a condition but Next {} leads nowhere",
                        k + 1,
                        k + 1
                    ),
                )),
                [to] => stage.next.push(StageLink {
                    when,
                    to: to.clone(),
                }),
                _ => problems.push(Problem::error(
                    at.clone(),
                    format!("Next {} leads to one stage", k + 1),
                )),
            }
        }
        for part in parts_of(node.id, 1) {
            *owners.entry(part).or_insert(0) += 1;
            let Some(line) = graph.node(part) else {
                continue;
            };
            let line_id = ids.get(&part).cloned().unwrap_or_default();
            stage.journal.push(JournalLine {
                text: extras.text(&format!("{id}.{line_id}"), 0, literal(line, 0)),
                tier: literal(line, 1).trim().to_owned(),
                id: line_id,
            });
        }
        for part in parts_of(node.id, 2) {
            *owners.entry(part).or_insert(0) += 1;
            let Some(objective) = graph.node(part) else {
                continue;
            };
            let objective_id = ids.get(&part).cloned().unwrap_or_default();
            let at = format!("{at} / objective {objective_id}");
            let mut report = |message: String| problems.push(Problem::error(at.clone(), message));
            let count = parse_count(&literal(objective, 2)).unwrap_or_else(|e| {
                report(format!("Count: {e}"));
                None
            });
            let marker = parse_marker(&literal(objective, 3)).unwrap_or_else(|e| {
                report(format!("Marker: {e}"));
                None
            });
            let flag = |pin: u16, name: &str, report: &mut dyn FnMut(String)| {
                parse_bool(&literal(objective, pin), name).unwrap_or_else(|e| {
                    report(e);
                    false
                })
            };
            let optional = flag(4, "Optional", &mut report);
            let hidden = flag(5, "Hidden", &mut report);
            let timer_ms = match literal(objective, 7).trim() {
                "" => None,
                other => other.parse().map(Some).unwrap_or_else(|_| {
                    report(format!("Timer (ms) is a whole number, not `{other}`"));
                    None
                }),
            };
            let on_expire = match literal(objective, 8).trim() {
                "" | "fail" => Expire::Fail,
                "complete" => Expire::Complete,
                other => {
                    report(format!("On Expire is fail or complete, not `{other}`"));
                    Expire::Fail
                }
            };
            stage.objectives.push(ObjectiveDef {
                text: extras.text(&objective_id, 0, literal(objective, 0)),
                id: objective_id,
                optional,
                hidden,
                done_when: literal(objective, 1),
                fail_when: literal(objective, 6),
                count,
                marker,
                timer_ms,
                on_expire,
                on_done: literal(objective, 9),
                on_fail: literal(objective, 10),
            });
        }
        asset.stages.push(stage);
    }
    for node in graph
        .nodes()
        .iter()
        .filter(|n| matches!(n.archetype.as_str(), OBJECTIVE | JOURNAL))
    {
        match owners.get(&node.id).copied().unwrap_or(0) {
            1 => {}
            0 => problems.push(Problem::error(
                ids.get(&node.id).cloned().unwrap_or_default(),
                "wire this into the stage it belongs to",
            )),
            _ => problems.push(Problem::error(
                ids.get(&node.id).cloned().unwrap_or_default(),
                "belongs to one stage only",
            )),
        }
    }
    if problems.is_empty() {
        Ok(asset)
    } else {
        Err(problems)
    }
}

/// Everything wrong that can be told without the game's manifest.
#[must_use]
pub fn problems(graph: &Graph) -> Vec<Problem> {
    match from_graph(graph) {
        Ok(asset) => {
            let mut problems = asset.structural_problems();
            for stage in asset.unreachable() {
                problems.push(Problem::warning(
                    format!("stage {stage}"),
                    "no path from the first stage reaches this stage",
                ));
            }
            problems
        }
        Err(problems) => problems,
    }
}

/// The Preview and Apply gate.
pub fn validate(graph: &Graph) -> Result<(), String> {
    let errors: Vec<Problem> = problems(graph).into_iter().filter(|p| !p.warning).collect();
    match errors.first() {
        None => Ok(()),
        Some(first) if errors.len() == 1 => Err(first.to_string()),
        Some(first) => Err(format!("{} problems · {first}", errors.len())),
    }
}

/// Graph node for a problem's location (`stage s2`, `stage s2 / objective o1`) or a bare id.
#[must_use]
pub fn find_node(graph: &Graph, at: &str) -> Option<NodeId> {
    let wanted = at.rsplit(['/', ' ']).find(|s| !s.is_empty())?;
    node_ids(graph)
        .into_iter()
        .find_map(|(node, id)| (id == wanted).then_some(node))
}

/// A starter quest: one stage with an objective, then the end.
#[must_use]
pub fn default_surface() -> GraphSurface {
    let mut asset = QuestAsset::new("new_quest", "New quest");
    asset.stages = vec![
        StageDef {
            id: "s1".into(),
            journal: vec![JournalLine {
                id: "j1".into(),
                text: "Someone asked for help.".into(),
                tier: String::new(),
            }],
            objectives: vec![ObjectiveDef {
                id: "o1".into(),
                text: "Do what was asked".into(),
                ..ObjectiveDef::default()
            }],
            next: vec![StageLink {
                when: String::new(),
                to: "s2".into(),
            }],
            ..StageDef::default()
        },
        StageDef {
            id: "s2".into(),
            end: Some(StageEnd::Complete),
            ..StageDef::default()
        },
    ];
    surface(&asset).0
}

/// A surface for an asset, and the extras a lossless save needs.
#[must_use]
pub fn surface(asset: &QuestAsset) -> (GraphSurface, Extras) {
    let (graph, extras) = to_graph(asset);
    let mut surface = GraphSurface::new(catalogue());
    surface.graph = graph;
    (surface, extras)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> QuestAsset {
        QuestAsset::from_json(
            r#"{
            "version": 1, "id": "varga_job", "title": {"key": "custom.title", "source": "The Varga Strongroom"},
            "summary": "Get in.", "category": "contract", "giver": "foreman", "priority": 3, "repeatable": true,
            "on_complete": "rep.varga += 5",
            "stages": [
              {"id": "offer", "title": "The offer", "journal": [{"id": "j1", "text": "A foreman wants a lock opened.", "tier": "witnessed"}],
               "objectives": [{"id": "hear", "text": "Hear him out", "done_when": "met.foreman"}],
               "next": [{"to": "inside"}]},
              {"id": "inside",
               "objectives": [
                 {"id": "guards", "text": "Deal with the guards", "count": {"fact": "varga.guards_down", "target": 3},
                  "marker": {"anchor": "varga_quay", "radius": 4.0}},
                 {"id": "unseen", "text": "Stay unseen", "optional": true, "fail_when": "varga.alarm"},
                 {"id": "door", "text": "Reach the strongroom", "marker": {"position": [12.5, 0.0, -3.0], "radius": 2.0, "arrive": true},
                  "timer_ms": 60000, "on_expire": "complete", "hidden": true}
               ],
               "next": [{"when": "varga.alarm", "to": "loud"}, {"when": "done('varga_job', 'door')", "to": "quiet"}, {"when": "varga.retry", "to": "offer"}]},
              {"id": "quiet", "end": "complete"},
              {"id": "loud", "journal": [{"id": "j1", "text": "The quay heard.", "tier": "retold"}], "end": "fail"}
            ]}"#,
        )
        .unwrap()
    }

    fn without_positions(mut asset: QuestAsset) -> QuestAsset {
        for stage in &mut asset.stages {
            stage.pos = None;
        }
        asset
    }

    #[test]
    fn a_quest_round_trips_through_the_graph() {
        let asset = sample();
        let (graph, extras) = to_graph(&asset);
        let back = from_graph_with(&graph, &extras).unwrap();
        assert_eq!(without_positions(back.clone()), asset);
        let (graph, extras) = to_graph(&back);
        assert_eq!(from_graph_with(&graph, &extras).unwrap(), back);
        assert!(graph.topological_order().is_some());
        let gotos = graph.nodes().iter().filter(|n| n.archetype == GOTO).count();
        assert_eq!(gotos, 1, "inside -> offer is a link back");
    }

    #[test]
    fn objectives_order_as_drawn_and_must_belong_to_one_stage() {
        let (mut graph, extras) = to_graph(&sample());
        let guards = find_node(&graph, "guards").unwrap();
        let door_y = graph
            .node(find_node(&graph, "door").unwrap())
            .unwrap()
            .position
            .y;
        graph.node_mut(guards).unwrap().position.y = door_y + 1.0;
        let back = from_graph_with(&graph, &extras).unwrap();
        let order: Vec<&str> = back
            .stage("inside")
            .unwrap()
            .objectives
            .iter()
            .map(|o| o.id.as_str())
            .collect();
        assert_eq!(order, ["unseen", "door", "guards"]);
        let c = catalogue();
        graph.add(&c, OBJECTIVE, Vec2::ZERO).unwrap();
        let problems = from_graph(&graph).unwrap_err();
        assert!(
            problems
                .iter()
                .any(|p| p.message.contains("wire this into")),
            "{problems:?}"
        );
    }

    #[test]
    fn markers_and_counts_read_as_one_line() {
        let marker = parse_marker("strongroom_door r=2 arrive").unwrap().unwrap();
        assert_eq!(
            (marker.anchor.as_str(), marker.radius, marker.arrive),
            ("strongroom_door", 2.0, true)
        );
        assert_eq!(format_marker(&marker), "strongroom_door r=2 arrive");
        let at = parse_marker("(12.5, 0, -3) r=10").unwrap().unwrap();
        assert_eq!(at.position, Some([12.5, 0.0, -3.0]));
        assert_eq!(format_marker(&at), "(12.5, 0, -3) r=10");
        assert!(parse_marker("(1, 2) r=3").is_err());
        assert!(parse_marker("door radius=3").is_err());
        assert_eq!(
            parse_count("varga.guards_down >= 3")
                .unwrap()
                .unwrap()
                .target,
            3
        );
        assert!(parse_count("x > 3").is_err());
    }

    #[test]
    fn exits_without_wires_and_bad_fields_are_located() {
        let (mut graph, _) = to_graph(&sample());
        let inside = find_node(&graph, "inside").unwrap();
        graph
            .node_mut(inside)
            .unwrap()
            .literals
            .insert(9, "true".into());
        graph
            .node_mut(inside)
            .unwrap()
            .literals
            .insert(5, "maybe".into());
        let problems = from_graph(&graph).unwrap_err();
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems.iter().all(|p| p.at == "stage inside"));
        assert_eq!(find_node(&graph, "stage inside"), Some(inside));
    }

    #[test]
    fn the_starter_is_valid() {
        assert!(validate(&default_surface().graph).is_ok());
    }
}
