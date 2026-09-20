//! Public work/mirror authoring examples using only shared engine contracts.
use glam::Vec3;
use serde_json::{Value, json};
use somnium_core::{
    EngineContext, EngineEvent, InputState, KeyCode, MeshKind, Name, Parent, Transform,
    WorldTransform,
};
use somnium_core::{
    staged_mirror::{StagedMirror, alcove_boxes},
    work::WorkTarget,
};
use somnium_ecs::{Component, Entity, World, component_schema, reflect::TypeRegistry};
#[derive(Clone, Copy, Default)]
pub struct MirrorPart {
    pub role: u32,
}
impl Component for MirrorPart {}
fn schemas(r: &mut TypeRegistry) {
    r.register(component_schema!{MirrorPart as "hello.MirrorPart",display "Mirror Assembly Part",version 1,fields{role {group:"Assembly"}}});
}
pub fn register(r: &mut somnium_core::authoring::GameRegistration) {
    r.components.push(schemas);
    for (id, label, build) in [
        (
            "hello.work_demo",
            "Burn / Repair Demo",
            spawn_work as fn(&mut World, &Value) -> Result<Vec<Entity>, String>,
        ),
        ("hello.mirror_demo", "Staged Mirror Demo", spawn_mirror),
        ("hello.dream_lens", "Dream Lens Demo", spawn_dream_lens),
    ] {
        r.presets.push(somnium_core::authoring::GamePreset{id,label,category:"Examples",schema:json!({"type":"object","properties":{"position":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3}},"additionalProperties":false}),build});
    }
}
fn origin(args: &Value) -> Result<Vec3, String> {
    let p = Vec3::from_array(
        serde_json::from_value(args.get("position").cloned().unwrap_or(json!([0, 1.4, 0])))
            .map_err(|e| format!("{e}"))?,
    );
    if !p.is_finite() {
        return Err("Position must be finite".into());
    }
    Ok(p)
}
fn spawn_work(world: &mut World, args: &Value) -> Result<Vec<Entity>, String> {
    let at = origin(args)?;
    Ok((0..2)
        .map(|kind| {
            world.spawn((
                Name::new(if kind == 0 {
                    "Burn demo — hold F + E"
                } else {
                    "Repair demo — hold E"
                }),
                Transform {
                    translation: at + Vec3::X * (kind as f32 * 1.6 - 0.8),
                    scale: Vec3::new(0.65, 0.65, 0.1),
                    ..Default::default()
                },
                WorldTransform::identity(),
                MeshKind::Cube,
                WorkTarget {
                    kind,
                    reach: 2.0,
                    prompt: if kind == 0 {
                        "Burn".into()
                    } else {
                        "Repair".into()
                    },
                    ..Default::default()
                },
            ))
        })
        .collect())
}
fn spawn_mirror(world: &mut World, args: &Value) -> Result<Vec<Entity>, String> {
    let at = origin(args)?;
    let mirror = StagedMirror::default();
    let root = world.spawn((
        Name::new("Staged Mirror"),
        Transform::from_translation(at),
        WorldTransform::identity(),
        mirror,
    ));
    let mut entities = vec![root];
    for (role, (name, position, scale)) in alcove_boxes(mirror).into_iter().enumerate() {
        entities.push(world.spawn((
            Name::new(name),
            Transform {
                translation: position,
                scale,
                ..Default::default()
            },
            WorldTransform::identity(),
            Parent { entity: root },
            MeshKind::Cube,
            MirrorPart { role: role as u32 },
        )));
    }
    entities.push(world.spawn((
        Name::new("Reflected animation sample"),
        Transform::from_translation(at + Vec3::new(0.0, -1.4, 1.5)),
        WorldTransform::identity(),
        crate::animated_preview::AnimatedPreview::default(),
    )));
    Ok(entities)
}
#[derive(Default)]
pub struct Demo {
    held: bool,
    flame: bool,
    pub status: String,
}
impl Demo {
    pub fn event(&mut self, event: &EngineEvent) {
        match event {
            EngineEvent::KeyInput {
                key: KeyCode::KeyE,
                state,
            } => self.held = *state == InputState::Pressed,
            EngineEvent::KeyInput {
                key: KeyCode::KeyF,
                state,
            } => self.flame = *state == InputState::Pressed,
            EngineEvent::WindowFocused(false) => {
                self.held = false;
                self.flame = false;
            }
            _ => {}
        }
    }
    pub fn reset(&mut self, world: &mut World) {
        self.held = false;
        self.flame = false;
        self.status.clear();
        let entities: Vec<_> = world
            .entities()
            .filter(|e| world.get::<WorkTarget>(*e).is_some())
            .collect();
        for e in entities {
            world.get_mut::<WorkTarget>(e).unwrap().reset();
        }
    }
    pub fn tick(&mut self, ctx: &mut EngineContext, eye: Vec3, forward: Vec3) {
        let frame = somnium_core::work::update(ctx, eye, forward, self.held, self.flame, true);
        self.status = frame.prompt;
        for e in frame.completed {
            if let Some(name) = ctx.world.get::<Name>(e) {
                ctx.ui.push_toast(&format!("{} complete", name.as_str()));
            }
        }
    }
    pub fn sync(world: &mut World) {
        let parts: Vec<_> = world
            .entities()
            .filter_map(|e| {
                Some((
                    e,
                    world.get::<MirrorPart>(e)?.role,
                    world.get::<Parent>(e)?.entity,
                ))
            })
            .collect();
        for (e, role, parent) in parts {
            if let Some(m) = world.get::<StagedMirror>(parent).copied() {
                if let Some((_, p, s)) = alcove_boxes(m).get(role as usize) {
                    if let Some(t) = world.get_mut::<Transform>(e) {
                        t.translation = *p;
                        t.scale = *s;
                    }
                }
            }
        }
    }
}

fn spawn_dream_lens(world: &mut World, _args: &Value) -> Result<Vec<Entity>, String> {
    let existing = world.entities().find(|e| world.get::<somnium_core::PostProcessComponent>(*e).is_some());
    let entity = existing.unwrap_or_else(|| world.spawn((Name::new("Dream Lens"), somnium_core::PostProcessComponent::default())));
    let pp = world.get_mut::<somnium_core::PostProcessComponent>(entity).unwrap();
    pp.dream_mode = 2;
    pp.dream_strength = 0.35;
    pp.dream_speed = 0.65;
    Ok(vec![entity])
}
