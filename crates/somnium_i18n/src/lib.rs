//! String tables, plural rules and runtime locale switching (MORROWIND-AH).
//!
//! MORROWIND-G built the *hook* — `somnium_ui`'s `Resolver` trait, a key plus
//! typed arguments — and argued why a formatted string is the wrong shape. This
//! crate is what sits behind it.
//!
//! # No engine dependencies, deliberately
//!
//! A translation table is data. This crate depends on `serde` and nothing else,
//! so it is usable by a build tool, a server, a test or the extraction pass —
//! and so `somnium_ui` is not made to depend on it. `somnium_core` implements
//! the `Resolver` trait, which is where wiring belongs.
//!
//! # Fallback is a chain, not a flag
//!
//! A key missing from `pt-BR` should try `pt`, then the default locale, then
//! show the key. Three reasons that chain matters more than it looks:
//!
//! - **Regional tables are mostly empty.** `pt-BR` differs from `pt` in a
//!   handful of strings, and requiring a complete table per region means every
//!   region is a full retranslation.
//! - **Partial translation is the normal state.** A language ships at 60%
//!   while the rest is in flight, and the untranslated part must read as the
//!   default language rather than as blanks.
//! - **The last resort is the key itself**, never an empty string. G's argument
//!   holds: a blank label looks like a layout bug and gets filed against the
//!   wrong system.

#![deny(missing_docs)]

pub mod extract;
pub mod gender;
pub mod number;
pub mod plural;

pub use extract::{Extraction, ExtractionIssue, check_tables};
pub use gender::Gender;
pub use number::Numbers;
pub use plural::{Plural, PluralRule};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A BCP 47 language tag: `en`, `pt-BR`, `ja`.
pub type LocaleTag = String;

/// One locale's strings.
///
/// `BTreeMap` rather than `HashMap` so a serialised table is **ordered and
/// diffable**. A translation file is reviewed by humans and merged by git, and
/// a hash-ordered one produces a different diff on every save.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Table {
    /// The language this table is for.
    pub locale: LocaleTag,
    /// Human-readable name, for a language picker. In the language itself —
    /// a player looking for their language is looking for "Deutsch", not for
    /// "German" in a language they do not read.
    #[serde(default)]
    pub display_name: String,
    /// Font families this language needs, most specific first.
    ///
    /// **This is the coordination with MORROWIND-G's `FallbackChain`** and it
    /// lives in the translation file rather than in the UI for one reason: the
    /// person who knows Japanese needs a CJK face is the person adding the
    /// Japanese table, and requiring a code change alongside every new language
    /// means a language can ship as a screen full of tofu boxes with the
    /// translation itself perfectly correct.
    ///
    /// G resolves *coverage* — which loaded face has this codepoint — and that
    /// stays in G, because it is a font question. This is the other half: which
    /// faces to load in the first place.
    #[serde(default)]
    pub fonts: Vec<String>,
    /// `key -> text`. A plural key stores one entry per category as
    /// `key.one`, `key.other`, and so on.
    #[serde(default)]
    pub strings: BTreeMap<String, String>,
}

impl Table {
    /// An empty table for `locale`.
    #[must_use]
    pub fn new(locale: impl Into<LocaleTag>) -> Self {
        Self {
            locale: locale.into(),
            display_name: String::new(),
            fonts: Vec::new(),
            strings: BTreeMap::new(),
        }
    }

    /// Add a string.
    #[must_use]
    pub fn with(mut self, key: impl Into<String>, text: impl Into<String>) -> Self {
        self.strings.insert(key.into(), text.into());
        self
    }

    /// The plural rule this table's language follows.
    #[must_use]
    pub fn plural_rule(&self) -> PluralRule {
        PluralRule::for_language(&self.locale)
    }

    /// Look a key up, without fallback.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.strings.get(key).map(String::as_str)
    }

    /// How many strings this table holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// Whether the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }
}

/// Loaded tables and the current locale.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    tables: BTreeMap<LocaleTag, Table>,
    current: LocaleTag,
    default: LocaleTag,
}

impl Catalog {
    /// A catalog whose default locale is `default`.
    ///
    /// The default is the language the game is authored in, and it is the last
    /// table tried before the key itself.
    #[must_use]
    pub fn new(default: impl Into<LocaleTag>) -> Self {
        let default = default.into();
        Self {
            tables: BTreeMap::new(),
            current: default.clone(),
            default,
        }
    }

    /// Add or replace a table.
    pub fn insert(&mut self, table: Table) {
        self.tables.insert(table.locale.clone(), table);
    }

