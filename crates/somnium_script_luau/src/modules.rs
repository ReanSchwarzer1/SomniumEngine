//! `require`, and why it is resolved before a script runs.
//!
//! # The rule
//!
//! `require` takes a **string literal** and nothing else. Not a variable,
//! not a concatenation, not a value returned by a function.
//!
//! That is not a limitation the implementation happened to have; it is the
//! whole point. A `require` whose argument is only known at run time makes
//! the module graph undiscoverable, and three things in this phase depend
//! on the graph being static:
//!
//! * **hot reload** has to know the blast radius of an edit — which
//!   modules must be recompiled and which instances must be rebuilt —
//!   before it touches anything;
//! * **the cook** has to know what to bundle;
//! * **cycle detection** happens once, in Rust, on a graph, rather than as
//!   a runtime guard that only fires when the unlucky path is taken.
//!
//! So the argument is read out of the source text at compile time, and a
//! dynamic one is a compile error with a line number rather than a
//! surprise at frame four hundred.
//!
//! # And why the scan is a scanner
//!
//! `text.contains("require(")` would find the word inside a comment and
//! inside a string, and would miss `require "x"` — which Lua's call
//! sugar makes legal. The scanner below skips comments and strings
//! properly. It is about eighty lines and it is the difference between a
//! dependency graph and a guess.

/// What every misuse of `require` is told.
///
/// One message for every shape, because they are all the same mistake:
/// the engine reads the dependency graph out of the source text, so any
/// `require` it cannot follow by reading is one it cannot follow at all.
/// That includes merely *mentioning* the name — `local r = require` is an
/// alias, and an alias is a computed call one line later.
const DYNAMIC_REQUIRE: &str = "`require` may only be written as `require(\"module\")` with a \
     literal name. The engine reads the dependency graph out of the source \
     before anything runs — so a computed name, a concatenation, or storing \
     `require` in a variable cannot be resolved, and would leave hot reload \
     unable to tell what an edit affects.";

/// One `require` found in a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequireSite {
    /// The literal module name.
    pub module: String,
    /// One-based line, for diagnostics.
    pub line: u32,
}

/// Every `require` in a source file, in source order.
///
/// # Errors
///
/// A message and a line for a `require` whose argument is not a string
/// literal.
pub fn parse_requires(text: &str) -> Result<Vec<RequireSite>, (u32, String)> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    let mut line = 1_u32;

    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                i += 2;
                if let Some(len) = long_bracket_len(bytes, i) {
                    let (end, lines) = skip_long_bracket(bytes, i + len, len);
                    line += lines;
                    i = end;
                } else {
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
            }
            b'"' | b'\'' => {
                let (end, lines) = skip_quoted(bytes, i);
                line += lines;
                i = end;
            }
            b'[' => {
                if let Some(len) = long_bracket_len(bytes, i) {
                    let (end, lines) = skip_long_bracket(bytes, i + len, len);
                    line += lines;
                    i = end;
                } else {
                    i += 1;
                }
            }
            _ if is_word_start(bytes[i]) => {
                let start = i;
                while i < bytes.len() && is_word(bytes[i]) {
                    i += 1;
                }
                // A preceding `.` or `:` means this is a field access —
                // `self.require` is not the global.
                let qualified = text[..start]
                    .trim_end_matches([' ', '\t'])
                    .ends_with(['.', ':']);
                if !qualified && &text[start..i] == "require" {
                    let (module, next, lines) = read_require_argument(text, bytes, i, line)?;
                    found.push(RequireSite { module, line });
                    line += lines;
                    i = next;
                }
            }
            _ => i += 1,
        }
    }
    Ok(found)
}

/// Read the literal after a `require`, or explain why there isn't one.
fn read_require_argument(
    text: &str,
    bytes: &[u8],
    mut i: usize,
    line: u32,
) -> Result<(String, usize, u32), (u32, String)> {
    let mut lines = 0;
    // `require("x")` and `require "x"` are both legal Lua.
    let mut expect_close = false;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\r' => i += 1,
            b'\n' => {
                lines += 1;
                i += 1;
            }
            b'(' if !expect_close => {
                expect_close = true;
                i += 1;
            }
            b'"' | b'\'' => break,
            _ => return Err((line, DYNAMIC_REQUIRE.to_string())),
        }
    }
    if i >= bytes.len() {
        return Err((line, DYNAMIC_REQUIRE.to_string()));
    }

    let quote = bytes[i];
    let start = i + 1;
    let mut j = start;
    while j < bytes.len() && bytes[j] != quote {
        if bytes[j] == b'\\' {
            j += 1;
        }
        if bytes.get(j) == Some(&b'\n') {
            return Err((line, "`require`'s module name is unterminated".to_string()));
        }
        j += 1;
    }
    if j >= bytes.len() {
        return Err((line, "`require`'s module name is unterminated".to_string()));
    }
    let module = text[start..j].to_string();
    if module.is_empty() {
        return Err((line, "`require` was given an empty module name".to_string()));
    }

    let mut after = j + 1;
    if expect_close {
        // `require('a' .. suffix)` starts with a literal and is still
        // dynamic. The literal alone is not enough — the call has to end
        // right after it.
        while matches!(bytes.get(after), Some(b' ' | b'\t' | b'\r')) {
            after += 1;
        }
        if bytes.get(after) != Some(&b')') {
            return Err((line, DYNAMIC_REQUIRE.to_string()));
        }
        after += 1;
    }
    Ok((module, after, lines))
}

