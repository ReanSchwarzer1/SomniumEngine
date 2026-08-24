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
//! | 1 — VIVEC | a screen-space HUD canvas and a world-space marker |
//! | 3 — HLAALU | a prefab instanced a few times |
//! | 4 — SILT STRIDER | the cooked-asset path and a streamed cell |
//! | 5 — DWEMER | one skinned character with a walk cycle |
//! | 6 — SIXTH HOUSE | one agent pathing across the slice |
//! | 8 — ALMSIVI | input actions, a save/reload, and a positional sound |
//!
//! Until Track 1 lands there is nothing to draw, and drawing something now with
//! `hello_engine`'s scaffolding would defeat the purpose — the emptiness is the
//! measurement.

use somnium_core::{Engine, EngineConfig, EngineContext, GameApp};

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
}

impl GameApp for Vvardenfell {
    fn on_init(&mut self, _ctx: &mut EngineContext) {
        println!("vvardenfell: the slice is open and empty (MORROWIND-A).");
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
