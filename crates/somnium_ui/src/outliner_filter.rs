//! The Outliner's typed filter grammar.
//!
//! `type:light`, `script:`, `hidden:`, `locked:` and bare words, combined with
//! AND. Kept pure and separate from the panel so the ranking rule §5.1 asks for
//! — prefix beats substring, shorter names win ties — is testable without a
//! window, and so a malformed query degrades to a name search instead of
//! matching nothing.

use crate::editor_event::OutlinerRow;

/// One parsed query.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OutlinerFilter {
    /// Bare words, lowercased. Every one must appear in the name.
    terms: Vec<String>,
    /// `type:` clauses. A row matches if it carries **any** of these tags,
    /// because `type:light type:mesh` reads as "lights or meshes" to everyone
    /// who has ever used a search box.
    types: Vec<String>,
    /// `script:` — with a value it is a name test on the row's tags, bare it
    /// simply means "has a script".
    scripts: bool,
    hidden: Option<bool>,
    locked: Option<bool>,
    errors: bool,
}

impl OutlinerFilter {
    /// Parse a query. Never fails: an unrecognised `key:value` is treated as a
    /// bare term, so a typo narrows the list instead of emptying it.
    #[must_use]
    pub fn parse(query: &str) -> Self {
        let mut filter = Self::default();
        for token in query.split_whitespace() {
            let lower = token.to_ascii_lowercase();
            match lower.split_once(':') {
                Some(("type", value)) if !value.is_empty() => filter.types.push(value.to_owned()),
                Some(("script", "")) => filter.scripts = true,
                Some(("script", value)) => {
                    filter.scripts = true;
                    filter.terms.push(value.to_owned());
                }
                Some(("hidden", value)) => filter.hidden = Some(parse_bool(value)),
                Some(("locked", value)) => filter.locked = Some(parse_bool(value)),
                Some(("error", _) | ("errors", _)) => filter.errors = true,
                _ => filter.terms.push(lower),
            }
        }
        filter
    }

    /// Whether the query would show every row.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Whether `row` survives the filter.
    #[must_use]
    pub fn matches(&self, row: &OutlinerRow) -> bool {
        if self.is_empty() {
            return true;
        }
        let name = row.name.to_ascii_lowercase();
        if !self.terms.iter().all(|term| {
            name.contains(term) || row.tags.iter().any(|tag| tag.contains(term.as_str()))
        }) {
            return false;
        }
        if !self.types.is_empty()
            && !self
                .types
                .iter()
                .any(|wanted| row.tags.iter().any(|tag| tag == wanted))
        {
            return false;
        }
        if self.scripts && !row.tags.contains(&"script") {
            return false;
        }
        if self.errors && !row.script_error {
            return false;
        }
        if self.hidden.is_some_and(|wanted| row.hidden != wanted) {
            return false;
        }
        if self.locked.is_some_and(|wanted| row.locked != wanted) {
            return false;
        }
        true
    }

    /// §5.1's ranking: a prefix match beats a substring match, and a shorter
    /// name breaks the tie. Lower is better, so this sorts ascending.
    #[must_use]
    pub fn rank(&self, row: &OutlinerRow) -> (u8, usize) {
        let name = row.name.to_ascii_lowercase();
        let best = self
            .terms
            .iter()
            .map(|term| {
                if name.starts_with(term.as_str()) {
                    0
                } else if name.contains(term.as_str()) {
                    1
                } else {
                    2
                }
            })
            .min()
            .unwrap_or(0);
        (best, name.len())
    }
}

fn parse_bool(value: &str) -> bool {
    !matches!(value, "false" | "no" | "0" | "off")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, tags: &[&'static str]) -> OutlinerRow {
        OutlinerRow {
            id: 0,
            name: name.to_owned(),
            depth: 0,
            has_children: false,
            hidden: false,
            locked: false,
            script_error: false,
            tags: tags.to_vec(),
        }
    }

    #[test]
    fn an_empty_query_shows_everything() {
        let filter = OutlinerFilter::parse("   ");
        assert!(filter.is_empty());
        assert!(filter.matches(&row("anything", &[])));
    }

    #[test]
    fn type_matches_a_component_tag_not_the_name() {
        let filter = OutlinerFilter::parse("type:light");
        assert!(filter.matches(&row("Streetlamp", &["light", "mesh"])));
        assert!(
            !filter.matches(&row("Light Post", &["mesh"])),
            "a name containing the word is not a type"
        );
    }

    /// Two `type:` clauses read as "or", which is what a search box means by
    /// listing two things.
    #[test]
    fn several_types_are_a_union() {
        let filter = OutlinerFilter::parse("type:light type:water");
        assert!(filter.matches(&row("a", &["water"])));
        assert!(filter.matches(&row("b", &["light"])));
        assert!(!filter.matches(&row("c", &["mesh"])));
    }

    #[test]
    fn bare_script_means_has_a_script() {
        let filter = OutlinerFilter::parse("script:");
        assert!(filter.matches(&row("Door", &["script"])));
        assert!(!filter.matches(&row("Wall", &["mesh"])));
    }

    #[test]
    fn terms_and_clauses_combine_with_and() {
        let filter = OutlinerFilter::parse("type:mesh lamp");
        assert!(filter.matches(&row("Lamp Post", &["mesh"])));
        assert!(!filter.matches(&row("Bench", &["mesh"])));
        assert!(!filter.matches(&row("Lamp Post", &["light"])));
    }

    #[test]
    fn hidden_and_locked_filter_on_the_badges() {
        let mut hidden = row("Ghost", &[]);
        hidden.hidden = true;
        assert!(OutlinerFilter::parse("hidden:").matches(&hidden));
        assert!(!OutlinerFilter::parse("hidden:").matches(&row("Solid", &[])));
        assert!(OutlinerFilter::parse("hidden:false").matches(&row("Solid", &[])));
    }

    /// A typo must narrow, never blank the panel.
    #[test]
    fn an_unknown_clause_degrades_to_a_name_search() {
        let filter = OutlinerFilter::parse("colour:red");
        assert!(filter.matches(&row("colour:red lamp", &[])));
        assert!(!filter.matches(&row("lamp", &[])));
    }

    #[test]
    fn prefix_matches_outrank_substring_matches() {
        let filter = OutlinerFilter::parse("lamp");
        let prefix = filter.rank(&row("Lamp Post", &[]));
        let substring = filter.rank(&row("Street Lamp", &[]));
        assert!(prefix < substring);

        // Shorter breaks the tie between two prefix matches.
        assert!(filter.rank(&row("Lamp", &[])) < filter.rank(&row("Lamp Post", &[])));
    }
}
