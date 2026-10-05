//! Semantic renderer/script actions that reuse the native editor implementations.
use super::*;
use serde::Deserialize;
use somnium_ecs::Entity;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Stroke {
    mode: String,
    radius: f32,
    strength: f32,
    hardness: f32,
    #[serde(default)]
    layer: usize,
    #[serde(default)]
    target_height: f32,
    /// Terrain-local x, z, and exposure seconds for each dab.
    samples: Vec<[f32; 3]>,
}
impl Stroke {
    fn validate(&self) -> Result<(), String> {
        if !(0.05..=128.0).contains(&self.radius)
            || !(0.0..=1.0).contains(&self.strength)
            || !(0.0..=1.0).contains(&self.hardness)
            || !self.target_height.is_finite()
        {
            return Err("invalid brush radius/strength/hardness/target".into());
        }
        if self.samples.is_empty()
            || self.samples.len() > 128
            || self
                .samples
                .iter()
                .any(|p| p.iter().any(|v| !v.is_finite()) || !(0.001..=0.25).contains(&p[2]))
        {
            return Err(
                "stroke needs 1..128 finite [local_x,local_z,seconds] samples; seconds .001..=.25"
                    .into(),
            );
        }
        Ok(())
    }
}
impl<G: GameApp> Engine<G> {
    pub(super) fn authoring_target(
        &mut self,
        host: &mut AuthoringHost,
        p: &Value,
    ) -> Result<Entity, String> {
        host.session
            .check_revision(
                &mut self.world,
                p["expected_revision"]
                    .as_u64()
                    .ok_or("expected_revision required")?,
            )
            .map_err(|e| e.message)?;
        let id = required(p, "entity")?;
        if host.session.protected.contains(id) {
            return Err("entity is protected".into());
        }
        let e = somnium_ecs::PersistentId::parse_hex(id)
            .and_then(|id| self.world.entity_by_persistent_id(id))
            .ok_or("unknown or unloaded entity")?;
        if self.world.get::<EditorFlags>(e).is_some_and(|f| f.locked) {
            return Err("entity is locked".into());
        }
        self.selection.set_single(Some(e));
        self.after_selection_change();
        Ok(e)
    }
    pub(super) fn execute_specialized(
        &mut self,
        host: &mut AuthoringHost,
        p: &Value,
    ) -> Result<Value, String> {
        let action = required(p, "action")?;
        if action == "content" {
            return self.execute_authoring_content(host, p);
        }
        let entity = self.authoring_target(host, p)?;
        if matches!(action, "terrain_settings" | "foliage_settings") {
            return self.execute_authoring_controls(p);
        }

        match action {
            "terrain_stroke" => {
                if self.terrain_stroke.is_some() {
                    return Err("finish the designer's current brush stroke first".into());
                }
                let stroke: Stroke =
                    serde_json::from_value(p["stroke"].clone()).map_err(|e| e.to_string())?;
                stroke.validate()?;
                let mode = match stroke.mode.as_str() {
                    "raise" => BrushMode::Raise,
                    "lower" => BrushMode::Lower,
                    "smooth" => BrushMode::Smooth,
                    "flatten" => BrushMode::Flatten,
                    "noise" => BrushMode::Noise,
                    "paint" => BrushMode::Paint,
                    _ => return Err("unknown sculpt/paint mode".into()),
                };
                let tc = self
                    .world
                    .get::<TerrainComponent>(entity)
                    .copied()
                    .ok_or("entity is not terrain")?;
                let terrain = self
                    .renderer
                    .as_mut()
                    .and_then(|r| r.terrain_mut(tc.terrain_id))
                    .ok_or("terrain is not loaded")?;
                if p["terrain_revision"].as_u64() != Some(terrain.edit_revision) {
                    return Err(
                        "terrain_conflict: query the terrain revision before painting".into(),
                    );
                }
                if stroke.layer >= terrain.layers.len() {
                    return Err("paint layer is out of range".into());
                }
                let is_paint = mode == BrushMode::Paint;
                self.terrain_stroke = Some(TerrainStroke {
                    terrain_id: tc.terrain_id,
                    is_paint,
                    start_heights: if is_paint {
                        vec![]
                    } else {
                        terrain.heightmap.clone()
                    },
                    start_texels: if is_paint {
                        terrain.splatmap.data.clone()
                    } else {
                        vec![]
                    },
                    region: None,
                });
                let brush = somnium_renderer::terrain::brush::TerrainBrush {
                    mode,
                    radius: stroke.radius,
                    strength: stroke.strength,
                    hardness: stroke.hardness,
                    paint_layer: stroke.layer,
                    target_height: stroke.target_height,
                    ..self.terrain_brush
                };
                self.terrain_brush = brush;
                for [x, z, dt] in stroke.samples {
                    let region = if is_paint {
                        apply_paint(terrain, &brush, x, z, dt)
                    } else {
                        apply_sculpt(terrain, &brush, x, z, dt)
                    };
                    if let (Some(r), Some(s)) = (region, self.terrain_stroke.as_mut()) {
                        s.region = Some(s.region.map_or(r, |a| {
                            (a.0.min(r.0), a.1.min(r.1), a.2.max(r.2), a.3.max(r.3))
                        }));
                    }
                }
                let revision = terrain.edit_revision;
                self.end_terrain_stroke();
                self.scene_dirty = true;
                Ok(json!({"ok":true,"terrain_revision":revision,"undo":"one terrain stroke"}))
            }
            "vertex_paint" => self.execute_vertex_paint(entity, p),
            "foliage_stroke" => {
                let tc = self
                    .world
                    .get::<TerrainComponent>(entity)
                    .copied()
                    .ok_or("entity is not terrain")?;
                let samples: Vec<[f32; 2]> = serde_json::from_value(p["samples"].clone())
                    .map_err(|_| "samples need terrain-local [x,z]")?;
                if samples.is_empty()
                    || samples.len() > 128
                    || samples.iter().flatten().any(|v| !v.is_finite())
                {
                    return Err("foliage stroke needs 1..128 finite samples".into());
                }
                let kind = p["kind"]
                    .as_u64()
                    .filter(|v| *v <= u8::MAX as u64 && self.foliage_kind_exists(*v as u8))
                    .ok_or("kind must be a foliage palette index")?
                    as u8;
                if self.foliage_brush.kind != kind {
                    self.foliage_brush.kind = kind;
                    self.apply_foliage_palette_defaults();
                }
                let mut brush = self.foliage_brush;
                let read = |key: &str, default: f32, min: f32, max: f32| -> Result<f32, String> {
                    let value = p
                        .get(key)
                        .map_or(Some(f64::from(default.clamp(min, max))), Value::as_f64)
                        .ok_or_else(|| format!("{key} must be a number"))?
                        as f32;
                    if (min..=max).contains(&value) {
                        Ok(value)
                    } else {
                        Err(format!("{key} outside {min}..={max}"))
                    }
                };
                brush.radius = read("radius", brush.radius, 0.05, 64.0)?;
                brush.density = read("density", brush.density, 0.01, 10.0)?;
                brush.min_layer_weight =
                    read("min_layer_weight", brush.min_layer_weight, 0.0, 1.0)?;
                brush.single = p["single"].as_bool().unwrap_or(brush.single);
                if !brush.single
                    && samples.len() as f32
                        * std::f32::consts::PI
                        * brush.radius.powi(2)
                        * brush.density
                        > 50_000.0
                {
                    return Err(
                        "foliage request exceeds 50,000 candidate budget; split the stroke".into(),
                    );
                }
                let terrain = self
                    .renderer
                    .as_mut()
                    .and_then(|r| r.terrain_mut(tc.terrain_id))
                    .ok_or("terrain not loaded")?;
                if p["terrain_revision"].as_u64() != Some(terrain.edit_revision) {
                    return Err("terrain_conflict: query terrain before painting".into());
                }
                let before = terrain.painted_foliage.clone();
                let mut placed = 0;
                for (i, center) in samples.into_iter().enumerate() {
                    if p["erase"].as_bool().unwrap_or(false) {
                        somnium_renderer::terrain::foliage_paint::erase(
                            &mut terrain.painted_foliage,
                            center,
                            brush.radius,
                            Some(kind),
                        );
                    } else {
                        let mut instances = std::mem::take(&mut terrain.painted_foliage);
                        let report = somnium_renderer::terrain::foliage_paint::paint(
                            &mut instances,
                            &brush,
                            center,
                            (p["seed"].as_u64().unwrap_or(0) as u32).wrapping_add(i as u32),
                            |x, z| terrain.ground_sample(x, z, brush.layer),
                        );
                        placed += report.placed;
                        terrain.painted_foliage = instances;
                    }
                }
                if terrain.painted_foliage != before {
                    terrain.edit_revision = terrain.edit_revision.wrapping_add(1);
                    self.undo_stack.push_silent(Box::new(
                        crate::editor_commands::FoliageEditCmd::new(
                            tc.terrain_id,
                            before,
                            terrain.painted_foliage.clone(),
                            self.terrain_restore_queue.clone(),
                        ),
                    ));
                    self.scene_dirty = true;
                }
                self.foliage_brush = brush;
                Ok(
                    json!({"ok":true,"placed":placed,"total":terrain.painted_foliage.len(),"terrain_revision":terrain.edit_revision,"undo":"one foliage stroke"}),
                )
            }
            "designer" => {
                use somnium_ui::editor_event::DesignerTool as D;
                let tool = match required(p, "tool")? {
                    "navigation_bake" => D::NavigationBake,
                    "navigation_clear" => D::NavigationClear,
                    "behavior_edit" => D::BehaviorEdit,
                    "animation_rig" => D::AnimationRig,
                    "animation_reset" => D::AnimationReset,
                    "animation_reload" => D::AnimationReload,
                    "animation_events" => D::AnimationEvents,
                    "animation_save_events" => D::AnimationSaveEvents,
                    "save_play" => D::SavePlay,
                    "load_play" => D::LoadPlay,
                    _ => return Err("unsupported designer tool".into()),
                };
                let status = self.run_designer_tool_result(tool)?;
                Ok(json!({"ok":true,"status":status}))
            }
            "script" => {
                let operation = required(p, "operation")?;
                let index = p["index"].as_u64().unwrap_or(0) as usize;
                let event = match operation {
                    "attach" => {
                        let path = Path::new(required(p, "path")?);
                        if !path.starts_with(&host.project.manifest.content)
                            || path.extension().and_then(|e| e.to_str()) != Some("luau")
                        {
                            return Err("scripts must be .luau under content".into());
                        }
                        let path = host.project.resolve(path)?;
                        if !path.is_file() {
                            return Err("script source is absent".into());
                        }
                        EditorEvent::AttachScript(path.to_string_lossy().into_owned())
                    }
                    "detach" => EditorEvent::DetachScript(index),
                    "reorder" => {
                        let delta = p["delta"]
                            .as_i64()
                            .filter(|v| *v == 1 || *v == -1)
                            .ok_or("delta must be -1 or 1")?;
                        EditorEvent::ReorderScript {
                            index,
                            delta: delta as i32,
                        }
                    }
                    "enabled" => EditorEvent::SetScriptEnabled {
                        index,
                        enabled: p["value"].as_bool().ok_or("value must be boolean")?,
                    },
                    "number" => {
                        let value =
                            p["value"].as_f64().ok_or("value must be finite number")? as f32;
                        if !value.is_finite() {
                            return Err("value is not finite".into());
                        }
                        EditorEvent::SetScriptNumber {
                            index,
                            field: required(p, "field")?.into(),
                            value,
                            live: false,
                        }
                    }
                    "bool" => EditorEvent::SetScriptBool {
                        index,
                        field: required(p, "field")?.into(),
                        value: p["value"].as_bool().ok_or("value must be boolean")?,
                    },
                    "reload" => EditorEvent::ReloadScripts,
                    _ => return Err("unknown script operation".into()),
                };
                let before = self.undo_stack.history().1;
                self.handle_editor_event(event);
                host.session.observe(&mut self.world);
                if operation != "reload" && before == self.undo_stack.history().1 {
                    return Err("script edit was refused or unchanged; inspect script diagnostics/selection".into());
                }
                Ok(
                    json!({"ok":true,"revision":host.session.revision,"history_cursor":self.undo_stack.history().1}),
                )
            }
            _ => Err("unknown specialized action".into()),
        }
    }
    /// MCP vertex paint on one entity. `op`:
    /// - `get`: vertex count and per-channel mean/max;
    /// - `stroke`: world-space `samples` [[x,y,z]..] with radius, strength,
    ///   falloff, `channels` and `erase`;
    /// - `fill`: `channels` set to `value`;
    /// - `clear`;
    /// - `generate`: `layers` [{channel, rule, amount, blend, ..rule params}];
    /// - `auto_weather`: the editor button (`metal` overrides the material);
    /// - `layers`: `layers` [{channel, material, tiling, height_contrast,
    ///   slope_min, slope_max, breakup, breakup_scale}] sets what each
    ///   channel paints. `material` is a content-relative `.sommat` path or
    ///   asset id, or null for the built-in weathering; omitted keys keep
    ///   their value;
    /// - `preview`: `mode` 0 off, 1 all masks, 2..=5 one channel.
    ///
    /// Channels are `dirt`/`rust`/`wet`/`blood`, `r`/`g`/`b`/`a` or `1`..`4`.
    fn execute_vertex_paint(&mut self, entity: Entity, p: &Value) -> Result<Value, String> {
        use crate::vertex_paint as vp;
        let mesh = self
            .world
            .get::<MeshComponent>(entity)
            .copied()
            .ok_or("entity has no mesh")?;
        let count = self
            .renderer
            .as_ref()
            .and_then(|r| r.geometry.static_vertex_count(mesh.vertex_offset))
            .ok_or("mesh is not a static upload")? as usize;
        let colors = vp::colors_of(&self.world, entity, count);
        let channel = |name: &str| -> Result<usize, String> {
            vp::channel_index(name)
                .ok_or_else(|| format!("unknown channel {name}; use dirt, rust, wet, blood or r, g, b, a"))
        };
        let channels = |p: &Value| -> Result<[bool; 4], String> {
            let mut on = [false; 4];
            for name in p["channels"].as_array().ok_or("channels must be an array")? {
                on[channel(name.as_str().ok_or("channel names are strings")?)?] = true;
            }
            Ok(on)
        };
        let num = |key: &str, default: f32, min: f32, max: f32| -> Result<f32, String> {
            let v = p.get(key).map_or(Some(f64::from(default)), Value::as_f64).ok_or(format!("{key} must be a number"))? as f32;
            if (min..=max).contains(&v) { Ok(v) } else { Err(format!("{key} outside {min}..={max}")) }
        };
        let model = self.entity_model(entity);
        let read_mesh = |this: &Self| -> Result<vp::PaintMesh, String> {
            let (Some(r), Some(ctx)) = (this.renderer.as_ref(), this.render_ctx.as_ref()) else {
                return Err("renderer unavailable".into());
            };
            vp::PaintMesh::read(&this.world, r, ctx, entity).ok_or_else(|| "mesh readback failed".into())
        };
        let op = required(p, "op")?;
        if op == "layers" {
            let before = self.world.get::<vp::VertexPaintComponent>(entity).cloned();
            let mut after = before.clone().unwrap_or_default();
            let layers = p["layers"].as_array().ok_or("layers must be an array")?;
            if layers.is_empty() || layers.len() > 4 {
                return Err("layers needs 1..4 entries".into());
            }
            for layer in layers {
                let c = channel(layer["channel"].as_str().ok_or("layer channel required")?)?;
                match &layer["material"] {
                    Value::Null if layer.get("material").is_none() => {}
                    Value::Null => after.set_layer_asset(c, somnium_asset::database::AssetId::NONE),
                    Value::String(text) if text.is_empty() => {
                        after.set_layer_asset(c, somnium_asset::database::AssetId::NONE);
                    }
                    Value::String(text) => {
                        let id = match u128::from_str_radix(text, 16) {
                            Ok(raw) if text.len() == 32 => somnium_asset::database::AssetId::from_raw(raw),
                            _ => somnium_asset::database::AssetId::from_relative_path(text),
                        };
                        let known = self
                            .asset_gate
                            .published()
                            .and_then(|snapshot| snapshot.get(id))
                            .is_some_and(|record| record.kind == somnium_asset::database::AssetKind::Material);
                        if !known {
                            return Err(format!("{text} is not a material asset in this project"));
                        }
                        after.set_layer_asset(c, id);
                    }
                    _ => return Err("layer material must be a path, an asset id or null".into()),
                }
                let mut set = |key: &str, slot: &mut f32, min: f32, max: f32| -> Result<(), String> {
                    if let Some(v) = layer.get(key) {
                        let v = v.as_f64().ok_or(format!("{key} must be a number"))? as f32;
                        if !(min..=max).contains(&v) {
                            return Err(format!("{key} outside {min}..={max}"));
                        }
                        *slot = v;
                    }
                    Ok(())
                };
                set("tiling", &mut after.tiling[c], 0.01, 16.0)?;
                set("height_contrast", &mut after.height_contrast[c], 0.0, 1.0)?;
                set("slope_min", &mut after.slope_min[c], -1.0, 1.0)?;
                set("slope_max", &mut after.slope_max[c], -1.0, 1.0)?;
                set("breakup", &mut after.breakup[c], 0.0, 1.0)?;
                set("breakup_scale", &mut after.breakup_scale[c], 0.05, 50.0)?;
            }
            let changed = before.as_ref() != Some(&after);
            if changed {
                match self.world.get_mut::<vp::VertexPaintComponent>(entity) {
                    Some(c) => *c = after.clone(),
                    None => {
                        let _ = self.world.insert_component(entity, after.clone());
                    }
                }
                self.undo_stack.push_silent(Box::new(vp::VertexPaintCmd { entity, before, after }));
                self.scene_dirty = true;
            }
            return Ok(json!({"ok":true,"changed":changed,"undo":"one vertex paint step"}));
        }
        if op == "preview" {
            // Show the masks (0 off, 1 all, 2..=5 dirt/rust/wet/blood) by
            // entering Vertex Paint mode, as the panel's Preview does.
            let mode = p["mode"].as_u64().filter(|m| *m <= 5).ok_or("mode must be 0..=5")? as u32;
            self.vertex_paint_preview = mode;
            self.vertex_paint_active = mode != 0;
            return Ok(json!({"ok":true,"preview":mode}));
        }
        let new = match op {
            "get" => {
                let mut mean = [0.0f64; 4];
                let mut max = [0.0f32; 4];
                for c in &colors {
                    for (i, v) in vp::unpack(*c).iter().enumerate() {
                        mean[i] += f64::from(*v);
                        max[i] = max[i].max(*v);
                    }
                }
                let n = count.max(1) as f64;
                let stats: serde_json::Map<String, Value> = vp::CHANNELS
                    .iter()
                    .enumerate()
                    .map(|(i, name)| ((*name).to_owned(), json!({"mean": mean[i] / n, "max": max[i]})))
                    .collect();
                let paint = self.world.get::<vp::VertexPaintComponent>(entity).cloned().unwrap_or_default();
                let layers: Vec<Value> = paint
                    .layer_assets()
                    .iter()
                    .enumerate()
                    .map(|(i, asset)| {
                        json!({
                            "channel": vp::CHANNELS[i],
                            "material": (*asset != somnium_asset::database::AssetId::NONE).then(|| asset.to_string()),
                            "tiling": paint.tiling[i],
                            "height_contrast": paint.height_contrast[i],
                            "slope_min": paint.slope_min[i],
                            "slope_max": paint.slope_max[i],
                            "breakup": paint.breakup[i],
                            "breakup_scale": paint.breakup_scale[i],
                        })
                    })
                    .collect();
                return Ok(json!({"ok":true,"vertex_count":count,"painted":colors.iter().any(|c| *c != 0),"channels":stats,"layers":layers}));
            }
            "clear" => vec![0; count],
            "fill" => {
                let mut c = colors.clone();
                vp::fill(&mut c, channels(p)?, num("value", 1.0, 0.0, 1.0)?);
                c
            }
            "stroke" => {
                let samples: Vec<[f32; 3]> = serde_json::from_value(p["samples"].clone())
                    .map_err(|_| "samples need world-space [x,y,z]")?;
                if samples.is_empty() || samples.len() > 256 || samples.iter().flatten().any(|v| !v.is_finite()) {
                    return Err("stroke needs 1..256 finite samples".into());
                }
                let brush = vp::VertexPaintBrush {
                    radius: num("radius", 0.5, 0.01, 50.0)?,
                    strength: num("strength", 0.5, 0.0, 1.0)?,
                    falloff: num("falloff", 0.75, 0.0, 1.0)?,
                    channels: channels(p)?,
                    erase: p["erase"].as_bool().unwrap_or(false),
                };
                let positions = read_mesh(self)?.world(model).0;
                let mut c = colors.clone();
                for s in samples {
                    vp::dab(&mut c, &positions, glam::Vec3::from_array(s), &brush);
                }
                c
            }
            "generate" | "auto_weather" => {
                let mesh = read_mesh(self)?;
                let (positions, normals) = mesh.world(model);
                if op == "auto_weather" {
                    let metal = p["metal"].as_bool().unwrap_or_else(|| {
                        self.world
                            .get::<MaterialComponent>(entity)
                            .and_then(|m| self.renderer.as_ref()?.materials_pool.get(m.runtime_id))
                            .is_some_and(|m| m.metallic > 0.5)
                    });
                    vp::auto_weather(&colors, &positions, &normals, &mesh.indices, metal)
                } else {
                    let layers = p["layers"].as_array().ok_or("layers must be an array")?;
                    if layers.is_empty() || layers.len() > 16 {
                        return Err("generate needs 1..16 layers".into());
                    }
                    let mut c = colors.clone();
                    for layer in layers {
                        let f = |key: &str, default: f32| layer[key].as_f64().map_or(default, |v| v as f32);
                        let rule = match layer["rule"].as_str().ok_or("layer rule required")? {
                            "ground" => vp::Rule::Ground { height: f("height", 0.6), ground: layer["ground"].as_f64().map(|v| v as f32) },
                            "up" => vp::Rule::Up { power: f("power", 1.5) },
                            "down" => vp::Rule::Down { power: f("power", 1.5) },
                            "slope" => vp::Rule::Slope { min: f("min", -0.3), max: f("max", 0.3) },
                            "cavity" => vp::Rule::Cavity { gain: f("gain", 3.0) },
                            "edges" => vp::Rule::Edges { gain: f("gain", 2.5) },
                            "noise" => vp::Rule::Noise { scale: f("scale", 1.5), seed: f("seed", 0.0) },
                            "streaks" => vp::Rule::Streaks { scale: f("scale", 1.5), seed: f("seed", 0.0) },
                            "all" => vp::Rule::All,
                            other => return Err(format!("unknown rule {other}")),
                        };
                        let blend = match layer["blend"].as_str().unwrap_or("max") {
                            "max" => vp::Blend::Max,
                            "add" => vp::Blend::Add,
                            "set" => vp::Blend::Set,
                            "multiply" => vp::Blend::Multiply,
                            other => return Err(format!("unknown blend {other}")),
                        };
                        let values = vp::generate(rule, &positions, &normals, &mesh.indices);
                        vp::apply(&mut c, channel(layer["channel"].as_str().ok_or("layer channel required")?)?, &values, f("amount", 1.0).clamp(0.0, 1.0), blend);
                    }
                    c
                }
            }
            other => return Err(format!("unknown vertex_paint op {other}")),
        };
        let before = self.world.get::<vp::VertexPaintComponent>(entity).cloned();
        vp::store(&mut self.world, entity, &new);
        let after = self
            .world
            .get::<vp::VertexPaintComponent>(entity)
            .cloned()
            .ok_or("vertex paint was not stored")?;
        let changed = before.as_ref() != Some(&after);
        if changed {
            self.undo_stack.push_silent(Box::new(vp::VertexPaintCmd { entity, before, after }));
            self.scene_dirty = true;
        }
        Ok(json!({"ok":true,"vertex_count":count,"changed":changed,"undo":"one vertex paint step"}))
    }