const fn is_word_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

const fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// If `bytes[i..]` opens a long bracket (`[[`, `[=[`, `[==[` …), how many
/// bytes the opener takes.
fn long_bracket_len(bytes: &[u8], i: usize) -> Option<usize> {
    if bytes.get(i) != Some(&b'[') {
        return None;
    }
    let mut j = i + 1;
    while bytes.get(j) == Some(&b'=') {
        j += 1;
    }
    if bytes.get(j) == Some(&b'[') {
        Some(j + 1 - i)
    } else {
        None
    }
}

/// Skip to just past the closing bracket of a long string or comment.
fn skip_long_bracket(bytes: &[u8], from: usize, opener_len: usize) -> (usize, u32) {
    let equals = opener_len.saturating_sub(2);
    let mut i = from;
    let mut lines = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            lines += 1;
        }
        if bytes[i] == b']' {
            let mut j = i + 1;
            let mut seen = 0;
            while bytes.get(j) == Some(&b'=') {
                seen += 1;
                j += 1;
            }
            if seen == equals && bytes.get(j) == Some(&b']') {
                return (j + 1, lines);
            }
        }
        i += 1;
    }
    (bytes.len(), lines)
}

/// Skip a `"…"` or `'…'` string.
fn skip_quoted(bytes: &[u8], from: usize) -> (usize, u32) {
    let quote = bytes[from];
    let mut i = from + 1;
    let mut lines = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'\n' => {
                lines += 1;
                i += 1;
            }
            b if b == quote => return (i + 1, lines),
            _ => i += 1,
        }
    }
    (bytes.len(), lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(text: &str) -> Vec<String> {
        parse_requires(text)
            .unwrap()
            .into_iter()
            .map(|site| site.module)
            .collect()
    }

    #[test]
    fn both_call_spellings_are_found() {
        assert_eq!(
            names("local a = require('one')\nlocal b = require \"two\"\n"),
            vec!["one", "two"]
        );
    }

    #[test]
    fn a_require_inside_a_comment_or_a_string_is_not_a_dependency() {
        assert!(names("-- require('ghost')\n").is_empty());
        assert!(names("--[[ require('ghost') ]]\n").is_empty());
        assert!(names("local s = \"require('ghost')\"\n").is_empty());
        assert!(names("local s = [[ require('ghost') ]]\n").is_empty());
        assert!(names("local s = [==[ require('ghost') ]==]\n").is_empty());
    }

    #[test]
    fn a_field_named_require_is_not_the_global() {
        assert!(names("self.require('ghost')").is_empty());
        assert!(names("t:require('ghost')").is_empty());
        assert!(
            names("local prerequire = 1\nlocal requires = 2\n").is_empty(),
            "and neither is a longer identifier that merely contains it"
        );
    }

    #[test]
    fn a_computed_module_name_is_a_compile_error_with_a_line() {
        let (line, message) = parse_requires("local x = 'a'\nlocal m = require(x)\n").unwrap_err();
        assert_eq!(line, 2);
        assert!(message.contains("dependency graph"), "{message}");
    }

    #[test]
    fn merely_mentioning_require_is_refused_too() {
        // `local r = require` is an alias, and an alias is a computed call
        // one line later. Refusing the mention is what makes the graph a
        // fact about the source rather than a hope.
        assert!(parse_requires("local r = require").is_err());
        assert!(parse_requires("if type(require) ~= 'nil' then end").is_err());
        assert!(parse_requires("return { r = require }").is_err());
    }

    #[test]
    fn a_concatenated_module_name_is_refused_too() {
        // The whole point: this is the shape that makes a graph impossible.
        assert!(parse_requires("require('a' .. suffix)").is_err());
    }

    #[test]
    fn the_reported_line_survives_earlier_multi_line_constructs() {
        let text = "--[[\n\n\n]]\nlocal s = \"x\\ny\"\n\nrequire(nope)\n";
        let (line, _) = parse_requires(text).unwrap_err();
        assert_eq!(line, 7, "a long comment and an escaped newline both count");
    }

    #[test]
    fn an_unterminated_or_empty_name_is_refused() {
        assert!(parse_requires("require('')").is_err());
        assert!(parse_requires("require('unclosed").is_err());
    }
}
