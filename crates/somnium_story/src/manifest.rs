//! A game's story manifest (`*.somstory`): the facts it keeps, its cast, its truth tiers and the commands it accepts.
//!
//! One per game. Everything else (dialogue, quests, barks, journal notes) is checked against it, so it is the place a
//! writer looks up what `mireille.affection` means and which expressions Mireille's portrait has.

use crate::expr::CommandDef;
use crate::text::LocalText;
use crate::value::{FactDef, Schema, valid_id};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The manifest format this build reads.
pub const MANIFEST_VERSION: u32 = 1;

fn english() -> String {
    "en".into()
}

/// Which side of the dialogue box a speaker's portrait stands on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    #[default]
    Right,
}

/// One speaker: a character, a voice over a radio, the narrator.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Speaker {
    pub id: String,
    /// The name plate.
    pub name: LocalText,
    /// The portrait set the game resolves to images (`portraits/mireille`). Empty: no portrait (the narrator, a
    /// voice in the dark).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub portrait: String,
    /// Expressions beyond the manifest's base set (CAST's "portrait extras").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expressions: Vec<String>,
    /// The expression a line without one shows. Empty: the base set's first.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default_expression: String,
    #[serde(default)]
    pub side: Side,
    /// Name-plate colour, `#rrggbb`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
    /// The language this speaker uses unless a line says otherwise (the game's fluency transform reads it).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub language: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// On the journal's People page once met. Off for the player, the narrator, a voice on the wind.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub journal: bool,
}

fn yes() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

/// The manifest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryManifest {
    pub version: u32,
    pub id: String,
    /// The language content is authored in (a BCP 47 tag).
    #[serde(default = "english")]
    pub source_locale: String,
    #[serde(default)]
    pub facts: Vec<FactDef>,
    #[serde(default)]
    pub cast: Vec<Speaker>,
    /// Truth tiers, most certain first (Rimeborn: absolute, measured, witnessed, recorded, retold). Journal entries
    /// name one; the journal shows the player how a fact was learned.
    #[serde(default)]
    pub tiers: Vec<String>,
    /// Commands effects may send the game (`give_item('lantern', 1)`).
    #[serde(default)]
    pub commands: Vec<CommandDef>,
    /// The portrait expressions every speaker with a portrait has (Rimeborn's base ten).
    #[serde(default)]
    pub expressions: Vec<String>,
    /// Languages lines may be marked with. Empty: any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
}

impl StoryManifest {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            version: MANIFEST_VERSION,
            id: id.into(),
            source_locale: english(),
            facts: Vec::new(),
            cast: Vec::new(),
            tiers: Vec::new(),
            commands: Vec::new(),
            expressions: Vec::new(),
            languages: Vec::new(),
        }
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }

    /// The schema its facts make, and every problem found on the way.
    pub fn compile(&self) -> Result<Schema, Vec<String>> {
        let mut errors = Vec::new();
        if self.version != MANIFEST_VERSION {
            errors.push(format!(
                "manifest version {} is not {MANIFEST_VERSION}",
                self.version
            ));
        }
        if !valid_id(&self.id) {
            errors.push(format!("manifest id \"{}\" is not a valid id", self.id));
        }
        let mut seen = BTreeSet::new();
        for speaker in &self.cast {
            if !valid_id(&speaker.id) || !seen.insert(speaker.id.as_str()) {
                errors.push(format!(
                    "speaker id \"{}\" is invalid or repeated",
                    speaker.id
                ));
            }
            if speaker.name.is_empty() {
                errors.push(format!("speaker {} has no name", speaker.id));
            }
            if !speaker.default_expression.is_empty()
                && !self.has_expression(speaker, &speaker.default_expression)
            {
                errors.push(format!(
                    "speaker {}: default expression {} is not in the base set or their extras",
                    speaker.id, speaker.default_expression
                ));
            }
            if !speaker.color.is_empty() && parse_color(&speaker.color).is_none() {
                errors.push(format!(
                    "speaker {}: colour {} is not #rrggbb",
                    speaker.id, speaker.color
                ));
            }
            if !speaker.language.is_empty() && !self.has_language(&speaker.language) {
                errors.push(format!(
                    "speaker {}: language {} is not declared",
                    speaker.id, speaker.language
                ));
            }
        }
        let mut names = BTreeSet::new();
        for command in &self.commands {
            if !valid_id(&command.name)
                || command.name.contains('.')
                || !names.insert(command.name.as_str())
            {
                errors.push(format!(
                    "command name \"{}\" is invalid or repeated",
                    command.name
                ));
            }
            if crate::expr::BUILTIN_COMMANDS
                .iter()
                .any(|(name, _)| *name == command.name)
            {
                errors.push(format!(
                    "command {} is one of the story's own",
                    command.name
                ));
            }
        }
        for list in [&self.tiers, &self.expressions, &self.languages] {
            let mut seen = BTreeSet::new();
            for item in list {
                if item.trim().is_empty() || !seen.insert(item) {
                    errors.push(format!(
                        "\"{item}\" is empty or repeated in a manifest list"
                    ));
                }
            }
        }
        match Schema::new(self.facts.iter().cloned()) {
            Ok(schema) if errors.is_empty() => Ok(schema),
            Ok(_) => Err(errors),
            Err(mut fact_errors) => {
                errors.append(&mut fact_errors);
                Err(errors)
            }
        }
    }

    #[must_use]
    pub fn speaker(&self, id: &str) -> Option<&Speaker> {
        self.cast.iter().find(|s| s.id == id)
    }

    /// Whether `speaker`'s portrait has `expression`. Without a base set any expression is accepted.
    #[must_use]
    pub fn has_expression(&self, speaker: &Speaker, expression: &str) -> bool {
        (self.expressions.is_empty() && speaker.expressions.is_empty())
            || self.expressions.iter().any(|e| e == expression)
            || speaker.expressions.iter().any(|e| e == expression)
    }

    #[must_use]
    pub fn has_language(&self, language: &str) -> bool {
        self.languages.is_empty() || self.languages.iter().any(|l| l == language)
    }

    #[must_use]
    pub fn has_tier(&self, tier: &str) -> bool {
        tier.is_empty() || self.tiers.iter().any(|t| t == tier)
    }

    /// How certain a tier is: 0 for the first (most certain), `None` when undeclared or empty.
    #[must_use]
    pub fn tier_rank(&self, tier: &str) -> Option<usize> {
        self.tiers.iter().position(|t| t == tier)
    }
}

