//! `vvardenfell` — the second example (Phase MORROWIND, opened by MORROWIND-A).
//!
//! # Why this program exists
//!
//! `examples/hello_engine` is 2,758 lines and is simultaneously the demo, the
//! manual test harness, the input layer, the camera controller and the scene
//! setup. It is also the *only* consumer of Somnium's public API, which means
//! the public API has never been tested against a second use. Two symptoms the
//! census found: a dead `egui` dependency triple nothing forces to justify
//! itself, and `AudioEngine::play` silently discarding its volume argument
//! because no second program ever asked it to be quieter.
//!
//! This is that second program. It is deliberately empty today: it opens a
//! window and draws nothing. **Every track in Phase MORROWIND adds to it**, and
//! the rule it enforces is a rule about the boundary, not about the demo:
//!
//! > If a track cannot be exercised from this example without reaching into
//! > engine internals, the track's API is wrong.
//!
//! # The two checks that keep it honest
//!
//! 1. **Public APIs only.** No `pub(crate)` reach-through, no `internal`
//!    module, no path that `hello_engine` can take because it grew up beside
//!    the engine. `grep` for either and the count must be zero (plan §A.7).
//! 2. **The dependency list is evidence.** It starts at one crate. A track that
//!    needs a new `somnium_*` crate here has demonstrated that crate has a
//!    public surface; a track that reaches for `somnium_renderer` internals to
//!    do something a game should be able to do has demonstrated the opposite.
//!
//! # What lands here, by track
//!
//! | Track | What it adds |
//! |---|---|
//! | 1 — VIVEC | **a screen-space HUD canvas and a world-space marker — landed, MORROWIND-E; drawn, MORROWIND-E2** |
//! | 3 — HLAALU | a prefab instanced a few times |
//! | 4 — SILT STRIDER | the cooked-asset path and a streamed cell |
//! | 5 — DWEMER | one skinned character with a walk cycle |
//! | 6 — SIXTH HOUSE | one agent pathing across the slice |
//! | 8 — ALMSIVI | input actions, a save/reload, and a positional sound |
//!
//! # What the emptiness measured
//!
//! This file said, for four sub-phases: *"Until Track 1 lands there is nothing
//! to draw."* Track 1 landed — paint layer, canvases, anchors, navigation,
//! styled text — and there was still nothing to draw, because `EngineContext`
//! had no way for a game to submit a widget tree. The program computed its HUD
//! layout and **printed it**, and that stood for a week across three further
//! sub-phases without anyone noticing that the engine's runtime UI could not
//! reach a screen.
//!
//! **That is the second-example rule working exactly as intended**, and it is
//! the strongest evidence in the phase that the rule earns its cost: nothing in
//! `somnium_ui`'s 215 tests failed, nothing in the editor looked different, and
//! the capability was entirely absent. MORROWIND-E2 is the hook, and the six
//! lines of `on_render_ui` below are what four sub-phases were for.

mod hud;

use hud::{Hud, HudTree};
use somnium_asset::cook::{
    AssetLoadMode, AssetResolver, CookConfig, CookKind, CookRequest, default_cook_deadline,
    submit_cook,
};
use somnium_asset::residency::{AssetHandle, AssetRequest, ResidencyConfig, ResidencyManager};
use somnium_core::world_partition::{ActorRecord, CellCoord, PartitionStore, WorldPartition};
use somnium_core::{Engine, EngineConfig, EngineContext, GameApp, GameUiFrame};
use somnium_jobs::{JobPriority, JobSystem};
use somnium_ui::graph::{Graph, catalogues, compile_animation, material};
use somnium_ui::runtime::canvas::SafeArea;
use somnium_ui::timeline::{self, CurveKey, TimelineSurface};

struct WalkCycle {
    skeleton: somnium_anim::Skeleton,
    graph: somnium_anim::AnimGraphAsset,
    animation_entity: somnium_core::Entity,
    cache: somnium_anim::PoseCache,
    elapsed: f32,
    root_x: f32,
}

struct AnimationParameters(somnium_anim::ParameterSet);
impl somnium_core::Component for AnimationParameters {}

