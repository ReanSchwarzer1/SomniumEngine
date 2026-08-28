//! Plural categories (MORROWIND-AH).
//!
//! MORROWIND-G's localisation hook argued that a count must stay a **number**
//! until the resolver sees it, because a substituted `{n}` cannot choose
//! between plural forms. This is the module that does the choosing.
//!
//! # CLDR's six categories, and why not "one and many"
//!
//! English has two forms and it is tempting to build for two. The languages
//! that break it are not obscure:
//!
//! - **Polish** has three: 1 file, 2–4 pliki, 5+ plików — and the rule depends
//!   on the *last two digits*, so 22 takes the same form as 2 while 12 does not.
//! - **Russian** has the same shape.
//! - **Arabic** has six, including distinct forms for zero and for exactly two.
//! - **Japanese and Chinese** have one, so a table that demands a plural form
//!   makes every translator write the same string twice.
//!
//! A two-form system does not degrade gracefully into any of those; it produces
//! text that is *wrong in a way native speakers find jarring* while looking
//! completely fine to the person who wrote it.
//!
//! # Scope, stated
//!
//! These are CLDR's **cardinal** rules for the languages Somnium can plausibly
//! ship in, hand-written rather than generated from CLDR data. That is a real
//! limitation: a language not listed here falls back to the English rule, which
//! is wrong for it. It is recorded rather than hidden, and
//! [`PluralRule::for_language`] returning [`PluralRule::Other`] for an unknown
//! tag is what makes it visible in a test rather than in a review.
//!
//! Ordinals ("1st", "2nd") are **not** here. They are a different CLDR rule set
//! and nothing in the engine formats one.

use serde::{Deserialize, Serialize};

/// A CLDR plural category.
///
/// Not every language uses every category. A table supplies the ones its
/// language needs and [`PluralRule::select`] never asks for one it does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Plural {
    /// Arabic's distinct zero form.
    Zero,
    /// The singular.
    One,
    /// A distinct dual, as in Arabic and Slovenian.
    Two,
    /// Polish's 2–4 form; Arabic's 3–10.
    Few,
    /// Polish's 5+ form; Arabic's 11–99.
    Many,
    /// Everything else. The only category every language has, which is why it
    /// is the fallback.
    Other,
}

impl Plural {
    /// The key suffix a table uses: `count.one`, `count.other`.
    #[must_use]
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Zero => "zero",
            Self::One => "one",
            Self::Two => "two",
            Self::Few => "few",
            Self::Many => "many",
            Self::Other => "other",
        }
    }

    /// The category a key suffix names, if it names one.
    ///
    /// The extraction tool needs this to tell `inventory.potions.one` (a
    /// category of a plural key) from `menu.settings.one` (a key that happens
    /// to end in a word). It cannot: nothing distinguishes them but the name.
    /// The trade is deliberate — treating a real plural's categories as four
    /// separate unused keys would report every translated plural in the game as
    /// dead weight, which is the louder wrong answer.
    #[must_use]
    pub fn from_suffix(suffix: &str) -> Option<Self> {
        Some(match suffix {
            "zero" => Self::Zero,
            "one" => Self::One,
            "two" => Self::Two,
            "few" => Self::Few,
            "many" => Self::Many,
            "other" => Self::Other,
            _ => return None,
        })
    }
}

/// Which CLDR rule family a language follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluralRule {
    /// One form for every count. Japanese, Chinese, Korean, Thai, Vietnamese.
    ///
    /// A table for these must not be forced to supply a plural, or every
    /// translator writes the same string twice and one of the two eventually
    /// drifts.
    Other,
    /// `1` is one, everything else is other. English, German, Spanish, Italian,
    /// Dutch, and most of Europe.
    OneOther,
    /// `0` and `1` are one. French, Brazilian Portuguese.
    ///
    /// The difference from [`Self::OneOther`] is exactly one string — "0 file"
    /// versus "0 files" — and it is the kind of thing a French reviewer notices
    /// immediately.
    ZeroOneOther,
    /// Polish and Russian: one / few / many, by the last two digits.
    Slavic,
    /// Arabic's six categories.
    Arabic,
}

impl Default for PluralRule {
    fn default() -> Self {
        Self::OneOther
    }
}

