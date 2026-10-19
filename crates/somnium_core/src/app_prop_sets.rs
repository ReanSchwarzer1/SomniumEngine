//! The engine's half of `somnium.PropSet` (`prop_set.rs`): each frame the planner names the sets that changed or
//! went; their sources are decoded off the frame and uploaded once, and their batches and static bodies replaced.
use super::*;
use crate::prop_set::{PropSetComponent, PropSource, SourcePart};

impl<G: GameApp> Engine<G> {
    /// Bring every prop set's drawing and colliders up to date. A frame where no set changed costs the planner's
    /// walk over the sets.
    pub(super) fn update_prop_sets(&mut self) {
        let plan = self.prop_sets.planner.plan(&self.world);
        if plan.build.is_empty() && plan.gone.is_empty() {
            return;
        }
        for entity in &plan.gone {
            if let Some((ids, bodies)) = self.prop_sets.live.remove(entity) {
                if let Some(renderer) = self.renderer.as_mut() {
                    for id in ids {
                        renderer.remove_instance_batch(id);
                    }
                }
                if let Some(physics) = self.physics.as_mut() {
                    for body in bodies {
                        physics.remove_body(body);
                    }
                }
            }
        }
        // Sources first asked for this frame start decoding now.
        let wanted: Vec<String> = plan
            .build
            .iter()
            .filter_map(|(entity, _, _)| self.world.get::<PropSetComponent>(*entity).map(|s| s.source.clone()))
            .filter(|source| !self.prop_sets.sources.contains_key(source))
            .collect();
        for source in wanted {
            let state = match self.imported_source_path(&source) {
                Ok(path) => {
                    let (send, receive) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        let _ = send.send(somnium_asset::load_gltf_for(&path, None));
                    });
                    PropSource::Loading(receive)
                }
                Err(error) => {
                    warn!(%error, %source, "Prop set source path rejected");
                    PropSource::Failed
                }
            };
            self.prop_sets.sources.insert(source, state);
        }
        let Some((renderer, ctx)) = self.renderer.as_mut().zip(self.render_ctx.as_ref()) else {
            return;
        };
        // Decoded sources go up to the GPU, once.
        for (source, state) in &mut self.prop_sets.sources {
            let PropSource::Loading(receive) = state else { continue };
            match receive.try_recv() {
                Ok(Ok(scene)) => {
                    let nodes = renderer.upload_scene_for(ctx, &scene, None);
                    *state = PropSource::Ready(
                        nodes
                            .iter()
                            .map(|n| SourcePart {
                                vertex_offset: n.vertex_offset,
                                index_offset: n.index_offset,
                                index_count: n.index_count,
                                material_id: n.material_id,
                                transform: n.transform,
                            })
                            .collect(),
                        nodes.iter().map(|n| n.entity_name.clone()).collect(),
                    );
                }
                Ok(Err(error)) => {
                    warn!(%error, %source, "Prop set source could not be loaded");
                    *state = PropSource::Failed;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => *state = PropSource::Failed,
            }
        }
        for (entity, signature, at) in plan.build {
            let Some(set) = self.world.get::<PropSetComponent>(entity) else { continue };
            // Not decoded yet: the planner asks again next frame.
            let Some(PropSource::Ready(parts, names)) = self.prop_sets.sources.get(&set.source) else { continue };
            let chosen = crate::prop_set::chosen_parts(set, parts, names);
            let tinted = &mut self.prop_sets.tinted;
            let batches = crate::prop_set::batches(entity, set, at, &chosen, |material, tint| {
                *tinted.entry((material, tint.map(f32::to_bits))).or_insert_with(|| {
                    let mut copy = renderer.materials_pool.get(material).unwrap_or_else(|| {
                        renderer.materials_pool.get(0).expect("the pool has a default material")
                    });
                    for (channel, t) in copy.base_color.iter_mut().zip(tint) {
                        *channel *= t;
                    }
                    renderer.materials_pool.add_material(&ctx.queue, copy)
                })
            });
            // The chosen parts' bounds, in the copy's frame, for `collide_bounds`.
            let bounds = chosen.iter().filter_map(|p| {
                let (min, max) = renderer.geometry.mesh_aabb(p.vertex_offset)?;
                Some(somnium_renderer::culling::transform_aabb(p.transform, glam::Vec3::from(min), glam::Vec3::from(max)))
            });
            let bounds = bounds.reduce(|(a, b), (c, d)| (a.min(c), b.max(d)));
            let boxes = crate::prop_set::colliders(set, at, bounds);

            let (old_ids, old_bodies) = self.prop_sets.live.remove(&entity).unwrap_or_default();
            let ids: Vec<u64> = batches.iter().map(|(id, _)| *id).collect();
            for (id, batch) in &batches {
                renderer.set_instance_batch(*id, batch);
            }
            for id in old_ids.into_iter().filter(|id| !ids.contains(id)) {
                renderer.remove_instance_batch(id);
            }
            let mut bodies = Vec::new();
            if let Some(physics) = self.physics.as_mut() {
                for body in old_bodies {
                    physics.remove_body(body);
                }
                for (position, rotation, half_extents) in boxes {
                    bodies.push(physics.create_body(RigidBodyDescriptor {
                        shape: ColliderShape::Box { half_extents },
                        position,
                        rotation,
                        motion_type: MotionType::Static,
                        object_layer: somnium_physics::layer::LAYER_NON_MOVING,
                        ..Default::default()
                    }));
                }
            }
            self.prop_sets.live.insert(entity, (ids, bodies));
            self.prop_sets.planner.built(entity, signature);
        }
    }
}
