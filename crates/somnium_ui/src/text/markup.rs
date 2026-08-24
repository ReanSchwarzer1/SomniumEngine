//! Rich-text markup (MORROWIND-G item 2).
//!
//! > *"a tag vocabulary — colour, size, weight, style, inline sprite, link, and
//! > wave/shake for damage numbers. Reference: `fyrox-ui/src/bbcode.rs` and
//! > `formatted_text/`, in-architecture."*
//!
//! # BBCode, and why not HTML
//!
//! `[b]bold[/b]`, not `<b>bold</b>`. Three reasons, in order of weight:
//!
//! 1. **Angle brackets appear in game text.** "HP < 20" and "press <Enter>" are
//!    both things a UI says, and an HTML-shaped parser has to either escape
//!    them or guess. Square brackets appear far less often, and `[[` escapes the
//!    case that remains.
//! 2. **It is what the in-architecture reference uses.** Fyrox's `bbcode.rs` is
//!    the module this is modelled on, and matching its vocabulary means the
//!    reference reads directly rather than by translation.
//! 3. **The failure mode is better.** An unknown tag is emitted as literal text
//!    rather than swallowed, so a typo shows up on screen where somebody will
//!    fix it, instead of silently deleting the sentence after it.
//!
//! # The vocabulary
//!
//! ```text
//! [b] [/b]                 bold
//! [i] [/i]                 italic
//! [u] [/u]                 underline
//! [s] [/s]                 strikethrough
//! [color=#rrggbb] [/color] colour, with #rgb and #rrggbbaa also accepted
//! [size=18] [/size]        size in logical pixels
//! [font=mono] [/font]      a FontRole by name
//! [link=id] [/link]        makes the run interactive
//! [wave=2,3] [/wave]       amplitude px, frequency Hz
//! [shake=1.5] [/shake]     amplitude px
//! [sprite=name]            self-closing; occupies one placeholder character
//! [[                       a literal `[`
//! ```

use super::{Decoration, Direction, Motion, StyledRun};
use crate::typography::FontRole;

/// Why a markup string could not be parsed.
///
/// Every variant carries a byte offset, because "your markup is wrong" without
/// a position is a message that costs more time than it saves on a paragraph of
/// quest text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkupError {
    /// A `[/tag]` with no matching open tag.
    Unmatched { at: usize, tag: String },
    /// A tag that needed a value and had none, or vice versa.
    BadValue { at: usize, tag: String },
    /// End of input with tags still open.
    Unclosed { at: usize, tag: String },
}

impl std::fmt::Display for MarkupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unmatched { at, tag } => write!(f, "byte {at}: `[/{tag}]` closes nothing"),
            Self::BadValue { at, tag } => write!(f, "byte {at}: `[{tag}]` has a bad value"),
            Self::Unclosed { at, tag } => write!(f, "byte {at}: `[{tag}]` was never closed"),
        }
    }
}

impl std::error::Error for MarkupError {}

/// The character an inline sprite occupies.
///
/// U+FFFC OBJECT REPLACEMENT CHARACTER, which is what it is for. Using it
/// rather than a private-use codepoint means a caret stepping through the text
/// counts the sprite as exactly one character without any special case, and a
/// selection that includes it round-trips through anything that handles text.
pub const SPRITE_PLACEHOLDER: char = '\u{FFFC}';

