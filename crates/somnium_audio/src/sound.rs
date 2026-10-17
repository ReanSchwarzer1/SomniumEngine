use kira::sound::static_sound::StaticSoundHandle;

/// A handle to a currently playing sound: one of Kira's, or a binaural voice (`hrtf.rs`) when positioned sounds are
/// rendered for headphones.
pub struct SoundHandle {
    pub(crate) voice: Voice,
}

pub(crate) enum Voice {
    Kira(StaticSoundHandle),
    Binaural(crate::hrtf::HrtfHandle),
}

impl SoundHandle {
    pub(crate) fn kira(handle: StaticSoundHandle) -> Self {
        Self { voice: Voice::Kira(handle) }
    }

    pub(crate) fn binaural(handle: crate::hrtf::HrtfHandle) -> Self {
        Self { voice: Voice::Binaural(handle) }
    }

    pub fn pause(&mut self) {
        match &mut self.voice {
            Voice::Kira(handle) => { let _ = handle.pause(kira::tween::Tween::default()); }
            Voice::Binaural(handle) => handle.set_paused(true),
        }
    }

    pub fn resume(&mut self) {
        match &mut self.voice {
            Voice::Kira(handle) => { let _ = handle.resume(kira::tween::Tween::default()); }
            Voice::Binaural(handle) => handle.set_paused(false),
        }
    }

    pub fn stop(&mut self) {
        match &mut self.voice {
            Voice::Kira(handle) => { let _ = handle.stop(kira::tween::Tween::default()); }
            Voice::Binaural(handle) => handle.stop(),
        }
    }

    /// Current position reported by the audio thread, in seconds.
    #[must_use]
    pub fn position_seconds(&self) -> f64 {
        match &self.voice {
            Voice::Kira(handle) => handle.position(),
            Voice::Binaural(handle) => handle.position_seconds(),
        }
    }

    /// Whether the audio thread has completed its pause transition.
    #[must_use]
    pub fn is_paused(&self) -> bool {
        match &self.voice {
            Voice::Kira(handle) => matches!(handle.state(), kira::sound::PlaybackState::Paused),
            Voice::Binaural(handle) => handle.is_paused(),
        }
    }

    /// Whether playback has drained or has been stopped.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        match &self.voice {
            Voice::Kira(handle) => matches!(handle.state(), kira::sound::PlaybackState::Stopped),
            Voice::Binaural(handle) => handle.is_stopped(),
        }
    }

    /// Update gain, stereo placement and pitch for a live spatial voice. A binaural voice takes its direction from
    /// `set_direction` and ignores the pan and the pitch.
    pub fn set_spatial(&mut self, gain: f64, panning: f64, playback_rate: f64) {
        match &mut self.voice {
            Voice::Kira(handle) => {
                let tween = kira::tween::Tween::default();
                let _ = handle.set_volume(kira::Volume::Amplitude(gain.max(0.0)), tween);
                let _ = handle.set_panning(panning.clamp(0.0, 1.0), tween);
                let _ = handle.set_playback_rate(playback_rate.clamp(0.25, 4.0), tween);
            }
            Voice::Binaural(handle) => handle.set_gain(gain as f32),
        }
    }

    /// Where a binaural voice is heard from (degrees clockwise from ahead, degrees up). Nothing for Kira's voices.
    pub fn set_direction(&mut self, azimuth: f32, elevation: f32) {
        if let Voice::Binaural(handle) = &self.voice {
            handle.set_direction(azimuth, elevation);
        }
    }

    /// Update only the gain of a non-spatial voice.
    pub fn set_gain(&mut self, gain: f64) {
        match &mut self.voice {
            Voice::Kira(handle) => {
                let _ = handle.set_volume(kira::Volume::Amplitude(gain.max(0.0)), kira::tween::Tween::default());
            }
            Voice::Binaural(handle) => handle.set_gain(gain as f32),
        }
    }
}

/// Settings for playing a sound.
#[derive(Debug, Clone)]
pub struct SoundSettings {
    pub volume: f64,
    pub looping: bool,
}

impl Default for SoundSettings {
    fn default() -> Self {
        Self {
            volume: 1.0,
            looping: false,
        }
    }
}
