//! Host actions for the MORROWIND designer tools.
use super::*;
use somnium_ui::editor_event::DesignerTool;

/// Include a preview rig's targets or a navigation volume when framing it.
pub(super) fn designer_focus_points(world: &World, entity: somnium_ecs::Entity) -> Vec<glam::Vec3> {
    let position = |entity| {
        // Create and Focus can arrive in one event batch, before propagation.
        if world.get::<Parent>(entity).is_none() {
            return world.get::<Transform>(entity).map(|t| t.translation);
        }
        world
            .get::<WorldTransform>(entity)
            .map(|world| world.0.to_scale_rotation_translation().2)
            .or_else(|| world.get::<Transform>(entity).map(|t| t.translation))
    };
    let Some(center) = position(entity) else {
        return Vec::new();
    };
    if let Some(profile) = world.get::<crate::ai::NavigationProfile>(entity) {
        if profile.bounds_size.is_finite() && profile.bounds_size.min_element() > 0.0 {
            let half = profile.bounds_size * 0.5;
            return vec![center - half, center + half];
        }
    }
    let mut points = vec![center];
    if world
        .get::<crate::animation_authoring::AnimationAuthoring>(entity)
        .is_some()
    {
        for child in world.entities().filter(|child| {
            world
                .get::<Parent>(*child)
                .is_some_and(|parent| parent.entity == entity)
        }) {
            if let Some(point) = position(child) {
                points.push(point - glam::Vec3::splat(0.2));
                points.push(point + glam::Vec3::splat(0.2));
            }
        }
    }
    points
}

impl<G: GameApp> Engine<G> {
    pub(super) fn create_designer_component(&mut self, name: &str) {
        let registry = crate::reflect_registry::component_registry();
        let Some(schema) = registry.by_name(name) else {
            return;
        };
        let mut scratch = World::new();
        let origin = self
            .selection
            .primary
            .and_then(|e| self.world.get::<Transform>(e))
            .map_or(glam::Vec3::ZERO, |t| t.translation);
        let entity = scratch.spawn((
            Name::new(schema.display_name),
            Transform::from_translation(origin),
            WorldTransform::identity(),
        ));
        if let Err(error) = (schema.insert_default)(&mut scratch, entity) {
            if let Some(ui) = &mut self.ui_manager {
                ui.push_toast(&error.to_string());
            }
            return;
        }
        let snapshot = EntitySnapshot::capture(&scratch, entity);
        self.undo_stack.push(
            Box::new(CreateEntityCmd::new(snapshot)),
            &mut self.world,
            &mut self.selection.primary,
        );
        self.selection.reconcile();
        self.scene_dirty = true;
        self.after_selection_change();
        if let Some(ui) = &mut self.ui_manager {
            ui.push_toast(&format!(
                "{} created. Configure it in Details.",
                schema.display_name
            ));
        }
    }

