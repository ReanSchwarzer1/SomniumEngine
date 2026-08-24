//! Grammatical gender selection.
//!
//! §8 item 1 asks for "plural **and gender** rules", and the two are the same
//! mechanism pointed at a different variable: a key with one entry per
//! category, selected at resolve time from a value the call site passes.
//!
//! # What this is for, and what it is not
//!
//! It is for the case where **a string's own words change** because of who or
//! what it refers to:
//!
//! - Spanish: *"Estás cansado"* / *"Estás cansada"* depending on the player
//!   character. The adjective agrees; there is no neutral form to fall back on.
//! - Russian past tense inflects for gender: *"нашёл"* / *"нашла"* for the same
//!   verb, so "You found a key" has two forms before any noun is involved.
//! - German needs the article of an inserted noun: *"der Schlüssel"* vs
//!   *"die Tür"*, so `"You found {item}"` cannot be one string with a slot.
//!
//! That last case is the one that catches teams out. It is not about the
//! player's gender at all — it is the **grammatical gender of a substituted
//! noun**, which is a property of the word in that language and differs between
//! languages for the same object. So [`Gender`] is attached to whatever is being
//! referred to, not to a person, and the API takes it as an argument rather than
//! reading it from a character sheet.
//!
//! # Why this is not a policy about players
//!
//! [`Gender::Neuter`] and [`Gender::Common`] are here because languages have
//! them, and a game whose character creator offers a non-binary option needs a
//! category its translators can write into rather than a forced choice between
//! two. English needs none of this and gets `Other` for free.

use serde::{Deserialize, Serialize};

/// A grammatical gender a string can be written for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Gender {
    /// Masculine.
    Masculine,
    /// Feminine.
    Feminine,
    /// Neuter — a distinct third category in German, Russian, Dutch.
    Neuter,
    /// Common gender, where masculine and feminine have merged (Swedish,
    /// Danish, Dutch `de`-words).
    Common,
    /// Unspecified, and the fallback every table must be able to answer with.
    Other,
}

impl Gender {
    /// The key suffix a table uses: `greeting.feminine`.
    #[must_use]
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Masculine => "masculine",
            Self::Feminine => "feminine",
            Self::Neuter => "neuter",
            Self::Common => "common",
            Self::Other => "other",
        }
    }

    /// The gender a suffix names, if it names one.
    #[must_use]
    pub fn from_suffix(suffix: &str) -> Option<Self> {
        Some(match suffix {
            "masculine" => Self::Masculine,
            "feminine" => Self::Feminine,
            "neuter" => Self::Neuter,
            "common" => Self::Common,
            "other" => Self::Other,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Catalog, Table};

    fn catalog() -> Catalog {
        let mut catalog = Catalog::new("en");
        catalog.insert(Table::new("en").with("state.tired", "You are tired"));
        catalog.insert(
            Table::new("es")
                .with("state.tired.masculine", "Estas cansado")
                .with("state.tired.feminine", "Estas cansada"),
        );
        catalog
    }

    /// **The words themselves change**, which no placeholder can express.
    #[test]
    fn a_gendered_language_gets_the_right_form() {
        let mut catalog = catalog();
        catalog.set_locale("es");
        assert_eq!(
            catalog.lookup_gendered("state.tired", Gender::Masculine),
            Some("Estas cansado")
        );
        assert_eq!(
            catalog.lookup_gendered("state.tired", Gender::Feminine),
            Some("Estas cansada")
        );
    }

    /// **English needs none of this and pays nothing for it.**
    ///
    /// A language with no gendered forms stores one string under the plain key,
    /// and every gender resolves to it.
    #[test]
    fn a_language_without_gender_stores_one_string() {
        let catalog = catalog();
        for gender in [Gender::Masculine, Gender::Feminine, Gender::Other] {
            assert_eq!(
                catalog.lookup_gendered("state.tired", gender),
                Some("You are tired")
            );
        }
    }

    /// A gender a table has no form for falls back within the table before
    /// changing language — the same order the plural path uses.
    #[test]
    fn an_unwritten_gender_falls_back_to_other_then_to_the_plain_key() {
        let mut catalog = Catalog::new("en");
        catalog.insert(
            Table::new("en")
                .with("hello.other", "Hello, friend")
                .with("hello.masculine", "Hello, sir"),
        );
        assert_eq!(
            catalog.lookup_gendered("hello", Gender::Masculine),
            Some("Hello, sir")
        );
        assert_eq!(
            catalog.lookup_gendered("hello", Gender::Neuter),
            Some("Hello, friend"),
            "a category nobody wrote falls back rather than blanking"
        );
    }

    #[test]
    fn suffixes_round_trip() {
        for gender in [
            Gender::Masculine,
            Gender::Feminine,
            Gender::Neuter,
            Gender::Common,
            Gender::Other,
        ] {
            assert_eq!(Gender::from_suffix(gender.suffix()), Some(gender));
        }
        assert_eq!(Gender::from_suffix("plural"), None);
    }
}
