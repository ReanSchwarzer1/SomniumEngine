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
