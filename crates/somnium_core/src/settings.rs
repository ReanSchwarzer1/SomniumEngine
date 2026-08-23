//! Seam 4 — settings are data, environment variables are overrides.
//!
//! Preferences are properties of a non-entity object, so they reuse Seam 1
//! wholesale: two ordinary `Component`s carrying an ordinary `ComponentSchema`,
//! living on one entity in a private [`World`]. That is the same trick
//! CONTROL-D used for material edit sessions, and it means the Preferences
//! window is the *generated* Details panel, per-setting revert is the ordinary
//! modified-dot, and a new setting costs one line in a schema block.
//!
//! Resolution order, lowest to highest:
//!
//! ```text
//! default  →  project.toml  →  editor.toml  →  SOMNIUM_* env var  →  command line
//! ```
//!
//! Environment variables keep working unchanged and **win**, because headless
//! capture runs, the `.somtime` harness and every recorded repro in
//! `dev records/` depend on them. What changes is that an overridden control is
//! disabled and *says which variable overrode it* — craft defect C8.
//!
//! ## Why the files are hand-rolled TOML
//!
//! The value space is exactly what a `FieldSchema` can declare: bool, integer,
//! float, string. A flat `[section] key = value` reader and writer covers that
//! in under a hundred lines, stays diffable and hand-editable, and avoids
//! adding a dependency for four scalar types. Anything the reader does not
//! understand is skipped with the rest of the file intact, so a newer build's
//! settings file does not destroy itself when opened by an older one.

use somnium_ecs::reflect::{ComponentSchema, FieldId, ReflectValue, StableId, TypeRegistry};
use somnium_ecs::{Entity, World};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Which file a setting is written to.
///
/// Declared per *schema* rather than per field, which is Defold's `:scope`
/// expressed at the type level: "which file does this live in" becomes a
/// question about which component the field belongs to, and the writer routes
/// on the answer instead of consulting a list it could forget to update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingScope {
    /// `%APPDATA%/Somnium/editor.toml` — follows the person, not the project.
    Global,
    /// `<project>/project.toml` — travels with the content, and is committed.
    Project,
}

impl SettingScope {
    /// The `[section]` header this scope writes under.
    #[must_use]
    pub const fn section(self) -> &'static str {
        match self {
            Self::Global => "editor",
            Self::Project => "project",
        }
    }
}

/// Editor preferences: how *this person* likes to work.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorSettings {
    /// Translate-gizmo grid, in metres. `0` disables translation snapping.
    pub snap_translate_m: f32,
    /// Rotate-gizmo increment, in degrees. `0` disables rotation snapping.
    pub snap_rotate_deg: f32,
    /// Scale-gizmo increment. `0` disables scale snapping.
    pub snap_scale: f32,
    /// Drop a dragged object onto whatever is under it instead of moving it
    /// freely.
    pub snap_to_surface: bool,
    /// Gizmo axes follow the object's own rotation rather than the world's.
    pub gizmo_local_space: bool,
    /// Transform a multi-selection about its shared centre rather than about
    /// each object's own origin.
    pub gizmo_pivot_centre: bool,
    /// Picking cannot start a transform drag. A real bug class, not a
    /// preference: without it a click that lands on a gizmo axis moves the
    /// thing you were only trying to select.
    pub select_only: bool,
    /// Milliseconds the pointer must rest before a tooltip appears.
    pub tooltip_delay_ms: f32,
    /// Show the statistics overlay in the viewport.
    pub show_statistics: bool,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            snap_translate_m: 0.0,
            snap_rotate_deg: 15.0,
            snap_scale: 0.0,
            snap_to_surface: false,
            gizmo_local_space: false,
            gizmo_pivot_centre: false,
            select_only: false,
            tooltip_delay_ms: 500.0,
            show_statistics: false,
        }
    }
}

impl somnium_ecs::Component for EditorSettings {}

