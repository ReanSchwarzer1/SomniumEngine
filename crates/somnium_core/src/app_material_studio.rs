//! Host side of the Material panel's studio (`somnium_ui::editor::material_studio`): the live preview the renderer
//! draws every frame from the material's own GPU slot, its shape and turntable, and the presets, which are applied as
//! ordinary reflected field edits grouped into one undo step.
use super::*;
use somnium_renderer::pass::preview_studio::{self, StudioRequest, StudioShape, StudioTargetId};
use somnium_ui::editor::material_studio::MaterialPreviewState;
use somnium_ui::editor_event::MaterialStudioEvent;

/// The preview target's side, texels: the panel's widest preview at a 1.25 UI scale, and a little over.
const PREVIEW_SIZE: u32 = 512;
/// Turntable speed, radians a second: a full turn in about fourteen seconds.
const SPIN_RATE: f32 = 0.45;
/// The gesture every preset's field edits carry (they are pushed together, never coalesced with a drag).
const PRESET_GESTURE: GestureId = GestureId(u64::MAX - 5);

/// The preview's state between frames. Core owns it; the panel shows it through `ToolContext::material_preview`.
pub(super) struct MaterialStudio {
    shape: StudioShape,
    spinning: bool,
    /// Turntable angle, radians.
    yaw: f32,
    /// The renderer drew the panel's material last frame.
    live: bool,
    target: Option<StudioTargetId>,
    /// The target's picture has been handed to the UI.
    bound: bool,
}

impl Default for MaterialStudio {
    fn default() -> Self {
        Self { shape: StudioShape::Sphere, spinning: true, yaw: 0.0, live: false, target: None, bound: false }
    }
}

impl MaterialStudio {
    pub(super) fn state(&self) -> MaterialPreviewState {
        MaterialPreviewState { shape: self.shape.index() as u8, spinning: self.spinning, live: self.live }
    }

    /// Take a preview control; a preset is handed back for the host, which owns the material.
    fn control(&mut self, event: MaterialStudioEvent) -> Option<usize> {
        match event {
            MaterialStudioEvent::Shape(index) => self.shape = StudioShape::from_index(index).unwrap_or_default(),
            MaterialStudioEvent::Spin(on) => self.spinning = on,
            MaterialStudioEvent::Turn(radians) => self.advance(radians),
            MaterialStudioEvent::Preset(index) => return Some(index),
        }
        None
    }

    fn advance(&mut self, radians: f32) {
        if radians.is_finite() {
            self.yaw = (self.yaw + radians).rem_euclid(std::f32::consts::TAU);
        }
    }

    /// One frame of the turntable.
    fn tick(&mut self, seconds: f32) {
        if self.spinning {
            self.advance(seconds.clamp(0.0, 0.1) * SPIN_RATE);
        }
    }
}

/// The field edits that put `preset` on the material carried by `entity`: one validated `SetFieldCmd` per field it
/// changes, the same write a Details edit makes. Empty when the material already answers light that way.
pub(super) fn preset_commands(
    world: &World,
    entity: Entity,
    preset: &somnium_asset::material::MaterialPreset,
) -> Result<Vec<Box<dyn crate::editor_commands::EditorCommand>>, String> {
    let registry = crate::reflect_registry::editor_registry();
    let schema = registry.by_name("somnium.asset.Material").ok_or("the material schema is not registered")?;
    let current = world
        .get::<somnium_asset::material::MaterialAsset>(entity)
        .ok_or("nothing is open in the Material panel")?
        .clone();
    let before = (schema.snapshot)(world, entity).ok_or("could not read the material")?;
    // The preset's values, read back through the same schema from a scratch world.
    let mut scratch = World::new();
    let probe = scratch.spawn((preset.applied_to(&current),));
    let after = (schema.snapshot)(&scratch, probe).ok_or("could not read the preset")?;
    let mut commands: Vec<Box<dyn crate::editor_commands::EditorCommand>> = Vec::new();
    for field in &schema.fields {
        let (Some(old), Some(new)) = (before.get(&field.id), after.get(&field.id)) else {
            continue;
        };
        if old != new {
            commands.push(Box::new(SetFieldCmd::new(
                world,
                entity,
                schema.stable_id,
                field.id,
                new.clone(),
                PRESET_GESTURE,
                None,
            )?));
        }
    }
    Ok(commands)
}

impl<G: GameApp> Engine<G> {
    /// What the Material panel edits: the material opened from the drawer, else the selected entity's own (as the
    /// Details panels are built). The asset and the entity carrying its edit session.
    pub(super) fn material_panel(&self) -> Option<(somnium_asset::database::AssetId, Entity)> {
        if let Some((asset, entity)) = self.material_asset_target.filter(|(_, e)| self.world.is_alive(*e))
            && self.world.get::<somnium_asset::material::MaterialAsset>(entity).is_some()
        {
            return Some((asset, entity));
        }
        let entity = self.selection.primary?;
        self.world.get::<somnium_asset::material::MaterialAsset>(entity)?;
        let asset = self.world.get::<MaterialComponent>(entity)?.asset;
        (asset != somnium_asset::database::AssetId::NONE).then_some((asset, entity))
    }

    pub(super) fn handle_material_studio_event(&mut self, event: MaterialStudioEvent) {
        let Some(index) = self.material_studio.control(event) else {
            return;
        };
        let message = match self.apply_material_preset(index) {
            Ok(0) => "The material already looks like that preset".to_owned(),
            Ok(_) => return,
            Err(error) => format!("Preset not applied: {error}"),
        };
        if let Some(ui) = self.ui_manager.as_mut() {
            ui.push_toast(&message);
        }
    }