    /// Switch locale. Returns whether a table for it exists.
    ///
    /// **Switching to a locale with no table still succeeds in setting it**, and
    /// returns `false`. That is deliberate: the fallback chain will serve the
    /// default language, which is better than refusing the switch and leaving a
    /// player who picked their language looking at an unchanged menu with no
    /// explanation.
    pub fn set_locale(&mut self, locale: impl Into<LocaleTag>) -> bool {
        self.current = locale.into();
        self.tables.contains_key(&self.current)
    }

    /// The current locale tag.
    #[must_use]
    pub fn locale(&self) -> &str {
        &self.current
    }

    /// The default locale tag.
    #[must_use]
    pub fn default_locale(&self) -> &str {
        &self.default
    }

    /// Every loaded locale, sorted.
    #[must_use]
    pub fn locales(&self) -> Vec<&str> {
        self.tables.keys().map(String::as_str).collect()
    }

    /// `(tag, display name)` for a language picker.
    #[must_use]
    pub fn language_options(&self) -> Vec<(&str, &str)> {
        self.tables
            .values()
            .map(|t| {
                (
                    t.locale.as_str(),
                    if t.display_name.is_empty() {
                        t.locale.as_str()
                    } else {
                        t.display_name.as_str()
                    },
                )
            })
            .collect()
    }

    /// A table by tag.
    #[must_use]
    pub fn table(&self, locale: &str) -> Option<&Table> {
        self.tables.get(locale)
    }

    /// The tables to try, in order: current, its base language, then default.
    ///
    /// `pt-BR` -> `pt` -> `en`. Duplicates are dropped, so a catalog whose
    /// current locale *is* the default tries it once.
    #[must_use]
    pub fn fallback_chain(&self) -> Vec<&str> {
        let mut chain: Vec<&str> = Vec::new();
        let mut push = |tag: &str| {
            // Compare by value against what is already queued: `pt-BR` and `pt`
            // are different entries, but `en` twice is not.
            if !chain.contains(&tag)
                && let Some(table) = self.tables.get(tag)
            {
                chain.push(table.locale.as_str());
            }
        };
        push(&self.current);
        if let Some((base, _)) = self.current.split_once('-') {
            push(base);
        }
        push(&self.default);
        chain
    }

    /// Resolve `key` through the fallback chain.
    ///
    /// Returns `None` when no table has it — the caller shows the key, per
    /// MORROWIND-G's rule.
    #[must_use]
    pub fn lookup(&self, key: &str) -> Option<&str> {
        self.fallback_chain()
            .into_iter()
            .find_map(|tag| self.tables.get(tag).and_then(|table| table.get(key)))
    }

    /// Resolve a plural key for `count`.
    ///
    /// Tries `key.<category>` then `key.other`, **using the plural rule of the
    /// table the string was found in** — not of the current locale. A string
    /// falling back from Polish to English must be selected by English's rule,
    /// or a count of 22 asks an English table for a `few` form it has no reason
    /// to contain.
    #[must_use]
    pub fn lookup_plural(&self, key: &str, count: i64) -> Option<&str> {
        for tag in self.fallback_chain() {
            let table = self.tables.get(tag)?;
            let category = table.plural_rule().select(count);
            if let Some(text) = table.get(&format!("{key}.{}", category.suffix())) {
                return Some(text);
            }
            // `other` exists in every rule, so it is the within-table fallback
            // before moving to the next locale.
            if let Some(text) = table.get(&format!("{key}.{}", Plural::Other.suffix())) {
                return Some(text);
            }
            // A non-plural key under the same name is still better than nothing.
            if let Some(text) = table.get(key) {
                return Some(text);
            }
        }
        None
    }

    /// Resolve a gendered key.
    ///
    /// Tries `key.<gender>`, then `key.other`, then the plain `key`, then the
    /// next locale. The plain-key step is what lets **a language with no
    /// gendered forms store one string** and pay nothing: English writes
    /// `state.tired` once and every gender finds it.
    #[must_use]
    pub fn lookup_gendered(&self, key: &str, gender: Gender) -> Option<&str> {
        for tag in self.fallback_chain() {
            let table = self.tables.get(tag)?;
            if let Some(text) = table.get(&format!("{key}.{}", gender.suffix())) {
                return Some(text);
            }
            if let Some(text) = table.get(&format!("{key}.{}", Gender::Other.suffix())) {
                return Some(text);
            }
            if let Some(text) = table.get(key) {
                return Some(text);
            }
        }
        None
    }

    /// The font families to load for the current locale, through the fallback
    /// chain — so `pt-BR` inherits `pt`'s fonts without repeating them.
    #[must_use]
    pub fn fonts(&self) -> &[String] {
        for tag in self.fallback_chain() {
            if let Some(table) = self.tables.get(tag)
                && !table.fonts.is_empty()
            {
                return &table.fonts;
            }
        }
        &[]
    }

