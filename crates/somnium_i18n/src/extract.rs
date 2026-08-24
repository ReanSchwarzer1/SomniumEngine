//! Finding every user-visible string (MORROWIND-AH, the extraction tool).
//!
//! §8 names Ren'Py's `renpy/translation/` as the reference, and the reason it
//! is the reference is not its file format. It is that Ren'Py **extracts from
//! the game rather than from a list a human maintains**. A hand-maintained list
//! of translatable strings is wrong the day after it is written: someone adds a
//! label, ships it, and the string is simply never translated — in every
//! language, silently, until a player reports it.
//!
//! So this module does two things a hand-maintained list cannot:
//!
//! 1. **Collects the keys the code actually asks for**, so a key that no table
//!    defines is a build-time finding rather than a `menu.quit` on screen.
//! 2. **Finds string literals that were never turned into keys at all** — the
//!    `Label::new("Quit")` that bypassed the whole system. This is the failure
//!    the checklist calls "hardcoded strings", and it is invisible to every
//!    check that starts from the translation files.
//!
//! # Why a scanner and not a macro
//!
//! A `tr!` macro could register its keys at compile time and be exact. It could
//! not find (2) — a hardcoded string is precisely the case where the macro was
//! not used. Catching the strings that escaped the system is the larger half of
//! the problem, and it needs a scanner that reads source as text.
//!
//! The scanner is therefore **deliberately heuristic and deliberately
//! advisory**. It is string- and comment-aware so it does not read a key out of
//! a comment, but it does not parse Rust. Its output is a list to look at, not
//! a gate that fails a build on a false positive.

use crate::{Catalog, Plural};
use std::collections::{BTreeMap, BTreeSet};

/// Calls whose first string argument is a translation key.
const KEY_CALLS: &[&str] = &["tr", "tr!", "t", "t!", "resolve", "localize", "text_key"];

/// Calls whose first string argument is a *plural* key — the table needs one
/// entry per category, not one entry.
const PLURAL_CALLS: &[&str] = &[
    "tr_plural",
    "tr_plural!",
    "resolve_plural",
    "localize_plural",
];

/// Calls whose first string argument lands on screen as-is.
///
/// This list is where the false positives live and it is meant to be short: a
/// scanner that flags everything gets ignored, and an ignored scanner is worth
/// less than no scanner.
const TEXT_CALLS: &[&str] = &[
    "Label::new",
    "Text::new",
    "Button::new",
    "set_text",
    "with_text",
    "tooltip",
    "placeholder",
    "title",
];

/// A string literal the scanner decided a player would read.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Literal {
    /// Source file it came from.
    pub file: String,
    /// 1-based line.
    pub line: usize,
    /// The call it was passed to, e.g. `Label::new`.
    pub call: String,
    /// The literal text.
    pub text: String,
}

/// What a scan found.
#[derive(Clone, Debug, Default)]
pub struct Extraction {
    /// Plain keys the code asks for.
    pub keys: BTreeSet<String>,
    /// Plural keys — each needs one table entry per category.
    pub plural_keys: BTreeSet<String>,
    /// Strings that reach a player without going through a key at all.
    pub hardcoded: Vec<Literal>,
    /// Where each key is used, for a report that can point at a file.
    pub sites: BTreeMap<String, Vec<(String, usize)>>,
}

impl Extraction {
    /// Scan one file's source.
    pub fn scan(&mut self, file: &str, source: &str) {
        for (line, call, text) in literals(source) {
            // An explicit opt-out, because every scanner needs one: the
            // alternative is that a team with three unavoidable false positives
            // stops reading the output entirely.
            if line_of(source, line).contains("i18n-ignore") {
                continue;
            }
            if PLURAL_CALLS.contains(&call.as_str()) {
                self.plural_keys.insert(text.clone());
                self.sites
                    .entry(text)
                    .or_default()
                    .push((file.to_string(), line));
            } else if KEY_CALLS.contains(&call.as_str()) {
                self.keys.insert(text.clone());
                self.sites
                    .entry(text)
                    .or_default()
                    .push((file.to_string(), line));
            } else if TEXT_CALLS.contains(&call.as_str()) && looks_like_prose(&text) {
                self.hardcoded.push(Literal {
                    file: file.to_string(),
                    line,
                    call,
                    text,
                });
            }
        }
    }

    /// Scan several files.
    #[must_use]
    pub fn from_sources<'a>(sources: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut extraction = Self::default();
        for (file, source) in sources {
            extraction.scan(file, source);
        }
        extraction
    }

    /// Every key, plural or not.
    #[must_use]
    pub fn all_keys(&self) -> BTreeSet<&str> {
        self.keys
            .iter()
            .chain(self.plural_keys.iter())
            .map(String::as_str)
            .collect()
    }
}