    pub(super) fn query_authoring_spatial(&self, p: &Value) -> Result<Value, String> {
        let origin = glam::Vec3::from_array(vec3(p, "origin", [0.0; 3])?);
        if p["kind"] == "clearance" {
            let displacement = glam::Vec3::from_array(vec3(p, "displacement", [0.0; 3])?);
            let radius = p["radius"].as_f64().ok_or("capsule radius required")? as f32;
            let half_height = p["half_height"]
                .as_f64()
                .ok_or("capsule cylindrical half_height required")?
                as f32;
            if !(0.01..=2.0).contains(&radius)
                || !(0.0..=4.0).contains(&half_height)
                || !(0.0001..=100.0).contains(&displacement.length())
            {
                return Err("sweep needs radius .01..2m, cylindrical half_height 0..4m and nonzero displacement up to 100m".into());
            }
            let hit = self
                .physics
                .as_ref()
                .ok_or("physics unavailable")?
                .cast_capsule(somnium_physics::animation::CapsuleSweep {
                    position: origin,
                    rotation: glam::Quat::IDENTITY,
                    displacement,
                    half_height,
                    radius,
                    ignore_body: None,
                })
                .map_err(|e| format!("{e:?}"))?;
            return Ok(
                json!({"ok":true,"probe":"physics capsule sweep","clear":hit.is_none(),"hit":hit.map(|h|json!({"fraction":h.fraction,"normal":h.normal.to_array(),"body":h.body.index()})),"note":"Tests the swept path against registered physics colliders; it does not infer collisions for decorative render meshes."}),
            );
        }
        let direction = glam::Vec3::from_array(vec3(p, "direction", [0.0, 0.0, -1.0])?);
        let max_distance = p["max_distance"].as_f64().unwrap_or(1000.0) as f32;
        if direction.length_squared() < 0.000001 || !(0.01..=10000.0).contains(&max_distance) {
            return Err("pick needs a nonzero direction and max_distance .01..10000m".into());
        }
        let direction = direction.normalize();
        let renderer = self.renderer.as_ref().ok_or("renderer unavailable")?;
        let mut hits:Vec<_>=self.world.entities().filter_map(|entity|{
    if self.world.get::<EditorFlags>(entity).is_some_and(|f|f.hidden||f.locked){return None;}
    // Native viewport ranking stores squared distance; the public ray contract uses metres.
    let distance=entity_ray_hit_distance(&self.world,renderer,entity,origin,direction)?.sqrt();
    if distance>max_distance{return None;}
    let id=self.world.persistent_id(entity)?;Some(json!({"entity":id.to_string(),"name":self.world.get::<Name>(entity).map_or("",Name::as_str),"distance":distance,"point":(origin+direction*distance).to_array()}))
  }).collect();
        hits.sort_by(|a, b| {
            a["distance"]
                .as_f64()
                .unwrap_or(f64::MAX)
                .total_cmp(&b["distance"].as_f64().unwrap_or(f64::MAX))
        });
        hits.truncate(64);
        Ok(
            json!({"ok":true,"probe":"native viewport mesh/proxy bounds","hits":hits,"note":"Uses the editor's transformed mesh and authoring-proxy bounds, respecting hidden and locked actors."}),
        )
    }
    pub(super) fn query_authoring_terrain(&mut self, p: &Value) -> Result<Value, String> {
        let id = required(p, "entity")?;
        let e = somnium_ecs::PersistentId::parse_hex(id)
            .and_then(|id| self.world.entity_by_persistent_id(id))
            .ok_or("unknown terrain entity")?;
        let tc = self
            .world
            .get::<TerrainComponent>(e)
            .ok_or("entity is not terrain")?;
        let t = self
            .renderer
            .as_ref()
            .and_then(|r| r.terrain(tc.terrain_id))
            .ok_or("terrain unavailable")?;
        Ok(
            json!({"ok":true,"entity":id,"terrain_revision":t.edit_revision,"vertices":[t.desc.total_vertices_x(),t.desc.total_vertices_z()],"paint_layers":t.layers.len(),"foliage_count":t.painted_foliage.len(),"height_range":t.heightmap.iter().fold([f32::INFINITY,f32::NEG_INFINITY],|[a,b],v|[a.min(*v),b.max(*v)])}),
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strokes_reject_unbounded_or_invalid_work() {
        let mut s = Stroke {
            mode: "raise".into(),
            radius: 2.0,
            strength: 0.3,
            hardness: 0.5,
            layer: 0,
            target_height: 0.0,
            samples: vec![[0.0, 0.0, 0.1]],
        };
        assert!(s.validate().is_ok());
        s.samples[0][2] = 1.0;
        assert!(s.validate().is_err());
        s.samples = vec![[0.0, 0.0, 0.1]; 129];
        assert!(s.validate().is_err());
    }
}