    /// Substitute `{name}` placeholders.
    ///
    /// Deliberately simple: one pass, no nesting, no expressions. Anything more
    /// is a template language inside a translation file, and the person editing
    /// it is a translator rather than a programmer.
    ///
    /// An unmatched placeholder is **left in the text** rather than blanked, so
    /// a missing argument shows as `{count}` on screen — findable — instead of
    /// as a gap that reads like a typo in the translation.
    #[must_use]
    pub fn substitute(template: &str, args: &[(&str, String)]) -> String {
        if args.is_empty() || !template.contains('{') {
            return template.to_string();
        }
        let mut out = template.to_string();
        for (name, value) in args {
            out = out.replace(&format!("{{{name}}}"), value);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        let mut catalog = Catalog::new("en");
        catalog.insert(
            Table::new("en")
                .with("menu.quit", "Quit")
                .with("inventory.potions.one", "{n} potion")
                .with("inventory.potions.other", "{n} potions"),
        );
        let mut pt = Table::new("pt").with("menu.quit", "Sair");
        pt.display_name = "Portugues".into();
        catalog.insert(pt);
        catalog.insert(Table::new("pt-BR").with("menu.quit", "Sair do jogo"));
        catalog.insert(
            Table::new("pl")
                .with("inventory.potions.one", "{n} mikstura")
                .with("inventory.potions.few", "{n} mikstury")
                .with("inventory.potions.many", "{n} mikstur"),
        );
        catalog
    }

    #[test]
    fn the_current_locale_wins() {
        let mut catalog = catalog();
        assert_eq!(catalog.lookup("menu.quit"), Some("Quit"));
        assert!(catalog.set_locale("pt"));
        assert_eq!(catalog.lookup("menu.quit"), Some("Sair"));
    }

    /// **A regional table falls back to its base language.**
    ///
    /// `pt-BR` differs from `pt` in a handful of strings; requiring a complete
    /// table per region makes every region a full retranslation.
    #[test]
    fn a_regional_locale_falls_back_to_its_base() {
        let mut catalog = catalog();
        catalog.set_locale("pt-BR");
        assert_eq!(
            catalog.lookup("menu.quit"),
            Some("Sair do jogo"),
            "its own string"
        );
        assert_eq!(
            catalog.fallback_chain(),
            vec!["pt-BR", "pt", "en"],
            "then its base, then the default"
        );
    }

    /// A partially translated locale reads as the default for what is missing.
    #[test]
    fn a_missing_string_falls_back_to_the_default_language() {
        let mut catalog = catalog();
        catalog.set_locale("pl");
        assert_eq!(
            catalog.lookup("menu.quit"),
            Some("Quit"),
            "Polish has no menu.quit yet, so English shows"
        );
    }

    /// The last resort is `None`, so the caller can show the key.
    #[test]
    fn an_unknown_key_resolves_to_nothing() {
        assert_eq!(catalog().lookup("nope.at.all"), None);
    }

    /// The chain does not repeat the default when it is already current.
    #[test]
    fn the_chain_has_no_duplicates() {
        let catalog = catalog();
        assert_eq!(catalog.fallback_chain(), vec!["en"]);
    }

    /// **Switching to a locale with no table still switches.**
    ///
    /// Refusing would leave a player who picked their language looking at an
    /// unchanged menu with no explanation; the fallback chain serves the
    /// default, which is the honest degradation.
    #[test]
    fn switching_to_an_unloaded_locale_reports_but_proceeds() {
        let mut catalog = catalog();
        assert!(!catalog.set_locale("de"));
        assert_eq!(catalog.locale(), "de");
        assert_eq!(catalog.lookup("menu.quit"), Some("Quit"));
    }

    #[test]
    fn english_plurals_select_by_count() {
        let catalog = catalog();
        assert_eq!(
            catalog.lookup_plural("inventory.potions", 1),
            Some("{n} potion")
        );
        assert_eq!(
            catalog.lookup_plural("inventory.potions", 3),
            Some("{n} potions")
        );
        assert_eq!(
            catalog.lookup_plural("inventory.potions", 0),
            Some("{n} potions")
        );
    }

    /// **Polish gets three forms, and 22 gets the same one as 2.**
    ///
    /// This is the case a two-form system cannot express at all, and the reason
    /// MORROWIND-G insisted a count stay a number.
    #[test]
    fn polish_plurals_select_by_the_last_two_digits() {
        let mut catalog = catalog();
        catalog.set_locale("pl");
        assert_eq!(
            catalog.lookup_plural("inventory.potions", 1),
            Some("{n} mikstura")
        );
        assert_eq!(
            catalog.lookup_plural("inventory.potions", 2),
            Some("{n} mikstury")
        );
        assert_eq!(
            catalog.lookup_plural("inventory.potions", 5),
            Some("{n} mikstur")
        );
        assert_eq!(
            catalog.lookup_plural("inventory.potions", 12),
            Some("{n} mikstur")
        );
        assert_eq!(
            catalog.lookup_plural("inventory.potions", 22),
            Some("{n} mikstury")
        );
    }

    /// **A string falling back uses the rule of the table it came from.**
    ///
    /// A Polish count of 22 falling back to English must be selected by
    /// English's rule; asking the English table for a `few` form it has no
    /// reason to contain would find nothing and fall through to the key.
    #[test]
    fn a_fallback_plural_uses_the_fallback_languages_rule() {
        let mut catalog = Catalog::new("en");
        catalog.insert(
            Table::new("en")
                .with("count.one", "{n} thing")
                .with("count.other", "{n} things"),
        );
        catalog.insert(Table::new("pl")); // no strings at all
        catalog.set_locale("pl");
        assert_eq!(catalog.lookup_plural("count", 22), Some("{n} things"));
    }

    /// A table missing the selected category falls back to `other` within
    /// itself before changing language.
    #[test]
    fn a_missing_category_falls_back_to_other_in_the_same_table() {
        let mut catalog = Catalog::new("en");
        catalog.insert(Table::new("en").with("count.other", "{n} things"));
        assert_eq!(catalog.lookup_plural("count", 1), Some("{n} things"));
    }

    /// A non-plural key under the same name is better than nothing.
    #[test]
    fn a_plain_key_serves_a_plural_lookup() {
        let mut catalog = Catalog::new("en");
        catalog.insert(Table::new("en").with("count", "some things"));
        assert_eq!(catalog.lookup_plural("count", 5), Some("some things"));
    }

    #[test]
    fn substitution_replaces_named_placeholders() {
        assert_eq!(
            Catalog::substitute(
                "{n} potions for {who}",
                &[("n", "3".into()), ("who", "Nerevar".into())]
            ),
            "3 potions for Nerevar"
        );
    }

    /// **An unmatched placeholder stays visible.**
    ///
    /// Blanking it makes a missing argument read as a typo in the translation;
    /// leaving `{count}` on screen points at the call site.
    #[test]
    fn an_unmatched_placeholder_is_left_in_the_text() {
        assert_eq!(
            Catalog::substitute("{n} of {missing}", &[("n", "2".into())]),
            "2 of {missing}"
        );
    }

    #[test]
    fn substitution_with_no_arguments_is_the_template() {
        assert_eq!(Catalog::substitute("plain text", &[]), "plain text");
    }

    /// **A table serialises in a stable order.**
    ///
    /// A translation file is reviewed by humans and merged by git; a
    /// hash-ordered one produces a different diff on every save and makes
    /// review useless.
    #[test]
    fn a_table_round_trips_in_a_stable_order() {
        let table = Table::new("en")
            .with("z.last", "Z")
            .with("a.first", "A")
            .with("m.middle", "M");
        let json = serde_json::to_string_pretty(&table).expect("serialises");
        assert!(
            json.find("a.first").unwrap() < json.find("m.middle").unwrap(),
            "keys are ordered, so a diff is readable"
        );
        let back: Table = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(table, back);
        assert_eq!(json, serde_json::to_string_pretty(&back).unwrap());
    }

    /// **A new language brings its own fonts with it.**
    ///
    /// Requiring a code change alongside every new language means a language
    /// can ship as a screen full of tofu boxes with the translation itself
    /// perfectly correct.
    #[test]
    fn a_locale_names_the_fonts_it_needs() {
        let mut catalog = Catalog::new("en");
        let mut ja = Table::new("ja");
        ja.fonts = vec!["NotoSansJP".into(), "NotoSans".into()];
        catalog.insert(ja);
        catalog.insert(Table::new("en"));
        assert!(catalog.fonts().is_empty(), "English needs nothing special");
        catalog.set_locale("ja");
        assert_eq!(catalog.fonts(), ["NotoSansJP", "NotoSans"]);
    }

    /// A regional table inherits its base language's fonts rather than
    /// repeating them.
    #[test]
    fn a_region_inherits_its_languages_fonts() {
        let mut catalog = Catalog::new("en");
        let mut zh = Table::new("zh");
        zh.fonts = vec!["NotoSansSC".into()];
        catalog.insert(zh);
        catalog.insert(Table::new("zh-TW"));
        catalog.set_locale("zh-TW");
        assert_eq!(catalog.fonts(), ["NotoSansSC"]);
    }

    /// A language picker shows the language's own name.
    ///
    /// A player looking for their language is looking for "Deutsch", not for
    /// "German" written in a language they do not read.
    #[test]
    fn the_picker_shows_each_languages_own_name() {
        let catalog = catalog();
        let options = catalog.language_options();
        assert!(options.contains(&("pt", "Portugues")));
        // A table with no display name falls back to its tag rather than blank.
        assert!(options.contains(&("en", "en")));
    }
}
