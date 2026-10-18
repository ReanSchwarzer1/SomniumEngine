//! Dialogue authoring on the one graph surface.
//!
//! `somnium_story`'s dialogue asset (`*.somdialogue`) is the source of truth on disk; this module is the catalogue
//! that draws it and the conversion both ways, which round-trips without loss:
//!
//! - every asset node is a graph node whose header (the node title) is its id;
//! - `next`, `else` and `fail` lists are wires, ordered as drawn: the target nearest the top comes first, which is
//!   how a menu's options read top to bottom on the canvas and in the game;
//! - the graph surface refuses cycles, so a link back up the conversation (a hub, "ask something else") is drawn
//!   as an automatic **Go To** node; saving turns it back into a plain link. A Go To the writer placed stays one;
//! - a link into another dialogue (`dialogue/node`) is always a Go To;
//! - fields a node rarely needs on the canvas (voice, language, tags, notes) are inspector-only.

use super::{Catalogue, Graph, GraphSurface, NodeArchetype, NodeId, PinArchetype, PinRef, PinType};
use glam::Vec2;
use somnium_story::dialogue::{
    ActionDef, BranchDef, CallDef, CheckDef, DialogueAsset, EndDef, JumpDef, LineDef, NodeDef,
    OptionDef, RollDef,
};
use somnium_story::problem::Problem;
use somnium_story::text::LocalText;
use somnium_story::value::Value;
use std::collections::{BTreeMap, BTreeSet};

/// The catalogue id written into a dialogue graph.
pub const CATALOGUE: &str = "somnium.dialogue";

const FLOW: PinType = PinType::Opaque("dialogue.flow");
const TEXT: PinType = PinType::Opaque("dialogue.text");

pub const START: &str = "dialogue.start";
pub const ENTRY: &str = "dialogue.entry";
pub const LINE: &str = "dialogue.line";
pub const OPTION: &str = "dialogue.option";
pub const BRANCH: &str = "dialogue.branch";
pub const ACTION: &str = "dialogue.action";
pub const CALL: &str = "dialogue.call";
pub const JUMP: &str = "dialogue.jump";
pub const END: &str = "dialogue.end";

const WIDE: u16 = 300;
const COLUMN: f32 = 380.0;
const ROW: f32 = 40.0;

fn text(name: &'static str, tooltip: &'static str) -> PinArchetype {
    PinArchetype::new(name, TEXT)
        .with_default("")
        .with_tooltip(tooltip)
}

fn flow_in() -> PinArchetype {
    PinArchetype::new("In", FLOW).many()
}

fn note() -> PinArchetype {
    text(
        "Note",
        "A note for writers and translators; never shown to players.",
    )
    .detail()
    .multiline()
}

/// The dialogue catalogue.
#[must_use]
pub fn catalogue() -> Catalogue {
    let mut c = Catalogue::new(CATALOGUE);
    c.register(
        NodeArchetype::new(START, "Dialogue", "Dialogue")
            .with_keywords(&["start", "entry", "root"])
            .with_input(text("Id", "The dialogue's id: what calls, jumps and saves name it by."))
            .with_input(text("Title", "What the editor and lint call it; not shown to players."))
            .with_input(
                PinArchetype::new("Ambient", TEXT)
                    .with_default("false")
                    .with_tooltip("true: plays without the player's input (chatter, a radio); the presenter auto-advances.")
                    .detail(),
            )
            .with_input(note())
            .with_output(PinArchetype::new("Entry", FLOW))
            .with_output(PinArchetype::new("On Interrupt", FLOW))
            .as_root()
            .with_width(WIDE),
    );
    c.register(
        NodeArchetype::new(ENTRY, "Named Entry", "Dialogue")
            .with_keywords(&["entry", "revisit", "start"])
            .with_input(text(
                "Name",
                "Another way in: Conversation::start(story, dialogue, name).",
            ))
            .with_output(PinArchetype::new("Leads To", FLOW)),
    );
    c.register(
        NodeArchetype::new(LINE, "Line", "Dialogue")
            .with_keywords(&["say", "speak", "text", "line"])
            .with_input(flow_in())
            .with_input(text("Speaker", "A cast id from the story manifest; empty for the narrator."))
            .with_input(text("Expression", "Portrait expression; empty for the speaker's default. [expr=x] in the text changes it mid-line."))
            .with_input(
                text("Text", "The line in the source language. Rich-text markup; {fact.id} placeholders; [pause=ms] cues.")
                    .multiline(),
            )
            .with_input(text("When", "Entered only when this holds, so a line can vary with the world."))
            .with_input(text("Effects", "Applied as the line shows: fact = x; fact += 1; set add 'x'; start_quest('id')").multiline())
            .with_input(text("Voice", "A voice clip, project-relative.").detail())
            .with_input(text("Language", "The language spoken; empty for the speaker's.").detail())
            .with_input(text("Tags", "Comma-separated tags for the game and presenter: whisper, shout, thought.").detail())
            .with_input(note())
            .with_output(PinArchetype::new("Next", FLOW))
            .with_width(WIDE),
    );
    c.register(
        NodeArchetype::new(OPTION, "Choice", "Dialogue")
            .with_keywords(&["option", "choice", "answer", "menu", "reply"])
            .with_input(flow_in())
            .with_input(text("Text", "What the player chooses.").multiline())
            .with_input(text("When", "Offered only when this holds."))
            .with_input(text(
                "Check",
                "category: fact >= value (gated), or category: fact + d20 >= 11 (a roll; give it a Fail wire)",
            ))
            .with_input(text("Effects", "Applied when chosen.").multiline())
            .with_input(
                PinArchetype::new("Once", TEXT)
                    .with_default("false")
                    .with_tooltip("true: gone once chosen.")
                    .detail(),
            )
            .with_input(text("Show Locked", "true or false: show this choice greyed when it is unavailable. Empty: shown for checks, hidden otherwise.").detail())
            .with_input(text("Timeout (ms)", "A timed menu: this hidden choice is taken when time runs out.").detail())
            .with_input(text("Check Label", "What the player reads on the check (Mending 3); empty: made from the check.").detail())
            .with_input(text("Tags", "Comma-separated tags.").detail())
            .with_input(note())
            .with_output(PinArchetype::new("Next", FLOW))
            .with_output(PinArchetype::new("Fail", FLOW))
            .with_width(WIDE),
    );
    c.register(
        NodeArchetype::new(BRANCH, "Branch", "Flow")
            .with_keywords(&["if", "condition", "fork"])
            .with_input(flow_in())
            .with_input(text("When", "The condition."))
            .with_input(note())
            .with_output(PinArchetype::new("True", FLOW))
            .with_output(PinArchetype::new("False", FLOW)),
    );
    c.register(
        NodeArchetype::new(ACTION, "Action", "Flow")
            .with_keywords(&["effect", "set", "camera", "wait", "command"])
            .with_input(flow_in())
            .with_input(text("Effects", "Facts and commands, applied when reached.").multiline())
            .with_input(
                PinArchetype::new("Wait", TEXT)
                    .with_default("false")
                    .with_tooltip("true: hold the conversation until the game resumes it."),
            )
            .with_input(note())
            .with_output(PinArchetype::new("Next", FLOW))
            .with_width(WIDE),
    );
    c.register(
        NodeArchetype::new(CALL, "Call Dialogue", "Flow")
            .with_keywords(&["subgraph", "call", "include"])
            .with_input(flow_in())
            .with_input(text("Dialogue", "The dialogue to play."))
            .with_input(text(
                "Entry",
                "A named entry or node in it; empty for its entry.",
            ))
            .with_input(note())
            .with_output(PinArchetype::new("Then", FLOW)),
    );
    c.register(
        NodeArchetype::new(JUMP, "Go To", "Flow")
            .with_keywords(&["jump", "goto", "loop", "back"])
            .with_input(flow_in())
            .with_input(text("Target", "A node here, or dialogue/node to hand over to another dialogue."))
            .with_input(
                PinArchetype::new("Auto", TEXT)
                    .with_default("false")
                    .with_tooltip("true: drawn by the editor for a link back up the graph; saved as a plain link.")
                    .detail(),
            )
            .with_input(note()),
    );
    c.register(
        NodeArchetype::new(END, "End", "Flow")
            .with_keywords(&["end", "stop", "exit", "result"])
            .with_input(flow_in())
            .with_input(text(
                "Result",
                "What the game reads when the conversation ends here.",
            ))
            .with_input(note()),
    );
    c
}