/// Something worth a human's attention. Never an error — see the module note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtractionIssue {
    /// The code asks for a key the default table does not define. This one
    /// reaches players in *every* language, so it is listed first.
    Missing {
        /// The key.
        key: String,
    },
    /// A plural key whose table is missing one of the categories the language
    /// needs — the case that produces "1 potions" or, in Polish, nonsense.
    MissingCategory {
        /// The locale whose table is short.
        locale: String,
        /// The plural key.
        key: String,
        /// The category with no entry.
        category: Plural,
    },
    /// A key the default has and a translation does not. Expected mid-project;
    /// this is the progress bar, not an error.
    Untranslated {
        /// The locale.
        locale: String,
        /// The key.
        key: String,
    },
    /// A key in a table that no code asks for. Advisory: only as good as the
    /// scan's coverage, and a key built at runtime will look unused.
    Unused {
        /// The key.
        key: String,
    },
    /// A string that reaches a player without a key.
    Hardcoded(Literal),
}

/// Compare a scan against a catalog.
///
/// Ordered by how badly each issue reads on screen: a missing default breaks
/// every language, a missing plural category breaks one, an untranslated string
/// merely shows the wrong language, and an unused key breaks nothing.
#[must_use]
pub fn check_tables(extraction: &Extraction, catalog: &Catalog) -> Vec<ExtractionIssue> {
    let mut issues = Vec::new();
    let default = catalog.default_locale().to_string();

    for key in &extraction.keys {
        let present = catalog
            .table(&default)
            .is_some_and(|t| t.get(key).is_some());
        if !present {
            issues.push(ExtractionIssue::Missing { key: key.clone() });
        }
    }
    for key in &extraction.plural_keys {
        let has_any = catalog
            .table(&default)
            .is_some_and(|table| started(table, key));
        if !has_any {
            issues.push(ExtractionIssue::Missing { key: key.clone() });
        }
    }

    for locale in catalog.locales() {
        let Some(table) = catalog.table(locale) else {
            continue;
        };
        for key in &extraction.plural_keys {
            // Only complain about a locale that has *started* this key. A
            // locale that has none of it is untranslated, not miscategorised,
            // and reporting both for the same key is noise.
            if !started(table, key) {
                if locale != default {
                    issues.push(ExtractionIssue::Untranslated {
                        locale: locale.to_string(),
                        key: key.clone(),
                    });
                }
                continue;
            }
            for category in table.plural_rule().categories() {
                if table.get(&format!("{key}.{}", category.suffix())).is_none() {
                    issues.push(ExtractionIssue::MissingCategory {
                        locale: locale.to_string(),
                        key: key.clone(),
                        category: *category,
                    });
                }
            }
        }
        if locale == default {
            continue;
        }
        for key in &extraction.keys {
            if table.get(key).is_none() {
                issues.push(ExtractionIssue::Untranslated {
                    locale: locale.to_string(),
                    key: key.clone(),
                });
            }
        }
    }

    if let Some(table) = catalog.table(&default) {
        let used = extraction.all_keys();
        for key in table.strings.keys() {
            let base = key.rsplit_once('.').map_or(key.as_str(), |(base, suffix)| {
                if Plural::from_suffix(suffix).is_some() {
                    base
                } else {
                    key.as_str()
                }
            });
            if !used.contains(base) && !used.contains(key.as_str()) {
                issues.push(ExtractionIssue::Unused { key: key.clone() });
            }
        }
    }

    issues.sort_by_key(severity);
    issues.extend(
        extraction
            .hardcoded
            .iter()
            .cloned()
            .map(ExtractionIssue::Hardcoded),
    );
    issues
}

/// Whether a table has any category of a plural key at all.
fn started(table: &crate::Table, key: &str) -> bool {
    table
        .plural_rule()
        .categories()
        .iter()
        .any(|c| table.get(&format!("{key}.{}", c.suffix())).is_some())
}

fn severity(issue: &ExtractionIssue) -> u8 {
    match issue {
        ExtractionIssue::Missing { .. } => 0,
        ExtractionIssue::MissingCategory { .. } => 1,
        ExtractionIssue::Untranslated { .. } => 2,
        ExtractionIssue::Unused { .. } => 3,
        ExtractionIssue::Hardcoded(_) => 4,
    }
}

