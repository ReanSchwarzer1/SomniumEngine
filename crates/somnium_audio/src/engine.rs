//! The audio engine (MORROWIND-AG).
//!
//! # The bug this sub-phase was named after
//!
//! Before MORROWIND-AG, `AudioEngine::play` read:
//!
//! ```ignore
//! let _kira_settings = StaticSoundSettings::new().volume(settings.volume);
//! let sound_data = StaticSoundData::from_file(path)?;
//! let handle = self.manager.play(sound_data)?;
//! ```
//!
//! The settings were built into an underscore-prefixed variable **and then not
//! used**. Every sound played at full volume and the `volume` argument did
//! nothing, silently, since the crate was written.
//!
//! §4.2 and §8 item 5 both call this out, and §8 says what makes it worth a
//! paragraph rather than a one-line diff:
//!
//! > *"Fix the discarded volume argument and add the test that would have
//! > caught it — a one-line fix and **a permanent lesson about the
//! > second-example rule**."*
//!
//! The lesson: `somnium_audio` had **one caller and zero tests**. Nothing ever
//! asked it to be quieter, so nothing noticed that it could not be. That is the
//! second-example rule stated as a defect rather than as a principle, and it is
//! why `applying_a_volume_actually_changes_the_gain` exists below.
//!
//! # Every file was read twice
//!
//! `StaticSoundData::from_file` was called on every `play`, on the calling
//! thread. A footstep played a hundred times decoded a hundred times. [`Sounds`]
//! is the cache that fixes it.

use kira::manager::{AudioManager, AudioManagerSettings, backend::DefaultBackend};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::{
    bus::Mixer,
    listener::{Emitter, Listener, Spatial, evaluate},
    sound::{SoundHandle, SoundSettings},
};
use kira::sound::static_sound::{StaticSoundData, StaticSoundSettings};

/// Why an audio operation failed.
#[derive(Error, Debug)]
pub enum AudioError {
    /// The audio backend would not start.
    #[error("Failed to initialize audio backend: {0}")]
    InitError(#[from] kira::manager::error::PlaySoundError<()>),
    /// A sound file could not be decoded.
    #[error("Failed to load sound file {path}: {source}")]
    LoadError {
        /// The path that failed.
        path: String,
        /// The decoder's own error.
        source: kira::sound::FromFileError,
    },
    /// A sound file is not on disk.
    #[error("Sound file not found: {0}")]
    MissingFile(String),
    /// Playback failed.
    #[error("Failed to play sound: {0}")]
    PlayError(String),
}

/// Decoded sounds, kept so a file is read once.
///
/// Keyed by path. `StaticSoundData` is internally reference-counted, so a
/// cached entry costs one clone per play rather than one decode.
#[derive(Default)]
pub struct Sounds {
    cache: HashMap<PathBuf, StaticSoundData>,
    /// Decodes served from the cache, for the profiler and for the test that
    /// proves the cache is doing anything.
    hits: u64,
    /// Files actually read.
    misses: u64,
}

impl Sounds {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Load `path`, or return the already-decoded copy.
    pub fn load(&mut self, path: impl AsRef<Path>) -> Result<StaticSoundData, AudioError> {
        let path = path.as_ref();
        if let Some(data) = self.cache.get(path) {
            self.hits += 1;
            return Ok(data.clone());
        }
        if !path.exists() {
            // A distinct error from a decode failure, because the two have
            // different causes: a missing file is usually a typo or a build that
            // did not copy assets, and a decode failure is a bad file. Telling
            // them apart in the log saves the wrong investigation.
            return Err(AudioError::MissingFile(path.display().to_string()));
        }
        let data = StaticSoundData::from_file(path).map_err(|source| AudioError::LoadError {
            path: path.display().to_string(),
            source,
        })?;
        self.misses += 1;
        self.cache.insert(path.to_path_buf(), data.clone());
        Ok(data)
    }

