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

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// The localisation table as a data table (MORROWIND-M, item 2)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// > *"Its first customer is the localisation table, its second is any game's
// > item or dialogue data."*
//
// The join is here for the same reason `CatalogResolver` is: the data-table
// model lives in `somnium_ui`, the catalog in `somnium_i18n`, and neither knows
// about the other. A translator's view is *keys down, locales across*, which is
// exactly a table of text columns — so this is a projection and not a second
// representation.

/// The column a localisation table keys on.
pub const KEY_COLUMN: &str = "key";

/// Project a catalog into an editable table: one row per key, one column per
/// locale.
///
/// Keys come from the union of every locale, not from the default one. A key
/// that exists only in a translation is usually a mistake — and it is a mistake
/// nobody can see in a table that only lists what the default locale has.
#[must_use]
pub fn catalog_to_table(catalog: &Catalog) -> somnium_ui::data_table::DataTable {
    use somnium_ui::data_table::{Cell, Column, DataTable};

    let mut locales: Vec<String> = catalog.locales().into_iter().map(str::to_owned).collect();
    locales.sort();

    let mut columns = vec![Column::text(KEY_COLUMN, "Key")];
    columns.extend(
        locales
            .iter()
            .map(|locale| Column::text(locale.clone(), locale.clone())),
    );
    let mut table = DataTable::new(columns);

    let mut keys: Vec<&str> = Vec::new();
    for locale in &locales {
        if let Some(entries) = catalog.table(locale) {
            for key in entries.keys() {
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
        }
    }
    keys.sort_unstable();

    for key in keys {
        let mut cells = vec![(KEY_COLUMN.to_owned(), Cell::Text(key.to_owned()))];
        for locale in &locales {
            // `Empty` and not `Text("")`: an untranslated string is not a
            // string translated to nothing, and the `only_incomplete` filter is
            // the whole reason a translator opens this table.
            let cell = catalog
                .table(locale)
                .and_then(|entries| entries.get(key))
                .map_or(Cell::Empty, |text| Cell::Text(text.to_owned()));
            cells.push((locale.clone(), cell));
        }
        table.push_row(cells);
    }
    table
}

#[cfg(test)]
mod data_table_tests {
    use super::*;
    use somnium_i18n::Table;
    use somnium_ui::data_table::{Cell, View};

    fn catalog() -> Catalog {
        let mut catalog = Catalog::new("en");
        catalog.insert(
            Table::new("en")
                .with("menu.play", "Play")
                .with("menu.quit", "Quit"),
        );
        // `fr` is deliberately missing `menu.quit`, and carries a key `en` does
        // not have.
        catalog.insert(
            Table::new("fr")
                .with("menu.play", "Jouer")
                .with("menu.extra", "Bonus"),
        );
        catalog
    }

    #[test]
    fn a_catalog_becomes_keys_down_and_locales_across() {
        let table = catalog_to_table(&catalog());
        let titles: Vec<&str> = table
            .columns()
            .iter()
            .map(|column| column.title.as_str())
            .collect();
        assert_eq!(titles, ["Key", "en", "fr"]);
        assert_eq!(table.row_count(), 3, "the union of every locale's keys");
    }

    #[test]
    fn a_key_only_a_translation_has_is_still_a_row() {
        // Usually a mistake, and one nobody can see in a table that lists only
        // what the default locale has.
        let table = catalog_to_table(&catalog());
        let keys: Vec<String> = table
            .visible_rows(&View::default())
            .into_iter()
            .map(|row| table.get(row, KEY_COLUMN).display())
            .collect();
        assert!(keys.contains(&"menu.extra".to_owned()), "{keys:?}");
    }

    #[test]
    fn an_untranslated_string_is_empty_rather_than_blank_text() {
        let table = catalog_to_table(&catalog());
        let view = View {
            only_incomplete: true,
            ..View::default()
        };
        let missing: Vec<String> = table
            .visible_rows(&view)
            .into_iter()
            .map(|row| table.get(row, KEY_COLUMN).display())
            .collect();
        // `menu.play` is the only key both locales have.
        assert_eq!(missing, ["menu.extra", "menu.quit"]);

        let row = table
            .visible_rows(&View::default())
            .into_iter()
            .find(|row| table.get(*row, KEY_COLUMN).display() == "menu.quit")
            .unwrap();
        assert_eq!(table.get(row, "fr"), Cell::Empty);
    }

    #[test]
    fn a_translator_can_export_and_reimport_the_table() {
        // The workflow the stage names: send a CSV out, get it back edited.
        let table = catalog_to_table(&catalog());
        let mut back = somnium_ui::data_table::DataTable::new(table.columns().to_vec());
        back.read_csv(&table.to_csv()).expect("valid csv");
        assert_eq!(back.row_count(), table.row_count());
    }
}