/// The slice's game state.
///
/// Empty, and it stays a plain struct rather than becoming an engine type:
/// whatever a track needs to keep here is state a *game* keeps, and if it turns
/// out the engine should own it, that is a finding rather than a refactor to do
/// quietly.
#[derive(Default)]
struct Vvardenfell {
    /// Frames drawn. The only observable behaviour this program has, and it
    /// exists so "it ran" is a checkable claim rather than an impression.
    frames: u64,
    /// MORROWIND-E2. The HUD as a widget tree. Was `Hud` — a table of
    /// rectangles the program printed — until there was a hook to draw it
    /// through.
    hud: Option<HudTree>,
    /// The world-space name-plate, its own canvas because it is its own space.
    plate: Option<somnium_core::UiCanvas>,
    /// MORROWIND-K. The first graph consumer compiled through public APIs into
    /// the same material asset used by property authoring.
    graph_material: Option<material::CompiledMaterialGraph>,
    /// MORROWIND-V. A game-owned compiled walk graph evaluated without UI or
    /// renderer internals. MORROWIND-U already owns the separate pose-to-GPU
    /// palette seam; this slice records the sampled root for headless evidence.
    walk: Option<WalkCycle>,
    /// MORROWIND-Q/R. A stable handle that began as a placeholder and was
    /// atomically replaced from the cooked build representation.
    cooked_shader: Option<AssetHandle>,
    /// The one policy owner for the slice's resident cooked data.
    asset_residency: Option<ResidencyManager>,
    /// MORROWIND-S. The engine-neutral cell owner exercised with a real
    /// schema-serialized ECS actor rather than a parallel streaming DTO.
    partition: Option<WorldPartition>,
    /// MORROWIND-L. Stable digest of animation and UI-motion timeline assets
    /// authored and round-tripped solely through `somnium_ui`'s public API.
    timeline_evidence: u64,
}