impl PluralRule {
    /// The rule for a BCP 47 language tag.
    ///
    /// Only the primary subtag is read: `pt-BR` and `pt` differ in plural rules
    /// in principle, and the difference is not one Somnium's tables express, so
    /// pretending to handle regions here would be a lie the type tells.
    ///
    /// Returns [`Self::OneOther`] for an unknown tag — the English rule, which
    /// is wrong for a language that needs another. That is stated in the module
    /// docs and asserted in a test rather than being a silent fallback.
    #[must_use]
    pub fn for_language(tag: &str) -> Self {
        let primary = tag
            .split(['-', '_'])
            .next()
            .unwrap_or(tag)
            .to_ascii_lowercase();
        match primary.as_str() {
            "ja" | "zh" | "ko" | "th" | "vi" | "id" | "ms" => Self::Other,
            "fr" | "pt" | "hy" | "ff" => Self::ZeroOneOther,
            "pl" | "ru" | "uk" | "be" | "hr" | "sr" | "bs" | "cs" | "sk" => Self::Slavic,
            "ar" => Self::Arabic,
            _ => Self::OneOther,
        }
    }

    /// The category for `count`.
    #[must_use]
    pub fn select(self, count: i64) -> Plural {
        // Sign is not a plural distinction in any CLDR rule: -1 takes the same
        // form as 1. Taking the magnitude first is what makes a negative count
        // — a debt, a temperature, a score — read correctly instead of falling
        // through to `Other`.
        let n = count.unsigned_abs();
        match self {
            Self::Other => Plural::Other,
            Self::OneOther => {
                if n == 1 {
                    Plural::One
                } else {
                    Plural::Other
                }
            }
            Self::ZeroOneOther => {
                if n <= 1 {
                    Plural::One
                } else {
                    Plural::Other
                }
            }
            Self::Slavic => {
                let last = n % 10;
                let last_two = n % 100;
                if last == 1 && last_two != 11 {
                    Plural::One
                } else if (2..=4).contains(&last) && !(12..=14).contains(&last_two) {
                    Plural::Few
                } else {
                    Plural::Many
                }
            }
            Self::Arabic => {
                let last_two = n % 100;
                match n {
                    0 => Plural::Zero,
                    1 => Plural::One,
                    2 => Plural::Two,
                    _ if (3..=10).contains(&last_two) => Plural::Few,
                    _ if (11..=99).contains(&last_two) => Plural::Many,
                    _ => Plural::Other,
                }
            }
        }
    }

