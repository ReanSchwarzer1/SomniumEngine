//! Main-thread adapter shared by local IPC and the native authoring panel.
use super::*;
#[path = "app_authoring_specialized.rs"]
mod specialized;
#[path = "app_authoring_workspace.rs"]
mod workspace;
use crate::authoring::{
    AuthoringError, AuthoringSession, PlanRequest,
    feedback::{FeedbackKind, FeedbackQueue},
    project::ProjectPaths,
    transport::LocalBridge,
};
use crate::{InputState, KeyCode};
use serde_json::{Value, json};
use somnium_ui::authoring_panel::{
    AuthoringAction, AuthoringEntry, AuthoringField, AuthoringState,
};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

struct CaptureWork {
    id: String,
    path: PathBuf,
    revision: u64,
    frame: u64,
    started: Instant,
}
pub(super) struct AuthoringHost {
    project: ProjectPaths,
    bridge: Option<LocalBridge>,
    session: AuthoringSession,
    feedback: FeedbackQueue,
    documents: crate::authoring::documents::DocumentSession,
    capture: Option<CaptureWork>,
    pub(super) import: Option<(String, u64)>,
    next_refresh: Instant,
    semantic_receipts: std::collections::BTreeMap<String, (Value, Value)>,
}
fn fail(code: &'static str, message: impl Into<String>) -> Value {
    AuthoringError::new(code, message).json()
}
fn required<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} is required"))
}
fn vec3(params: &Value, key: &str, default: [f32; 3]) -> Result<[f32; 3], String> {
    let Some(v) = params.get(key) else {
        return Ok(default);
    };
    let a: [f32; 3] = serde_json::from_value(v.clone())
        .map_err(|_| format!("{key} needs three finite numbers"))?;
    if a.iter().all(|v| v.is_finite()) {
        Ok(a)
    } else {
        Err(format!("{key} is not finite"))
    }
}
const INPUT_KEYS: &[(&str, KeyCode)] = &[
    ("A", KeyCode::KeyA),
    ("B", KeyCode::KeyB),
    ("C", KeyCode::KeyC),
    ("D", KeyCode::KeyD),
    ("E", KeyCode::KeyE),
    ("F", KeyCode::KeyF),
    ("G", KeyCode::KeyG),
    ("H", KeyCode::KeyH),
    ("I", KeyCode::KeyI),
    ("J", KeyCode::KeyJ),
    ("K", KeyCode::KeyK),
    ("L", KeyCode::KeyL),
    ("M", KeyCode::KeyM),
    ("N", KeyCode::KeyN),
    ("O", KeyCode::KeyO),
    ("P", KeyCode::KeyP),
    ("Q", KeyCode::KeyQ),
    ("R", KeyCode::KeyR),
    ("S", KeyCode::KeyS),
    ("T", KeyCode::KeyT),
    ("U", KeyCode::KeyU),
    ("V", KeyCode::KeyV),
    ("W", KeyCode::KeyW),
    ("X", KeyCode::KeyX),
    ("Y", KeyCode::KeyY),
    ("Z", KeyCode::KeyZ),
    ("0", KeyCode::Digit0),
    ("1", KeyCode::Digit1),
    ("2", KeyCode::Digit2),
    ("3", KeyCode::Digit3),
    ("4", KeyCode::Digit4),
    ("5", KeyCode::Digit5),
    ("6", KeyCode::Digit6),
    ("7", KeyCode::Digit7),
    ("8", KeyCode::Digit8),
    ("9", KeyCode::Digit9),
    ("F1", KeyCode::F1),
    ("F2", KeyCode::F2),
    ("F3", KeyCode::F3),
    ("F4", KeyCode::F4),
    ("F5", KeyCode::F5),
    ("F6", KeyCode::F6),
    ("F7", KeyCode::F7),
    ("F8", KeyCode::F8),
    ("F9", KeyCode::F9),
    ("F10", KeyCode::F10),
    ("F11", KeyCode::F11),
    ("F12", KeyCode::F12),
    ("F13", KeyCode::F13),
    ("F14", KeyCode::F14),
    ("F15", KeyCode::F15),
    ("F16", KeyCode::F16),
    ("F17", KeyCode::F17),
    ("F18", KeyCode::F18),
    ("F19", KeyCode::F19),
    ("F20", KeyCode::F20),
    ("F21", KeyCode::F21),
    ("F22", KeyCode::F22),
    ("F23", KeyCode::F23),
    ("F24", KeyCode::F24),
    ("ArrowUp", KeyCode::ArrowUp),
    ("ArrowDown", KeyCode::ArrowDown),
    ("ArrowLeft", KeyCode::ArrowLeft),
    ("ArrowRight", KeyCode::ArrowRight),
    ("Escape", KeyCode::Escape),
    ("Enter", KeyCode::Enter),
    ("Tab", KeyCode::Tab),
    ("Space", KeyCode::Space),
    ("Backspace", KeyCode::Backspace),
    ("Delete", KeyCode::Delete),
    ("Insert", KeyCode::Insert),
    ("Home", KeyCode::Home),
    ("End", KeyCode::End),
    ("PageUp", KeyCode::PageUp),
    ("PageDown", KeyCode::PageDown),
    ("ShiftLeft", KeyCode::ShiftLeft),
    ("ShiftRight", KeyCode::ShiftRight),
    ("ControlLeft", KeyCode::ControlLeft),
    ("ControlRight", KeyCode::ControlRight),
    ("AltLeft", KeyCode::AltLeft),
    ("AltRight", KeyCode::AltRight),
    ("SuperLeft", KeyCode::SuperLeft),
    ("SuperRight", KeyCode::SuperRight),
    ("CapsLock", KeyCode::CapsLock),
    ("NumLock", KeyCode::NumLock),
    ("ScrollLock", KeyCode::ScrollLock),
    ("PrintScreen", KeyCode::PrintScreen),
    ("Pause", KeyCode::Pause),
    ("Backquote", KeyCode::Backquote),
    ("Backslash", KeyCode::Backslash),
    ("BracketLeft", KeyCode::BracketLeft),
    ("BracketRight", KeyCode::BracketRight),
    ("Comma", KeyCode::Comma),
    ("Period", KeyCode::Period),
    ("Minus", KeyCode::Minus),
    ("Equal", KeyCode::Equal),
    ("Semicolon", KeyCode::Semicolon),
    ("Quote", KeyCode::Quote),
    ("Slash", KeyCode::Slash),
    ("NumpadAdd", KeyCode::NumpadAdd),
    ("NumpadSubtract", KeyCode::NumpadSubtract),
    ("NumpadMultiply", KeyCode::NumpadMultiply),
    ("NumpadDivide", KeyCode::NumpadDivide),
    ("NumpadDecimal", KeyCode::NumpadDecimal),
    ("NumpadEnter", KeyCode::NumpadEnter),
    ("Numpad0", KeyCode::Numpad0),
    ("Numpad1", KeyCode::Numpad1),
    ("Numpad2", KeyCode::Numpad2),
    ("Numpad3", KeyCode::Numpad3),
    ("Numpad4", KeyCode::Numpad4),
    ("Numpad5", KeyCode::Numpad5),
    ("Numpad6", KeyCode::Numpad6),
    ("Numpad7", KeyCode::Numpad7),
    ("Numpad8", KeyCode::Numpad8),
    ("Numpad9", KeyCode::Numpad9),
    ("Shift", KeyCode::ShiftLeft),
    ("Control", KeyCode::ControlLeft),
    ("Alt", KeyCode::AltLeft),
    ("Esc", KeyCode::Escape),
];
fn authoring_key(name: &str) -> Result<KeyCode, String> {
    INPUT_KEYS
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, key)| *key)
        .ok_or_else(|| {
            "Unknown key; query authoring.discover input_keys for the supported names".into()
        })
}
fn action(label: &str, method: &str, params: Value) -> AuthoringAction {
    AuthoringAction {
        label: label.into(),
        method: method.into(),
        params,
        enabled: true,
        disabled_reason: None,
    }
}
impl<G: GameApp> Engine<G> {
    /// Open a project in its own registered editor; keep the current unsaved session alive.
    pub(super) fn open_editor_project(&mut self, folder: &Path) -> Result<String, String> {
        let project = ProjectPaths::open(folder)?;
        if self
            .config
            .project_root
            .as_ref()
            .and_then(|p| p.canonicalize().ok())
            .as_ref()
            == Some(&project.root)
        {
            return Ok("This project is already open".into());
        }
        let id = if project.editor_launcher()?.is_some() {
            project.launch_editor()?
        } else {
            let executable = std::env::current_exe().map_err(|e| e.to_string())?;
            if executable.file_stem().and_then(|v| v.to_str()) != Some("hello_engine") {
                return Err("This project has no editor.launch.json. Open generic projects with Hello Engine; game projects need their own built editor to retain custom schemas.".into());
            }
            std::process::Command::new(executable)
                .arg("--project")
                .arg(&project.root)
                .current_dir(&project.root)
                .spawn()
                .map_err(|e| e.to_string())?
                .id()
        };
        Ok(format!(
            "Opened {} in its editor (process {id}); this session is preserved",
            project.manifest.name
        ))
    }

