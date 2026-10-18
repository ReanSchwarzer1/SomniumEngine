//! Story actors' performance: who is talking, how the jaw and lids move, whom each actor looks at, the gestures
//! and marks a scene asks for.
//!
//! Promoted from The Somnus Fracture's story actor, which did all of this inside one game's renderer. Here it is the
//! part every game shares and nothing renders: a [`Director`] fed the conversation's beats and the story's commands,
//! answering per cast id with an [`ActorState`] the game's animation layer (or a script, through the engine's
//! `somnium.StoryActor` component) applies to the character: blend in the talk clip while `talking`, open the jaw by
//! `jaw`, close the lids by `lids`, aim the head at `look`, play `gesture`, walk to `mark`.
//!
//! Scenes direct actors with three commands a game declares in its manifest (the director reads them by name from
//! [`crate::StoryEvent::Command`]): `actor_move(actor, mark)`, `actor_look(actor, target)` (a cast id, `player`, a mark,
//! or `none`) and `actor_gesture(actor, gesture)`. A line tagged `gesture:shrug` makes its speaker shrug as it starts.

use crate::conversation::LineBeat;
use crate::value::Value;
use std::collections::BTreeMap;

/// Whom an actor looks at.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Look {
    /// Ahead, or wherever the idle animation looks.
    #[default]
    Ahead,
    Player,
    /// Another actor, by cast id.
    Actor(String),
    /// A named mark in the scene.
    Mark(String),
}

/// One actor's performance this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActorState {
    pub talking: bool,
    /// How far the jaw is open, 0 to 1.
    pub jaw: f32,
    /// How far the lids are shut, 0 to 1.
    pub lids: f32,
    pub look: Look,
    /// The face's expression (the same names as the portrait's), from the last line spoken.
    pub expression: String,
    /// A gesture to start, until the game takes it with [`Director::take_gesture`].
    pub gesture: Option<String>,
    /// A mark the scene sent this actor to; the game walks it there and calls [`Director::arrived`].
    pub mark: Option<String>,
    talk_ms: u64,
    talk_left_ms: Option<u64>,
    clock_ms: u64,
    blink_seed: f32,
}

/// How long a text-only line keeps its speaker talking, per visible character, and at most.
const TALK_MS_PER_CHAR: u64 = 55;
const TALK_MAX_MS: u64 = 7000;

/// The jaw `time` seconds into talking: syllables about five a second, in phrases with breaths between.
#[must_use]
pub fn jaw_open(time: f32) -> f32 {
    let syllable = (time * 5.3 * std::f32::consts::TAU / 2.0).sin().abs();
    let phrase = ((time * 0.45 * std::f32::consts::TAU).sin() * 0.5 + 0.5).powf(0.6);
    let stress = 0.65 + 0.35 * (time * 1.7 + 0.4).sin();
    (syllable * phrase * stress).clamp(0.0, 1.0)
}

/// The lids `time` seconds on, blinking about every `interval` seconds; the gap wanders so it never looks metered.
#[must_use]
pub fn lids_shut(time: f32, interval: f32) -> f32 {
    let interval = interval.max(0.5);
    let beat = time / interval;
    let wander = ((beat.floor() * 12.9898).sin() * 43_758.547).fract().abs() * 0.4;
    let into = (beat.fract() - wander) * interval;
    if (0.0..0.16).contains(&into) {
        (std::f32::consts::PI * into / 0.16).sin()
    } else {
        0.0
    }
}

/// The scene's actors.
#[derive(Clone, Debug, Default)]
pub struct Director {
    actors: BTreeMap<String, ActorState>,
    /// Seconds between blinks, on average.
    pub blink_interval: f32,
    last_speaker: Option<String>,
}

impl Director {
    #[must_use]
    pub fn new() -> Self {
        Self {
            blink_interval: 4.2,
            ..Self::default()
        }
    }

    /// Put an actor in the scene (a cast id). Lines from speakers not added are ignored.
    pub fn add(&mut self, actor: &str) {
        let seed = actor
            .bytes()
            .fold(0.0f32, |acc, b| acc * 0.37 + f32::from(b) * 0.013);
        self.actors
            .entry(actor.to_owned())
            .or_insert_with(|| ActorState {
                blink_seed: seed.fract() * 3.0,
                ..ActorState::default()
            });
    }

