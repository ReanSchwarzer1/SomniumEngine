//! Localised story text: every line is a key, and the authored source travels with it.
//!
//! A writer authors in the source language, in place: `"text": "You're late, little lord."` on the dialogue node.
//! The database gives the text a stable key from where it lives (`dlg.mireille_intro.n3`) unless the author chose
//! one, extraction turns every key into a `somnium_i18n::Table` for translators, and at run time the line resolves
//! through the loaded `Catalog`, falling back to the source when a table lacks it. A missing translation therefore
//! reads as the source language, never as a key or a blank (the rule `somnium_i18n` already keeps).
//!
//! Plurals and grammatical gender select by a fact (`plural: "gold"`, `gender: "player.gender"`), and `{fact.id}`
//! placeholders are filled from the world state, with numbers in the reader's locale.

use crate::expr::Env;
use crate::value::{Value, valid_id};
use serde::{Deserialize, Serialize};
use somnium_i18n::{Catalog, Gender, Numbers, PluralRule};
use std::collections::BTreeMap;

/// One piece of player-facing story text.
///
/// In JSON it is either a plain string (the source text) or an object with the fields below.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "TextRepr", into = "TextRepr")]
pub struct LocalText {
    /// String-table key. Empty: the database assigns one from where the text lives.
    pub key: String,
    /// The text in the source language. Rich-text markup allowed.
    pub source: String,
    /// Source-language plural or gender forms by category (`one`, `other`; `feminine`, `masculine`, ...).
    pub forms: BTreeMap<String, String>,
    /// An int fact whose value selects the plural form.
    pub plural: String,
    /// An enum fact whose value (`masculine`, `feminine`, `neuter`, `common`, `other`) selects the gendered form.
    pub gender: String,
    /// A note for translators: who is speaking, to whom, what the joke is.
    pub note: String,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum TextRepr {
    Plain(String),
    Full(FullText),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FullText {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    source: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    forms: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    plural: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    gender: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    note: String,
}

impl From<TextRepr> for LocalText {
    fn from(repr: TextRepr) -> Self {
        match repr {
            TextRepr::Plain(source) => Self {
                source,
                ..Self::default()
            },
            TextRepr::Full(full) => Self {
                key: full.key,
                source: full.source,
                forms: full.forms,
                plural: full.plural,
                gender: full.gender,
                note: full.note,
            },
        }
    }
}

impl From<LocalText> for TextRepr {
    fn from(text: LocalText) -> Self {
        if text.key.is_empty()
            && text.forms.is_empty()
            && text.plural.is_empty()
            && text.gender.is_empty()
            && text.note.is_empty()
        {
            TextRepr::Plain(text.source)
        } else {
            TextRepr::Full(FullText {
                key: text.key,
                source: text.source,
                forms: text.forms,
                plural: text.plural,
                gender: text.gender,
                note: text.note,
            })
        }
    }
}

impl From<&str> for LocalText {
    fn from(source: &str) -> Self {
        Self {
            source: source.to_owned(),
            ..Self::default()
        }
    }
}

/// The plural and gender categories a form may be written for.
const FORM_NAMES: &[&str] = &[
    "zero",
    "one",
    "two",
    "few",
    "many",
    "other",
    "masculine",
    "feminine",
    "neuter",
    "common",
];

fn gender_of(value: &str) -> Gender {
    match value {
        "masculine" => Gender::Masculine,
        "feminine" => Gender::Feminine,
        "neuter" => Gender::Neuter,
        "common" => Gender::Common,
        _ => Gender::Other,
    }
}

impl LocalText {
    /// Whether there is nothing to show.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.source.trim().is_empty() && self.forms.values().all(|f| f.trim().is_empty())
    }

    /// Give the text a key when the author did not.
    pub fn assign_key(&mut self, key: impl FnOnce() -> String) {
        if self.key.is_empty() {
            self.key = key();
        }
    }

    /// Resolve for the reader: translated when `catalog` has it, the source otherwise, with forms selected and
    /// placeholders filled from `env`.
    #[must_use]
    pub fn resolve(&self, catalog: &Catalog, source_locale: &str, env: &dyn Env) -> String {
        let count = (!self.plural.is_empty()).then(|| env.fact(&self.plural).as_int());
        let gender = (!self.gender.is_empty()).then(|| gender_of(env.fact(&self.gender).as_text()));
        let translated = if self.key.is_empty() {
            None
        } else if let Some(count) = count {
            catalog.lookup_plural(&self.key, count)
        } else if let Some(gender) = gender {
            catalog.lookup_gendered(&self.key, gender)
        } else {
            catalog.lookup(&self.key)
        };
        let template = translated.map_or_else(
            || {
                let form = if let Some(count) = count {
                    let category = PluralRule::for_language(source_locale).select(count);
                    self.forms.get(category.suffix())
                } else {
                    gender.and_then(|gender| self.forms.get(gender.suffix()))
                };
                form.or_else(|| self.forms.get("other"))
                    .filter(|_| count.is_some() || gender.is_some())
                    .map_or(self.source.as_str(), String::as_str)
                    .to_owned()
            },
            str::to_owned,
        );
        let numbers = Numbers::for_language(catalog.locale());
        fill(&template, |name| {
            if name == "n" {
                return count.map(|count| numbers.integer(count));
            }
            valid_id(name).then(|| match env.fact(name) {
                Value::Int(value) => numbers.integer(value),
                other => other.to_string(),
            })
        })
    }

    /// The string-table entries this text contributes: `key` for the source, `key.<form>` for each form.
    #[must_use]
    pub fn table_entries(&self) -> Vec<(String, String)> {
        let mut entries = Vec::new();
        if !self.source.is_empty() || self.forms.is_empty() {
            entries.push((self.key.clone(), self.source.clone()));
        }
        for (form, text) in &self.forms {
            entries.push((format!("{}.{form}", self.key), text.clone()));
        }
        entries
    }

    /// Every `{name}` placeholder in the source and forms, for the lint.
    #[must_use]
    pub fn placeholders(&self) -> Vec<String> {
        let mut names = Vec::new();
        for text in std::iter::once(&self.source).chain(self.forms.values()) {
            let _ = fill(text, |name| {
                if !names.iter().any(|n| n == name) {
                    names.push(name.to_owned());
                }
                None
            });
        }
        names
    }

    /// Problems a writer should hear about: unknown form names, a form with nothing selecting it.
    #[must_use]
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for form in self.forms.keys() {
            if !FORM_NAMES.contains(&form.as_str()) {
                problems.push(format!("\"{form}\" is not a plural or gender form"));
            }
        }
        if !self.forms.is_empty() && self.plural.is_empty() && self.gender.is_empty() {
            problems.push("forms are given but no plural or gender fact selects one".into());
        }
        if !self.plural.is_empty() && !self.gender.is_empty() {
            problems.push("one text selects by plural or by gender, not both; split it".into());
        }
        problems
    }
}