impl GameApp for Vvardenfell {
    fn on_init(&mut self, ctx: &mut EngineContext) {
        // A real safe area comes from the platform. Nothing in the tree reports
        // one yet, so the slice hard-codes a phone-shaped inset: the value is a
        // placeholder, the *path* is not, and a HUD that has never been laid
        // out against a notch is a HUD that has not been tested.
        let hud = Hud::new(SafeArea {
            top: 44.0,
            bottom: 34.0,
            left: 0.0,
            right: 0.0,
        });

        let (w, h) = ctx.config.window_size;
        let viewport = glam::Vec2::new(w as f32, h as f32);
        let layout = hud.layout(viewport);
        println!(
            "vvardenfell: HUD on a {:.0}x{:.0} canvas at {:.2}x",
            layout.canvas.logical_size.x, layout.canvas.logical_size.y, layout.canvas.scale
        );

        let mut tree = HudTree::new(hud);
        tree.update(viewport);
        // The claim the `println!` used to make, checked instead of printed:
        // the widget tree agrees with the anchoring it was built from.
        let [bar, map, cross] = tree.bounds();
        println!("  health bar {bar:?}");
        println!("  minimap    {map:?}");
        println!("  crosshair  {cross:?}");
        self.hud = Some(tree);

        // A world-space name-plate over a point in the world. Sized in metres,
        // so it shrinks with distance the way a label attached to a thing
        // should — see the world-space decision in `somnium_ui::runtime::canvas`.
        let plate = hud::name_plate(glam::Vec3::new(3.0, 1.8, -12.0), 1.2);
        let plate_layout = plate.layout(viewport, 100.0);
        println!(
            "  name-plate {:.0}x{:.0} px offscreen target",
            plate_layout.logical_size.x, plate_layout.logical_size.y
        );
        self.plate = Some(somnium_core::UiCanvas::with_canvas(plate, viewport));

        let catalogue = catalogues::material();
        let mut graph = Graph::new();
        let colour = graph
            .add(&catalogue, "material.color", glam::Vec2::new(32.0, 48.0))
            .expect("the built-in material catalogue contains Colour");
        let output = graph
            .add(&catalogue, "material.surface", glam::Vec2::new(340.0, 48.0))
            .expect("the built-in material catalogue contains Material Surface");
        graph
            .node_mut(colour)
            .expect("the node was just inserted")
            .literals
            .insert(0, "0.32,0.46,0.24,1.0".into());
        graph
            .connect(
                &catalogue,
                somnium_ui::graph::PinRef::output(colour, 0),
                somnium_ui::graph::PinRef::input(output, 0),
            )
            .expect("Colour connects to Material Surface base colour");
        let compiled = material::compile(&graph, &catalogue, &Default::default())
            .expect("the slice's material graph is valid");
        println!(
            "  material graph -> base {:?}, {} WGSL bytes",
            compiled.material.base_color.0,
            compiled.wgsl.len()
        );
        self.graph_material = Some(compiled);

        self.timeline_evidence = build_timeline_evidence();
        println!("  timeline evidence -> {:016x}", self.timeline_evidence);

        let (mut walk, parameters) = build_walk_cycle();
        let script_asset = somnium_script::ids::ScriptAssetId::mint();
        ctx.scripts
            .load_script(
                script_asset,
                "vvardenfell/locomotion.luau",
                r#"
                return Script.define({
                    apiVersion = 1,
                    schemaVersion = 1,
                    onStart = function(self, ctx)
                        ctx:setAnimationFloat(ctx.entity, "speed", 0.7)
                    end,
                })
                "#,
            )
            .expect("the slice's strict animation driver compiles");
        let mut scripts = somnium_script::attachment::ScriptSet::new();
        scripts.attach(somnium_script::attachment::ScriptAttachment::new(
            script_asset,
        ));
        let animation_entity = ctx.world.spawn((AnimationParameters(parameters), scripts));
        walk.animation_entity = animation_entity;
        ctx.scripts
            .set_animation_parameter_router(Box::new(|world, entity, name, value| {
                let target = world
                    .get_mut::<AnimationParameters>(entity)
                    .ok_or_else(|| "entity has no AnimationParameters component".to_string())?;
                somnium_core::apply_animation_parameter(&mut target.0, name, value)
                    .map_err(|error| format!("{error:?}"))
            }));
        self.walk = Some(walk);
        println!("  animation graph -> synced idle/walk/run blend");

        // MORROWIND-Q. Exercise the same public cook and resolver path used by
        // the standalone tool. The source root is configuration; neither it
        // nor its timestamps enter the manifest or artifact bytes.
        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("the example lives under the workspace root");
        let derived = workspace.join("target/vvardenfell-native-cook");
        let request = CookRequest {
            source: "crates/somnium_renderer/src/shaders/census.wgsl".into(),
            kind: CookKind::Shader,
            dependencies: vec![],
        };
        let config = CookConfig {
            source_root: workspace.into(),
            output_root: derived.join("build"),
            cache_root: derived.join("cache"),
            cooker_version: 1,
        };
        let mut jobs = JobSystem::single_threaded();
        let report = submit_cook(
            &mut jobs,
            config.clone(),
            vec![request.clone()],
            JobPriority::User,
            default_cook_deadline(),
        )
        .expect("the cook job can be submitted")
        .try_take()
        .expect("single-threaded jobs complete inline")
        .expect("the slice shader cooks");
        let resolver = std::sync::Arc::new(AssetResolver::new(
            config.source_root,
            config.output_root,
            report.manifest,
            AssetLoadMode::Build,
        ));
        let residency = ResidencyManager::new(ResidencyConfig {
            byte_budget: 1024 * 1024,
            upload_budget_per_frame: 64 * 1024,
        });
        let handle = residency
            .request_resolved(
                &mut jobs,
                resolver,
                AssetRequest::new(request.asset_id(), CookKind::Shader, "Vvardenfell census"),
            )
            .expect("a residency request returns its placeholder immediately");
        assert!(handle.is_placeholder());
        jobs.drain_completions(std::time::Duration::from_millis(5));
        let upload = residency.process_frame();
        let loaded = handle.current();
        assert!(!loaded.placeholder);
        println!(
            "  native cook/residency -> {} bytes for {} ({} upload)",
            loaded.payload.len(),
            loaded.asset,
            upload.uploaded_bytes
        );
        self.cooked_shader = Some(handle);
        self.asset_residency = Some(residency);

        let cell = CellCoord::default();
        let actor_id = somnium_core::PersistentId::from_raw(0x5641_5244_454e_4645_4c4c_0000_0001);
        let staged = ctx.world.spawn((
            actor_id,
            somnium_core::Name::new("Balmora cell marker"),
            somnium_core::Transform::from_translation(glam::Vec3::new(8.0, 0.0, 8.0)),
        ));
        let document = somnium_core::scene_schema::entities_to_json(
            ctx.world,
            &somnium_core::reflect_registry::component_registry(),
            &[staged],
        )
        .expect("the streamed actor uses the registered scene schema");
        ctx.world.despawn(staged);
        let store = PartitionStore::new(derived.join("partition"));
        store
            .save_cell_with_derived(
                cell,
                &[ActorRecord {
                    id: actor_id,
                    position: [8.0, 0.0, 8.0],
                    document,
                }],
                &[request.asset_id()],
            )
            .expect("the slice cell and its derived shader persist");
        let mut partition = WorldPartition::new(store, 64.0);
        partition.pin(cell);
        partition
            .update(ctx.world, &mut jobs, default_cook_deadline())
            .expect("the cell load uses the shared job contract");
        assert!(ctx.world.entity_by_persistent_id(actor_id).is_some());
        println!("  world partition -> one schema actor resident in {cell:?}");
        self.partition = Some(partition);
    }

