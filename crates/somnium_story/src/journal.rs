//! The journal: what the player has learned, when, and how sure of it they can be.
//!
//! Two kinds of entry share one ordered log: quest lines (written when a stage is entered) and notes (people,
//! places, lore, the protagonist's own pages), unlocked by the `journal('note_id')` effect from any dialogue, quest
//! or script. Every entry carries a truth tier from the manifest, so the journal can show *how* a fact was learned
//! (Rimeborn's CANON 1.1: absolute, measured, witnessed, recorded, retold), which is the difference between a
//! journal that informs and one that lies with authority.
//!
//! An entry can be **obscured** (`journal_obscure('note_id')`) and restored: the game decides what that looks like.
//! Rimeborn frosts a page of Earth memories over with every Return.

use crate::problem::Problem;
use crate::text::LocalText;
use crate::value::{Value, valid_id};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The notes format this build reads.
pub const NOTES_VERSION: u32 = 1;

/// One note a journal can hold.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteDef {
    /// Unique across every notes file of the game.
    pub id: String,
    /// The journal's section: `people`, `places`, `lore`, `earth`, `skills`. Free.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub category: String,
    /// What the note is about, for grouping (a cast id puts it on that person's page with their portrait).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subject: String,
    #[serde(default, skip_serializing_if = "LocalText::is_empty")]
    pub title: LocalText,
    pub text: LocalText,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tier: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// One thing the Voice can announce: Rimeborn's interface voice ("Confirmed. Weave acquired: Frost Step."). Its log is
/// a page of the journal; each announcement keeps the values its placeholders had when it was made.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceDef {
    /// Unique across every notes file of the game.
    pub id: String,
    /// What kind of announcement (`weave`, `skill`, `rank`, `trial`). Free: the game styles and files by it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub category: String,
    /// The small line above ("WEAVE ACQUIRED").
    #[serde(default, skip_serializing_if = "LocalText::is_empty")]
    pub heading: LocalText,
    pub text: LocalText,
    /// A clip of the Voice saying it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub voice: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// A notes file (`*.somnotes`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotesAsset {
    pub version: u32,
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<NoteDef>,
    /// What the Voice can say.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub voice: Vec<VoiceDef>,
}

impl NotesAsset {
    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }

    /// Problems that need no manifest; tiers and subjects are the database's.
    #[must_use]
    pub fn structural_problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        if self.version != NOTES_VERSION {
            problems.push(Problem::error(
                "",
                format!("notes version {} is not {NOTES_VERSION}", self.version),
            ));
        }
        if !valid_id(&self.id) {
            problems.push(Problem::error(
                "",
                format!("notes id \"{}\" is not a valid id", self.id),
            ));
        }
        let mut ids = BTreeSet::new();
        for note in &self.notes {
            if !valid_id(&note.id) || !ids.insert(note.id.as_str()) {
                problems.push(Problem::error(&note.id, "note ids are valid ids, unique"));
            }
            if note.text.is_empty() {
                problems.push(Problem::error(&note.id, "a note needs text"));
            }
        }
        let mut voice_ids = BTreeSet::new();
        for line in &self.voice {
            if !valid_id(&line.id) || !voice_ids.insert(line.id.as_str()) {
                problems.push(Problem::error(&line.id, "voice ids are valid ids, unique"));
            }
            if line.text.is_empty() {
                problems.push(Problem::error(&line.id, "a voice line needs text"));
            }
        }
        problems
    }

    /// Give each note's text its key: `note.<id>.title`, `note.<id>`.
    pub fn assign_keys(&mut self) {
        for note in &mut self.notes {
            let id = note.id.clone();
            note.title.assign_key(|| format!("note.{id}.title"));
            note.text.assign_key(|| format!("note.{id}"));
        }
        for line in &mut self.voice {
            let id = line.id.clone();
            line.heading.assign_key(|| format!("voice.{id}.heading"));
            line.text.assign_key(|| format!("voice.{id}"));
        }
    }
}

/// What a journal entry refers to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntryRef {
    /// A quest stage's journal line.
    Quest {
        quest: String,
        stage: String,
        line: String,
    },
    Note {
        note: String,
    },
    /// Something the Voice announced, with the facts its text names as they were then.
    Voice {
        line: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        values: BTreeMap<String, Value>,
    },
}