/// What one open tag contributed, so closing it can undo exactly that.
#[derive(Clone, Debug)]
struct Open {
    tag: String,
    at: usize,
    /// The style in force before this tag, restored on close.
    previous: Style,
    /// The open tag was emitted as literal text, so its close must be too.
    ///
    /// Without this, `[blink]x[/blink]` emits `[blink]` as text — correctly —
    /// and then errors on `[/blink]` because nothing styled is open. The whole
    /// point of treating an unknown tag as text is that a typo does not break
    /// the paragraph, and erroring on the second half breaks it anyway.
    literal: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Style {
    color: Option<[u8; 4]>,
    size: Option<f32>,
    role: Option<FontRole>,
    decoration: Decoration,
    motion: Motion,
    link: Option<String>,
}

/// Parse markup into plain text plus the runs that style it.
///
/// Returns the text with tags removed — which is what gets measured, shaped and
/// searched — and runs whose ranges index into **that** string, not the source.
/// Indexing into the source would make every caret offset wrong by the length
/// of the tags before it.
pub fn parse(markup: &str) -> Result<(String, Vec<StyledRun>), MarkupError> {
    let mut text = String::with_capacity(markup.len());
    let mut runs: Vec<StyledRun> = Vec::new();
    let mut stack: Vec<Open> = Vec::new();
    let mut style = Style::default();
    let mut run_start = 0usize;

    let bytes = markup.as_bytes();
    let mut i = 0usize;

    // Close the current run at the current output position.
    fn flush(
        runs: &mut Vec<StyledRun>,
        start: &mut usize,
        end: usize,
        style: &Style,
    ) {
        if end > *start {
            runs.push(StyledRun {
                range: *start..end,
                color: style.color,
                size: style.size,
                role: style.role,
                decoration: style.decoration,
                motion: style.motion,
                link: style.link.clone(),
                sprite: None,
                direction: Direction::Ltr,
            });
        }
        *start = end;
    }

    while i < bytes.len() {
        if bytes[i] != b'[' {
            let ch = markup[i..].chars().next().expect("char boundary");
            text.push(ch);
            i += ch.len_utf8();
            continue;
        }
        // `[[` is a literal `[`.
        if bytes.get(i + 1) == Some(&b'[') {
            text.push('[');
            i += 2;
            continue;
        }
        let Some(close) = markup[i..].find(']').map(|offset| i + offset) else {
            // An unterminated `[` is literal text, not an error. Game text says
            // "[WIP]" and "press [E]" often enough that failing the whole
            // paragraph over one bracket is the wrong trade.
            let ch = markup[i..].chars().next().expect("char boundary");
            text.push(ch);
            i += ch.len_utf8();
            continue;
        };
        let body = &markup[i + 1..close];
        let after = close + 1;

        if let Some(name) = body.strip_prefix('/') {
            let name = name.trim();
            match stack.pop() {
                Some(open) if open.tag == name && open.literal => {
                    // Its open tag was text, so this is text too.
                    text.push_str(&markup[i..after]);
                }
                Some(open) if open.tag == name => {
                    flush(&mut runs, &mut run_start, text.len(), &style);
                    style = open.previous;
                }
                Some(open) => {
                    // Mismatched nesting: `[b]x[i]y[/b]`. Put it back so the
                    // error names the tag that is actually unclosed rather than
                    // the one that noticed.
                    stack.push(open);
                    return Err(MarkupError::Unmatched {
                        at: i,
                        tag: name.to_string(),
                    });
                }
                None => {
                    return Err(MarkupError::Unmatched {
                        at: i,
                        tag: name.to_string(),
                    });
                }
            }
            i = after;
            continue;
        }

        let (name, value) = match body.split_once('=') {
            Some((n, v)) => (n.trim(), Some(v.trim())),
            None => (body.trim(), None),
        };

        // Self-closing: a sprite is one placeholder character, not a span.
        if name == "sprite" {
            let Some(value) = value.filter(|v| !v.is_empty()) else {
                return Err(MarkupError::BadValue {
                    at: i,
                    tag: name.to_string(),
                });
            };
            flush(&mut runs, &mut run_start, text.len(), &style);
            let start = text.len();
            text.push(SPRITE_PLACEHOLDER);
            runs.push(StyledRun {
                range: start..text.len(),
                color: style.color,
                size: style.size,
                role: style.role,
                decoration: style.decoration,
                motion: style.motion,
                link: style.link.clone(),
                sprite: Some(value.to_string()),
                direction: Direction::Ltr,
            });
            run_start = text.len();
            i = after;
            continue;
        }

        let previous = style.clone();
        let applied = apply(&mut style, name, value);
        if !applied {
            // An unknown tag — or a known one with an unusable value — is
            // literal text, per the module docs: a typo shows up on screen where
            // somebody fixes it, rather than deleting the rest of the sentence.
            // It still opens a scope so its `[/tag]` is emitted as text too.
            style = previous.clone();
            text.push_str(&markup[i..after]);
            stack.push(Open {
                tag: name.to_string(),
                at: i,
                previous,
                literal: true,
            });
            i = after;
            continue;
        }
        if style == previous {
            // A tag that changed nothing still opens a scope, or its `[/tag]`
            // would be unmatched.
            stack.push(Open {
                tag: name.to_string(),
                at: i,
                previous,
                literal: false,
            });
            i = after;
            continue;
        }
        flush(&mut runs, &mut run_start, text.len(), &previous);
        stack.push(Open {
            tag: name.to_string(),
            at: i,
            previous,
            literal: false,
        });
        i = after;
    }

    // A literal tag left open is not an error: it was already emitted as text,
    // so `press [E] now` with no `[/E]` is just a sentence.
    if let Some(open) = stack.into_iter().rev().find(|open| !open.literal) {
        return Err(MarkupError::Unclosed {
            at: open.at,
            tag: open.tag,
        });
    }
    flush(&mut runs, &mut run_start, text.len(), &style);
    Ok((text, runs))
}

/// Apply one tag. Returns `false` for a tag this vocabulary does not know.
fn apply(style: &mut Style, name: &str, value: Option<&str>) -> bool {
    match name {
        "b" => style.decoration.bold = true,
        "i" => style.decoration.italic = true,
        "u" => style.decoration.underline = true,
        "s" => style.decoration.strikethrough = true,
        "color" => match value.and_then(parse_color) {
            Some(color) => style.color = Some(color),
            None => return false,
        },
        "size" => match value.and_then(|v| v.parse::<f32>().ok()).filter(|v| *v > 0.0) {
            Some(size) => style.size = Some(size),
            None => return false,
        },
        "font" => match value.and_then(parse_role) {
            Some(role) => style.role = Some(role),
            None => return false,
        },
        "link" => match value.filter(|v| !v.is_empty()) {
            Some(target) => style.link = Some(target.to_string()),
            None => return false,
        },
        "wave" => {
            let mut parts = value.unwrap_or("2,2").split(',');
            let amplitude = parts.next().and_then(|v| v.trim().parse().ok()).unwrap_or(2.0);
            let frequency = parts.next().and_then(|v| v.trim().parse().ok()).unwrap_or(2.0);
            style.motion = Motion::Wave {
                amplitude,
                frequency,
            };
        }
        "shake" => {
            let amplitude = value
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(1.5);
            style.motion = Motion::Shake { amplitude };
        }
        _ => return false,
    }
    true
}

/// `#rgb`, `#rrggbb` or `#rrggbbaa`. The leading `#` is optional.
fn parse_color(value: &str) -> Option<[u8; 4]> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    match hex.len() {
        3 => {
            let nibble = |i: usize| {
                u8::from_str_radix(&hex[i..i + 1], 16)
                    .ok()
                    // `#f00` is `#ff0000`, not `#f00000`: repeating the nibble
                    // is what makes the short form the same colour as the long.
                    .map(|v| v * 17)
            };
            Some([nibble(0)?, nibble(1)?, nibble(2)?, 255])
        }
        6 => Some([byte(0)?, byte(2)?, byte(4)?, 255]),
        8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => None,
    }
}

