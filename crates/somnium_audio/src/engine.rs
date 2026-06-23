use kira::manager::{AudioManager, AudioManagerSettings, backend::DefaultBackend};
use thiserror::Error;

use crate::sound::{SoundHandle, SoundSettings};
use kira::sound::static_sound::{StaticSoundData, StaticSoundSettings};

#[derive(Error, Debug)]
pub enum AudioError {
    #[error("Failed to initialize audio backend: {0}")]
    InitError(#[from] kira::manager::error::PlaySoundError<()>),
    #[error("Failed to load sound file {path}: {source}")]
    LoadError { path: String, source: kira::sound::FromFileError },
    #[error("Failed to play sound: {0}")]
    PlayError(String),
}

/// The main audio subsystem for the engine.
pub struct AudioEngine {
    manager: AudioManager,
}

impl AudioEngine {
    pub fn new() -> Result<Self, AudioError> {
        let manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .map_err(|e| AudioError::PlayError(e.to_string()))?;

        Ok(Self { manager })
    }

    /// Play a one-shot sound from a file path.
    pub fn play(&mut self, path: &str, settings: SoundSettings) -> Result<SoundHandle, AudioError> {
        let _kira_settings = StaticSoundSettings::new()
            .volume(settings.volume);
            
        let sound_data = StaticSoundData::from_file(path)
            .map_err(|e| AudioError::LoadError {
                path: path.to_owned(),
                source: e,
            })?;
            
        let handle = self.manager.play(sound_data)
            .map_err(|e| AudioError::PlayError(e.to_string()))?;
        
        Ok(SoundHandle { handle })
    }
}
