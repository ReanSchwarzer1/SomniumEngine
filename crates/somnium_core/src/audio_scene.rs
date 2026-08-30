//! Runtime bridge from authored audio components to live mixer voices.
//!
//! The ECS stores durable intent. Kira handles are transient resources, so
//! this module reconciles them during Play and tears them down on Stop.

use std::collections::{HashMap, HashSet};

use somnium_asset::database::{AssetDbSnapshot, AssetId, AssetKind};
use somnium_audio::{
    bus::Mixer,
    engine::AudioEngine,
    listener::{Attenuation, Cone, Emitter, Listener},
    sound::{SoundHandle, SoundSettings},
};
use somnium_ecs::{Entity, World};
use tracing::warn;

use crate::{
    AudioAttenuationModel, AudioBus, AudioEmitterComponent, Parent, Transform, WorldTransform,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VoiceIdentity {
    asset: AssetId,
    bus: AudioBus,
    looping: bool,
    spatial: bool,
}

struct LiveVoice {
    identity: VoiceIdentity,
    handle: Option<SoundHandle>,
    finished: bool,
    previous_position: glam::Vec3,
}

/// Live authored voices for one engine instance.
#[derive(Default)]
pub(crate) struct AudioScene {
    voices: HashMap<Entity, LiveVoice>,
    missing_assets: HashSet<AssetId>,
    previous_listener_position: Option<glam::Vec3>,
    paused: bool,
}

impl AudioScene {
    pub(crate) fn update(
        &mut self,
        world: &World,
        assets: Option<&AssetDbSnapshot>,
        audio: &mut AudioEngine,
        camera_position: glam::Vec3,
        camera_orientation: glam::Quat,
        dt: f32,
    ) {
        let listener_velocity = self
            .previous_listener_position
            .map_or(glam::Vec3::ZERO, |previous| {
                velocity(previous, camera_position, dt)
            });
        self.previous_listener_position = Some(camera_position);
        audio.listener = Listener {
            position: camera_position,
            orientation: camera_orientation,
            velocity: listener_velocity,
        };

        let authored: Vec<_> = world
            .entities()
            .filter_map(|entity| {
                let local = *world.get::<Transform>(entity)?;
                let transform = if world.get::<Parent>(entity).is_some() {
                    world.get::<WorldTransform>(entity).map_or(local, |world| {
                        let (scale, rotation, translation) =
                            world.0.to_scale_rotation_translation();
                        Transform {
                            translation,
                            rotation,
                            scale,
                        }
                    })
                } else {
                    local
                };
                // A spline emitter is heard at the nearest point of its path
                // rather than at its origin, which is the whole difference
                // between "a marker somewhere out at sea" and "the surf,
                // wherever you are standing on the beach". Resolved here, at
                // the one place that already knows both the listener and the
                // emitter's world transform, so nothing downstream has to
                // learn that a second shape exists.
                let mut transform = transform;
                transform.translation = crate::spline::audible_position(
                    world,
                    entity,
                    transform.to_matrix(),
                    camera_position,
                );
                Some((
                    entity,
                    world.get::<AudioEmitterComponent>(entity)?.clone(),
                    transform,
                ))
            })
            .filter(|(_, emitter, _)| {
                emitter.enabled && emitter.autoplay && emitter.audio != AssetId::NONE
            })
            .collect();
        let wanted: HashSet<_> = authored.iter().map(|(entity, _, _)| *entity).collect();

        self.voices.retain(|entity, voice| {
            if wanted.contains(entity) {
                true
            } else {
                if let Some(handle) = voice.handle.as_mut() {
                    handle.stop();
                }
                false
            }
        });

        if self.paused {
            return;
        }

        for (entity, component, transform) in authored {
            let Some(record) = assets.and_then(|snapshot| snapshot.get(component.audio)) else {
                if self.missing_assets.insert(component.audio) {
                    warn!(asset = %component.audio, "Audio emitter references an unavailable asset");
                }
                continue;
            };
            if record.kind != AssetKind::Audio {
                if self.missing_assets.insert(component.audio) {
                    warn!(path = %record.relative_path, "Audio emitter asset is not an audio file");
                }
                continue;
            }
            self.missing_assets.remove(&component.audio);

            let identity = VoiceIdentity {
                asset: component.audio,
                bus: component.bus,
                looping: component.looping,
                spatial: component.spatial,
            };
            let voice = self.voices.entry(entity).or_insert_with(|| LiveVoice {
                identity,
                handle: None,
                finished: false,
                previous_position: transform.translation,
            });
            if voice.identity != identity {
                if let Some(handle) = voice.handle.as_mut() {
                    handle.stop();
                }
                voice.identity = identity;
                voice.handle = None;
                voice.finished = false;
            }
            if voice.handle.as_ref().is_some_and(SoundHandle::is_stopped) {
                voice.handle = None;
                voice.finished = true;
            }

            let settings = SoundSettings {
                volume: f64::from(component.volume.max(0.0)),
                looping: component.looping,
            };
            let bus = bus_name(component.bus);
            let emitter = spatial_emitter(
                &component,
                &transform,
                velocity(voice.previous_position, transform.translation, dt),
            );
            voice.previous_position = transform.translation;

            if let Some(handle) = voice.handle.as_mut() {
                if component.spatial {
                    audio.update_spatial(handle, bus, &settings, &emitter, component.doppler_scale);
                } else {
                    audio.update_gain(handle, bus, &settings);
                }
                continue;
            }
            if voice.finished {
                continue;
            }

            let path = record.absolute_path.to_string_lossy();
            let started = if component.spatial {
                audio.play_spatial_scaled(bus, &path, settings, &emitter, component.doppler_scale)
            } else {
                audio.play_on(bus, &path, settings).map(Some)
            };
            match started {
                Ok(handle) => voice.handle = handle,
                Err(error) => {
                    voice.finished = true;
                    warn!(path = %record.relative_path, %error, "Audio emitter could not start");
                }
            }
        }
    }

    pub(crate) fn set_paused(&mut self, paused: bool) {
        if self.paused == paused {
            return;
        }
        self.paused = paused;
        for voice in self.voices.values_mut() {
            if let Some(handle) = voice.handle.as_mut() {
                if paused {
                    handle.pause();
                } else {
                    handle.resume();
                }
            }
        }
    }

    pub(crate) fn stop_all(&mut self) {
        for voice in self.voices.values_mut() {
            if let Some(handle) = voice.handle.as_mut() {
                handle.stop();
            }
        }
        self.voices.clear();
        self.previous_listener_position = None;
        self.paused = false;
    }
}

fn bus_name(bus: AudioBus) -> &'static str {
    match bus {
        AudioBus::Sfx => Mixer::SFX,
        AudioBus::Music => Mixer::MUSIC,
        AudioBus::Dialogue => Mixer::DIALOGUE,
        AudioBus::Ui => Mixer::UI,
    }
}