    pub fn remove(&mut self, actor: &str) {
        self.actors.remove(actor);
    }

    #[must_use]
    pub fn state(&self, actor: &str) -> Option<&ActorState> {
        self.actors.get(actor)
    }

    pub fn actors(&self) -> impl Iterator<Item = (&str, &ActorState)> {
        self.actors.iter().map(|(id, state)| (id.as_str(), state))
    }

    /// A line began: its speaker talks and looks at whom it speaks to (the last other speaker, else the player);
    /// everyone else in the scene looks at the speaker.
    pub fn line(&mut self, line: &LineBeat) {
        let speaker = line.speaker.clone();
        let listener = self
            .last_speaker
            .clone()
            .filter(|last| *last != speaker && self.actors.contains_key(last));
        for (id, state) in &mut self.actors {
            if *id == speaker {
                state.talking = true;
                state.talk_ms = 0;
                state.talk_left_ms = if line.voice.is_empty() {
                    let chars = line.text.chars().filter(|c| !c.is_whitespace()).count() as u64;
                    Some((chars * TALK_MS_PER_CHAR).clamp(600, TALK_MAX_MS))
                } else {
                    None
                };
                state.look = listener.clone().map_or(Look::Player, Look::Actor);
                if !line.expression.is_empty() {
                    state.expression.clone_from(&line.expression);
                }
                if let Some(gesture) = line.tags.iter().find_map(|t| t.strip_prefix("gesture:")) {
                    state.gesture = Some(gesture.to_owned());
                }
            } else {
                state.talking = false;
                if !speaker.is_empty() {
                    state.look = Look::Actor(speaker.clone());
                }
            }
        }
        if self.actors.contains_key(&speaker) {
            self.last_speaker = Some(speaker);
        }
    }

    /// A menu is up: nobody talks; the last speaker waits on the player.
    pub fn choices(&mut self) {
        for (id, state) in &mut self.actors {
            state.talking = false;
            if self.last_speaker.as_deref() == Some(id) {
                state.look = Look::Player;
            }
        }
    }

    /// The conversation ended: nobody talks, everyone looks ahead again.
    pub fn end(&mut self) {
        for state in self.actors.values_mut() {
            state.talking = false;
            state.look = Look::Ahead;
        }
        self.last_speaker = None;
    }

    /// A voiced line's clip finished: its speaker stops talking.
    pub fn voice_finished(&mut self, actor: &str) {
        if let Some(state) = self.actors.get_mut(actor) {
            state.talking = false;
        }
    }

    /// A story command. Returns whether it was one of the director's.
    pub fn command(&mut self, name: &str, args: &[Value]) -> bool {
        let text = |i: usize| args.get(i).map_or("", Value::as_text).to_owned();
        if !matches!(name, "actor_move" | "actor_look" | "actor_gesture") {
            return false;
        }
        let target = text(1);
        // A look at someone not in the scene is a look at a mark of that name.
        let look = match target.as_str() {
            "" | "none" | "ahead" => Look::Ahead,
            "player" => Look::Player,
            other if self.actors.contains_key(other) => Look::Actor(target.clone()),
            _ => Look::Mark(target.clone()),
        };
        if let Some(state) = self.actors.get_mut(&text(0)) {
            match name {
                "actor_move" => state.mark = Some(target),
                "actor_gesture" => state.gesture = Some(target),
                _ => state.look = look,
            }
        }
        true
    }

    /// The game took the gesture to play.
    pub fn take_gesture(&mut self, actor: &str) -> Option<String> {
        self.actors.get_mut(actor)?.gesture.take()
    }

    /// The actor reached its mark.
    pub fn arrived(&mut self, actor: &str) {
        if let Some(state) = self.actors.get_mut(actor) {
            state.mark = None;
        }
    }

