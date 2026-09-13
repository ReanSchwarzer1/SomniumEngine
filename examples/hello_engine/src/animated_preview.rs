//! Imported animation example: authored settings stay in ECS; GPU ownership stays here.
use glam::{Mat4, Vec3};
use serde_json::{Value, json};
use somnium_anim::Playback;
use somnium_asset::{animated::AnimatedAsset, database::AssetId};
use somnium_core::{EditorFlags, EngineContext, Name, Transform, WorldTransform};
use somnium_ecs::{Component, Entity, World, component_schema, reflect::TypeRegistry};
use somnium_renderer::{
    SomniumRenderer, animated_geometry::AnimatedMeshId, geometry::MeshAllocation,
};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct AnimatedPreview {
    pub source: AssetId,
    pub clip: String,
    pub playing: bool,
    pub speed: f32,
    pub time: f32,
    pub looping: bool,
    pub hidden_joints: String,
    pub reload: bool,
    pub status: String,
}
impl Component for AnimatedPreview {}
impl Default for AnimatedPreview {
    fn default() -> Self {
        Self {
            source: AssetId::from_relative_path("models/animation_demo.gltf"),
            clip: "Bend".into(),
            playing: true,
            speed: 1.0,
            time: 0.0,
            looping: true,
            hidden_joints: String::new(),
            reload: false,
            status: "Waiting for the asset inventory".into(),
        }
    }
}
fn schema(r: &mut TypeRegistry) {
    let runtime = somnium_ecs::reflect::FieldFlags::EDIT;
    r.register(component_schema! {
        AnimatedPreview as "hello.AnimatedPreview", display "Animated Mesh Preview", version 1,
        fields {
            source { group:"Asset", asset_kind_mask:somnium_asset::database::ASSET_KIND_MESH, doc:"Import a skinned glTF/GLB, then choose it here. One named skeleton; baked LINEAR clips." },
            clip { group:"Playback", doc:"Exact clip name. Clear to choose the first clip; available names appear in the log and Game Authoring state." },
            playing { group:"Playback", doc:"Live GPU preview in Edit and Play. Turn off to scrub Time." },
            speed { group:"Playback",min:-4.0,max:4.0 },
            time { group:"Playback",min:0.0,max:3600.0,unit:"s",doc:"Seek time; changing it resets the preview clock." },
            looping { group:"Playback" },
            hidden_joints { group:"Visibility",doc:"Comma-separated joint names to exclude (for example head,neck_01); empty shows the full mesh." },
            reload { group:"Asset",doc:"Toggle to reload the source after external edits." },
            status { group:"Asset",read_only:true,flags:runtime,doc:"Import errors and available clip names." },
        }
    });
}
pub fn register(r: &mut somnium_core::authoring::GameRegistration) {
    r.components.push(schema);
    r.presets.push(somnium_core::authoring::GamePreset { id:"hello.animated_preview",label:"Animated Mesh Preview",category:"Examples",schema:json!({"type":"object","properties":{"position":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3}},"additionalProperties":false}),build:spawn });
}
fn spawn(world: &mut World, args: &Value) -> Result<Vec<Entity>, String> {
    let position: [f32; 3] =
        serde_json::from_value(args.get("position").cloned().unwrap_or(json!([0, 1, 0])))
            .map_err(|_| "position needs [x,y,z]")?;
    if position.iter().any(|v| !v.is_finite()) {
        return Err("position must be finite".into());
    }
    Ok(vec![world.spawn((
        Name::new("Animated Mesh Preview"),
        Transform::from_translation(Vec3::from(position)),
        WorldTransform::identity(),
        AnimatedPreview::default(),
    ))])
}
struct Draw {
    allocation: MeshAllocation,
    skin: AnimatedMeshId,
    posed: u32,
    indices: Vec<u32>,
    material: u32,
}
struct Instance {
    source: AssetId,
    reload: bool,
    mask: String,
    asset: AnimatedAsset,
    draws: Vec<Draw>,
    clock: f32,
    seek: f32,
    clip: String,
    palette: Vec<Mat4>,
    frames: u64,
}
impl Instance {
    fn release(self, r: &mut SomniumRenderer) {
        for draw in self.draws {
            r.animated_geometry.remove(draw.skin, &mut r.geometry);
            r.geometry.free_mesh(draw.allocation);
        }
    }
}
#[derive(Default)]
pub struct Previews {
    instances: HashMap<Entity, Instance>,
    failed: HashMap<Entity, (AssetId, bool, String, String)>,
}
impl Previews {
    pub fn state(&self) -> Value {
        json!({"loaded":self.instances.iter().map(|(e,i)|json!({"entity":format!("{e:?}"),"clips":i.asset.clips.keys().collect::<Vec<_>>(),"selected":i.clip,"time":i.clock,"frames":i.frames,"meshes":i.draws.len()})).collect::<Vec<_>>(),"errors":self.failed.values().map(|v|&v.3).collect::<Vec<_>>()})
    }
    pub fn render(&mut self, ctx: &mut EngineContext) {
        let Some(gpu) = ctx.render_ctx else { return };
        let Some(r) = ctx.renderer.as_deref_mut() else {
            return;
        };
        let removed: Vec<_> = self
            .instances
            .keys()
            .copied()
            .filter(|e| ctx.world.get::<AnimatedPreview>(*e).is_none())
            .collect();
        for e in removed {
            if let Some(i) = self.instances.remove(&e) {
                i.release(r);
            }
        }
        self.failed
            .retain(|e, _| ctx.world.get::<AnimatedPreview>(*e).is_some());
        let entities: Vec<_> = ctx
            .world
            .entities()
            .filter_map(|e| {
                Some((
                    e,
                    ctx.world.get::<AnimatedPreview>(e)?.clone(),
                    ctx.world.get::<Transform>(e).copied().unwrap_or_default(),
                ))
            })
            .collect();
        for (e, settings, transform) in entities {
            let reload = self.instances.get(&e).is_some_and(|i| {
                i.source != settings.source
                    || i.reload != settings.reload
                    || i.mask != settings.hidden_joints
            });
            if reload {
                if let Some(i) = self.instances.remove(&e) {
                    i.release(r);
                }
            }
            if !self.instances.contains_key(&e) {
                if self.failed.get(&e).is_some_and(|v| {
                    v.0 == settings.source
                        && v.1 == settings.reload
                        && v.2 == settings.hidden_joints
                }) {
                    continue;
                }
                let Some(record) = ctx.ui.asset_record(settings.source) else {
                    continue;
                };
                let result = (|| -> Result<Instance, String> {
                    let asset =
                        AnimatedAsset::load(&record.absolute_path).map_err(|e| e.to_string())?;
                    if asset.clips.is_empty() {
                        return Err("Asset contains no skeletal clips".into());
                    }
                    let materials = r.upload_scene_materials(gpu, &asset.scene);
                    let hidden: Vec<_> = settings
                        .hidden_joints
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .collect();
                    for name in &hidden {
                        if asset.joint(name).is_none() {
                            return Err(format!("Unknown hidden joint {name}"));
                        }
                    }
                    let mut draws: Vec<Draw> = vec![];
                    for (index, mesh) in asset.scene.meshes.iter().enumerate() {
                        let material = asset
                            .scene
                            .nodes
                            .iter()
                            .find(|n| n.mesh_index == Some(index))
                            .and_then(|n| n.material_index)
                            .and_then(|m| materials.get(m))
                            .copied()
                            .unwrap_or(0);
                        let indices = asset.masked_indices(index, &hidden);
                        let allocation = r.geometry.upload_mesh_pooled(
                            &gpu.queue,
                            &mesh.vertices,
                            &indices,
                            material,
                        );
                        let registration = if allocation.vertex_count as usize
                            != mesh.vertices.len()
                            || allocation.index_count as usize != indices.len()
                        {
                            Err("Rest geometry pool is full".into())
                        } else {
                            r.animated_geometry.register(
                                gpu,
                                &mut r.geometry,
                                &mut r.skin_pass,
                                &asset.skeleton,
                                mesh.skin.as_ref().expect("validated skin"),
                                allocation.vertex_offset,
                                &mesh.vertices,
                            )
                        };
                        match registration {
                            Ok((skin, posed)) => draws.push(Draw {
                                allocation,
                                skin,
                                posed,
                                indices,
                                material,
                            }),
                            Err(error) => {
                                r.geometry.free_mesh(allocation);
                                for draw in draws {
                                    r.animated_geometry.remove(draw.skin, &mut r.geometry);
                                    r.geometry.free_mesh(draw.allocation);
                                }
                                return Err(error);
                            }
                        }
                    }
                    tracing::info!(
                        "Animated preview clips: {}",
                        asset.clips.keys().cloned().collect::<Vec<_>>().join(", ")
                    );
                    let palette = vec![Mat4::IDENTITY; asset.skeleton.len()];
                    Ok(Instance {
                        source: settings.source,
                        reload: settings.reload,
                        mask: settings.hidden_joints.clone(),
                        asset,
                        draws,
                        clock: settings.time,
                        seek: settings.time,
                        clip: settings.clip.clone(),
                        palette,
                        frames: 0,
                    })
                })();
                match result {
                    Ok(instance) => {
                        self.failed.remove(&e);
                        self.instances.insert(e, instance);
                    }
                    Err(error) => {
                        ctx.ui.push_toast(&format!("Animated preview: {error}"));
                        if let Some(config) = ctx.world.get_mut::<AnimatedPreview>(e) {
                            config.status = error.clone();
                        }
                        self.failed.insert(
                            e,
                            (
                                settings.source,
                                settings.reload,
                                settings.hidden_joints.clone(),
                                error,
                            ),
                        );
                        continue;
                    }
                }
            }
            let i = self.instances.get_mut(&e).expect("loaded");
            if i.seek != settings.time || i.clip != settings.clip {
                i.clock = settings.time;
                i.seek = settings.time;
                i.clip = settings.clip.clone();
            }
            if settings.playing && ctx.simulation.state != somnium_core::SimulationState::Paused {
                i.clock += ctx.time.dt().min(0.1) * settings.speed;
            }
            let clip = if settings.clip.is_empty() {
                i.asset.clips.values().next()
            } else {
                i.asset.clips.get(&settings.clip)
            };
            if let Some(config) = ctx.world.get_mut::<AnimatedPreview>(e) {
                config.status = format!(
                    "{} | Clips: {}",
                    if clip.is_some() {
                        "Ready"
                    } else {
                        "Unknown clip; choose a listed name"
                    },
                    i.asset.clips.keys().cloned().collect::<Vec<_>>().join(", ")
                );
            }
            let Some(clip) = clip else { continue };
            let playback = if settings.looping {
                Playback::LOOPING
            } else {
                Playback::ONCE
            };
            let Ok(pose) = clip.sample(&i.asset.skeleton, i.clock, playback) else {
                continue;
            };
            if !pose.to_palette(&i.asset.skeleton, &mut i.palette) {
                continue;
            }
            if ctx.world.get::<EditorFlags>(e).is_some_and(|f| f.hidden) {
                continue;
            }
            for draw in &i.draws {
                r.animated_geometry.update(draw.skin, &i.palette);
                r.submit(somnium_renderer::command::DrawCommand {
                    sort_key: somnium_renderer::command::SortKey::new(
                        0,
                        draw.material as u16,
                        draw.posed,
                    ),
                    vertex_offset: draw.posed,
                    index_offset: draw.allocation.index_offset,
                    index_count: draw.indices.len() as u32,
                    material_id: draw.material,
                    transform: Mat4::from_scale_rotation_translation(
                        transform.scale,
                        transform.rotation,
                        transform.translation,
                    ),
                    casts_shadow: true,
                });
            }
            i.frames = i.frames.wrapping_add(1);
            r.invalidate_shadow_casters();
        }
    }
}
