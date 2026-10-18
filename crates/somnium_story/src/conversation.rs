//! Playing a dialogue: the conversation runtime.
//!
//! A [`Conversation`] is a cursor into the story's dialogues. It stops at a [`Beat`] (a line to show, a menu to
//! answer, an action waiting for the game, or the end) and moves when the game says so: [`Conversation::advance`]
//! after a line, [`Conversation::choose`] in a menu, [`Conversation::resume`] after a wait. Effects apply to the
//! [`Story`] as nodes are entered, so a line that sets `met.mireille` has set it by the time it is on screen, and
//! any quest that reacts has already moved.
//!
//! The conversation keeps no clock of its own except for a timed menu ([`Conversation::tick`]); reading speed, voice
//! and auto-advance are the presenter's (`presenter.rs`), which is what lets the same conversation run under a
//! light-novel box, a world-space bubble or a test.

use crate::dialogue::{CheckDef, Dialogue, NodeDef};
use crate::manifest::{Side, parse_color};
use crate::story::{Story, StoryEvent};
use crate::value::{FactType, Value};
use serde::Serialize;
use std::sync::Arc;

/// Non-presenting nodes one move may pass through before it is called a loop.
const WALK_BUDGET: usize = 256;

/// A line to show.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LineBeat {
    pub dialogue: String,
    pub node: String,
    /// Cast id; empty for the narrator.
    pub speaker: String,
    /// The name plate, resolved.
    pub name: String,
    pub portrait: String,
    pub expression: String,
    pub side: Side,
    pub color: Option<[u8; 4]>,
    /// Resolved text: markup and inline cues (`[expr=x]`, `[pause=ms]`) still in it, for the presenter.
    pub text: String,
    pub language: String,
    pub voice: String,
    pub tags: Vec<String>,
    /// Shown before in this playthrough (skip-seen reads it).
    pub seen: bool,
    /// The dialogue plays without the player (chatter, radio): auto-advance.
    pub ambient: bool,
    /// A menu follows this line, so the presenter can keep it on screen under the choices.
    pub menu_next: bool,
}

/// A check as the player sees it on a choice.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CheckBeat {
    pub category: String,
    pub label: String,
    /// A passive check's result now; `None` for a roll, which is decided when chosen.
    pub passes: Option<bool>,
    /// Chance of a roll succeeding, in percent.
    pub chance: Option<u8>,
}

/// One choice in a menu.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChoiceBeat {
    /// Position in the menu as shown: what number key 1 + `index` picks.
    pub index: usize,
    pub node: String,
    pub text: String,
    /// Locked choices are shown (greyed, with their check) but cannot be taken.
    pub available: bool,
    pub check: Option<CheckBeat>,
    pub tags: Vec<String>,
    /// Taken before in this playthrough.
    pub taken: bool,
}

/// Where a conversation is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "beat", rename_all = "snake_case")]
pub enum Beat {
    Line(LineBeat),
    Choices {
        dialogue: String,
        choices: Vec<ChoiceBeat>,
        /// Milliseconds the player has, for a timed menu.
        timeout_ms: Option<u64>,
    },
    /// An action is holding the conversation for the game (a camera move, an animation).
    Wait {
        dialogue: String,
        node: String,
    },
    End {
        result: String,
    },
}

/// How a check went, for the presenter's feedback.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CheckResult {
    pub node: String,
    pub passed: bool,
    /// The die and the total, for a roll.
    pub roll: Option<(i64, i64)>,
}

#[derive(Clone, Debug)]
struct Frame {
    dialogue: String,
    next: Vec<String>,
}

#[derive(Clone, Debug)]
struct Menu {
    options: Vec<(String, bool)>,
    timeout: Option<(u64, String)>,
}

/// One conversation in progress.
#[derive(Clone, Debug)]
pub struct Conversation {
    root: String,
    dialogue: String,
    node: String,
    stack: Vec<Frame>,
    beat: Beat,
    menu: Option<Menu>,
    menu_elapsed_ms: u64,
    interrupted: bool,
    last_check: Option<CheckResult>,
}

