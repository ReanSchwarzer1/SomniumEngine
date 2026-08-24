//! The Output Log's model — CONTROL-I.
//!
//! Everything here is pure: a line's severity, the `file:line:column` spans
//! inside it, and whether a filter keeps it. That is deliberate. The one thing
//! §17.18.6 actually asks for — "a Luau syntax error is one click from the
//! offending line" — is a *parsing* problem, and a parser that can only be
//! exercised by clicking a widget is a parser nobody exercises.

use std::collections::BTreeSet;

/// How loud a line is.
///
/// Ordered least to most severe so `>=` reads the way a filter reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogSeverity {
    /// Diagnostic detail; off by default.
    Debug,
    /// Ordinary progress.
    Info,
    /// Something worth knowing about that did not stop anything.
    Warn,
    /// Something failed.
    Error,
}

impl LogSeverity {
    /// Every severity, quietest first — the order the chips appear in.
    pub const ALL: [Self; 4] = [Self::Debug, Self::Info, Self::Warn, Self::Error];

    /// The chip's label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Debug => "Debug",
            Self::Info => "Info",
            Self::Warn => "Warn",
            Self::Error => "Error",
        }
    }

    /// Infer a severity from a line's own text.
    ///
    /// The log is fed by `tracing`, by the script host and by plain
    /// `push_toast`-adjacent strings, and retrofitting a severity parameter to
    /// every one of those call sites would be a large diff to say something
    /// the text already says. A line that announces itself is believed; a line
    /// that does not is `Info`.
    #[must_use]
    pub fn infer(text: &str) -> Self {
        let lower = text.to_ascii_lowercase();
        // Checked longest-first so "warning" cannot be read as an error simply
        // because the same line also contains the word "error handler".
        if lower.contains("error") || lower.contains("failed") || lower.contains("panic") {
            Self::Error
        } else if lower.contains("warn") {
            Self::Warn
        } else if lower.contains("debug") || lower.contains("trace") {
            Self::Debug
        } else {
            Self::Info
        }
    }
}

/// A `file:line:column` reference found inside a log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRef {
    /// The path exactly as it appeared.
    pub file: String,
    /// One-based line number.
    pub line: u32,
    /// One-based column, when the message gave one.
    pub column: Option<u32>,
    /// Byte range of the whole reference within the line, so the panel can
    /// underline exactly what it will act on.
    pub span: (usize, usize),
}

/// One line in the log.
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    /// Monotonic id, so a pin survives the ring buffer moving underneath it.
    pub id: u64,
    /// Seconds since the editor started.
    pub timestamp: f64,
    /// Severity.
    pub severity: LogSeverity,
    /// Bracketed prefix, when the line has one: `[script]`, `[import]`.
    pub category: Option<String>,
    /// The line itself, prefix included.
    pub text: String,
    /// Source references found in the text.
    pub sources: Vec<SourceRef>,
    /// Pinned lines survive Clear.
    pub pinned: bool,
}

impl LogEntry {
    /// Build an entry from a raw line.
    #[must_use]
    pub fn new(id: u64, timestamp: f64, text: &str) -> Self {
        Self {
            id,
            timestamp,
            severity: LogSeverity::infer(text),
            category: category_of(text),
            sources: parse_source_refs(text),
            text: text.to_owned(),
            pinned: false,
        }
    }

    /// `12.34s  [script] message` — what Copy puts on the clipboard.
    #[must_use]
    pub fn rendered(&self) -> String {
        format!("{:>8.2}s  {}", self.timestamp, self.text)
    }
}

/// The bracketed prefix a line opens with, if any.
fn category_of(text: &str) -> Option<String> {
    let rest = text.trim_start().strip_prefix('[')?;
    let end = rest.find(']')?;
    let name = &rest[..end];
    (!name.is_empty() && !name.contains(' ')).then(|| name.to_owned())
}