    /// Advance the curves by `ms` of game time.
    pub fn tick(&mut self, ms: u64) {
        for state in self.actors.values_mut() {
            state.clock_ms = state.clock_ms.saturating_add(ms);
            if state.talking {
                state.talk_ms = state.talk_ms.saturating_add(ms);
                if let Some(left) = state.talk_left_ms.as_mut() {
                    *left = left.saturating_sub(ms);
                    if *left == 0 {
                        state.talking = false;
                    }
                }
            }
            state.jaw = if state.talking {
                jaw_open(state.talk_ms as f32 / 1000.0)
            } else {
                // Close smoothly rather than snap.
                (state.jaw - ms as f32 / 120.0).max(0.0)
            };
            state.lids = lids_shut(
                state.clock_ms as f32 / 1000.0 + state.blink_seed,
                self.blink_interval,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Side;

    fn line(speaker: &str, text: &str) -> LineBeat {
        LineBeat {
            dialogue: "d".into(),
            node: "n".into(),
            speaker: speaker.into(),
            name: speaker.into(),
            portrait: String::new(),
            expression: "smile".into(),
            side: Side::Right,
            color: None,
            text: text.into(),
            language: String::new(),
            voice: String::new(),
            tags: vec!["gesture:wave".into()],
            seen: false,
            ambient: false,
            menu_next: false,
        }
    }

    #[test]
    fn the_speaker_talks_and_the_room_looks_at_them() {
        let mut director = Director::new();
        director.add("oren");
        director.add("mira");
        director.line(&line("oren", "Three lamps have gone out."));
        let oren = director.state("oren").unwrap();
        assert!(oren.talking && oren.look == Look::Player && oren.expression == "smile");
        assert_eq!(
            director.state("mira").unwrap().look,
            Look::Actor("oren".into())
        );
        assert_eq!(director.take_gesture("oren").as_deref(), Some("wave"));
        assert_eq!(
            director.take_gesture("oren"),
            None,
            "a gesture is taken once"
        );
        director.line(&line("mira", "Then light them."));
        assert_eq!(
            director.state("mira").unwrap().look,
            Look::Actor("oren".into()),
            "she answers him"
        );
        assert!(!director.state("oren").unwrap().talking);
        director.choices();
        assert_eq!(director.state("mira").unwrap().look, Look::Player);
        director.end();
        assert!(
            director
                .actors()
                .all(|(_, s)| s.look == Look::Ahead && !s.talking)
        );
    }

    #[test]
    fn text_only_lines_talk_for_their_length_and_the_jaw_closes() {
        let mut director = Director::new();
        director.add("oren");
        director.line(&line("oren", "Hm."));
        director.tick(300);
        assert!(director.state("oren").unwrap().talking);
        director.tick(400);
        let oren = director.state("oren").unwrap();
        assert!(!oren.talking, "600 ms at least, then quiet");
        let before = oren.jaw;
        director.tick(500);
        assert!(
            director.state("oren").unwrap().jaw <= before
                && director.state("oren").unwrap().jaw == 0.0
        );
        let mut voiced = line("oren", "Long voiced line.");
        voiced.voice = "vo/o1.ogg".into();
        director.line(&voiced);
        director.tick(60_000);
        assert!(
            director.state("oren").unwrap().talking,
            "a voiced line talks until its clip ends"
        );
        director.voice_finished("oren");
        assert!(!director.state("oren").unwrap().talking);
    }

    #[test]
    fn commands_send_actors_to_marks_and_aim_them() {
        let mut director = Director::new();
        director.add("oren");
        director.add("mira");
        assert!(director.command("actor_move", &["oren".into(), "fountain".into()]));
        assert_eq!(
            director.state("oren").unwrap().mark.as_deref(),
            Some("fountain")
        );
        director.arrived("oren");
        assert_eq!(director.state("oren").unwrap().mark, None);
        director.command("actor_look", &["oren".into(), "mira".into()]);
        assert_eq!(
            director.state("oren").unwrap().look,
            Look::Actor("mira".into())
        );
        director.command("actor_look", &["oren".into(), "bridge_lamp".into()]);
        assert_eq!(
            director.state("oren").unwrap().look,
            Look::Mark("bridge_lamp".into())
        );
        assert!(
            director.command("actor_gesture", &["nobody".into(), "wave".into()]),
            "known, for an absent actor"
        );
        assert!(!director.command("give_item", &["oren".into()]));
    }

    #[test]
    fn blinks_are_brief_and_wander() {
        let shut: Vec<bool> = (0..2000)
            .map(|i| lids_shut(i as f32 * 0.01, 4.2) > 0.5)
            .collect();
        let closed = shut.iter().filter(|s| **s).count();
        assert!(closed > 0 && closed < 60, "{closed} of 2000 frames shut");
        assert!((0..100).all(|i| (0.0..=1.0).contains(&jaw_open(i as f32 * 0.037))));
    }
}
