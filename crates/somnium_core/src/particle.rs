//! Shared particle simulation and designer-authored sprite appearance.
use crate::{Transform, WorldTransform};

/// A manual editor step advances particles once while the transport stays paused.
pub(crate) fn simulation_delta(
    clock: &crate::SimulationClock,
    stepping: bool,
    frame_delta: f32,
) -> f32 {
    if stepping {
        clock.fixed_delta_seconds
    } else if clock.state == crate::SimulationState::Paused {
        0.0
    } else {
        frame_delta
    }
}

// ─── Phase 11.5J: GPU Particle System ────────────────────────────────────────

/// Per-particle runtime state (CPU-side).
#[derive(Debug, Clone, Copy)]
pub struct ParticleState {
    /// World-space position, or offset from the emitter when `local_space` is set.
    pub position: glam::Vec3,
    /// World-space velocity (m/s).
    pub velocity: glam::Vec3,
    /// Current age in seconds (0 = just born).
    pub age: f32,
    /// Total lifetime in seconds.
    pub lifetime: f32,
    /// Stable random initial orientation in [-1, 1], independent of travel.
    pub rotation_seed: f32,
}

/// ECS component that drives a GPU particle emitter.
///
/// Add this component to an entity together with `Transform` and `WorldTransform`.
/// The engine simulates particles each frame and uploads the results to the
/// `ParticlePass` for instanced billboard rendering.
#[derive(Debug, Clone)]
pub struct ParticleEmitter {
    /// Stop emission while allowing existing particles to finish.
    pub enabled: bool,
    /// Sprite texture; NONE retains the original soft round particle.
    pub texture: somnium_asset::database::AssetId,
    /// Height divided by width of the camera-facing sprite.
    pub aspect: f32,
    /// Add emitted light instead of covering the background.
    pub additive: bool,
    /// Keep the sprite vertical in world space.
    pub world_up: bool,
    /// Birth positions remain relative to the moving emitter.
    pub local_space: bool,
    /// Additional colour multiplier at the sprite's top edge.
    pub tip_tint: glam::Vec3,
    /// Sprite-sheet columns, rows and usable frames.
    pub atlas_columns: u32,
    /// Number of sprite-sheet rows.
    pub atlas_rows: u32,
    /// Number of usable frames, bounded by columns times rows.
    pub atlas_frames: u32,
    /// Sprite-sheet frames/second; zero animates across particle lifetime.
    pub atlas_fps: f32,
    /// Random rotation range and spin in radians/second.
    pub rotation_spread: f32,
    /// Angular speed in radians per second after birth.
    pub spin: f32,
    /// One-shot burst, consumed by simulation.
    pub burst: u32,
    /// Resolved texture state, not persisted.
    pub texture_status: String,
    /// Runtime bindless slot from the common asset pipeline.
    pub texture_slot: i32,
    // ── Emitter parameters ────────────────────────────────────────────────────
    /// Maximum number of live particles at once.
    pub max_particles: u32,
    /// New particles spawned per second.
    pub spawn_rate: f32,
    /// Each particle's lifetime in seconds.
    pub lifetime: f32,
    /// Initial speed in m/s (direction is randomized within `spread_angle`).
    pub initial_speed: f32,
    /// Cone half-angle (radians) for direction randomization (0 = straight up).
    pub spread_angle: f32,
    /// Full billboard width at birth, in metres.
    pub size_start: f32,
    /// Particle size at end of life.
    pub size_end: f32,
    /// CONTROL-K: linear RGBA over the particle's life, `0` at birth and `1`
    /// at death.
    ///
    /// Replaces the `color_start`/`color_end` pair, which could express a
    /// straight line between two colours and nothing else — no flash, no
    /// fade-in-then-out, no hold. A two-stop ramp reproduces the old pair
    /// exactly, so the default is that pair.
    pub color_over_life: somnium_ecs::curve::Gradient,
    /// Downward gravity acceleration (m/s²).
    pub gravity: f32,
    /// Phase CONTROL-N: a constant velocity added to every particle at birth.
    ///
    /// The cone spawn is right for a fountain and useless for rain, which
    /// falls in one direction and is *sheared* by wind. One vector turns the
    /// same emitter into both, which is the plan's "precipitation through the
    /// existing particle emitter" rather than a second particle system.
    pub velocity_bias: [f32; 3],
    /// Phase CONTROL-N: half-extents of a box particles spawn in, around the
    /// emitter's origin.
    ///
    /// Zero is the point emitter every existing scene has. Non-zero makes the
    /// emitter a volume, which is what rain needs: a camera-anchored box
    /// overhead, so precipitation exists where the player is and nowhere else.
    pub spawn_extents: [f32; 3],