/// The id prefix new nodes of an archetype get (`n3`, `o2`).
#[must_use]
pub fn id_prefix(archetype: &str) -> &'static str {
    match archetype {
        LINE => "n",
        OPTION => "o",
        BRANCH => "b",
        ACTION => "a",
        CALL => "c",
        JUMP => "j",
        END => "e",
        _ => "x",
    }
}

/// A fresh id for a new node of `archetype`, above every id with that prefix.
#[must_use]
pub fn fresh_id(graph: &Graph, archetype: &str) -> String {
    let prefix = id_prefix(archetype);
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

/// `category: fact >= value` or `category: fact + 2 + d20 >= 11`.
#[must_use]
pub fn format_check(check: &CheckDef) -> String {
    let prefix = if check.category.is_empty() {
        String::new()
    } else {
        format!("{}: ", check.category)
    };
    match (&check.roll, &check.at_least) {
        (Some(roll), _) => {
            let bonus = if roll.bonus == 0 {
                String::new()
            } else if roll.bonus > 0 {
                format!(" + {}", roll.bonus)
            } else {
                format!(" - {}", -roll.bonus)
            };
            format!(
                "{prefix}{}{bonus} + d{} >= {}",
                check.fact, roll.sides, roll.difficulty
            )
        }
        (None, Some(value)) => format!("{prefix}{} >= {}", check.fact, value),
        (None, None) => format!("{prefix}{}", check.fact),
    }
}

/// Parse [`format_check`]'s text. Blank is no check.
pub fn parse_check(text: &str, label: LocalText) -> Result<Option<CheckDef>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let (category, body) = match text.split_once(':') {
        Some((category, body)) if !category.contains(['>', '+', ' ']) => {
            (category.trim(), body.trim())
        }
        _ => ("", text),
    };
    let (left, right) = body.split_once(">=").ok_or_else(|| {
        format!("a check reads `fact >= value` or `fact + d20 >= 11`, not `{text}`")
    })?;
    let right = right.trim();
    let mut parts = left.split('+').map(str::trim);
    let fact = parts.next().unwrap_or("").to_owned();
    let mut bonus = 0i64;
    let mut sides = None;
    for part in parts {
        if let Some(die) = part.strip_prefix('d') {
            sides = Some(
                die.parse::<u32>()
                    .map_err(|_| format!("`{part}` is not a die like d20"))?,
            );
        } else {
            bonus += part
                .replace(' ', "")
                .parse::<i64>()
                .map_err(|_| format!("`{part}` is not a number or a die"))?;
        }
    }
    let (fact, penalty) = match fact.split_once('-') {
        Some((fact, penalty)) => (
            fact.trim().to_owned(),
            penalty
                .trim()
                .parse::<i64>()
                .map_err(|_| format!("`{penalty}` is not a number"))?,
        ),
        None => (fact, 0),
    };
    if fact.is_empty() {
        return Err("a check names a fact".into());
    }
    let (at_least, roll) = match sides {
        Some(sides) => (
            None,
            Some(RollDef {
                sides,
                difficulty: right
                    .parse()
                    .map_err(|_| format!("`{right}` is not a difficulty"))?,
                bonus: bonus - penalty,
            }),
        ),
        None => {
            if bonus != 0 || penalty != 0 {
                return Err("a bonus needs a die: `fact + 2 + d20 >= 11`".into());
            }
            let value = right
                .parse::<i64>()
                .map(Value::Int)
                .unwrap_or_else(|_| Value::Text(right.trim_matches(['\'', '"']).to_owned()));
            (Some(value), None)
        }
    };
    Ok(Some(CheckDef {
        category: category.to_owned(),
        label,
        fact,
        at_least,
        roll,
    }))
}

