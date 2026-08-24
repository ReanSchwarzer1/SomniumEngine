//! The localisation hook (MORROWIND-G item 6).
//!
//! > *"text is a key plus arguments, resolved through `somnium_i18n`
//! > (MORROWIND-AH), never a baked literal."*
//!
//! # This is a hook, not an implementation
//!
//! `somnium_i18n` is MORROWIND-AH, in Track 8, and does not exist. What exists
//! here is the *shape* a localised string has, so that the ~86 widget call
//! sites which currently take `&str` can be migrated one at a time instead of
//! all at once when AH lands — and so that new code written between now and
//! then does not add to the pile.
//!
//! # Why a key plus arguments, and not a formatted string
//!
//! The tempting shortcut is `format!("You have {n} potions")` at the call site,
//! localised by translating the format string. It breaks on the first language
//! with grammatical number beyond singular/plural, and it breaks quietly:
//!
//! - Polish has three plural forms; Arabic has six. A `{n}` substituted into a
//!   pre-formatted string cannot choose between them, because the choice has to
//!   happen *inside* the resolver, which needs to see `n`.
//! - Word order differs. "You have 3 potions" and its Japanese equivalent put
//!   the count in different places, and a translator handed a format string can
//!   move `{n}` — but a translator handed "You have " and " potions" cannot.
//!
//! So the argument stays an argument until the resolver sees it. That is the
//! only design that lets a translation file decide plural form and word order,
//! and it is why [`LocalizedText`] carries `Vec<Argument>` rather than a
//! `String`.
//!
//! # Untranslated text is still allowed, and says so
//!
//! [`LocalizedText::literal`] exists because a debug overlay, a log line and a
//! developer tool are not translated and pretending otherwise adds a key per
//! diagnostic. The variant is named `Literal` so a lint or an audit can count
//! them, which is the point: an untranslated string that is *marked* is a
//! decision, and one that is merely a `&str` is an oversight nobody can find.

use std::borrow::Cow;

/// A translation key.
///
/// A `&'static str` because keys are authored in source and interned by the
/// compiler; a `String` here would allocate on every label in every frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextKey(pub &'static str);

impl TextKey {
    /// The key as written.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for TextKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// One substitution argument.
///
/// Typed rather than stringly, because the resolver needs to *see* a number to
/// choose a plural form and to format it in the locale's own digits and
/// separators. A pre-formatted `"1,234"` has already lost both.
#[derive(Clone, Debug, PartialEq)]
pub enum Argument {
    /// A whole number. Drives plural selection.
    Count(i64),
    /// A real number, formatted by the locale.
    Number(f64),
    /// Text that is itself already localised, or a proper noun.
    Text(String),
}

impl From<i64> for Argument {
    fn from(value: i64) -> Self {
        Self::Count(value)
    }
}

impl From<f64> for Argument {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

impl From<&str> for Argument {
    fn from(value: &str) -> Self {
        Self::Text(value.to_string())
    }
}

impl From<String> for Argument {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

/// Text a widget displays: a key with arguments, or a marked literal.
#[derive(Clone, Debug, PartialEq)]
pub enum LocalizedText {
    /// Resolved through the translation table.
    Key {
        key: TextKey,
        /// `(name, value)` pairs. Named rather than positional so a translator
        /// can reorder them, which is the whole reason arguments stay
        /// arguments.
        args: Vec<(&'static str, Argument)>,
    },
    /// Deliberately untranslated. Debug overlays, log lines, developer tools.
    Literal(String),
}

impl LocalizedText {
    /// A key with no arguments.
    #[must_use]
    pub fn key(key: &'static str) -> Self {
        Self::Key {
            key: TextKey(key),
            args: Vec::new(),
        }
    }

    /// Add a named argument.
    #[must_use]
    pub fn arg(mut self, name: &'static str, value: impl Into<Argument>) -> Self {
        if let Self::Key { args, .. } = &mut self {
            args.push((name, value.into()));
        }
        self
    }