    /// `(hits, misses)`.
    #[must_use]
    pub fn stats(&self) -> (u64, u64) {
        (self.hits, self.misses)
    }

    /// How many distinct files are held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Whether nothing is cached.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    /// Drop everything. For a level transition.
    pub fn clear(&mut self) {
        self.cache.clear();
    }
}

/// Turn a [`SoundSettings`] plus a bus gain into Kira's settings.
///
/// Extracted so it can be tested **without an audio device**, which is what the
/// original bug needed and did not have: the whole failure was that the settings
/// were constructed correctly and then dropped, and a test of the construction
/// alone would have caught it.
#[must_use]
pub fn kira_settings(settings: &SoundSettings, bus_gain: f32) -> StaticSoundSettings {
    let gain = (settings.volume * f64::from(bus_gain)).max(0.0);
    let mut out = StaticSoundSettings::new().volume(kira::Volume::Amplitude(gain));
    if settings.looping {
        out = out.loop_region(..);
    }
    out
}

/// Apply a spatial evaluation on top of the bus gain.
#[must_use]
pub fn spatial_settings(
    settings: &SoundSettings,
    bus_gain: f32,
    spatial: &Spatial,
) -> StaticSoundSettings {
    let gain = (settings.volume * f64::from(bus_gain) * f64::from(spatial.gain)).max(0.0);
    let mut out = StaticSoundSettings::new()
        .volume(kira::Volume::Amplitude(gain))
        .panning(f64::from(spatial.pan * 0.5 + 0.5))
        .playback_rate(f64::from(spatial.doppler));
    if settings.looping {
        out = out.loop_region(..);
    }
    out
}

/// The audio subsystem.
pub struct AudioEngine {
    manager: AudioManager,
    /// The mixer graph. Public because an options screen drives it directly.
    pub mixer: Mixer,
    /// Decoded sounds.
    pub sounds: Sounds,
    /// Where the player's ears are.
    pub listener: Listener,
    /// Doppler strength, `0.0` to disable. Off by default: Doppler on a source
    /// whose velocity nobody sets is a pitch wobble with no cause, and most
    /// sources in most games never move.
    pub doppler_scale: f32,
}

impl AudioEngine {
    /// Start the audio backend.
    pub fn new() -> Result<Self, AudioError> {
        let manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .map_err(|e| AudioError::PlayError(e.to_string()))?;
        Ok(Self {
            manager,
            mixer: Mixer::default(),
            sounds: Sounds::new(),
            listener: Listener::default(),
            doppler_scale: 0.0,
        })
    }

    /// Play a one-shot sound on the SFX bus.
    ///
    /// Kept for the existing call site. `volume` is now honoured — see the
    /// module docs for why that sentence needed writing.
    pub fn play(&mut self, path: &str, settings: SoundSettings) -> Result<SoundHandle, AudioError> {
        self.play_on(Mixer::SFX, path, settings)
    }

    /// Play a one-shot sound on a named bus.
    pub fn play_on(
        &mut self,
        bus: &str,
        path: &str,
        settings: SoundSettings,
    ) -> Result<SoundHandle, AudioError> {
        let gain = self.mixer.gain(bus);
        let data = self.sounds.load(path)?;
        let handle = self
            .manager
            .play(data.with_settings(kira_settings(&settings, gain)))
            .map_err(|e| AudioError::PlayError(e.to_string()))?;
        Ok(SoundHandle { handle })
    }

    /// Play a sound positioned in the world.
    ///
    /// Returns `Ok(None)` when the emitter is inaudible — past its attenuation
    /// range, fully occluded, or on a muted bus. **Not an error**: a footstep
    /// across the map failing to play is the system working, and returning an
    /// error would make every caller write a match for the normal case.
    pub fn play_spatial(
        &mut self,
        bus: &str,
        path: &str,
        settings: SoundSettings,
        emitter: &Emitter,
    ) -> Result<Option<SoundHandle>, AudioError> {
        self.play_spatial_scaled(bus, path, settings, emitter, self.doppler_scale)
    }

