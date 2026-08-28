//! Wiring `somnium_i18n` into `somnium_ui`'s localisation hook (MORROWIND-AH).
//!
//! MORROWIND-G put a `Resolver` trait in `somnium_ui` and a `Catalog` now lives
//! in `somnium_i18n`, and neither crate depends on the other. This file is the
//! join, and it is the same shape as [`crate::input_actions`]: the trait lives
//! with the consumer, the implementation lives with the data, and the crate
//! that already depends on both owns twelve lines of glue.
//!
//! The alternative — `somnium_ui` depending on `somnium_i18n` — would mean a
//! translation table could not be loaded, checked or diffed by a tool without
//! pulling in a widget tree, and `somnium_ui` could not be tested without one.

use somnium_i18n::{Catalog, Numbers};
use somnium_ui::text::localize::{Argument, Resolver, TextKey};

/// Resolves UI text through a loaded [`Catalog`].
///
/// Borrows rather than owns, so the catalog can be swapped at runtime — which
/// is the whole point of runtime locale switching — without the UI holding a
/// stale copy.
#[derive(Debug)]
pub struct CatalogResolver<'a>(pub &'a Catalog);

/// Decimal places for a real-numbered argument.
///
/// One. A UI number with more is unreadable at a glance and one that shows a
/// float's full precision (`33.333333333333336` health) is a bug report.
const DECIMAL_PLACES: usize = 1;

impl Resolver for CatalogResolver<'_> {
    fn resolve(&self, key: TextKey, args: &[(&'static str, Argument)]) -> Option<String> {
        let numbers = Numbers::for_language(self.0.locale());

        // A `Count` argument selects the plural form. The *first* one, because
        // a string with two counts ("3 potions and 2 scrolls") cannot select on
        // both — that string needs two keys, and picking the first at least
        // makes which one predictable rather than dependent on argument order
        // inside the resolver.
        let template = match args.iter().find_map(|(_, value)| match value {
            Argument::Count(n) => Some(*n),
            _ => None,
        }) {
            Some(count) => self.0.lookup_plural(key.as_str(), count)?,
            None => self.0.lookup(key.as_str())?,
        };

        let rendered: Vec<(&str, String)> = args
            .iter()
            .map(|(name, value)| {
                let text = match value {
                    Argument::Count(n) => numbers.integer(*n),
                    Argument::Number(n) => numbers.decimal(*n, DECIMAL_PLACES),
                    Argument::Text(t) => t.clone(),
                };
                (*name, text)
            })
            .collect();
        Some(Catalog::substitute(template, &rendered))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_i18n::Table;

    fn catalog() -> Catalog {
        let mut catalog = Catalog::new("en");
        catalog.insert(
            Table::new("en")
                .with("menu.quit", "Quit")
                .with("inv.potions.one", "{n} potion")
                .with("inv.potions.other", "{n} potions")
                .with("hud.gold", "{amount} gold"),
        );
        catalog.insert(
            Table::new("de")
                .with("menu.quit", "Beenden")
                .with("hud.gold", "{amount} Gold"),
        );
        catalog.insert(
            Table::new("pl")
                .with("inv.potions.one", "{n} mikstura")
                .with("inv.potions.few", "{n} mikstury")
                .with("inv.potions.many", "{n} mikstur"),
        );
        catalog
    }

    #[test]
    fn a_key_resolves_through_the_catalog() {
        let catalog = catalog();
        let resolver = CatalogResolver(&catalog);
        assert_eq!(
            resolver.resolve(TextKey("menu.quit"), &[]),
            Some("Quit".into())
        );
    }

    /// **A count selects the plural form**, which is the entire reason
    /// MORROWIND-G refused to let the call site pre-format its string.
    #[test]
    fn a_count_selects_the_form_and_is_substituted() {
        let catalog = catalog();
        let resolver = CatalogResolver(&catalog);
        assert_eq!(
            resolver.resolve(TextKey("inv.potions"), &[("n", Argument::Count(1))]),
            Some("1 potion".into())
        );
        assert_eq!(
            resolver.resolve(TextKey("inv.potions"), &[("n", Argument::Count(5))]),
            Some("5 potions".into())
        );
    }

    /// The three-form case the hook was designed for, end to end.
    #[test]
    fn polish_gets_three_forms_through_the_ui_hook() {
        let mut catalog = catalog();
        catalog.set_locale("pl");
        let resolver = CatalogResolver(&catalog);
        let of = |n| resolver.resolve(TextKey("inv.potions"), &[("n", Argument::Count(n))]);
        assert_eq!(of(1), Some("1 mikstura".into()));
        assert_eq!(of(22), Some("22 mikstury".into()));
        assert_eq!(of(25), Some("25 mikstur".into()));
    }

    /// **Numbers are written the locale's way**, not the source language's.
    #[test]
    fn a_number_is_formatted_for_the_locale() {
        let mut catalog = catalog();
        let resolver = CatalogResolver(&catalog);
        assert_eq!(
            resolver.resolve(TextKey("hud.gold"), &[("amount", Argument::Count(1234))]),
            Some("1,234 gold".into())
        );
        drop(resolver);
        catalog.set_locale("de");
        assert_eq!(
            CatalogResolver(&catalog)
                .resolve(TextKey("hud.gold"), &[("amount", Argument::Count(1234))]),
            Some("1.234 Gold".into())
        );
    }

    /// **Switching locale switches the UI**, because the resolver borrows.
    #[test]
    fn a_locale_switch_reaches_the_ui() {
        let mut catalog = catalog();
        assert_eq!(
            CatalogResolver(&catalog).resolve(TextKey("menu.quit"), &[]),
            Some("Quit".into())
        );
        catalog.set_locale("de");
        assert_eq!(
            CatalogResolver(&catalog).resolve(TextKey("menu.quit"), &[]),
            Some("Beenden".into())
        );
    }

    /// An unknown key resolves to `None`, and MORROWIND-G's rule takes over:
    /// the UI shows the key, never a blank that reads as a layout bug.
    #[test]
    fn an_unknown_key_is_none_so_the_ui_shows_the_key() {
        let catalog = catalog();
        assert_eq!(
            CatalogResolver(&catalog).resolve(TextKey("nothing.here"), &[]),
            None
        );
    }

    /// A partially translated locale falls back rather than blanking.
    #[test]
    fn a_missing_translation_falls_back_to_the_default_language() {
        let mut catalog = catalog();
        catalog.set_locale("pl");
        assert_eq!(
            CatalogResolver(&catalog).resolve(TextKey("menu.quit"), &[]),
            Some("Quit".into()),
            "Polish has no menu.quit, so English shows"
        );
    }
}