/// `#rrggbb` or `#rrggbbaa` to straight sRGB bytes.
#[must_use]
pub fn parse_color(text: &str) -> Option<[u8; 4]> {
    let hex = text.strip_prefix('#')?;
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    match hex.len() {
        6 => Some([byte(0)?, byte(2)?, byte(4)?, 255]),
        8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_reads_and_compiles() {
        let manifest = StoryManifest::from_json(
            r##"{
            "version": 1, "id": "rimeborn", "source_locale": "en",
            "facts": [{"id": "met.mireille", "type": "bool"}, {"id": "rep.*", "type": "int"}],
            "cast": [{"id": "mireille", "name": "Mireille", "portrait": "portraits/mireille",
                      "expressions": ["court_mask"], "side": "right", "color": "#c8a2ff", "language": "aurelian"}],
            "tiers": ["absolute", "measured", "witnessed", "recorded", "retold"],
            "commands": [{"name": "give_item", "params": ["text", "int"]}],
            "expressions": ["neutral", "smile"],
            "languages": ["aurelian", "tyffiran"]
        }"##,
        )
        .unwrap();
        let schema = manifest.compile().unwrap();
        assert_eq!(schema.len(), 2);
        let mireille = manifest.speaker("mireille").unwrap();
        assert!(manifest.has_expression(mireille, "court_mask"));
        assert!(manifest.has_expression(mireille, "smile"));
        assert!(!manifest.has_expression(mireille, "laugh"));
        assert_eq!(manifest.tier_rank("witnessed"), Some(2));
        assert_eq!(parse_color("#c8a2ff"), Some([0xc8, 0xa2, 0xff, 255]));
    }

    #[test]
    fn manifest_mistakes_are_collected_together() {
        let mut manifest = StoryManifest::new("bad id");
        manifest.cast.push(Speaker {
            id: "x".into(),
            name: LocalText::default(),
            portrait: String::new(),
            expressions: vec![],
            default_expression: "grin".into(),
            side: Side::Left,
            color: "red".into(),
            language: "klingon".into(),
            note: String::new(),
            journal: true,
        });
        manifest.expressions = vec!["neutral".into()];
        manifest.languages = vec!["tyffiran".into()];
        manifest.commands.push(CommandDef {
            name: "start_quest".into(),
            params: vec![],
            doc: String::new(),
        });
        manifest
            .facts
            .push(FactDef::new("e", crate::value::FactType::Enum));
        let errors = manifest.compile().unwrap_err();
        assert_eq!(errors.len(), 7, "{errors:#?}");
    }
}