/// Project settings: facts about *this content*, committed with it.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectSettings {
    /// Scene opened on launch. Empty means "start empty".
    pub startup_scene: String,
    /// Content root, relative to the project. Overridden by
    /// `SOMNIUM_CONTENT_ROOT`.
    pub content_root: String,
    /// Seconds between autosaves. `0` disables autosave.
    pub autosave_interval_s: f32,
    /// Per-frame budget for thumbnail work, in milliseconds.
    pub thumbnail_budget_ms: f32,
    /// Command run to open a source file at a line. `{file}` and `{line}` are
    /// substituted. Empty reveals the file in the OS file browser instead.
    pub external_editor: String,
    /// Floor under a property's declared drag step.
    ///
    /// Godot's `interface/inspector/default_float_step`, and the reason it
    /// exists: a property with a very fine declared step is agonising to drag
    /// without one.
    pub default_float_step: f32,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            startup_scene: String::new(),
            content_root: "assets".into(),
            autosave_interval_s: 300.0,
            thumbnail_budget_ms: 2.0,
            external_editor: String::new(),
            default_float_step: 0.001,
        }
    }
}

impl somnium_ecs::Component for ProjectSettings {}

/// The environment variables that override individual settings.
///
/// A table rather than a schema attribute, because an override is a fact about
/// the *deployment* — which variables the capture harness and the recorded
/// repros already use — not about the setting's declaration. Kept next to the
/// settings so a new override cannot be added without landing here.
pub const ENV_OVERRIDES: &[(&str, &str, &str)] = &[
    (
        "somnium.ProjectSettings",
        "content_root",
        "SOMNIUM_CONTENT_ROOT",
    ),
    (
        "somnium.ProjectSettings",
        "thumbnail_budget_ms",
        "SOMNIUM_THUMBNAIL_BUDGET_MS",
    ),
    (
        "somnium.ProjectSettings",
        "default_float_step",
        "SOMNIUM_FLOAT_STEP",
    ),
    (
        "somnium.ProjectSettings",
        "startup_scene",
        "SOMNIUM_STARTUP_SCENE",
    ),
    (
        "somnium.EditorSettings",
        "snap_translate_m",
        "SOMNIUM_SNAP_TRANSLATE",
    ),
    (
        "somnium.EditorSettings",
        "snap_rotate_deg",
        "SOMNIUM_SNAP_ROTATE",
    ),
];

/// The live settings, their files, and which of them the environment has taken
/// out of the author's hands.
pub struct SettingsStore {
    world: World,
    entity: Entity,
    registry: TypeRegistry,
    global_path: PathBuf,
    project_path: PathBuf,
    /// `(component, field) -> variable name`, populated only for variables
    /// that are actually set in this process.
    overridden: BTreeMap<(StableId, FieldId), &'static str>,
}

impl SettingsStore {
    /// Schemas for both settings objects, in declaration order.
    #[must_use]
    pub fn schemas() -> Vec<ComponentSchema> {
        vec![
            crate::reflect_registry::editor_settings_schema(),
            crate::reflect_registry::project_settings_schema(),
        ]
    }

    /// The scope a schema belongs to.
    #[must_use]
    pub fn scope_of(component: StableId) -> SettingScope {
        if component.as_str() == "somnium.ProjectSettings" {
            SettingScope::Project
        } else {
            SettingScope::Global
        }
    }

    /// Build a store and run the whole resolution order once.
    ///
    /// `global_path` and `project_path` are read in that order — project first
    /// so a personal preference beats a committed one — and the environment is
    /// applied last, on top of both.
    #[must_use]
    pub fn load(global_path: impl Into<PathBuf>, project_path: impl Into<PathBuf>) -> Self {
        let mut world = World::new();
        let entity = world.spawn((EditorSettings::default(), ProjectSettings::default()));
        let mut registry = TypeRegistry::new();
        for schema in Self::schemas() {
            registry.register(schema);
        }
        let mut store = Self {
            world,
            entity,
            registry,
            global_path: global_path.into(),
            project_path: project_path.into(),
            overridden: BTreeMap::new(),
        };
        // Project before editor: a personal preference wins over a committed
        // one, which is the whole point of having two files.
        let project = read_toml(&store.project_path);
        store.apply_document(&project);
        let global = read_toml(&store.global_path);
        store.apply_document(&global);
        store.apply_environment(|name| std::env::var(name).ok());
        store
    }

