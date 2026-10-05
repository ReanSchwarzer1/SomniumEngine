use glam::Vec3;
use somnium_ai::behavior::*;
use somnium_ai::perception::*;
use std::collections::BTreeMap;

#[derive(Default)]
struct Host {
    ticks: Vec<String>,
    cancelled: Vec<String>,
}
impl TaskHost for Host {
    fn tick(&mut self, _: bool, name: &str, _: &mut Blackboard, _: f32) -> Status {
        self.ticks.push(name.into());
        if name == "running" {
            Status::Running
        } else if name == "fail" {
            Status::Failure
        } else {
            Status::Success
        }
    }
    fn cancel(&mut self, _: bool, name: &str, _: &mut Blackboard) {
        self.cancelled.push(name.into());
    }
}
#[test]
fn sequence_resumes_and_completed_tasks_are_not_repeated() {
    let tree = BehaviorTree::new(
        0,
        vec![
            Node::Sequence(vec![1, 2, 3]),
            Node::Task(Task::Native("first".into())),
            Node::Task(Task::Wait { seconds: 1.0 }),
            Node::Task(Task::Set {
                key: "ready".into(),
                value: Value::Bool(true),
            }),
        ],
    )
    .unwrap();
    let mut run = BehaviorInstance::new(BehaviorTree::decode(&tree.encode().unwrap()).unwrap());
    let mut host = Host::default();
    assert_eq!(run.tick(0.5, &mut host), Status::Running);
    assert_eq!(run.tick(0.5, &mut host), Status::Success);
    assert_eq!(host.ticks, vec!["first"]);
    assert_eq!(run.blackboard.get("ready"), Some(&Value::Bool(true)));
    assert_eq!(run.tick(1.0, &mut host), Status::Success);
    assert_eq!(host.ticks.len(), 1);
    run.reset(&mut host);
    assert_eq!(run.tick(0.1, &mut host), Status::Running);
    assert_eq!(host.ticks.len(), 2);
}
#[test]
fn selector_inverter_repeat_and_parallel_cancellation() {
    let tree = BehaviorTree::new(
        0,
        vec![
            Node::Selector(vec![1, 2]),
            Node::Task(Task::Native("fail".into())),
            Node::Repeat { child: 3, count: 2 },
            Node::Invert(4),
            Node::Task(Task::Native("fail".into())),
        ],
    )
    .unwrap();
    let mut run = BehaviorInstance::new(tree);
    let mut host = Host::default();
    assert_eq!(run.tick(0.1, &mut host), Status::Running);
    assert_eq!(run.tick(0.1, &mut host), Status::Success);
    assert_eq!(host.ticks.len(), 3);
    let tree = BehaviorTree::new(
        0,
        vec![
            Node::Parallel {
                children: vec![1, 2],
                successes: 1,
            },
            Node::Task(Task::Native("running".into())),
            Node::Task(Task::Native("ok".into())),
        ],
    )
    .unwrap();
    let mut run = BehaviorInstance::new(tree);
    assert_eq!(run.tick(0.1, &mut host), Status::Success);
    assert_eq!(host.cancelled, vec!["running"]);
}
#[test]
fn timeout_aborts_running_task_once_and_validation_rejects_bad_graphs() {
    let tree = BehaviorTree::new(
        0,
        vec![
            Node::Timeout {
                child: 1,
                seconds: 0.5,
            },
            Node::Task(Task::Native("running".into())),
        ],
    )
    .unwrap();
    let mut run = BehaviorInstance::new(tree);
    let mut host = Host::default();
    assert_eq!(run.tick(0.25, &mut host), Status::Running);
    assert_eq!(run.tick(0.25, &mut host), Status::Failure);
    run.tick(0.25, &mut host);
    assert_eq!(host.cancelled, vec!["running"]);
    assert!(BehaviorTree::new(0, vec![Node::Invert(0)]).is_err());
    assert!(
        BehaviorTree::new(
            0,
            vec![
                Node::Sequence(vec![1, 1]),
                Node::Task(Task::Wait { seconds: 1.0 })
            ]
        )
        .is_err()
    );
    assert!(BehaviorTree::new(0, vec![Node::Task(Task::Wait { seconds: f32::NAN })]).is_err());
}
#[test]
fn sight_occlusion_hearing_falloff_and_memory_work_together() {
    let observer = Observer {
        position: Vec3::new(0.0, 1.0, 0.0),
        forward: Vec3::X,
        sight_range: 10.0,
        half_angle_degrees: 45.0,
        hearing_range: 10.0,
    };
    let target = Vec3::new(5.0, 1.0, 0.0);
    let wall = [
        [[2.0, 0.0, -1.0], [2.0, 3.0, -1.0], [2.0, 3.0, 1.0]],
        [[2.0, 0.0, -1.0], [2.0, 3.0, 1.0], [2.0, 0.0, 1.0]],
    ];
    assert!(
        observer
            .see(1, target, 0.0, |a, b| occluded(&wall, a, b))
            .is_none()
    );
    assert!(observer.see(1, -target, 0.0, |_, _| false).is_none());
    let visible = observer.see(1, target, 0.0, |_, _| false).unwrap();
    let hearing = observer.hear(1, target, 1.0, 1.0).unwrap();
    assert!((hearing.strength - 0.25).abs() < 0.001);
    let mut memory = StimulusMemory::new(2, 2.0).unwrap();
    memory.remember(visible);
    memory.remember(hearing);
    assert_eq!(memory.len(), 2);
    assert!(memory.strongest(1.0).is_some());
    memory.expire(2.1);
    assert_eq!(memory.len(), 1);
    assert_eq!(memory.strongest(2.1).unwrap().sense, Sense::Hearing);
    memory.expire(3.1);
    assert!(memory.is_empty());
    assert!(memory.strongest(3.1).is_none());
}

