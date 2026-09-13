//! Registered JSON documents with staged publication and a designer-owned draft.
//! Only the declared document directory is exposed; IPC credentials and source
//! backups are never readable through this API.
use super::{AuthoringError, project::ProjectPaths, registration::game_registration};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, AuthoringError>;
fn error(message: impl Into<String>) -> AuthoringError {
    AuthoringError::new("document_error", message)
}
fn text<'a>(p: &'a Value, key: &str) -> Result<&'a str> {
    p[key]
        .as_str()
        .ok_or_else(|| error(format!("{key} required")))
}
const MAX_BYTES: usize = 512 * 1024;
#[derive(Clone)]
struct Revision {
    path: PathBuf,
    before: Vec<u8>,
    after: Vec<u8>,
    label: String,
}
#[derive(Clone)]
struct Plan {
    change: Revision,
    generation: u64,
}
/// Unsaved native draft. Remote file commits cannot overwrite its edits.
pub struct DocumentDraft {
    pub path: String,
    pub document_type: String,
    pub value: Value,
    pub focus: String,
    pub generation: u64,
    base: Vec<u8>,
    undo: Vec<Value>,
    redo: Vec<Value>,
}
impl DocumentDraft {
    pub fn dirty(&self) -> bool {
        serde_json::from_slice::<Value>(&self.base).ok().as_ref() != Some(&self.value)
    }
}
/// Bounded per-editor document state, using exact bytes for conflict checks.
pub struct DocumentSession {
    project: ProjectPaths,
    session: String,
    sequence: u64,
    views: BTreeMap<String, (PathBuf, Vec<u8>)>,
    plans: BTreeMap<String, Plan>,
    receipts: BTreeMap<String, (Value, Value)>,
    undo: Vec<Revision>,
    redo: Vec<Revision>,
    pub draft: Option<DocumentDraft>,
}
impl DocumentSession {
    pub fn new(project: ProjectPaths, session: String) -> Self {
        Self {
            project,
            session,
            sequence: 0,
            views: BTreeMap::new(),
            plans: BTreeMap::new(),
            receipts: BTreeMap::new(),
            undo: vec![],
            redo: vec![],
            draft: None,
        }
    }
    fn path(&self, path: &str) -> Result<PathBuf> {
        let relative = Path::new(path);
        if !relative.starts_with(&self.project.manifest.documents)
            || relative.extension().and_then(|s| s.to_str()) != Some("json")
        {
            return Err(error(
                "Only .json files in the declared documents directory are accessible",
            ));
        }
        self.project.resolve(relative).map_err(error)
    }
    fn read(path: &Path) -> Result<Vec<u8>> {
        let size = std::fs::metadata(path)
            .map_err(|e| error(e.to_string()))?
            .len();
        if size > MAX_BYTES as u64 {
            return Err(error("Document exceeds 512 KiB"));
        }
        let bytes = std::fs::read(path).map_err(|e| error(e.to_string()))?;
        if bytes.len() > MAX_BYTES {
            return Err(error("Document exceeds 512 KiB"));
        }
        Ok(bytes)
    }
    fn validate(id: &str, value: &Value) -> Result<()> {
        let game = game_registration();
        let doc = game
            .documents
            .iter()
            .find(|d| d.id == id)
            .ok_or_else(|| error("Unknown registered document type"))?;
        (doc.validate)(value).map_err(error)?;
        if serde_json::to_vec(value)
            .map_err(|e| error(e.to_string()))?
            .len()
            > MAX_BYTES
        {
            return Err(error("Document exceeds 512 KiB"));
        }
        Ok(())
    }
    pub fn list(&self) -> Result<Value> {
        fn visit(
            root: &Path,
            project: &ProjectPaths,
            out: &mut Vec<Value>,
            depth: usize,
        ) -> Result<()> {
            if depth > 12 || out.len() >= 256 {
                return Ok(());
            }
            for entry in std::fs::read_dir(root).map_err(|e| error(e.to_string()))? {
                let entry = entry.map_err(|e| error(e.to_string()))?;
                let path = entry.path();
                if entry
                    .file_type()
                    .map_err(|e| error(e.to_string()))?
                    .is_symlink()
                {
                    continue;
                }
                let relative = path
                    .strip_prefix(&project.root)
                    .map_err(|e| error(e.to_string()))?;
                if project.resolve(relative).is_err() {
                    continue;
                }
                if path.is_dir() {
                    visit(&path, project, out, depth + 1)?;
                } else if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    out.push(json!({"path":relative}));
                }
            }
            Ok(())
        }
        let mut files = vec![];
        visit(
            &self
                .project
                .resolve(&self.project.manifest.documents)
                .map_err(error)?,
            &self.project,
            &mut files,
            0,
        )?;
        Ok(json!({"ok":true,"documents":files}))
    }
    pub fn query(&mut self, p: &Value) -> Result<Value> {
        let path = self.path(text(p, "path")?)?;
        let bytes = Self::read(&path)?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|e| error(e.to_string()))?;
        self.sequence += 1;
        let token = format!("doc-view:{}:{}", self.session, self.sequence);
        if self.views.len() >= 32 {
            self.views.clear();
        }
        self.views.insert(token.clone(), (path, bytes));
        Ok(
            json!({"ok":true,"view_token":token,"content":value,"dirty":self.draft.as_ref().is_some_and(|d|d.path==p["path"]&&d.dirty())}),
        )
    }
    pub fn plan(&mut self, p: &Value) -> Result<Value> {
        let path = self.path(text(p, "path")?)?;
        let (view_path, before) = self
            .views
            .get(text(p, "view_token")?)
            .ok_or_else(|| error("Refresh the document before planning"))?;
        if view_path != &path || Self::read(&path)? != *before {
            return Err(error("Document changed since query"));
        }
        if self
            .draft
            .as_ref()
            .is_some_and(|d| self.path(&d.path).ok().as_ref() == Some(&path) && d.dirty())
        {
            return Err(error(
                "Designer has an unsaved draft; save or revert it before remote publication",
            ));
        }
        let value = p.get("content").ok_or_else(|| error("content required"))?;
        Self::validate(text(p, "document_type")?, value)?;
        let label = text(p, "label")?;
        if label.is_empty() || label.len() > 160 {
            return Err(error("Label must contain 1–160 bytes"));
        }
        if self.plans.len() >= 32 {
            return Err(error("Too many document plans"));
        }
        self.sequence += 1;
        let token = format!("doc-plan:{}:{}", self.session, self.sequence);
        self.plans.insert(
            token.clone(),
            Plan {
                change: Revision {
                    path,
                    before: before.clone(),
                    after: serde_json::to_vec_pretty(value).map_err(|e| error(e.to_string()))?,
                    label: label.into(),
                },
                generation: self.draft.as_ref().map_or(0, |d| d.generation),
            },
        );
        Ok(json!({"ok":true,"plan_token":token,"label":label}))
    }
    pub fn commit(&mut self, p: &Value) -> Result<Value> {
        let request = text(p, "request_id")?;
        if request.is_empty() || request.len() > 256 {
            return Err(error("request_id must contain 1–256 bytes"));
        }
        if let Some((input, result)) = self.receipts.get(request) {
            return if input == p {
                Ok(result.clone())
            } else {
                Err(error("request_id reused with different inputs"))
            };
        }
        let token = text(p, "plan_token")?;
        let plan = self
            .plans
            .get(token)
            .cloned()
            .ok_or_else(|| error("Unknown document plan"))?;
        if self
            .draft
            .as_ref()
            .is_some_and(|d| d.generation != plan.generation || d.dirty())
        {
            return Err(error("Designer draft changed after planning"));
        }
        self.publish(&plan.change, false)?;
        self.undo.push(plan.change);
        self.redo.clear();
        if self.undo.len() > 32 {
            self.undo.remove(0);
        }
        self.plans.remove(token);
        let result = json!({"ok":true,"request_id":request,"document_cursor":self.undo.len()});
        if self.receipts.len() >= 128 {
            self.receipts.clear();
        }
        self.receipts
            .insert(request.into(), (p.clone(), result.clone()));
        Ok(result)
    }
    fn publish(&mut self, change: &Revision, reverse: bool) -> Result<()> {
        let (expected, next) = if reverse {
            (&change.after, &change.before)
        } else {
            (&change.before, &change.after)
        };
        let relative = change
            .path
            .strip_prefix(&self.project.root)
            .map_err(|e| error(e.to_string()))?;
        let path = self.path(&relative.to_string_lossy())?;
        if Self::read(&path)? != *expected {
            return Err(error(
                "File conflict: preserve the designer edit and refresh",
            ));
        }
        crate::save_game::atomic_write(&path, next).map_err(error)?;
        if let Some(d) = self.draft.as_mut() {
            if self.project.root.join(&d.path) == path {
                d.value = serde_json::from_slice(next).map_err(|e| error(e.to_string()))?;
                d.base = next.clone();
                d.generation += 1;
                d.undo.clear();
                d.redo.clear();
            }
        }
        Ok(())
    }
    pub fn history(&mut self, p: &Value) -> Result<Value> {
        let action = p["action"].as_str().unwrap_or("list");
        if action != "list" {
            if self.draft.as_ref().is_some_and(DocumentDraft::dirty) {
                return Err(error(
                    "Save or revert the draft before changing published history",
                ));
            }
            if p["expected_cursor"].as_u64() != Some(self.undo.len() as u64) {
                return Err(error("Document history changed"));
            }
            match action {
                "undo" => {
                    let c = self
                        .undo
                        .last()
                        .cloned()
                        .ok_or_else(|| error("No document edit to undo"))?;
                    self.publish(&c, true)?;
                    self.undo.pop();
                    self.redo.push(c);
                }
                "redo" => {
                    let c = self
                        .redo
                        .last()
                        .cloned()
                        .ok_or_else(|| error("No document edit to redo"))?;
                    self.publish(&c, false)?;
                    self.redo.pop();
                    self.undo.push(c);
                }
                _ => return Err(error("History action must be list, undo or redo")),
            }
        }
        Ok(
            json!({"ok":true,"cursor":self.undo.len(),"entries":self.undo.iter().map(|c|&c.label).collect::<Vec<_>>(),"redo":self.redo.len()}),
        )
    }
    pub fn execute(&mut self, p: &Value) -> Result<Value> {
        let action = text(p, "action")?;
        if action == "document_close" {
            if self.draft.as_ref().is_some_and(DocumentDraft::dirty) {
                return Err(error("Save or revert the draft before closing"));
            }
            self.draft = None;
            return Ok(json!({"ok":true}));
        }
        if action == "document_open" {
            if self.draft.as_ref().is_some_and(DocumentDraft::dirty) {
                return Err(error("Save or revert the current draft first"));
            }
            let relative = text(p, "path")?;
            let path = self.path(relative)?;
            let base = Self::read(&path)?;
            let value: Value = serde_json::from_slice(&base).map_err(|e| error(e.to_string()))?;
            let document_type = text(p, "document_type")?;
            Self::validate(document_type, &value)?;
            self.draft = Some(DocumentDraft {
                path: relative.into(),
                document_type: document_type.into(),
                value,
                focus: String::new(),
                generation: 1,
                base,
                undo: vec![],
                redo: vec![],
            });
            return Ok(json!({"ok":true}));
        }
        let draft_path = self
            .draft
            .as_ref()
            .map(|d| self.path(&d.path))
            .transpose()?;
        let d = self
            .draft
            .as_mut()
            .ok_or_else(|| error("Open a document first"))?;
        if action == "document_focus" {
            let pointer = text(p, "pointer")?;
            if d.value.pointer(pointer).is_none() {
                return Err(error("Document location is absent"));
            }
            d.focus = pointer.into();
            return Ok(json!({"ok":true}));
        }
        if action == "document_save" {
            Self::validate(&d.document_type, &d.value)?;
            let change = Revision {
                path: draft_path
                    .clone()
                    .ok_or_else(|| error("Open a document first"))?,
                before: d.base.clone(),
                after: serde_json::to_vec_pretty(&d.value).map_err(|e| error(e.to_string()))?,
                label: format!("Edit {}", d.path),
            };
            self.publish(&change, false)?;
            self.undo.push(change);
            if self.undo.len() > 32 {
                self.undo.remove(0);
            }
            self.redo.clear();
            return Ok(json!({"ok":true,"document_cursor":self.undo.len()}));
        }
        if action == "document_revert" {
            let bytes = Self::read(
                draft_path
                    .as_ref()
                    .ok_or_else(|| error("Open a document first"))?,
            )?;
            d.value = serde_json::from_slice(&bytes).map_err(|e| error(e.to_string()))?;
            d.base = bytes;
            d.undo.clear();
            d.redo.clear();
            d.generation += 1;
            return Ok(json!({"ok":true}));
        }
        if action == "document_undo" || action == "document_redo" {
            let (from, to) = if action == "document_undo" {
                (&mut d.undo, &mut d.redo)
            } else {
                (&mut d.redo, &mut d.undo)
            };
            let value = from.pop().ok_or_else(|| error("Draft history is empty"))?;
            to.push(std::mem::replace(&mut d.value, value));
            d.generation += 1;
            return Ok(json!({"ok":true}));
        }
        let before = d.value.clone();
        let pointer = text(p, "pointer")?;
        if action == "document_edit" {
            let target = d
                .value
                .pointer_mut(pointer)
                .ok_or_else(|| error("Document field is absent"))?;
            let value = p.get("value").ok_or_else(|| error("value required"))?;
            if target.is_array() || target.is_object() {
                return Err(error("Use array controls for compound values"));
            }
            *target = value.clone();
        } else if action == "document_array" {
            let array = d
                .value
                .pointer_mut(pointer)
                .and_then(Value::as_array_mut)
                .ok_or_else(|| error("Array location is absent"))?;
            let index = p["index"]
                .as_u64()
                .ok_or_else(|| error("Array index required"))? as usize;
            if index >= array.len() {
                return Err(error("Array index is stale"));
            }
            match text(p, "operation")? {
                "duplicate" => array.insert(index + 1, array[index].clone()),
                "remove" => {
                    array.remove(index);
                }
                "up" if index > 0 => array.swap(index, index - 1),
                "down" if index + 1 < array.len() => array.swap(index, index + 1),
                _ => return Err(error("Array move is outside its bounds")),
            }
        } else {
            return Err(error("Unsupported document action"));
        }
        if serde_json::to_vec(&d.value)
            .map_err(|e| error(e.to_string()))?
            .len()
            > MAX_BYTES
        {
            d.value = before;
            return Err(error("Draft exceeds 512 KiB"));
        }
        d.undo.push(before);
        if d.undo.len() > 64 {
            d.undo.remove(0);
        }
        d.redo.clear();
        d.generation += 1;
        let validation = Self::validate(&d.document_type, &d.value)
            .err()
            .map(|e| e.message);
        Ok(json!({"ok":true,"dirty":d.dirty(),"generation":d.generation,"validation":validation}))
    }
}
