//! A game's story content, loaded, compiled and checked as a whole.
//!
//! Each asset checks itself; this checks what only the whole can: that a line's speaker is in the cast and has that
//! expression, that `start_quest('varga_job')` names a quest that exists, that `done('varga_job', 'door')` names
//! one of its objectives, that a call reaches a real dialogue entry, that a `{placeholder}` is a declared fact. The
//! editor, the MCP lint and the game all build through here, so they all agree on what is broken.
//!
//! Files are found by extension under one root: `*.somstory` (exactly one), `*.somdialogue`, `*.somquest`,
//! `*.sombarks`, `*.somnotes`.

use crate::barks::{BarkSet, BarkSetAsset};
use crate::dialogue::{Dialogue, DialogueAsset, NodeDef};
use crate::expr::{Context, Effect, Effects, Expr, Func};
use crate::journal::{NoteDef, NotesAsset};
use crate::manifest::StoryManifest;
use crate::problem::Problem;
use crate::quest::{Quest, QuestAsset};
use crate::text::LocalText;
use crate::value::{Schema, Value};
use serde::Serialize;
use somnium_i18n::Table;
use std::collections::BTreeMap;
use std::path::Path;

/// How bad an issue is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

/// One finding, with the file it is in.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Issue {
    pub severity: Severity,
    /// The file, as given to [`StorySources`] (relative to the story root when read from disk).
    pub file: String,
    /// `dialogue mireille_intro`, `quest varga_job`, `manifest`.
    pub asset: String,
    /// Where in the asset: a node, a stage, an objective.
    pub at: String,
    pub message: String,
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{kind}: {} ({})", self.message, self.file)?;
        if !self.at.is_empty() {
            write!(f, " at {}", self.at)?;
        }
        Ok(())
    }
}

/// Story files before compiling.
#[derive(Clone, Debug, Default)]
pub struct StorySources {
    pub manifest: Option<(String, StoryManifest)>,
    pub dialogues: Vec<(String, DialogueAsset)>,
    pub quests: Vec<(String, QuestAsset)>,
    pub barks: Vec<(String, BarkSetAsset)>,
    pub notes: Vec<(String, NotesAsset)>,
    /// Files that could not be read or parsed.
    pub unreadable: Vec<Issue>,
}

/// The extensions story files use, and what they hold.
pub const EXTENSIONS: [(&str, &str); 5] = [
    ("somstory", "manifest"),
    ("somdialogue", "dialogue"),
    ("somquest", "quest"),
    ("sombarks", "barks"),
    ("somnotes", "notes"),
];

impl StorySources {
    /// Add one file by its extension. Unknown extensions are ignored.
    pub fn add_file(&mut self, file: &str, text: &str) {
        let extension = Path::new(file)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let unreadable = |message: String| Issue {
            severity: Severity::Error,
            file: file.to_owned(),
            asset: String::new(),
            at: String::new(),
            message,
        };
        match extension {
            "somstory" => match StoryManifest::from_json(text) {
                Ok(manifest) if self.manifest.is_none() => {
                    self.manifest = Some((file.into(), manifest))
                }
                Ok(_) => self.unreadable.push(unreadable(
                    "a second story manifest; a game has exactly one".into(),
                )),
                Err(e) => self.unreadable.push(unreadable(e)),
            },
            "somdialogue" => match DialogueAsset::from_json(text) {
                Ok(asset) => self.dialogues.push((file.into(), asset)),
                Err(e) => self.unreadable.push(unreadable(e)),
            },
            "somquest" => match QuestAsset::from_json(text) {
                Ok(asset) => self.quests.push((file.into(), asset)),
                Err(e) => self.unreadable.push(unreadable(e)),
            },
            "sombarks" => match BarkSetAsset::from_json(text) {
                Ok(asset) => self.barks.push((file.into(), asset)),
                Err(e) => self.unreadable.push(unreadable(e)),
            },
            "somnotes" => match NotesAsset::from_json(text) {
                Ok(asset) => self.notes.push((file.into(), asset)),
                Err(e) => self.unreadable.push(unreadable(e)),
            },
            _ => {}
        }
    }