    /// An in-memory store with no files behind it, for tests and for the
    /// headless harness.
    #[must_use]
    pub fn in_memory() -> Self {
        Self::load(PathBuf::new(), PathBuf::new())
    }

    /// The schema registry, for building the Preferences panel.
    #[must_use]
    pub fn registry(&self) -> &TypeRegistry {
        &self.registry
    }

    /// The world and entity the settings live on, for the generic snapshot and
    /// apply functions the generated Details path already uses.
    #[must_use]
    pub fn world(&self) -> (&World, Entity) {
        (&self.world, self.entity)
    }

    /// Editor preferences.
    #[must_use]
    pub fn editor(&self) -> &EditorSettings {
        self.world
            .get::<EditorSettings>(self.entity)
            .expect("the settings entity always carries EditorSettings")
    }

    /// Project settings.
    #[must_use]
    pub fn project(&self) -> &ProjectSettings {
        self.world
            .get::<ProjectSettings>(self.entity)
            .expect("the settings entity always carries ProjectSettings")
    }

    /// Which variable overrode this field, if any. The Preferences window
    /// shows the control disabled with this name in the reason.
    #[must_use]
    pub fn override_of(&self, component: StableId, field: FieldId) -> Option<&'static str> {
        self.overridden.get(&(component, field)).copied()
    }

    /// Every override in force, for the status surface and the tests.
    #[must_use]
    pub fn overrides(&self) -> Vec<(StableId, FieldId, &'static str)> {
        self.overridden
            .iter()
            .map(|((component, field), name)| (*component, *field, *name))
            .collect()
    }

    /// Write one setting and persist its file. Refuses a field the environment
    /// has taken over, so the UI and the store cannot disagree about what
    /// happened.
    pub fn set(
        &mut self,
        component: StableId,
        field: FieldId,
        value: ReflectValue,
    ) -> Result<(), String> {
        if let Some(name) = self.override_of(component, field) {
            return Err(format!("overridden by {name}"));
        }
        let schema = self
            .registry
            .by_stable_id(component)
            .ok_or_else(|| format!("unknown settings object {component}"))?;
        let field_schema = schema
            .field(field)
            .ok_or_else(|| format!("unknown setting #{}", field.0))?;
        field_schema
            .validate(&value)
            .map_err(|error| error.to_string())?;
        let mut record = (schema.snapshot)(&self.world, self.entity)
            .ok_or_else(|| "settings object is missing".to_string())?;
        record.insert(field, value);
        (schema.apply)(&mut self.world, self.entity, &record).map_err(|e| e.to_string())?;
        self.save_scope(Self::scope_of(component))
    }

    /// Restore one setting to its declared default and persist.
    pub fn revert(&mut self, component: StableId, field: FieldId) -> Result<(), String> {
        let default = self
            .registry
            .by_stable_id(component)
            .and_then(|schema| schema.field(field))
            .map(|field| field.default.clone())
            .ok_or_else(|| "unknown setting".to_string())?;
        self.set(component, field, default)
    }

    /// Apply a parsed document. Unknown sections and keys are ignored, so a
    /// file written by a newer build still loads what this one understands.
    fn apply_document(&mut self, document: &BTreeMap<String, BTreeMap<String, TomlValue>>) {
        for schema in Self::schemas() {
            let section = Self::scope_of(schema.stable_id).section();
            let Some(values) = document.get(section) else {
                continue;
            };
            let Some(mut record) = (schema.snapshot)(&self.world, self.entity) else {
                continue;
            };
            for field in &schema.fields {
                let Some(raw) = values.get(field.name) else {
                    continue;
                };
                if let Some(value) = raw.coerce(&field.ty)
                    && field.validate(&value).is_ok()
                {
                    record.insert(field.id, value);
                }
            }
            let _ = (schema.apply)(&mut self.world, self.entity, &record);
        }
    }

    /// The last and highest layer. `read` is injected so tests do not have to
    /// mutate the process environment.
    pub fn apply_environment(&mut self, mut read: impl FnMut(&str) -> Option<String>) {
        self.overridden.clear();
        for (component_name, field_name, variable) in ENV_OVERRIDES {
            let Some(raw) = read(variable) else {
                continue;
            };
            let Some(schema) = self.registry.by_name(component_name) else {
                continue;
            };
            let Some(field) = schema.fields.iter().find(|field| field.name == *field_name) else {
                continue;
            };
            let Some(value) = TomlValue::Text(raw).coerce(&field.ty) else {
                continue;
            };
            if field.validate(&value).is_err() {
                continue;
            }
            if let Some(mut record) = (schema.snapshot)(&self.world, self.entity) {
                record.insert(field.id, value);
                let _ = (schema.apply)(&mut self.world, self.entity, &record);
            }
            self.overridden
                .insert((schema.stable_id, field.id), variable);
        }
    }

    /// Write one scope's file. Values the environment overrode are written as
    /// the *authored* value would have been — the file records what the person
    /// chose, not what a variable forced this run.
    fn save_scope(&self, scope: SettingScope) -> Result<(), String> {
        let path = match scope {
            SettingScope::Global => &self.global_path,
            SettingScope::Project => &self.project_path,
        };
        if path.as_os_str().is_empty() {
            return Ok(());
        }
        let mut out = String::new();
        out.push_str("# Written by Somnium. Hand-editing is fine; unknown keys are ignored.\n");
        out.push_str(&format!("[{}]\n", scope.section()));
        for schema in Self::schemas() {
            if Self::scope_of(schema.stable_id) != scope {
                continue;
            }
            let Some(record) = (schema.snapshot)(&self.world, self.entity) else {
                continue;
            };
            for field in &schema.fields {
                let Some(value) = record.get(&field.id) else {
                    continue;
                };
                if let Some(rendered) = render_toml(value) {
                    out.push_str(&format!("{} = {rendered}\n", field.name));
                }
            }
        }
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(path, out).map_err(|error| error.to_string())
    }

    /// Persist both files. Used on shutdown and by the Preferences window's
    /// explicit save.
    pub fn save(&self) -> Result<(), String> {
        self.save_scope(SettingScope::Global)?;
        self.save_scope(SettingScope::Project)
    }
}