    pub(super) fn poll_authoring(&mut self) {
        if !self.config.authoring_enabled {
            return;
        }
        if self.authoring.is_none() {
            let result = (|| -> Result<AuthoringHost, String> {
                let project = ProjectPaths::open(
                    self.config
                        .project_root
                        .as_deref()
                        .ok_or("authoring requires a project root")?,
                )?;
                let descriptor =
                    project.resolve(&project.manifest.runtime.join("authoring-connection.json"))?;
                let bridge = LocalBridge::bind(&project.manifest.id, &descriptor)
                    .map_err(|e| e.to_string())?;
                let session_id = bridge.session_id().to_owned();
                let mut session = AuthoringSession::new(&project.manifest.id, &session_id);
                session.observe(&mut self.world);
                let documents = crate::authoring::documents::DocumentSession::new(
                    project.clone(),
                    session_id.clone(),
                );
                Ok(AuthoringHost {
                    documents,
                    project,
                    bridge: Some(bridge),
                    session,
                    feedback: FeedbackQueue::new(session_id),
                    capture: None,
                    import: None,
                    next_refresh: Instant::now(),
                    semantic_receipts: Default::default(),
                })
            })();
            match result {
                Ok(host) => {
                    tracing::info!("Local authoring connection ready");
                    self.authoring = Some(host);
                }
                Err(error) => {
                    tracing::error!(%error,"Authoring connection failed");
                    self.config.authoring_enabled = false;
                    if let Some(ui) = self.ui_manager.as_mut() {
                        ui.push_toast(&format!("Authoring unavailable: {error}"));
                    }
                    return;
                }
            }
        }
        let mut host = self.authoring.take().expect("initialized");
        self.finish_authoring_capture(&mut host);
        if let Some(mut bridge) = host.bridge.take() {
            if let Err(error) =
                bridge.poll(|method, params| self.dispatch_authoring(&mut host, method, params))
            {
                tracing::warn!(%error,"Authoring transport poll failed");
            }
            if self.config.authoring_enabled {
                host.bridge = Some(bridge);
            }
        }
        self.start_authoring_feedback(&mut host);
        if Instant::now() >= host.next_refresh {
            self.refresh_authoring_panel(&mut host);
            host.next_refresh = Instant::now() + Duration::from_millis(500);
        }
        self.authoring = Some(host);
    }
    pub(super) fn authoring_request(&mut self, method: &str, params: Value) -> Value {
        let Some(mut host) = self.authoring.take() else {
            return fail("not_ready", "Authoring connection is not ready");
        };
        let result = self.dispatch_authoring(&mut host, method, params);
        host.next_refresh = Instant::now();
        self.authoring = Some(host);
        result
    }
    fn dispatch_authoring(
        &mut self,
        host: &mut AuthoringHost,
        method: &str,
        params: Value,
    ) -> Value {
        let marks_scene_dirty = (method == "authoring.commit"
            && !params["plan_token"]
                .as_str()
                .is_some_and(|t| t.starts_with("doc-plan:")))
            || (method == "authoring.history"
                && params["kind"] != "document"
                && params["action"].as_str().is_some_and(|a| a != "list"));
        let result = match method {
            "authoring.discover" => {
                host.session.observe(&mut self.world);
                let mut result = host.session.discover();
                result["commands"] = json!(
                    self.ui_manager
                        .as_ref()
                        .map(|ui| ui.authoring_commands())
                        .unwrap_or_default()
                );
                let section = params["section"].as_str().unwrap_or("overview");
                if section != "full" {
                    let components = result["components"].take();
                    let commands = result["commands"].take();
                    let game = result["game"].take();
                    result["counts"] = json!({"components":components.as_array().map_or(0,Vec::len),"commands":commands.as_array().map_or(0,Vec::len)});
                    result.as_object_mut().unwrap().remove("components");
                    result.as_object_mut().unwrap().remove("commands");
                    result.as_object_mut().unwrap().remove("game");
                    match section {
                        "components" | "commands" => {
                            let source = if section == "components" {
                                components
                            } else {
                                commands
                            };
                            let id = params["id"].as_str();
                            let search = params["search"].as_str().unwrap_or("").to_lowercase();
                            let offset = params["offset"].as_u64().unwrap_or(0) as usize;
                            let limit = params["limit"].as_u64().unwrap_or(10).min(100) as usize;
                            let items: Vec<_> = source
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter(|v| {
                                    id.is_none_or(|id| v["id"] == id)
                                        && v.to_string().to_lowercase().contains(&search)
                                })
                                .cloned()
                                .collect();
                            result["total"] = json!(items.len());
                            result[section] = json!(
                                items
                                    .into_iter()
                                    .skip(offset)
                                    .take(limit)
                                    .collect::<Vec<_>>()
                            );
                        }
                        "game" => result["game"] = game,
                        "overview" => {
                            result["component_ids"] = json!(
                                components
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .map(|c| c["id"].clone())
                                    .collect::<Vec<_>>()
                            );
                            result["game_label"] = game["label"].clone();
                            result["schema_query"] = json!({"section":"components","id":"component id from component_ids"});
                        }
                        _ => {
                            return fail(
                                "invalid_params",
                                "section must be overview, components, commands, game or full",
                            );
                        }
                    }
                }
                result["input_keys"] =
                    json!(INPUT_KEYS.iter().map(|(name, _)| *name).collect::<Vec<_>>());
                result["execute_actions"] = json!([
                    "terrain_settings",
                    "foliage_settings",
                    "content",
                    "workspace",
                    "bookmark",
                    "material_open",
                    "material_save",
                    "setting",
                    "terrain_stroke",
                    "foliage_stroke",
                    "designer",
                    "script",
                    "editor",
                    "graph_open",
                    "graph_apply",
                    "graph_preview",
                    "graph_save",
                    "open_project",
                    "command",
                    "preset",
                    "select",
                    "protect",
                    "capture",
                    "import",
                    "camera",
                    "input",
                    "tap",
                    "step",
                    "save_scene",
                    "load_scene",
                    "revoke"
                ]);
                result["coverage"] = json!({
                    "scene":"typed plan/commit, reflected Details, persistent IDs, one transaction undo",
                    "graph":{"owner":"retained GraphSurface","operations":["add_node","literal","select","move","connect","disconnect","comment","group","delete","copy","paste","align","replace","undo","redo"],"guards":["expected_view includes document and selection","native pointer/literal gesture exclusion"]},
                    "state_machine":{"owner":"native graph overlay","operations":["state_initial","state_add_transition","state_set_transition","state_remove_transition","state_undo","state_redo"],"guard":"expected_state_view"},
                    "timeline":{"owner":"retained TimelineSurface","operations":["add_group","add_track","add_media","move_media","resize_media","add_marker","move_marker","add_key","move_key","remove_track","select_channel","replace","scrub","undo","redo"],"guard":"expected_view"},
                    "terrain":{"actions":["terrain_stroke","foliage_stroke"],"owner":"renderer terrain buffers + native restore history","guards":["expected_revision","terrain_revision","bounded brush samples"]},
                    "designer":{"tools":["navigation_bake","navigation_clear","behavior_edit","animation_rig","animation_reset","animation_reload","animation_events","animation_save_events","save_play","load_play"],"owner":"native designer handlers; bake completion in NavigationProfile status/pending_cells"},
                    "scripts":{"operations":["attach","detach","reorder","enabled","number","bool","reload"],"source":"validated Luau document publication and exact-byte undo"},
                    "materials":"material_open -> reflected native material draft -> material_save; native dirty/GPU path",
                    "documents":["somui","somgraph","somtimeline","luau","registered game JSON"],
                    "spatial":{"queries":["pick","clearance","bookmarks"],"actions":["camera","bookmark"],"limits":"picking uses native proxy bounds; clearance uses registered physics colliders"},
                    "project":"native File > Open Project or open_project; game launcher preserves private schemas",
                    "renderer_controls":"terrain_settings / foliage_settings query and native value updates with expected_value; session preview scope",
                    "content":{"operations":["new_folder","new_script","new_material","rename","assign_material","make_unique"],"owner":"native Content Drawer handlers"},
                    "workspace":{"operations":["float","dock","place"],"owner":"actual native panel windows"},
                    "localisation":{"operations":["cell","range","replace","sort","filter","undo","redo","save","export"],"owner":"retained DataGrid; shared catalogue save/export"},
                    "preferences":"settings query + setting with expected_value; stored preference has no scene undo",
                    "evidence":"tools/somnium_mcp/editor_acceptance.py generates live case receipts and inventory; metadata describes routes, not a claim that every decorative UI gesture was replayed"
                });
                result
            }
            "authoring.query" => match params["kind"].as_str().unwrap_or("scene") {
                "project" => {
                    json!({"ok":true,"project":host.project.manifest,"root":host.project.root,"revision":host.session.revision})
                }
                "pick" | "clearance" => self
                    .query_authoring_spatial(&params)
                    .unwrap_or_else(|e| fail("spatial_error", e)),
                "bookmarks" => {
                    json!({"ok":true,"bookmarks":self.camera_bookmarks.iter().enumerate().map(|(index,p)|json!({"slot":index+1,"pose":p.map(|(position,yaw,pitch)|json!({"position":position.to_array(),"yaw":yaw,"pitch":pitch}))})).collect::<Vec<_>>()})
                }
                "settings" => {
                    let (world, entity) = self.settings.world();
                    let schemas = self.settings.registry().iter();
                    let values:Vec<_>=schemas.map(|schema|{
                        let fields=(schema.snapshot)(world,entity).unwrap_or_default();
                        json!({"component":schema.stable_id.as_str(),"fields":schema.fields.iter().map(|field|json!({"name":field.name,"schema":crate::authoring::codec::type_schema(&field.ty),"value":fields.get(&field.id).map(|v|crate::authoring::codec::encode(world,&field.ty,v)),"override":self.settings.override_of(schema.stable_id,field.id)})).collect::<Vec<_>>()})
                    }).collect();
                    json!({"ok":true,"settings":values})
                }
                "terrain_settings" | "foliage_settings" => self
                    .query_authoring_controls(&params)
                    .unwrap_or_else(|e| fail("control_error", e)),
                "workspace" => self
                    .query_authoring_workspace(&params)
                    .unwrap_or_else(|e| fail("workspace_error", e)),
                "terrain" => self
                    .query_authoring_terrain(&params)
                    .unwrap_or_else(|e| fail("terrain_error", e)),
                "editor" => self
                    .ui_manager
                    .as_mut()
                    .ok_or("editor unavailable".to_string())
                    .and_then(|ui| {
                        ui.authoring_editor(
                            params["editor"].as_str().unwrap_or("graph"),
                            &json!({"operation":"query"}),
                        )
                    })
                    .unwrap_or_else(|e| fail("editor_error", e)),
                "game" => json!({"ok":true,"game":self.game.authoring_state()}),
                "commands" => {
                    json!({"ok":true,"commands":self.ui_manager.as_ref().map(|ui|ui.authoring_commands()).unwrap_or_default()})
                }
                "diagnostics" => {
                    json!({"ok":true,"frame":self.time.frame_count(),"simulation":format!("{:?}",self.simulation_clock.state),
                    "play_cursor":{"requested":self.play_cursor.requested,"captured":self.play_cursor.window.is_some(),"mode":self.play_cursor.mode,
                        "immersive":self.ui_manager.as_ref().is_some_and(|ui|ui.is_immersive())},
                    "entities":self.world.entity_count(),"revision":host.session.revision,"fps":self.time.fps(),"surface_acquire_ms":self.renderer.as_ref().map(|r|r.profiler.surface_acquire_ms),"frame_cpu_ms":self.renderer.as_ref().map(|r|r.profiler.frame_cpu_ms),"cumulative_stage_ms":self.authoring_frame_timings,
                    "adapter":self.render_ctx.as_ref().map(|r|format!("{:?}",r.adapter.get_info())),
                    "cpu":self.renderer.as_ref().map(|r|r.profiler.cpu_results().iter().map(|s|json!({"name":s.name,"ms":s.ms})).collect::<Vec<_>>()),
                    "gpu":self.renderer.as_ref().map(|r|r.profiler.results().iter().map(|s|json!({"name":s.name,"ms":s.ms})).collect::<Vec<_>>()),
                    "simulation_seconds":self.simulation_clock.elapsed_seconds,"pending_steps":self.pending_steps,
                    "camera":self.renderer.as_ref().map(|r|r.camera_pos.to_array())})
                }
                "documents" => host.documents.list().unwrap_or_else(|e| e.json()),
                "document" => host.documents.query(&params).unwrap_or_else(|e| e.json()),
                "scene" => host
                    .session
                    .query(&mut self.world, &params)
                    .unwrap_or_else(|e| e.json()),
                _ => fail(
                    "unsupported",
                    "This query kind is not implemented; inspect discovery",
                ),
            },
            "authoring.plan" if params["kind"] == "document" => {
                host.documents.plan(&params).unwrap_or_else(|e| e.json())
            }
            "authoring.plan" => match serde_json::from_value::<PlanRequest>(params) {
                Ok(request) => host
                    .session
                    .plan(&mut self.world, request)
                    .unwrap_or_else(|e| e.json()),
                Err(error) => fail("invalid_params", error.to_string()),
            },
            "authoring.commit"
                if params["plan_token"]
                    .as_str()
                    .is_some_and(|t| t.starts_with("doc-plan:")) =>
            {
                host.documents.commit(&params).unwrap_or_else(|e| e.json())
            }
            "authoring.commit" => host
                .session
                .commit(
                    &mut self.world,
                    &mut self.undo_stack,
                    &mut self.selection.primary,
                    &params,
                )
                .unwrap_or_else(|e| e.json()),
            "authoring.history" if params["kind"] == "document" => {
                host.documents.history(&params).unwrap_or_else(|e| e.json())
            }
            "authoring.history" => host
                .session
                .history(
                    &mut self.world,
                    &mut self.undo_stack,
                    &mut self.selection.primary,
                    &params,
                )
                .unwrap_or_else(|e| e.json()),
            "authoring.execute" => self
                .execute_authoring(host, &params)
                .unwrap_or_else(|e| fail("execution_failed", e)),
            "authoring.jobs" => {
                let result = (|| -> Result<Value, String> {
                    match params["action"].as_str().unwrap_or("list") {
                        "list" => Ok(json!({"ok":true,"jobs":host.feedback.list()})),
                        "get" => Ok(
                            json!({"ok":true,"job":host.feedback.get(required(&params,"id")?).ok_or("unknown job")?}),
                        ),
                        "cancel" => {
                            let id = required(&params, "id")?;
                            let receipt = host.feedback.cancel(id)?;
                            if host.import.as_ref().is_some_and(|(job, _)| job == id) {
                                if let Some(job) = self.import_job.as_ref() {
                                    job.cancel();
                                }
                            }
                            Ok(json!({"ok":true,"job":receipt}))
                        }
                        _ => Err("jobs action must be list, get or cancel".into()),
                    }
                })();
                result.unwrap_or_else(|e| fail("job_error", e))
            }
            _ => fail("method_not_found", "Unknown authoring method"),
        };
        if marks_scene_dirty && result["ok"] == true {
            self.apply_terrain_restores();
            self.scene_dirty = true;
            self.selection.reconcile();
            self.after_selection_change();
        }
        result
    }
    fn execute_authoring(
        &mut self,
        host: &mut AuthoringHost,
        params: &Value,
    ) -> Result<Value, String> {
        let kind = required(params, "action")?;
        if kind.starts_with("document_") {
            return host.documents.execute(params).map_err(|e| e.message);
        }
        match kind {
            "bookmark"=>{
                let slot=params["slot"].as_u64().filter(|v|(1..=9).contains(v)).ok_or("bookmark slot must be 1..9")? as u8;
                let operation=required(params,"operation")?;
                if operation=="set" {self.handle_editor_event(EditorEvent::SetCameraBookmark(slot));}
                else if operation=="recall" {if self.camera_bookmarks[slot as usize-1].is_none(){return Err("bookmark is empty".into());}self.handle_editor_event(EditorEvent::RecallCameraBookmark(slot));}
                else{return Err("bookmark operation must be set or recall".into());}
                Ok(json!({"ok":true,"slot":slot,"status":if operation=="set"{"stored"}else{"queued"}}))
            }
            "material_open"=>{
                let path=Path::new(required(params,"path")?);
                if !path.starts_with(&host.project.manifest.content)||path.extension().and_then(|v|v.to_str())!=Some("sommat"){return Err("material needs .sommat under content".into());}
                let resolved=host.project.resolve(path)?;
                somnium_asset::material::load_material(&resolved)?;
                self.inspect_material_asset(resolved);
                let (_,entity)=self.material_asset_target.ok_or("material editor could not open")?;
                let entity=self.world.ensure_persistent_id(entity).map_err(|e|format!("material edit session: {e:?}"))?;
                host.session.observe(&mut self.world);
                Ok(json!({"ok":true,"entity":entity.to_string(),"revision":host.session.revision,"edit":"Use authoring.plan set operations for somnium.asset.Material fields; native Details and GPU update share the same session"}))
            }
            "material_save"=>{
                self.flush_material_assets();
                let failed:Vec<_>=self.material_documents.values().filter(|d|d.dirty).map(|d|d.path.clone()).collect();
                if !failed.is_empty(){return Err(format!("Some material saves failed: {failed:?}"));}
                Ok(json!({"ok":true,"status":"Material assets saved"}))
            }
            "setting"=>{
                let name=required(params,"component")?;let field_name=required(params,"field")?;
                if name=="somnium.ProjectSettings"&&field_name=="content_root"{return Err("Use Open Project; content roots belong to the manifest".into());}
                let schema=self.settings.registry().by_name(name).ok_or("unknown settings component")?;
                let field=schema.fields.iter().find(|f|f.name==field_name).ok_or("unknown settings field")?;
                let (world,entity)=self.settings.world();
                let snapshot=(schema.snapshot)(world,entity).ok_or("settings absent")?;
                let previous=snapshot.get(&field.id).map(|v|crate::authoring::codec::encode(world,&field.ty,v)).ok_or("setting absent")?;
                if params["expected_value"]!=previous{return Err("setting_conflict: refresh the current preference".into());}
                let value=crate::authoring::codec::decode(world,&field.ty,&params["value"])?;
                self.settings.set(schema.stable_id,field.id,value)?;self.apply_settings();
                Ok(json!({"ok":true,"previous_value":previous,"value":params["value"]}))
            }
            "terrain_stroke"|"foliage_stroke"|"designer"|"script"|"terrain_settings"|"foliage_settings"|"content"=> {
                let id=required(params,"request_id")?;
                if let Some((old,receipt))=host.semantic_receipts.get(id){return if old==params{Ok(receipt.clone())}else{Err("request_id reused with different inputs".into())};}
                let result=self.execute_specialized(host,params)?;
                if host.semantic_receipts.len()>=128{host.semantic_receipts.clear();}
                host.semantic_receipts.insert(id.into(),(params.clone(),result.clone()));Ok(result)
            },
            "editor"=>{
                let request_id=required(params,"request_id")?;
                if let Some((previous,receipt))=host.semantic_receipts.get(request_id){
                    return if previous==params {Ok(receipt.clone())}else{Err("request_id already used with different editor parameters".into())};
                }
                let result=self.ui_manager.as_mut().ok_or("editor unavailable")?.authoring_editor(required(params,"editor")?,params)?;
                if host.semantic_receipts.len()>=128 {host.semantic_receipts.clear();}
                host.semantic_receipts.insert(request_id.into(),(params.clone(),result.clone()));
                Ok(result)
            }
            "graph_open"=>{
                let path=Path::new(required(params,"path")?);
                if !path.starts_with(&host.project.manifest.content)||path.extension().and_then(|v|v.to_str())!=Some("somgraph"){return Err("graph needs .somgraph under project content".into());}
                let source=host.project.resolve(path)?;
                let json=std::fs::read_to_string(&source).map_err(|e|e.to_string())?;
                let catalogue=serde_json::from_str::<Value>(&json).map_err(|e|e.to_string())?["catalogue"].as_str().ok_or("graph catalogue absent")?.to_owned();
                let ui=self.ui_manager.as_mut().ok_or("editor unavailable")?;
                ui.edit_authoring_graph(&catalogue,&json)?;ui.set_authoring_graph_source(&source.to_string_lossy());
                Ok(json!({"ok":true,"status":"queued","path":path}))
            }
            "graph_apply"|"graph_preview"|"graph_save"=>{
                host.session.check_revision(&mut self.world,params["expected_revision"].as_u64().ok_or("expected_revision required")?).map_err(|e|e.message)?;
                let q=self.ui_manager.as_mut().ok_or("editor unavailable")?.authoring_editor("graph",&json!({"operation":"query"}))?;
                if params["expected_view"]!=q["view_token"]{return Err("graph changed; query its current view before applying".into());}
                let doc=q["document"].to_string();let catalogue=q["document"]["catalogue"].as_str().ok_or("graph catalogue missing")?;
                let path=if kind=="graph_save" {let p=Path::new(required(params,"path")?);if !p.starts_with(&host.project.manifest.content)||p.extension().and_then(|v|v.to_str())!=Some("somgraph"){return Err("save requires .somgraph under content".into());}Some(host.project.resolve(p)?)}else{None};
                let status=self.apply_authoring_graph(catalogue,&doc,kind!="graph_save",kind=="graph_preview",path.as_ref().map(|p|p.to_str().ok_or("invalid path")).transpose()?)?;
                host.session.observe(&mut self.world);
                Ok(json!({"ok":true,"status":status,"revision":host.session.revision}))
            }
            "workspace"=>self.execute_authoring_workspace(host,params),
            "open_project"=>{
                let message=self.open_editor_project(Path::new(required(params,"path")?))?;
                Ok(json!({"ok":true,"message":message}))
            }
            "panel_capture"=>{
                self.execute_authoring(host,&json!({"action":"capture","request_id":format!("panel-capture-{}",self.time.frame_count()),"include_editor":true}))
            }
            "command"=>{
                let id=required(params,"id")?;
                let ui=self.ui_manager.as_mut().ok_or("editor unavailable")?;
                let cap=ui.authoring_commands().into_iter().find(|c|c.id==id).ok_or("unknown command")?;
                if !cap.enabled{return Err(cap.disabled_reason.unwrap_or("command disabled".into()));}
                if !ui.run_command_id(id){return Err("command could not be dispatched".into());}
                Ok(json!({"ok":true,"status":"queued","command":id}))
            }
            "preset"=>{
                host.session.observe(&mut self.world);
                let label=required(params,"id")?;
                let mut args=params.get("args").cloned().unwrap_or(json!({}));
                if args.get("position").is_none() && crate::authoring::game_registration().presets.iter().any(|p|p.id==label && p.schema["properties"].get("position").is_some()) {
                    let (position,_,_)=camera_spawn_basis(self.renderer.as_ref(),5.0);args["position"]=json!(position.to_array());
                }
                let plan=host.session.plan(&mut self.world,PlanRequest { expected_revision:host.session.revision,
                    label:format!("Create {label}"),operations:vec![crate::authoring::Operation::Preset{id:label.into(),args}]
                }).map_err(|e|e.message)?;
                self.scene_dirty=true;
                host.session.commit(&mut self.world,&mut self.undo_stack,&mut self.selection.primary,
                    &json!({"request_id":format!("panel:{}",plan["plan_token"]),"plan_token":plan["plan_token"]})).map_err(|e|e.message)
            }
            "select"|"protect"=>{
                let id=required(params,"entity")?;
                let e=somnium_ecs::PersistentId::parse_hex(id).and_then(|id|self.world.entity_by_persistent_id(id)).ok_or("entity is absent or unloaded")?;
                if kind=="protect" {
                    if params["protected"].as_bool().unwrap_or(true){host.session.protected.insert(id.into());}
                    else{host.session.protected.remove(id);}
                }else{
                    self.selection.set_single(Some(e));
                    self.after_selection_change();
                    if params["frame"].as_bool().unwrap_or(true){self.focus_camera_on_selection();}
                }
                Ok(json!({"ok":true,"entity":id}))
            }
            "capture"|"import"=>{
                host.session.observe(&mut self.world);
                let revision=params["expected_revision"].as_u64().unwrap_or(host.session.revision);
                host.session.check_revision(&mut self.world,revision).map_err(|e|e.message)?;
                if kind=="import" {
                    let path=Path::new(required(params,"path")?);
                    let allowed=path.starts_with(&host.project.manifest.content)||path.starts_with(&host.project.manifest.source_assets);
                    if !allowed || !matches!(path.extension().and_then(|v|v.to_str()),Some("glb"|"gltf")){return Err("Import needs a .glb/.gltf under content or source_assets".into());}
                    let resolved=host.project.resolve(path)?;
                    if !resolved.is_file(){return Err("Import file does not exist".into());}
                    vec3(params,"position",[0.0;3])?;
                }
                let receipt=host.feedback.submit(required(params,"request_id")?,
                    if kind=="capture"{FeedbackKind::Capture}else{FeedbackKind::Import},params.clone(),revision)?;
                Ok(json!({"ok":true,"job":receipt}))
            }
            "camera"=>{
                let position=vec3(params,"position",[0.0,2.0,5.0])?;
                let yaw=params["yaw"].as_f64().unwrap_or(-90.0) as f32;
                let pitch=params["pitch"].as_f64().unwrap_or(0.0) as f32;
                if !yaw.is_finite()||!pitch.is_finite(){return Err("camera angles must be finite".into());}
                self.camera_pose_request=Some((glam::Vec3::from_array(position),yaw,pitch.clamp(-89.0,89.0)));
                Ok(json!({"ok":true,"status":"queued"}))
            }
            "tap"=>{
                let key=required(params,"key")?;
                self.execute_authoring(host,&json!({"action":"input","key":key,"pressed":true}))?;
                self.execute_authoring(host,&json!({"action":"input","key":key,"pressed":false}))
            }
            "step"=>{
                if self.simulation_clock.state!=SimulationState::Paused||!self.play_session_active{return Err("Pause a running Play session before stepping".into());}
                let count=params["count"].as_u64().filter(|n|*n>0&&*n<=120).ok_or("count must be 1–120")?;
                if self.pending_steps>0{return Err("Wait for the pending steps to finish".into());}
                for _ in 0..count {self.handle_editor_event(EditorEvent::StepSimulation);}
                Ok(json!({"ok":true,"status":"queued","steps":count}))
            }
            "input"=> {
                let event=if let Some(look)=params.get("look") {
                    let v:[f32;2]=serde_json::from_value(look.clone()).map_err(|_|"look needs [dx,dy]")?;
                    if v.iter().any(|n|!n.is_finite()||n.abs()>3600.0){return Err("look delta is invalid".into());}
                    EngineEvent::MouseMotion{delta_x:v[0],delta_y:v[1]}
                }else{
                let key=authoring_key(required(params,"key")?)?;
                let state=if params["pressed"].as_bool().ok_or("pressed must be boolean")?{InputState::Pressed}else{InputState::Released};
                if key==KeyCode::Escape && state==InputState::Pressed && self.play_session_active {
                    self.release_play_cursor();
                    if let Some(ui)=&mut self.ui_manager {ui.set_immersive(false);}
                    return Ok(json!({"ok":true,"frame":self.time.frame_count(),"cursor_released":true}));
                }
                EngineEvent::KeyInput{key,state}
                };
                let mut ctx=EngineContext::new(&self.time,&self.config,&mut self.world,
                    self.physics.as_mut().ok_or("physics unavailable")?,self.audio.as_mut().ok_or("audio unavailable")?,
                    &mut self.jobs,&mut self.navigation_editor,self.render_ctx.as_ref(),self.renderer.as_mut(),
                    &mut self.selection.primary,self.ui_manager.as_mut().ok_or("editor unavailable")?,
                    crate::camera_speed_from_normalized(self.camera_speed_norm),self.simulation_clock,&mut self.scripts);
                self.game.on_event(&mut ctx,&EngineEvent::WindowFocused(true));
                self.game.on_event(&mut ctx,&event);
                Ok(json!({"ok":true,"frame":self.time.frame_count()}))
            }
            "save_scene"|"load_scene"=>{
                let relative=Path::new(required(params,"path")?);
                if !relative.starts_with(&host.project.manifest.scenes)||relative.extension().and_then(|s|s.to_str())!=Some("json"){
                    return Err("Scene path must be a .json under the declared scenes directory".into());
                }
                let path=host.project.resolve(relative)?;
                if kind=="save_scene"{
                    if self.simulation_clock.state!=SimulationState::Editing{return Err("Stop Play before saving authored scenes".into());}
                    if let Some(parent)=path.parent(){std::fs::create_dir_all(parent).map_err(|e|e.to_string())?;}
                    crate::scene_schema::save_scene_schema(&mut self.world,&self.type_registry,&path.to_string_lossy()).map_err(|e|e.to_string())?;
                    self.scene_dirty=false;
                    Ok(json!({"ok":true,"path":relative}))
                }else{
                    if self.simulation_clock.state!=SimulationState::Editing{return Err("Stop Play before loading authored scenes".into());}
                    let (_,doc)=crate::scene_file::read(&path)?;
                    let mut staged=World::new();
                    crate::scene_schema::scene_from_json(&mut staged,&self.type_registry,&doc).map_err(|e|e.to_string())?;
                    self.handle_editor_event(EditorEvent::LoadScene(path.to_string_lossy().into_owned()));
                    Ok(json!({"ok":true,"path":relative,"status":"dispatched"}))
                }
            }
            "revoke"=>{
                self.config.authoring_enabled=false;
                host.bridge=None;
                if let Some(ui)=self.ui_manager.as_mut(){ui.set_authoring_state(AuthoringState{enabled:true,project:host.project.manifest.name.clone(),connection:"Revoked; restart editor to reconnect".into(),..Default::default()});}
                Ok(json!({"ok":true,"connection":"revoked"}))
            }
            _=>Err("Unsupported execution action; inspect discovery".into()),
        }
    }
    fn finish_authoring_capture(&mut self, host: &mut AuthoringHost) {
        let Some(work) = host.capture.take() else {
            return;
        };
        let cancelled = host
            .feedback
            .get(&work.id)
            .is_some_and(|r| r.cancellation_requested);
        if work.path.is_file() {
            if cancelled {
                let _ = host.feedback.acknowledge_cancelled(&work.id);
                return;
            }
            host.session.observe(&mut self.world);
            let result = (|| -> Result<Value, String> {
                if host.session.revision != work.revision {
                    return Err(
                        "World changed before capture completed; request a fresh revision".into(),
                    );
                }
                let bytes = std::fs::read(&work.path).map_err(|e| e.to_string())?;
                if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
                    return Err("Renderer output is not a PNG".into());
                }
                let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
                let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
                if width == 0 || height == 0 {
                    return Err("Renderer output has empty dimensions".into());
                }
                Ok(
                    json!({"capture":{"path":work.path,"width":width,"height":height,"revision":work.revision,
                    "frame":self.time.frame_count(),"requested_frame":work.frame,
                    "camera":self.renderer.as_ref().map(|r|r.camera_pos.to_array())}}),
                )
            })();
            match result {
                Ok(value) => {
                    let _ = host.feedback.succeed(&work.id, work.revision, value);
                }
                Err(error) => {
                    let _ = host.feedback.fail(&work.id, error);
                }
            }
        } else if work.started.elapsed() > Duration::from_secs(20) {
            let _ = host.feedback.fail(
                &work.id,
                "Renderer did not publish a fresh PNG within 20 seconds",
            );
        } else {
            host.capture = Some(work);
        }
    }
    fn start_authoring_feedback(&mut self, host: &mut AuthoringHost) {
        if host.capture.is_some() || host.import.is_some() {
            return;
        }
        let Some(request) = host.feedback.take_pending() else {
            return;
        };
        let id = &request.job_id;
        let _ = host.feedback.start(id);
        let result = (|| -> Result<(), String> {
            host.session
                .check_revision(&mut self.world, request.expected_revision)
                .map_err(|e| e.message)?;
            match request.kind {
                FeedbackKind::Capture => {
                    let name = format!(
                        "capture-{}-{}.png",
                        host.session.session_id,
                        self.time.frame_count()
                    );
                    let path = host
                        .project
                        .resolve(&host.project.manifest.captures.join(name))?;
                    if path.exists() {
                        return Err("Capture target already exists".into());
                    }
                    self.renderer
                        .as_mut()
                        .ok_or("renderer unavailable")?
                        .request_authoring_capture(
                            &path,
                            request.params["include_editor"].as_bool().unwrap_or(false),
                        )?;
                    host.capture = Some(CaptureWork {
                        id: id.clone(),
                        path,
                        revision: request.expected_revision,
                        frame: self.time.frame_count(),
                        started: Instant::now(),
                    });
                }
                FeedbackKind::Import => {
                    if self.import_job.is_some() {
                        return Err("A designer import is already running".into());
                    }
                    let path = host
                        .project
                        .resolve(Path::new(required(&request.params, "path")?))?;
                    self.queue_import_model(path, vec3(&request.params, "position", [0.0; 3])?);
                    if self.import_job.is_none() {
                        return Err("Import queue rejected the job".into());
                    }
                    host.import = Some((id.clone(), request.expected_revision));
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = host.feedback.fail(id, error);
        }
    }
    pub(super) fn authoring_import_may_publish(&mut self) -> bool {
        let Some(host) = self.authoring.as_mut() else {
            return true;
        };
        let Some((id, revision)) = host.import.as_ref() else {
            return true;
        };
        if host
            .feedback
            .get(id)
            .is_some_and(|r| r.cancellation_requested)
        {
            let _ = host.feedback.acknowledge_cancelled(id);
            host.import = None;
            return false;
        }
        if let Err(error) = host.session.check_revision(&mut self.world, *revision) {
            let _ = host.feedback.fail(id, error.message);
            host.import = None;
            return false;
        }
        true
    }
    pub(super) fn authoring_import_finished(&mut self, result: Result<usize, String>) {
        let Some(host) = self.authoring.as_mut() else {
            return;
        };
        let Some((id, _)) = host.import.take() else {
            return;
        };
        match result {
            Ok(count) if count > 0 => {
                host.session.observe(&mut self.world);
                let _ = host
                    .feedback
                    .succeed(&id, host.session.revision, json!({"spawned":count}));
            }
            Ok(_) => {
                let _ = host
                    .feedback
                    .fail(&id, "Import produced no renderable entities");
            }
            Err(error) => {
                let _ = host.feedback.fail(&id, error);
            }
        }
        host.next_refresh = Instant::now();
    }
    fn refresh_authoring_panel(&mut self, host: &mut AuthoringHost) {
        if !self.config.authoring_enabled {
            return;
        }
        let game = crate::authoring::game_registration();
        let mut entries = Vec::new();
        document_entries(&host.documents, &mut entries);
        for p in game.presets {
            entries.push(AuthoringEntry {
                fields: vec![],
                id: p.id.into(),
                label: p.label.into(),
                category: p.category.into(),
                detail: "Undoable game preset".into(),
                actions: vec![action(
                    "Create",
                    "authoring.execute",
                    json!({"action":"preset","id":p.id,"args":{}}),
                )],
            });
        }
        if let Ok(files) = host.documents.list() {
            for file in files["documents"].as_array().into_iter().flatten() {
                for document in &game.documents {
                    if Path::new(file["path"].as_str().unwrap_or(""))
                        .extension()
                        .and_then(|e| e.to_str())
                        != Some(document.extension)
                    {
                        continue;
                    }
                    entries.push(AuthoringEntry{fields:vec![],id:format!("doc:{}:{}",file["path"],document.id),label:file["path"].as_str().unwrap_or("").into(),category:"Documents".into(),detail:document.label.into(),actions:vec![action("Open","authoring.execute",json!({"action":"document_open","path":file["path"],"document_type":document.id}))]});
                }
            }
        }
        let mut registry = somnium_ecs::reflect::TypeRegistry::new();
        crate::authoring::registration::extend_components(&mut registry);
        for e in self.world.entities() {
            if registry.schemas_on(&self.world, e).is_empty() {
                continue;
            }
            let Some(id) = self.world.persistent_id(e).map(|id| id.to_string()) else {
                continue;
            };
            let name = self
                .world
                .get::<Name>(e)
                .map_or("Game object", Name::as_str);
            entries.push(AuthoringEntry{fields:vec![],id:id.clone(),label:name.into(),category:"Game objects".into(),detail:"Select to edit in Details".into(),
                actions:vec![action("Select / frame","authoring.execute",json!({"action":"select","entity":id,"frame":true})),
                    action(if host.session.protected.contains(&id){"Unprotect"}else{"Protect"},"authoring.execute",json!({"action":"protect","entity":id,"protected":!host.session.protected.contains(&id)}))]});
        }
        for job in host.feedback.list().into_iter().rev().take(8) {
            entries.push(AuthoringEntry {
                fields: vec![],
                id: job.job_id.clone(),
                label: format!("{:?}: {:?}", job.kind, job.status),
                category: "Jobs".into(),
                detail: job
                    .error
                    .unwrap_or_else(|| format!("{:.0}%", job.progress * 100.0)),
                actions: if job.status.terminal() {
                    vec![]
                } else {
                    vec![action(
                        "Cancel",
                        "authoring.jobs",
                        json!({"action":"cancel","id":job.job_id}),
                    )]
                },
            });
        }
        if let Some(ui) = self.ui_manager.as_mut() {
            ui.set_authoring_state(AuthoringState {
                enabled: true,
                project: host.project.manifest.name.clone(),
                connection: "Local MCP available · current Windows user".into(),
                revision: host.session.revision,
                entries,
                actions: vec![
                    action(
                        "Capture",
                        "authoring.execute",
                        json!({"action":"panel_capture"}),
                    ),
                    action(
                        "Play",
                        "authoring.execute",
                        json!({"action":"command","id":"editor.simulation.play"}),
                    ),
                    action(
                        "Stop",
                        "authoring.execute",
                        json!({"action":"command","id":"editor.simulation.stop"}),
                    ),
                    action(
                        "Pause",
                        "authoring.execute",
                        json!({"action":"command","id":"editor.simulation.pause"}),
                    ),
                    action(
                        "Step",
                        "authoring.execute",
                        json!({"action":"step","count":1}),
                    ),
                    action(
                        "Revoke MCP",
                        "authoring.execute",
                        json!({"action":"revoke"}),
                    ),
                ],
            });
        }
    }
}

fn document_entries(
    documents: &crate::authoring::documents::DocumentSession,
    entries: &mut Vec<AuthoringEntry>,
) {
    let Some(d) = documents.draft.as_ref() else {
        return;
    };
    let mut actions = vec![
        action(
            "Save",
            "authoring.execute",
            json!({"action":"document_save"}),
        ),
        action(
            "Revert",
            "authoring.execute",
            json!({"action":"document_revert"}),
        ),
        action(
            "Undo draft",
            "authoring.execute",
            json!({"action":"document_undo"}),
        ),
        action(
            "Redo draft",
            "authoring.execute",
            json!({"action":"document_redo"}),
        ),
        action(
            "Root",
            "authoring.execute",
            json!({"action":"document_focus","pointer":""}),
        ),
    ];
    actions.push(action(
        "Close document",
        "authoring.execute",
        json!({"action":"document_close"}),
    ));
    if !d.focus.is_empty() {
        actions.push(action("Up","authoring.execute",json!({"action":"document_focus","pointer":d.focus.rsplit_once('/').map_or("",|(p,_)|p)})));
    }
    entries.push(AuthoringEntry {
        fields: vec![],
        id: "draft".into(),
        label: format!("{}{}", d.path, if d.dirty() { " *" } else { "" }),
        category: "Document draft".into(),
        detail: format!(
            "Location: {}. Enter or leave a field to apply. Save validates every reference.",
            d.focus
        ),
        actions,
    });
    let Some(value) = d.value.pointer(&d.focus) else {
        return;
    };
    let children: Vec<(String, &Value)> = match value {
        Value::Object(m) => m.iter().map(|(k, v)| (k.clone(), v)).collect(),
        Value::Array(a) => a
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v))
            .collect(),
        _ => vec![],
    };
    for (key, value) in children.into_iter().take(128) {
        let pointer = format!("{}/{}", d.focus, key.replace('~', "~0").replace('/', "~1"));
        let label = value
            .get("id")
            .or_else(|| value.get("text"))
            .and_then(Value::as_str)
            .map(|s| s.chars().take(64).collect::<String>())
            .unwrap_or_else(|| key.clone());
        let mut actions = vec![];
        let fields = if value.is_array() || value.is_object() {
            actions.push(action(
                "Open",
                "authoring.execute",
                json!({"action":"document_focus","pointer":pointer}),
            ));
            vec![]
        } else {
            vec![AuthoringField {
                pointer: pointer.clone(),
                label: key.clone(),
                value: value.clone(),
            }]
        };
        if value.is_boolean() {
            actions.push(action("Toggle","authoring.execute",json!({"action":"document_edit","pointer":pointer,"value":!value.as_bool().unwrap()})));
        }
        if let Ok(index) = key.parse::<usize>() {
            if d.value.pointer(&d.focus).is_some_and(Value::is_array) {
                for (label, operation) in [
                    ("Duplicate", "duplicate"),
                    ("Earlier", "up"),
                    ("Later", "down"),
                    ("Delete", "remove"),
                ] {
                    actions.push(action(label,"authoring.execute",json!({"action":"document_array","pointer":d.focus,"index":index,"operation":operation})));
                }
            }
        }
        entries.push(AuthoringEntry {
            fields,
            id: pointer,
            label,
            category: "Document fields".into(),
            detail: if value.is_array() {
                format!("{} items", value.as_array().unwrap().len())
            } else {
                String::new()
            },
            actions,
        });
    }
}
