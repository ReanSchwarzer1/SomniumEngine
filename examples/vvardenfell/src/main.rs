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
use somnium_core::{Engine, EngineConfig, EngineContext, GameApp, GameUiFrame};
use somnium_ui::graph::{Graph, catalogues, material};
use somnium_ui::runtime::canvas::SafeArea;

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
    }

    fn on_update(&mut self, _ctx: &mut EngineContext) {
        self.frames += 1;
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
        println!("vvardenfell: {} frames.", self.frames);
    }
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