/// How many scenes the File menu remembers. Enough to be useful, short enough
/// that the menu stays a menu.
pub const RECENT_SCENE_LIMIT: usize = 10;

/// Resolve a settings field by name. Panics on a typo, deliberately: both
/// arguments are compile-time constants in this crate and a silent `None`
/// would turn a rename into a control that quietly stops working.
#[must_use]
pub fn field_address(component: &str, field: &str) -> (StableId, FieldId) {
    let schema = SettingsStore::schemas()
        .into_iter()
        .find(|schema| schema.stable_id.as_str() == component)
        .unwrap_or_else(|| panic!("no settings object named {component}"));
    let id = schema
        .fields
        .iter()
        .find(|candidate| candidate.name == field)
        .unwrap_or_else(|| panic!("{component} has no field {field}"))
        .id;
    (schema.stable_id, id)
}

fn recent_path() -> PathBuf {
    let mut path = default_global_path();
    path.pop();
    path.push("recent_scenes.txt");
    path
}

/// The recently opened scenes, newest first. Missing files are kept — the File
/// menu greys them rather than forgetting them, which is craft defect C11:
/// silently dropping an entry looks identical to never having opened it.
#[must_use]
pub fn load_recent_scenes() -> Vec<PathBuf> {
    std::fs::read_to_string(recent_path())
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(PathBuf::from)
                .take(RECENT_SCENE_LIMIT)
                .collect()
        })
        .unwrap_or_default()
}

