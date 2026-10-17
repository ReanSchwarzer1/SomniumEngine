//! Rooms and walls for positioned sounds (The Somnus Fracture's spatial audio, 15 October 2026).
//!
//! Positioned sounds play on a world track whose send into a room reverb follows how enclosed the listener is; a
//! one-shot that starts behind a wall plays on a muffled copy of that track (a low-pass), so it is heard through the
//! wall instead of merely quieter. Music and interface sounds stay dry on the main track: a score in a reverb is a
//! different score. Built on Kira's own tracks, sends, reverb and filter; nothing is mixed here by hand.

use std::time::Duration;

use kira::effect::filter::{FilterBuilder, FilterMode};
use kira::effect::reverb::{ReverbBuilder, ReverbHandle};
use kira::manager::{backend::Backend, AudioManager};
use kira::track::{TrackBuilder, TrackHandle, TrackRoutes};
use kira::tween::Tween;

/// Below this occlusion gain a one-shot starts muffled.
pub const MUFFLE_BELOW: f32 = 0.6;
/// The muffled track's low-pass: a voice through a door or a wall keeps its body and loses its edges.
const MUFFLED_CUTOFF: f64 = 900.0;

/// The send into the room reverb and how long the room rings, for how enclosed the listener is (0 open air, 1 a small
/// hard room). Open air keeps a little: a forest and a street of houses are not anechoic.
#[must_use]
pub fn room(enclosure: f32) -> (f64, f64) {
    let e = f64::from(enclosure.clamp(0.0, 1.0));
    (0.06 + 0.34 * e, 0.55 + 0.30 * e)
}

/// The tracks positioned sounds play on.
pub struct Acoustics {
    world: TrackHandle,
    muffled: TrackHandle,
    room: TrackHandle,
    reverb: ReverbHandle,
    enclosure: f32,
}

impl Acoustics {
    /// The room reverb, then the world and muffled tracks that send into it (and play dry to the main track).
    pub fn new<B: Backend>(manager: &mut AudioManager<B>) -> Result<Self, String> {
        let (send, feedback) = room(0.0);
        let mut room_track = TrackBuilder::new();
        let reverb = room_track.add_effect(ReverbBuilder::new().feedback(feedback).damping(0.45).mix(1.0));
        let room = manager.add_sub_track(room_track).map_err(|e| e.to_string())?;
        let world = manager
            .add_sub_track(TrackBuilder::new().routes(TrackRoutes::new().with_route(&room, send)))
            .map_err(|e| e.to_string())?;
        let mut muffled_track = TrackBuilder::new().routes(TrackRoutes::new().with_route(&room, send));
        muffled_track.add_effect(FilterBuilder::new().mode(FilterMode::LowPass).cutoff(MUFFLED_CUTOFF));
        let muffled = manager.add_sub_track(muffled_track).map_err(|e| e.to_string())?;
        Ok(Self { world, muffled, room, reverb, enclosure: 0.0 })
    }

    /// How enclosed the listener is now, 0 to 1. Eased over 0.4 s so walking through a door is not a cut.
    pub fn set_enclosure(&mut self, enclosure: f32) {
        let enclosure = if enclosure.is_finite() { enclosure.clamp(0.0, 1.0) } else { 0.0 };
        if (enclosure - self.enclosure).abs() < 0.01 {
            return;
        }
        self.enclosure = enclosure;
        let (send, feedback) = room(enclosure);
        let tween = Tween { duration: Duration::from_millis(400), ..Default::default() };
        let _ = self.world.set_route(&self.room, send, tween);
        let _ = self.muffled.set_route(&self.room, send, tween);
        self.reverb.set_feedback(feedback, tween);
    }

    #[must_use]
    pub fn enclosure(&self) -> f32 {
        self.enclosure
    }

    /// The track a positioned sound with this occlusion gain starts on.
    #[must_use]
    pub fn track(&self, occlusion: f32) -> &TrackHandle {
        if occlusion < MUFFLE_BELOW { &self.muffled } else { &self.world }
    }

    /// Whether a sound with this occlusion gain starts muffled.
    #[must_use]
    pub fn muffles(occlusion: f32) -> bool {
        occlusion < MUFFLE_BELOW
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kira::manager::{backend::mock::MockBackend, AudioManagerSettings};

    #[test]
    fn a_room_sends_more_and_rings_longer_than_open_air_and_never_rings_forever() {
        let (open_send, open_ring) = room(0.0);
        let (shut_send, shut_ring) = room(1.0);
        assert!(open_send > 0.0 && shut_send > open_send && shut_ring > open_ring);
        assert!(shut_ring < 0.95, "a feedback of 1 rings forever");
        assert_eq!(room(-3.0), room(0.0));
        assert_eq!(room(7.0), room(1.0));
    }

    #[test]
    fn the_tracks_build_on_a_real_manager_and_a_wall_muffles() {
        let mut manager = AudioManager::<MockBackend>::new(AudioManagerSettings::default()).unwrap();
        let mut acoustics = Acoustics::new(&mut manager).unwrap();
        acoustics.set_enclosure(0.8);
        assert!((acoustics.enclosure() - 0.8).abs() < 1e-6);
        acoustics.set_enclosure(f32::NAN);
        assert_eq!(acoustics.enclosure(), 0.0);
        assert!(Acoustics::muffles(0.3) && !Acoustics::muffles(1.0));
        assert_ne!(acoustics.track(0.3).id(), acoustics.track(1.0).id());
    }
}
