//! The listener, attenuation and spatialisation (MORROWIND-AG).
//!
//! **This file used to be one line: `// Listener stub`.**
//!
//! A listener is where the player's ears are. Without one there is no such
//! thing as a sound being *over there*, which is most of what audio does in a
//! 3D game: a footstep behind you, a river to the left, a door closing in the
//! next room.
//!
//! # Attenuation curves are computed here, and the reason is CONTROL-K
//!
//! §8 item 2 says attenuation curves come from CONTROL-K's curve editor. That
//! editor produces a curve an author drags; this module evaluates one. Keeping
//! the evaluation here — as a small enum with an authored-curve variant — means
//! the default curves are testable arithmetic and the authored case is one more
//! variant rather than a different code path.

use glam::{Quat, Vec3};

/// Where the player's ears are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    pub position: Vec3,
    /// Orientation. Forward is -Z and up is +Y, matching the renderer's camera
    /// convention — the listener normally *is* the camera, and two conventions
    /// would put every sound on the wrong side exactly half the time.
    pub orientation: Quat,
    /// Metres per second, for Doppler.
    pub velocity: Vec3,
}

impl Default for Listener {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            velocity: Vec3::ZERO,
        }
    }
}

impl Listener {
    /// A listener at `position` looking at `target`.
    #[must_use]
    pub fn looking_at(position: Vec3, target: Vec3, up: Vec3) -> Self {
        let forward = (target - position).normalize_or_zero();
        let orientation = if forward.length_squared() < 1e-8 {
            // A listener looking at its own position has no orientation. The
            // identity is the only answer that does not produce a NaN basis,
            // and a NaN basis puts every sound at pan NaN — which most mixers
            // render as silence, so it would look like the audio died.
            Quat::IDENTITY
        } else {
            // The listener's own basis: -Z forward, +Y up, +X right, matching
            // the renderer's camera. Built directly rather than via a look-at
            // matrix so the handedness is visible here instead of depending on
            // whether a helper returns a view matrix or its inverse.
            let right = forward.cross(up).normalize_or_zero();
            if right.length_squared() < 1e-8 {
                // Looking straight along `up`: the cross product degenerates.
                Quat::IDENTITY
            } else {
                let true_up = right.cross(forward);
                Quat::from_mat3(&glam::Mat3::from_cols(right, true_up, -forward))
            }
        };
        Self {
            position,
            orientation,
            velocity: Vec3::ZERO,
        }
    }

    /// A direction in world space, expressed in the listener's frame.
    ///
    /// `+x` is to the listener's right, `-z` in front. This is what a panner
    /// needs and it is the one calculation that decides whether a sound comes
    /// out of the correct ear.
    #[must_use]
    pub fn to_local(&self, world: Vec3) -> Vec3 {
        self.orientation.inverse() * (world - self.position)
    }
}

/// How loudness falls off with distance.
#[derive(Clone, Debug, PartialEq)]
pub enum Attenuation {
    /// Full volume inside `min`, silent past `max`, straight line between.
    ///
    /// Cheap and wrong-sounding: real sound falls off fast at first and slowly
    /// after, so a linear curve makes a source seem to leap in volume as you
    /// approach. Kept because it is what a UI or a 2D game wants, and because
    /// it is the curve a designer reaches for when they want predictability.
    Linear { min: f32, max: f32 },
    /// Inverse-square, the physical law, clamped to full inside `min`.
    ///
    /// The default, because it is what ears expect.
    InverseSquare { min: f32, max: f32 },
    /// An authored curve: sorted `(distance, gain)` points, linearly
    /// interpolated. CONTROL-K's editor produces exactly this.
    Curve(Vec<(f32, f32)>),
    /// No falloff. Music, narration, a UI click.
    None,
}

impl Default for Attenuation {
    fn default() -> Self {
        Self::InverseSquare {
            min: 1.0,
            max: 100.0,
        }
    }
}

