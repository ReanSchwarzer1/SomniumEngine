//! Staged world edits with optimistic concurrency and shared editor history.
use super::{codec, registration};
use crate::{
    EditorFlags, Name, Parent, Transform,
    editor_commands::{EditorCommand, ReparentBatchCmd, SetFieldCmd, UndoStack},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use somnium_ecs::{
    Entity, PersistentId, World,
    reflect::{FieldFlags, ReflectObject, TypeRegistry},
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, Serialize)]
pub struct AuthoringError {
    pub code: &'static str,
    pub message: String,
    pub operation: Option<usize>,
}
impl AuthoringError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            operation: None,
        }
    }
    pub fn json(&self) -> Value {
        json!({"ok":false,"error":self})
    }
}
type Result<T> = std::result::Result<T, AuthoringError>;
fn invalid(message: impl Into<String>) -> AuthoringError {
    AuthoringError::new("validation_failed", message)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Create {
        key: String,
        name: String,
        #[serde(default)]
        components: BTreeMap<String, BTreeMap<String, Value>>,
    },
    Set {
        entity: String,
        component: String,
        field: String,
        value: Value,
    },
    AddComponent {
        entity: String,
        component: String,
        #[serde(default)]
        fields: BTreeMap<String, Value>,
    },
    RemoveComponent {
        entity: String,
        component: String,
    },
    Delete {
        entity: String,
    },
    Reparent {
        entity: String,
        parent: Option<String>,
    },
    Preset {
        id: String,
        #[serde(default)]
        args: Value,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRequest {
    pub expected_revision: u64,
    pub label: String,
    pub operations: Vec<Operation>,
}
#[derive(Clone)]
struct PreparedPlan {
    before: Value,
    after: Value,
    request: PlanRequest,
    created: BTreeMap<String, String>,
    revision: u64,
}
#[derive(Clone)]
struct Receipt {
    input: Value,
    result: Value,
}
/// One session owns plans/receipts, while undo remains the editor's existing stack.
pub struct AuthoringSession {
    pub project_id: String,
    pub session_id: String,
    pub revision: u64,
    observed: Option<Value>,
    sequence: u64,
    plans: BTreeMap<String, PreparedPlan>,
    receipts: BTreeMap<String, Receipt>,
    receipt_order: VecDeque<String>,
    pub protected: BTreeSet<String>,
}
impl AuthoringSession {
    pub fn new(project_id: impl Into<String>, session_id: impl Into<String>) -> Self {
        Self {
            project_id: project_id.into(),
            session_id: session_id.into(),
            revision: 0,
            observed: None,
            sequence: 0,
            plans: BTreeMap::new(),
            receipts: BTreeMap::new(),
            receipt_order: VecDeque::new(),
            protected: BTreeSet::new(),
        }
    }
    /// Called at request boundaries, never once per rendered frame. Detects edits
    /// from any adapter, including scripts/file reloads that bypass undo.
    pub fn observe(&mut self, world: &mut World) -> Value {
        let doc = authoring_snapshot(world);
        if self.observed.as_ref().is_some_and(|old| old != &doc) {
            self.revision += 1;
        }
        self.observed = Some(doc.clone());
        doc
    }
    pub fn check_revision(&mut self, world: &mut World, expected: u64) -> Result<()> {
        self.observe(world);
        if self.revision != expected {
            Err(AuthoringError::new(
                "revision_conflict",
                format!(
                    "expected {expected}, current {}; refresh before editing",
                    self.revision
                ),
            ))
        } else {
            Ok(())
        }
    }
    pub fn discover(&self) -> Value {
        let registry = crate::reflect_registry::editor_registry();
        let components:Vec<_>=registry.iter().map(|s|{
            let fields:Vec<_>=s.fields.iter().map(|f|json!({
                "id":f.id.0,"name":f.name,"label":f.display_name.unwrap_or(f.name),
                "schema":codec::type_schema(&f.ty),"editable":f.flags.contains(FieldFlags::EDIT)&&!f.read_only,
                "serialized":f.flags.contains(FieldFlags::SERIALIZE),"unit":f.unit,"minimum":f.min,"maximum":f.max,
                "help":f.doc,"group":f.group,"scope":format!("{:?}",f.scope)
            })).collect();
            json!({"id":s.stable_id.as_str(),"label":s.display_name,"version":s.version,"fields":fields})
        }).collect();
        let game = registration::game_registration();
        let commands:Vec<_>=somnium_ui::commands::registry().commands().iter().map(|c|json!({"id":c.id,"label":c.label,"category":c.category,"action":format!("{:?}",c.action)})).collect();
        json!({"ok":true,"project_id":self.project_id,"session_id":self.session_id,"revision":self.revision,
            "command_schema_version":1,"limits":{"operations":256,"plans":32,"page_size":500},"components":components,"commands":commands,
            "game":{"label":game.label,"presets":game.presets.iter().map(|p|json!({"id":p.id,"label":p.label,"category":p.category,"schema":p.schema})).collect::<Vec<_>>(),
                "documents":game.documents.iter().map(|d|json!({"id":d.id,"label":d.label,"extension":d.extension,"schema":d.schema})).collect::<Vec<_>>()},
            "operations":["create","set","add_component","remove_component","delete","reparent","preset"]})
    }
    pub fn query(&mut self, world: &mut World, params: &Value) -> Result<Value> {
        self.observe(world);
        let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(100)
            .min(500) as usize;
        let filter = params
            .get("search")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();
        let target = params.get("entity").and_then(Value::as_str);
        let registry = crate::reflect_registry::editor_registry();
        let mut entities: Vec<_> = world
            .entities()
            .filter_map(|e| {
                let id = world.persistent_id(e)?.to_string();
                let name = world.get::<Name>(e).map_or("", Name::as_str);
                if target.is_some_and(|t| t != id)
                    || (!filter.is_empty() && !name.to_lowercase().contains(&filter))
                {
                    return None;
                }
                Some((id, e, name.to_owned()))
            })
            .collect();
        entities.sort_by(|a, b| a.0.cmp(&b.0));
        let total = entities.len();
        let rows:Vec<_>=entities.into_iter().skip(offset).take(limit).map(|(id,e,name)|{
            let components:Vec<_>=registry.schemas_on(world,e).into_iter().map(|s|{
                let fields:serde_json::Map<String,Value>=s.fields.iter().filter_map(|f|{
                    (s.read_field)(world,e,f.id).map(|v|(f.name.to_owned(),codec::encode(world,&f.ty,&v)))
                }).collect();
                json!({"id":s.stable_id.as_str(),"fields":fields})
            }).collect();
            json!({"id":id,"name":name,"protected":self.protected.contains(&id),"components":components})
        }).collect();
        if target.is_some() && rows.is_empty() {
            return Err(AuthoringError::new(
                "stale_entity",
                "entity is absent or unloaded",
            ));
        }
        Ok(
            json!({"ok":true,"revision":self.revision,"total":total,"offset":offset,"entities":rows,"next_offset":(offset+limit<total).then_some(offset+limit)}),
        )
    }
    pub fn plan(&mut self, world: &mut World, request: PlanRequest) -> Result<Value> {
        self.check_revision(world, request.expected_revision)?;
        if request.label.is_empty()
            || request.label.len() > 160
            || request.operations.is_empty()
            || request.operations.len() > 256
        {
            return Err(invalid(
                "label must contain 1–160 bytes; batch must contain 1–256 operations",
            ));
        }
        if self.plans.len() >= 32 {
            return Err(AuthoringError::new(
                "plan_limit",
                "commit or discard an existing plan",
            ));
        }
        let before = self.observed.clone().expect("observe completed");
        let registry = crate::reflect_registry::editor_registry();
        let mut staged = World::new();
        crate::scene_schema::scene_from_json(&mut staged, &registry, &before)
            .map_err(|e| invalid(e.to_string()))?;
        let mut created = BTreeMap::new();
        for (index, op) in request.operations.iter().enumerate() {
            if let Err(mut e) =
                apply_operation(&mut staged, &registry, op, &mut created, &self.protected)
            {
                e.operation = Some(index);
                return Err(e);
            }
        }
        let after = authoring_snapshot(&mut staged);
        for id in changed_ids(&before, &after) {
            if let Some(e) =
                PersistentId::parse_hex(&id).and_then(|id| world.entity_by_persistent_id(id))
            {
                editable(world, e, &self.protected)?;
            }
        }
        validate_references(&after)?;
        self.sequence += 1;
        let token = format!("{}:plan:{}", self.session_id, self.sequence);
        let changed = changed_ids(&before, &after);
        self.plans.insert(
            token.clone(),
            PreparedPlan {
                before,
                after,
                request,
                created: created.clone(),
                revision: self.revision,
            },
        );
        Ok(
            json!({"ok":true,"plan_token":token,"revision":self.revision,"created":created,"changed":changed,"operations":self.plans[&token].request.operations.len()}),
        )
    }
    pub fn discard(&mut self, token: &str) -> bool {
        self.plans.remove(token).is_some()
    }
    pub fn commit(
        &mut self,
        world: &mut World,
        undo: &mut UndoStack,
        selected: &mut Option<Entity>,
        params: &Value,
    ) -> Result<Value> {
        let request_id = params
            .get("request_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or_else(|| invalid("request_id required (1–256 bytes)"))?;
        if let Some(receipt) = self.receipts.get(request_id) {
            if receipt.input != *params {
                return Err(invalid("request_id reused with different input"));
            }
            return Ok(receipt.result.clone());
        }
        let token = params
            .get("plan_token")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("plan_token required"))?;
        let plan = self.plans.get(token).cloned().ok_or_else(|| {
            AuthoringError::new(
                "plan_expired",
                "unknown plan token or editor session restarted",
            )
        })?;
        self.check_revision(world, plan.revision)?;
        if self.observed.as_ref() != Some(&plan.before) {
            return Err(AuthoringError::new(
                "revision_conflict",
                "world changed; plan again",
            ));
        }
        for id in changed_ids(&plan.before, &plan.after) {
            if self.protected.contains(&id) {
                return Err(AuthoringError::new(
                    "protected_entity",
                    format!("{id} is protected by the designer"),
                ));
            }
        }
        let changed = changed_ids(&plan.before, &plan.after);
        let command = WorldPatch {
            before: plan.before.clone(),
            after: plan.after.clone(),
            label: plan.request.label.clone(),
            before_runtime: capture_removed(world, &plan.before, &plan.after),
            after_runtime: Vec::new(),
        };
        if let Err(error) = apply_delta(world, &command.before, &command.after) {
            if let Err(rollback) = apply_delta(world, &command.after, &command.before) {
                return Err(AuthoringError::new(
                    "rollback_failed",
                    format!("{error}; rollback: {rollback}"),
                ));
            }
            return Err(invalid(error));
        }
        if !changed.is_empty() {
            undo.push_silent(Box::new(command));
        }
        if selected.is_some_and(|e| !world.is_alive(e)) {
            *selected = None;
        }
        registration::rebuild(world);
        self.observe(world);
        self.plans.remove(token);
        let result = json!({"ok":true,"request_id":request_id,"revision_before":plan.revision,"revision":self.revision,
            "changed":changed,"created":plan.created,"undo_cursor":undo.history().1,"label":plan.request.label});
        while self.receipts.len() >= 128 {
            if let Some(id) = self.receipt_order.pop_front() {
                self.receipts.remove(&id);
            }
        }
        self.receipt_order.push_back(request_id.into());
        self.receipts.insert(
            request_id.into(),
            Receipt {
                input: params.clone(),
                result: result.clone(),
            },
        );
        Ok(result)
    }
    pub fn history(
        &mut self,
        world: &mut World,
        undo: &mut UndoStack,
        selected: &mut Option<Entity>,
        params: &Value,
    ) -> Result<Value> {
        self.observe(world);
        let action = params
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("list");
        if action != "list" {
            let expected = params
                .get("expected_revision")
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid("expected_revision required for history mutation"))?;
            self.check_revision(world, expected)?;
            let cursor = params
                .get("expected_cursor")
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid("expected_cursor required"))?
                as usize;
            if cursor != undo.history().1 {
                return Err(AuthoringError::new(
                    "history_conflict",
                    "designer changed undo history",
                ));
            }
            match action {
                "undo" => {
                    undo.undo(world, selected);
                }
                "redo" => {
                    undo.redo(world, selected);
                }
                _ => return Err(invalid("history action must be list, undo or redo")),
            }
            registration::rebuild(world);
            self.observe(world);
        }
        let (names, cursor) = undo.history();
        Ok(json!({"ok":true,"revision":self.revision,"cursor":cursor,"entries":names}))
    }
}
fn resolve(world: &World, key: &str, created: &BTreeMap<String, String>) -> Result<Entity> {
    let text = created.get(key).map_or(key, String::as_str);
    PersistentId::parse_hex(text)
        .and_then(|id| world.entity_by_persistent_id(id))
        .ok_or_else(|| {
            AuthoringError::new(
                "stale_entity",
                format!("entity {key} is absent or unloaded"),
            )
        })
}
fn editable(world: &World, e: Entity, protected: &BTreeSet<String>) -> Result<()> {
    if world.get::<EditorFlags>(e).is_some_and(|f| f.locked)
        || world
            .persistent_id(e)
            .is_some_and(|id| protected.contains(&id.to_string()))
    {
        return Err(AuthoringError::new(
            "protected_entity",
            "entity is locked or designer-protected",
        ));
    }
    Ok(())
}
fn apply_operation(
    world: &mut World,
    registry: &TypeRegistry,
    op: &Operation,
    created: &mut BTreeMap<String, String>,
    protected: &BTreeSet<String>,
) -> Result<()> {
    match op {
        Operation::Create {
            key,
            name,
            components,
        } => {
            if key.is_empty() || created.contains_key(key) || name.len() > 63 {
                return Err(invalid(
                    "create needs a unique key and a name of at most 63 UTF-8 bytes",
                ));
            }
            let id = PersistentId::mint();
            let e = world.spawn((id, Name::new(name), Transform::default()));
            created.insert(key.clone(), id.to_string());
            for (component, fields) in components {
                add_component(world, registry, e, component, fields, created)?;
            }
        }
        Operation::Set {
            entity,
            component,
            field,
            value,
        } => {
            let e = resolve(world, entity, created)?;
            editable(world, e, protected)?;
            set_field(world, registry, e, component, field, value, created)?;
        }
        Operation::AddComponent {
            entity,
            component,
            fields,
        } => {
            let e = resolve(world, entity, created)?;
            editable(world, e, protected)?;
            if world.get::<crate::prefab::PrefabMember>(e).is_some() {
                return Err(invalid(
                    "unpack or edit the prefab source before adding a component",
                ));
            }
            add_component(world, registry, e, component, fields, created)?;
        }
        Operation::RemoveComponent { entity, component } => {
            let e = resolve(world, entity, created)?;
            editable(world, e, protected)?;
            let s = registry
                .by_name(component)
                .ok_or_else(|| invalid(format!("unknown component {component}")))?;
            if component == "somnium.Parent" {
                return Err(invalid("use reparent to detach a parent"));
            }
            if world.get::<crate::prefab::PrefabMember>(e).is_some() {
                return Err(invalid(
                    "unpack or edit the prefab source before structural component removal",
                ));
            }
            (s.remove)(world, e).map_err(|e| invalid(e.to_string()))?;
        }
        Operation::Delete { entity } => {
            let e = resolve(world, entity, created)?;
            editable(world, e, protected)?;
            if world.get::<crate::prefab::PrefabMember>(e).is_some() {
                return Err(invalid("unpack the prefab before deleting a member"));
            }
            let id = world
                .persistent_id(e)
                .ok_or_else(|| invalid("entity has no identity"))?
                .to_string();
            let document = authoring_snapshot(world);
            for (other, entry) in entries(&document) {
                if other != id && has_reference(&entry, &id) {
                    return Err(invalid(format!(
                        "detach references from {other} before deleting {id}"
                    )));
                }
            }
            world.despawn(e);
        }
        Operation::Reparent { entity, parent } => {
            let e = resolve(world, entity, created)?;
            editable(world, e, protected)?;
            let p = parent
                .as_ref()
                .map(|p| resolve(world, p, created))
                .transpose()?;
            let mut command = ReparentBatchCmd::new(world, vec![e], p).map_err(invalid)?;
            command.execute(world, &mut None);
        }
        Operation::Preset { id, args } => {
            let game = registration::game_registration();
            let p = game
                .presets
                .iter()
                .find(|p| p.id == id)
                .ok_or_else(|| invalid(format!("unknown preset {id}")))?;
            for (e_index, e) in (p.build)(world, args)
                .map_err(invalid)?
                .into_iter()
                .enumerate()
            {
                let pid = world
                    .ensure_persistent_id(e)
                    .map_err(|e| invalid(e.to_string()))?;
                created.insert(format!("{id}:{e_index}"), pid.to_string());
            }
        }
    }
    Ok(())
}
fn with_resolved_keys(value: &Value, created: &BTreeMap<String, String>) -> Value {
    match value {
        Value::Object(map) => {
            let mut map = map.clone();
            if let Some(key) = map.get("$entity").and_then(Value::as_str) {
                if let Some(id) = created.get(key) {
                    map.insert("$entity".into(), json!(id));
                }
            }
            Value::Object(
                map.into_iter()
                    .map(|(k, v)| (k, with_resolved_keys(&v, created)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| with_resolved_keys(v, created))
                .collect(),
        ),
        v => v.clone(),
    }
}
fn set_field(
    world: &mut World,
    registry: &TypeRegistry,
    e: Entity,
    component: &str,
    field: &str,
    value: &Value,
    created: &BTreeMap<String, String>,
) -> Result<()> {
    if component == "somnium.Parent" {
        return Err(invalid("use reparent for validated hierarchy editing"));
    }
    let s = registry
        .by_name(component)
        .ok_or_else(|| invalid(format!("unknown component {component}")))?;
    let f = s
        .field_by_name(field)
        .ok_or_else(|| invalid(format!("unknown field {component}.{field}")))?;
    let value =
        codec::decode(world, &f.ty, &with_resolved_keys(value, created)).map_err(invalid)?;
    SetFieldCmd::apply_live(world, e, s.stable_id, f.id, value).map_err(invalid)
}
fn add_component(
    world: &mut World,
    registry: &TypeRegistry,
    e: Entity,
    component: &str,
    fields: &BTreeMap<String, Value>,
    created: &BTreeMap<String, String>,
) -> Result<()> {
    let s = registry
        .by_name(component)
        .ok_or_else(|| invalid(format!("unknown component {component}")))?;
    if (s.snapshot)(world, e).is_none() {
        (s.insert_default)(world, e).map_err(|e| invalid(e.to_string()))?;
    }
    for (field, value) in fields {
        set_field(world, registry, e, component, field, value, created)?;
    }
    Ok(())
}
fn entries(doc: &Value) -> BTreeMap<String, Value> {
    doc.get("entities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| Some((e.get("persistent_id")?.as_str()?.to_owned(), e.clone())))
        .collect()
}
fn changed_ids(before: &Value, after: &Value) -> Vec<String> {
    let a = entries(before);
    let b = entries(after);
    a.keys()
        .chain(b.keys())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|id| a.get(id) != b.get(id))
        .collect()
}
fn validate_references(doc: &Value) -> Result<()> {
    let ids: BTreeSet<_> = entries(doc).keys().cloned().collect();
    fn visit(value: &Value, ids: &BTreeSet<String>) -> Result<()> {
        match value {
            Value::Object(m) => {
                if let Some(id) = m.get("$entity").and_then(Value::as_str) {
                    if !ids.contains(id) {
                        return Err(invalid(format!("referenced entity {id} would be deleted")));
                    }
                }
                for v in m.values() {
                    visit(v, ids)?;
                }
            }
            Value::Array(a) => {
                for v in a {
                    visit(v, ids)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    visit(doc, &ids)
}
fn has_reference(value: &Value, id: &str) -> bool {
    match value {
        Value::Object(m) => {
            m.get("$entity").and_then(Value::as_str) == Some(id)
                || m.values().any(|v| has_reference(v, id))
        }
        Value::Array(a) => a.iter().any(|v| has_reference(v, id)),
        _ => false,
    }
}
fn capture_removed(
    world: &World,
    before: &Value,
    after: &Value,
) -> Vec<crate::editor_commands::EntitySnapshot> {
    let next = entries(after);
    entries(before)
        .keys()
        .filter(|id| !next.contains_key(*id))
        .filter_map(|id| {
            PersistentId::parse_hex(id).and_then(|id| world.entity_by_persistent_id(id))
        })
        .map(|e| crate::editor_commands::EntitySnapshot::capture(world, e))
        .collect()
}
fn restore_runtime(world: &mut World, snapshots: &[crate::editor_commands::EntitySnapshot]) {
    for snapshot in snapshots {
        if snapshot
            .persistent_id
            .is_some_and(|id| world.entity_by_persistent_id(id).is_none())
        {
            snapshot.clone().respawn(world);
        }
    }
}
struct WorldPatch {
    before: Value,
    after: Value,
    label: String,
    before_runtime: Vec<crate::editor_commands::EntitySnapshot>,
    after_runtime: Vec<crate::editor_commands::EntitySnapshot>,
}
impl EditorCommand for WorldPatch {
    fn execute(&mut self, world: &mut World, _selected: &mut Option<Entity>) {
        restore_runtime(world, &self.after_runtime);
        if let Err(e) = apply_delta(world, &self.before, &self.after) {
            tracing::error!(error=%e,"authoring redo failed");
        }
    }
    fn undo(&mut self, world: &mut World, selected: &mut Option<Entity>) {
        self.after_runtime = capture_removed(world, &self.after, &self.before);
        restore_runtime(world, &self.before_runtime);
        if let Err(e) = apply_delta(world, &self.after, &self.before) {
            tracing::error!(error=%e,"authoring undo failed");
        }
        if selected.is_some_and(|e| !world.is_alive(e)) {
            *selected = None;
        }
    }
    fn description(&self) -> &str {
        &self.label
    }
}
/// Patch only changed fields/components on existing entities, preserving live
/// handles and unreflected runtime state. Durable references resolve after creates.
fn apply_delta(
    world: &mut World,
    before: &Value,
    after: &Value,
) -> std::result::Result<(), String> {
    let registry = crate::reflect_registry::editor_registry();
    let previous = entries(before);
    let next = entries(after);
    let changes = changed_ids(before, after);
    for id in &changes {
        if next.contains_key(id) {
            let pid = PersistentId::parse_hex(id).ok_or("invalid persistent id")?;
            if world.entity_by_persistent_id(pid).is_none() {
                world.spawn((pid,));
            }
        }
    }
    for id in &changes {
        let Some(entry) = next.get(id) else {
            continue;
        };
        let pid = PersistentId::parse_hex(id).ok_or("invalid persistent id")?;
        let e = world
            .entity_by_persistent_id(pid)
            .ok_or("target absent after creation")?;
        if !previous.contains_key(id) {
            crate::scene_schema::apply_entity_document(world, &registry, e, entry)?;
        }
        if previous.get(id).map(|v| &v["prefab"]) != Some(&entry["prefab"]) {
            let _ = world.remove_component::<crate::prefab::PrefabMember>(e);
            if !entry["prefab"].is_null() {
                let member =
                    serde_json::from_value::<crate::prefab::PrefabMember>(entry["prefab"].clone())
                        .map_err(|e| e.to_string())?;
                world
                    .insert_component(e, member)
                    .map_err(|e| e.to_string())?;
            }
        }
        let empty = serde_json::Map::new();
        let old_components = previous
            .get(id)
            .and_then(|v| v["components"].as_object())
            .unwrap_or(&empty);
        let components = entry["components"]
            .as_object()
            .ok_or("components object required")?;
        for (name, _) in old_components {
            if !components.contains_key(name) {
                if let Some(s) = registry.by_name(name) {
                    (s.remove)(world, e).map_err(|e| e.to_string())?;
                }
            }
        }
        for (name, body) in components {
            if old_components.get(name) == Some(body) {
                continue;
            }
            let Some(s) = registry.by_name(name) else {
                continue;
            };
            if (s.snapshot)(world, e).is_none() {
                (s.insert_default)(world, e).map_err(|e| e.to_string())?;
            }
            let fields = body["fields"].as_object().ok_or("fields object required")?;
            let old_fields = old_components
                .get(name)
                .and_then(|v| v["fields"].as_object());
            let mut patch = ReflectObject::new();
            for f in &s.fields {
                let Some(value) = fields.get(f.name) else {
                    continue;
                };
                if old_fields.and_then(|old| old.get(f.name)) == Some(value) {
                    continue;
                }
                let parsed = crate::scene_schema::value_from_json(
                    &|id| world.entity_by_persistent_id(id),
                    &f.ty,
                    value,
                )
                .ok_or_else(|| format!("invalid field {name}.{}", f.name))?;
                f.validate(&parsed).map_err(|e| e.to_string())?;
                patch.insert(f.id, parsed);
            }
            (s.apply)(world, e, &patch).map_err(|e| e.to_string())?;
        }
    }
    for id in &changes {
        if !next.contains_key(id) {
            if let Some(e) =
                PersistentId::parse_hex(id).and_then(|id| world.entity_by_persistent_id(id))
            {
                world.despawn(e);
            }
        }
    }
    // Derive Children from authoritative Parent after restores and creates.
    let entities: Vec<_> = world.entities().collect();
    for e in &entities {
        let _ = world.remove_component::<crate::Children>(*e);
    }
    for e in entities {
        if let Some(parent) = world.get::<Parent>(e).map(|p| p.entity) {
            if world.is_alive(parent) {
                crate::editor_commands::do_reparent_entity(world, e, Some(parent));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(revision: u64, operations: Vec<Operation>) -> PlanRequest {
        PlanRequest {
            expected_revision: revision,
            label: "Build room".into(),
            operations,
        }
    }
    fn create(key: &str) -> Operation {
        Operation::Create {
            key: key.into(),
            name: key.into(),
            components: BTreeMap::new(),
        }
    }
    #[test]
    fn invalid_middle_operation_does_not_publish_or_create_history() {
        let mut world = World::new();
        let mut s = AuthoringSession::new("test", "session");
        s.observe(&mut world);
        let result = s.plan(
            &mut world,
            request(
                0,
                vec![
                    create("first"),
                    Operation::Set {
                        entity: "first".into(),
                        component: "somnium.Transform".into(),
                        field: "does_not_exist".into(),
                        value: json!(2),
                    },
                    create("last"),
                ],
            ),
        );
        assert!(result.is_err());
        assert_eq!(world.entity_count(), 0);
        assert_eq!(s.revision, 0);
    }
    #[test]
    fn create_reference_reparent_commit_retry_undo_and_redo() {
        let mut world = World::new();
        let mut s = AuthoringSession::new("test", "session");
        let mut undo = UndoStack::new(100);
        let mut selected = None;
        let p = s
            .plan(
                &mut world,
                request(
                    0,
                    vec![
                        create("parent"),
                        create("child"),
                        Operation::Reparent {
                            entity: "child".into(),
                            parent: Some("parent".into()),
                        },
                    ],
                ),
            )
            .unwrap();
        let args = json!({"request_id":"one","plan_token":p["plan_token"]});
        let result = s
            .commit(&mut world, &mut undo, &mut selected, &args)
            .unwrap();
        assert_eq!(world.entity_count(), 2);
        assert_eq!(undo.history().1, 1);
        assert_eq!(
            s.commit(&mut world, &mut undo, &mut selected, &args)
                .unwrap(),
            result
        );
        assert_eq!(world.entity_count(), 2);
        s.history(
            &mut world,
            &mut undo,
            &mut selected,
            &json!({"action":"undo","expected_revision":1,"expected_cursor":1}),
        )
        .unwrap();
        assert_eq!(world.entity_count(), 0);
        s.history(
            &mut world,
            &mut undo,
            &mut selected,
            &json!({"action":"redo","expected_revision":2,"expected_cursor":0}),
        )
        .unwrap();
        assert_eq!(world.entity_count(), 2);
    }
    #[test]
    fn manual_edit_invalidates_plan_and_keeps_same_live_entity() {
        let mut world = World::new();
        let id = PersistentId::mint();
        let e = world.spawn((id, Name::new("original"), Transform::default()));
        let mut s = AuthoringSession::new("test", "session");
        let mut undo = UndoStack::new(10);
        let mut selected = None;
        let p = s.plan(
            &mut world,
            request(
                0,
                vec![Operation::Set {
                    entity: id.to_string(),
                    component: "somnium.Name".into(),
                    field: "value".into(),
                    value: json!("agent"),
                }],
            ),
        );
        let p = p.unwrap();
        *world.get_mut::<Name>(e).unwrap() = Name::new("designer");
        assert_eq!(
            s.commit(
                &mut world,
                &mut undo,
                &mut selected,
                &json!({"request_id":"one","plan_token":p["plan_token"]})
            )
            .unwrap_err()
            .code,
            "revision_conflict"
        );
        assert_eq!(world.get::<Name>(e).unwrap().as_str(), "designer");
        assert!(world.is_alive(e));
    }
}

/// Authoring includes editable transient fields (preview requests, asset
/// sessions); save_scene_schema still writes only persistent scene fields.
fn authoring_snapshot(world: &mut World) -> Value {
    let registry = crate::reflect_registry::editor_registry();
    let all: Vec<_> = world.entities().collect();
    let mut doc =
        crate::scene_schema::entities_to_json(world, &registry, &all).expect("live world identity");
    for entry in doc["entities"].as_array_mut().expect("entity array") {
        let Some(entity) = entry["persistent_id"]
            .as_str()
            .and_then(PersistentId::parse_hex)
            .and_then(|id| world.entity_by_persistent_id(id))
        else {
            continue;
        };
        for schema in registry.schemas_on(world, entity) {
            let Some(values) = (schema.snapshot)(world, entity) else {
                continue;
            };
            let mut fields = entry["components"]
                .get(schema.stable_id.as_str())
                .and_then(|c| c["fields"].as_object())
                .cloned()
                .unwrap_or_default();
            for f in &schema.fields {
                if f.flags.contains(FieldFlags::EDIT) && !f.read_only {
                    if let Some(v) = values.get(&f.id) {
                        fields.insert(
                            f.name.to_owned(),
                            crate::scene_schema::value_to_json(world, v),
                        );
                    }
                }
            }
            if !fields.is_empty() {
                entry["components"][schema.stable_id.as_str()] =
                    json!({"version":schema.version,"fields":fields});
            }
        }
    }
    doc
}
