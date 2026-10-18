//! What a place sounds like between scenes: who barks, who chats, and when to keep quiet.
//!
//! [`Ambience`] sits between the game and [`Story::bark_in`] / [`Story::chatter_in`]. It keeps one voice going at a
//! time near the listener; a bark (something happened) outranks chatter (nothing did) and cuts it off, and only a
//! more urgent bark cuts off a bark. Repeats are kept down three ways: each bark's own `cooldown_ms` and `once` (the
//! story's, saved), a rest for each speaker (an urgent bark, priority above 0, ignores it), and the last few lines
//! heard, which are passed over while anything else fits. Chatter waits for a quiet stretch.
//!
//! It runs on the story's clock and holds nothing worth saving: after a load the place is quiet for a moment, and the
//! cooldowns that matter are in the story's save.

use crate::barks::BarkContext;
use crate::story::{BarkPick, ChatterPick, Story};
use std::collections::{BTreeMap, VecDeque};

/// How lived-in a place sounds.
#[derive(Clone, Debug, PartialEq)]
pub struct AmbienceSettings {
    /// Game milliseconds before the same person barks again.
    pub speaker_rest_ms: u64,
    /// Game milliseconds of quiet before chatter starts.
    pub chatter_gap_ms: u64,
    /// How many recent barks and exchanges are passed over while something else fits.
    pub recent: usize,
}

impl Default for AmbienceSettings {
    fn default() -> Self {
        Self {
            speaker_rest_ms: 8_000,
            chatter_gap_ms: 20_000,
            recent: 6,
        }
    }
}

/// What was picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Heard {
    Bark(BarkPick),
    /// An exchange, with who takes each role (`cast[i]` says role `i`).
    Chatter {
        pick: ChatterPick,
        cast: Vec<String>,
    },
}

/// A line (or exchange) to play now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spoken {
    pub heard: Heard,
    /// It cuts off what was being said: stop that voice first.
    pub interrupts: bool,
    /// Game time the ambience counts it as finished, unless [`Ambience::finished`] says sooner.
    pub until_ms: u64,
}

#[derive(Clone, Debug)]
struct Playing {
    priority: i32,
    chatter: bool,
    until_ms: u64,
}

/// One listener's surroundings.
#[derive(Clone, Debug, Default)]
pub struct Ambience {
    pub settings: AmbienceSettings,
    playing: Option<Playing>,
    rested: BTreeMap<String, u64>,
    recent: VecDeque<String>,
    quiet_since_ms: u64,
}

/// How long a line takes to say, by its length: the same pace a story actor talks at.
fn line_ms(text: &str) -> u64 {
    let chars = text.chars().filter(|c| !c.is_whitespace()).count() as u64;
    (900 + chars * 55).clamp(1_200, 7_000)
}

impl Ambience {
    #[must_use]
    pub fn new(settings: AmbienceSettings) -> Self {
        Self {
            settings,
            ..Self::default()
        }
    }

    /// Whether something is being said at `now_ms`.
    #[must_use]
    pub fn busy(&self, now_ms: u64) -> bool {
        self.playing.as_ref().is_some_and(|p| p.until_ms > now_ms)
    }

    /// The game finished playing what was picked (its voice clip ended, the player walked off).
    pub fn finished(&mut self, story: &Story) {
        if self.playing.take().is_some() {
            self.quiet_since_ms = story.clock_ms();
        }
    }

    /// Something happened (`trigger`): a bark for it from someone in `context.near`, unless a line that matters as
    /// much is still being said or everyone near has spoken too lately.
    pub fn event(
        &mut self,
        story: &mut Story,
        trigger: &str,
        context: &BarkContext,
    ) -> Option<Spoken> {
        let now = story.clock_ms();
        self.settle(now);
        let near = self.rested_near(context.near, now);
        let recent: Vec<String> = self.recent.iter().cloned().collect();
        let min_priority = match &self.playing {
            Some(p) if !p.chatter => p.priority.saturating_add(1),
            _ => i32::MIN,
        };
        let floor = min_priority.max(context.min_priority);
        // Rested speakers, passing over what was heard lately; then allowing it; then, for something urgent
        // (priority above 0), anyone near: a guard who just spoke still shouts "Thief!".
        let attempts: [(&[&str], &[String], i32); 3] = [
            (&near, &recent, floor),
            (&near, &[], floor),
            (context.near, &[], floor.max(1)),
        ];
        let pick = attempts
            .into_iter()
            .find_map(|(near, avoid, min_priority)| {
                let asked = BarkContext {
                    near,
                    avoid,
                    min_priority,
                    ..*context
                };
                story.bark_in(trigger, &asked)
            })?;
        let interrupts = self.playing.is_some();
        let until_ms = now + line_ms(&pick.text);
        self.rested.insert(pick.speaker.clone(), now);
        self.remember(format!("{}/{}", pick.set, pick.id));
        self.playing = Some(Playing {
            priority: pick.priority,
            chatter: false,
            until_ms,
        });
        Some(Spoken {
            heard: Heard::Bark(pick),
            interrupts,
            until_ms,
        })
    }

    /// A quiet moment: chatter (`trigger`, usually `ambient`) among those near, once nothing has been said for
    /// [`AmbienceSettings::chatter_gap_ms`]. Call it every second or so.
    pub fn idle(
        &mut self,
        story: &mut Story,
        trigger: &str,
        context: &BarkContext,
    ) -> Option<Spoken> {
        let now = story.clock_ms();
        self.settle(now);
        if self.playing.is_some() || now < self.quiet_since_ms + self.settings.chatter_gap_ms {
            return None;
        }
        let near = self.rested_near(context.near, now);
        let recent: Vec<String> = self.recent.iter().cloned().collect();
        let mut asked: BarkContext = *context;
        asked.near = &near;
        asked.avoid = &recent;
        let pick = story.chatter_in(trigger, &asked).or_else(|| {
            asked.avoid = &[];
            story.chatter_in(trigger, &asked)
        })?;
        let cast: Vec<String> = near
            .iter()
            .take(pick.roles.len())
            .map(|s| (*s).to_owned())
            .collect();
        let length: u64 = pick.lines.iter().map(|l| line_ms(&l.text) + 400).sum();
        let until_ms = now + length;
        for speaker in &cast {
            self.rested.insert(speaker.clone(), now);
        }
        self.remember(format!("{}/{}", pick.set, pick.id));
        self.playing = Some(Playing {
            priority: pick.priority,
            chatter: true,
            until_ms,
        });
        Some(Spoken {
            heard: Heard::Chatter { pick, cast },
            interrupts: false,
            until_ms,
        })
    }

    /// Let go of a line whose time has run out.
    fn settle(&mut self, now: u64) {
        if let Some(playing) = &self.playing {
            if playing.until_ms <= now {
                self.quiet_since_ms = playing.until_ms;
                self.playing = None;
            }
        }
    }

    fn rested_near<'a>(&self, near: &[&'a str], now: u64) -> Vec<&'a str> {
        near.iter()
            .copied()
            .filter(|speaker| {
                self.rested
                    .get(*speaker)
                    .is_none_or(|last| now.saturating_sub(*last) >= self.settings.speaker_rest_ms)
            })
            .collect()
    }

    fn remember(&mut self, key: String) {
        self.recent.retain(|k| *k != key);
        self.recent.push_back(key);
        while self.recent.len() > self.settings.recent {
            self.recent.pop_front();
        }
    }
}