    // ── Runtime state (not user-facing) ──────────────────────────────────────
    /// Live particles owned by this emitter.
    pub particles: Vec<ParticleState>,
    /// Fractional carry-over for sub-frame spawning.
    pub spawn_accum: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_ecs::World;
    #[test]
    fn initial_rotation_stays_constant_as_particles_travel_without_spin() {
        let mut world = World::new();
        world.spawn((ParticleEmitter {
            enabled: false,
            burst: 1,
            gravity: 0.0,
            initial_speed: 0.0,
            velocity_bias: [2.0, 0.0, 0.0],
            rotation_spread: 1.0,
            spin: 0.0,
            ..Default::default()
        },));
        let first = simulate_particles(&mut world, 0.01, 1);
        let moved = simulate_particles(&mut world, 0.05, 2);
        assert_ne!(first[0].position, moved[0].position);
        assert_eq!(first[0].rotation, moved[0].rotation);
    }

    #[test]
    fn manual_step_consumes_a_paused_burst_and_advances_only_one_fixed_tick() {
        let mut clock = crate::SimulationClock {
            state: crate::SimulationState::Paused,
            ..Default::default()
        };
        let mut world = World::new();
        let entity = world.spawn((ParticleEmitter {
            enabled: false,
            burst: 1,
            initial_speed: 1.0,
            gravity: 0.0,
            ..Default::default()
        },));
        simulate_particles(&mut world, simulation_delta(&clock, false, 0.04), 1);
        assert_eq!(world.get::<ParticleEmitter>(entity).unwrap().burst, 1);
        simulate_particles(&mut world, simulation_delta(&clock, true, 0.04), 2);
        assert_eq!(world.get::<ParticleEmitter>(entity).unwrap().burst, 0);
        simulate_particles(&mut world, simulation_delta(&clock, true, 0.04), 3);
        let emitter = world.get::<ParticleEmitter>(entity).unwrap();
        assert_eq!(emitter.particles.len(), 1);
        assert_eq!(emitter.particles[0].age, clock.fixed_delta_seconds);
        let position = emitter.particles[0].position;
        simulate_particles(&mut world, simulation_delta(&clock, false, 0.04), 4);
        let particle = world.get::<ParticleEmitter>(entity).unwrap().particles[0];
        assert_eq!(particle.age, clock.fixed_delta_seconds);
        assert_eq!(particle.position, position);
        for state in [
            crate::SimulationState::Editing,
            crate::SimulationState::Playing,
        ] {
            clock.state = state;
            assert_eq!(simulation_delta(&clock, false, 0.04), 0.04);
        }
    }
    #[test]
    fn local_particles_follow_emitter_but_world_smoke_keeps_its_birth_position() {
        let mut world = World::new();
        let entity = world.spawn((
            Transform::default(),
            ParticleEmitter {
                enabled: false,
                burst: 1,
                local_space: true,
                initial_speed: 0.0,
                gravity: 0.0,
                ..Default::default()
            },
        ));
        let first = simulate_particles(&mut world, 0.01, 1);
        world.get_mut::<Transform>(entity).unwrap().translation.x = 2.0;
        let second = simulate_particles(&mut world, 0.01, 2);
        assert_eq!(second[0].position[0] - first[0].position[0], 2.0);
        let p = world.get_mut::<ParticleEmitter>(entity).unwrap();
        p.particles.clear();
        p.local_space = false;
        p.burst = 1;
        let first = simulate_particles(&mut world, 0.01, 3);
        world.get_mut::<Transform>(entity).unwrap().translation.x = 5.0;
        let second = simulate_particles(&mut world, 0.01, 4);
        assert_eq!(first[0].position, second[0].position);
    }
    #[test]
    fn paused_burst_is_retained_and_missing_texture_never_becomes_a_round_proxy() {
        let mut world = World::new();
        let entity = world.spawn((ParticleEmitter {
            enabled: false,
            burst: 3,
            texture: somnium_asset::database::AssetId::from_relative_path(std::path::Path::new(
                "smoke.png",
            )),
            ..Default::default()
        },));
        assert!(simulate_particles(&mut world, 0.0, 0).is_empty());
        assert_eq!(world.get::<ParticleEmitter>(entity).unwrap().burst, 3);
        assert!(simulate_particles(&mut world, 0.01, 1).is_empty());
        let p = world.get_mut::<ParticleEmitter>(entity).unwrap();
        assert_eq!(p.particles.len(), 3);
        assert_eq!(p.burst, 0);
        p.texture_slot = 7;
        assert_eq!(simulate_particles(&mut world, 0.01, 2).len(), 3);
    }
    #[test]
    fn flipbook_uv_tracks_lifetime_without_leaving_the_sheet() {
        let mut world = World::new();
        let entity = world.spawn((ParticleEmitter {
            enabled: false,
            atlas_columns: 4,
            atlas_rows: 2,
            atlas_frames: 6,
            ..Default::default()
        },));
        world
            .get_mut::<ParticleEmitter>(entity)
            .unwrap()
            .particles
            .push(ParticleState {
                rotation_seed: 0.0,
                position: glam::Vec3::ZERO,
                velocity: glam::Vec3::ZERO,
                age: 0.8,
                lifetime: 1.0,
            });
        let p = simulate_particles(&mut world, 0.0, 0)[0];
        assert_eq!(p.uv_rect, [0.0, 0.5, 0.25, 0.5]);
        assert_eq!(std::mem::size_of_val(&p), 80);
    }
}

