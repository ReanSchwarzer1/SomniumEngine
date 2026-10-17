//! Binaural rendering of positioned sounds through measured head responses (The Somnus Fracture's spatial audio,
//! 15 October 2026).
//!
//! On headphones a stereo pan can say left or right but not in front, behind, above or below; the ear's own filtering
//! says that. A voice here is a mono source convolved, sample by sample, with the left and right impulse responses
//! measured on a KEMAR dummy head for its direction (MIT Media Lab, Bill Gardner and Keith Martin, 1994: "provided free
//! with no restrictions on use, provided the authors are cited"; packed by `tools/hrir_kemar_convert.py`). The game
//! moves the direction from its own thread through atomics; a change crossfades over one block so it never clicks.
//! Nothing in `process` allocates.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

use kira::clock::clock_info::ClockInfoProvider;
use kira::modulator::value_provider::ModulatorValueProvider;
use kira::sound::{Sound, SoundData};
use kira::{Frame, OutputDestination};

/// Longest response after resampling to the output rate (128 taps at 44.1 kHz is 279 at 96 kHz).
const MAX_TAPS: usize = 288;
/// Output frames between direction checks; a change crossfades across one block.
const BLOCK: u32 = 64;

/// The measured responses, by elevation ring.
pub struct HrirSet {
    taps: usize,
    rate: f32,
    /// (elevation, [(azimuth, left, right)]) sorted by elevation, each ring by azimuth (0 to 180: a source on the right).
    rings: Vec<(f32, Vec<(f32, Vec<f32>, Vec<f32>)>)>,
}

impl HrirSet {
    /// The KEMAR set compiled into the engine.
    pub fn kemar() -> &'static HrirSet {
        static SET: OnceLock<HrirSet> = OnceLock::new();
        SET.get_or_init(|| HrirSet::parse(include_bytes!("../assets/hrir_mit_kemar.bin")).expect("the packed KEMAR set"))
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let u16_at = |at: usize| bytes.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or("truncated");
        if bytes.get(..4) != Some(b"HRIR".as_slice()) || u16_at(4)? != 1 {
            return Err("not a version 1 HRIR table".into());
        }
        let taps = usize::from(u16_at(6)?);
        let rate = bytes.get(8..12).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or("truncated")?;
        let count = usize::from(u16_at(12)?);
        let stride = 4 + taps * 4;
        if taps == 0 || taps > 128 || bytes.len() != 14 + count * stride {
            return Err("size does not match the header".into());
        }
        let mut rings: Vec<(f32, Vec<(f32, Vec<f32>, Vec<f32>)>)> = Vec::new();
        for entry in bytes[14..].chunks_exact(stride) {
            let elevation = f32::from(i16::from_le_bytes([entry[0], entry[1]]));
            let azimuth = f32::from(i16::from_le_bytes([entry[2], entry[3]]));
            let sample = |i: usize| f32::from(i16::from_le_bytes([entry[4 + i * 2], entry[5 + i * 2]])) / 32768.0;
            let left = (0..taps).map(|t| sample(t * 2)).collect();
            let right = (0..taps).map(|t| sample(t * 2 + 1)).collect();
            match rings.last_mut() {
                Some((at, ring)) if *at == elevation => ring.push((azimuth, left, right)),
                _ => rings.push((elevation, vec![(azimuth, left, right)])),
            }
        }
        Ok(Self { taps, rate: rate as f32, rings })
    }

    /// The responses for a direction in the listener's frame (azimuth degrees clockwise from straight ahead, 90 the
    /// right ear; elevation degrees up), resampled to `rate`, into `left` and `right`. Returns the number of taps.
    /// The nearest ring, interpolated between its two nearest azimuths; the left side is the right side mirrored.
    pub fn responses(&self, azimuth: f32, elevation: f32, rate: f32, left: &mut [f32; MAX_TAPS], right: &mut [f32; MAX_TAPS]) -> usize {
        let azimuth = (azimuth + 180.0).rem_euclid(360.0) - 180.0;
        let (mirror, azimuth) = (azimuth < 0.0, azimuth.abs());
        let ring = &self.rings.iter().min_by(|a, b| (a.0 - elevation).abs().total_cmp(&(b.0 - elevation).abs())).unwrap().1;
        let upper = ring.iter().position(|(at, _, _)| *at >= azimuth).unwrap_or(ring.len() - 1);
        let lower = upper.saturating_sub(1);
        let (a, b) = (&ring[lower], &ring[upper]);
        let t = if b.0 > a.0 { ((azimuth - a.0) / (b.0 - a.0)).clamp(0.0, 1.0) } else { 0.0 };
        let step = self.rate / rate.max(1.0);
        let taps = ((self.taps as f32 / step).ceil() as usize).min(MAX_TAPS);
        let at = |response: &[f32], position: f32| {
            let i = position as usize;
            let f = position - i as f32;
            response.get(i).copied().unwrap_or(0.0) * (1.0 - f) + response.get(i + 1).copied().unwrap_or(0.0) * f
        };
        for k in 0..taps {
            let p = k as f32 * step;
            let l = (at(&a.1, p) * (1.0 - t) + at(&b.1, p) * t) * step;
            let r = (at(&a.2, p) * (1.0 - t) + at(&b.2, p) * t) * step;
            (left[k], right[k]) = if mirror { (r, l) } else { (l, r) };
        }
        taps
    }
}