    /// Every category this rule can produce, so a table can be checked for
    /// completeness rather than discovering a gap at 22.
    #[must_use]
    pub fn categories(self) -> &'static [Plural] {
        match self {
            Self::Other => &[Plural::Other],
            Self::OneOther | Self::ZeroOneOther => &[Plural::One, Plural::Other],
            Self::Slavic => &[Plural::One, Plural::Few, Plural::Many],
            Self::Arabic => &[
                Plural::Zero,
                Plural::One,
                Plural::Two,
                Plural::Few,
                Plural::Many,
                Plural::Other,
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_has_two_forms() {
        let rule = PluralRule::for_language("en");
        assert_eq!(rule, PluralRule::OneOther);
        assert_eq!(rule.select(0), Plural::Other);
        assert_eq!(rule.select(1), Plural::One);
        assert_eq!(rule.select(2), Plural::Other);
    }

    /// French treats zero as singular. The difference from English is exactly
    /// one string, and a French reviewer notices it immediately.
    #[test]
    fn french_treats_zero_as_singular() {
        let rule = PluralRule::for_language("fr");
        assert_eq!(rule.select(0), Plural::One);
        assert_eq!(rule.select(1), Plural::One);
        assert_eq!(rule.select(2), Plural::Other);
    }

    /// **Polish depends on the last two digits.**
    ///
    /// 22 takes the same form as 2 while 12 does not, which is the case a
    /// naive `n < 5` rule gets wrong and nobody who wrote it would think to
    /// check.
    #[test]
    fn polish_uses_the_last_two_digits() {
        let rule = PluralRule::for_language("pl");
        assert_eq!(rule, PluralRule::Slavic);
        assert_eq!(rule.select(1), Plural::One);
        assert_eq!(rule.select(2), Plural::Few);
        assert_eq!(rule.select(4), Plural::Few);
        assert_eq!(rule.select(5), Plural::Many);

        // The teens are the trap.
        assert_eq!(rule.select(11), Plural::Many, "11 is not 1");
        assert_eq!(rule.select(12), Plural::Many, "12 is not 2");
        assert_eq!(rule.select(14), Plural::Many);

        // And the twenties resume the pattern.
        assert_eq!(rule.select(21), Plural::One);
        assert_eq!(rule.select(22), Plural::Few);
        assert_eq!(rule.select(25), Plural::Many);
        assert_eq!(
            rule.select(112),
            Plural::Many,
            "the last two digits, not the last"
        );
    }

    #[test]
    fn russian_follows_the_same_shape_as_polish() {
        assert_eq!(PluralRule::for_language("ru"), PluralRule::Slavic);
        assert_eq!(PluralRule::for_language("uk"), PluralRule::Slavic);
    }

    /// Arabic has six categories, including zero and an explicit dual.
    #[test]
    fn arabic_has_six_categories() {
        let rule = PluralRule::for_language("ar");
        assert_eq!(rule.select(0), Plural::Zero);
        assert_eq!(rule.select(1), Plural::One);
        assert_eq!(rule.select(2), Plural::Two);
        assert_eq!(rule.select(3), Plural::Few);
        assert_eq!(rule.select(10), Plural::Few);
        assert_eq!(rule.select(11), Plural::Many);
        assert_eq!(rule.select(99), Plural::Many);
        assert_eq!(rule.select(100), Plural::Other);
        assert_eq!(rule.categories().len(), 6);
    }

    /// **Japanese has one form, and must not be made to write two.**
    #[test]
    fn japanese_has_a_single_form() {
        let rule = PluralRule::for_language("ja");
        for n in [0, 1, 2, 5, 100] {
            assert_eq!(rule.select(n), Plural::Other, "n={n}");
        }
        assert_eq!(rule.categories(), &[Plural::Other]);
    }

    /// A region subtag does not change the rule family, and does not confuse it.
    #[test]
    fn a_region_subtag_is_ignored() {
        assert_eq!(PluralRule::for_language("pt-BR"), PluralRule::ZeroOneOther);
        assert_eq!(PluralRule::for_language("en_US"), PluralRule::OneOther);
        assert_eq!(PluralRule::for_language("ZH-Hans"), PluralRule::Other);
    }

    /// **An unknown language falls back to English, and that is a limitation.**
    ///
    /// Asserted rather than left implicit, so adding a language is a visible
    /// change to this test rather than a silent behaviour nobody checks.
    #[test]
    fn an_unknown_language_falls_back_to_the_english_rule() {
        assert_eq!(PluralRule::for_language("xx"), PluralRule::OneOther);
        assert_eq!(PluralRule::for_language(""), PluralRule::OneOther);
    }

    /// **A negative count pluralises by magnitude.**
    ///
    /// -1 takes the same form as 1 in every CLDR rule. Without the magnitude
    /// step a debt, a temperature or a negative score falls through to `Other`
    /// and reads wrong.
    #[test]
    fn a_negative_count_uses_its_magnitude() {
        assert_eq!(PluralRule::OneOther.select(-1), Plural::One);
        assert_eq!(PluralRule::Slavic.select(-22), Plural::Few);
        assert_eq!(PluralRule::Arabic.select(-2), Plural::Two);
    }

    /// `i64::MIN` has no positive counterpart; taking the magnitude must not
    /// overflow.
    #[test]
    fn the_extremes_do_not_overflow() {
        for rule in [
            PluralRule::OneOther,
            PluralRule::ZeroOneOther,
            PluralRule::Slavic,
            PluralRule::Arabic,
            PluralRule::Other,
        ] {
            let _ = rule.select(i64::MIN);
            let _ = rule.select(i64::MAX);
        }
    }

    /// Every category a rule can produce is in its `categories()` list.
    ///
    /// This is what lets a table be checked for completeness up front rather
    /// than discovering a missing form at count 22 in production.
    #[test]
    fn categories_covers_everything_select_can_return() {
        for rule in [
            PluralRule::Other,
            PluralRule::OneOther,
            PluralRule::ZeroOneOther,
            PluralRule::Slavic,
            PluralRule::Arabic,
        ] {
            for n in 0..250i64 {
                let selected = rule.select(n);
                assert!(
                    rule.categories().contains(&selected),
                    "{rule:?} produced {selected:?} at {n}, which is not in its categories"
                );
            }
        }
    }
}
