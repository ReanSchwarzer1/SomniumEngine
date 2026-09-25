//! Failing-fixture flicker for local lights.
//!
//! A multiplier on the light's output, evaluated where lights are submitted to
//! the renderer. The authored [`LightComponent`](crate::LightComponent) is never
//! written, so a flickering lamp does not dirty the scene or save a random
//! intensity, and turning the component off restores the exact authored value.
//!
//! The pattern is a deterministic function of time and `seed`: a low buzz, and
//! occasional bursts where the fixture stutters toward dark. Two lamps with
//! different seeds never flicker in step.

use somnium_ecs::Component;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightFlickerComponent {
    pub enabled: bool,
    /// Depth of the continuous buzz, `0..1` of the light's output.
    pub strength: f32,
    /// Bursts per second, on average.
    pub rate: f32,
    /// Chance that a burst takes the fixture nearly dark, `0..1`.
    pub dropout: f32,
    /// Decorrelates fixtures that share every other setting.
    pub seed: f32,
}

impl Component for LightFlickerComponent {}

impl Default for LightFlickerComponent {
    fn default() -> Self {
        Self {
            enabled: true,
            strength: 0.25,
            rate: 0.6,
            dropout: 0.35,
            seed: 0.0,
        }
    }
}

fn hash(x: f32) -> f32 {
    ((x * 12.9898).sin() * 43_758.547).fract().abs()
}

fn value_noise(t: f32, seed: f32) -> f32 {
    let i = t.floor();
    let f = t - i;
    let s = f * f * (3.0 - 2.0 * f);
    hash(i + seed * 17.13) * (1.0 - s) + hash(i + 1.0 + seed * 17.13) * s
}

impl LightFlickerComponent {
    /// Output multiplier at `time` seconds, in `0..=1`.
    #[must_use]
    pub fn factor(&self, time: f32) -> f32 {
        if !self.enabled {
            return 1.0;
        }
        let strength = self.strength.clamp(0.0, 1.0);
        // Mains buzz: fast, shallow, never regular.
        let buzz = 1.0 - strength * 0.5 * value_noise(time * 11.0, self.seed);
        let rate = self.rate.max(0.0);
        if rate <= 0.0 {
            return buzz.clamp(0.0, 1.0);
        }
        // Bursts: each window of 1/rate seconds may hold one short stutter.
        let window = time * rate + self.seed * 3.7;
        let index = window.floor();
        let phase = window - index;
        let burst_len = 0.35;
        if phase < burst_len && hash(index + 0.5) < self.dropout.clamp(0.0, 1.0) {
            // Square on/off chatter inside the burst, like a failing starter.
            let chatter = hash((time * 24.0).floor() + index * 7.0);
            let depth = if chatter < 0.55 { 0.04 } else { 0.55 + 0.3 * chatter };
            return (buzz * depth).clamp(0.0, 1.0);
        }
        buzz.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_is_exactly_the_authored_light() {
        let flicker = LightFlickerComponent {
            enabled: false,
            ..Default::default()
        };
        assert_eq!(flicker.factor(12.3), 1.0);
    }

    #[test]
    fn the_factor_stays_in_range_and_actually_dips() {
        let flicker = LightFlickerComponent {
            dropout: 1.0,
            ..Default::default()
        };
        let samples: Vec<f32> = (0..4000).map(|i| flicker.factor(i as f32 * 0.013)).collect();
        assert!(samples.iter().all(|f| (0.0..=1.0).contains(f)));
        assert!(samples.iter().any(|&f| f < 0.1), "bursts must reach near-dark");
        assert!(samples.iter().any(|&f| f > 0.8), "the lamp must also be lit");
    }

    #[test]
    fn seeds_decorrelate_neighbouring_fixtures() {
        let a = LightFlickerComponent { seed: 1.0, ..Default::default() };
        let b = LightFlickerComponent { seed: 2.0, ..Default::default() };
        let differ = (0..500)
            .filter(|i| (a.factor(*i as f32 * 0.05) - b.factor(*i as f32 * 0.05)).abs() > 1.0e-3)
            .count();
        assert!(differ > 250);
    }
}