    pub(super) fn run_designer_tool(&mut self, action: DesignerTool) {
        let result = self.run_designer_tool_result(action);
        // A transport press says nothing; the dock's header already shows what changed.
        let message = result.unwrap_or_else(|error| error);
        if let Some(ui) = &mut self.ui_manager
            && !message.is_empty()
        {
            ui.push_toast(&message);
        }
    }
    pub(super) fn run_designer_tool_result(
        &mut self,
        action: DesignerTool,
    ) -> Result<String, String> {
        (|| -> Result<String, String> {
            match action {
                DesignerTool::NavigationBake => {
                    let entity = self.selection.primary.ok_or("Select a Navigation Volume")?;
                    let renderer = self.renderer.as_ref().ok_or("Renderer is not ready")?;
                    self.navigation_editor
                        .bake(&mut self.world, renderer, &mut self.jobs, entity)
                }
                DesignerTool::NavigationClear => Ok(self.navigation_editor.clear(&mut self.world)),
                DesignerTool::BehaviorEdit => {
                    let entity = self
                        .selection
                        .primary
                        .ok_or("Select an entity with a Behavior component")?;
                    let json = crate::ai::behavior_document(&self.world, entity)?;
                    self.ui_manager
                        .as_mut()
                        .ok_or("Editor is not ready")?
                        .edit_authoring_graph("somnium.behavior", &json)?;
                    Ok("Behavior graph opened. Apply updates the selected entity.".into())
                }
                DesignerTool::AnimationRig => {
                    let before =
                        crate::scene_schema::scene_to_json(&mut self.world, &self.type_registry);
                    let (spawn, _, right) = camera_spawn_basis(self.renderer.as_ref(), 7.0);
                    let origin = spawn + right * 4.0;
                    let root =
                        crate::animation_authoring::create_preview_rig(&mut self.world, origin);
                    self.selection.set_single(Some(root));
                    self.reconstruct_scene_gpu("assets/animation-preview.somscene");
                    let after =
                        crate::scene_schema::scene_to_json(&mut self.world, &self.type_registry);
                    self.undo_stack
                        .push_silent(Box::new(crate::prefab::PrefabEditCommand::new(
                            before, after,
                        )));
                    self.scene_dirty = true;
                    self.after_selection_change();
                    self.focus_camera_on_selection();
                    Ok("Animation rig created. Set IK weights in Details and drag its targets in the viewport.".into())
                }
                DesignerTool::AnimationReset => {
                    let entity = self.animation_selection()?;
                    self.animation_authoring
                        .reset(&mut self.world, self.physics.as_mut(), entity);
                    Ok("Animation preview reset".into())
                }
                DesignerTool::AnimationPlayPause => {
                    let entity = self.animation_selection()?;
                    let settings = self
                        .world
                        .get_mut::<crate::animation_authoring::AnimationAuthoring>(entity)
                        .ok_or("The rig has no animation settings")?;
                    settings.playing = !settings.playing;
                    Ok(String::new())
                }
                DesignerTool::AnimationReload => {
                    let entity = self.animation_selection()?;
                    if let Some(profile) = self
                        .world
                        .get_mut::<crate::animation_authoring::AnimationAuthoring>(entity)
                    {
                        profile.reload_revision = profile.reload_revision.wrapping_add(1);
                    }
                    Ok("Animation events will reload on the next preview frame".into())
                }
                DesignerTool::AnimationEvents => {
                    let entity = self.animation_selection()?;
                    let document = crate::animation_authoring::event_timeline(&self.world, entity)?;
                    self.ui_manager
                        .as_mut()
                        .ok_or("Editor is not ready")?
                        .edit_animation_timeline(document);
                    self.animation_event_owner = Some(entity);
                    Ok("Edit timeline markers, then choose Save Animation Events.".into())
                }
                DesignerTool::AnimationSaveEvents => {
                    let entity = self
                        .animation_event_owner
                        .filter(|e| self.world.is_alive(*e))
                        .ok_or("Open a rig's event timeline first")?;
                    let document = self
                        .ui_manager
                        .as_ref()
                        .ok_or("Editor is not ready")?
                        .animation_timeline_document()
                        .clone();
                    let source_before = self
                        .world
                        .get::<crate::animation_authoring::AnimationAuthoring>(entity)
                        .and_then(|settings| {
                            std::fs::read(&settings.events_timeline).ok().map(|bytes| {
                                (std::path::PathBuf::from(&settings.events_timeline), bytes)
                            })
                        });
                    let before =
                        crate::scene_schema::scene_to_json(&mut self.world, &self.type_registry);
                    let path = crate::animation_authoring::save_event_timeline(
                        &mut self.world,
                        entity,
                        &document,
                    )?;
                    let after =
                        crate::scene_schema::scene_to_json(&mut self.world, &self.type_registry);
                    let sources = source_before
                        .and_then(|(path, before)| {
                            std::fs::read(&path).ok().map(|after| (path, before, after))
                        })
                        .into_iter()
                        .collect();
                    self.undo_stack.push_silent(Box::new(
                        crate::prefab::PrefabEditCommand::with_sources(before, after, sources),
                    ));
                    self.scene_dirty = true;
                    Ok(format!("Animation events saved to {path}"))
                }
                DesignerTool::SavePlay | DesignerTool::LoadPlay => {
                    if !self.play_session_active {
                        return Err("Press Play before saving or loading a play slot".into());
                    }
                    let save_root = match self.config.project_root.as_deref() {
                        Some(root) => {
                            let project = crate::authoring::project::ProjectPaths::open(root)?;
                            project.resolve(&project.manifest.saves)?
                        }
                        None => std::path::PathBuf::from("saves"),
                    };
                    let message = self.designer_saves.run_with_slots(
                        &mut self.world,
                        self.selection.primary,
                        action == DesignerTool::LoadPlay,
                        &crate::save_game::SaveSlots::new(&save_root),
                    )?;
                    if action == DesignerTool::LoadPlay {
                        self.animation_authoring
                            .clear(&mut self.world, self.physics.as_mut());
                        crate::ai::reset_editor_runtime(&mut self.world);
                        crate::ai::reset_behaviors(&mut self.world);
                        self.reconstruct_scene_gpu("saves/designer.somsave");
                        self.selection.reconcile();
                        self.after_selection_change();
                    }
                    Ok(message)
                }
                DesignerTool::SaveThumbnail => {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("PNG thumbnail", &["png"])
                        .set_directory("assets")
                        .pick_file()
                    {
                        let before = crate::scene_schema::scene_to_json(
                            &mut self.world,
                            &self.type_registry,
                        );
                        crate::save_game::editor::DesignerSaves::set_thumbnail(
                            &mut self.world,
                            self.selection.primary,
                            path.to_string_lossy().into_owned(),
                        );
                        let after = crate::scene_schema::scene_to_json(
                            &mut self.world,
                            &self.type_registry,
                        );
                        self.undo_stack.push_silent(Box::new(
                            crate::prefab::PrefabEditCommand::new(before, after),
                        ));
                        self.scene_dirty = true;
                        Ok("Save slot thumbnail selected".into())
                    } else {
                        Ok("Thumbnail selection cancelled".into())
                    }
                }
            }
        })()
    }
    /// The rig the animation tools act on: the selection when it is one, otherwise the scene's first, so the
    /// dock's transport works without hunting for the rig in the Outliner.
    pub(super) fn animation_selection(&self) -> Result<somnium_ecs::Entity, String> {
        let is_rig = |e: &somnium_ecs::Entity| {
            self.world
                .get::<crate::animation_authoring::AnimationAuthoring>(*e)
                .is_some()
        };
        self.selection
            .primary
            .filter(is_rig)
            .or_else(|| {
                self.world
                    .entities_with::<crate::animation_authoring::AnimationAuthoring>()
                    .find(is_rig)
            })
            .ok_or("Create an animation rig first, or select one in the Outliner".into())
    }

