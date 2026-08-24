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
//! | 1 — VIVEC | **a screen-space HUD canvas and a world-space marker — landed, MORROWIND-E** |
//! | 3 — HLAALU | a prefab instanced a few times |
//! | 4 — SILT STRIDER | the cooked-asset path and a streamed cell |
//! | 5 — DWEMER | one skinned character with a walk cycle |
//! | 6 — SIXTH HOUSE | one agent pathing across the slice |
//! | 8 — ALMSIVI | input actions, a save/reload, and a positional sound |
//!
//! Until Track 1 lands there is nothing to draw, and drawing something now with
//! `hello_engine`'s scaffolding would defeat the purpose — the emptiness is the
//! measurement.

mod hud;

use hud::Hud;
use somnium_core::{Engine, EngineConfig, EngineContext, GameApp};
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
    /// MORROWIND-E. The HUD's anchoring, resolved per frame against whatever
    /// the window currently is.
    hud: Hud,
}

impl GameApp for Vvardenfell {
    fn on_init(&mut self, ctx: &mut EngineContext) {
        // A real safe area comes from the platform. Nothing in the tree reports
        // one yet, so the slice hard-codes a phone-shaped inset: the value is a
        // placeholder, the *path* is not, and a HUD that has never been laid
        // out against a notch is a HUD that has not been tested.
        self.hud = Hud::new(SafeArea {
            top: 44.0,
            bottom: 34.0,
            left: 0.0,
            right: 0.0,
        });

        let (w, h) = ctx.config.window_size;
        let layout = self.hud.layout(glam::Vec2::new(w as f32, h as f32));
        println!(
            "vvardenfell: HUD on a {:.0}x{:.0} canvas at {:.2}x",
            layout.canvas.logical_size.x, layout.canvas.logical_size.y, layout.canvas.scale
        );
        println!("  health bar {:?}", layout.health_bar);
        println!("  minimap    {:?}", layout.minimap);
        println!("  crosshair  {:?}", layout.crosshair);

        // A world-space name-plate over a point in the world. Sized in metres,
        // so it shrinks with distance the way a label attached to a thing
        // should — see the world-space decision in `somnium_ui::runtime::canvas`.
        let plate = hud::name_plate(glam::Vec3::new(3.0, 1.8, -12.0), 1.2);
        let plate_layout = plate.layout(glam::Vec2::new(w as f32, h as f32), 100.0);
        println!(
            "  name-plate {:.0}x{:.0} px offscreen target",
            plate_layout.logical_size.x, plate_layout.logical_size.y
        );
    }

    fn on_update(&mut self, _ctx: &mut EngineContext) {
        self.frames += 1;
    }

    fn on_shutdown(&mut self) {
        println!("vvardenfell: {} frames.", self.frames);
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