impl Default for ParticleEmitter {
    fn default() -> Self {
        Self {
            enabled: true,
            texture: somnium_asset::database::AssetId::NONE,
            aspect: 1.0,
            additive: false,
            world_up: false,
            local_space: false,
            tip_tint: glam::Vec3::ONE,
            atlas_columns: 1,
            atlas_rows: 1,
            atlas_frames: 1,
            atlas_fps: 0.0,
            rotation_spread: 0.0,
            spin: 0.0,
            burst: 0,
            texture_status: "Soft round particle".into(),
            texture_slot: -1,
            max_particles: 1000,
            spawn_rate: 100.0,
            lifetime: 3.0,
            initial_speed: 5.0,
            spread_angle: 0.8,
            size_start: 1.0,
            size_end: 0.2,
            color_over_life: somnium_ecs::curve::Gradient::ramp(
                [1.0, 0.4, 0.1, 1.0],
                [0.2, 0.0, 0.0, 0.0],
            ),
            velocity_bias: [0.0; 3],
            spawn_extents: [0.0; 3],
            gravity: 1.0,
            particles: Vec::new(),
            spawn_accum: 0.0,
        }
    }
}
impl somnium_ecs::Component for ParticleEmitter {}

/// Simulate all particle emitters and return a flat list of GPU instances.
///
/// Call each frame in `about_to_wait` after physics and before `render()`.
/// `seed` increments each frame (used for deterministic pseudo-random spawn direction).
pub fn simulate_particles(
    world: &mut somnium_ecs::World,
    dt: f32,
    frame: u64,
) -> Vec<somnium_renderer::pass::particle::GpuParticle> {
    use somnium_renderer::pass::particle::GpuParticle;

    let dt = if dt.is_finite() {
        dt.clamp(0.0, 0.1)
    } else {
        0.0
    };
    let mut gpu_particles = Vec::new();

    let emitter_entities: Vec<somnium_ecs::Entity> = world
        .entities()
        .filter(|e| world.get::<ParticleEmitter>(*e).is_some())
        .collect();

    for entity in emitter_entities {
        // Borrow world piecemeal to satisfy the borrow checker.
        let origin = world
            .get::<WorldTransform>(entity)
            .map(|wt| glam::Vec3::new(wt.0.w_axis.x, wt.0.w_axis.y, wt.0.w_axis.z))
            .or_else(|| world.get::<Transform>(entity).map(|t| t.translation))
            .unwrap_or(glam::Vec3::ZERO);

        let Some(emitter) = world.get_mut::<ParticleEmitter>(entity) else {
            continue;
        };

        // ── 1. Advance existing particles ─────────────────────────────────────
        let gravity = emitter.gravity;
        emitter.particles.retain_mut(|p| {
            p.age += dt;
            p.velocity.y -= gravity * dt;
            p.position += p.velocity * dt;
            p.age < p.lifetime
        });

        // ── 2. Spawn new particles ────────────────────────────────────────────
        emitter.spawn_accum += if emitter.enabled {
            emitter.spawn_rate.clamp(0.0, 100_000.0) * dt
        } else {
            0.0
        };
        let to_spawn = emitter.spawn_accum.floor() as u32;
        emitter.spawn_accum -= to_spawn as f32;
        let available = emitter
            .max_particles
            .min(10_000)
            .saturating_sub(emitter.particles.len() as u32);
        let burst = if dt > 0.0 {
            std::mem::take(&mut emitter.burst).min(10_000)
        } else {
            0
        };
        let count = to_spawn.saturating_add(burst).min(available);

        let speed = emitter.initial_speed;
        let spread = emitter.spread_angle;
        let lifetime = emitter.lifetime.clamp(0.001, 3600.0);

        for i in 0..count {
            // Deterministic LCG pseudo-random — good enough for particles.
            let seed = frame
                .wrapping_mul(1_000_003)
                .wrapping_add((i as u64).wrapping_mul(6_364_136_223_846_793_005));
            let r1 = ((seed >> 33) & 0xFFFF) as f32 / 65535.0; // 0..1
            let r2 = ((seed >> 17) & 0xFFFF) as f32 / 65535.0 * 2.0 * std::f32::consts::PI;
            let theta = r1 * spread;
            let dir = glam::Vec3::new(theta.sin() * r2.cos(), theta.cos(), theta.sin() * r2.sin());
            // CONTROL-N: a spawn volume and a constant velocity, both zero for
            // every emitter authored before this existed.
            let extents = glam::Vec3::from(emitter.spawn_extents);
            let jitter = if extents == glam::Vec3::ZERO {
                glam::Vec3::ZERO
            } else {
                let r3 = ((seed >> 5) & 0xFFFF) as f32 / 65535.0;
                let r4 = ((seed >> 41) & 0xFFFF) as f32 / 65535.0;
                let r5 = ((seed >> 23) & 0xFFFF) as f32 / 65535.0;
                (glam::Vec3::new(r3, r4, r5) * 2.0 - glam::Vec3::ONE) * extents
            };
            emitter.particles.push(ParticleState {
                position: if emitter.local_space {
                    jitter
                } else {
                    origin + jitter
                },
                velocity: dir * speed + glam::Vec3::from(emitter.velocity_bias),
                rotation_seed: r2 / std::f32::consts::PI - 1.0,
                age: 0.0,
                lifetime,
            });
        }

        // ── 3. Emit GPU instances ─────────────────────────────────────────────
        let size_start = emitter.size_start;
        let size_end = emitter.size_end;
        // CONTROL-K: the ramp is sampled per particle rather than baked into a
        // table, because an emitter's particle count is the small number here
        // and a table would need invalidating whenever the ramp was edited —
        // which is every frame of a drag.
        let ramp = &emitter.color_over_life;

        for p in &emitter.particles {
            if emitter.texture != somnium_asset::database::AssetId::NONE && emitter.texture_slot < 0
            {
                continue;
            }
            let frac = (p.age / p.lifetime).clamp(0.0, 1.0);
            let size = size_start + (size_end - size_start) * frac;
            let color = ramp.evaluate(frac);
            let cols = emitter.atlas_columns.clamp(1, 64);
            let rows = emitter.atlas_rows.clamp(1, 64);
            let frames = emitter.atlas_frames.clamp(1, cols * rows);
            let frame = if emitter.atlas_fps > 0.0 {
                (p.age * emitter.atlas_fps.min(240.0)) as u32 % frames
            } else {
                ((frac * frames as f32) as u32).min(frames - 1)
            };
            gpu_particles.push(GpuParticle {
                position: (p.position
                    + if emitter.local_space {
                        origin
                    } else {
                        glam::Vec3::ZERO
                    })
                .to_array(),
                size,
                color,
                tip_tint: emitter.tip_tint.to_array(),
                aspect: emitter.aspect.clamp(0.01, 100.0),
                uv_rect: [
                    (frame % cols) as f32 / cols as f32,
                    (frame / cols) as f32 / rows as f32,
                    1.0 / cols as f32,
                    1.0 / rows as f32,
                ],
                rotation: emitter.spin * p.age + emitter.rotation_spread * p.rotation_seed,
                texture_index: emitter.texture_slot,
                flags: u32::from(emitter.additive) | (u32::from(emitter.world_up) << 1),
                _pad: 0,
            });
        }
    }

    gpu_particles
}