    /// Phase ALYX-J: tell the renderer what glows. The selection and, separately, what a click would select now;
    /// each with every entity under it, since selecting a house means its walls and windows. Nothing in Play.
    pub(super) fn update_editor_highlight(&mut self) {
        let editing = !self.play_session_active;
        let over = editing
            && self.ui_manager.as_ref().is_some_and(|ui| {
                ui.pointer_over_viewport() && !ui.viewport_camera_active()
            });
        let busy = self.marquee.is_some()
            || self.gizmo_drag.is_some()
            || self.terrain_edit_active
            || self.foliage_paint_active
            || self.vertex_paint_active;
        if !over || busy {
            self.hovered_entity = None;
        } else if self.hover_pick_due
            && self.hover_picked_at.elapsed() >= std::time::Duration::from_millis(50)
        {
            self.hover_pick_due = false;
            self.hover_picked_at = std::time::Instant::now();
            self.hovered_entity = self.entities_under_cursor().into_iter().next();
        }
        self.hovered_entity = self.hovered_entity.filter(|e| self.world.is_alive(*e));
        let (selected, hovered) = if editing {
            let selected = subtree(&self.world, self.selection.as_slice());
            let hovered = self
                .hovered_entity
                .filter(|e| !selected.contains(&e.index()))
                .map(|e| subtree(&self.world, &[e]))
                .unwrap_or_default();
            (selected, hovered)
        } else {
            Default::default()
        };
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_editor_highlight(selected, hovered);
        }
    }

    /// Phase ALYX-J: a glyph where each entity with nothing to see stands (lights, sound, particles, decals,
    /// navigation, the sky and weather), nearest 96 within 150 m, fading with distance. Not in Play.
    pub(super) fn submit_viewport_icons(&mut self) {
        use somnium_ui::icons::IconId;
        let mut icons = Vec::new();
        if !self.play_session_active
            && let Some(renderer) = self.renderer.as_ref()
        {
            let view_proj = renderer.picking_view_proj();
            let camera = renderer.camera_pos;
            let selected: std::collections::HashSet<u32> =
                self.selection.as_slice().iter().map(|e| e.index()).collect();
            let world = &self.world;
            let mut found: Vec<(f32, somnium_ui::ViewportIcon)> = Vec::new();
            let mut place = |entity: Entity, icon: IconId| {
                if crate::is_hidden(world, entity)
                    || world.get::<crate::MeshComponent>(entity).is_some()
                    || world.get::<crate::ImportedMesh>(entity).is_some()
                {
                    return;
                }
                let Some(matrix) = entity_world_matrix(world, entity) else {
                    return;
                };
                let position = matrix.w_axis.truncate();
                let distance = (position - camera).length();
                let clip = view_proj * position.extend(1.0);
                if distance > 150.0 || clip.w <= 0.05 {
                    return;
                }
                let ndc = clip.truncate().truncate() / clip.w;
                if ndc.x.abs() > 1.02 || ndc.y.abs() > 1.02 {
                    return;
                }
                let fade = 1.0 - ((distance - 60.0) / 90.0).clamp(0.0, 1.0);
                found.push((
                    distance,
                    somnium_ui::ViewportIcon {
                        ndc: ndc.to_array(),
                        icon,
                        entity: entity.index(),
                        selected: selected.contains(&entity.index()),
                        fade: (fade * 20.0).round() / 20.0,
                    },
                ));
            };
            for entity in world.entities_with::<LightComponent>() {
                let icon = match world.get::<LightComponent>(entity).map(|l| l.light_type) {
                    Some(LightType::Directional) => IconId::DirectionalLight,
                    Some(LightType::Spot | LightType::Tube) => IconId::SpotLight,
                    _ => IconId::PointLight,
                };
                place(entity, icon);
            }
            for entity in world.entities_with::<crate::AudioEmitterComponent>() {
                place(entity, IconId::Audio);
            }
            for entity in world.entities_with::<crate::ParticleEmitter>() {
                place(entity, IconId::Particle);
            }
            for entity in world.entities_with::<crate::decal::DecalComponent>() {
                place(entity, IconId::Texture);
            }
            for entity in world.entities_with::<crate::ai::NavigationProfile>() {
                place(entity, IconId::NavMesh);
            }
            for entity in world.entities_with::<crate::ai::NavigationLink>() {
                place(entity, IconId::Route);
            }
            for entity in world.entities_with::<crate::sky::SkyComponent>() {
                place(entity, IconId::World);
            }
            for entity in world.entities_with::<crate::weather::WeatherComponent>() {
                place(entity, IconId::Weather);
            }
            for entity in world.entities_with::<crate::PostProcessComponent>() {
                place(entity, IconId::PostFx);
            }
            found.sort_by(|a, b| a.0.total_cmp(&b.0));
            // One glyph per entity, the first kind found; nearest last so it draws on top.
            let mut seen = std::collections::HashSet::new();
            icons = found
                .into_iter()
                .filter(|(_, icon)| seen.insert(icon.entity))
                .take(96)
                .map(|(_, icon)| icon)
                .collect();
            icons.reverse();
        }
        if let Some(ui) = self.ui_manager.as_mut() {
            ui.set_viewport_icons(icons);
        }
    }

    /// Components a designer may add from Details: everything registered except identity, hierarchy, settings
    /// singletons and the editor's own bookkeeping.
    pub(super) fn addable_components(&self) -> Vec<(String, String)> {
        self.type_registry
            .iter()
            .filter(|schema| {
                let id = schema.stable_id.as_str();
                !matches!(
                    id,
                    "somnium.Transform"
                        | "somnium.Name"
                        | "somnium.Parent"
                        | "somnium.EditorFlags"
                        | "somnium.AnimationPreviewJoint"
                        | "somnium.MeshKind"
                ) && !id.ends_with("Settings")
                    && !id.starts_with("somnium.asset.")
            })
            .map(|schema| (schema.stable_id.as_str().to_string(), schema.display_name.to_string()))
            .collect()
    }

    /// Add or remove one component on the selected entity, as one undo step.
    pub(super) fn edit_selected_components(&mut self, component: &str, add: bool) {
        let result = (|| -> Result<String, String> {
            let entity = self.selection.primary.ok_or("Select an entity first")?;
            if self.world.get::<crate::prefab::PrefabMember>(entity).is_some() {
                return Err("Unpack the prefab, or edit its source, before changing its components".into());
            }
            let schema = self.type_registry.by_name(component).ok_or("Unknown component")?;
            let (name, insert, remove) = (schema.display_name, schema.insert_default, schema.remove);
            let before = crate::scene_schema::scene_to_json(&mut self.world, &self.type_registry);
            if add {
                insert(&mut self.world, entity).map_err(|e| e.to_string())?;
            } else if !remove(&mut self.world, entity).map_err(|e| e.to_string())? {
                return Err(format!("The entity has no {name}"));
            }
            let after = crate::scene_schema::scene_to_json(&mut self.world, &self.type_registry);
            self.undo_stack
                .push_silent(Box::new(crate::prefab::PrefabEditCommand::new(before, after)));
            self.scene_dirty = true;
            self.after_selection_change();
            Ok(if add { format!("Added {name}") } else { format!("Removed {name}") })
        })();
        if let Some(ui) = &mut self.ui_manager {
            ui.push_toast(&result.unwrap_or_else(|error| error));
        }
    }

    /// What the Animation dock's header shows, in the Animation workspace only.
    pub(super) fn animation_transport(&self) -> Option<somnium_ui::AnimationTransport> {
        if self.ui_manager.as_ref()?.active_workspace() != somnium_ui::workspace::Workspace::Animation {
            return None;
        }
        let entity = self.animation_selection().ok()?;
        let settings = self
            .world
            .get::<crate::animation_authoring::AnimationAuthoring>(entity)?;
        let length = self
            .animation_authoring
            .clip_length(entity)
            .unwrap_or(settings.duration)
            .max(1e-3);
        Some(somnium_ui::AnimationTransport {
            rig: self
                .world
                .get::<Name>(entity)
                .map_or_else(|| "Rig".to_string(), |name| name.as_str().to_string()),
            time: settings.preview_time.rem_euclid(length),
            length,
            playing: settings.playing,
            speed: settings.speed,
            status: settings.status.clone(),
        })
    }
}