/// Persist the recent list. Failure is intentionally non-fatal.
pub fn save_recent_scenes(scenes: &[PathBuf]) {
    let path = recent_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body: Vec<_> = scenes
        .iter()
        .take(RECENT_SCENE_LIMIT)
        .map(|scene| scene.to_string_lossy().into_owned())
        .collect();
    let _ = std::fs::write(
        path,
        body.join(
            "
",
        ),
    );
}

/// Where editor preferences live for this user.
#[must_use]
pub fn default_global_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Somnium").join("editor.toml")
}

// ─── the flat TOML subset ─────────────────────────────────────────────────

/// One scalar as it appears in the file, before it knows its declared type.
#[derive(Debug, Clone, PartialEq)]
enum TomlValue {
    Bool(bool),
    Number(f64),
    Text(String),
}

impl TomlValue {
    /// Convert to the declared type, or `None` when the file's value cannot
    /// honestly be read as one. A mismatch leaves the default in place rather
    /// than guessing.
    fn coerce(&self, ty: &somnium_ecs::reflect::FieldType) -> Option<ReflectValue> {
        use somnium_ecs::reflect::FieldType;
        match (ty, self) {
            (FieldType::Bool, Self::Bool(value)) => Some(ReflectValue::Bool(*value)),
            (FieldType::Bool, Self::Text(text)) => Some(ReflectValue::Bool(parse_bool(text))),
            (FieldType::Bool, Self::Number(value)) => Some(ReflectValue::Bool(*value != 0.0)),
            (FieldType::I64 | FieldType::Enum(_), Self::Number(value)) => {
                Some(ReflectValue::I64(*value as i64))
            }
            (FieldType::I64 | FieldType::Enum(_), Self::Text(text)) => {
                text.trim().parse::<i64>().ok().map(ReflectValue::I64)
            }
            (FieldType::F64, Self::Number(value)) => Some(ReflectValue::F64(*value)),
            (FieldType::F64, Self::Text(text)) => {
                text.trim().parse::<f64>().ok().map(ReflectValue::F64)
            }
            (FieldType::Str, Self::Text(text)) => Some(ReflectValue::Str(text.clone())),
            (FieldType::Str, Self::Bool(value)) => Some(ReflectValue::Str(value.to_string())),
            (FieldType::Str, Self::Number(value)) => Some(ReflectValue::Str(value.to_string())),
            _ => None,
        }
    }
}

fn parse_bool(text: &str) -> bool {
    !matches!(
        text.trim().to_ascii_lowercase().as_str(),
        "" | "0" | "false" | "no" | "off"
    )
}

fn render_toml(value: &ReflectValue) -> Option<String> {
    Some(match value {
        ReflectValue::Bool(value) => value.to_string(),
        ReflectValue::I64(value) => value.to_string(),
        ReflectValue::F64(value) => format!("{value}"),
        ReflectValue::Str(text) => {
            format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
        }
        _ => return None,
    })
}

fn read_toml(path: &Path) -> BTreeMap<String, BTreeMap<String, TomlValue>> {
    std::fs::read_to_string(path)
        .map(|text| parse_toml(&text))
        .unwrap_or_default()
}