/// What the game changes while a voice plays: direction, gain, stop.
#[derive(Default)]
pub struct HrtfControl {
    azimuth: AtomicU32,
    elevation: AtomicU32,
    gain: AtomicU32,
    stopped: AtomicBool,
    paused: AtomicBool,
    finished: AtomicBool,
    /// Seconds played, for whoever times subtitles by it.
    seconds: AtomicU32,
}

/// The game's side of a binaural voice.
#[derive(Clone)]
pub struct HrtfHandle(Arc<HrtfControl>);

impl HrtfHandle {
    pub fn set(&self, azimuth: f32, elevation: f32, gain: f32) {
        self.set_direction(azimuth, elevation);
        self.set_gain(gain);
    }
    pub fn set_direction(&self, azimuth: f32, elevation: f32) {
        self.0.azimuth.store(azimuth.to_bits(), Ordering::Relaxed);
        self.0.elevation.store(elevation.to_bits(), Ordering::Relaxed);
    }
    pub fn set_gain(&self, gain: f32) {
        self.0.gain.store(gain.max(0.0).to_bits(), Ordering::Relaxed);
    }
    pub fn stop(&self) {
        self.0.stopped.store(true, Ordering::Relaxed);
    }
    pub fn set_paused(&self, paused: bool) {
        self.0.paused.store(paused, Ordering::Relaxed);
    }
    pub fn is_paused(&self) -> bool {
        self.0.paused.load(Ordering::Relaxed)
    }
    pub fn is_stopped(&self) -> bool {
        self.0.finished.load(Ordering::Relaxed)
    }
    pub fn position_seconds(&self) -> f64 {
        f64::from(f32::from_bits(self.0.seconds.load(Ordering::Relaxed)))
    }
}

/// A mono sound to play binaurally.
pub struct HrtfSoundData {
    pub samples: Arc<[f32]>,
    pub rate: u32,
    pub looping: bool,
    pub azimuth: f32,
    pub elevation: f32,
    pub gain: f32,
    pub destination: OutputDestination,
}

impl SoundData for HrtfSoundData {
    type Error = ();
    type Handle = HrtfHandle;

    fn into_sound(self) -> Result<(Box<dyn Sound>, HrtfHandle), ()> {
        let handle = HrtfHandle(Arc::new(HrtfControl::default()));
        handle.set(self.azimuth, self.elevation, self.gain);
        let voice = HrtfVoice {
            control: handle.0.clone(),
            set: HrirSet::kemar(),
            samples: self.samples,
            rate: f64::from(self.rate),
            looping: self.looping,
            destination: self.destination,
            position: 0.0,
            history: [0.0; MAX_TAPS],
            head: 0,
            taps: 0,
            filters: [[0.0; MAX_TAPS]; 4],
            fade: 0,
            since: BLOCK,
            direction: (f32::NAN, f32::NAN),
            output_rate: 0.0,
        };
        Ok((Box::new(voice), handle))
    }
}

