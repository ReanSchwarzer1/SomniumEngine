use kira::sound::static_sound::StaticSoundHandle;

/// A handle to a currently playing sound.
pub struct SoundHandle {
    pub(crate) handle: StaticSoundHandle,
}

impl SoundHandle {
    pub fn pause(&mut self) {
        let _ = self.handle.pause(kira::tween::Tween::default());
    }

    pub fn resume(&mut self) {
        let _ = self.handle.resume(kira::tween::Tween::default());
    }

    pub fn stop(&mut self) {
        let _ = self.handle.stop(kira::tween::Tween::default());
    }

    /// Whether playback has drained or has been stopped.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        matches!(
            self.handle.state(),
            kira::sound::PlaybackState::Stopped
        )
    }

    /// Update gain, stereo placement and pitch for a live spatial voice.
    pub fn set_spatial(&mut self, gain: f64, panning: f64, playback_rate: f64) {
        let tween = kira::tween::Tween::default();
        let _ = self
            .handle
            .set_volume(kira::Volume::Amplitude(gain.max(0.0)), tween);
        let _ = self.handle.set_panning(panning.clamp(0.0, 1.0), tween);
        let _ = self
            .handle
            .set_playback_rate(playback_rate.clamp(0.25, 4.0), tween);
    }

    /// Update only the gain of a non-spatial voice.
    pub fn set_gain(&mut self, gain: f64) {
        let _ = self.handle.set_volume(
            kira::Volume::Amplitude(gain.max(0.0)),
            kira::tween::Tween::default(),
        );
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