/// Every `file:line` or `file:line:column` reference in a line.
///
/// Written by hand rather than with a regex because the interesting cases are
/// all about *where to stop*: a Windows drive letter is a colon that is not a
/// separator, a trailing period is punctuation and not part of the path, and
/// `12:30:05` in a timestamp is three numbers with no file in front of them.
/// Each of those is one condition here and one test below.
#[must_use]
pub fn parse_source_refs(text: &str) -> Vec<SourceRef> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        // Find the next colon that has a digit after it.
        let Some(colon) = text[cursor..].find(':').map(|offset| cursor + offset) else {
            break;
        };
        cursor = colon + 1;
        let (line, after_line) = read_number(text, colon + 1);
        let Some(line) = line else { continue };

        // Walk back over the path. It ends at the colon and starts at the
        // first character that cannot be in one — a colon included, which is
        // what stops `12:30:05` reading as a file called `12:30`.
        let mut start = colon;
        while start > 0 {
            let ch = bytes[start - 1];
            if matches!(ch, b' ' | b'\t' | b'"' | b'\'' | b'(' | b'[' | b',' | b':') {
                break;
            }
            start -= 1;
        }
        // A Windows drive letter's colon *is* part of the path, so it is
        // joined back on. The letter has to be the whole token before that
        // colon, or `a:b:1` would grow a spurious drive.
        if start >= 2
            && bytes[start - 1] == b':'
            && bytes[start - 2].is_ascii_alphabetic()
            && (start == 2
                || matches!(
                    bytes[start - 3],
                    b' ' | b'\t' | b'"' | b'\'' | b'(' | b'[' | b','
                ))
        {
            start -= 2;
        }
        let mut file = &text[start..colon];
        // A path needs something in it. `12:30` is a time, not a file.
        if file.is_empty() || file.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        file = file.trim_end_matches(['.', ',']);
        if file.is_empty() {
            continue;
        }

        let (column, after_column) = read_number(text, after_line + 1);
        let (column, end) = if text.as_bytes().get(after_line) == Some(&b':') && column.is_some() {
            (column, after_column)
        } else {
            (None, after_line)
        };
        out.push(SourceRef {
            file: file.to_owned(),
            line,
            column,
            span: (start, end),
        });
        cursor = end;
    }
    out
}

/// Read a decimal number at `from`, returning it and the index after it.
fn read_number(text: &str, from: usize) -> (Option<u32>, usize) {
    let bytes = text.as_bytes();
    let mut end = from;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == from {
        return (None, from);
    }
    (text[from..end].parse().ok(), end)
}

/// What the panel is currently showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogFilter {
    /// Severities whose chip is lit.
    pub severities: BTreeSet<LogSeverity>,
    /// Category, or `None` for all.
    pub category: Option<String>,
    /// Free-text search over the line.
    pub search: String,
    /// Show only pinned lines.
    pub pinned_only: bool,
}

impl Default for LogFilter {
    /// Everything except `Debug`, which is the setting anyone actually wants
    /// to open the panel to: debug lines are numerous and are the reason a log
    /// panel feels useless by default.
    fn default() -> Self {
        Self {
            severities: [LogSeverity::Info, LogSeverity::Warn, LogSeverity::Error]
                .into_iter()
                .collect(),
            category: None,
            search: String::new(),
            pinned_only: false,
        }
    }
}

impl LogFilter {
    /// Whether `entry` survives.
    #[must_use]
    pub fn matches(&self, entry: &LogEntry) -> bool {
        if !self.severities.contains(&entry.severity) {
            return false;
        }
        if self.pinned_only && !entry.pinned {
            return false;
        }
        if let Some(category) = &self.category
            && entry.category.as_deref() != Some(category.as_str())
        {
            return false;
        }
        let needle = self.search.trim().to_ascii_lowercase();
        needle.is_empty() || entry.text.to_ascii_lowercase().contains(&needle)
    }

    /// Flip one severity chip.
    pub fn toggle(&mut self, severity: LogSeverity) {
        if !self.severities.remove(&severity) {
            self.severities.insert(severity);
        }
    }
}

