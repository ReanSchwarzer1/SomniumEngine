//! IME composition (MORROWIND-G item 5).
//!
//! > *"composition strings, candidate windows, and the winit plumbing. Without
//! > it the engine cannot accept a Japanese character in a text box."*
//!
//! # What "cannot accept" means concretely
//!
//! `text_box.rs` handles `KeyboardInput` and appends the character it receives.
//! On a Japanese, Chinese or Korean keyboard the characters a person types are
//! not the characters they mean: typing `nihon` produces five Latin letters
//! that the input method converts into 日本 once the user confirms it. An engine
//! that reads the raw keystrokes gets `nihon` in the box and no way to reach
//! 日本 at all.
//!
//! winit reports this through `WindowEvent::Ime`, which nothing in the tree
//! currently handles. This module is the state machine behind that event.
//!
//! # Preedit is not text yet, and that is the whole design
//!
//! The composition string is **provisional**: it is shown to the user, underlined
//! and with a cursor inside it, and it can change completely on the next
//! keystroke or be abandoned. It must not be committed to the buffer, must not
//! be part of undo, and must not be sent to whatever is listening for the
//! field's value.
//!
//! Getting that wrong is the classic IME bug and it is *invisible in English*:
//! everything works until somebody types Japanese, and then a half-finished
//! romanisation is saved as a character's name.

/// What the platform told us about composition.
///
/// Mirrors `winit::event::Ime` without depending on the event type, so the
/// state machine is testable without a window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImeEvent {
    /// The IME is now active for this window. Nothing is composed yet.
    Enabled,
    /// The provisional string, and the byte range of the cursor within it.
    ///
    /// An empty string with `None` means composition was cancelled.
    Preedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
    /// The user confirmed. This text — and only this — becomes real.
    Commit(String),
    /// The IME is no longer active. Any in-flight composition is abandoned.
    Disabled,
}

/// What the caller should do after feeding an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImeOutcome {
    /// Nothing changed.
    Ignored,
    /// The provisional string changed; redraw, do not commit.
    PreeditChanged,
    /// Insert this text at the caret. **The only path to the buffer.**
    Commit(String),
    /// Composition was abandoned; drop the provisional string.
    Cancelled,
}

/// In-flight composition state for one text field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Composition {
    /// The provisional string, empty when nothing is composing.
    text: String,
    /// Byte range of the IME's cursor within `text`.
    cursor: Option<(usize, usize)>,
    /// Whether the platform IME is active for this field.
    enabled: bool,
}

impl Composition {
    /// Nothing composing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The provisional string. Draw it underlined; do not store it.
    #[must_use]
    pub fn preedit(&self) -> &str {
        &self.text
    }

    /// Byte range of the IME cursor within [`Self::preedit`].
    #[must_use]
    pub fn cursor(&self) -> Option<(usize, usize)> {
        self.cursor
    }

    /// Whether something is currently being composed.
    ///
    /// A field with an active composition **must not** commit on Enter, submit
    /// a form, or close a dialog: the first Enter confirms the composition and
    /// belongs to the IME. Swallowing it is the difference between a text box
    /// that works in Japanese and one that closes the dialog every time
    /// somebody finishes a word.
    #[must_use]
    pub fn is_composing(&self) -> bool {
        !self.text.is_empty()
    }

    /// Whether the platform IME is enabled for this field.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Feed a platform event.
    pub fn handle(&mut self, event: ImeEvent) -> ImeOutcome {
        match event {
            ImeEvent::Enabled => {
                self.enabled = true;
                // Enabling does not clear an existing composition on every
                // platform, but starting from a clean slate is the only state
                // that is correct on all of them.
                let was = std::mem::take(&mut self.text);
                self.cursor = None;
                if was.is_empty() {
                    ImeOutcome::Ignored
                } else {
                    ImeOutcome::Cancelled
                }
            }
            ImeEvent::Preedit { text, cursor } => {
                if text.is_empty() && cursor.is_none() {
                    // The documented cancellation signal.
                    let was_composing = self.is_composing();
                    self.text.clear();
                    self.cursor = None;
                    return if was_composing {
                        ImeOutcome::Cancelled
                    } else {
                        ImeOutcome::Ignored
                    };
                }
                if self.text == text && self.cursor == cursor {
                    return ImeOutcome::Ignored;
                }
                self.text = text;
                self.cursor = cursor;
                ImeOutcome::PreeditChanged
            }
            ImeEvent::Commit(text) => {
                // The composition ends whether or not the commit is empty: a
                // stale preedit left behind here is drawn forever over text the
                // user has already accepted.
                self.text.clear();
                self.cursor = None;
                if text.is_empty() {
                    ImeOutcome::Cancelled
                } else {
                    ImeOutcome::Commit(text)
                }
            }
            ImeEvent::Disabled => {
                self.enabled = false;
                let was_composing = self.is_composing();
                self.text.clear();
                self.cursor = None;
                if was_composing {
                    ImeOutcome::Cancelled
                } else {
                    ImeOutcome::Ignored
                }
            }
        }
    }