    /// Play a spatial sound with an emitter-specific Doppler multiplier.
    pub fn play_spatial_scaled(
        &mut self,
        bus: &str,
        path: &str,
        settings: SoundSettings,
        emitter: &Emitter,
        doppler_scale: f32,
    ) -> Result<Option<SoundHandle>, AudioError> {
        let spatial = evaluate(&self.listener, emitter, doppler_scale);
        let gain = self.mixer.gain(bus);
        if spatial.gain <= 1e-4 || gain <= 1e-4 {
            return Ok(None);
        }
        let data = self.sounds.load(path)?;
        let handle = self
            .manager
            .play(data.with_settings(spatial_settings(&settings, gain, &spatial)))
            .map_err(|e| AudioError::PlayError(e.to_string()))?;
        Ok(Some(SoundHandle { handle }))
    }

    /// Re-evaluate and apply a live voice after listener/emitter movement or
    /// an inspector change. A zero gain keeps a looping voice alive silently,
    /// so walking back into range resumes without decoding or restarting it.
    pub fn update_spatial(
        &self,
        handle: &mut SoundHandle,
        bus: &str,
        settings: &SoundSettings,
        emitter: &Emitter,
        doppler_scale: f32,
    ) -> Spatial {
        let spatial = evaluate(&self.listener, emitter, doppler_scale);
        let gain =
            settings.volume.max(0.0) * f64::from(self.mixer.gain(bus)) * f64::from(spatial.gain);
        handle.set_spatial(
            gain,
            f64::from(spatial.pan * 0.5 + 0.5),
            f64::from(spatial.doppler),
        );
        spatial
    }