impl Attenuation {
    /// Gain at `distance` metres, in `0..=1`.
    #[must_use]
    pub fn gain(&self, distance: f32) -> f32 {
        let distance = distance.max(0.0);
        match self {
            Self::None => 1.0,
            Self::Linear { min, max } => {
                let (min, max) = ordered(*min, *max);
                if distance <= min {
                    1.0
                } else if distance >= max {
                    0.0
                } else {
                    1.0 - (distance - min) / (max - min)
                }
            }
            Self::InverseSquare { min, max } => {
                let (min, max) = ordered(*min, *max);
                if distance <= min {
                    return 1.0;
                }
                if distance >= max {
                    return 0.0;
                }
                // `min / d` is the physical law. It never reaches zero, so it
                // is faded to nothing over the last stretch before `max` —
                // without that, a distant source cuts out abruptly at `max`
                // instead of receding, and the cut is audible.
                let physical = min / distance;
                let fade = 1.0 - (distance - min) / (max - min);
                (physical * fade).clamp(0.0, 1.0)
            }
            Self::Curve(points) => evaluate_curve(points, distance),
        }
    }
}

/// `min` and `max` in the right order, `max` strictly greater.
///
/// An authored `min` above `max` is a data-entry mistake that would otherwise
/// divide by a negative range and produce gains above one — a sound that gets
/// *louder* with distance, which is a memorable bug to chase.
fn ordered(min: f32, max: f32) -> (f32, f32) {
    let lo = min.min(max).max(0.0);
    let hi = max.max(min).max(lo + 1e-4);
    (lo, hi)
}

fn evaluate_curve(points: &[(f32, f32)], distance: f32) -> f32 {
    if points.is_empty() {
        return 1.0;
    }
    if distance <= points[0].0 {
        return points[0].1.clamp(0.0, 1.0);
    }
    if let Some(last) = points.last()
        && distance >= last.0
    {
        return last.1.clamp(0.0, 1.0);
    }
    for pair in points.windows(2) {
        let (d0, g0) = pair[0];
        let (d1, g1) = pair[1];
        if distance >= d0 && distance <= d1 {
            let span = (d1 - d0).max(1e-6);
            let t = (distance - d0) / span;
            return (g0 + (g1 - g0) * t).clamp(0.0, 1.0);
        }
    }
    points.last().map_or(1.0, |p| p.1.clamp(0.0, 1.0))
}

/// A directional cone, for a sound that points somewhere.
///
/// A loudspeaker, a torch's crackle, an NPC talking to someone else.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cone {
    /// Full volume within this half-angle, radians.
    pub inner: f32,
    /// Silent past this half-angle, radians.
    pub outer: f32,
    /// Gain outside `outer`. Rarely zero: a speaker behind you is quieter, not
    /// absent, and zero makes a listener walking past it pop.
    pub outer_gain: f32,
}

impl Default for Cone {
    fn default() -> Self {
        Self {
            inner: std::f32::consts::PI,
            outer: std::f32::consts::PI,
            outer_gain: 1.0,
        }
    }
}

impl Cone {
    /// Gain for a listener at `angle` radians off the cone's axis.
    #[must_use]
    pub fn gain(&self, angle: f32) -> f32 {
        let angle = angle.abs();
        let (inner, outer) = ordered(self.inner, self.outer);
        if angle <= inner {
            1.0
        } else if angle >= outer {
            self.outer_gain.clamp(0.0, 1.0)
        } else {
            let t = (angle - inner) / (outer - inner);
            (1.0 - t * (1.0 - self.outer_gain)).clamp(0.0, 1.0)
        }
    }
}

/// A sound source in the world.
#[derive(Clone, Debug, PartialEq)]
pub struct Emitter {
    pub position: Vec3,
    pub velocity: Vec3,
    pub attenuation: Attenuation,
    /// Direction the cone points. Ignored when `cone` is omnidirectional.
    pub direction: Vec3,
    pub cone: Option<Cone>,
    /// Extra attenuation from geometry between source and listener, `0..=1`.
    ///
    /// Set by whoever queries the physics world. Kept as a plain factor rather
    /// than computed here, because occlusion is a raycast and this crate must
    /// not depend on the physics one — a sound system that cannot be tested
    /// without a physics world is a sound system nobody tests.
    pub occlusion: f32,
}

impl Default for Emitter {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            velocity: Vec3::ZERO,
            attenuation: Attenuation::default(),
            direction: Vec3::NEG_Z,
            cone: None,
            occlusion: 1.0,
        }
    }
}

/// What a spatial evaluation produced.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spatial {
    /// Combined distance, cone and occlusion gain, `0..=1`.
    pub gain: f32,
    /// -1 fully left, 0 centre, +1 fully right.
    pub pan: f32,
    /// Playback rate multiplier from Doppler. 1.0 is no shift.
    pub doppler: f32,
    /// Distance in metres, for a caller that wants to cull.
    pub distance: f32,
}