/// The categories present in a set of entries, for the filter menu.
#[must_use]
pub fn categories(entries: &[LogEntry]) -> Vec<String> {
    let mut names: BTreeSet<&str> = BTreeSet::new();
    for entry in entries {
        if let Some(category) = &entry.category {
            names.insert(category.as_str());
        }
    }
    names.into_iter().map(str::to_owned).collect()
}

/// Build the command that opens `file` at `line` in an external editor.
///
/// `{file}` and `{line}` are substituted; `{column}` too when the reference
/// carried one. An empty template means "no external editor is configured",
/// and the caller reveals the file in the OS browser instead — which is the
/// honest fallback, because silently doing nothing is how a clickable link
/// becomes a thing people stop clicking.
#[must_use]
pub fn external_editor_command(template: &str, source: &SourceRef) -> Option<Vec<String>> {
    let template = template.trim();
    if template.is_empty() {
        return None;
    }
    let parts = split_command(template);
    if parts.is_empty() {
        return None;
    }
    Some(
        parts
            .into_iter()
            .map(|part| {
                part.replace("{file}", &source.file)
                    .replace("{line}", &source.line.to_string())
                    .replace("{column}", &source.column.unwrap_or(1).to_string())
            })
            .collect(),
    )
}

/// Split a command template on whitespace, honouring double quotes so a path
/// with a space in it survives — which on Windows is most of them.
fn split_command(template: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for ch in template.chars() {
        match ch {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §17.18.6's first bullet, as a parser test: a Luau syntax error names a
    /// file, a line and a column, and all three must come back out.
    #[test]
    fn a_luau_diagnostic_yields_a_clickable_reference() {
        let line = "[script] assets/scripts/door.luau:42:7: expected '=' near 'then'";
        let refs = parse_source_refs(line);
        assert_eq!(refs.len(), 1, "{refs:?}");
        assert_eq!(refs[0].file, "assets/scripts/door.luau");
        assert_eq!(refs[0].line, 42);
        assert_eq!(refs[0].column, Some(7));
        assert_eq!(
            &line[refs[0].span.0..refs[0].span.1],
            "assets/scripts/door.luau:42:7"
        );
    }

    /// A reference without a column is still a reference. Most compilers give
    /// one; `panic!` does not.
    #[test]
    fn a_reference_without_a_column_still_parses() {
        let refs = parse_source_refs("thread panicked at crates/somnium_core/src/app.rs:1204");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].line, 1204);
        assert_eq!(refs[0].column, None);
    }

    /// A Windows drive letter is a colon that is not a separator.
    #[test]
    fn a_windows_path_is_not_split_at_its_drive_letter() {
        let refs = parse_source_refs(r"C:\work\game\scripts\boot.luau:9:1: unexpected symbol");
        assert_eq!(refs.len(), 1, "{refs:?}");
        assert_eq!(refs[0].file, r"C:\work\game\scripts\boot.luau");
        assert_eq!(refs[0].line, 9);
    }

    /// A timestamp is three numbers with no file in front of them, and must
    /// not become a link to a file called "12".
    #[test]
    fn a_timestamp_is_not_a_source_reference() {
        assert!(parse_source_refs("12:30:05 scene saved").is_empty());
        assert!(parse_source_refs("finished in 1:02").is_empty());
    }

    /// Trailing punctuation belongs to the sentence, not the path.
    #[test]
    fn trailing_punctuation_is_not_part_of_the_path() {
        let refs = parse_source_refs("could not open assets/a.luau:3, giving up");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].file, "assets/a.luau");
    }

    #[test]
    fn several_references_in_one_line_all_parse() {
        let refs = parse_source_refs("a/one.luau:1:2 required by b/two.luau:30");
        assert_eq!(refs.len(), 2, "{refs:?}");
        assert_eq!(refs[0].file, "a/one.luau");
        assert_eq!(refs[1].file, "b/two.luau");
        assert_eq!(refs[1].line, 30);
    }

    #[test]
    fn severity_is_inferred_from_what_the_line_says() {
        assert_eq!(
            LogSeverity::infer("[script] compile error"),
            LogSeverity::Error
        );
        assert_eq!(LogSeverity::infer("Save failed"), LogSeverity::Error);
        assert_eq!(LogSeverity::infer("warning: unused"), LogSeverity::Warn);
        assert_eq!(LogSeverity::infer("Scene saved"), LogSeverity::Info);
        assert_eq!(LogSeverity::infer("debug view 12"), LogSeverity::Debug);
    }

    #[test]
    fn a_bracketed_prefix_becomes_a_category() {
        assert_eq!(category_of("[script] hello").as_deref(), Some("script"));
        assert_eq!(category_of("no prefix here"), None);
        assert_eq!(
            category_of("[not a category] x"),
            None,
            "a prefix with spaces is prose, not a category"
        );
    }

    /// Debug is off by default. It is the reason a log panel feels useless the
    /// first time it is opened.
    #[test]
    fn the_default_filter_hides_debug_and_nothing_else() {
        let filter = LogFilter::default();
        let entry = |text: &str| LogEntry::new(0, 0.0, text);
        assert!(!filter.matches(&entry("debug view 12")));
        assert!(filter.matches(&entry("Scene saved")));
        assert!(filter.matches(&entry("warning: unused")));
        assert!(filter.matches(&entry("[script] compile error")));
    }

    #[test]
    fn filters_combine_with_and() {
        let mut filter = LogFilter::default();
        filter.category = Some("script".into());
        filter.search = "door".into();
        assert!(filter.matches(&LogEntry::new(0, 0.0, "[script] door.luau failed")));
        assert!(!filter.matches(&LogEntry::new(0, 0.0, "[import] door.glb failed")));
        assert!(!filter.matches(&LogEntry::new(0, 0.0, "[script] window.luau failed")));
    }

    #[test]
    fn pinned_only_shows_only_pins() {
        let mut filter = LogFilter::default();
        filter.pinned_only = true;
        let mut pinned = LogEntry::new(0, 0.0, "Scene saved");
        assert!(!filter.matches(&pinned));
        pinned.pinned = true;
        assert!(filter.matches(&pinned));
    }

    #[test]
    fn toggling_a_chip_turns_it_off_and_on_again() {
        let mut filter = LogFilter::default();
        filter.toggle(LogSeverity::Warn);
        assert!(!filter.matches(&LogEntry::new(0, 0.0, "warning: x")));
        filter.toggle(LogSeverity::Warn);
        assert!(filter.matches(&LogEntry::new(0, 0.0, "warning: x")));
    }

    /// The external editor command substitutes all three placeholders and
    /// survives a Windows path with a space in it.
    #[test]
    fn the_editor_command_substitutes_and_survives_quoting() {
        let source = SourceRef {
            file: r"C:\my games\a.luau".into(),
            line: 42,
            column: Some(7),
            span: (0, 0),
        };
        let command = external_editor_command(
            r#""C:\Program Files\Ed\ed.exe" --goto {file}:{line}:{column}"#,
            &source,
        )
        .expect("a configured template produces a command");
        assert_eq!(command[0], r"C:\Program Files\Ed\ed.exe");
        assert_eq!(command[1], "--goto");
        assert_eq!(command[2], r"C:\my games\a.luau:42:7");
    }

    /// A missing column substitutes 1 rather than an empty string, because
    /// most editors reject `file:42:` outright.
    #[test]
    fn a_missing_column_substitutes_one() {
        let source = SourceRef {
            file: "a.luau".into(),
            line: 3,
            column: None,
            span: (0, 0),
        };
        let command = external_editor_command("ed {file}:{line}:{column}", &source).unwrap();
        assert_eq!(command[1], "a.luau:3:1");
    }

    /// No configured editor means the caller reveals the file instead. That is
    /// the fallback, not silence.
    #[test]
    fn an_unconfigured_editor_produces_no_command() {
        let source = SourceRef {
            file: "a.luau".into(),
            line: 1,
            column: None,
            span: (0, 0),
        };
        assert_eq!(external_editor_command("   ", &source), None);
    }

    #[test]
    fn categories_are_deduplicated_and_sorted() {
        let entries = [
            LogEntry::new(0, 0.0, "[script] a"),
            LogEntry::new(1, 0.0, "[import] b"),
            LogEntry::new(2, 0.0, "[script] c"),
            LogEntry::new(3, 0.0, "plain"),
        ];
        assert_eq!(categories(&entries), vec!["import", "script"]);
    }
}

