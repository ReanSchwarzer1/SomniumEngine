//! Public example of shared designer interactions. No private game code or assets.
use glam::{Quat, Vec3};
use serde_json::{Value, json};
use somnium_core::interaction::{Interactable, InteractionEvent, InteractionSession};
use somnium_core::{
    EditorFlags, EngineContext, LightComponent, MeshKind, Name, Transform, WorldTransform,
};
use somnium_ecs::{Entity, World};

pub fn register(r: &mut somnium_core::authoring::GameRegistration) {
    r.label = "Hello Engine".into();
    r.presets.push(somnium_core::authoring::GamePreset{id:"hello.interaction_demo",label:"Interaction Demo",category:"Examples",schema:json!({"type":"object","properties":{"position":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3}},"additionalProperties":false}),build:spawn});
}
fn spawn(world: &mut World, args: &Value) -> Result<Vec<Entity>, String> {
    let origin: Vec3 = Vec3::from_array(
        serde_json::from_value(args.get("position").cloned().unwrap_or(json!([0, 1, 0])))
            .map_err(|_| "position needs [x,y,z]")?,
    );
    if !origin.is_finite() {
        return Err("position must be finite".into());
    }
    let mut entities = vec![];
    for (kind, name, scale) in [
        (0, "Demo Door", Vec3::new(0.8, 2.0, 0.15)),
        (1, "Demo Collectible", Vec3::splat(0.25)),
        (2, "Demo Inspection Card", Vec3::new(0.4, 0.3, 0.04)),
        (3, "Demo Machine", Vec3::splat(0.5)),
        (4, "Demo Portable Light", Vec3::new(0.15, 0.3, 0.15)),
    ] {
        let entity = world.spawn((
            Name::new(name),
            Transform {
                translation: origin + Vec3::X * (kind as f32 - 2.0) * 1.1,
                scale,
                rotation: Quat::IDENTITY,
            },
            WorldTransform::identity(),
            MeshKind::Cube,
            Interactable {
                kind,
                prompt: format!(
                    "E: {}",
                    [
                        "Open door",
                        "Collect",
                        "Inspect / return",
                        "Operate",
                        "Toggle light"
                    ][kind as usize]
                ),
                trigger: format!("hello.demo.{kind}"),
                reach: 3.0,
                ..Default::default()
            },
        ));
        if kind == 4 {
            world
                .insert_component(entity, LightComponent::point(0.0, 4.0))
                .map_err(|e| format!("{e:?}"))?;
        }
        entities.push(entity);
    }
    Ok(entities)
}

pub struct InteractionDemo {
    session: InteractionSession,
    target: Option<(Entity, Interactable, Transform)>,
    request: bool,
    pub status: String,
    pub completed: u64,
    before_play: Vec<(
        Entity,
        Transform,
        Interactable,
        Option<EditorFlags>,
        Option<LightComponent>,
    )>,
}
impl Default for InteractionDemo {
    fn default() -> Self {
        Self{session:Default::default(),target:None,request:false,status:"Create Interaction Demo in Game Authoring; Play, look at an object, press E. Details also provides Preview.".into(),completed:0,before_play:vec![]}
    }
}
impl InteractionDemo {
    pub fn begin_play(&mut self, world: &World) {
        self.before_play = world
            .entities()
            .filter_map(|e| {
                Some((
                    e,
                    *world.get::<Transform>(e)?,
                    world.get::<Interactable>(e)?.clone(),
                    world.get::<EditorFlags>(e).copied(),
                    world.get::<LightComponent>(e).copied(),
                ))
            })
            .collect();
    }
    pub fn end_play(&mut self, world: &mut World) {
        for (e, transform, mut config, flags, light) in self.before_play.drain(..) {
            if !world.is_alive(e) {
                continue;
            }
            config.preview_requested = false;
            let _ = world.insert_component(e, transform);
            let _ = world.insert_component(e, config);
            if let Some(flags) = flags {
                let _ = world.insert_component(e, flags);
            } else {
                let _ = world.remove_component::<EditorFlags>(e);
            }
            if let Some(light) = light {
                let _ = world.insert_component(e, light);
            }
        }
        self.session = Default::default();
        self.target = None;
        self.request = false;
        self.status = "Stopped; authored demo state restored".into();
    }

