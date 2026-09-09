//! Behavior task adapter over the existing script backend and its budgets.
//!
//! Task modules implement `loadState`, `onEvent`, and `saveState`. The adapter
//! preserves the module's own state, supplies `blackboard` and `delta`, and
//! sends `ai.tick` / `ai.cancel`. `saveState` returns `status` as "running",
//! "success" or "failure" plus an optional updated `blackboard`. No VM is
//! created here and no alternate unbudgeted script execution path is added.
use crate::behavior::{Blackboard, Status, TaskHost, Value};
use somnium_script::{
    backend::{Callback, ScriptBackend},
    command::CommandBuffer,
    ids::ScriptInstanceId,
    order::OrderKey,
    snapshot::{ScriptEvent, ScriptSnapshot, WorldView},
    value::ScriptValue,
};
use std::collections::BTreeMap;

pub struct ScriptTaskBinding {
    pub instance: ScriptInstanceId,
    pub order: OrderKey,
}
pub struct ScriptTaskHost<'a> {
    pub backend: &'a mut dyn ScriptBackend,
    pub bindings: &'a BTreeMap<String, ScriptTaskBinding>,
    pub snapshot: &'a ScriptSnapshot,
    pub world: &'a dyn WorldView,
    pub commands: &'a mut CommandBuffer,
    pub diagnostics: Vec<String>,
}
impl ScriptTaskHost<'_> {
    fn run(
        &mut self,
        name: &str,
        board: &mut Blackboard,
        delta: f32,
        cancel: bool,
    ) -> Result<Status, String> {
        let binding = self
            .bindings
            .get(name)
            .ok_or_else(|| format!("unbound AI script task: {name}"))?;
        let state = self
            .backend
            .export_state(binding.instance)
            .map_err(|e| format!("{e:?}"))?;
        let mut state = match state {
            ScriptValue::Map(v) => v,
            ScriptValue::Nil => BTreeMap::new(),
            _ => return Err("AI task state must be a named record".into()),
        };
        state.insert(
            "blackboard".into(),
            ScriptValue::Map(
                board
                    .iter()
                    .map(|(key, v)| (key.clone(), to_script(v)))
                    .collect(),
            ),
        );
        state.insert("delta".into(), ScriptValue::F64(f64::from(delta)));
        // Clear the previous result so a missing callback cannot inherit success.
        state.insert("status".into(), ScriptValue::Str("running".into()));
        self.backend
            .import_state(binding.instance, ScriptValue::Map(state))
            .map_err(|e| format!("{e:?}"))?;
        let mut snapshot = self.snapshot.clone();
        snapshot.events = vec![ScriptEvent {
            name: if cancel { "ai.cancel" } else { "ai.tick" }.into(),
            sequence: snapshot.time.step,
            source: None,
            payload: BTreeMap::new(),
        }];
        self.backend
            .invoke(
                binding.instance,
                binding.order,
                Callback::Event,
                &snapshot,
                self.world,
                self.commands,
            )
            .map_err(|e| format!("{e:?}"))?;
        let ScriptValue::Map(state) = self
            .backend
            .export_state(binding.instance)
            .map_err(|e| format!("{e:?}"))?
        else {
            return Err("AI task must export state".into());
        };
        let status = match state.get("status") {
            Some(ScriptValue::Str(s)) if s == "success" => Status::Success,
            Some(ScriptValue::Str(s)) if s == "failure" => Status::Failure,
            Some(ScriptValue::Str(s)) if s == "running" => Status::Running,
            _ => return Err("AI script task returned an invalid status".into()),
        };
        if let Some(ScriptValue::Map(values)) = state.get("blackboard") {
            if values.len() > 4096 {
                return Err("AI blackboard exceeds entry budget".into());
            }
            let updated: Result<Blackboard, String> = values
                .iter()
                .map(|(key, v)| from_script(v).map(|v| (key.clone(), v)))
                .collect();
            *board = updated?;
        }
        Ok(status)
    }
}
impl TaskHost for ScriptTaskHost<'_> {
    fn tick(&mut self, script: bool, name: &str, board: &mut Blackboard, delta: f32) -> Status {
        if !script {
            self.diagnostics
                .push(format!("native task {name} needs a native host"));
            return Status::Failure;
        }
        match self.run(name, board, delta, false) {
            Ok(s) => s,
            Err(e) => {
                self.diagnostics.push(e);
                Status::Failure
            }
        }
    }
    fn cancel(&mut self, script: bool, name: &str, board: &mut Blackboard) {
        if script {
            if let Err(e) = self.run(name, board, 0.0, true) {
                self.diagnostics.push(e);
            }
        }
    }
}