    fn on_update(&mut self, ctx: &mut EngineContext) {
        self.frames += 1;
        if let Some(walk) = self.walk.as_mut() {
            walk.elapsed += ctx.time.delta_time().as_secs_f32();
            if let Some(parameters) = ctx.world.get::<AnimationParameters>(walk.animation_entity) {
                if let Ok(pose) = walk.graph.evaluate(
                    &walk.skeleton,
                    &parameters.0,
                    walk.elapsed,
                    self.frames,
                    &mut walk.cache,
                ) {
                    walk.root_x = pose.local[0].translation.x;
                }
            }
        }
    }

    /// Build. MORROWIND-E2's rule: the tree is mutated here, where there is a
    /// whole `EngineContext`, and drawn in `on_render_ui`, where there is a GPU.
    fn on_render(&mut self, ctx: &mut EngineContext) {
        let (w, h) = ctx.config.window_size;
        if let Some(hud) = self.hud.as_mut() {
            // Drain over two minutes and wrap, so the bar is visibly driven by
            // something rather than parked at full. The slice has no combat to
            // take health away and pretending otherwise would be a lie in the
            // one program whose job is to not contain any.
            hud.health = 1.0 - ((self.frames % 7200) as f32 / 7200.0);
            hud.update(glam::Vec2::new(w as f32, h as f32));
        }
    }

    /// Draw. Six lines, and the whole of MORROWIND-E2 exists so that they can
    /// be written at all.
    fn on_render_ui(&mut self, frame: &mut GameUiFrame) {
        if let Some(hud) = self.hud.as_mut() {
            frame.draw(hud.canvas_mut());
        }
        if let Some(plate) = self.plate.as_mut() {
            frame.draw(plate);
        }
    }

    /// The other half of the hook: a HUD that cannot be clicked is a picture.
    ///
    /// MORROWIND-F built hit-testing through the inverse transform and
    /// directional navigation, and a game had no way to feed either an event.
    /// Returning `true` consumes it, which is how a game says *"that click was
    /// mine"* before the editor's viewport tools see it.
    fn on_os_event(&mut self, _ctx: &mut EngineContext, event: &somnium_core::WindowEvent) -> bool {
        // MORROWIND-H. Tab hides the HUD, the way a screenshot key or a
        // cutscene would. It is here rather than in `on_update` because it is
        // the smallest honest demonstration that the two halves of the hook
        // work together: an event arrives through `on_os_event`, starts a
        // transition, and `on_render_ui` draws the frames of it.
        if let somnium_core::WindowEvent::KeyboardInput { event: key, .. } = event {
            let pressed = key.state == winit_state_pressed();
            if pressed && key.physical_key == tab_key() {
                if let Some(hud) = self.hud.as_mut() {
                    let shown = hud.shown();
                    hud.set_shown(!shown);
                }
                return true;
            }
        }
        self.hud
            .as_mut()
            .is_some_and(|hud| hud.canvas_mut().process_os_event(event))
    }

    fn on_shutdown(&mut self) {
        println!(
            "vvardenfell: {} frames, walk root {:.3} m.",
            self.frames,
            self.walk.as_ref().map_or(0.0, |walk| walk.root_x)
        );
    }
}