fn spatial_emitter(
    component: &AudioEmitterComponent,
    transform: &Transform,
    velocity: glam::Vec3,
) -> Emitter {
    let min = component.min_distance.max(0.0);
    let max = component.max_distance.max(min + 0.001);
    let attenuation = match component.attenuation {
        AudioAttenuationModel::Linear => Attenuation::Linear { min, max },
        AudioAttenuationModel::InverseSquare => Attenuation::InverseSquare { min, max },
        AudioAttenuationModel::None => Attenuation::None,
        AudioAttenuationModel::Authored => {
            let points = if component.attenuation_curve.is_empty() {
                vec![(min, 1.0), (max, 0.0)]
            } else {
                sample_curve(&component.attenuation_curve)
            };
            Attenuation::Curve(points)
        }
    };
    let cone = component.cone_enabled.then(|| Cone {
        inner: component.cone_inner_degrees.to_radians(),
        outer: component.cone_outer_degrees.to_radians(),
        outer_gain: component.cone_outer_gain,
    });
    Emitter {
        position: transform.translation,
        velocity,
        attenuation,
        direction: transform.rotation * glam::Vec3::NEG_Z,
        cone,
        occlusion: component.occlusion,
    }
}

fn sample_curve(curve: &somnium_ecs::curve::Curve) -> Vec<(f32, f32)> {
    let Some((start, end)) = curve.domain() else {
        return Vec::new();
    };
    const SEGMENTS: usize = 32;
    (0..=SEGMENTS)
        .map(|index| {
            let t = index as f32 / SEGMENTS as f32;
            let distance = start + (end - start) * t;
            (distance, curve.evaluate(distance))
        })
        .collect()
}

fn velocity(previous: glam::Vec3, current: glam::Vec3, dt: f32) -> glam::Vec3 {
    if dt > 1.0e-5 {
        (current - previous) / dt
    } else {
        glam::Vec3::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_ecs::curve::{Curve, CurveKey};

    #[test]
    fn authored_curve_keeps_smooth_interpolation_when_baked_for_audio() {
        let curve = Curve::from_keys(vec![
            CurveKey::smooth(0.0, 1.0),
            CurveKey::smooth(10.0, 0.0),
        ]);
        let points = sample_curve(&curve);
        assert_eq!(points.first().copied(), Some((0.0, 1.0)));
        assert_eq!(points.last().copied(), Some((10.0, 0.0)));
        assert_eq!(points.len(), 33);
    }

    #[test]
    fn component_builds_directional_spatial_state_from_its_transform() {
        let component = AudioEmitterComponent {
            cone_enabled: true,
            ..Default::default()
        };
        let transform = Transform {
            rotation: glam::Quat::from_rotation_y(1.0),
            ..Default::default()
        };
        let emitter = spatial_emitter(&component, &transform, glam::Vec3::X);
        assert!(emitter.cone.is_some());
        assert!((emitter.direction - transform.rotation * glam::Vec3::NEG_Z).length() < 1.0e-6);
        assert_eq!(emitter.velocity, glam::Vec3::X);
    }

    #[test]
    fn every_shipped_acceptance_clip_decodes() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/audio");
        let mut sounds = somnium_audio::engine::Sounds::new();
        for relative in [
            "ambient/coastal_waves_cc0.flac",
            "ambient/island_waves_cc0.flac",
            "footsteps/footstep_01_cc0.ogg",
            "footsteps/footstep_02_cc0.ogg",
            "footsteps/footstep_03_cc0.ogg",
            "footsteps/footstep_04_cc0.ogg",
            "sfx/water_splash_cc0.wav",
        ] {
            sounds
                .load(root.join(relative))
                .unwrap_or_else(|error| panic!("{relative} must decode: {error}"));
        }
        assert_eq!(sounds.len(), 7);
    }
}