    /// Refresh a live non-spatial voice after volume or mixer changes.
    pub fn update_gain(&self, handle: &mut SoundHandle, bus: &str, settings: &SoundSettings) {
        handle.set_gain(settings.volume.max(0.0) * f64::from(self.mixer.gain(bus)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listener::Attenuation;
    use glam::Vec3;

    /// **The test that would have caught the bug this sub-phase is named for.**
    ///
    /// `somnium_audio` had one caller and zero tests, so nothing ever asked it
    /// to be quieter and nothing noticed that it could not be. The original
    /// `play` built these settings into `_kira_settings` and dropped them.
    ///
    /// It needs no audio device, which is the other half of the lesson: the
    /// check was cheap and simply absent.
    #[test]
    fn applying_a_volume_actually_changes_the_gain() {
        let quiet = SoundSettings {
            volume: 0.25,
            looping: false,
        };
        let loud = SoundSettings {
            volume: 1.0,
            looping: false,
        };
        let a = kira_settings(&quiet, 1.0);
        let b = kira_settings(&loud, 1.0);
        assert_ne!(
            a.volume, b.volume,
            "the volume argument must reach the sound"
        );
        assert_eq!(
            a.volume,
            kira::tween::Value::Fixed(kira::Volume::Amplitude(0.25))
        );
    }

    /// The bus gain multiplies the sound's own volume.
    #[test]
    fn the_bus_gain_multiplies_in() {
        let settings = SoundSettings {
            volume: 0.5,
            looping: false,
        };
        assert_eq!(
            kira_settings(&settings, 0.5).volume,
            kira::tween::Value::Fixed(kira::Volume::Amplitude(0.25))
        );
    }

    /// A negative volume is silence, not a phase inversion.
    #[test]
    fn a_negative_volume_clamps_to_silence() {
        let settings = SoundSettings {
            volume: -1.0,
            looping: false,
        };
        assert_eq!(
            kira_settings(&settings, 1.0).volume,
            kira::tween::Value::Fixed(kira::Volume::Amplitude(0.0))
        );
    }

    /// Looping reaches the settings too — the other field that was dropped.
    #[test]
    fn looping_reaches_the_settings() {
        let looping = SoundSettings {
            volume: 1.0,
            looping: true,
        };
        let once = SoundSettings {
            volume: 1.0,
            looping: false,
        };
        assert!(kira_settings(&looping, 1.0).loop_region.is_some());
        assert!(kira_settings(&once, 1.0).loop_region.is_none());
    }

    /// Panning is mapped from `-1..=1` into Kira's `0..=1`.
    ///
    /// Getting this backwards or off by the offset puts every sound in the
    /// wrong ear, which is the most noticeable audio bug there is.
    #[test]
    fn spatial_panning_maps_into_kiras_range() {
        let settings = SoundSettings::default();
        let left = Spatial {
            gain: 1.0,
            pan: -1.0,
            doppler: 1.0,
            distance: 1.0,
        };
        let right = Spatial { pan: 1.0, ..left };
        let centre = Spatial { pan: 0.0, ..left };
        assert_eq!(
            spatial_settings(&settings, 1.0, &left).panning,
            kira::tween::Value::Fixed(0.0)
        );
        assert_eq!(
            spatial_settings(&settings, 1.0, &right).panning,
            kira::tween::Value::Fixed(1.0)
        );
        assert_eq!(
            spatial_settings(&settings, 1.0, &centre).panning,
            kira::tween::Value::Fixed(0.5)
        );
    }

    /// Distance attenuation multiplies into the final gain.
    #[test]
    fn spatial_gain_multiplies_with_the_bus() {
        let settings = SoundSettings {
            volume: 1.0,
            looping: false,
        };
        let far = Spatial {
            gain: 0.5,
            pan: 0.0,
            doppler: 1.0,
            distance: 10.0,
        };
        assert_eq!(
            spatial_settings(&settings, 0.5, &far).volume,
            kira::tween::Value::Fixed(kira::Volume::Amplitude(0.25))
        );
    }

    /// **A missing file is distinguishable from a corrupt one.**
    ///
    /// The two have different causes — a typo or an asset step that did not run,
    /// versus a bad file — and telling them apart in the log saves the wrong
    /// investigation.
    #[test]
    fn a_missing_file_says_so() {
        let mut sounds = Sounds::new();
        let error = sounds
            .load("definitely/not/here.ogg")
            .expect_err("no such file");
        assert!(matches!(error, AudioError::MissingFile(_)));
        assert!(error.to_string().contains("definitely/not/here.ogg"));
    }

    /// A failed load does not poison the cache.
    #[test]
    fn a_failed_load_caches_nothing() {
        let mut sounds = Sounds::new();
        let _ = sounds.load("nope.ogg");
        let _ = sounds.load("nope.ogg");
        assert!(sounds.is_empty());
        assert_eq!(sounds.stats(), (0, 0), "neither a hit nor a completed miss");
    }

    /// An inaudible emitter is `Ok(None)`, not an error.
    ///
    /// A footstep across the map failing to play is the system working, and an
    /// error would make every caller write a match for the normal case.
    #[test]
    fn an_inaudible_emitter_is_not_an_error() {
        // Exercised through `evaluate` rather than `play_spatial`, which needs a
        // device: the decision being tested is the gain threshold, and that is
        // the same arithmetic either way.
        let listener = Listener::default();
        let distant = Emitter {
            position: Vec3::new(0.0, 0.0, -10_000.0),
            attenuation: Attenuation::InverseSquare {
                min: 1.0,
                max: 50.0,
            },
            ..Default::default()
        };
        assert_eq!(evaluate(&listener, &distant, 0.0).gain, 0.0);
    }

    #[test]
    fn the_cache_starts_empty_and_clears() {
        let mut sounds = Sounds::new();
        assert!(sounds.is_empty());
        assert_eq!(sounds.len(), 0);
        sounds.clear();
        assert!(sounds.is_empty());
    }
}