/// One entry in the log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Order of arrival; never reused.
    pub sequence: u64,
    #[serde(flatten)]
    pub entry: EntryRef,
    /// Game time it was written, in milliseconds.
    pub at_ms: u64,
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub obscured: bool,
}

/// The log itself, saved with the story.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    entries: Vec<JournalEntry>,
    next: u64,
}

impl Journal {
    /// Add an entry unless the same one is already there. Returns its sequence when added.
    pub fn add(&mut self, entry: EntryRef, at_ms: u64) -> Option<u64> {
        if self.entries.iter().any(|e| e.entry == entry) {
            return None;
        }
        let sequence = self.next;
        self.next += 1;
        self.entries.push(JournalEntry {
            sequence,
            entry,
            at_ms,
            read: false,
            obscured: false,
        });
        Some(sequence)
    }

    /// Add an entry even when the same one is there (the Voice may say the same thing twice). Returns its sequence.
    pub fn push(&mut self, entry: EntryRef, at_ms: u64) -> u64 {
        let sequence = self.next;
        self.next += 1;
        self.entries.push(JournalEntry {
            sequence,
            entry,
            at_ms,
            read: false,
            obscured: false,
        });
        sequence
    }

    #[must_use]
    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }

    #[must_use]
    pub fn has_note(&self, note: &str) -> bool {
        self.entries
            .iter()
            .any(|e| matches!(&e.entry, EntryRef::Note { note: n } if n == note))
    }

    #[must_use]
    pub fn unread(&self) -> usize {
        self.entries.iter().filter(|e| !e.read).count()
    }

    /// Mark one entry read, or every entry with `None`.
    pub fn mark_read(&mut self, sequence: Option<u64>) {
        for entry in &mut self.entries {
            if sequence.is_none_or(|s| s == entry.sequence) {
                entry.read = true;
            }
        }
    }

    /// Obscure or restore a note. Returns whether anything changed.
    pub fn set_obscured(&mut self, note: &str, obscured: bool) -> bool {
        let mut changed = false;
        for entry in &mut self.entries {
            if matches!(&entry.entry, EntryRef::Note { note: n } if n == note)
                && entry.obscured != obscured
            {
                entry.obscured = obscured;
                changed = true;
            }
        }
        changed
    }

    /// Drop entries `keep` refuses (content removed between builds). Returns how many went.
    pub fn retain(&mut self, mut keep: impl FnMut(&EntryRef) -> bool) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| keep(&e.entry));
        before - self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_arrive_once_in_order_and_can_be_obscured() {
        let mut journal = Journal::default();
        let note = |n: &str| EntryRef::Note { note: n.into() };
        assert_eq!(journal.add(note("earth_mother"), 10), Some(0));
        assert_eq!(
            journal.add(note("earth_mother"), 20),
            None,
            "a note arrives once"
        );
        assert_eq!(
            journal.add(
                EntryRef::Quest {
                    quest: "q".into(),
                    stage: "s".into(),
                    line: "j1".into()
                },
                30
            ),
            Some(1)
        );
        assert_eq!(journal.unread(), 2);
        journal.mark_read(Some(0));
        assert_eq!(journal.unread(), 1);
        assert!(journal.set_obscured("earth_mother", true));
        assert!(!journal.set_obscured("earth_mother", true));
        let json = serde_json::to_string(&journal).unwrap();
        assert!(json.contains(r#""kind":"note""#), "{json}");
        assert_eq!(serde_json::from_str::<Journal>(&json).unwrap(), journal);
    }

    #[test]
    fn notes_files_are_checked_and_keyed() {
        let mut notes = NotesAsset::from_json(
            r#"{"version": 1, "id": "earth", "notes": [
                {"id": "earth_mother", "category": "earth", "title": "Ma", "text": "She hummed when she cooked.", "tier": "witnessed"},
                {"id": "earth_mother", "text": ""}]}"#,
        )
        .unwrap();
        assert_eq!(notes.structural_problems().len(), 2);
        notes.assign_keys();
        assert_eq!(notes.notes[0].text.key, "note.earth_mother");
    }
}