struct HrtfVoice {
    control: Arc<HrtfControl>,
    set: &'static HrirSet,
    samples: Arc<[f32]>,
    rate: f64,
    looping: bool,
    destination: OutputDestination,
    /// Where in the source, in source samples.
    position: f64,
    /// The last MAX_TAPS input samples, a ring ending at `head`.
    history: [f32; MAX_TAPS],
    head: usize,
    taps: usize,
    /// Left and right now, then left and right being faded in.
    filters: [[f32; MAX_TAPS]; 4],
    fade: u32,
    since: u32,
    direction: (f32, f32),
    output_rate: f32,
}

impl HrtfVoice {
    fn convolve(&self, left: usize, right: usize) -> (f32, f32) {
        let (mut l, mut r) = (0.0, 0.0);
        for k in 0..self.taps {
            let x = self.history[(self.head + MAX_TAPS - k) % MAX_TAPS];
            l += x * self.filters[left][k];
            r += x * self.filters[right][k];
        }
        (l, r)
    }
}

impl Sound for HrtfVoice {
    fn output_destination(&mut self) -> OutputDestination {
        self.destination
    }

    fn process(&mut self, dt: f64, _: &ClockInfoProvider, _: &ModulatorValueProvider) -> Frame {
        if self.control.stopped.load(Ordering::Relaxed) || self.samples.is_empty() {
            self.control.finished.store(true, Ordering::Relaxed);
            return Frame::ZERO;
        }
        if self.control.paused.load(Ordering::Relaxed) {
            return Frame::ZERO;
        }
        self.output_rate = (1.0 / dt.max(1e-6)) as f32;
        // The next source sample, linearly between two.
        let at = self.position as usize;
        let f = (self.position - at as f64) as f32;
        let next = |i: usize| self.samples.get(if self.looping { i % self.samples.len() } else { i }).copied().unwrap_or(0.0);
        let x = next(at) * (1.0 - f) + next(at + 1) * f;
        self.position += dt * self.rate;
        if self.looping {
            self.position %= self.samples.len() as f64;
        } else if self.position >= self.samples.len() as f64 {
            self.control.finished.store(true, Ordering::Relaxed);
        }
        self.head = (self.head + 1) % MAX_TAPS;
        self.history[self.head] = x;
        // Once a block: has it moved? Then fade to the new responses across the next block.
        self.since += 1;
        if self.since >= BLOCK && self.fade == 0 {
            self.since = 0;
            self.control.seconds.store(((self.position / self.rate) as f32).to_bits(), Ordering::Relaxed);
            let direction = (f32::from_bits(self.control.azimuth.load(Ordering::Relaxed)), f32::from_bits(self.control.elevation.load(Ordering::Relaxed)));
            let moved = (direction.0 - self.direction.0).abs() > 0.5 || (direction.1 - self.direction.1).abs() > 0.5;
            if moved || self.direction.0.is_nan() {
                let [_, _, left, right] = &mut self.filters;
                self.taps = self.set.responses(direction.0, direction.1, self.output_rate, left, right);
                if self.direction.0.is_nan() {
                    self.filters.copy_within(2..4, 0);
                } else {
                    self.fade = BLOCK;
                }
                self.direction = direction;
            }
        }
        let gain = f32::from_bits(self.control.gain.load(Ordering::Relaxed));
        let (mut l, mut r) = self.convolve(0, 1);
        if self.fade > 0 {
            let t = 1.0 - self.fade as f32 / BLOCK as f32;
            let (nl, nr) = self.convolve(2, 3);
            (l, r) = (l * (1.0 - t) + nl * t, r * (1.0 - t) + nr * t);
            self.fade -= 1;
            if self.fade == 0 {
                self.filters.copy_within(2..4, 0);
            }
        }
        Frame { left: l * gain, right: r * gain }
    }

    fn finished(&self) -> bool {
        self.control.finished.load(Ordering::Relaxed)
    }
}