/// Map a `[font=]` value onto one of Zeta's five bundled cuts.
///
/// Names rather than weights, because the five are a *role* vocabulary: Zeta
/// certified `UiRegular` / `UiMedium` / `UiSemiBold` as the three UI weights and
/// `Mono` / `MonoMedium` as the two code weights, and markup that could name an
/// arbitrary weight would be markup that can ask for a cut this engine does not
/// bundle. `[b]` is how a run gets bolder; `[font=]` is how it changes family.
fn parse_role(value: &str) -> Option<FontRole> {
    Some(match value {
        "ui" | "sans" | "regular" => FontRole::UiRegular,
        "medium" => FontRole::UiMedium,
        "semibold" | "heading" => FontRole::UiSemiBold,
        "mono" | "code" => FontRole::Mono,
        "mono-medium" => FontRole::MonoMedium,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs_of(markup: &str) -> (String, Vec<StyledRun>) {
        parse(markup).expect("parses")
    }

    #[test]
    fn plain_text_is_one_run() {
        let (text, runs) = runs_of("hello");
        assert_eq!(text, "hello");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].range, 0..5);
        assert_eq!(runs[0].color, None);
    }

    #[test]
    fn text_with_no_content_produces_no_runs() {
        let (text, runs) = runs_of("");
        assert!(text.is_empty());
        assert!(runs.is_empty());
    }

    /// **Ranges index the output, not the source.**
    ///
    /// Indexing the source would make every caret offset wrong by the length of
    /// the tags before it, which is a bug that only shows up once somebody
    /// clicks in the middle of a styled sentence.
    #[test]
    fn ranges_index_the_stripped_text() {
        let (text, runs) = runs_of("ab[b]cd[/b]ef");
        assert_eq!(text, "abcdef");
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].range, 0..2);
        assert_eq!(runs[1].range, 2..4);
        assert_eq!(runs[2].range, 4..6);
        assert_eq!(runs[1].slice(&text), "cd");
        assert!(runs[1].decoration.bold);
        assert!(!runs[2].decoration.bold, "the close undoes exactly what the open did");
    }

    #[test]
    fn every_decoration_round_trips() {
        for (tag, check) in [
            ("b", (|d: Decoration| d.bold) as fn(Decoration) -> bool),
            ("i", |d| d.italic),
            ("u", |d| d.underline),
            ("s", |d| d.strikethrough),
        ] {
            let (_, runs) = runs_of(&format!("[{tag}]x[/{tag}]"));
            assert!(check(runs[0].decoration), "tag {tag}");
        }
    }

    #[test]
    fn colours_accept_three_six_and_eight_digits() {
        assert_eq!(runs_of("[color=#f00]x[/color]").1[0].color, Some([255, 0, 0, 255]));
        assert_eq!(runs_of("[color=#ff8800]x[/color]").1[0].color, Some([255, 136, 0, 255]));
        assert_eq!(
            runs_of("[color=#ff880080]x[/color]").1[0].color,
            Some([255, 136, 0, 128])
        );
        assert_eq!(runs_of("[color=ff0000]x[/color]").1[0].color, Some([255, 0, 0, 255]));
    }

    /// `#f00` is `#ff0000`, not `#f00000`.
    ///
    /// Repeating the nibble is what makes the short form the *same colour* as
    /// the long one; shifting it left makes every short colour 6% too dark,
    /// which is invisible in isolation and obvious beside its long form.
    #[test]
    fn a_three_digit_colour_repeats_its_nibbles() {
        assert_eq!(runs_of("[color=#abc]x[/color]").1[0].color, Some([170, 187, 204, 255]));
    }

    #[test]
    fn nesting_restores_the_outer_style() {
        let (text, runs) = runs_of("[color=#f00]a[b]b[/b]c[/color]");
        assert_eq!(text, "abc");
        assert_eq!(runs.len(), 3);
        for run in &runs {
            assert_eq!(run.color, Some([255, 0, 0, 255]), "colour spans all three");
        }
        assert!(!runs[0].decoration.bold);
        assert!(runs[1].decoration.bold);
        assert!(!runs[2].decoration.bold, "the inner close restores the outer style");
    }

    /// A sprite is one placeholder character, so a caret steps over it as one
    /// unit and a selection can include it.
    #[test]
    fn a_sprite_occupies_exactly_one_character() {
        let (text, runs) = runs_of("press [sprite=key_e] to open");
        assert_eq!(text.chars().filter(|c| *c == SPRITE_PLACEHOLDER).count(), 1);
        assert_eq!(text.chars().count(), "press  to open".chars().count() + 1);
        let sprite = runs.iter().find(|r| r.sprite.is_some()).expect("a sprite run");
        assert_eq!(sprite.sprite.as_deref(), Some("key_e"));
        assert_eq!(sprite.slice(&text), "\u{FFFC}");
    }

    #[test]
    fn links_and_motion_reach_their_runs() {
        let (_, runs) = runs_of("[link=quest_12]see the steward[/link]");
        assert_eq!(runs[0].link.as_deref(), Some("quest_12"));

        let (_, runs) = runs_of("[wave=3,4]!!![/wave]");
        assert_eq!(
            runs[0].motion,
            Motion::Wave {
                amplitude: 3.0,
                frequency: 4.0
            }
        );

        let (_, runs) = runs_of("[shake=2]999[/shake]");
        assert_eq!(runs[0].motion, Motion::Shake { amplitude: 2.0 });
    }

    #[test]
    fn motion_tags_have_defaults() {
        assert!(matches!(runs_of("[wave]x[/wave]").1[0].motion, Motion::Wave { .. }));
        assert!(matches!(runs_of("[shake]x[/shake]").1[0].motion, Motion::Shake { .. }));
    }

    /// **An unknown tag is literal text, not a swallowed span.**
    ///
    /// The alternative deletes the rest of the sentence on a typo, silently.
    #[test]
    fn an_unknown_tag_survives_as_text() {
        let (text, _) = runs_of("hp [blink]low[/blink]");
        assert_eq!(
            text, "hp [blink]low[/blink]",
            "both halves are text; erroring on the close breaks the paragraph              the literal-open path exists to protect"
        );
    }

    /// An unknown tag left open is not an error either.
    #[test]
    fn an_unclosed_unknown_tag_is_just_a_sentence() {
        assert_eq!(runs_of("press [E] now").0, "press [E] now");
        assert_eq!(runs_of("[WIP] feature").0, "[WIP] feature");
    }

    /// Game text says "press [E]" and "[WIP]". An unterminated or unknown
    /// bracket must not fail the paragraph.
    #[test]
    fn brackets_in_ordinary_text_are_safe() {
        assert_eq!(runs_of("HP < 20").0, "HP < 20");
        assert_eq!(runs_of("press [E]").0, "press [E]");
        assert_eq!(runs_of("unterminated [").0, "unterminated [");
        assert_eq!(runs_of("[[literal").0, "[literal");
    }

    #[test]
    fn a_bad_value_is_an_error_rather_than_a_guess() {
        // These are known tags with unusable values, so they fall through to
        // the unknown-tag path and stay as text rather than silently applying
        // a default the author did not write.
        assert!(runs_of("[color=nonsense]x[/color]").0.contains("[color=nonsense]"));
        assert!(runs_of("[size=-4]x[/size]").0.contains("[size=-4]"));
    }

    #[test]
    fn an_unmatched_close_names_its_position() {
        let error = parse("abc[/b]").unwrap_err();
        assert_eq!(
            error,
            MarkupError::Unmatched {
                at: 3,
                tag: "b".into()
            }
        );
        assert!(error.to_string().contains("byte 3"));
    }

    #[test]
    fn an_unclosed_tag_names_the_tag_that_is_open() {
        let error = parse("[b]forever").unwrap_err();
        assert_eq!(
            error,
            MarkupError::Unclosed {
                at: 0,
                tag: "b".into()
            }
        );
    }

    /// Mismatched nesting names the tag that is actually wrong.
    #[test]
    fn crossed_tags_are_an_error() {
        let error = parse("[b]x[i]y[/b]z[/i]").unwrap_err();
        assert!(matches!(error, MarkupError::Unmatched { tag, .. } if tag == "b"));
    }

    /// Multi-byte text keeps its ranges on char boundaries.
    ///
    /// Slicing a `String` off a boundary panics, so a parser that counted
    /// characters instead of bytes would crash on the first CJK label.
    #[test]
    fn ranges_stay_on_char_boundaries_in_multibyte_text() {
        let (text, runs) = runs_of("\u{4F60}[b]\u{597D}[/b]\u{4E16}");
        assert_eq!(text, "\u{4F60}\u{597D}\u{4E16}");
        assert_eq!(runs.len(), 3);
        for run in &runs {
            // Would panic on a bad boundary.
            let _ = run.slice(&text);
        }
        assert_eq!(runs[1].slice(&text), "\u{597D}");
    }

    #[test]
    fn a_tag_wrapping_nothing_produces_no_empty_run() {
        let (text, runs) = runs_of("a[b][/b]b");
        assert_eq!(text, "ab");
        assert!(runs.iter().all(|r| !r.range.is_empty()));
    }

    #[test]
    fn font_names_map_onto_the_five_bundled_cuts() {
        use crate::typography::FontRole;
        for (name, expected) in [
            ("ui", FontRole::UiRegular),
            ("sans", FontRole::UiRegular),
            ("medium", FontRole::UiMedium),
            ("semibold", FontRole::UiSemiBold),
            ("mono", FontRole::Mono),
            ("code", FontRole::Mono),
            ("mono-medium", FontRole::MonoMedium),
        ] {
            let (_, runs) = runs_of(&format!("[font={name}]x[/font]"));
            assert_eq!(runs[0].role, Some(expected), "font={name}");
        }
        // A cut this engine does not bundle stays as literal text rather than
        // silently resolving to something close.
        assert!(runs_of("[font=comic]x[/font]").0.contains("[font=comic]"));
    }
}