impl Conversation {
    /// Start `dialogue` at `entry` (a named entry or node id; `""` for its entry).
    pub fn start(story: &mut Story, dialogue: &str, entry: &str) -> Result<Self, String> {
        let db = Arc::clone(story.database());
        let found = db
            .dialogue(dialogue)
            .ok_or_else(|| format!("no dialogue {dialogue}"))?;
        let node = found
            .asset
            .entry_node(entry)
            .ok_or_else(|| format!("{dialogue} has no entry or node {entry:?}"))?
            .to_owned();
        let mut conversation = Self {
            root: dialogue.to_owned(),
            dialogue: dialogue.to_owned(),
            node: String::new(),
            stack: Vec::new(),
            beat: Beat::End {
                result: String::new(),
            },
            menu: None,
            menu_elapsed_ms: 0,
            interrupted: false,
            last_check: None,
        };
        conversation.follow(story, vec![node]);
        Ok(conversation)
    }

    #[must_use]
    pub fn beat(&self) -> &Beat {
        &self.beat
    }

    #[must_use]
    pub fn is_over(&self) -> bool {
        matches!(self.beat, Beat::End { .. })
    }

    /// The dialogue the conversation is in now (a call moves it).
    #[must_use]
    pub fn dialogue(&self) -> &str {
        &self.dialogue
    }

    /// The result of the last choice's check, until the next choice.
    #[must_use]
    pub fn last_check(&self) -> Option<&CheckResult> {
        self.last_check.as_ref()
    }

    /// Past the line on screen.
    pub fn advance(&mut self, story: &mut Story) -> Result<(), String> {
        let Beat::Line(_) = &self.beat else {
            return Err("advance is for a line; this beat is not one".into());
        };
        let next = self.links(story, &self.node.clone(), "next");
        self.follow(story, next);
        Ok(())
    }

    /// Take the choice shown at `index` (0-based; number key 1 is index 0).
    pub fn choose(&mut self, story: &mut Story, index: usize) -> Result<(), String> {
        let Some(menu) = &self.menu else {
            return Err("choose is for a menu; this beat is not one".into());
        };
        let Some((node, available)) = menu.options.get(index).cloned() else {
            return Err(format!("there is no choice {}", index + 1));
        };
        if !available {
            return Err(format!("choice {} is locked", index + 1));
        }
        self.take(story, &node);
        Ok(())
    }

    /// Let the game's wait go.
    pub fn resume(&mut self, story: &mut Story) -> Result<(), String> {
        let Beat::Wait { .. } = &self.beat else {
            return Err("resume is for an action that waits".into());
        };
        let next = self.links(story, &self.node.clone(), "next");
        self.follow(story, next);
        Ok(())
    }

    /// Run a timed menu's clock. Returns whether the timeout choice was taken.
    pub fn tick(&mut self, story: &mut Story, ms: u64) -> bool {
        let Some(Menu {
            timeout: Some((limit, node)),
            ..
        }) = &self.menu
        else {
            return false;
        };
        let (limit, node) = (*limit, node.clone());
        self.menu_elapsed_ms = self.menu_elapsed_ms.saturating_add(ms);
        if self.menu_elapsed_ms >= limit {
            self.take(story, &node);
            return true;
        }
        false
    }

    /// Milliseconds left on a timed menu.
    #[must_use]
    pub fn time_left_ms(&self) -> Option<u64> {
        let (limit, _) = self.menu.as_ref()?.timeout.as_ref()?;
        Some(limit.saturating_sub(self.menu_elapsed_ms))
    }

