//! Designer behavior tasks resolve scripts already attached to their actor.
//! This is a child of ScriptHost so VM ownership and command commit stay private.
use super::*;
use somnium_ai::behavior::{BehaviorInstance, Blackboard, Status, TaskHost, Value};

type Bindings = BTreeMap<String, Result<InstanceUuid, String>>;
struct PreviousBindings(Bindings);
impl somnium_ecs::Component for PreviousBindings {}

fn aliases(path: &str) -> Vec<String> {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    let path = path.trim_start_matches("./");
    let filename = path.rsplit('/').next().unwrap_or(path);
    let stem = |value: &str| {
        value
            .strip_suffix(".luau")
            .or_else(|| value.strip_suffix(".lua"))
            .unwrap_or(value)
            .to_owned()
    };
    vec![
        path.to_owned(),
        filename.to_owned(),
        stem(path),
        stem(filename),
    ]
}

struct ActorTasks<'a> {
    runtime: &'a mut ScriptRuntime,
    bindings: &'a Bindings,
    phase: &'a PhaseInput,
    view: EngineWorldView<'a>,
    commands: &'a mut CommandBuffer,
    errors: &'a mut Vec<(InstanceUuid, String)>,
    calls: &'a mut u32,
}
impl ActorTasks<'_> {
    fn invoke(
        &mut self,
        name: &str,
        board: &mut Blackboard,
        dt: f32,
        cancel: bool,
    ) -> Result<Status, String> {
        let key = name.trim().replace('\\', "/").to_ascii_lowercase();
        let instance=self.bindings.get(&key).ok_or_else(||format!("No attached script matches '{name}'. Attach it in Scripts Details, then use its filename or full path."))?.as_ref().map_err(Clone::clone)?;
        let mut phase = self.phase.clone();
        phase.time.delta = dt;
        *self.calls = self.calls.saturating_add(1);
        let result = somnium_ai::script::invoke_runtime_task(
            self.runtime,
            *instance,
            board,
            &phase,
            &self.view,
            self.commands,
            cancel,
        );
        if let Err(error) = &result {
            self.errors.push((*instance, error.clone()));
        }
        result
    }
}
impl TaskHost for ActorTasks<'_> {
    fn tick(&mut self, script: bool, name: &str, board: &mut Blackboard, dt: f32) -> Status {
        let result = if script {
            self.invoke(name, board, dt, false)
        } else {
            match name {
                "succeed" => Ok(Status::Success),
                "fail" => Ok(Status::Failure),
                _ => Err(format!(
                    "Unknown native task '{name}'; use succeed/fail or an attached Script task"
                )),
            }
        };
        match result {
            Ok(status) => {
                board.remove("task_error");
                status
            }
            Err(error) => {
                board.insert("task_error".into(), Value::Text(error));
                Status::Failure
            }
        }
    }
    fn cancel(&mut self, script: bool, name: &str, board: &mut Blackboard) {
        if script && let Err(error) = self.invoke(name, board, 0.0, true) {
            board.insert("task_error".into(), Value::Text(error));
        }
    }
}

impl ScriptHost {
    fn behavior_bindings(&self, world: &World, entity: Entity) -> Bindings {
        let mut bindings = Bindings::new();
        let Some(scripts) = world.get::<ScriptSet>(entity) else {
            return bindings;
        };
        for attachment in &scripts.attachments {
            let path = self
                .runtime
                .asset_source(attachment.asset)
                .map(|source| source.display_path.as_str())
                .or_else(|| {
                    self.script_paths
                        .get(&attachment.asset)
                        .and_then(|path| path.to_str())
                });
            let Some(path) = path else {
                continue;
            };
            let binding = if attachment.enabled {
                Ok(attachment.instance)
            } else {
                Err(format!(
                    "Attached script '{path}' is disabled. Enable it in Scripts Details."
                ))
            };
            for alias in aliases(path) {
                match bindings.entry(alias) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(binding.clone());
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        if entry.get() != &binding {
                            let _ = entry.insert(Err(format!("Multiple attached scripts match '{}'; use a unique full path or remove the duplicate attachment",entry.key())));
                        }
                    }
                }
            }
        }
        bindings
    }

    /// Tick authored behavior graphs with their actor's existing script
    /// attachments. Script task names accept a filename, stem or full path;
    /// ambiguous/disabled/missing bindings report errors in Behavior Details.
    /// Call after `sync`, using the same clock/input/services as script updates.
    /// Every task runs in the current VM and commits through the normal ordered
    /// capability and schema validation path after all actor reads complete.
    pub fn tick_behaviors(
        &mut self,
        world: &mut World,
        time: TimeSnapshot,
        input: &InputSnapshot,
        services: &mut HostServices<'_>,
    ) -> Vec<(Entity, Status)> {
        let started = std::time::Instant::now();
        let phase = PhaseInput {
            time,
            input: input.clone(),
        };
        let mut entities: Vec<_> = world
            .entities()
            .filter(|entity| world.get::<crate::ai::BehaviorComponent>(*entity).is_some())
            .collect();
        for &entity in &entities {
            let _ = world.ensure_persistent_id(entity);
        }
        entities.sort_by_key(|entity| world.persistent_id(*entity));
        let mut commands = CommandBuffer::new();
        let mut errors = Vec::new();
        let mut calls = 0;
        let mut result = Vec::new();
        for entity in entities {
            let bindings = self.behavior_bindings(world, entity);
            if world
                .get::<PreviousBindings>(entity)
                .is_none_or(|previous| previous.0 != bindings)
            {
                crate::ai::invalidate_behavior_execution(world, entity);
                let _ = world.insert_component(entity, PreviousBindings(bindings.clone()));
            }
            let status = crate::ai::tick_behavior_scoped(
                world,
                entity,
                |view, instance: &mut BehaviorInstance, reset| {
                    let mut host = ActorTasks {
                        runtime: &mut self.runtime,
                        bindings: &bindings,
                        phase: &phase,
                        view: EngineWorldView::new(view, &self.registry),
                        commands: &mut commands,
                        errors: &mut errors,
                        calls: &mut calls,
                    };
                    if reset {
                        instance.reset(&mut host);
                        Status::Failure
                    } else {
                        instance.tick(time.delta, &mut host)
                    }
                },
            );
            if let Some(status) = status {
                result.push((entity, status));
            }
        }
        for (instance, error) in &errors {
            self.logs.push(ScriptLogLine {
                level: LogLevel::Error,
                instance: *instance,
                message: format!("Behavior task: {error}"),
            });
        }
        self.commit_commands(world, &mut commands, services);
        self.stats.calls = self.stats.calls.saturating_add(calls);
        self.stats.errors = self.stats.errors.saturating_add(errors.len() as u32);
        self.stats.update_ms += started.elapsed().as_secs_f32() * 1000.0;
        result
    }
}