    /// Whether a `Return` should be swallowed by the IME rather than submitting.
    ///
    /// The one-line version of the rule above, so a call site reads as intent
    /// rather than as a state check.
    #[must_use]
    pub fn swallows_enter(&self) -> bool {
        self.is_composing()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preedit(text: &str) -> ImeEvent {
        ImeEvent::Preedit {
            text: text.to_string(),
            cursor: Some((text.len(), text.len())),
        }
    }

    /// The whole point: `nihon` is provisional, 日本 is what gets committed.
    #[test]
    fn a_romanised_composition_never_reaches_the_buffer() {
        let mut ime = Composition::new();
        assert_eq!(ime.handle(ImeEvent::Enabled), ImeOutcome::Ignored);

        for step in ["n", "ni", "nih", "niho", "nihon"] {
            assert_eq!(ime.handle(preedit(step)), ImeOutcome::PreeditChanged);
            assert_eq!(ime.preedit(), step);
            assert!(ime.is_composing());
        }

        assert_eq!(
            ime.handle(ImeEvent::Commit("\u{65E5}\u{672C}".into())),
            ImeOutcome::Commit("\u{65E5}\u{672C}".into()),
            "only the confirmed text is real"
        );
        assert!(!ime.is_composing());
        assert_eq!(ime.preedit(), "", "no stale preedit drawn over the result");
    }

    /// **The bug that is invisible in English.** Enter during composition
    /// belongs to the IME, not to the dialog.
    #[test]
    fn enter_is_swallowed_while_composing() {
        let mut ime = Composition::new();
        ime.handle(ImeEvent::Enabled);
        assert!(!ime.swallows_enter(), "nothing composing, Enter submits");

        ime.handle(preedit("nihon"));
        assert!(
            ime.swallows_enter(),
            "the first Enter confirms the composition; a dialog that closes \
             here closes every time somebody finishes a word"
        );

        ime.handle(ImeEvent::Commit("\u{65E5}\u{672C}".into()));
        assert!(!ime.swallows_enter(), "composition over, Enter submits again");
    }

    #[test]
    fn an_empty_preedit_with_no_cursor_cancels() {
        let mut ime = Composition::new();
        ime.handle(ImeEvent::Enabled);
        ime.handle(preedit("nih"));
        assert_eq!(
            ime.handle(ImeEvent::Preedit {
                text: String::new(),
                cursor: None
            }),
            ImeOutcome::Cancelled
        );
        assert!(!ime.is_composing());
    }

    /// Cancelling when nothing was composing is not an event worth redrawing for.
    #[test]
    fn cancelling_nothing_is_ignored() {
        let mut ime = Composition::new();
        assert_eq!(
            ime.handle(ImeEvent::Preedit {
                text: String::new(),
                cursor: None
            }),
            ImeOutcome::Ignored
        );
    }

    /// A repeated identical preedit does not redraw.
    ///
    /// Some platforms resend it on every key, including keys that changed
    /// nothing, and a text box that redraws on each one flickers its underline.
    #[test]
    fn an_unchanged_preedit_is_ignored() {
        let mut ime = Composition::new();
        ime.handle(ImeEvent::Enabled);
        assert_eq!(ime.handle(preedit("ni")), ImeOutcome::PreeditChanged);
        assert_eq!(ime.handle(preedit("ni")), ImeOutcome::Ignored);
    }

    /// Losing focus mid-composition abandons it rather than committing it.
    ///
    /// Committing a half-finished romanisation is how "nihon" ends up saved as
    /// a character's name.
    #[test]
    fn disabling_mid_composition_cancels_rather_than_commits() {
        let mut ime = Composition::new();
        ime.handle(ImeEvent::Enabled);
        ime.handle(preedit("nihon"));
        assert_eq!(ime.handle(ImeEvent::Disabled), ImeOutcome::Cancelled);
        assert!(!ime.is_composing());
        assert!(!ime.is_enabled());
    }

    /// Re-enabling starts clean, whatever the platform left behind.
    #[test]
    fn re_enabling_discards_a_stale_composition() {
        let mut ime = Composition::new();
        ime.handle(ImeEvent::Enabled);
        ime.handle(preedit("nih"));
        assert_eq!(ime.handle(ImeEvent::Enabled), ImeOutcome::Cancelled);
        assert_eq!(ime.preedit(), "");
    }

    /// An empty commit ends the composition without inserting anything.
    #[test]
    fn an_empty_commit_ends_composition_without_inserting() {
        let mut ime = Composition::new();
        ime.handle(ImeEvent::Enabled);
        ime.handle(preedit("ni"));
        assert_eq!(ime.handle(ImeEvent::Commit(String::new())), ImeOutcome::Cancelled);
        assert!(!ime.is_composing());
    }

    /// The cursor range is carried through, so a candidate window can be placed.
    #[test]
    fn the_preedit_cursor_survives() {
        let mut ime = Composition::new();
        ime.handle(ImeEvent::Enabled);
        ime.handle(ImeEvent::Preedit {
            text: "nihon".into(),
            cursor: Some((2, 4)),
        });
        assert_eq!(ime.cursor(), Some((2, 4)));
    }
}
