//! Native editor controls whose state lives outside scene component fields.
use super::*;
use somnium_ui::editor_event::{FoliageBrushField as F, TerrainToolField as T};
const TERRAIN: &[(&str, T, f32, f32)] = &[
    ("aerial_distance", T::AerialDistance, 20., 4000.),
    ("paint_layer", T::PaintLayer, 0., 7.),
    ("tile_scale", T::TileScale, 0.01, 10000.),
    ("relief", T::Relief, 0., 4.),
    ("wetness", T::Wetness, 0., 1.),
    ("macro_strength", T::MacroStrength, 0., 1.),
    ("debug_view", T::DebugView, 0., 35.),
    ("morph_start", T::MorphStart, 0., 1.),
    ("splat_noise", T::SplatNoise, 0., 1.),
    ("splat_noise_scale", T::SplatNoiseScale, 0.02, 2.),
    ("macro_octaves", T::MacroOctaves, 0., 4.),
    ("damp_tint", T::DampTint, 0., 4.),
    ("sky_visibility", T::SkyVisibility, 0., 1.),
    ("relief_takeover", T::ReliefTakeover, 10., 600.),
];
const FOLIAGE: &[(&str, F, f32, f32)] = &[
    ("density", F::Density, 0., 40.),
    ("radius", F::Radius, 0.25, 200.),
    ("max_slope", F::MaxSlope, 0., 90.),
    ("kind", F::Kind, 0., 255.),
    ("scale_min", F::ScaleMin, 0.01, 1000.),
    ("scale_max", F::ScaleMax, 0.01, 1000.),
    ("min_layer_weight", F::MinLayerWeight, 0., 1.),
];
fn leaf(value: &str) -> Result<&str, String> {
    if value.is_empty()
        || value.len() > 128
        || value == "."
        || value == ".."
        || value.contains(['/', '\\', ':', '\0'])
    {
        Err("name must be a single filename".into())
    } else {
        Ok(value)
    }
}
fn content_path(host: &AuthoringHost, value: &str) -> Result<PathBuf, String> {
    let p = Path::new(value);
    if !p.starts_with(&host.project.manifest.content) {
        return Err("path must be inside project content".into());
    }
    host.project.resolve(p)
}
fn panel(value: &str) -> Result<somnium_ui::floating::FloatingKind, String> {
    use somnium_ui::floating::FloatingKind as P;
    match value {
        "outliner" => Ok(P::Outliner),
        "details" => Ok(P::Details),
        "viewport" => Ok(P::Viewport),
        "output_log" => Ok(P::OutputLog),
        _ => Err("panel must be outliner, details, viewport or output_log".into()),
    }
}
impl<G: GameApp> Engine<G> {
    pub(super) fn query_authoring_controls(&self, p: &Value) -> Result<Value, String> {
        if p["kind"] == "foliage_settings" || p["action"] == "foliage_settings" {
            let b = self.foliage_brush;
            return Ok(
                json!({"ok":true,"values":{"density":b.density,"radius":b.radius,"max_slope":b.max_slope_deg,"kind":b.kind,"scale_min":b.scale_min,"scale_max":b.scale_max,"min_layer_weight":b.min_layer_weight},"fields":FOLIAGE.iter().map(|(id,_,min,max)|json!({"id":id,"min":min,"max":if *id=="kind"{(FOLIAGE_PALETTE.len()-1) as f32}else{*max}})).collect::<Vec<_>>(),"scope":"session brush settings; pass previous_value to restore"}),
            );
        }
        let e = somnium_ecs::PersistentId::parse_hex(required(p, "entity")?)
            .and_then(|id| self.world.entity_by_persistent_id(id))
            .ok_or("unknown terrain")?;
        let c = self
            .world
            .get::<TerrainComponent>(e)
            .ok_or("entity is not terrain")?;
        let r = self.renderer.as_ref().ok_or("renderer unavailable")?;
        let t = r.terrain(c.terrain_id).ok_or("terrain unavailable")?;
        Ok(
            json!({"ok":true,"values":{"aerial_distance":r.aerial_split,"paint_layer":self.terrain_brush.paint_layer,"tile_scale":t.layers.get(self.terrain_brush.paint_layer).map(|v|v.tiling),"relief":t.parallax_scale,"wetness":t.wetness,"macro_strength":t.macro_strength,"debug_view":self.terrain_debug_view,"morph_start":t.lod_morph_start,"splat_noise":t.weight_noise_strength,"splat_noise_scale":t.weight_noise_scale,"macro_octaves":t.macro_octave_scale,"damp_tint":t.skyvis_tint,"sky_visibility":t.sky_visibility_strength,"relief_takeover":t.relief_takeover},"fields":TERRAIN.iter().map(|(id,_,min,max)|json!({"id":id,"min":min,"max":if *id=="paint_layer"{t.layers.len().saturating_sub(1) as f32}else{*max}})).collect::<Vec<_>>(),"scope":"native renderer preview controls; pass previous_value to restore"}),
        )
    }
    pub(super) fn execute_authoring_controls(&mut self, p: &Value) -> Result<Value, String> {
        let q = self.query_authoring_controls(p)?;
        let name = required(p, "field")?;
        let previous = q["values"].get(name).ok_or("unknown control")?.clone();
        if p["expected_value"] != previous {
            return Err("control_conflict: query the control before changing it".into());
        }
        let value = p["value"].as_f64().ok_or("finite numeric value required")? as f32;
        let field = q["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["id"] == name)
            .ok_or("unknown control")?;
        if !value.is_finite()
            || f64::from(value) < field["min"].as_f64().unwrap()
            || f64::from(value) > field["max"].as_f64().unwrap()
        {
            return Err("value outside the native control range".into());
        }
        let event = if p["action"] == "foliage_settings" {
            EditorEvent::SetFoliageBrushValue {
                field: FOLIAGE.iter().find(|v| v.0 == name).unwrap().1,
                value,
                live: false,
            }
        } else {
            EditorEvent::SetTerrainToolValue {
                field: TERRAIN.iter().find(|v| v.0 == name).unwrap().1,
                value,
                live: false,
            }
        };
        self.handle_editor_event(event);
        let mut q = self.query_authoring_controls(p)?;
        q["previous_value"] = previous;
        Ok(q)
    }
    pub(super) fn execute_authoring_content(
        &mut self,
        host: &mut AuthoringHost,
        p: &Value,
    ) -> Result<Value, String> {
        let op = required(p, "operation")?;
        if matches!(op, "assign_material" | "make_unique") {
            let e = self.authoring_target(host, p)?;
            let path = content_path(host, required(p, "path")?)?;
            if !path.is_file() {
                return Err("source asset is absent".into());
            }
            let before = self.undo_stack.history().1;
            let mut copy = None;
            if op == "assign_material" {
                somnium_asset::material::load_material(&path)?;
                let relative = path
                    .strip_prefix(&self.config.content_root)
                    .map_err(|_| "content root mismatch")?;
                let asset = somnium_asset::database::AssetId::from_relative_path(relative);
                self.handle_editor_event(EditorEvent::AssignMaterial {
                    entities: vec![e],
                    asset,
                });
            } else {
                let schema = self
                    .type_registry
                    .by_name(required(p, "component")?)
                    .ok_or("unknown component")?;
                let field = schema
                    .field_by_name(required(p, "field")?)
                    .ok_or("unknown field")?;
                if field.read_only || !field.flags.contains(somnium_ecs::reflect::FieldFlags::EDIT)
                {
                    return Err("field is read-only".into());
                }
                let value =
                    (schema.read_field)(&self.world, e, field.id).ok_or("component absent")?;
                if !matches!(value, somnium_ecs::reflect::ReflectValue::Asset(_)) {
                    return Err("field must be an asset reference".into());
                }
                let target = somnium_asset::material::unique_sibling(&path);
                self.handle_editor_event(EditorEvent::MakeAssetUnique {
                    source: path.to_string_lossy().into_owned(),
                    entity: e,
                    component: schema.stable_id,
                    field: field.id,
                });
                if !target.is_file() {
                    return Err("native asset copy failed; see Output Log".into());
                }
                copy = Some(target);
            }
            if before == self.undo_stack.history().1 {
                return Err("native assignment was unchanged or refused".into());
            }
            return Ok(
                json!({"ok":true,"path":copy,"history_cursor":self.undo_stack.history().1,"note":"Assignment uses scene undo. A created asset copy remains in Content when assignment is undone."}),
            );
        }
        let name = leaf(required(p, "name")?)?;
        let (event, target) = if op == "rename" {
            let source = content_path(host, required(p, "path")?)?;
            if !source.exists() {
                return Err("source does not exist".into());
            }
            let mut target = source.with_file_name(name);
            if target.extension().is_none() {
                if let Some(ext) = source.extension() {
                    target.set_extension(ext);
                }
            }
            let relative = target
                .strip_prefix(&host.project.root)
                .map_err(|_| "target outside project")?;
            host.project.resolve(relative)?;
            if target.exists() {
                return Err("rename target already exists".into());
            }
            (
                EditorEvent::RenameContentItem {
                    path: source.to_string_lossy().into_owned(),
                    name: name.into(),
                },
                target,
            )
        } else {
            let parent = content_path(host, required(p, "parent")?)?;
            if !parent.is_dir() {
                return Err("parent folder is absent".into());
            }
            let parent_text = parent.to_string_lossy().into_owned();
            let mut target = parent.join(name);
            let event = match op {
                "new_folder" => EditorEvent::CreateContentFolder {
                    parent: parent_text,
                    name: name.into(),
                },
                "new_script" => {
                    target.set_extension("luau");
                    EditorEvent::CreateContentScript {
                        parent: parent_text,
                        name: name.into(),
                    }
                }
                "new_material" => {
                    target.set_extension("sommat");
                    EditorEvent::CreateContentMaterial {
                        parent: parent_text,
                        name: name.into(),
                    }
                }
                _ => return Err("unknown content operation".into()),
            };
            if target.exists() {
                return Err("content target already exists".into());
            }
            (event, target)
        };
        let selected = self.selection.primary;
        if op == "new_script" {
            if p.get("entity").is_some() {
                self.authoring_target(host, p)?;
            } else {
                self.selection.set_single(None);
            }
        }
        self.handle_editor_event(event);
        if op == "new_script" && p.get("entity").is_none() {
            self.selection.set_single(selected);
            self.after_selection_change();
        }
        if !target.exists() {
            return Err("native content operation failed; see Output Log".into());
        }
        Ok(
            json!({"ok":true,"path":target,"scope":"native Content operation; files are not part of scene undo"}),
        )
    }
    pub(super) fn query_authoring_workspace(&mut self, p: &Value) -> Result<Value, String> {
        if p["target"] == "localisation" {
            let mut query = p.clone();
            query["operation"] = json!("query");
            return self
                .ui_manager
                .as_mut()
                .ok_or("UI unavailable")?
                .authoring_editor("localisation", &query);
        }
        Ok(
            json!({"ok":true,"panels":somnium_ui::floating::FloatingKind::ALL.iter().map(|kind|{let window=self.floating.iter().find(|w|w.kind==*kind);json!({"panel":format!("{kind:?}"),"floating":self.ui_manager.as_ref().is_some_and(|ui|ui.is_panel_floating(*kind)),"position":window.and_then(|w|w.window.outer_position().ok()).map(|p|[p.x,p.y]),"size":window.map(|w|[w.window.inner_size().width,w.window.inner_size().height])})}).collect::<Vec<_>>()}),
        )
    }
    pub(super) fn execute_authoring_workspace(
        &mut self,
        _host: &mut AuthoringHost,
        p: &Value,
    ) -> Result<Value, String> {
        let op = required(p, "operation")?;
        if p["target"] == "localisation" {
            if matches!(op, "save" | "export") {
                let q = self
                    .ui_manager
                    .as_mut()
                    .ok_or("UI unavailable")?
                    .authoring_editor("localisation", &json!({"operation":"query"}))?;
                if p["expected_view"] != q["view_token"] {
                    return Err("localisation changed; refresh first".into());
                }
                let path = if op == "save" {
                    self.save_localisation_result()?
                } else {
                    self.export_localisation_result()?
                };
                return Ok(json!({"ok":true,"path":path}));
            }
            let ui = self.ui_manager.as_mut().ok_or("UI unavailable")?;
            ui.show_localisation();
            return ui.authoring_editor("localisation", p);
        }
        let kind = panel(required(p, "panel")?)?;
        match op {
            "float" => {
                if !self
                    .ui_manager
                    .as_ref()
                    .is_some_and(|ui| ui.is_panel_floating(kind))
                {
                    self.ui_manager
                        .as_mut()
                        .ok_or("UI unavailable")?
                        .float_panel(kind);
                }
                Ok(
                    json!({"ok":true,"status":"queued; query workspace until the OS window is present"}),
                )
            }
            "dock" => {
                self.handle_editor_event(EditorEvent::ClosePanelWindow(kind));
                self.query_authoring_workspace(p)
            }
            "place" => {
                let position: [i32; 2] = serde_json::from_value(p["position"].clone())
                    .map_err(|_| "position needs [x,y] screen pixels")?;
                let size: [u32; 2] = serde_json::from_value(p["size"].clone())
                    .map_err(|_| "size needs [width,height] pixels")?;
                if position.iter().any(|v| !(-32768..=32767).contains(v))
                    || size.iter().any(|v| !(320..=8192).contains(v))
                {
                    return Err("invalid panel position or size".into());
                }
                let window = self
                    .floating
                    .iter()
                    .find(|w| w.kind == kind)
                    .ok_or("float the panel and wait for its OS window first")?;
                window
                    .window
                    .set_outer_position(winit::dpi::PhysicalPosition::new(
                        position[0],
                        position[1],
                    ));
                let _ = window
                    .window
                    .request_inner_size(winit::dpi::PhysicalSize::new(size[0], size[1]));
                self.remember_floating_placement(kind);
                self.floating_layout.flush(true);
                Ok(json!({"ok":true,"status":"requested; query reports actual platform placement"}))
            }
            _ => Err("panel operation must be float, dock or place".into()),
        }
    }
    pub(in crate::app) fn save_localisation_result(&mut self) -> Result<PathBuf, String> {
        let dir = self.locale_dir();
        let table = self
            .ui_manager
            .as_ref()
            .and_then(UiManager::localisation_table)
            .ok_or("localisation table unavailable")?;
        let template = self
            .locale_catalog
            .clone()
            .unwrap_or_else(|| somnium_i18n::Catalog::new("en"));
        let catalog = crate::i18n::table_to_catalog(table, &template);
        crate::i18n::save_catalog(&dir, &catalog)?;
        self.locale_catalog = Some(catalog);
        self.next_asset_scan = Instant::now();
        Ok(dir)
    }
    pub(in crate::app) fn export_localisation_result(&mut self) -> Result<PathBuf, String> {
        let table = self
            .ui_manager
            .as_ref()
            .and_then(UiManager::localisation_table)
            .ok_or("localisation table unavailable")?;
        let path = self.locale_dir().join("localisation.csv");
        std::fs::create_dir_all(self.locale_dir())
            .and_then(|()| std::fs::write(&path, table.to_csv()))
            .map_err(|e| e.to_string())?;
        self.next_asset_scan = Instant::now();
        Ok(path)
    }
}