    /// Deliberately untranslated text.
    #[must_use]
    pub fn literal(text: impl Into<String>) -> Self {
        Self::Literal(text.into())
    }

    /// Whether this is a marked literal rather than a key.
    #[must_use]
    pub fn is_literal(&self) -> bool {
        matches!(self, Self::Literal(_))
    }

    /// Resolve through `resolver`, falling back to the key itself.
    ///
    /// **Falling back to the key, not to empty.** A missing translation should
    /// render `inventory.potion_count` on screen — ugly, findable, and
    /// obviously wrong — rather than a blank label, which looks like a layout
    /// bug and gets filed against the wrong system.
    #[must_use]
    pub fn resolve<'a>(&'a self, resolver: &dyn Resolver) -> Cow<'a, str> {
        match self {
            Self::Literal(text) => Cow::Borrowed(text.as_str()),
            Self::Key { key, args } => match resolver.resolve(*key, args) {
                Some(text) => Cow::Owned(text),
                None => Cow::Borrowed(key.as_str()),
            },
        }
    }
}

impl From<&'static str> for LocalizedText {
    /// A bare `&'static str` becomes a **key**, not a literal.
    ///
    /// The default has to be the translated one: a call site that says
    /// `"menu.quit"` means a key, and one that genuinely wants untranslated
    /// text should have to say [`LocalizedText::literal`] and be countable.
    fn from(value: &'static str) -> Self {
        Self::key(value)
    }
}

/// Something that can turn a key and arguments into text.
///
/// `somnium_i18n` (MORROWIND-AH) will implement this. Until then
/// [`PassthroughResolver`] stands in, and the trait is what makes the
/// substitution a one-line change rather than a migration.
pub trait Resolver {
    /// Resolve, or `None` when the key is unknown to this resolver.
    fn resolve(&self, key: TextKey, args: &[(&'static str, Argument)]) -> Option<String>;
}

/// The stand-in until `somnium_i18n` exists.
///
/// Returns the key with `{name}` placeholders substituted, so a call site can
/// be written and *seen to work* before there is a translation table. It does
/// no plural selection and no locale-aware number formatting, and it must not
/// be mistaken for one that does — hence the name.
#[derive(Debug, Default)]
pub struct PassthroughResolver;

impl Resolver for PassthroughResolver {
    fn resolve(&self, key: TextKey, args: &[(&'static str, Argument)]) -> Option<String> {
        if args.is_empty() {
            return None;
        }
        let mut out = key.as_str().to_string();
        for (name, value) in args {
            let placeholder = format!("{{{name}}}");
            let rendered = match value {
                Argument::Count(n) => n.to_string(),
                Argument::Number(n) => n.to_string(),
                Argument::Text(t) => t.clone(),
            };
            out = out.replace(&placeholder, &rendered);
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Table(&'static str);

    impl Resolver for Table {
        fn resolve(&self, _key: TextKey, args: &[(&'static str, Argument)]) -> Option<String> {
            let mut out = self.0.to_string();
            for (name, value) in args {
                let rendered = match value {
                    Argument::Count(n) => n.to_string(),
                    Argument::Number(n) => n.to_string(),
                    Argument::Text(t) => t.clone(),
                };
                out = out.replace(&format!("{{{name}}}"), &rendered);
            }
            Some(out)
        }
    }

    #[test]
    fn a_key_resolves_through_the_table() {
        let text = LocalizedText::key("inventory.potions").arg("n", 3i64);
        assert_eq!(
            text.resolve(&Table("You have {n} potions")),
            "You have 3 potions"
        );
    }

    /// **Word order is the translator's, not the call site's.**
    ///
    /// The same key and the same argument produce a different sentence shape,
    /// which is impossible if the call site pre-formats the string.
    #[test]
    fn the_translation_decides_where_the_argument_goes() {
        let text = LocalizedText::key("inventory.potions").arg("n", 3i64);
        assert_eq!(
            text.resolve(&Table("You have {n} potions")),
            "You have 3 potions"
        );
        assert_eq!(
            text.resolve(&Table("{n} potions remain")),
            "3 potions remain"
        );
        assert_eq!(text.resolve(&Table("Potions: {n}")), "Potions: 3");
    }

    /// The resolver sees the *number*, which is what plural selection needs.
    ///
    /// A pre-formatted "3" cannot be dispatched on; a `Count(3)` can, and Polish
    /// needs three forms while Arabic needs six.
    #[test]
    fn a_count_argument_stays_a_number() {
        let text = LocalizedText::key("k").arg("n", 1i64);
        let LocalizedText::Key { args, .. } = &text else {
            panic!("a key");
        };
        assert_eq!(args[0].1, Argument::Count(1));
        assert!(
            matches!(args[0].1, Argument::Count(_)),
            "the resolver must be able to dispatch on the number itself"
        );
    }

    /// **A missing translation shows the key, not a blank.**
    ///
    /// A blank label looks like a layout bug and gets filed against the wrong
    /// system; `inventory.potions` on screen is ugly, findable, and obviously a
    /// translation problem.
    #[test]
    fn a_missing_translation_falls_back_to_the_key() {
        struct Empty;
        impl Resolver for Empty {
            fn resolve(&self, _: TextKey, _: &[(&'static str, Argument)]) -> Option<String> {
                None
            }
        }
        let text = LocalizedText::key("inventory.potions");
        assert_eq!(text.resolve(&Empty), "inventory.potions");
    }

    /// A literal is never sent to the resolver.
    #[test]
    fn a_literal_passes_through_untouched() {
        struct Panics;
        impl Resolver for Panics {
            fn resolve(&self, _: TextKey, _: &[(&'static str, Argument)]) -> Option<String> {
                panic!("a literal must not reach the resolver");
            }
        }
        let text = LocalizedText::literal("fps: 60");
        assert_eq!(text.resolve(&Panics), "fps: 60");
        assert!(text.is_literal());
    }

    /// A bare `&'static str` is a key, so untranslated text has to say so.
    ///
    /// The default has to be the translated one: making `Literal` the easy path
    /// is how a codebase ends up with a thousand untranslatable labels nobody
    /// can enumerate.
    #[test]
    fn a_bare_string_defaults_to_a_key_not_a_literal() {
        let text: LocalizedText = "menu.quit".into();
        assert!(!text.is_literal());
        assert_eq!(text, LocalizedText::key("menu.quit"));
    }

    #[test]
    fn arguments_accept_the_obvious_types() {
        let text = LocalizedText::key("k")
            .arg("count", 2i64)
            .arg("ratio", 0.5f64)
            .arg("name", "Nerevar");
        let LocalizedText::Key { args, .. } = &text else {
            panic!("a key");
        };
        assert_eq!(args[0].1, Argument::Count(2));
        assert_eq!(args[1].1, Argument::Number(0.5));
        assert_eq!(args[2].1, Argument::Text("Nerevar".into()));
    }

    /// Adding an argument to a literal does nothing rather than panicking.
    #[test]
    fn arguments_on_a_literal_are_a_no_op() {
        let text = LocalizedText::literal("plain").arg("n", 1i64);
        assert_eq!(text, LocalizedText::Literal("plain".into()));
    }

    /// The stand-in substitutes but does not pretend to pluralise.
    #[test]
    fn the_passthrough_resolver_substitutes_and_no_more() {
        let text = LocalizedText::key("have {n} of {what}")
            .arg("n", 2i64)
            .arg("what", "potion");
        assert_eq!(text.resolve(&PassthroughResolver), "have 2 of potion");

        // No arguments means nothing to substitute, so it declines and the key
        // shows — which is the same visible outcome as a missing translation,
        // deliberately.
        assert_eq!(
            LocalizedText::key("menu.quit").resolve(&PassthroughResolver),
            "menu.quit"
        );
    }
}