/// Speed of sound in air, metres per second.
pub const SPEED_OF_SOUND: f32 = 343.0;

/// The slowest a Doppler shift may drive playback.
///
/// Two octaves either side. Past that the resampling artefacts are worse than
/// the effect, and nothing in a game moves fast enough to want more.
pub const MIN_DOPPLER: f32 = 0.25;
/// The fastest a Doppler shift may drive playback.
pub const MAX_DOPPLER: f32 = 4.0;

/// Evaluate an emitter against a listener.
#[must_use]
pub fn evaluate(listener: &Listener, emitter: &Emitter, doppler_scale: f32) -> Spatial {
    let to_emitter = emitter.position - listener.position;
    let distance = to_emitter.length();

    let mut gain = emitter.attenuation.gain(distance);
    if let Some(cone) = emitter.cone {
        let axis = emitter.direction.normalize_or_zero();
        if axis.length_squared() > 1e-8 && distance > 1e-4 {
            // The angle between where the cone points and where the listener is.
            let to_listener = (-to_emitter) / distance;
            let angle = axis.dot(to_listener).clamp(-1.0, 1.0).acos();
            gain *= cone.gain(angle);
        }
    }
    gain *= emitter.occlusion.clamp(0.0, 1.0);

    // Pan from the emitter's position in the listener's own frame.
    let local = listener.to_local(emitter.position);
    let pan = if local.length_squared() < 1e-8 {
        // A source exactly at the listener has no direction. Centre is the only
        // answer that does not jump as they walk through it.
        0.0
    } else {
        (local.x / local.length()).clamp(-1.0, 1.0)
    };

    Spatial {
        gain: gain.clamp(0.0, 1.0),
        pan,
        doppler: doppler(listener, emitter, distance, doppler_scale),
        distance,
    }
}