    /// The game cut the conversation short (the player walked off, combat began). The dialogue's `on_interrupt`
    /// plays once if it has one; interrupting again, or a dialogue without one, ends it. Returns whether anything
    /// is left to show.
    pub fn interrupt(&mut self, story: &mut Story) -> bool {
        if self.is_over() {
            return false;
        }
        let db = Arc::clone(story.database());
        let handler = db
            .dialogue(&self.root)
            .map(|d| d.asset.on_interrupt.clone())
            .unwrap_or_default();
        if self.interrupted || handler.is_empty() {
            self.finish("interrupted");
            return false;
        }
        self.interrupted = true;
        self.stack.clear();
        self.menu = None;
        self.dialogue = self.root.clone();
        self.follow(story, vec![handler]);
        !self.is_over()
    }

    // --- walking ---------------------------------------------------------------------------------------------

    fn links(&self, story: &Story, node: &str, role: &str) -> Vec<String> {
        story
            .database()
            .dialogue(&self.dialogue)
            .and_then(|d| d.node(node))
            .and_then(|n| {
                n.def
                    .links()
                    .into_iter()
                    .find(|(r, _)| *r == role)
                    .map(|(_, targets)| targets.to_vec())
            })
            .unwrap_or_default()
    }

    fn finish(&mut self, result: &str) {
        self.menu = None;
        self.stack.clear();
        self.beat = Beat::End {
            result: if result.is_empty() && self.interrupted {
                "interrupted".to_owned()
            } else {
                result.to_owned()
            },
        };
    }

    fn follow(&mut self, story: &mut Story, mut targets: Vec<String>) {
        let db = Arc::clone(story.database());
        self.menu = None;
        self.menu_elapsed_ms = 0;
        for _ in 0..WALK_BUDGET {
            if targets.is_empty() {
                if let Some(frame) = self.stack.pop() {
                    self.dialogue = frame.dialogue;
                    targets = frame.next;
                    continue;
                }
                self.finish("");
                return;
            }
            let Some(dialogue) = db.dialogue(&self.dialogue) else {
                self.problem(story, format!("no dialogue {}", self.dialogue));
                return;
            };
            if targets
                .iter()
                .all(|t| matches!(dialogue.node(t).map(|n| &n.def), Some(NodeDef::Option(_))))
            {
                self.open_menu(story, dialogue, &targets);
                return;
            }
            let chosen = targets.iter().find(|target| {
                let (d, n) = self.split(target);
                db.dialogue(d)
                    .and_then(|d| d.node(n))
                    .is_some_and(|node| match &node.def {
                        NodeDef::Line(_) => story.eval(&node.when, Some(d)),
                        _ => true,
                    })
            });
            let Some(chosen) = chosen.cloned() else {
                targets.clear();
                continue;
            };
            let (dialogue_id, node_id) = {
                let (d, n) = self.split(&chosen);
                (d.to_owned(), n.to_owned())
            };
            self.dialogue = dialogue_id.clone();
            let Some(node) = db.dialogue(&dialogue_id).and_then(|d| d.node(&node_id)) else {
                self.problem(story, format!("{chosen} does not exist"));
                return;
            };
            if let NodeDef::Option(_) = &node.def {
                // An option reached outside a menu list is a menu of one; it counts as taken only when taken.
                let dialogue = db.dialogue(&dialogue_id).expect("looked up above");
                self.open_menu(story, dialogue, &[node_id]);
                return;
            }
            story.visit(&dialogue_id, &node_id);
            self.node = node_id.clone();
            match &node.def {
                NodeDef::Line(line) => {
                    story.apply(&node.effects, Some(&dialogue_id));
                    let beat = self.line_beat(story, &db, &dialogue_id, line);
                    self.beat = Beat::Line(beat);
                    return;
                }
                NodeDef::Option(_) => unreachable!("options are opened as menus above"),
                NodeDef::Branch(branch) => {
                    targets = if story.eval(&node.when, Some(&dialogue_id)) {
                        branch.next.clone()
                    } else {
                        branch.otherwise.clone()
                    };
                }
                NodeDef::Action(action) => {
                    story.apply(&node.effects, Some(&dialogue_id));
                    if action.wait {
                        self.beat = Beat::Wait {
                            dialogue: dialogue_id,
                            node: node_id,
                        };
                        return;
                    }
                    targets = action.next.clone();
                }
                NodeDef::Call(call) => {
                    let Some(entry) = db
                        .dialogue(&call.dialogue)
                        .and_then(|d| d.asset.entry_node(&call.entry))
                    else {
                        self.problem(
                            story,
                            format!("{} has no entry {:?}", call.dialogue, call.entry),
                        );
                        return;
                    };
                    self.stack.push(Frame {
                        dialogue: dialogue_id,
                        next: call.next.clone(),
                    });
                    self.dialogue = call.dialogue.clone();
                    targets = vec![entry.to_owned()];
                }
                NodeDef::Jump(jump) => targets = vec![jump.target.clone()],
                NodeDef::End(end) => {
                    if let Some(frame) = self.stack.pop() {
                        self.dialogue = frame.dialogue;
                        targets = frame.next;
                    } else {
                        self.finish(&end.result);
                        return;
                    }
                }
            }
        }
        self.problem(
            story,
            format!(
                "{} went through {WALK_BUDGET} nodes without a line or a menu; a loop of branches or jumps?",
                self.dialogue
            ),
        );
    }

