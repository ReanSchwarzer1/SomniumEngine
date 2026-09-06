//! Floating window placement, independent of the retained panel and its edits.
use crate::floating::FloatingKind;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug)]
pub struct MonitorBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: f64,
    pub height: f64,
}
impl Placement {
    /// Save logical client size, physical desktop position. A removed monitor
    /// cannot leave an unreachable title bar; negative desktop origins are valid.
    pub fn recovered(self, monitors: &[MonitorBounds], minimum: (u32, u32)) -> Self {
        let Some(monitor) = monitors
            .iter()
            .filter(|m| m.width > 0 && m.height > 0 && m.scale.is_finite() && m.scale > 0.0)
            .min_by(|a, b| distance(self, a).total_cmp(&distance(self, b)))
        else {
            return self;
        };
        let width = finite_size(self.width, minimum.0 as f64).clamp(
            minimum.0 as f64,
            (monitor.width as f64 / monitor.scale).max(minimum.0 as f64),
        );
        let height = finite_size(self.height, minimum.1 as f64).clamp(
            minimum.1 as f64,
            ((monitor.height as f64 - 40.0) / monitor.scale).max(minimum.1 as f64),
        );
        let right = monitor.x as f64 + (monitor.width as f64 - width * monitor.scale).max(0.0);
        let bottom =
            monitor.y as f64 + (monitor.height as f64 - height * monitor.scale - 40.0).max(0.0);
        Self {
            x: (self.x as f64).clamp(monitor.x as f64, right) as i32,
            y: (self.y as f64).clamp(monitor.y as f64, bottom) as i32,
            width,
            height,
        }
    }
}
fn finite_size(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}
fn distance(p: Placement, m: &MonitorBounds) -> f64 {
    let dx = (p.x as f64 - (p.x as f64).clamp(m.x as f64, m.x as f64 + m.width as f64)).abs();
    let dy = (p.y as f64 - (p.y as f64).clamp(m.y as f64, m.y as f64 + m.height as f64)).abs();
    dx * dx + dy * dy
}
pub struct FloatingLayout {
    placements: BTreeMap<String, Placement>,
    dirty: bool,
    changed: Instant,
    monitor_check: Instant,
}
impl Default for FloatingLayout {
    fn default() -> Self {
        Self {
            placements: BTreeMap::new(),
            dirty: false,
            changed: Instant::now(),
            monitor_check: Instant::now(),
        }
    }
}
impl FloatingLayout {
    fn path() -> Option<std::path::PathBuf> {
        std::env::var_os("APPDATA")
            .map(|p| std::path::PathBuf::from(p).join("SomniumEngine/floating_windows.json"))
    }
    pub fn load() -> Self {
        let mut state = Self::default();
        state.placements = Self::path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        state
            .placements
            .retain(|name, _| FloatingKind::from_slug(name).is_some());
        state
    }
    pub fn get(&self, kind: FloatingKind) -> Option<Placement> {
        self.placements.get(kind.slug()).copied()
    }
    pub fn record(&mut self, kind: FloatingKind, placement: Placement) {
        if self.get(kind) != Some(placement) {
            self.placements.insert(kind.slug().into(), placement);
            self.dirty = true;
            self.changed = Instant::now();
        }
    }
    pub fn monitor_check_due(&mut self) -> bool {
        if self.monitor_check.elapsed() < Duration::from_secs(2) {
            return false;
        }
        self.monitor_check = Instant::now();
        true
    }
    pub fn flush(&mut self, force: bool) {
        if !self.dirty || (!force && self.changed.elapsed() < Duration::from_millis(500)) {
            return;
        }
        if cfg!(test) {
            self.dirty = false;
            return;
        }
        let Some(path) = Self::path() else {
            return;
        };
        let result = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(path, serde_json::to_vec_pretty(&self.placements)?)
        })();
        match result {
            Ok(()) => self.dirty = false,
            Err(error) => {
                self.changed = Instant::now();
                tracing::warn!(%error, "Could not save floating window placement");
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn monitor(x: i32, scale: f64) -> MonitorBounds {
        MonitorBounds {
            x,
            y: 0,
            width: 1920,
            height: 1080,
            scale,
        }
    }
    #[test]
    fn removed_monitor_returns_the_titlebar_and_size_to_a_live_display() {
        let saved = Placement {
            x: 5000,
            y: 2500,
            width: 900.0,
            height: 1000.0,
        };
        let got = saved.recovered(&[monitor(0, 2.0)], (320, 240));
        assert!(got.x >= 0 && got.y >= 0);
        assert!(got.x as f64 + got.width * 2.0 <= 1920.0);
        assert!(got.y as f64 + got.height * 2.0 + 40.0 <= 1080.0);
    }
    #[test]
    fn negative_desktop_origin_and_logical_size_survive_restart() {
        let saved = Placement {
            x: -1800,
            y: 80,
            width: 400.0,
            height: 500.0,
        };
        let bytes = serde_json::to_vec(&saved).unwrap();
        let decoded: Placement = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            decoded.recovered(&[monitor(0, 1.0), monitor(-1920, 1.5)], (320, 240)),
            saved
        );
    }
    #[test]
    fn invalid_and_oversized_dimensions_recover_without_nan() {
        let saved = Placement {
            x: i32::MAX,
            y: i32::MIN,
            width: f64::NAN,
            height: f64::INFINITY,
        };
        let got = saved.recovered(&[monitor(0, 1.0)], (320, 240));
        assert_eq!((got.width, got.height), (320.0, 240.0));
        assert_eq!(got.y, 0);
    }
    #[test]
    fn recording_one_window_does_not_erase_another() {
        let mut layout = FloatingLayout::default();
        let saved = Placement {
            x: 20,
            y: 40,
            width: 400.0,
            height: 500.0,
        };
        layout.record(FloatingKind::Details, saved);
        layout.record(FloatingKind::Outliner, Placement { x: 80, ..saved });
        assert_eq!(layout.get(FloatingKind::Details), Some(saved));
        layout.flush(true);
        assert!(!layout.dirty);
        layout.record(FloatingKind::Details, saved);
        assert!(!layout.dirty);
    }
}