/// `(line, call, text)` for every string literal, skipping comments.
///
/// Comment-awareness is the one piece of real parsing here, and it earns its
/// place: doc comments in this codebase are full of example calls, and a
/// scanner that reads keys out of documentation reports keys that do not exist.
fn literals(source: &str) -> Vec<(usize, String, String)> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1;
    let mut block_depth = 0usize;

    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if block_depth > 0 {
            if bytes[i..].starts_with(b"/*") {
                block_depth += 1;
                i += 2;
            } else if bytes[i..].starts_with(b"*/") {
                block_depth -= 1;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if bytes[i..].starts_with(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i..].starts_with(b"/*") {
            block_depth = 1;
            i += 2;
            continue;
        }
        if c == b'"' {
            let start = i + 1;
            let mut j = start;
            let mut text = String::new();
            while j < bytes.len() {
                match bytes[j] {
                    b'\\' if j + 1 < bytes.len() => {
                        // Keep the escape as written; the scanner reports
                        // source text, and unescaping it would misreport what a
                        // translator has to copy.
                        text.push('\\');
                        text.push(bytes[j + 1] as char);
                        j += 2;
                    }
                    b'"' => break,
                    b'\n' => {
                        line += 1;
                        text.push('\n');
                        j += 1;
                    }
                    other => {
                        text.push(other as char);
                        j += 1;
                    }
                }
            }
            if let Some(call) = call_before(source, i) {
                out.push((line, call, text));
            }
            i = j + 1;
            continue;
        }
        i += 1;
    }
    out
}

/// The `path::name` of the call whose argument list opens just before `at`.
fn call_before(source: &str, at: usize) -> Option<String> {
    let bytes = source.as_bytes();
    let mut i = at;
    while i > 0 && bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    if i == 0 || bytes[i - 1] != b'(' {
        return None;
    }
    i -= 1;
    // A macro is `name!(`; keep the bang so `tr!` and `tr` are one lookup.
    let mut bang = false;
    if i > 0 && bytes[i - 1] == b'!' {
        bang = true;
        i -= 1;
    }
    let end = i;
    while i > 0
        && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_' || bytes[i - 1] == b':')
    {
        i -= 1;
    }
    if i == end {
        return None;
    }
    let mut call = source[i..end].to_string();
    if bang {
        call.push('!');
    }
    Some(call)
}

fn line_of(source: &str, line: usize) -> &str {
    source.lines().nth(line.saturating_sub(1)).unwrap_or("")
}