    /// Every story file under `root`, in sorted path order. Paths are recorded relative to `root`, with `/`.
    #[must_use]
    pub fn read_dir(root: &Path) -> Self {
        let mut files = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        let mut sources = Self::default();
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                sources.unreadable.push(Issue {
                    severity: Severity::Error,
                    file: dir.display().to_string(),
                    asset: String::new(),
                    at: String::new(),
                    message: "cannot read this folder".into(),
                });
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    stack.push(path);
                } else if path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| EXTENSIONS.iter().any(|(x, _)| *x == e))
                {
                    files.push(path);
                }
            }
        }
        files.sort();
        for path in files {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            match std::fs::read_to_string(&path) {
                Ok(text) => sources.add_file(&relative, &text),
                Err(e) => sources.unreadable.push(Issue {
                    severity: Severity::Error,
                    file: relative,
                    asset: String::new(),
                    at: String::new(),
                    message: e.to_string(),
                }),
            }
        }
        sources
    }
}

/// Compiled, cross-checked story content. Immutable once built; a running [`crate::Story`] shares it.
#[derive(Clone, Debug)]
pub struct StoryDatabase {
    pub manifest: StoryManifest,
    pub schema: Schema,
    dialogues: BTreeMap<String, Dialogue>,
    quests: BTreeMap<String, Quest>,
    bark_sets: BTreeMap<String, BarkSet>,
    notes: BTreeMap<String, NoteDef>,
    voice: BTreeMap<String, crate::journal::VoiceDef>,
    files: BTreeMap<String, String>,
}

impl StoryDatabase {
    /// Build, refusing on any error. `Ok` carries the warnings.
    pub fn build(sources: StorySources) -> Result<(Self, Vec<Issue>), Vec<Issue>> {
        let (database, issues) = Self::build_lenient(sources);
        match database {
            Some(database) if !issues.iter().any(|i| i.severity == Severity::Error) => {
                Ok((database, issues))
            }
            _ => Err(issues),
        }
    }