fn doppler(listener: &Listener, emitter: &Emitter, distance: f32, scale: f32) -> f32 {
    if scale <= 0.0 || distance < 1e-4 {
        return 1.0;
    }
    // `direction` points from the listener towards the emitter.
    let direction = (emitter.position - listener.position) / distance;

    // `f' = f (c + v_r) / (c - v_s)`, where **both** speeds are measured
    // *towards the other party*:
    //
    // - `v_r` is the listener moving towards the source, which is along
    //   `direction`.
    // - `v_s` is the source moving towards the listener, which is along
    //   **minus** `direction`.
    //
    // The first version of this function used `direction` for both and made
    // every approaching sound drop in pitch — the exact opposite of the effect,
    // and the sort of sign error that sounds "a bit off" rather than obviously
    // broken. `doppler_shifts_the_right_way` is what caught it.
    let listener_speed = listener.velocity.dot(direction) * scale;
    let emitter_speed = -emitter.velocity.dot(direction) * scale;

    let denominator = SPEED_OF_SOUND - emitter_speed;
    if denominator <= 1.0 {
        // At or past the speed of sound the formula has a pole and then goes
        // negative; physically there is a shock front and no Doppler shift to
        // compute. Returning the ceiling keeps a scripted supersonic object
        // from producing an infinite or negative playback rate, which is a
        // crash in most resamplers.
        return MAX_DOPPLER;
    }
    ((SPEED_OF_SOUND + listener_speed) / denominator).clamp(MIN_DOPPLER, MAX_DOPPLER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attenuation_is_full_inside_min_and_silent_past_max() {
        for curve in [
            Attenuation::Linear { min: 2.0, max: 20.0 },
            Attenuation::InverseSquare { min: 2.0, max: 20.0 },
        ] {
            assert_eq!(curve.gain(0.0), 1.0, "{curve:?}");
            assert_eq!(curve.gain(2.0), 1.0, "{curve:?}");
            assert_eq!(curve.gain(20.0), 0.0, "{curve:?}");
            assert_eq!(curve.gain(1000.0), 0.0, "{curve:?}");
        }
    }

    /// Inverse-square falls off faster near the source than linear does.
    ///
    /// That difference is the whole reason to have both: a linear curve makes a
    /// source seem to leap in volume as you approach.
    #[test]
    fn inverse_square_falls_off_faster_than_linear() {
        let linear = Attenuation::Linear { min: 1.0, max: 100.0 };
        let physical = Attenuation::InverseSquare { min: 1.0, max: 100.0 };
        assert!(physical.gain(10.0) < linear.gain(10.0));
    }

    /// Both curves are monotonic: further is never louder.
    #[test]
    fn attenuation_never_increases_with_distance() {
        for curve in [
            Attenuation::Linear { min: 1.0, max: 50.0 },
            Attenuation::InverseSquare { min: 1.0, max: 50.0 },
        ] {
            let mut previous = 1.0;
            for step in 0..200 {
                let gain = curve.gain(step as f32 * 0.5);
                assert!(gain <= previous + 1e-5, "{curve:?} rose at {step}");
                previous = gain;
            }
        }
    }

    /// **An authored min above max must not make a sound louder with distance.**
    ///
    /// A data-entry mistake in a curve editor would otherwise divide by a
    /// negative range and produce gains above one.
    #[test]
    fn an_inverted_range_is_corrected_rather_than_inverting_the_curve() {
        let curve = Attenuation::Linear { min: 50.0, max: 5.0 };
        for d in [0.0, 10.0, 25.0, 60.0] {
            let gain = curve.gain(d);
            assert!((0.0..=1.0).contains(&gain), "gain {gain} at {d}");
        }
        assert!(curve.gain(60.0) <= curve.gain(10.0));
    }

    /// An authored curve interpolates between its points and clamps outside.
    #[test]
    fn an_authored_curve_interpolates() {
        let curve = Attenuation::Curve(vec![(0.0, 1.0), (10.0, 0.5), (20.0, 0.0)]);
        assert_eq!(curve.gain(0.0), 1.0);
        assert_eq!(curve.gain(10.0), 0.5);
        assert!((curve.gain(5.0) - 0.75).abs() < 1e-4);
        assert_eq!(curve.gain(100.0), 0.0, "past the last point");
    }

    #[test]
    fn an_empty_curve_is_full_volume_rather_than_silence() {
        // A curve nobody authored yet should be audible and obviously unshaped,
        // not silent and mistaken for a broken sound.
        assert_eq!(Attenuation::Curve(vec![]).gain(1000.0), 1.0);
    }

    /// **A sound to the right comes out of the right ear.**
    ///
    /// The one calculation that is instantly noticeable when it is backwards.
    #[test]
    fn panning_follows_the_listeners_orientation() {
        let listener = Listener::looking_at(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let right = Emitter {
            position: Vec3::new(10.0, 0.0, 0.0),
            attenuation: Attenuation::None,
            ..Default::default()
        };
        assert!(evaluate(&listener, &right, 0.0).pan > 0.9);

        let left = Emitter {
            position: Vec3::new(-10.0, 0.0, 0.0),
            attenuation: Attenuation::None,
            ..Default::default()
        };
        assert!(evaluate(&listener, &left, 0.0).pan < -0.9);
    }

    /// Turning the listener moves the sound to the other ear.
    #[test]
    fn turning_around_swaps_the_ears() {
        let facing = Listener::looking_at(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let turned = Listener::looking_at(Vec3::ZERO, Vec3::Z, Vec3::Y);
        let source = Emitter {
            position: Vec3::new(10.0, 0.0, 0.0),
            attenuation: Attenuation::None,
            ..Default::default()
        };
        let a = evaluate(&facing, &source, 0.0).pan;
        let b = evaluate(&turned, &source, 0.0).pan;
        assert!(a * b < 0.0, "the sound changed sides: {a} then {b}");
    }

    /// A source exactly at the listener is centred, not jumping.
    #[test]
    fn a_source_at_the_listener_is_centred() {
        let listener = Listener::default();
        let source = Emitter {
            position: Vec3::ZERO,
            attenuation: Attenuation::None,
            ..Default::default()
        };
        assert_eq!(evaluate(&listener, &source, 0.0).pan, 0.0);
    }

    /// A cone quietens a listener off its axis, and `outer_gain` stops the pop.
    #[test]
    fn a_cone_quietens_off_axis() {
        let cone = Cone {
            inner: 0.3,
            outer: 1.0,
            outer_gain: 0.25,
        };
        assert_eq!(cone.gain(0.0), 1.0);
        assert_eq!(cone.gain(0.3), 1.0);
        assert!((cone.gain(2.0) - 0.25).abs() < 1e-5);
        let middle = cone.gain(0.65);
        assert!(middle > 0.25 && middle < 1.0, "got {middle}");
    }

    /// A cone pointing at the listener is full volume; away is `outer_gain`.
    #[test]
    fn cone_direction_is_measured_against_the_listener() {
        let listener = Listener::default();
        let toward = Emitter {
            position: Vec3::new(0.0, 0.0, -5.0),
            direction: Vec3::Z, // pointing back at the listener
            cone: Some(Cone {
                inner: 0.2,
                outer: 0.8,
                outer_gain: 0.1,
            }),
            attenuation: Attenuation::None,
            ..Default::default()
        };
        let mut away = toward.clone();
        away.direction = Vec3::NEG_Z;

        assert!(evaluate(&listener, &toward, 0.0).gain > 0.9);
        assert!(evaluate(&listener, &away, 0.0).gain < 0.2);
    }

    /// Occlusion multiplies in, and is supplied rather than computed.
    #[test]
    fn occlusion_is_a_plain_factor() {
        let listener = Listener::default();
        let source = Emitter {
            position: Vec3::new(0.0, 0.0, -5.0),
            attenuation: Attenuation::None,
            occlusion: 0.25,
            ..Default::default()
        };
        assert!((evaluate(&listener, &source, 0.0).gain - 0.25).abs() < 1e-5);
    }

    /// Approaching raises pitch; receding lowers it.
    #[test]
    fn doppler_shifts_the_right_way() {
        let listener = Listener::default();
        let approaching = Emitter {
            position: Vec3::new(0.0, 0.0, -50.0),
            velocity: Vec3::new(0.0, 0.0, 30.0), // moving towards the listener
            attenuation: Attenuation::None,
            ..Default::default()
        };
        let mut receding = approaching.clone();
        receding.velocity = Vec3::new(0.0, 0.0, -30.0);

        assert!(evaluate(&listener, &approaching, 1.0).doppler > 1.0);
        assert!(evaluate(&listener, &receding, 1.0).doppler < 1.0);
    }

    #[test]
    fn doppler_is_off_at_zero_scale() {
        let listener = Listener::default();
        let source = Emitter {
            position: Vec3::new(0.0, 0.0, -50.0),
            velocity: Vec3::new(0.0, 0.0, 100.0),
            ..Default::default()
        };
        assert_eq!(evaluate(&listener, &source, 0.0).doppler, 1.0);
    }

    /// **A supersonic source does not produce an infinite pitch.**
    ///
    /// The Doppler formula has a pole at the speed of sound, and an infinite
    /// playback rate is a crash in most resamplers. A scripted object moving
    /// that fast is a thing designers do.
    #[test]
    fn a_supersonic_source_is_clamped_rather_than_infinite() {
        let listener = Listener::default();
        let source = Emitter {
            position: Vec3::new(0.0, 0.0, -50.0),
            velocity: Vec3::new(0.0, 0.0, 400.0),
            ..Default::default()
        };
        let doppler = evaluate(&listener, &source, 1.0).doppler;
        assert!(doppler.is_finite() && doppler <= 4.0, "got {doppler}");
    }

    /// A listener moving towards a stationary source also raises the pitch.
    ///
    /// The two halves of the formula have different signs, so testing only the
    /// emitter half would leave the listener half free to be wrong.
    #[test]
    fn a_moving_listener_shifts_too() {
        let source = Emitter {
            position: Vec3::new(0.0, 0.0, -50.0),
            attenuation: Attenuation::None,
            ..Default::default()
        };
        let approaching = Listener {
            velocity: Vec3::new(0.0, 0.0, -30.0), // towards the source
            ..Default::default()
        };
        let receding = Listener {
            velocity: Vec3::new(0.0, 0.0, 30.0),
            ..Default::default()
        };
        assert!(evaluate(&approaching, &source, 1.0).doppler > 1.0);
        assert!(evaluate(&receding, &source, 1.0).doppler < 1.0);
    }

    /// A source moving sideways produces no shift.
    #[test]
    fn perpendicular_motion_does_not_shift() {
        let listener = Listener::default();
        let crossing = Emitter {
            position: Vec3::new(0.0, 0.0, -50.0),
            velocity: Vec3::new(40.0, 0.0, 0.0),
            attenuation: Attenuation::None,
            ..Default::default()
        };
        assert!((evaluate(&listener, &crossing, 1.0).doppler - 1.0).abs() < 1e-4);
    }
}