/// Replace `{name}` with `value(name)`; a name with no value stays as written so it can be found on screen.
/// `{{` and `}}` are literal braces.
fn fill(template: &str, mut value: impl FnMut(&str) -> Option<String>) -> String {
    if !template.contains('{') && !template.contains("}}") {
        return template.to_owned();
    }
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open].replace("}}", "}"));
        let after = &rest[open + 1..];
        if let Some(stripped) = after.strip_prefix('{') {
            out.push('{');
            rest = stripped;
            continue;
        }
        match after.find('}') {
            Some(close) if !after[..close].contains('{') => {
                let name = &after[..close];
                match value(name.trim()) {
                    Some(text) => out.push_str(&text),
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[close + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(&rest.replace("}}", "}"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_i18n::Table;

    struct Facts;
    impl Env for Facts {
        fn fact(&self, id: &str) -> Value {
            match id {
                "gold" => Value::Int(1200),
                "one" => Value::Int(1),
                "player.gender" => "feminine".into(),
                "player.name" => "Caelan".into(),
                _ => Value::Bool(false),
            }
        }
    }

    #[test]
    fn plain_strings_and_objects_both_read() {
        let plain: LocalText = serde_json::from_str("\"Hello.\"").unwrap();
        assert_eq!(plain.source, "Hello.");
        assert_eq!(serde_json::to_string(&plain).unwrap(), "\"Hello.\"");
        let full: LocalText =
            serde_json::from_str(r#"{"key":"k","source":"Hi","note":"she is tired"}"#).unwrap();
        assert_eq!(
            (full.key.as_str(), full.note.as_str()),
            ("k", "she is tired")
        );
        assert!(serde_json::from_str::<LocalText>(r#"{"sorce":"typo"}"#).is_err());
    }

    #[test]
    fn source_is_the_fallback_and_placeholders_fill_from_facts() {
        let mut text =
            LocalText::from("Welcome back, {player.name}. You owe {gold} crowns. {missing.fact");
        text.key = "dlg.a.n1".into();
        let catalog = Catalog::new("en");
        assert_eq!(
            text.resolve(&catalog, "en", &Facts),
            "Welcome back, Caelan. You owe 1,200 crowns. {missing.fact"
        );
        let mut catalog = Catalog::new("en");
        catalog
            .insert(Table::new("de").with("dlg.a.n1", "Willkommen, {player.name}. {gold} Kronen."));
        catalog.set_locale("de");
        assert_eq!(
            text.resolve(&catalog, "en", &Facts),
            "Willkommen, Caelan. 1.200 Kronen."
        );
    }

    #[test]
    fn plural_and_gender_forms_select_by_fact() {
        let coins = LocalText {
            key: "k.coins".into(),
            forms: [
                ("one".into(), "{n} coin".into()),
                ("other".into(), "{n} coins".into()),
            ]
            .into(),
            plural: "one".into(),
            ..LocalText::default()
        };
        assert_eq!(coins.resolve(&Catalog::new("en"), "en", &Facts), "1 coin");
        let tired = LocalText {
            key: "k.tired".into(),
            source: "You look tired.".into(),
            forms: [("feminine".into(), "Estás cansada.".into())].into(),
            gender: "player.gender".into(),
            ..LocalText::default()
        };
        assert_eq!(
            tired.resolve(&Catalog::new("es"), "es", &Facts),
            "Estás cansada."
        );
        let entries = coins.table_entries();
        assert_eq!(
            entries,
            [
                ("k.coins.one".into(), "{n} coin".into()),
                ("k.coins.other".into(), "{n} coins".into())
            ]
        );
        assert!(coins.problems().is_empty());
    }

    #[test]
    fn placeholders_and_problems_are_reported() {
        let text = LocalText {
            source: "{a} and {b.c} and {{literal}} and {a}".into(),
            forms: [("plenty".into(), "x".into())].into(),
            ..LocalText::default()
        };
        assert_eq!(text.placeholders(), ["a", "b.c"]);
        assert_eq!(text.problems().len(), 2);
        assert_eq!(fill("{{x}} {y}", |_| Some("v".into())), "{x} v");
    }
}