struct EmptyWorld;
impl somnium_script::snapshot::WorldView for EmptyWorld {
    fn read_field_id(
        &self,
        _: somnium_ecs::Entity,
        _: somnium_ecs::StableId,
        _: somnium_ecs::FieldId,
    ) -> Option<somnium_script::value::ScriptValue> {
        None
    }
    fn script_fields(&self, _: somnium_ecs::StableId) -> Vec<(String, somnium_ecs::FieldId, bool)> {
        vec![]
    }
    fn field_type(
        &self,
        _: somnium_ecs::StableId,
        _: somnium_ecs::FieldId,
    ) -> Option<somnium_ecs::reflect::FieldType> {
        None
    }
    fn is_alive(&self, _: somnium_ecs::Entity) -> bool {
        false
    }
    fn read_component(
        &self,
        _: somnium_ecs::Entity,
        _: somnium_ecs::StableId,
    ) -> Option<somnium_ecs::ReflectObject> {
        None
    }
    fn read_field(
        &self,
        _: somnium_ecs::Entity,
        _: somnium_ecs::StableId,
        _: &str,
    ) -> Option<somnium_script::value::ScriptValue> {
        None
    }
    fn persistent_id(&self, _: somnium_ecs::Entity) -> Option<somnium_ecs::PersistentId> {
        None
    }
    fn entity_by_persistent_id(&self, _: somnium_ecs::PersistentId) -> Option<somnium_ecs::Entity> {
        None
    }
    fn components_on(&self, _: somnium_ecs::Entity) -> Vec<somnium_ecs::StableId> {
        vec![]
    }
    fn component_by_name(&self, _: &str) -> Option<somnium_ecs::StableId> {
        None
    }
    fn field_by_name(&self, _: somnium_ecs::StableId, _: &str) -> Option<somnium_ecs::FieldId> {
        None
    }
    fn is_field_writable(&self, _: somnium_ecs::StableId, _: &str) -> bool {
        false
    }
}
#[test]
fn real_luau_task_updates_blackboard_and_preserves_state_between_ticks() {
    use somnium_ai::script::{ScriptTaskBinding, ScriptTaskHost};
    use somnium_script::{
        backend::{Budget, ScriptBackend, ScriptSource},
        command::CommandBuffer,
        ids::{InstanceUuid, LanguageTag, ScriptAssetId, ScriptInstanceId},
        order::OrderKey,
        snapshot::ScriptSnapshot,
    };
    let mut backend = somnium_script_luau::LuauBackend::new(Budget::default()).unwrap();
    let module=backend.compile(&ScriptSource {id:ScriptAssetId::mint(),language:LanguageTag::LUAU,display_path:"ai_task.luau".into(),text:r#"
        return Script.define({
            loadState=function(self,state) self.count=state.count or 0;self.board=state.blackboard;self.status=state.status end,
            onEvent=function(self,ctx,events)
                if events[1].name == "ai.cancel" then self.status="failure"; return end
                self.count+=1
                self.board.alert=true
                self.status=if self.count>=2 then "success" else "running"
            end,
            saveState=function(self) return {count=self.count or 0,blackboard=self.board,status=self.status} end
        })
    "#.into()}).unwrap();
    let id = ScriptInstanceId::next();
    backend.instantiate(id, module, &BTreeMap::new()).unwrap();
    let persistent = somnium_ecs::PersistentId::from_raw(1);
    let bindings = BTreeMap::from([(
        "alert".into(),
        ScriptTaskBinding {
            instance: id,
            order: OrderKey::new(0, persistent, InstanceUuid::mint()),
        },
    )]);
    let snapshot = ScriptSnapshot {
        time: Default::default(),
        input: Default::default(),
        self_entity: somnium_ecs::Entity::DANGLING,
        self_persistent: persistent,
        self_components: BTreeMap::new(),
        spawn_results: vec![],
        events: vec![],
        rng_seed: 1,
    };
    let mut commands = CommandBuffer::new();
    let mut host = ScriptTaskHost {
        backend: &mut backend,
        bindings: &bindings,
        snapshot: &snapshot,
        world: &EmptyWorld,
        commands: &mut commands,
        diagnostics: vec![],
    };
    let tree = BehaviorTree::new(0, vec![Node::Task(Task::Script("alert".into()))]).unwrap();
    let mut run = BehaviorInstance::new(tree);
    assert_eq!(
        run.tick(0.1, &mut host),
        Status::Running,
        "{:?}",
        host.diagnostics
    );
    assert_eq!(
        run.tick(0.1, &mut host),
        Status::Success,
        "{:?}",
        host.diagnostics
    );
    assert_eq!(run.blackboard.get("alert"), Some(&Value::Bool(true)));
    assert!(host.diagnostics.is_empty());
}