    /// Build everything that compiles and report the rest. `None` only when there is no usable manifest.
    #[must_use]
    pub fn build_lenient(sources: StorySources) -> (Option<Self>, Vec<Issue>) {
        let mut issues = sources.unreadable;
        let Some((manifest_file, mut manifest)) = sources.manifest else {
            issues.push(Issue {
                severity: Severity::Error,
                file: String::new(),
                asset: "manifest".into(),
                at: String::new(),
                message: "no story manifest (*.somstory) was found".into(),
            });
            return (None, issues);
        };
        let schema = match manifest.compile() {
            Ok(schema) => schema,
            Err(errors) => {
                issues.extend(errors.into_iter().map(|message| Issue {
                    severity: Severity::Error,
                    file: manifest_file.clone(),
                    asset: "manifest".into(),
                    at: String::new(),
                    message,
                }));
                return (None, issues);
            }
        };
        for speaker in &mut manifest.cast {
            let id = speaker.id.clone();
            speaker.name.assign_key(|| format!("cast.{id}.name"));
        }
        let mut database = Self {
            manifest,
            schema,
            dialogues: BTreeMap::new(),
            quests: BTreeMap::new(),
            bark_sets: BTreeMap::new(),
            notes: BTreeMap::new(),
            voice: BTreeMap::new(),
            files: BTreeMap::new(),
        };
        let manifest = database.manifest.clone();
        let ctx = Context::new(&database.schema, &manifest.commands);
        let mut report = |file: &str, asset: String, problems: Vec<Problem>| {
            issues.extend(problems.into_iter().map(|p| Issue {
                severity: if p.warning {
                    Severity::Warning
                } else {
                    Severity::Error
                },
                file: file.to_owned(),
                asset: asset.clone(),
                at: p.at,
                message: p.message,
            }));
        };
        let claim =
            |files: &mut BTreeMap<String, String>, key: String, file: &str| -> Option<String> {
                match files.get(&key) {
                    Some(other) => Some(format!("{key} is also defined in {other}")),
                    None => {
                        files.insert(key, file.to_owned());
                        None
                    }
                }
            };
        for (file, asset) in sources.dialogues {
            let name = format!("dialogue {}", asset.id);
            if let Some(clash) = claim(&mut database.files, name.clone(), &file) {
                report(&file, name, vec![Problem::error("", clash)]);
                continue;
            }
            match Dialogue::compile(asset, ctx) {
                Ok(dialogue) => {
                    database
                        .dialogues
                        .insert(dialogue.id().to_owned(), dialogue);
                }
                Err(problems) => report(&file, name, problems),
            }
        }
        // A node another dialogue jumps or calls into is reachable, which no single file can know.
        let mut entered_from_outside = std::collections::BTreeSet::new();
        for dialogue in database.dialogues.values() {
            for node in dialogue.nodes() {
                for (_, targets) in node.def.links() {
                    for target in targets {
                        if let Some((other, node)) = target.split_once('/') {
                            entered_from_outside.insert((other.to_owned(), node.to_owned()));
                        }
                    }
                }
                if let NodeDef::Call(call) = &node.def
                    && let Some(entry) = database
                        .dialogues
                        .get(&call.dialogue)
                        .and_then(|d| d.asset.entry_node(&call.entry))
                {
                    entered_from_outside.insert((call.dialogue.clone(), entry.to_owned()));
                }
            }
        }
        for dialogue in database.dialogues.values_mut() {
            let id = dialogue.id().to_owned();
            let file = database
                .files
                .get(&format!("dialogue {id}"))
                .cloned()
                .unwrap_or_default();
            let mut warnings = std::mem::take(&mut dialogue.warnings);
            if warnings
                .iter()
                .any(|w| w.message.ends_with("can never be reached"))
            {
                let mut reachable = dialogue.asset.clone();
                for (other, node) in &entered_from_outside {
                    if *other == id {
                        reachable
                            .entries
                            .insert(format!("from outside {node}"), node.clone());
                    }
                }
                let still: std::collections::BTreeSet<String> =
                    reachable.unreachable().into_iter().collect();
                warnings.retain(|w| {
                    !w.message.ends_with("can never be reached") || still.contains(&w.at)
                });
            }
            report(&file, format!("dialogue {id}"), warnings);
        }
        for (file, asset) in sources.quests {
            let name = format!("quest {}", asset.id);
            if let Some(clash) = claim(&mut database.files, name.clone(), &file) {
                report(&file, name, vec![Problem::error("", clash)]);
                continue;
            }
            match Quest::compile(asset, ctx) {
                Ok(mut quest) => {
                    report(&file, name, std::mem::take(&mut quest.warnings));
                    database.quests.insert(quest.id().to_owned(), quest);
                }
                Err(problems) => report(&file, name, problems),
            }
        }
        for (file, asset) in sources.barks {
            let name = format!("barks {}", asset.id);
            if let Some(clash) = claim(&mut database.files, name.clone(), &file) {
                report(&file, name, vec![Problem::error("", clash)]);
                continue;
            }
            match BarkSet::compile(asset, ctx) {
                Ok(set) => {
                    database.bark_sets.insert(set.id.clone(), set);
                }
                Err(problems) => report(&file, name, problems),
            }
        }
        for (file, mut asset) in sources.notes {
            let name = format!("notes {}", asset.id);
            let mut problems = asset.structural_problems();
            asset.assign_keys();
            for note in asset.notes {
                if let Some(clash) = claim(&mut database.files, format!("note {}", note.id), &file)
                {
                    problems.push(Problem::error(&note.id, clash));
                } else {
                    database.notes.insert(note.id.clone(), note);
                }
            }
            for line in asset.voice {
                if let Some(clash) = claim(&mut database.files, format!("voice {}", line.id), &file)
                {
                    problems.push(Problem::error(&line.id, clash));
                } else {
                    database.voice.insert(line.id.clone(), line);
                }
            }
            report(&file, name, problems);
        }
        let cross = database.cross_check();
        issues.extend(cross);
        issues.sort();
        issues.dedup();
        (Some(database), issues)
    }

    /// Load every story file under `root` and build strictly.
    pub fn load_dir(root: &Path) -> Result<(Self, Vec<Issue>), Vec<Issue>> {
        Self::build(StorySources::read_dir(root))
    }