/// Each root and every entity under it, by index: what a selection means on screen.
fn subtree(world: &World, roots: &[Entity]) -> std::collections::HashSet<u32> {
    let mut out = std::collections::HashSet::new();
    let mut stack: Vec<Entity> = roots.to_vec();
    while let Some(entity) = stack.pop() {
        if !world.is_alive(entity) || !out.insert(entity.index()) {
            continue;
        }
        if let Some(children) = world.get::<crate::Children>(entity) {
            stack.extend_from_slice(&children.entities[..usize::from(children.count).min(children.entities.len())]);
        }
    }
    out
}

#[cfg(test)]
mod focus_tests {
    use super::*;

    #[test]
    fn newly_created_navigation_volume_frames_authored_position_before_propagation() {
        let mut world = World::new();
        let center = glam::Vec3::new(100.0, 4.0, 30.0);
        let profile = crate::ai::NavigationProfile::default();
        let half = profile.bounds_size * 0.5;
        let entity = world.spawn((
            Transform::from_translation(center),
            WorldTransform::identity(),
            profile,
        ));
        let points = designer_focus_points(&world, entity);
        assert_eq!(points, vec![center - half, center + half]);
    }

    #[test]
    fn animation_focus_includes_offset_targets_and_joint_markers() {
        let mut world = World::new();
        let origin = glam::Vec3::new(10.0, 20.0, 30.0);
        let rig = crate::animation_authoring::create_preview_rig(&mut world, origin);
        let points = designer_focus_points(&world, rig);
        let min = points.iter().copied().reduce(glam::Vec3::min).unwrap();
        let max = points.iter().copied().reduce(glam::Vec3::max).unwrap();
        assert!(min.cmple(origin).all());
        assert!(max.cmpge(origin + glam::Vec3::new(1.2, 2.0, 2.0)).all());
        assert_eq!(points.len(), 11);
    }
}