    /// Apply preset `index` to the Material panel's material as one undo step; how many fields it changed.
    fn apply_material_preset(&mut self, index: usize) -> Result<usize, String> {
        let preset = somnium_asset::material::MATERIAL_PRESETS.get(index).ok_or("no such preset")?;
        let (_, entity) = self.material_panel().ok_or("select a material, or an object that uses one")?;
        let commands = preset_commands(&self.world, entity, preset)?;
        let changed = commands.len();
        if changed > 0 {
            self.undo_stack.push(
                Box::new(crate::editor_commands::CommandGroup::named(format!("Apply {} preset", preset.name), commands)),
                &mut self.world,
                &mut self.selection.primary,
            );
            self.scene_dirty = true;
        }
        Ok(changed)
    }

    /// Once a frame, before the renderer records: turn the turntable and, when a Material panel is up and its
    /// material has a GPU slot, ask the renderer's preview studio to draw it.
    pub(super) fn drive_material_preview(&mut self) {
        self.material_studio.tick(self.time.delta_time().as_secs_f32());
        self.material_studio.live = false;
        let Some((asset, _)) = self.material_panel() else {
            return;
        };
        let Some(&material_id) = self.material_runtime.get(&asset) else {
            return;
        };
        let (Some(renderer), Some(ctx), Some(ui)) =
            (self.renderer.as_mut(), self.render_ctx.as_ref(), self.ui_manager.as_mut())
        else {
            return;
        };
        let studio = &mut self.material_studio;
        let target = *studio.target.get_or_insert_with(|| renderer.studio_target(ctx, PREVIEW_SIZE));
        if !studio.bound {
            // Without a UI texture slot the panel keeps the material's thumbnail: nothing to draw into.
            let Some(view) = renderer.studio_view(target) else {
                return;
            };
            if !ui.set_material_preview_texture(view) {
                return;
            }
            studio.bound = true;
        }
        let model = preview_studio::turntable(studio.shape, studio.yaw);
        let Some(draw) = renderer.studio_shape(ctx, studio.shape, material_id, model) else {
            return;
        };
        let (view, proj, eye) = preview_studio::shape_camera();
        renderer.studio_render(
            target,
            StudioRequest {
                draws: vec![draw],
                view,
                proj,
                eye,
                backdrop: somnium_ui::editor::material_studio::preview_backdrop(),
            },
        );
        studio.live = true;
        ui.keep_material_swatches();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_asset::material::{MATERIAL_PRESETS, MaterialAsset};

    fn preset(name: &str) -> &'static somnium_asset::material::MaterialPreset {
        MATERIAL_PRESETS.iter().find(|p| p.name == name).unwrap()
    }

    /// A preset is one undo step made of ordinary field edits: it applies, undoes to exactly what was there, and
    /// redoes.
    #[test]
    fn a_preset_is_one_undoable_step() {
        let mut world = World::new();
        let original = MaterialAsset {
            roughness: 0.8,
            albedo_map: somnium_asset::database::AssetId::from_raw(42),
            ..MaterialAsset::default()
        };
        let entity = world.spawn((original.clone(),));
        let mut selected = Some(entity);
        let mut undo = crate::editor_commands::UndoStack::new(16);

        let glass = preset("Glass");
        let commands = preset_commands(&world, entity, glass).unwrap();
        assert!(commands.len() >= 4, "opacity, roughness, alpha mode and two-sided at least: {}", commands.len());
        undo.push(Box::new(crate::editor_commands::CommandGroup::named("Apply Glass preset".into(), commands)), &mut world, &mut selected);
        let applied = world.get::<MaterialAsset>(entity).unwrap().clone();
        assert_eq!(applied, glass.applied_to(&original));
        assert_eq!(applied.albedo_map, original.albedo_map, "textures stay");
        assert_eq!(undo.history().0, vec!["Apply Glass preset"], "one entry, named for the preset");

        assert!(undo.undo(&mut world, &mut selected));
        assert_eq!(world.get::<MaterialAsset>(entity).unwrap(), &original);
        assert!(undo.redo(&mut world, &mut selected));
        assert_eq!(world.get::<MaterialAsset>(entity).unwrap(), &applied);

        // Applied again, nothing changes and nothing is recorded.
        assert!(preset_commands(&world, entity, glass).unwrap().is_empty());
    }

    #[test]
    fn the_turntable_turns_only_while_spinning_and_a_drag_always_turns_it() {
        let mut studio = MaterialStudio::default();
        assert_eq!(studio.state(), MaterialPreviewState { shape: 0, spinning: true, live: false });
        studio.tick(1.0 / 60.0);
        assert!(studio.yaw > 0.0);
        assert_eq!(studio.control(MaterialStudioEvent::Spin(false)), None);
        let held = studio.yaw;
        studio.tick(1.0);
        assert_eq!(studio.yaw, held, "held still");
        studio.control(MaterialStudioEvent::Turn(1.0));
        assert!((studio.yaw - held - 1.0).abs() < 1.0e-6, "a drag turns it even when held");
        studio.control(MaterialStudioEvent::Turn(f32::NAN));
        assert!(studio.yaw.is_finite());
        studio.control(MaterialStudioEvent::Shape(2));
        assert_eq!(studio.state().shape, 2);
        studio.control(MaterialStudioEvent::Shape(9));
        assert_eq!(studio.state().shape, 0, "an unknown shape falls back to the sphere");
        assert_eq!(studio.control(MaterialStudioEvent::Preset(3)), Some(3), "presets go to the host");
    }
}