/// Direction of `world` from a listener at `position` with orientation `orientation` (glam: -Z forward, +Y up), as
/// (azimuth clockwise from straight ahead, elevation up), both in degrees.
#[must_use]
pub fn direction(listener: &crate::listener::Listener, world: glam::Vec3) -> (f32, f32) {
    let local = listener.to_local(world);
    let flat = (local.x * local.x + local.z * local.z).sqrt();
    (local.x.atan2(-local.z).to_degrees(), local.y.atan2(flat).to_degrees())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn energy(response: &[f32]) -> f32 {
        response.iter().map(|x| x * x).sum()
    }

    #[test]
    fn the_packed_set_has_every_ring_and_a_source_on_the_right_is_louder_and_earlier_in_the_right_ear() {
        let set = HrirSet::kemar();
        assert_eq!(set.rings.len(), 14, "elevations -40 to 90");
        assert_eq!(set.rings.iter().map(|r| r.1.len()).sum::<usize>(), 368);
        let (mut l, mut r) = ([0.0; MAX_TAPS], [0.0; MAX_TAPS]);
        let taps = set.responses(90.0, 0.0, 44100.0, &mut l, &mut r);
        assert_eq!(taps, 128);
        assert!(energy(&r[..taps]) > 4.0 * energy(&l[..taps]), "the right ear hears a source on the right");
        let first = |x: &[f32]| x.iter().position(|v| v.abs() > 0.05 * x.iter().fold(0.0f32, |m, v| m.max(v.abs()))).unwrap();
        assert!(first(&r[..taps]) < first(&l[..taps]), "and hears it first");
        // The left side mirrors the right.
        let (mut l2, mut r2) = ([0.0; MAX_TAPS], [0.0; MAX_TAPS]);
        set.responses(-90.0, 0.0, 44100.0, &mut l2, &mut r2);
        assert_eq!((l2[..taps].to_vec(), r2[..taps].to_vec()), (r[..taps].to_vec(), l[..taps].to_vec()));
    }

    #[test]
    fn front_and_back_differ_though_both_ears_hear_them_alike() {
        let set = HrirSet::kemar();
        let (mut fl, mut fr, mut bl, mut br) = ([0.0; MAX_TAPS], [0.0; MAX_TAPS], [0.0; MAX_TAPS], [0.0; MAX_TAPS]);
        set.responses(0.0, 0.0, 48000.0, &mut fl, &mut fr);
        let taps = set.responses(180.0, 0.0, 48000.0, &mut bl, &mut br);
        assert!(taps > 128, "resampled to 48 kHz");
        assert!((energy(&fl) / energy(&fr) - 1.0).abs() < 0.05, "straight ahead is the same in both ears");
        let difference: f32 = fl.iter().zip(&bl).map(|(a, b)| (a - b) * (a - b)).sum();
        assert!(difference > 0.05 * energy(&fl), "behind is not ahead");
    }

    #[test]
    fn a_voice_plays_through_the_responses_and_stops_when_told() {
        let samples: Arc<[f32]> = (0..2000).map(|i| ((i as f32) * 0.05).sin()).collect();
        let data = HrtfSoundData { samples, rate: 48000, looping: false, azimuth: 90.0, elevation: 0.0, gain: 1.0,
            destination: OutputDestination::MAIN_TRACK };
        let (mut voice, handle) = data.into_sound().unwrap();
        let clock = kira::clock::clock_info::MockClockInfoProviderBuilder::new(0).build();
        let modulators = kira::modulator::value_provider::MockModulatorValueProviderBuilder::new(0).build();
        let (mut left, mut right) = (0.0, 0.0);
        for _ in 0..1500 {
            let frame = voice.process(1.0 / 48000.0, &clock, &modulators);
            left += frame.left * frame.left;
            right += frame.right * frame.right;
        }
        assert!(right > 2.0 * left && left > 0.0, "the source on the right: {left} {right}");
        handle.set(-90.0, 0.0, 1.0);
        let (mut left2, mut right2) = (0.0, 0.0);
        for _ in 0..400 {
            let frame = voice.process(1.0 / 48000.0, &clock, &modulators);
            left2 += frame.left * frame.left;
            right2 += frame.right * frame.right;
        }
        assert!(left2 > right2, "moved to the left: {left2} {right2}");
        handle.stop();
        assert_eq!(voice.process(1.0 / 48000.0, &clock, &modulators), Frame::ZERO);
        assert!(voice.finished() && handle.is_stopped());
    }
}