    #[must_use]
    pub fn context(&self) -> Context<'_> {
        Context::new(&self.schema, &self.manifest.commands)
    }

    #[must_use]
    pub fn dialogue(&self, id: &str) -> Option<&Dialogue> {
        self.dialogues.get(id)
    }

    pub fn dialogues(&self) -> impl Iterator<Item = &Dialogue> {
        self.dialogues.values()
    }

    #[must_use]
    pub fn quest(&self, id: &str) -> Option<&Quest> {
        self.quests.get(id)
    }

    /// Quests in id order: the order the runtime evaluates them in.
    pub fn quests(&self) -> impl Iterator<Item = &Quest> {
        self.quests.values()
    }

    pub fn bark_sets(&self) -> impl Iterator<Item = &BarkSet> {
        self.bark_sets.values()
    }

    #[must_use]
    pub fn note(&self, id: &str) -> Option<&NoteDef> {
        self.notes.get(id)
    }

    pub fn notes(&self) -> impl Iterator<Item = &NoteDef> {
        self.notes.values()
    }

    /// A line the Voice can say.
    #[must_use]
    pub fn voice_line(&self, id: &str) -> Option<&crate::journal::VoiceDef> {
        self.voice.get(id)
    }

    pub fn voice_lines(&self) -> impl Iterator<Item = &crate::journal::VoiceDef> {
        self.voice.values()
    }

    /// The file an asset came from (`dialogue mireille_intro` -> `dialogue/mireille_intro.somdialogue`).
    #[must_use]
    pub fn file_of(&self, asset: &str) -> Option<&str> {
        self.files.get(asset).map(String::as_str)
    }

    /// Every string in the source language, keyed as the runtime looks them up: the table translators start from.
    #[must_use]
    pub fn strings(&self) -> Table {
        let mut table = Table::new(self.manifest.source_locale.clone());
        let mut add = |text: &LocalText| {
            if !text.is_empty() {
                for (key, value) in text.table_entries() {
                    table.strings.insert(key, value);
                }
            }
        };
        for speaker in &self.manifest.cast {
            add(&speaker.name);
        }
        for dialogue in self.dialogues.values() {
            for node in dialogue.nodes() {
                match &node.def {
                    NodeDef::Line(line) => add(&line.text),
                    NodeDef::Option(option) => {
                        add(&option.text);
                        if let Some(check) = &option.check {
                            add(&check.label);
                        }
                    }
                    _ => {}
                }
            }
        }
        for quest in self.quests.values() {
            add(&quest.asset.title);
            add(&quest.asset.summary);
            for stage in &quest.asset.stages {
                add(&stage.title);
                for line in &stage.journal {
                    add(&line.text);
                }
                for objective in &stage.objectives {
                    add(&objective.text);
                }
            }
        }
        for set in self.bark_sets.values() {
            for bark in &set.barks {
                add(&bark.def.text);
            }
            for chatter in &set.chatter {
                for line in &chatter.def.lines {
                    add(&line.text);
                }
            }
        }
        for note in self.notes.values() {
            add(&note.title);
            add(&note.text);
        }
        for line in self.voice.values() {
            add(&line.heading);
            add(&line.text);
        }
        table
    }

    fn issue(&self, severity: Severity, asset: &str, at: &str, message: String) -> Issue {
        Issue {
            severity,
            file: self.files.get(asset).cloned().unwrap_or_default(),
            asset: asset.to_owned(),
            at: at.to_owned(),
            message,
        }
    }

    /// The checks that need the whole content.
    fn cross_check(&self) -> Vec<Issue> {
        let mut issues = Vec::new();
        let manifest = &self.manifest;
        for dialogue in self.dialogues.values() {
            let asset = format!("dialogue {}", dialogue.id());
            let mut error = |at: &str, message: String| {
                issues.push(self.issue(Severity::Error, &asset, at, message));
            };
            for node in dialogue.nodes() {
                let at = node.def.id();
                match &node.def {
                    NodeDef::Line(line) => {
                        if !line.speaker.is_empty() {
                            match manifest.speaker(&line.speaker) {
                                None => error(
                                    at,
                                    format!("speaker {} is not in the cast", line.speaker),
                                ),
                                Some(speaker) => {
                                    if !line.expression.is_empty()
                                        && !manifest.has_expression(speaker, &line.expression)
                                    {
                                        error(
                                            at,
                                            format!(
                                                "{} has no {} expression",
                                                line.speaker, line.expression
                                            ),
                                        );
                                    }
                                    for inline in inline_expressions(&line.text.source) {
                                        if !manifest.has_expression(speaker, &inline) {
                                            error(
                                                at,
                                                format!(
                                                    "{} has no {inline} expression",
                                                    line.speaker
                                                ),
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        if !line.language.is_empty() && !manifest.has_language(&line.language) {
                            error(at, format!("language {} is not declared", line.language));
                        }
                        if !line.voice.is_empty() && !safe_asset_path(&line.voice) {
                            error(
                                at,
                                "voice must be a project-relative path with / separators".into(),
                            );
                        }
                        self.check_text(&line.text, &mut error, at);
                    }
                    NodeDef::Option(option) => {
                        self.check_text(&option.text, &mut error, at);
                    }
                    NodeDef::Call(call) => match self.dialogues.get(&call.dialogue) {
                        None => error(
                            at,
                            format!("calls dialogue {}, which does not exist", call.dialogue),
                        ),
                        Some(target) => {
                            if target.asset.entry_node(&call.entry).is_none() {
                                error(
                                    at,
                                    format!(
                                        "{} has no entry or node {}",
                                        call.dialogue, call.entry
                                    ),
                                );
                            }
                        }
                    },
                    _ => {}
                }
                for (role, targets) in node.def.links() {
                    for target in targets {
                        if let Some((other, node_id)) = target.split_once('/') {
                            let ok = self
                                .dialogues
                                .get(other)
                                .is_some_and(|d| d.node(node_id).is_some());
                            if !ok {
                                error(
                                    at,
                                    format!("{role} leads to {target}, which does not exist"),
                                );
                            }
                        }
                    }
                }
                let refs = Refs::of(Some(dialogue), &node.when, &node.effects);
                for message in self.check_refs(&refs) {
                    error(at, message);
                }
            }
        }
        for quest in self.quests.values() {
            let asset = format!("quest {}", quest.id());
            let mut error = |at: &str, message: String| {
                issues.push(self.issue(Severity::Error, &asset, at, message));
            };
            if !quest.asset.giver.is_empty() && manifest.speaker(&quest.asset.giver).is_none() {
                error(
                    "",
                    format!("giver {} is not in the cast", quest.asset.giver),
                );
            }
            let mut refs = Refs::default();
            refs.add_condition(None, quest.start_when.as_ref());
            refs.add_condition(None, Some(&quest.fail_when));
            for effects in [&quest.on_start, &quest.on_complete, &quest.on_fail] {
                refs.add_effects(None, effects);
            }
            for stage in quest.stages() {
                let at = format!("stage {}", stage.def.id);
                for line in &stage.def.journal {
                    if !manifest.has_tier(&line.tier) {
                        error(
                            &at,
                            format!("tier {} is not one of the manifest's", line.tier),
                        );
                    }
                    self.check_text(&line.text, &mut error, &at);
                }
                refs.add_effects(None, &stage.on_enter);
                for (when, _) in &stage.next {
                    refs.add_condition(None, when.as_ref());
                }
                for objective in &stage.objectives {
                    refs.add_condition(None, Some(&objective.done_when));
                    refs.add_condition(None, Some(&objective.fail_when));
                    refs.add_effects(None, &objective.on_done);
                    refs.add_effects(None, &objective.on_fail);
                    self.check_text(&objective.def.text, &mut error, &at);
                }
            }
            for message in self.check_refs(&refs) {
                error("", message);
            }
        }
        for set in self.bark_sets.values() {
            let asset = format!("barks {}", set.id);
            let mut error = |at: &str, message: String| {
                issues.push(self.issue(Severity::Error, &asset, at, message));
            };
            for bark in &set.barks {
                if !bark.def.language.is_empty() && !manifest.has_language(&bark.def.language) {
                    error(
                        &bark.def.id,
                        format!("language {} is not declared", bark.def.language),
                    );
                }
                self.check_text(&bark.def.text, &mut error, &bark.def.id);
                for message in self.check_refs(&Refs::of(None, &bark.when, &bark.effects)) {
                    error(&bark.def.id, message);
                }
            }
            for chatter in &set.chatter {
                for line in &chatter.def.lines {
                    if !line.language.is_empty() && !manifest.has_language(&line.language) {
                        error(
                            &chatter.def.id,
                            format!("language {} is not declared", line.language),
                        );
                    }
                    self.check_text(&line.text, &mut error, &chatter.def.id);
                }
                for message in self.check_refs(&Refs::of(None, &chatter.when, &chatter.effects)) {
                    error(&chatter.def.id, message);
                }
            }
        }
        for note in self.notes.values() {
            if !manifest.has_tier(&note.tier) {
                let asset = format!("note {}", note.id);
                issues.push(self.issue(
                    Severity::Error,
                    &asset,
                    &note.id,
                    format!("tier {} is not one of the manifest's", note.tier),
                ));
            }
        }
        for line in self.voice.values() {
            let asset = format!("voice {}", line.id);
            let mut problems = Vec::new();
            for text in [&line.heading, &line.text] {
                self.check_text(
                    text,
                    &mut |at: &str, m: String| problems.push((at.to_owned(), m)),
                    &line.id,
                );
            }
            if !line.voice.is_empty() && !safe_asset_path(&line.voice) {
                problems.push((
                    line.id.clone(),
                    format!("voice {} is not a content path", line.voice),
                ));
            }
            for (at, message) in problems {
                issues.push(self.issue(Severity::Error, &asset, &at, message));
            }
        }
        issues
    }

    fn check_text(&self, text: &LocalText, error: &mut impl FnMut(&str, String), at: &str) {
        for name in text.placeholders() {
            if name == "n" && !text.plural.is_empty() {
                continue;
            }
            if self.schema.get(&name).is_none() {
                error(at, format!("{{{name}}} is not a declared fact"));
            }
        }
        for fact in [&text.plural, &text.gender] {
            if !fact.is_empty() && self.schema.get(fact).is_none() {
                error(
                    at,
                    format!("{fact} selects a form but is not a declared fact"),
                );
            }
        }
    }

    /// Check the ids story functions and commands name.
    fn check_refs(&self, refs: &Refs) -> Vec<String> {
        let mut messages = Vec::new();
        let quest = |id: &str| self.quests.get(id);
        for (scope, func, args) in &refs.calls {
            match (func, args.as_slice()) {
                (Func::Visited, [target]) => {
                    let found = match target.split_once('/') {
                        Some((d, n)) => self.dialogues.get(d).is_some_and(|d| d.node(n).is_some()),
                        None => scope
                            .as_ref()
                            .and_then(|d| self.dialogues.get(d))
                            .is_some_and(|d| d.node(target).is_some()),
                    };
                    if !found {
                        messages.push(if scope.is_none() && !target.contains('/') {
                            format!("visited('{target}') outside a dialogue needs 'dialogue/node'")
                        } else {
                            format!("visited('{target}') names no node")
                        });
                    }
                }
                (Func::Quest | Func::Stage, [q]) => {
                    if quest(q).is_none() {
                        messages.push(format!("{}('{q}'): no such quest", func.name()));
                    }
                }
                (Func::Done | Func::Failed, [q, o]) => match quest(q) {
                    None => messages.push(format!("{}('{q}', ...): no such quest", func.name())),
                    Some(found) if found.objective(o).is_none() => {
                        messages.push(format!("{q} has no objective {o}"));
                    }
                    Some(_) => {}
                },
                _ => {}
            }
        }
        for (q, stage) in &refs.stage_comparisons {
            if let Some(found) = quest(q)
                && found.stage(stage).is_none()
            {
                messages.push(format!("{q} has no stage {stage}"));
            }
        }
        for (name, args) in &refs.commands {
            match (name.as_str(), args.as_slice()) {
                ("start_quest" | "complete_quest" | "fail_quest" | "reset_quest", [Some(q)]) => {
                    if quest(q).is_none() {
                        messages.push(format!("{name}('{q}'): no such quest"));
                    }
                }
                ("set_stage", [Some(q), Some(s)]) => match quest(q) {
                    None => messages.push(format!("set_stage('{q}', ...): no such quest")),
                    Some(found) if found.stage(s).is_none() => {
                        messages.push(format!("{q} has no stage {s}"));
                    }
                    Some(_) => {}
                },
                ("complete_objective" | "fail_objective", [Some(q), Some(o)]) => match quest(q) {
                    None => messages.push(format!("{name}('{q}', ...): no such quest")),
                    Some(found) if found.objective(o).is_none() => {
                        messages.push(format!("{q} has no objective {o}"));
                    }
                    Some(_) => {}
                },
                ("journal" | "journal_obscure" | "journal_restore", [Some(n)]) => {
                    if !self.notes.contains_key(n.as_str()) {
                        messages.push(format!("{name}('{n}'): no such note"));
                    }
                }
                ("voice", [Some(v)]) => {
                    if !self.voice.contains_key(v.as_str()) {
                        messages.push(format!("voice('{v}'): no such voice line"));
                    }
                }
                _ => {}
            }
        }
        messages
    }
}

/// The ids an asset's expressions name literally.
#[derive(Default)]
struct Refs {
    /// `(dialogue scope, function, literal text arguments)`.
    calls: Vec<(Option<String>, Func, Vec<String>)>,
    /// `stage('q') == 's'`.
    stage_comparisons: Vec<(String, String)>,
    /// `(command, arguments that are literal text)`.
    commands: Vec<(String, Vec<Option<String>>)>,
}

fn literal(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Lit(Value::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

impl Refs {
    fn of(dialogue: Option<&Dialogue>, when: &crate::expr::Condition, effects: &Effects) -> Self {
        let mut refs = Self::default();
        let scope = dialogue.map(|d| d.id());
        refs.add_condition(scope, Some(when));
        refs.add_effects(scope, effects);
        refs
    }

    fn add_expr(&mut self, scope: Option<&str>, expr: &Expr) {
        expr.walk(&mut |e| match e {
            Expr::Call(func, args) => {
                let texts: Option<Vec<String>> = args.iter().map(literal).collect();
                if let Some(texts) = texts {
                    self.calls.push((scope.map(str::to_owned), *func, texts));
                }
            }
            Expr::Compare { left, right, .. } => {
                for (a, b) in [(left, right), (right, left)] {
                    if let (Expr::Call(Func::Stage, args), Some(stage)) = (a.as_ref(), literal(b))
                        && let [quest] = args.as_slice()
                        && let Some(quest) = literal(quest)
                        && !stage.is_empty()
                    {
                        self.stage_comparisons.push((quest, stage));
                    }
                }
            }
            _ => {}
        });
    }

    fn add_condition(&mut self, scope: Option<&str>, condition: Option<&crate::expr::Condition>) {
        if let Some(expr) = condition.and_then(|c| c.expr()) {
            self.add_expr(scope, expr);
        }
    }

    fn add_effects(&mut self, scope: Option<&str>, effects: &Effects) {
        for effect in effects.list() {
            for expr in effect.exprs() {
                self.add_expr(scope, expr);
            }
            if let Effect::Command { name, args } = effect {
                self.commands
                    .push((name.clone(), args.iter().map(literal).collect()));
            }
        }
    }
}

/// `[expr=name]` cues inside a line.
fn inline_expressions(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("[expr=") {
        let after = &rest[start + 6..];
        let Some(end) = after.find(']') else { break };
        found.push(after[..end].trim().to_owned());
        rest = &after[end..];
    }
    found
}

/// A project-relative asset path: no drive, no leading slash, no backslashes, no `.` or `..` parts.
#[must_use]
pub fn safe_asset_path(path: &str) -> bool {
    !path.trim().is_empty()
        && !path.starts_with('/')
        && !path.contains(':')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const MANIFEST: &str = r##"{
        "version": 1, "id": "test_story", "source_locale": "en",
        "facts": [
            {"id": "met.mireille", "type": "bool"},
            {"id": "met.foreman", "type": "bool"},
            {"id": "mireille.affection", "type": "int", "min": -10, "max": 10},
            {"id": "lang.patter", "type": "enum", "values": ["none", "phrases", "conversant", "fluent"]},
            {"id": "skill.mending", "type": "int", "min": 0, "max": 20},
            {"id": "varga.guards_down", "type": "int"},
            {"id": "varga.alarm", "type": "bool"},
            {"id": "lidless", "type": "int"},
            {"id": "gold", "type": "int"},
            {"id": "player.name", "type": "text", "default": "Caelan"},
            {"id": "rep.*", "type": "int"}
        ],
        "cast": [
            {"id": "mireille", "name": "Mireille", "portrait": "portraits/mireille", "side": "right", "color": "#c8a2ff",
             "expressions": ["court_mask"]},
            {"id": "caelan", "name": "Caelan", "portrait": "portraits/caelan", "side": "left"},
            {"id": "foreman", "name": "Varga Foreman", "portrait": "portraits/foreman"},
            {"id": "watch_guard", "name": "Watchman"}
        ],
        "tiers": ["absolute", "measured", "witnessed", "recorded", "retold"],
        "commands": [{"name": "give_item", "params": ["text", "int"]}, {"name": "camera", "params": ["text"]}],
        "expressions": ["neutral", "smile", "laugh", "angry", "sad"],
        "languages": ["tyffiran", "patter", "aurelian"]
    }"##;

    pub(crate) fn sources() -> StorySources {
        let mut sources = StorySources::default();
        sources.add_file("story.somstory", MANIFEST);
        let mut dialogue = crate::dialogue::tests::sample();
        dialogue.nodes.retain(|n| n.id() != "walked_off");
        dialogue.on_interrupt.clear();
        sources
            .dialogues
            .push(("dialogue/mireille_intro.somdialogue".into(), dialogue));
        sources.quests.push((
            "quests/varga_job.somquest".into(),
            crate::quest::tests::varga(),
        ));
        sources
            .barks
            .push(("barks/watch.sombarks".into(), crate::barks::tests::watch()));
        sources.add_file(
            "notes/earth.somnotes",
            r#"{"version": 1, "id": "earth", "notes": [
                {"id": "earth_mother", "category": "earth", "title": "Ma", "text": "She hummed when she cooked.", "tier": "witnessed"}]}"#,
        );
        sources
    }

    #[test]
    fn the_sample_content_builds_clean() {
        let (database, warnings) = StoryDatabase::build(sources()).unwrap_or_else(|issues| {
            panic!(
                "{}",
                issues
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        });
        assert!(warnings.is_empty(), "{warnings:#?}");
        assert!(database.dialogue("mireille_intro").is_some());
        assert_eq!(
            database.file_of("quest varga_job"),
            Some("quests/varga_job.somquest")
        );
        let table = database.strings();
        assert_eq!(table.get("cast.mireille.name"), Some("Mireille"));
        assert_eq!(table.get("dlg.mireille_intro.n3"), Some("Oh, he bites."));
        assert_eq!(
            table.get("quest.varga_job.quiet.j1"),
            Some("Nobody saw a thing.")
        );
        assert_eq!(
            table.get("note.earth_mother"),
            Some("She hummed when she cooked.")
        );
        assert_eq!(
            table.get("chatter.watch.march_news.1"),
            Some("Frozen. All of it.")
        );
    }

    #[test]
    fn cross_references_are_checked_across_files() {
        let mut sources = sources();
        let broken = r#"{
            "version": 1, "id": "broken",
            "nodes": [
                {"type": "line", "id": "n1", "speaker": "nobody", "text": "Hi {player.nam}.", "next": ["n2"]},
                {"type": "line", "id": "n2", "speaker": "mireille", "expression": "grin", "language": "klingon",
                 "text": "[expr=pout]No.", "next": ["n3"], "when": "done('varga_job', 'vault') or stage('varga_job') == 'outside'"},
                {"type": "action", "id": "n3", "effects": "start_quest('no_quest'); journal('no_note'); set_stage('varga_job', 'roof'); give_item('x', 1)", "next": ["n4"]},
                {"type": "call", "id": "n4", "dialogue": "mireille_intro", "entry": "nowhere", "next": ["n5"]},
                {"type": "jump", "id": "n5", "target": "mireille_intro/zzz"}
            ]}"#;
        sources.add_file("dialogue/broken.somdialogue", broken);
        sources.add_file(
            "dialogue/copy.somdialogue",
            &sources.dialogues[0].1.to_json().unwrap(),
        );
        sources.add_file("dialogue/garbage.somdialogue", "{ not json");
        let (database, issues) = StoryDatabase::build_lenient(sources);
        assert!(database.is_some());
        let text: Vec<String> = issues.iter().map(ToString::to_string).collect();
        for needle in [
            "speaker nobody is not in the cast",
            "{player.nam} is not a declared fact",
            "mireille has no grin expression",
            "mireille has no pout expression",
            "language klingon is not declared",
            "varga_job has no objective vault",
            "varga_job has no stage outside",
            "start_quest('no_quest'): no such quest",
            "journal('no_note'): no such note",
            "varga_job has no stage roof",
            "mireille_intro has no entry or node nowhere",
            "leads to mireille_intro/zzz, which does not exist",
            "dialogue mireille_intro is also defined in",
            "key must be a string",
        ] {
            assert!(
                text.iter().any(|t| t.contains(needle)),
                "missing {needle:?} in\n{}",
                text.join("\n")
            );
        }
        assert!(StoryDatabase::build(StorySources::default()).is_err());
    }

    #[test]
    fn unknown_fields_in_assets_are_refused() {
        let mut sources = sources();
        sources.add_file(
            "dialogue/typo.somdialogue",
            r#"{"version": 1, "id": "typo", "nodes": [{"type": "line", "id": "n1", "text": "x", "nxt": ["n2"]}]}"#,
        );
        let (_, issues) = StoryDatabase::build_lenient(sources);
        assert!(
            issues
                .iter()
                .any(|i| i.file == "dialogue/typo.somdialogue" && i.message.contains("nxt")),
            "{issues:#?}"
        );
    }
}