    pub fn request(&mut self) {
        if self.session.busy() {
            self.session.cancel();
        } else {
            self.request = true;
        }
    }
    pub fn cancel(&mut self) {
        self.request = false;
        self.session.cancel();
    }
    pub fn state(&self) -> Value {
        json!({"status":self.status,"completed":self.completed,"hands_busy":self.session.busy(),"holding":self.session.holding})
    }
    pub fn tick(&mut self, ctx: &mut EngineContext, eye: Vec3, forward: Vec3) {
        if !self.session.busy() {
            let request = std::mem::take(&mut self.request);
            let target = ctx
                .world
                .entities()
                .filter_map(|e| {
                    let config = ctx.world.get::<Interactable>(e)?;
                    let transform = *ctx.world.get::<Transform>(e)?;
                    let delta = transform.translation + config.anchor - eye;
                    let distance = delta.length();
                    (config.preview_requested
                        || (request
                            && distance <= config.reach
                            && delta.normalize_or_zero().dot(forward) > 0.8))
                        .then_some((e, config.clone(), transform, distance))
                })
                .min_by(|a, b| a.3.total_cmp(&b.3));
            if let Some((entity, config, transform, _)) = target {
                if let Some(c) = ctx.world.get_mut::<Interactable>(entity) {
                    c.preview_requested = false;
                }
                match self.session.begin(&config) {
                    Ok(()) => {
                        self.status = config.prompt.clone();
                        self.target = Some((entity, config, transform));
                    }
                    Err(e) => self.status = e.into(),
                }
            }
        }
        let holding = self.target.as_ref().is_some_and(|(_, c, _)| c.kind == 2);
        if let Some(event) = self
            .session
            .tick(ctx.simulation.fixed_delta_seconds, holding)
        {
            if let Some((entity, config, original)) = self.target.as_ref() {
                if event == InteractionEvent::Contact {
                    self.completed += 1;
                    match config.kind {
                        0 => {
                            if let Some(t) = ctx.world.get_mut::<Transform>(*entity) {
                                t.rotation *= Quat::from_rotation_y(config.open_angle.to_radians());
                            }
                        }
                        1 => {
                            let _ = ctx.world.insert_component(
                                *entity,
                                EditorFlags {
                                    hidden: true,
                                    ..Default::default()
                                },
                            );
                            if let Some(c) = ctx.world.get_mut::<Interactable>(*entity) {
                                c.enabled = false;
                            }
                        }
                        2 => {
                            if let Some(t) = ctx.world.get_mut::<Transform>(*entity) {
                                t.translation = eye + forward * 0.65;
                            }
                        }
                        3 => {
                            if let Some(t) = ctx.world.get_mut::<Transform>(*entity) {
                                t.rotation *= Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
                            }
                        }
                        4 => {
                            if let Some(light) = ctx.world.get_mut::<LightComponent>(*entity) {
                                light.intensity = if light.intensity > 0.0 { 0.0 } else { 200.0 };
                            }
                        }
                        _ => {}
                    }
                    self.status = format!(
                        "{}: contact {}{}",
                        config.trigger,
                        self.completed,
                        if holding { " (E to return)" } else { "" }
                    );
                    tracing::info!("{}", self.status);
                } else {
                    if config.kind == 2 {
                        if let Some(t) = ctx.world.get_mut::<Transform>(*entity) {
                            *t = *original;
                        }
                    }
                    self.target = None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn demo_preset_exposes_five_shared_actions() {
        let mut world = World::new();
        let entities = spawn(&mut world, &json!({"position":[10,2,0]})).unwrap();
        assert_eq!(entities.len(), 5);
        for (kind, e) in entities.into_iter().enumerate() {
            assert_eq!(world.get::<Interactable>(e).unwrap().kind, kind as u32);
            assert!(world.get::<MeshKind>(e).is_some());
        }
    }
}