    /// `dialogue/node` or a node of the current dialogue.
    fn split<'a>(&'a self, target: &'a str) -> (&'a str, &'a str) {
        target
            .split_once('/')
            .unwrap_or((self.dialogue.as_str(), target))
    }

    fn problem(&mut self, story: &mut Story, message: String) {
        story.push_event(StoryEvent::Problem { message });
        self.finish("error");
    }

    fn open_menu(&mut self, story: &mut Story, dialogue: &Dialogue, options: &[String]) {
        let id = dialogue.id().to_owned();
        let mut shown = Vec::new();
        let mut timeout = None;
        let mut choices = Vec::new();
        for option_id in options {
            let Some(node) = dialogue.node(option_id) else {
                continue;
            };
            let NodeDef::Option(option) = &node.def else {
                continue;
            };
            let taken = story.visits(&id, option_id) > 0;
            if option.once && taken {
                continue;
            }
            if let Some(ms) = option.timeout_ms {
                timeout = Some((ms, option_id.clone()));
                continue;
            }
            let condition = story.eval(&node.when, Some(&id));
            let check = option
                .check
                .as_ref()
                .map(|check| self.check_beat(story, check));
            let gated =
                option.fail.is_empty() && check.as_ref().is_some_and(|c| c.passes == Some(false));
            let available = condition && !gated;
            if !available && !option.show_locked.unwrap_or(option.check.is_some()) {
                continue;
            }
            choices.push(ChoiceBeat {
                index: shown.len(),
                node: option_id.clone(),
                text: story.text(&option.text, Some(&id)),
                available,
                check,
                tags: option.tags.clone(),
                taken,
            });
            shown.push((option_id.clone(), available));
        }
        if !shown.iter().any(|(_, available)| *available) && timeout.is_none() {
            self.problem(
                story,
                format!(
                    "{id}: a menu ({}) had no choice the player could take",
                    options.join(", ")
                ),
            );
            return;
        }
        self.beat = Beat::Choices {
            dialogue: id,
            choices,
            timeout_ms: timeout.as_ref().map(|(ms, _)| *ms),
        };
        self.menu = Some(Menu {
            options: shown,
            timeout,
        });
        self.menu_elapsed_ms = 0;
    }

    fn take(&mut self, story: &mut Story, option_id: &str) {
        let db = Arc::clone(story.database());
        let dialogue_id = self.dialogue.clone();
        let Some(node) = db.dialogue(&dialogue_id).and_then(|d| d.node(option_id)) else {
            self.problem(story, format!("{dialogue_id} has no option {option_id}"));
            return;
        };
        let NodeDef::Option(option) = &node.def else {
            return;
        };
        story.visit(&dialogue_id, option_id);
        self.node = option_id.to_owned();
        let passed = match &option.check {
            None => {
                self.last_check = None;
                true
            }
            Some(check) => {
                let value = story.fact(&check.fact).unwrap_or(Value::Int(0));
                let (passed, roll) = match &check.roll {
                    Some(roll) => {
                        let die = story.roll(roll.sides);
                        let total = value
                            .as_int()
                            .saturating_add(roll.bonus)
                            .saturating_add(die);
                        (total >= roll.difficulty, Some((die, total)))
                    }
                    None => (passes(story, check), None),
                };
                self.last_check = Some(CheckResult {
                    node: option_id.to_owned(),
                    passed,
                    roll,
                });
                passed
            }
        };
        story.apply(&node.effects, Some(&dialogue_id));
        let targets = if passed || option.fail.is_empty() {
            option.next.clone()
        } else {
            option.fail.clone()
        };
        self.follow(story, targets);
    }

    fn check_beat(&self, story: &Story, check: &CheckDef) -> CheckBeat {
        let label = if check.label.is_empty() {
            auto_label(check)
        } else {
            story.text(&check.label, Some(&self.dialogue))
        };
        match &check.roll {
            Some(roll) => {
                let value = story.fact(&check.fact).map_or(0, |v| v.as_int());
                let need = roll.difficulty - value - roll.bonus;
                let sides = i64::from(roll.sides.max(1));
                let wins = (sides - need + 1).clamp(0, sides);
                CheckBeat {
                    category: check.category.clone(),
                    label,
                    passes: None,
                    chance: Some(u8::try_from(wins * 100 / sides).unwrap_or(100)),
                }
            }
            None => CheckBeat {
                category: check.category.clone(),
                label,
                passes: Some(passes(story, check)),
                chance: None,
            },
        }
    }

    fn line_beat(
        &self,
        story: &Story,
        db: &crate::database::StoryDatabase,
        dialogue_id: &str,
        line: &crate::dialogue::LineDef,
    ) -> LineBeat {
        let manifest = &db.manifest;
        let speaker = manifest.speaker(&line.speaker);
        let expression = [
            line.expression.as_str(),
            speaker.map_or("", |s| s.default_expression.as_str()),
            speaker
                .filter(|s| !s.portrait.is_empty())
                .and_then(|_| manifest.expressions.first())
                .map_or("", String::as_str),
        ]
        .into_iter()
        .find(|e| !e.is_empty())
        .unwrap_or("")
        .to_owned();
        let menu_next = {
            let dialogue = db.dialogue(dialogue_id);
            !line.next.is_empty()
                && line.next.iter().all(|t| {
                    matches!(
                        dialogue.and_then(|d| d.node(t)).map(|n| &n.def),
                        Some(NodeDef::Option(_))
                    )
                })
        };
        LineBeat {
            dialogue: dialogue_id.to_owned(),
            node: line.id.clone(),
            speaker: line.speaker.clone(),
            name: speaker
                .map(|s| story.text(&s.name, Some(dialogue_id)))
                .unwrap_or_default(),
            portrait: speaker.map(|s| s.portrait.clone()).unwrap_or_default(),
            expression,
            side: speaker.map(|s| s.side).unwrap_or_default(),
            color: speaker.and_then(|s| parse_color(&s.color)),
            text: story.text(&line.text, Some(dialogue_id)),
            language: if line.language.is_empty() {
                speaker.map(|s| s.language.clone()).unwrap_or_default()
            } else {
                line.language.clone()
            },
            voice: line.voice.clone(),
            tags: line.tags.clone(),
            seen: story.visits(dialogue_id, &line.id) > 1,
            ambient: db.dialogue(dialogue_id).is_some_and(|d| d.asset.ambient),
            menu_next,
        }
    }
}