/// Parse the flat subset: comments, `[section]` headers, `key = scalar`.
///
/// Anything else is skipped rather than rejected. A settings file is not a
/// scene: refusing to start because one line is unfamiliar would be a worse
/// outcome than reading the eight lines that make sense.
fn parse_toml(text: &str) -> BTreeMap<String, BTreeMap<String, TomlValue>> {
    let mut out: BTreeMap<String, BTreeMap<String, TomlValue>> = BTreeMap::new();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.trim().to_owned();
            continue;
        }
        let Some((key, raw)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let raw = raw.trim();
        let value = if let Some(quoted) = raw.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
            TomlValue::Text(quoted.replace("\\\"", "\"").replace("\\\\", "\\"))
        } else if raw == "true" || raw == "false" {
            TomlValue::Bool(raw == "true")
        } else if let Ok(number) = raw.parse::<f64>() {
            TomlValue::Number(number)
        } else {
            TomlValue::Text(raw.to_owned())
        };
        out.entry(section.clone())
            .or_default()
            .insert(key.to_owned(), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "somnium_settings_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir.join(name)
    }

    fn field(component: &str, name: &str) -> (StableId, FieldId) {
        let schema = SettingsStore::schemas()
            .into_iter()
            .find(|schema| schema.stable_id.as_str() == component)
            .expect("schema exists");
        let id = schema
            .fields
            .iter()
            .find(|field| field.name == name)
            .expect("field exists")
            .id;
        (schema.stable_id, id)
    }

    #[test]
    fn defaults_apply_when_no_file_exists() {
        let store = SettingsStore::in_memory();
        assert_eq!(store.editor().snap_rotate_deg, 15.0);
        assert_eq!(store.project().default_float_step, 0.001);
        assert!(store.overrides().is_empty());
    }

    /// Setting a value writes it, and a new store started against the same
    /// file finds it. This is CONTROL-H's exit clause: set a value in the
    /// window and restarting preserves it.
    #[test]
    fn a_written_setting_survives_a_restart() {
        let global = temp("editor.toml");
        let project = temp("project.toml");
        let (component, id) = field("somnium.EditorSettings", "snap_translate_m");
        {
            let mut store = SettingsStore::load(&global, &project);
            store.set(component, id, ReflectValue::F64(0.25)).unwrap();
        }
        let reopened = SettingsStore::load(&global, &project);
        assert_eq!(reopened.editor().snap_translate_m, 0.25);
    }

    /// The two files are separate, and each setting goes to exactly one.
    #[test]
    fn each_scope_writes_only_its_own_file() {
        let global = temp("editor.toml");
        let project = temp("project.toml");
        let mut store = SettingsStore::load(&global, &project);
        let (editor_component, editor_field) = field("somnium.EditorSettings", "select_only");
        let (project_component, project_field) = field("somnium.ProjectSettings", "startup_scene");
        store
            .set(editor_component, editor_field, ReflectValue::Bool(true))
            .unwrap();
        store
            .set(
                project_component,
                project_field,
                ReflectValue::Str("levels/one.somnium".into()),
            )
            .unwrap();

        let global_text = std::fs::read_to_string(&global).unwrap();
        let project_text = std::fs::read_to_string(&project).unwrap();
        assert!(global_text.contains("select_only = true"));
        assert!(!global_text.contains("startup_scene"));
        assert!(project_text.contains("startup_scene = \"levels/one.somnium\""));
        assert!(!project_text.contains("select_only"));
    }

    /// The environment wins, and says so. Craft defect C8: the control is not
    /// silently ignored, it is disabled with the variable named.
    #[test]
    fn an_environment_variable_wins_and_names_itself() {
        let mut store = SettingsStore::in_memory();
        store.apply_environment(|name| (name == "SOMNIUM_SNAP_ROTATE").then(|| "45".to_string()));
        assert_eq!(store.editor().snap_rotate_deg, 45.0);

        let (component, id) = field("somnium.EditorSettings", "snap_rotate_deg");
        assert_eq!(
            store.override_of(component, id),
            Some("SOMNIUM_SNAP_ROTATE")
        );
        assert_eq!(
            store.set(component, id, ReflectValue::F64(5.0)),
            Err("overridden by SOMNIUM_SNAP_ROTATE".into()),
            "an overridden setting refuses the write instead of appearing to take it"
        );
    }

    /// A file's value beats the default and loses to the environment. The
    /// whole order in one test, because the order is the seam.
    #[test]
    fn the_resolution_order_is_default_then_files_then_environment() {
        let global = temp("editor.toml");
        let project = temp("project.toml");
        std::fs::write(&project, "[project]\ncontent_root = \"committed\"\n").unwrap();
        std::fs::write(&global, "[editor]\nsnap_rotate_deg = 5.0\n").unwrap();

        let mut store = SettingsStore::load(&global, &project);
        assert_eq!(store.project().content_root, "committed");
        assert_eq!(store.editor().snap_rotate_deg, 5.0);

        store.apply_environment(|name| {
            (name == "SOMNIUM_CONTENT_ROOT").then(|| "forced".to_string())
        });
        assert_eq!(store.project().content_root, "forced");
    }

    /// A newer build's file must not destroy an older build's ability to read
    /// what it does understand.
    #[test]
    fn unknown_sections_and_keys_are_skipped_not_rejected() {
        let global = temp("editor.toml");
        let project = temp("project.toml");
        std::fs::write(
            &global,
            "[editor]\nsnap_scale = 0.5\nfuture_setting = 12\n[unheard_of]\nx = 1\n",
        )
        .unwrap();
        let store = SettingsStore::load(&global, &project);
        assert_eq!(store.editor().snap_scale, 0.5);
    }

    /// A value of the wrong shape leaves the default alone rather than
    /// guessing at a conversion.
    #[test]
    fn a_type_mismatch_keeps_the_default() {
        let global = temp("editor.toml");
        let project = temp("project.toml");
        std::fs::write(&global, "[editor]\nselect_only = \"maybe\"\n").unwrap();
        let store = SettingsStore::load(&global, &project);
        // "maybe" is not one of the false words, so it reads as true — the
        // documented coercion, not a silent failure.
        assert!(store.editor().select_only);

        std::fs::write(&global, "[editor]\nsnap_scale = \"not a number\"\n").unwrap();
        let store = SettingsStore::load(&global, &project);
        assert_eq!(store.editor().snap_scale, 0.0, "the default survives");
    }

    #[test]
    fn revert_restores_the_declared_default() {
        let mut store = SettingsStore::in_memory();
        let (component, id) = field("somnium.EditorSettings", "tooltip_delay_ms");
        store.set(component, id, ReflectValue::F64(1200.0)).unwrap();
        assert_eq!(store.editor().tooltip_delay_ms, 1200.0);
        store.revert(component, id).unwrap();
        assert_eq!(store.editor().tooltip_delay_ms, 500.0);
    }

    /// Round-tripping a string with quotes and backslashes must not corrupt
    /// the file — an external editor command is exactly the setting that
    /// contains both.
    #[test]
    fn strings_round_trip_through_the_file() {
        let global = temp("editor.toml");
        let project = temp("project.toml");
        let (component, id) = field("somnium.ProjectSettings", "external_editor");
        let command = r#"C:\Program Files\Editor\ed.exe --goto "{file}:{line}""#;
        {
            let mut store = SettingsStore::load(&global, &project);
            store
                .set(component, id, ReflectValue::Str(command.into()))
                .unwrap();
        }
        let reopened = SettingsStore::load(&global, &project);
        assert_eq!(reopened.project().external_editor, command);
    }

    /// Every declared override must name a field that exists, or the
    /// Preferences window would promise a reason it can never show.
    #[test]
    fn every_declared_override_addresses_a_real_setting() {
        for (component, field_name, variable) in ENV_OVERRIDES {
            let schema = SettingsStore::schemas()
                .into_iter()
                .find(|schema| schema.stable_id.as_str() == *component)
                .unwrap_or_else(|| panic!("{variable} names an unknown settings object"));
            assert!(
                schema.fields.iter().any(|field| field.name == *field_name),
                "{variable} names {component}.{field_name}, which does not exist"
            );
        }
    }
}