/// Whether a literal reads like something a player would see.
///
/// The rejections are the interesting part: identifiers, paths and format
/// fragments all reach these call sites legitimately, and flagging them is how
/// a scanner trains people to ignore it.
fn looks_like_prose(text: &str) -> bool {
    if text.trim().is_empty() || !text.chars().any(char::is_alphabetic) {
        return false;
    }
    if text.contains('/') || text.contains('\\') {
        return false; // a path
    }
    // A key handed straight through: lowercase, dotted or underscored, no
    // spaces. `menu.quit` at a text call is a wiring question, not a
    // translation one.
    let keyish = !text.contains(' ')
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_');
    !keyish
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Table;

    #[test]
    fn keys_are_collected_from_calls() {
        let mut extraction = Extraction::default();
        extraction.scan(
            "hud.rs",
            r#"let label = tr("menu.quit"); let n = tr_plural("inv.potions", count);"#,
        );
        assert!(extraction.keys.contains("menu.quit"));
        assert!(extraction.plural_keys.contains("inv.potions"));
    }

    /// **A key in a comment is not a key.**
    ///
    /// This codebase documents its APIs with example calls; a scanner that
    /// reads them reports keys that do not exist and never will.
    #[test]
    fn comments_are_not_scanned() {
        let source = concat!(
            "// tr(\"commented.out\")\n",
            "/// Example: tr(\"doc.example\")\n",
            "/* tr(\"block.comment\") */\n",
            "tr(\"real.key\");\n",
        );
        let mut extraction = Extraction::default();
        extraction.scan("a.rs", source);
        assert_eq!(extraction.all_keys(), ["real.key"].into_iter().collect());
    }

    #[test]
    fn a_line_number_points_at_the_call() {
        let mut extraction = Extraction::default();
        extraction.scan("a.rs", "one\ntwo\ntr(\"third.line\");\n");
        assert_eq!(
            extraction.sites["third.line"],
            vec![("a.rs".to_string(), 3)]
        );
    }

    /// **The larger half: a string that never became a key.**
    #[test]
    fn a_hardcoded_label_is_found() {
        let mut extraction = Extraction::default();
        extraction.scan("hud.rs", r#"Label::new("Quit to desktop")"#);
        assert_eq!(extraction.hardcoded.len(), 1);
        assert_eq!(extraction.hardcoded[0].text, "Quit to desktop");
        assert_eq!(extraction.hardcoded[0].call, "Label::new");
    }

    /// Paths, identifiers and keys passed straight through are not prose.
    ///
    /// A scanner that flags them gets ignored, and an ignored scanner is worth
    /// less than no scanner.
    #[test]
    fn non_prose_literals_are_not_flagged() {
        let mut extraction = Extraction::default();
        extraction.scan(
            "a.rs",
            concat!(
                "Label::new(\"assets/ui/font.ttf\");\n",
                "Label::new(\"menu.quit\");\n",
                "Label::new(\"\");\n",
                "Label::new(\"   \");\n",
                "Label::new(\"42\");\n",
            ),
        );
        assert!(
            extraction.hardcoded.is_empty(),
            "{:?}",
            extraction.hardcoded
        );
    }

    /// **There is an opt-out**, because a team with three unavoidable false
    /// positives stops reading the output entirely.
    #[test]
    fn an_ignore_comment_suppresses_a_finding() {
        let mut extraction = Extraction::default();
        extraction.scan("a.rs", "Label::new(\"Debug overlay\"); // i18n-ignore\n");
        assert!(extraction.hardcoded.is_empty());
    }

    #[test]
    fn a_bare_string_with_no_call_is_ignored() {
        let mut extraction = Extraction::default();
        extraction.scan("a.rs", "let s = \"loose string\";\n");
        assert!(extraction.hardcoded.is_empty());
        assert!(extraction.all_keys().is_empty());
    }

    fn catalog() -> Catalog {
        let mut catalog = Catalog::new("en");
        catalog.insert(
            Table::new("en")
                .with("menu.quit", "Quit")
                .with("inv.potions.one", "{n} potion")
                .with("inv.potions.other", "{n} potions")
                .with("menu.orphan", "Nobody asks for this"),
        );
        catalog.insert(Table::new("pl").with("inv.potions.one", "{n} mikstura"));
        catalog
    }

    fn extraction() -> Extraction {
        Extraction::from_sources([(
            "hud.rs",
            "tr(\"menu.quit\"); tr(\"menu.absent\"); tr_plural(\"inv.potions\", n);",
        )])
    }

    /// **A key no table defines reaches players in every language**, so it
    /// sorts first.
    #[test]
    fn a_key_missing_from_the_default_is_the_worst_issue() {
        let issues = check_tables(&extraction(), &catalog());
        assert_eq!(
            issues.first(),
            Some(&ExtractionIssue::Missing {
                key: "menu.absent".into()
            })
        );
    }

    /// **A half-translated plural is the "1 potions" bug.**
    ///
    /// Polish has started `inv.potions` but has only `one`; that is a different
    /// finding from not having started it, and reporting it as merely
    /// untranslated would hide it.
    #[test]
    fn a_plural_missing_a_category_is_reported_per_locale() {
        let issues = check_tables(&extraction(), &catalog());
        assert!(issues.contains(&ExtractionIssue::MissingCategory {
            locale: "pl".into(),
            key: "inv.potions".into(),
            category: Plural::Few,
        }));
        assert!(issues.contains(&ExtractionIssue::MissingCategory {
            locale: "pl".into(),
            key: "inv.potions".into(),
            category: Plural::Many,
        }));
    }

    /// A locale that has not started a key is untranslated, not miscategorised
    /// — reporting both for one key is noise.
    #[test]
    fn an_unstarted_key_is_untranslated_and_not_also_miscategorised() {
        let issues = check_tables(&extraction(), &catalog());
        assert!(issues.contains(&ExtractionIssue::Untranslated {
            locale: "pl".into(),
            key: "menu.quit".into()
        }));
        assert!(
            !issues.iter().any(|i| matches!(
                i,
                ExtractionIssue::MissingCategory { key, .. } if key == "menu.quit"
            )),
            "a plain key never produces a category finding"
        );
    }

    /// A key nothing asks for is advisory and sorts last of the table issues.
    #[test]
    fn an_unused_key_is_reported() {
        let issues = check_tables(&extraction(), &catalog());
        assert!(issues.contains(&ExtractionIssue::Unused {
            key: "menu.orphan".into()
        }));
    }

    /// **A plural key's category entries are not each "unused".**
    ///
    /// `inv.potions.one` is reached through `inv.potions`; treating the two as
    /// unrelated names would report every translated plural as dead weight.
    #[test]
    fn plural_category_entries_count_as_used() {
        let issues = check_tables(&extraction(), &catalog());
        assert!(
            !issues.iter().any(|i| matches!(
                i,
                ExtractionIssue::Unused { key } if key.starts_with("inv.potions")
            )),
            "{issues:?}"
        );
    }

    #[test]
    fn a_clean_catalog_produces_nothing() {
        let mut catalog = Catalog::new("en");
        catalog.insert(Table::new("en").with("menu.quit", "Quit"));
        let extraction = Extraction::from_sources([("a.rs", "tr(\"menu.quit\");")]);
        assert!(check_tables(&extraction, &catalog).is_empty());
    }
}
