//! Editor play-session slots. Authored scenes remain the Stop checkpoint.
use super::{SaveGame, SaveSlots, SceneDelta};
use crate::{Name, Transform, WorldTransform};
use serde_json::Value;
use somnium_ecs::{Component, Entity, World, component_schema, reflect::TypeRegistry};

/// Designer-owned slot configuration; runtime diagnostics are never serialized.
#[derive(Clone, Debug, PartialEq)]
pub struct SaveSettings {
    /// Filename-safe slot key, relative to the project's save directory.
    pub slot: String,
    /// Friendly title stored with the slot.
    pub title: String,
    /// Content schema revision checked before loading.
    pub content_version: u32,
    /// Optional PNG chosen by the designer.
    pub thumbnail: String,
    /// Last operation result.
    pub status: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::PerceptionComponent;
    #[test]
    fn play_slot_restores_durable_references_and_keeps_the_authored_stop_checkpoint() {
        let directory = std::env::temp_dir().join(format!(
            "somnium_designer_saves_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let slots = SaveSlots::new(&directory);
        let registry = crate::reflect_registry::component_registry();
        let mut world = World::new();
        let target = world.spawn((Name::new("Authored target"), Transform::default()));
        let sensor = world.spawn((
            Name::new("Sensor"),
            Transform::default(),
            PerceptionComponent {
                target,
                ..Default::default()
            },
        ));
        let profile = world.spawn((Name::new("Slot"), SaveSettings::default()));
        let mut session = DesignerSaves::default();
        session.begin(&mut world, &registry);
        let checkpoint = session.baseline.clone().unwrap();
        let target_id = world.persistent_id(target).unwrap();
        let sensor_id = world.persistent_id(sensor).unwrap();
        world.get_mut::<Transform>(target).unwrap().translation.x = 7.0;
        let spawned = world.spawn((Name::new("Saved gameplay target"), Transform::default()));
        world.get_mut::<PerceptionComponent>(sensor).unwrap().target = spawned;
        session.tick(2.0);
        session
            .run_with_slots(&mut world, Some(profile), false, &slots)
            .unwrap();
        let spawned_id = world.persistent_id(spawned).unwrap();
        world.despawn(spawned);
        world.get_mut::<Transform>(target).unwrap().translation.x = 99.0;
        let unsaved = world.spawn((Name::new("Unsaved gameplay spawn"),));
        assert!(world.persistent_id(unsaved).is_none());
        session
            .run_with_slots(&mut world, Some(profile), true, &slots)
            .unwrap();
        assert!(!world.is_alive(unsaved));
        assert_eq!(world.get::<Transform>(target).unwrap().translation.x, 7.0);
        let restored_spawn = world.entity_by_persistent_id(spawned_id).unwrap();
        assert_eq!(
            world.get::<PerceptionComponent>(sensor).unwrap().target,
            restored_spawn
        );
        assert_eq!(session.baseline.as_ref(), Some(&checkpoint));
        assert_eq!(session.played_seconds, 2.0);
        // A malformed known field must fail staging before removing or changing
        // anything in the live scene. The ordinary scene loader warns here.
        let mut invalid = checkpoint.clone();
        let entry = invalid["entities"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry["persistent_id"] == target_id.to_string())
            .unwrap();
        entry["components"]["somnium.Transform"]["fields"]["translation"] =
            serde_json::json!("bad vector");
        let mut corrupt = SaveGame::new(1);
        corrupt.global = SceneDelta::between(&checkpoint, &invalid).unwrap();
        slots.write("designer", &corrupt).unwrap();
        let before = crate::scene_schema::scene_to_json(&mut world, &registry);
        assert!(
            session
                .run_with_slots(&mut world, Some(profile), true, &slots)
                .is_err()
        );
        assert_eq!(
            crate::scene_schema::scene_to_json(&mut world, &registry),
            before
        );
        // This is the same checkpoint restoration the Stop action owns.
        crate::prefab::restore_document(&mut world, &checkpoint).unwrap();
        session.end();
        assert!(world.entity_by_persistent_id(spawned_id).is_none());
        let target = world.entity_by_persistent_id(target_id).unwrap();
        let sensor = world.entity_by_persistent_id(sensor_id).unwrap();
        assert_eq!(world.get::<Transform>(target).unwrap().translation.x, 0.0);
        assert_eq!(
            world.get::<PerceptionComponent>(sensor).unwrap().target,
            target
        );
        assert!(
            session
                .run_with_slots(&mut world, None, false, &slots)
                .is_err()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
impl Default for SaveSettings {
    fn default() -> Self {
        Self {
            slot: "designer".into(),
            title: "Designer play session".into(),
            content_version: 1,
            thumbnail: String::new(),
            status: "Press Play, then Save Play Slot. Stop restores authored content.".into(),
        }
    }
}
impl Component for SaveSettings {}
pub(crate) fn register(registry: &mut TypeRegistry) {
    registry.register(component_schema! {
        SaveSettings as "somnium.SaveSettings", display "Save Game Slot", version 1,
        fields {
            slot { display_name:"Slot",doc:"Letters, numbers, dash and underscore. Saved under saves/." },
            title { display_name:"Title" },
            content_version { display_name:"Content Version",min:1.0,doc:"Loading rejects a different version until the game's migration has run." },
            thumbnail { display_name:"Thumbnail PNG",doc:"Optional PNG path. Choose Save Slot Thumbnail from Create or the command palette." },
            status { display_name:"Status",read_only:true,flags:somnium_ecs::reflect::FieldFlags::EDIT },
        }
    });
}

/// One editor play session and its delta baseline.
#[derive(Default)]
pub struct DesignerSaves {
    baseline: Option<Value>,
    played_seconds: f64,
}
impl DesignerSaves {
    /// Snapshot authored values immediately before Play.
    pub fn begin(&mut self, world: &mut World, registry: &TypeRegistry) {
        self.baseline = Some(crate::scene_schema::scene_to_json(world, registry));
        self.played_seconds = 0.0;
    }
    /// Advance only with the editor's running simulation clock.
    pub fn tick(&mut self, dt: f32) {
        if dt.is_finite() && dt > 0.0 {
            self.played_seconds += f64::from(dt);
        }
    }
    /// End the slot session after Stop has restored the authored checkpoint.
    pub fn end(&mut self) {
        self.baseline = None;
    }
    fn settings(world: &mut World, selected: Option<Entity>) -> (Entity, SaveSettings) {
        let target = selected
            .filter(|e| world.get::<SaveSettings>(*e).is_some())
            .or_else(|| {
                world
                    .entities()
                    .find(|e| world.get::<SaveSettings>(*e).is_some())
            });
        let entity = target.unwrap_or_else(|| {
            world.spawn((
                Name::new("Save Game Slot"),
                Transform::default(),
                WorldTransform::identity(),
                SaveSettings::default(),
            ))
        });
        (
            entity,
            world
                .get::<SaveSettings>(entity)
                .cloned()
                .unwrap_or_default(),
        )
    }
    /// Save or load values in the active play world, leaving Stop reversible.
    pub fn run(
        &mut self,
        world: &mut World,
        selected: Option<Entity>,
        load: bool,
    ) -> Result<String, String> {
        self.run_with_slots(world, selected, load, &SaveSlots::new("saves"))
    }
    /// Use a project-selected slot directory with the same editor transaction.
    pub fn run_with_slots(
        &mut self,
        world: &mut World,
        selected: Option<Entity>,
        load: bool,
        slots: &SaveSlots,
    ) -> Result<String, String> {
        let baseline = self
            .baseline
            .as_ref()
            .ok_or("Press Play before saving or loading a play slot")?;
        let (entity, settings) = Self::settings(world, selected);
        let result = (|| {
            let registry = crate::reflect_registry::component_registry();
            if load {
                let save = slots.read(&settings.slot)?;
                if save.content_version != u64::from(settings.content_version) {
                    return Err(format!(
                        "Slot content version {} differs from {}. Run a game content migration before loading.",
                        save.content_version, settings.content_version
                    ));
                }
                let rebased = save.global.rebase(baseline)?;
                // Validate the complete document before touching the live world.
                let mut staging = World::new();
                crate::scene_schema::scene_from_json(&mut staging, &registry, &rebased.scene)
                    .map_err(|e| e.to_string())?;
                // The scene loader retains warnings, while in-place apply rejects
                // malformed known fields. Exercise that stricter path in staging.
                crate::prefab::restore_document(&mut staging, &rebased.scene)?;
                // Gameplay may have spawned entities since the last snapshot.
                // Give them identities so restore can remove unsaved entities.
                let _ = crate::scene_schema::scene_to_json(world, &registry);
                crate::prefab::restore_document(world, &rebased.scene)?;
                crate::ai::reset_behaviors(world);
                self.played_seconds = save.metadata.played_seconds;
                Ok(format!(
                    "Loaded '{}': {} unresolved content paths",
                    settings.slot,
                    rebased.unresolved.len()
                ))
            } else {
                let played = crate::scene_schema::scene_to_json(world, &registry);
                let mut save = SaveGame::new(u64::from(settings.content_version));
                save.global = SceneDelta::between(baseline, &played)?;
                save.metadata.title = settings.title.clone();
                save.metadata.played_seconds = self.played_seconds;
                save.metadata.saved_unix_secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_secs();
                if !settings.thumbnail.trim().is_empty() {
                    let path = std::path::Path::new(&settings.thumbnail);
                    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 4 * 1024 * 1024 {
                        return Err("Thumbnail exceeds 4 MiB".into());
                    }
                    save.metadata.screenshot_png =
                        std::fs::read(path).map_err(|e| e.to_string())?;
                }
                slots.write(&settings.slot, &save)?;
                Ok(format!("Saved '{}'", settings.slot))
            }
        })();
        if let Some(profile) = world.get_mut::<SaveSettings>(entity) {
            profile.status = result.clone().unwrap_or_else(|e| e);
        }
        result
    }
    /// Attach a reviewed image to the selected (or first) slot profile.
    pub fn set_thumbnail(world: &mut World, selected: Option<Entity>, path: String) {
        let (entity, _) = Self::settings(world, selected);
        if let Some(settings) = world.get_mut::<SaveSettings>(entity) {
            settings.thumbnail = path;
        }
    }
}