/// Run an attached task through the engine's existing instance scheduler. This
/// retains lifecycle eligibility, private-event isolation and quarantine while
/// sharing the same state/blackboard protocol as `ScriptTaskHost`.
pub fn invoke_runtime_task(
    runtime: &mut somnium_script::runtime::ScriptRuntime,
    instance: somnium_script::ids::InstanceUuid,
    board: &mut Blackboard,
    phase: &somnium_script::runtime::PhaseInput,
    world: &dyn WorldView,
    commands: &mut CommandBuffer,
    cancel: bool,
) -> Result<Status, String> {
    let result = (|| {
        let previous = runtime.export_state(instance).map_err(|e| e.to_string())?;
        let mut state = match previous {
            ScriptValue::Map(state) => state,
            ScriptValue::Nil => BTreeMap::new(),
            _ => return Err("AI task state must be a named record".into()),
        };
        state.insert(
            "blackboard".into(),
            ScriptValue::Map(
                board
                    .iter()
                    .map(|(key, value)| (key.clone(), to_script(value)))
                    .collect(),
            ),
        );
        state.insert(
            "delta".into(),
            ScriptValue::F64(f64::from(phase.time.delta)),
        );
        state.insert("status".into(), ScriptValue::Str("running".into()));
        runtime
            .import_state(instance, ScriptValue::Map(state))
            .map_err(|e| e.to_string())?;
        runtime
            .invoke_instance_event(
                instance,
                if cancel { "ai.cancel" } else { "ai.tick" },
                phase,
                world,
                commands,
            )
            .map_err(|e| e.to_string())?;
        let ScriptValue::Map(state) = runtime.export_state(instance).map_err(|e| e.to_string())?
        else {
            return Err(
                "AI task must implement loadState and saveState returning a named record".into(),
            );
        };
        let status = match state.get("status") {
            Some(ScriptValue::Str(s)) if s == "running" => Status::Running,
            Some(ScriptValue::Str(s)) if s == "success" => Status::Success,
            Some(ScriptValue::Str(s)) if s == "failure" => Status::Failure,
            _ => return Err("AI script task returned an invalid status".into()),
        };
        if let Some(ScriptValue::Map(values)) = state.get("blackboard") {
            if values.len() > 4096 {
                return Err("AI blackboard exceeds entry budget".into());
            }
            let updated: Result<Blackboard, String> = values
                .iter()
                .map(|(key, value)| from_script(value).map(|value| (key.clone(), value)))
                .collect();
            *board = updated?;
        }
        Ok(status)
    })();
    if result.is_err() {
        let orders: Vec<_> = commands
            .queued()
            .iter()
            .filter(|command| command.order.instance == instance)
            .map(|command| command.order)
            .collect();
        for order in orders {
            commands.discard_from(order);
        }
    }
    result
}
fn to_script(value: &Value) -> ScriptValue {
    match value {
        Value::Bool(v) => ScriptValue::Bool(*v),
        Value::Number(v) => ScriptValue::F64(*v),
        Value::Text(v) => ScriptValue::Str(v.clone()),
        Value::Position(v) => ScriptValue::Vec3(*v),
        // Never narrow durable identity to Luau's 53-bit number representation.
        Value::Entity(v) => ScriptValue::Map(BTreeMap::from([(
            "$entity".into(),
            ScriptValue::Str(v.to_string()),
        )])),
    }
}
fn from_script(value: &ScriptValue) -> Result<Value, String> {
    if !value.is_finite() {
        return Err("AI task produced a nonfinite value".into());
    }
    Ok(match value {
        ScriptValue::Bool(v) => Value::Bool(*v),
        ScriptValue::F64(v) => Value::Number(*v),
        ScriptValue::I64(v) => Value::Number(*v as f64),
        ScriptValue::Str(v) => Value::Text(v.clone()),
        ScriptValue::Vec3(v) => Value::Position(*v),
        ScriptValue::Map(v) if v.len() == 1 => match v.get("$entity") {
            Some(ScriptValue::Str(id)) => {
                Value::Entity(id.parse().map_err(|_| "invalid AI entity identity")?)
            }
            _ => return Err("unsupported AI blackboard record".into()),
        },
        _ => return Err("unsupported AI blackboard value".into()),
    })
}
