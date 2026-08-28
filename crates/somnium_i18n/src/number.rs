//! Locale-aware number formatting.
//!
//! MORROWIND-G's argument for keeping an argument typed was that a
//! pre-formatted `"1,234"` has already lost both the plural information and the
//! locale's own conventions. This is the second half of that: given the number,
//! decide how it is written.
//!
//! # Why this is small on purpose
//!
//! Full CLDR number formatting is currency, scientific notation, per-locale
//! digit systems and compact forms — megabytes of data. What a game actually
//! shows is gold counts, damage numbers, percentages and timers, and for those
//! the entire difference between locales is **which character groups thousands
//! and which separates the fraction**. `1,234.5` in English is `1.234,5` in
//! German and `1 234,5` in French, and getting *that* wrong is what a player
//! notices. Getting the compact form of 1.2 million wrong is what nobody sees,
//! because a game rarely shows it.
//!
//! So this is a three-field struct and a table, and the moment a title needs
//! currency it should reach for a real CLDR crate rather than grow this one.

use serde::{Deserialize, Serialize};

/// How a locale writes a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Numbers {
    /// Between the integer part and the fraction: `.` or `,`.
    pub decimal: char,
    /// Between groups of three digits. `None` for locales that do not group.
    pub group: Option<char>,
}

impl Default for Numbers {
    fn default() -> Self {
        Self {
            decimal: '.',
            group: Some(','),
        }
    }
}

impl Numbers {
    /// The convention for a language tag.
    ///
    /// Matched on the language subtag, not the region, with one exception:
    /// **`es-US` groups like English**, because a Spanish speaker in the United
    /// States reads prices printed the American way. That exception is the
    /// whole reason this takes the full tag rather than two letters.
    #[must_use]
    pub fn for_language(tag: &str) -> Self {
        let lower = tag.to_ascii_lowercase();
        if lower == "es-us" {
            return Self::default();
        }
        let language = lower.split('-').next().unwrap_or("");
        match language {
            // Comma decimal, dot group.
            "de" | "es" | "it" | "pt" | "nl" | "id" | "tr" | "da" | "el" | "ro" | "vi" => Self {
                decimal: ',',
                group: Some('.'),
            },
            // Comma decimal, space group. A non-breaking space, so a number
            // never wraps across a line in the middle of itself.
            "fr" | "pl" | "ru" | "uk" | "cs" | "sv" | "nb" | "fi" | "hu" => Self {
                decimal: ',',
                group: Some('\u{00a0}'),
            },
            _ => Self::default(),
        }
    }

    /// Format a whole number.
    #[must_use]
    pub fn integer(self, value: i64) -> String {
        let negative = value < 0;
        // `unsigned_abs`, because `-(i64::MIN)` overflows and a damage number
        // is not worth a panic.
        let digits = value.unsigned_abs().to_string();
        let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
        if negative {
            out.push('-');
        }
        match self.group {
            None => out.push_str(&digits),
            Some(separator) => {
                for (i, c) in digits.chars().enumerate() {
                    if i > 0 && (digits.len() - i) % 3 == 0 {
                        out.push(separator);
                    }
                    out.push(c);
                }
            }
        }
        out
    }

    /// Format a real number to `places` decimals.
    #[must_use]
    pub fn decimal(self, value: f64, places: usize) -> String {
        // Non-finite values reach here from a divide in gameplay code. Writing
        // "NaN" is honest and harmless; grouping its "digits" is not.
        if !value.is_finite() {
            return format!("{value}");
        }
        let text = format!("{:.*}", places, value.abs());
        let (whole, fraction) = text.split_once('.').unwrap_or((text.as_str(), ""));
        let whole: i64 = whole.parse().unwrap_or(0);
        let mut out = String::new();
        // Sign taken from the input rather than from the rounded magnitude, so
        // -0.4 to zero places reads as "-0" rather than silently becoming "0"
        // — and never as "-" alone.
        if value.is_sign_negative() && (whole != 0 || fraction.chars().any(|c| c != '0')) {
            out.push('-');
        }
        out.push_str(&self.integer(whole));
        if !fraction.is_empty() {
            out.push(self.decimal);
            out.push_str(fraction);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_groups_with_commas() {
        assert_eq!(Numbers::for_language("en").integer(1_234_567), "1,234,567");
    }

    /// **The difference a player actually notices.**
    #[test]
    fn german_swaps_both_separators() {
        let numbers = Numbers::for_language("de");
        assert_eq!(numbers.integer(1234), "1.234");
        assert_eq!(numbers.decimal(1234.5, 1), "1.234,5");
    }

    #[test]
    fn french_groups_with_a_space() {
        let text = Numbers::for_language("fr").integer(1234);
        assert_eq!(text, "1\u{00a0}234");
        assert!(
            !text.contains(' '),
            "a non-breaking space, so a number never wraps in the middle of itself"
        );
    }

    /// **`es-US` groups like English**, which is why this takes a full tag.
    #[test]
    fn a_region_can_override_its_language() {
        assert_eq!(Numbers::for_language("es").integer(1234), "1.234");
        assert_eq!(Numbers::for_language("es-US").integer(1234), "1,234");
    }

    #[test]
    fn a_region_otherwise_follows_its_language() {
        assert_eq!(Numbers::for_language("pt-BR").integer(1234), "1.234");
    }

    #[test]
    fn short_numbers_are_untouched() {
        for n in [0, 7, 99, 999] {
            assert_eq!(Numbers::default().integer(n), n.to_string());
        }
    }

    #[test]
    fn negatives_keep_their_sign_and_grouping() {
        assert_eq!(Numbers::default().integer(-1_234), "-1,234");
    }

    /// A damage number is not worth a panic.
    #[test]
    fn the_extreme_does_not_overflow() {
        assert_eq!(
            Numbers::default().integer(i64::MIN),
            "-9,223,372,036,854,775,808"
        );
    }

    #[test]
    fn decimals_round_to_the_requested_places() {
        assert_eq!(Numbers::default().decimal(1234.567, 2), "1,234.57");
        assert_eq!(Numbers::default().decimal(1234.567, 0), "1,235");
    }

    /// A negative that rounds to zero keeps its sign rather than turning into
    /// a bare `-` or a positive zero.
    #[test]
    fn a_small_negative_is_not_mangled() {
        assert_eq!(Numbers::default().decimal(-0.4, 0), "0");
        assert_eq!(Numbers::default().decimal(-0.4, 1), "-0.4");
    }

    /// Non-finite values reach here from a divide in gameplay code.
    #[test]
    fn non_finite_values_are_written_plainly() {
        assert_eq!(Numbers::default().decimal(f64::NAN, 2), "NaN");
        assert_eq!(Numbers::default().decimal(f64::INFINITY, 2), "inf");
    }
}