fn parse_bool(text: &str, field: &str) -> Result<bool, String> {
    match text.trim().to_ascii_lowercase().as_str() {
        "" | "false" | "no" | "0" => Ok(false),
        "true" | "yes" | "1" => Ok(true),
        other => Err(format!("{field} is true or false, not `{other}`")),
    }
}

fn parse_tags(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A text's source, or its key and source when it carries more than the source (forms, a note, an explicit key):
/// the graph edits the source; the rest is kept on the side so a save loses nothing.
fn local(text: &LocalText) -> String {
    text.source.clone()
}

// ---------------------------------------------------------------------------------------------------------------
// Asset -> graph

/// Draw an asset as a graph. Texts' keys, forms and translator notes are not graph fields: they ride along in
/// `extras`, which [`from_graph_with`] puts back.
#[must_use]
pub fn to_graph(asset: &DialogueAsset) -> (Graph, Extras) {
    let c = catalogue();
    let mut graph = Graph::new();
    let mut extras = Extras::default();
    let mut by_id: BTreeMap<&str, NodeId> = BTreeMap::new();

    let start = graph
        .add(&c, START, Vec2::ZERO)
        .expect("catalogue has a start");
    set(&mut graph, start, 0, &asset.id);
    set(&mut graph, start, 1, &asset.title);
    set(
        &mut graph,
        start,
        2,
        if asset.ambient { "true" } else { "false" },
    );
    set(&mut graph, start, 3, &asset.note);

    for node in &asset.nodes {
        let archetype = match node {
            NodeDef::Line(_) => LINE,
            NodeDef::Option(_) => OPTION,
            NodeDef::Branch(_) => BRANCH,
            NodeDef::Action(_) => ACTION,
            NodeDef::Call(_) => CALL,
            NodeDef::Jump(_) => JUMP,
            NodeDef::End(_) => END,
        };
        let id = graph
            .add(&c, archetype, Vec2::ZERO)
            .expect("archetype registered");
        graph.node_mut(id).expect("just added").title = node.id().to_owned();
        by_id.insert(node.id(), id);
        match node {
            NodeDef::Line(line) => {
                set(&mut graph, id, 1, &line.speaker);
                set(&mut graph, id, 2, &line.expression);
                set(&mut graph, id, 3, &local(&line.text));
                set(&mut graph, id, 4, &line.when);
                set(&mut graph, id, 5, &line.effects);
                set(&mut graph, id, 6, &line.voice);
                set(&mut graph, id, 7, &line.language);
                set(&mut graph, id, 8, &line.tags.join(", "));
                set(&mut graph, id, 9, &line.note);
                extras.texts.insert((line.id.clone(), 0), line.text.clone());
            }
            NodeDef::Option(option) => {
                set(&mut graph, id, 1, &local(&option.text));
                set(&mut graph, id, 2, &option.when);
                set(
                    &mut graph,
                    id,
                    3,
                    &option.check.as_ref().map(format_check).unwrap_or_default(),
                );
                set(&mut graph, id, 4, &option.effects);
                set(
                    &mut graph,
                    id,
                    5,
                    if option.once { "true" } else { "false" },
                );
                set(
                    &mut graph,
                    id,
                    6,
                    &option
                        .show_locked
                        .map(|b| b.to_string())
                        .unwrap_or_default(),
                );
                set(
                    &mut graph,
                    id,
                    7,
                    &option
                        .timeout_ms
                        .map(|ms| ms.to_string())
                        .unwrap_or_default(),
                );
                if let Some(check) = &option.check {
                    set(&mut graph, id, 8, &local(&check.label));
                    extras
                        .texts
                        .insert((option.id.clone(), 8), check.label.clone());
                }
                set(&mut graph, id, 9, &option.tags.join(", "));
                set(&mut graph, id, 10, &option.note);
                extras
                    .texts
                    .insert((option.id.clone(), 1), option.text.clone());
            }
            NodeDef::Branch(branch) => {
                set(&mut graph, id, 1, &branch.when);
                set(&mut graph, id, 2, &branch.note);
            }
            NodeDef::Action(action) => {
                set(&mut graph, id, 1, &action.effects);
                set(
                    &mut graph,
                    id,
                    2,
                    if action.wait { "true" } else { "false" },
                );
                set(&mut graph, id, 3, &action.note);
            }
            NodeDef::Call(call) => {
                set(&mut graph, id, 1, &call.dialogue);
                set(&mut graph, id, 2, &call.entry);
                set(&mut graph, id, 3, &call.note);
            }
            NodeDef::Jump(jump) => {
                set(&mut graph, id, 1, &jump.target);
                set(&mut graph, id, 3, &jump.note);
            }
            NodeDef::End(end) => {
                set(&mut graph, id, 1, &end.result);
                set(&mut graph, id, 2, &end.note);
            }
        }
    }

    // Wires, explored depth first from the ways in, so the links that run back up the conversation are the ones
    // drawn as automatic Go Tos (removing a depth-first search's back edges leaves a graph without cycles).
    let mut roots: Vec<(NodeId, u16, String)> = Vec::new();
    if let Some(entry) = asset.entry_node("") {
        roots.push((start, 0, entry.to_owned()));
    }
    if !asset.on_interrupt.is_empty() {
        roots.push((start, 1, asset.on_interrupt.clone()));
    }
    let mut entry_nodes = Vec::new();
    for (name, target) in &asset.entries {
        let entry = graph.add(&c, ENTRY, Vec2::ZERO).expect("registered");
        set(&mut graph, entry, 0, name);
        entry_nodes.push(entry);
        roots.push((entry, 0, target.clone()));
    }
    let links_of = |id: &str| -> Vec<(u16, String)> {
        asset
            .node(id)
            .map(|node| {
                node.links()
                    .into_iter()
                    .filter_map(|(role, targets)| match role {
                        "next" => Some((0u16, targets)),
                        "else" | "fail" => Some((1u16, targets)),
                        _ => None,
                    })
                    .flat_map(|(pin, targets)| targets.iter().map(move |t| (pin, t.clone())))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut wires: Vec<(NodeId, u16, String)> = Vec::new();
    let mut autos: Vec<(NodeId, u16, String)> = Vec::new();
    let mut state: BTreeMap<&str, bool> = BTreeMap::new(); // false: on the stack; true: finished
    let mut starts: Vec<(Option<&str>, NodeId, u16, String)> = roots
        .into_iter()
        .map(|(from, pin, target)| (None, from, pin, target))
        .collect();
    starts.extend(
        asset
            .nodes
            .iter()
            .map(|n| (Some(n.id()), start, u16::MAX, n.id().to_owned())),
    );
    for (_, from, pin, target) in starts {
        if pin != u16::MAX {
            match (by_id.get(target.as_str()), state.get(target.as_str())) {
                (None, _) => {
                    autos.push((from, pin, target));
                    continue;
                }
                (Some(_), Some(_)) => {
                    wires.push((from, pin, target));
                    continue;
                }
                (Some(_), None) => wires.push((from, pin, target.clone())),
            }
        } else if state.contains_key(target.as_str()) {
            continue;
        }
        // Iterative depth-first walk from `target`.
        let Some((root_id, _)) = by_id.get_key_value(target.as_str()) else {
            continue;
        };
        let mut stack: Walk<'_> = vec![(*root_id, links_of(root_id), 0)];
        state.insert(*root_id, false);
        while let Some(top) = stack.last_mut() {
            let node = top.0;
            let step = top.1.get(top.2).cloned();
            top.2 += 1;
            let Some((pin, link)) = step else {
                state.insert(node, true);
                stack.pop();
                continue;
            };
            let from = by_id[node];
            match by_id.get_key_value(link.as_str()) {
                None => autos.push((from, pin, link)),
                Some((&to, _)) => match state.get(to) {
                    Some(false) => autos.push((from, pin, link)),
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
    for (from, pin, target) in &wires {
        let to = by_id[target.as_str()];
        if graph
            .connect(&c, PinRef::output(*from, *pin), PinRef::input(to, 0))
            .is_err()
        {
            autos.push((*from, *pin, target.clone()));
        }
    }
    let mut auto_nodes = Vec::new();
    for (from, pin, target) in autos {
        let jump = graph.add(&c, JUMP, Vec2::ZERO).expect("registered");
        set(&mut graph, jump, 1, &target);
        set(&mut graph, jump, 2, "true");
        graph
            .connect(&c, PinRef::output(from, pin), PinRef::input(jump, 0))
            .expect("a fresh Go To accepts its one wire");
        auto_nodes.push((from, jump));
    }

    layout(
        &mut graph,
        &c,
        asset,
        start,
        &entry_nodes,
        &auto_nodes,
        &by_id,
    );
    (graph, extras)
}

fn set(graph: &mut Graph, node: NodeId, pin: u16, value: &str) {
    if let Some(node) = graph.node_mut(node) {
        node.literals.insert(pin, value.to_owned());
    }
}

/// Lay a dialogue out left to right: each conversation path a row, siblings top to bottom in their authored order
/// (the order the save reads back). Nodes with a saved position keep it; the rest go below them. The start and the
/// named entries sit left of where they lead; automatic Go Tos sit right of the node they leave.
fn layout(
    graph: &mut Graph,
    c: &Catalogue,
    asset: &DialogueAsset,
    start: NodeId,
    entries: &[NodeId],
    autos: &[(NodeId, NodeId)],
    by_id: &BTreeMap<&str, NodeId>,
) {
    let size = |graph: &Graph, id: NodeId| {
        super::geometry::layout_node(graph.node(id).expect("node exists"), c)
            .map_or(Vec2::new(180.0, 120.0), |l| l.bounds.size())
    };
    let auto_set: BTreeSet<NodeId> = autos.iter().map(|(_, jump)| *jump).collect();
    let children = |graph: &Graph, id: NodeId| -> Vec<NodeId> {
        let mut out = Vec::new();
        for pin in [0u16, 1] {
            let mut targets: Vec<NodeId> = graph
                .output_targets(PinRef::output(id, pin))
                .into_iter()
                .map(|p| p.node)
                .filter(|t| !auto_set.contains(t))
                .collect();
            // Authored order, which is the order these were wired in.
            let order: Vec<String> = asset
                .node(graph.node(id).map_or("", |n| n.title.as_str()))
                .map(|n| {
                    n.links()
                        .into_iter()
                        .filter(|(role, _)| (*role == "next") == (pin == 0) && *role != "target")
                        .flat_map(|(_, t)| t.to_vec())
                        .collect()
                })
                .unwrap_or_default();
            targets.sort_by_key(|t| {
                let title = graph.node(*t).map_or("", |n| n.title.as_str());
                order.iter().position(|o| o == title).unwrap_or(usize::MAX)
            });
            out.extend(targets);
        }
        out
    };

    // Rows: a node shares its first child's row; a leaf ends the row.
    let mut placed: BTreeMap<NodeId, Vec2> = BTreeMap::new();
    let mut y = 0.0f32;
    let mut row_height = 0.0f32;
    let mut roots: Vec<NodeId> = Vec::new();
    for from in std::iter::once(start).chain(entries.iter().copied()) {
        for pin in [0u16, 1] {
            roots.extend(
                graph
                    .output_targets(PinRef::output(from, pin))
                    .iter()
                    .map(|p| p.node),
            );
        }
    }
    roots.extend(asset.nodes.iter().map(|n| by_id[n.id()]));
    for root in roots {
        if placed.contains_key(&root) || auto_set.contains(&root) {
            continue;
        }
        let mut stack = vec![(root, 1usize)];
        while let Some((node, depth)) = stack.pop() {
            if placed.contains_key(&node) {
                continue;
            }
            placed.insert(node, Vec2::new(depth as f32 * COLUMN, y));
            row_height = row_height.max(size(graph, node).y);
            let kids: Vec<NodeId> = children(graph, node)
                .into_iter()
                .filter(|k| !placed.contains_key(k))
                .collect();
            if kids.is_empty() {
                y += row_height + ROW;
                row_height = 0.0;
            }
            for kid in kids.into_iter().rev() {
                stack.push((kid, depth + 1));
            }
        }
    }

    // Saved positions win; everything else goes below them.
    let saved: BTreeMap<NodeId, Vec2> = asset
        .nodes
        .iter()
        .filter_map(|n| n.pos().map(|p| (by_id[n.id()], Vec2::from_array(p))))
        .collect();
    if !saved.is_empty() {
        let bottom = saved
            .iter()
            .map(|(id, p)| p.y + size(graph, *id).y)
            .fold(f32::MIN, f32::max)
            + ROW * 3.0;
        let left = saved.values().map(|p| p.x).fold(f32::MAX, f32::min);
        for (id, position) in &mut placed {
            if !saved.contains_key(id) {
                *position += Vec2::new(left - COLUMN, bottom);
            }
        }
        placed.extend(saved.iter().map(|(id, p)| (*id, *p)));
    }
    for (id, position) in &placed {
        if let Some(node) = graph.node_mut(*id) {
            node.position = *position;
        }
    }

    // Wire order is drawing order: restack any fan-out whose saved positions disagree with its authored list.
    for node in &asset.nodes {
        let from = by_id[node.id()];
        for pin in [0u16, 1] {
            let targets: Vec<NodeId> = children(graph, from)
                .into_iter()
                .filter(|t| {
                    graph
                        .output_targets(PinRef::output(from, pin))
                        .contains(&PinRef::input(*t, 0))
                })
                .collect();
            if targets.len() < 2 || ordered_targets(graph, &targets) == targets {
                continue;
            }
            let mut at = targets
                .iter()
                .filter_map(|t| graph.node(*t).map(|n| n.position.y))
                .fold(f32::MAX, f32::min);
            for target in &targets {
                let h = size(graph, *target).y;
                if let Some(n) = graph.node_mut(*target) {
                    n.position.y = at;
                }
                at += h + ROW;
            }
        }
    }

    // The start and named entries: left of what they lead to, never on top of one another.
    let lead = |graph: &Graph, from: NodeId| {
        graph
            .output_targets(PinRef::output(from, 0))
            .first()
            .and_then(|p| graph.node(p.node))
            .map(|n| n.position)
    };
    let mut column_bottom = f32::MIN;
    for from in std::iter::once(start).chain(entries.iter().copied()) {
        let target = lead(graph, from).unwrap_or(Vec2::new(COLUMN, column_bottom.max(0.0)));
        let h = size(graph, from).y;
        let top = target.y.max(column_bottom + ROW);
        if let Some(n) = graph.node_mut(from) {
            n.position = Vec2::new(target.x - COLUMN, top);
        }
        column_bottom = top + h;
    }

    // Automatic Go Tos: just right of the node they leave, stacked.
    let mut per_source: BTreeMap<NodeId, usize> = BTreeMap::new();
    for (from, jump) in autos {
        let Some(source) = graph.node(*from).map(|n| n.position) else {
            continue;
        };
        let width = size(graph, *from).x;
        let k = per_source.entry(*from).or_insert(0);
        let at = source + Vec2::new(width + 60.0, *k as f32 * (size(graph, *jump).y + 16.0));
        *k += 1;
        if let Some(n) = graph.node_mut(*jump) {
            n.position = at;
        }
    }
}

/// A depth-first walk's stack: the node, its outgoing `(pin, target)` links, and the next one to follow.
pub(super) type Walk<'a> = Vec<(&'a str, Vec<(u16, String)>, usize)>;

/// Targets in drawing order: top to bottom, then left to right, then creation.
fn ordered_targets(graph: &Graph, targets: &[NodeId]) -> Vec<NodeId> {
    let mut ordered = targets.to_vec();
    ordered.sort_by(|a, b| {
        let (pa, pb) = (
            graph.node(*a).map_or(Vec2::ZERO, |n| n.position),
            graph.node(*b).map_or(Vec2::ZERO, |n| n.position),
        );
        pa.y.total_cmp(&pb.y)
            .then(pa.x.total_cmp(&pb.x))
            .then(a.cmp(b))
    });
    ordered
}

// ---------------------------------------------------------------------------------------------------------------
// Graph -> asset

/// What a graph does not hold of an asset's texts: explicit keys, plural and gender forms and translator notes,
/// by `(node id, pin)`. Kept beside the graph by the editor so a save is lossless.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extras {
    pub texts: BTreeMap<(String, u16), LocalText>,
}

impl Extras {
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

/// Convert a graph back to the asset, without extras.
pub fn from_graph(graph: &Graph) -> Result<DialogueAsset, Vec<Problem>> {
    from_graph_with(graph, &Extras::default())
}

/// Whether a node is a Go To the editor drew for a back link.
fn is_auto(node: &super::Node) -> bool {
    node.archetype == JUMP
        && node
            .literals
            .get(&2)
            .is_some_and(|v| parse_bool(v, "Auto").unwrap_or(false))
}

/// The node id a graph node saves as: its title, or a fresh one for a node not yet named.
#[must_use]
pub fn node_ids(graph: &Graph) -> BTreeMap<NodeId, String> {
    let mut ids = BTreeMap::new();
    let mut taken: BTreeSet<String> = graph
        .nodes()
        .iter()
        .map(|n| n.title.trim().to_owned())
        .filter(|t| !t.is_empty())
        .collect();
    for node in graph.nodes() {
        if matches!(node.archetype.as_str(), START | ENTRY) || is_auto(node) {
            continue;
        }
        let title = node.title.trim();
        let id = if title.is_empty() {
            let prefix = id_prefix(&node.archetype);
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

/// Convert a graph back to the asset, restoring texts' keys and forms from `extras`.
pub fn from_graph_with(graph: &Graph, extras: &Extras) -> Result<DialogueAsset, Vec<Problem>> {
    let mut problems = Vec::new();
    let starts: Vec<_> = graph
        .nodes()
        .iter()
        .filter(|n| n.archetype == START)
        .collect();
    let [start] = starts.as_slice() else {
        return Err(vec![Problem::error(
            "",
            format!(
                "a dialogue graph has one Dialogue node; this one has {}",
                starts.len()
            ),
        )]);
    };
    let literal =
        |node: &super::Node, pin: u16| node.literals.get(&pin).cloned().unwrap_or_default();
    let ids = node_ids(graph);
    let auto: BTreeMap<NodeId, String> = graph
        .nodes()
        .iter()
        .filter(|n| is_auto(n))
        .map(|n| (n.id, literal(n, 1).trim().to_owned()))
        .collect();
    let targets = |node: NodeId, pin: u16| -> Vec<String> {
        let found: Vec<NodeId> = graph
            .output_targets(PinRef::output(node, pin))
            .into_iter()
            .map(|p| p.node)
            .collect();
        ordered_targets(graph, &found)
            .into_iter()
            .filter_map(|t| auto.get(&t).cloned().or_else(|| ids.get(&t).cloned()))
            .collect()
    };
    let mut asset = DialogueAsset::new(literal(start, 0).trim());
    asset.title = literal(start, 1);
    asset.ambient = parse_bool(&literal(start, 2), "Ambient").unwrap_or_else(|e| {
        problems.push(Problem::error("", e));
        false
    });
    asset.note = literal(start, 3);
    let entry = targets(start.id, 0);
    let first = graph
        .nodes()
        .iter()
        .find(|n| !matches!(n.archetype.as_str(), START | ENTRY) && !is_auto(n))
        .and_then(|n| ids.get(&n.id));
    match entry.as_slice() {
        [] => {}
        // The first node is the entry anyway; saying so would only add noise to the file.
        [one] if Some(one) == first => {}
        [one] => asset.entry = one.clone(),
        _ => problems.push(Problem::error(
            "",
            "the Dialogue node's Entry leads to one node",
        )),
    }
    let interrupt = targets(start.id, 1);
    match interrupt.as_slice() {
        [] => {}
        [one] => asset.on_interrupt = one.clone(),
        _ => problems.push(Problem::error("", "On Interrupt leads to one node")),
    }
    for node in graph.nodes().iter().filter(|n| n.archetype == ENTRY) {
        let name = literal(node, 0).trim().to_owned();
        match targets(node.id, 0).as_slice() {
            [one] if !name.is_empty() => {
                if asset.entries.insert(name.clone(), one.clone()).is_some() {
                    problems.push(Problem::error("", format!("two entries are named {name}")));
                }
            }
            _ => problems.push(Problem::error(
                "",
                format!("named entry \"{name}\" needs a name and one wire"),
            )),
        }
    }
    for node in graph.nodes() {
        if matches!(node.archetype.as_str(), START | ENTRY) || auto.contains_key(&node.id) {
            continue;
        }
        let id = ids[&node.id].clone();
        let pos = Some(node.position.to_array());
        let get = |pin: u16| literal(node, pin);
        let mut report = |message: String| problems.push(Problem::error(id.clone(), message));
        let def = match node.archetype.as_str() {
            LINE => NodeDef::Line(LineDef {
                id: id.clone(),
                speaker: get(1).trim().to_owned(),
                expression: get(2).trim().to_owned(),
                text: extras.text(&id, 0, get(3)),
                when: get(4),
                effects: get(5),
                voice: get(6).trim().to_owned(),
                language: get(7).trim().to_owned(),
                tags: parse_tags(&get(8)),
                next: targets(node.id, 0),
                pos,
                note: get(9),
            }),
            OPTION => {
                let label = extras.text(&id, 8, get(8));
                let check = parse_check(&get(3), label).unwrap_or_else(|e| {
                    report(format!("Check: {e}"));
                    None
                });
                let show_locked = match get(6).trim() {
                    "" => None,
                    other => Some(parse_bool(other, "Show Locked").unwrap_or_else(|e| {
                        report(e);
                        false
                    })),
                };
                let timeout_ms = match get(7).trim() {
                    "" => None,
                    other => other.parse::<u64>().map(Some).unwrap_or_else(|_| {
                        report(format!("Timeout (ms) is a whole number, not `{other}`"));
                        None
                    }),
                };
                NodeDef::Option(OptionDef {
                    id: id.clone(),
                    text: extras.text(&id, 1, get(1)),
                    when: get(2),
                    show_locked,
                    check,
                    once: parse_bool(&get(5), "Once").unwrap_or_else(|e| {
                        report(e);
                        false
                    }),
                    timeout_ms,
                    effects: get(4),
                    tags: parse_tags(&get(9)),
                    next: targets(node.id, 0),
                    fail: targets(node.id, 1),
                    pos,
                    note: get(10),
                })
            }
            BRANCH => NodeDef::Branch(BranchDef {
                id: id.clone(),
                when: get(1),
                next: targets(node.id, 0),
                otherwise: targets(node.id, 1),
                pos,
                note: get(2),
            }),
            ACTION => NodeDef::Action(ActionDef {
                id: id.clone(),
                effects: get(1),
                wait: parse_bool(&get(2), "Wait").unwrap_or_else(|e| {
                    report(e);
                    false
                }),
                next: targets(node.id, 0),
                pos,
                note: get(3),
            }),
            CALL => NodeDef::Call(CallDef {
                id: id.clone(),
                dialogue: get(1).trim().to_owned(),
                entry: get(2).trim().to_owned(),
                next: targets(node.id, 0),
                pos,
                note: get(3),
            }),
            JUMP => NodeDef::Jump(JumpDef {
                id: id.clone(),
                target: get(1).trim().to_owned(),
                pos,
                note: get(3),
            }),
            END => NodeDef::End(EndDef {
                id: id.clone(),
                result: get(1).trim().to_owned(),
                pos,
                note: get(2),
            }),
            other => {
                report(format!("{other} is not a dialogue node"));
                continue;
            }
        };
        asset.nodes.push(def);
    }
    if problems.is_empty() {
        Ok(asset)
    } else {
        Err(problems)
    }
}

/// Everything wrong that can be told without the game's manifest: conversion and structure. Warnings included.
#[must_use]
pub fn problems(graph: &Graph) -> Vec<Problem> {
    match from_graph(graph) {
        Ok(asset) => asset.structural_problems(),
        Err(problems) => problems,
    }
}

/// The Preview and Apply gate: errors refuse, warnings pass.
pub fn validate(graph: &Graph) -> Result<(), String> {
    let errors: Vec<Problem> = problems(graph).into_iter().filter(|p| !p.warning).collect();
    match errors.first() {
        None => Ok(()),
        Some(first) if errors.len() == 1 => Err(first.to_string()),
        Some(first) => Err(format!("{} problems · {first}", errors.len())),
    }
}

/// Graph node by saved id, for jumping to a problem or the line a preview is on.
#[must_use]
pub fn find_node(graph: &Graph, id: &str) -> Option<NodeId> {
    node_ids(graph)
        .into_iter()
        .find_map(|(node, saved)| (saved == id).then_some(node))
}

/// A starter dialogue: a line and an end.
#[must_use]
pub fn default_surface() -> GraphSurface {
    let mut asset = DialogueAsset::new("new_dialogue");
    asset.title = "New dialogue".into();
    asset.nodes = vec![
        NodeDef::Line(LineDef {
            id: "n1".into(),
            text: "Hello.".into(),
            next: vec!["e1".into()],
            ..LineDef::default()
        }),
        NodeDef::End(EndDef {
            id: "e1".into(),
            ..EndDef::default()
        }),
    ];
    surface(&asset).0
}

/// A surface for an asset, and the extras a lossless save needs.
#[must_use]
pub fn surface(asset: &DialogueAsset) -> (GraphSurface, Extras) {
    let (graph, extras) = to_graph(asset);
    let mut surface = GraphSurface::new(catalogue());
    surface.graph = graph;
    (surface, extras)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DialogueAsset {
        DialogueAsset::from_json(
            r#"{
            "version": 1, "id": "foreman", "title": "The foreman", "on_interrupt": "hey",
            "entries": {"again": "back"},
            "nodes": [
                {"type": "line", "id": "n1", "speaker": "foreman", "expression": "angry",
                 "text": {"key": "custom.key", "source": "You the lockpick?", "note": "gruff"},
                 "effects": "met.foreman = true", "next": ["menu"], "voice": "vo/f1.ogg", "tags": ["shout", "first"]},
                {"type": "line", "id": "menu", "speaker": "foreman", "text": "Well?", "next": ["ask", "mend", "bluff", "quiet"]},
                {"type": "option", "id": "ask", "text": "What's it pay?", "once": true, "next": ["pay"]},
                {"type": "line", "id": "pay", "speaker": "foreman", "text": "Fifty.", "next": ["menu"]},
                {"type": "option", "id": "mend", "text": "Your hand.", "show_locked": true,
                 "check": {"category": "skill", "fact": "skill.mending", "at_least": 3}, "next": ["b1"]},
                {"type": "option", "id": "bluff", "text": "Trust me.",
                 "check": {"category": "skill", "fact": "skill.mending", "label": "Bluff", "roll": {"sides": 20, "difficulty": 11, "bonus": 2}},
                 "next": ["b1"], "fail": ["other/n1"]},
                {"type": "option", "id": "quiet", "text": "", "timeout_ms": 5000, "next": ["e1"]},
                {"type": "branch", "id": "b1", "when": "rep.varga > 0", "next": ["a1"], "else": ["e1"]},
                {"type": "action", "id": "a1", "effects": "camera('wide')", "wait": true, "next": ["c1"]},
                {"type": "call", "id": "c1", "dialogue": "aside", "entry": "x", "next": ["j1"]},
                {"type": "jump", "id": "j1", "target": "menu"},
                {"type": "end", "id": "e1", "result": "done"},
                {"type": "line", "id": "back", "speaker": "foreman", "text": "Back?", "next": ["menu"]},
                {"type": "line", "id": "hey", "text": "Oi!"}
            ]}"#,
        )
        .unwrap()
    }

    /// Positions are the graph's business; compare the rest.
    fn without_positions(mut asset: DialogueAsset) -> DialogueAsset {
        for node in &mut asset.nodes {
            node.set_pos(None);
        }
        asset
    }

    #[test]
    fn an_asset_round_trips_through_the_graph() {
        let asset = sample();
        let (graph, extras) = to_graph(&asset);
        let back = from_graph_with(&graph, &extras).unwrap();
        assert_eq!(without_positions(back.clone()), asset);
        // Saved positions come back exactly.
        let (graph, extras) = to_graph(&back);
        assert_eq!(from_graph_with(&graph, &extras).unwrap(), back);
    }

    #[test]
    fn back_links_and_other_dialogues_are_drawn_as_automatic_go_tos() {
        let (graph, _) = to_graph(&sample());
        let autos: Vec<String> = graph
            .nodes()
            .iter()
            .filter(|n| {
                n.archetype == JUMP && n.literals.get(&2).map(String::as_str) == Some("true")
            })
            .map(|n| n.literals[&1].clone())
            .collect();
        assert_eq!(autos.len(), 2, "{autos:?}");
        assert!(
            autos.contains(&"menu".to_owned()),
            "pay loops back to the menu"
        );
        assert!(autos.contains(&"other/n1".to_owned()));
        assert!(
            graph.topological_order().is_some(),
            "the drawn graph is acyclic"
        );
    }

    #[test]
    fn menu_order_is_drawing_order() {
        let (mut graph, extras) = to_graph(&sample());
        let menu = find_node(&graph, "menu").unwrap();
        let shown = |graph: &Graph| {
            from_graph_with(graph, &extras)
                .unwrap()
                .node("menu")
                .map(|n| n.links()[0].1.to_vec())
                .unwrap()
        };
        assert_eq!(shown(&graph), ["ask", "mend", "bluff", "quiet"]);
        let ask = find_node(&graph, "ask").unwrap();
        let quiet_y = graph
            .node(find_node(&graph, "quiet").unwrap())
            .unwrap()
            .position
            .y;
        graph.node_mut(ask).unwrap().position.y = quiet_y + 10.0;
        assert_eq!(
            shown(&graph),
            ["mend", "bluff", "quiet", "ask"],
            "moving a choice down reorders the menu"
        );
        let _ = menu;
    }

    #[test]
    fn a_saved_layout_that_disagrees_with_a_menu_is_restacked() {
        let mut asset = sample();
        for node in &mut asset.nodes {
            node.set_pos(Some([0.0, 0.0]));
        }
        if let Some(NodeDef::Option(o)) = asset.nodes.iter_mut().find(|n| n.id() == "ask") {
            o.pos = Some([0.0, 900.0]);
        }
        let (graph, extras) = to_graph(&asset);
        let back = from_graph_with(&graph, &extras).unwrap();
        assert_eq!(
            back.node("menu").unwrap().links()[0].1,
            ["ask", "mend", "bluff", "quiet"]
        );
    }

    #[test]
    fn checks_read_and_write_as_one_line() {
        let check = parse_check("skill: skill.mending + 2 + d20 >= 11", LocalText::default())
            .unwrap()
            .unwrap();
        assert_eq!(
            check.roll,
            Some(RollDef {
                sides: 20,
                difficulty: 11,
                bonus: 2
            })
        );
        assert_eq!(format_check(&check), "skill: skill.mending + 2 + d20 >= 11");
        let passive = parse_check("language: lang.patter >= conversant", LocalText::default())
            .unwrap()
            .unwrap();
        assert_eq!(passive.at_least, Some(Value::Text("conversant".into())));
        assert_eq!(
            format_check(&passive),
            "language: lang.patter >= conversant"
        );
        let bare = parse_check("skill.mending >= 3", LocalText::default())
            .unwrap()
            .unwrap();
        assert_eq!(
            (bare.category.as_str(), bare.at_least.clone()),
            ("", Some(Value::Int(3)))
        );
        let penalty = parse_check("skill.mending - 3 + d6 >= 2", LocalText::default())
            .unwrap()
            .unwrap();
        assert_eq!(penalty.roll.as_ref().unwrap().bonus, -3);
        assert_eq!(format_check(&penalty), "skill.mending - 3 + d6 >= 2");
        assert!(parse_check("skill.mending", LocalText::default()).is_err());
        assert!(parse_check("x + 2 >= 3", LocalText::default()).is_err());
        assert!(parse_check("x + dq >= 3", LocalText::default()).is_err());
    }

    #[test]
    fn new_nodes_get_fresh_ids_and_mistakes_come_back_by_node() {
        let (mut graph, _) = to_graph(&sample());
        let c = catalogue();
        let fresh = graph.add(&c, LINE, Vec2::new(5000.0, 0.0)).unwrap();
        assert_eq!(node_ids(&graph)[&fresh], "n2");
        assert_eq!(fresh_id(&graph, OPTION), "o1");
        let option = find_node(&graph, "mend").unwrap();
        graph
            .node_mut(option)
            .unwrap()
            .literals
            .insert(3, "skill.mending > 3".into());
        graph
            .node_mut(option)
            .unwrap()
            .literals
            .insert(7, "soon".into());
        let problems = from_graph(&graph).unwrap_err();
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems.iter().all(|p| p.at == "mend"));
        assert!(validate(&graph).unwrap_err().starts_with("2 problems"));
    }

    #[test]
    fn the_starter_is_valid() {
        let surface = default_surface();
        assert!(validate(&surface.graph).is_ok());
        let json = super::super::serial::to_json(&surface.graph, &surface.catalogue).unwrap();
        let graph = super::super::serial::from_json(&json, &catalogue()).unwrap();
        assert_eq!(from_graph(&graph).unwrap().nodes.len(), 2);
    }
}