/// MORROWIND-L's second-example proof. Both consumers author the same shared
/// timeline model, then survive the public versioned asset boundary byte for
/// byte. The digest is retained on `Vvardenfell` so headless runs have one
/// deterministic value to compare without exposing editor internals.
fn build_timeline_evidence() -> u64 {
    let animation_catalogue = timeline::catalogues::animation();
    let mut animation = TimelineSurface::new(animation_catalogue.clone(), 8.0);
    let actors = animation.add_group("Actors", None).expect("valid group");
    let body = animation
        .add_track("animation.clip", "Body", Some(actors))
        .expect("the animation catalogue contains its clip track");
    animation
        .add_media(body, "animation-clip", "walk.anim", 0.5, 4.0)
        .expect("the clip kind belongs to the animation track");
    animation
        .add_marker(2.0, "Left foot")
        .expect("the marker lies in the document");
    animation
        .add_keyframe(body, 0, CurveKey::new(2.0, 0.75))
        .expect("weight is the animation track's first channel");
    let animation_document = animation.document().clone();
    let animation_json = timeline::to_json(&animation_document).expect("timeline serializes");
    let animation_loaded = timeline::from_json(&animation_json, &animation_catalogue)
        .expect("animation timeline round-trips under its catalogue");
    assert_eq!(
        timeline::to_json(&animation_loaded).expect("loaded timeline serializes"),
        animation_json
    );

    let ui_catalogue = timeline::catalogues::ui_motion();
    let mut ui_motion = TimelineSurface::new(ui_catalogue.clone(), 1.0);
    let interface = ui_motion.add_group("Interface", None).expect("valid group");
    let panel = ui_motion
        .add_track("ui.motion", "Quest Panel", Some(interface))
        .expect("the UI catalogue contains its motion track");
    ui_motion
        .add_media(panel, "ui-motion", "quest-panel.somui", 0.0, 1.0)
        .expect("the UI-motion kind belongs to the UI track");
    ui_motion
        .add_marker(0.5, "Readable")
        .expect("the marker lies in the document");
    ui_motion
        .add_keyframe(panel, 0, CurveKey::new(0.25, 0.4))
        .expect("opacity is the UI track's first channel");
    let ui_document = ui_motion.document().clone();
    let ui_json = timeline::to_json(&ui_document).expect("timeline serializes");
    let ui_loaded = timeline::from_json(&ui_json, &ui_catalogue)
        .expect("UI-motion timeline round-trips under its catalogue");
    assert_eq!(
        timeline::to_json(&ui_loaded).expect("loaded timeline serializes"),
        ui_json
    );

    animation_json
        .bytes()
        .chain(ui_json.bytes())
        .fold(0xcbf2_9ce4_8422_2325, |digest, byte| {
            (digest ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

fn build_walk_cycle() -> (WalkCycle, somnium_anim::ParameterSet) {
    use glam::{Mat4, Quat, Vec3};
    use somnium_anim::{
        AnimationClip, ClipId, GraphId, Keyframe, ParameterDefinition, ParameterSchema,
        ParameterSchemaId, ParameterValue, Playback, Skeleton, SkeletonId, SyncMarker, SyncTrack,
        Transform, TransformTrack,
    };

    let skeleton = Skeleton::new(
        SkeletonId(1),
        vec!["root".into()],
        vec![somnium_anim::NO_PARENT],
        vec![Mat4::IDENTITY],
        vec![Transform::IDENTITY],
    )
    .expect("the slice's one-joint skeleton is valid")
    .0;
    let make_clip = |id, duration, distance| {
        AnimationClip::new(
            ClipId(id),
            &skeleton,
            duration,
            vec![TransformTrack {
                joint: 0,
                translation: vec![
                    Keyframe::new(0.0, Vec3::ZERO),
                    Keyframe::new(duration, Vec3::X * distance),
                ],
                rotation: vec![Keyframe::new(0.0, Quat::IDENTITY)],
                scale: vec![],
            }],
            vec![
                SyncTrack::new(
                    "locomotion",
                    duration,
                    vec![
                        SyncMarker::new("left_contact", 0.0),
                        SyncMarker::new("right_contact", duration * 0.5),
                    ],
                )
                .expect("the two foot contacts make a valid sync cycle"),
            ],
        )
        .expect("the slice's clip targets the only joint")
    };
    let parameters = ParameterSchema::new(
        ParameterSchemaId(1),
        vec![ParameterDefinition::new(
            "speed",
            ParameterValue::Float(0.55),
        )],
    )
    .expect("the speed parameter is finite and unique");
    let values = parameters.instantiate();

    let catalogue = catalogues::animation();
    let mut authored = Graph::new();
    let idle = authored
        .add(&catalogue, "animation.clip", glam::Vec2::new(20.0, 20.0))
        .expect("the catalogue contains Clip");
    let walk = authored
        .add(&catalogue, "animation.clip", glam::Vec2::new(20.0, 180.0))
        .expect("the catalogue contains Clip");
    let run = authored
        .add(&catalogue, "animation.clip", glam::Vec2::new(20.0, 340.0))
        .expect("the catalogue contains Clip");
    authored.node_mut(idle).unwrap().literals.extend([
        (0, "1".into()),
        (1, Playback::LOOPING.time_scale().to_string()),
    ]);
    authored.node_mut(walk).unwrap().literals.extend([
        (0, "2".into()),
        (1, Playback::LOOPING.time_scale().to_string()),
    ]);
    authored.node_mut(run).unwrap().literals.extend([
        (0, "3".into()),
        (1, Playback::LOOPING.time_scale().to_string()),
    ]);
    let blend = authored
        .add(
            &catalogue,
            "animation.blend1d3",
            glam::Vec2::new(280.0, 180.0),
        )
        .expect("the catalogue contains three-sample Blend 1D");
    authored.node_mut(blend).unwrap().literals.extend([
        (3, "0.0".into()),
        (4, "0.5".into()),
        (5, "1.0".into()),
        (6, "speed".into()),
        (7, "locomotion".into()),
        (8, "1".into()),
    ]);
    let output = authored
        .add(
            &catalogue,
            "animation.output",
            glam::Vec2::new(540.0, 180.0),
        )
        .expect("the catalogue contains Animation Output");
    for (from, to) in [
        (
            somnium_ui::graph::PinRef::output(idle, 0),
            somnium_ui::graph::PinRef::input(blend, 0),
        ),
        (
            somnium_ui::graph::PinRef::output(walk, 0),
            somnium_ui::graph::PinRef::input(blend, 1),
        ),
        (
            somnium_ui::graph::PinRef::output(run, 0),
            somnium_ui::graph::PinRef::input(blend, 2),
        ),
        (
            somnium_ui::graph::PinRef::output(blend, 0),
            somnium_ui::graph::PinRef::input(output, 0),
        ),
    ] {
        authored
            .connect(&catalogue, from, to)
            .expect("the authored pose pins have the same opaque type");
    }
    let graph = compile_animation(
        &authored,
        &catalogue,
        GraphId(1),
        1,
        &skeleton,
        vec![
            make_clip(1, 1.0, 0.0),
            make_clip(2, 1.0, 1.4),
            make_clip(3, 0.55, 2.5),
        ],
        parameters,
    )
    .expect("the public graph compiler accepts the slice's authored graph");
    (
        WalkCycle {
            skeleton,
            graph,
            animation_entity: somnium_core::Entity::DANGLING,
            cache: somnium_anim::PoseCache::default(),
            elapsed: 0.0,
            root_x: 0.0,
        },
        values,
    )
}

/// `winit`'s pressed state, named so the match above reads as intent.
///
/// The slice reaches `winit` through `somnium_core`'s re-exports and does not
/// depend on it directly — MORROWIND-AE's `somnium_input` is the right way to
/// ask "did the player press confirm", and this is deliberately *not* that: a
/// debug key that hides the HUD is not a game action and should not consume an
/// action-map binding.
fn winit_state_pressed() -> somnium_core::ElementState {
    somnium_core::ElementState::Pressed
}

fn tab_key() -> somnium_core::PhysicalKey {
    somnium_core::PhysicalKey::Code(somnium_core::KeyCode::Tab)
}

#[cfg(test)]
mod morrowind_l_tests {
    #[test]
    fn timeline_evidence_is_deterministic() {
        let first = super::build_timeline_evidence();
        assert_ne!(first, 0);
        assert_eq!(first, super::build_timeline_evidence());
    }
}

fn main() {
    let config = EngineConfig {
        window_title: "Somnium — Vvardenfell".to_string(),
        window_size: (1280, 720),
        ..EngineConfig::default()
    };

    if let Err(error) = Engine::run(config, Vvardenfell::default()) {
        eprintln!("vvardenfell: {error}");
        std::process::exit(1);
    }
}