/// A passive check: the fact against `at_least`, numbers by value and enums by declared order.
fn passes(story: &Story, check: &CheckDef) -> bool {
    let Some(value) = story.fact(&check.fact) else {
        return false;
    };
    match (&check.at_least, &value) {
        (Some(Value::Int(need)), Value::Int(have)) => have >= need,
        (Some(Value::Text(need)), Value::Text(have)) => {
            let def = story.database().schema.get(&check.fact);
            let rank = |v: &str| {
                def.filter(|d| d.ty == FactType::Enum)
                    .and_then(|d| d.values.iter().position(|x| x == v))
            };
            matches!((rank(have), rank(need)), (Some(h), Some(n)) if h >= n)
        }
        (None, _) => true,
        _ => false,
    }
}

/// `Mending 3`, `Patter: conversant`: the fact's last name part, capitalised, and what it needs.
fn auto_label(check: &CheckDef) -> String {
    let name = check
        .fact
        .rsplit('.')
        .next()
        .unwrap_or(&check.fact)
        .replace('_', " ");
    let mut chars = name.chars();
    let name: String = chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    match (&check.roll, &check.at_least) {
        (Some(roll), _) => format!("{name} {}", roll.difficulty),
        (None, Some(Value::Int(need))) => format!("{name} {need}"),
        (None, Some(Value::Text(need))) => format!("{name}: {need}"),
        _ => name,
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Playing without a screen

/// One step of a played-through conversation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TranscriptEntry {
    Line {
        node: String,
        speaker: String,
        name: String,
        expression: String,
        text: String,
    },
    Choices {
        choices: Vec<ChoiceBeat>,
    },
    Chose {
        index: usize,
        node: String,
        check: Option<CheckResult>,
    },
    Wait {
        node: String,
    },
    End {
        result: String,
    },
    /// The script ran out at a menu, or an index was not a choice the player could take.
    Stopped {
        reason: String,
    },
}

/// Play a conversation through with scripted choices (0-based indices), without a presenter: the play-from-node
/// preview, the MCP `simulate` query and tests. Waits resume at once; a menu with no script left stops the run.
pub fn simulate(
    story: &mut Story,
    dialogue: &str,
    entry: &str,
    script: &[usize],
    max_beats: usize,
) -> Result<Vec<TranscriptEntry>, String> {
    let mut conversation = Conversation::start(story, dialogue, entry)?;
    let mut transcript = Vec::new();
    let mut script = script.iter().copied();
    for _ in 0..max_beats.max(1) {
        match conversation.beat().clone() {
            Beat::Line(line) => {
                transcript.push(TranscriptEntry::Line {
                    node: line.node,
                    speaker: line.speaker,
                    name: line.name,
                    expression: line.expression,
                    text: line.text,
                });
                conversation.advance(story)?;
            }
            Beat::Choices { choices, .. } => {
                transcript.push(TranscriptEntry::Choices {
                    choices: choices.clone(),
                });
                let Some(index) = script.next() else {
                    transcript.push(TranscriptEntry::Stopped {
                        reason: "no scripted choice left".into(),
                    });
                    return Ok(transcript);
                };
                if let Err(reason) = conversation.choose(story, index) {
                    transcript.push(TranscriptEntry::Stopped { reason });
                    return Ok(transcript);
                }
                transcript.push(TranscriptEntry::Chose {
                    index,
                    node: choices[index].node.clone(),
                    check: conversation.last_check().cloned(),
                });
            }
            Beat::Wait { node, .. } => {
                transcript.push(TranscriptEntry::Wait { node });
                conversation.resume(story)?;
            }
            Beat::End { result } => {
                transcript.push(TranscriptEntry::End { result });
                return Ok(transcript);
            }
        }
    }
    transcript.push(TranscriptEntry::Stopped {
        reason: format!("stopped after {max_beats} beats"),
    });
    Ok(transcript)
}