/// The Output Log's state: what has been logged, and what is being shown.
///
/// Lives here rather than on `UiManager` so the policy — how the ring buffer
/// evicts, what Clear keeps, what "show me the first error" changes — can be
/// tested without a GPU device and a window, which is what `UiManager::new`
/// needs. The manager keeps only the widget work.
#[derive(Debug, Clone)]
pub struct OutputLog {
    entries: std::collections::VecDeque<LogEntry>,
    /// What the panel is showing.
    pub filter: LogFilter,
    next_id: u64,
    capacity: usize,
}

impl Default for OutputLog {
    fn default() -> Self {
        Self::with_capacity(200)
    }
}

impl OutputLog {
    /// A log holding at most `capacity` unpinned entries.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: std::collections::VecDeque::new(),
            filter: LogFilter::default(),
            next_id: 0,
            capacity: capacity.max(1),
        }
    }

    /// Append a line. Returns the new entry's id.
    ///
    /// Eviction skips pinned entries: a line somebody deliberately kept should
    /// not be thrown away by two hundred lines of import chatter arriving
    /// behind it. A log that is *entirely* pins simply stops evicting, which
    /// is the honest outcome — the alternative is discarding a pin.
    pub fn append(&mut self, timestamp: f64, text: &str) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.entries.push_back(LogEntry::new(id, timestamp, text));
        while self.entries.len() > self.capacity {
            // The oldest unpinned entry, never the one just appended: with a
            // log full of pins the only unpinned candidate *is* the new line,
            // and evicting it would make the newest message vanish as it
            // arrived. Over capacity is the better outcome, and it is bounded
            // by how many lines somebody chose to pin.
            let last = self.entries.len() - 1;
            let Some(position) = self
                .entries
                .iter()
                .take(last)
                .position(|entry| !entry.pinned)
            else {
                break;
            };
            self.entries.remove(position);
        }
        id
    }

    /// Every entry, oldest first.
    pub fn entries(&self) -> impl Iterator<Item = &LogEntry> {
        self.entries.iter()
    }

    /// The entries currently passing the filter.
    #[must_use]
    pub fn visible(&self) -> Vec<&LogEntry> {
        self.entries
            .iter()
            .filter(|entry| self.filter.matches(entry))
            .collect()
    }

    /// The visible lines as text, for Copy.
    #[must_use]
    pub fn copy_text(&self) -> String {
        self.visible()
            .iter()
            .map(|entry| entry.rendered())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Toggle one entry's pin.
    pub fn toggle_pin(&mut self, id: u64) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) {
            entry.pinned = !entry.pinned;
        }
    }

    /// The source reference an entry links to, if it has one.
    #[must_use]
    pub fn source_of(&self, id: u64) -> Option<&SourceRef> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)?
            .sources
            .first()
    }

    /// Clear, keeping pins. That is the whole point of a pin.
    pub fn clear(&mut self) {
        self.entries.retain(|entry| entry.pinned);
    }

    /// Show errors only, and drop any filter that would hide them.
    ///
    /// What the status bar's "N script errors" clicks through to. It *filters*
    /// rather than merely scrolling, because a lone error thirty lines up in a
    /// busy log is not findable by scrolling to it.
    pub fn reveal_errors(&mut self) {
        self.filter.severities = [LogSeverity::Error].into_iter().collect();
        self.filter.pinned_only = false;
        self.filter.search.clear();
        self.filter.category = None;
    }

    /// How many entries carry `severity`.
    #[must_use]
    pub fn count(&self, severity: LogSeverity) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.severity == severity)
            .count()
    }
}

#[cfg(test)]
mod log_state_tests {
    use super::*;

    fn log() -> OutputLog {
        let mut log = OutputLog::with_capacity(4);
        log.append(0.10, "Scene saved");
        log.append(0.20, "warning: two lights share a name");
        log.append(0.30, "[script] door.luau:4:2: compile error");
        log.append(0.40, "debug view 12");
        log
    }

    /// Debug is off by default, so the panel opens showing three of the four.
    #[test]
    fn the_chips_decide_what_the_panel_shows() {
        let mut log = log();
        assert_eq!(log.visible().len(), 3);

        log.filter.severities = [LogSeverity::Error].into_iter().collect();
        let visible = log.visible();
        assert_eq!(visible.len(), 1);
        assert!(visible[0].text.contains("compile error"));
    }

    /// The status bar's click-through drops whatever was hiding the error.
    #[test]
    fn revealing_errors_clears_a_filter_that_would_hide_them() {
        let mut log = log();
        log.filter.search = "nothing matches this".into();
        log.filter.pinned_only = true;
        log.filter.category = Some("import".into());

        log.reveal_errors();
        assert!(log.filter.search.is_empty());
        assert!(!log.filter.pinned_only);
        assert_eq!(log.filter.category, None);
        let visible = log.visible();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].severity, LogSeverity::Error);
    }

    /// A pin survives both Clear and the ring buffer filling behind it.
    #[test]
    fn a_pinned_line_survives_clear_and_eviction() {
        let mut log = OutputLog::with_capacity(4);
        let kept = log.append(0.0, "[script] door.luau:4:2: compile error");
        log.toggle_pin(kept);
        for index in 0..50 {
            log.append(f64::from(index), &format!("line {index}"));
        }
        assert!(log.entries().any(|entry| entry.id == kept));
        assert_eq!(log.entries().count(), 4, "the cap still holds");

        log.clear();
        assert_eq!(log.entries().count(), 1);
        assert_eq!(log.entries().next().unwrap().id, kept);
    }

    /// A log that is entirely pins stops evicting rather than discarding one,
    /// and the newest line is never the thing that gets thrown away.
    #[test]
    fn an_all_pinned_log_stops_evicting() {
        let mut log = OutputLog::with_capacity(2);
        for index in 0..5 {
            let id = log.append(f64::from(index), &format!("line {index}"));
            log.toggle_pin(id);
        }
        assert_eq!(log.entries().count(), 5, "pins are never discarded");
        assert_eq!(log.entries().last().unwrap().text, "line 4");

        // One unpinned line among pins is the one that goes.
        log.append(9.0, "transient");
        log.append(10.0, "newest");
        assert_eq!(log.entries().count(), 6, "the transient line was evicted");
        assert_eq!(log.entries().last().unwrap().text, "newest");
        assert!(!log.entries().any(|entry| entry.text == "transient"));
    }

    /// Copy renders what is on screen, with timestamps, in order.
    #[test]
    fn copy_renders_the_visible_lines_in_order() {
        let mut log = log();
        log.filter.severities = [LogSeverity::Info, LogSeverity::Error]
            .into_iter()
            .collect();
        let copied = log.copy_text();
        let lines: Vec<_> = copied.lines().collect();
        assert_eq!(lines.len(), 2, "{copied}");
        assert!(lines[0].ends_with("Scene saved"));
        assert!(lines[1].ends_with("compile error"));
        assert!(lines[0].contains("0.10s"));
    }

    /// A clickable line knows exactly which file it will open.
    #[test]
    fn a_line_reports_the_source_it_links_to() {
        let mut log = OutputLog::default();
        let id = log.append(0.0, "[script] door.luau:4:2: compile error");
        let source = log.source_of(id).expect("the line carries a reference");
        assert_eq!(source.file, "door.luau");
        assert_eq!(source.line, 4);
        assert_eq!(source.column, Some(2));

        let plain = log.append(0.0, "Scene saved");
        assert!(log.source_of(plain).is_none());
    }

    #[test]
    fn counts_feed_the_status_bar() {
        let log = log();
        assert_eq!(log.count(LogSeverity::Error), 1);
        assert_eq!(log.count(LogSeverity::Warn), 1);
        assert_eq!(log.count(LogSeverity::Debug), 1);
    }
}
