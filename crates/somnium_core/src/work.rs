//! Interruptible work shared by games, Details and authoring. Progress advances
//! only after contact, with valid aim/reach/visibility and the required tool.
use crate::{EngineContext, Transform, WorldTransform};
use glam::{Quat, Vec3};
use somnium_ecs::{
    Component, Entity, component_schema,
    reflect::{FieldFlags, FieldType, TypeRegistry},
};

#[derive(Clone, Debug)]
/// Authored burn/repair work and inspectable runtime progress.
pub struct WorkTarget {
    /// 0 burn with a flame, 1 repair with the free hand.
    pub kind: u32,
    /// Player-facing action.
    pub prompt: String,
    /// Completion event, emitted once per reset.
    pub trigger: String,
    /// Availability, controlled by the game or designer.
    pub enabled: bool,
    /// Seconds of uninterrupted contact work required in total.
    pub seconds: f32,
    /// Keep partial work when the player releases the input or loses the target.
    pub retain_progress: bool,
    /// Eye-to-contact distance limit in metres.
    pub reach: f32,
    /// Contact point relative to this entity.
    pub anchor: Vec3,
    /// Runtime fraction; save systems may record this separately.
    pub progress: f32,
    /// Runtime hand contact blend.
    pub contact: f32,
    /// Whether the completion event has already fired.
    pub complete: bool,
    /// Hold from Details during Play; normal tool and reach checks still apply.
    pub preview_held: bool,
    /// Reset this target's work through Details.
    pub reset_requested: bool,
}
impl Component for WorkTarget {}
impl Default for WorkTarget {
    fn default() -> Self {
        Self {
            kind: 1,
            prompt: "Repair".into(),
            trigger: String::new(),
            enabled: true,
            seconds: 3.0,
            retain_progress: true,
            reach: 1.3,
            anchor: Vec3::ZERO,
            progress: 0.0,
            contact: 0.0,
            complete: false,
            preview_held: false,
            reset_requested: false,
        }
    }
}
impl WorkTarget {
    /// Reset runtime work without modifying authored settings.
    pub fn reset(&mut self) {
        self.progress = 0.0;
        self.contact = 0.0;
        self.complete = false;
        self.preview_held = false;
        self.reset_requested = false;
    }
    /// Advance fixed time. Returns true exactly once at completion. The caller
    /// owns aim, reach, visibility, input and tool eligibility as one predicate.
    pub fn tick(&mut self, dt: f32, eligible: bool) -> bool {
        if !dt.is_finite() || dt <= 0.0 {
            return false;
        }
        if self.reset_requested {
            self.reset();
        }
        let valid = self.enabled
            && !self.complete
            && self.kind <= 1
            && self.seconds.is_finite()
            && (0.25..=60.0).contains(&self.seconds);
        let working = eligible && valid;
        let dt = dt.min(0.1);
        self.contact = (self.contact + if working { dt / 0.25 } else { -dt / 0.2 }).clamp(0.0, 1.0);
        if !working {
            if !self.retain_progress && !self.complete {
                self.progress = 0.0;
            }
            return false;
        }
        if self.contact < 1.0 {
            return false;
        }
        self.progress = (self.progress + dt / self.seconds).min(1.0);
        if self.progress >= 1.0 {
            self.complete = true;
            self.preview_held = false;
            return true;
        }
        false
    }
}
/// Register one schema for native Details, scene persistence and automation.
pub fn register(registry: &mut TypeRegistry) {
    let runtime = FieldFlags::EDIT.union(FieldFlags::SCRIPT_READ);
    let request = runtime.union(FieldFlags::SCRIPT_WRITE);
    let mut schema = component_schema! {
        WorkTarget as "somnium.WorkTarget", display "Burn / Repair", version 1,
        fields {
            kind {group:"Work",min:0.0,max:1.0}, prompt {group:"Work"},
            trigger {group:"Work",doc:"Game event emitted once on completion."},
            enabled {group:"Work"}, seconds {group:"Work",unit:"s",min:0.25,max:60.0},
            retain_progress {group:"Work",doc:"Resume partial work after releasing, leaving reach or losing the required tool."},
            reach {group:"Contact",unit:"m",min:0.2,max:2.0}, anchor {group:"Contact",unit:"m"},
            progress {group:"Runtime",read_only:true,flags:runtime}, contact {group:"Runtime",read_only:true,flags:runtime}, complete {group:"Runtime",read_only:true,flags:runtime},
            preview_held {group:"Preview",flags:request,doc:"Hold while in Play and within reach; burn still needs an active flame."},
            reset_requested {group:"Preview",flags:request,doc:"Reset runtime work without changing settings."},
        }
    };
    schema
        .fields
        .iter_mut()
        .find(|f| f.name == "kind")
        .unwrap()
        .ty = FieldType::Enum(&["Burn", "Repair"]);
    registry.register(schema);
}
/// One frame's shared interaction result.
#[derive(Default)]
pub struct WorkFrame {
    /// Incomplete target under the reticle.
    pub target: Option<Entity>,
    /// Completion event sources. Usually at most one.
    pub completed: Vec<Entity>,
    /// Smoothed contact, plus true for the lighter hand and false for repair.
    pub hand: Option<(Vec3, f32, bool)>,
    /// World-space outward normal of the target owning `hand`, including while
    /// retracting after looking away. Zero when no hand contact is active.
    pub hand_normal: Vec3,
    /// Player-facing status and percentage.
    pub prompt: String,
}
/// Fixed-step work selection. Walls occlude targets and only the nearest aimed
/// target can advance; preview requests cannot bypass these rules.
pub fn update(
    ctx: &mut EngineContext,
    eye: Vec3,
    forward: Vec3,
    held: bool,
    flame: bool,
    hands_free: bool,
) -> WorkFrame {
    update_with_contact(ctx, eye, forward, held, flame, hands_free, |_, _, _| true)
}
/// Add rig-specific contact eligibility without coupling shared work to a
/// character skeleton. The callback receives kind, world anchor and normal;
/// callers can validate repair-hand reach separately from the flame socket.
pub fn update_with_contact(
    ctx: &mut EngineContext,
    eye: Vec3,
    forward: Vec3,
    held: bool,
    flame: bool,
    hands_free: bool,
    mut contact_eligible: impl FnMut(u32, Vec3, Vec3) -> bool,
) -> WorkFrame {
    let targets: Vec<_> = ctx
        .world
        .entities()
        .filter_map(|e| {
            let w = ctx.world.get::<WorkTarget>(e)?;
            let t = ctx.world.get::<Transform>(e)?;
            let matrix = ctx
                .world
                .get::<WorldTransform>(e)
                .map_or_else(|| t.to_matrix(), |t| t.0);
            if !matrix.is_finite() || matrix.determinant() == 0.0 {
                return None;
            }
            let normal = matrix
                .inverse()
                .transpose()
                .transform_vector3(Vec3::Z)
                .try_normalize()?;
            Some((e, w.clone(), matrix.transform_point3(w.anchor), normal))
        })
        .collect();
    let selected = targets
        .iter()
        .filter(|(_, w, at, normal)| {
            in_contact_reach(w, eye, forward, *at, *normal, &mut contact_eligible)
                && visible(ctx, eye, *at)
        })
        .min_by(|a, b| {
            a.2.distance_squared(eye)
                .total_cmp(&b.2.distance_squared(eye))
        })
        .map(|v| v.0);
    let returning = targets
        .iter()
        .any(|(e, w, _, _)| Some(*e) != selected && w.contact > 0.0);
    let mut frame = WorkFrame {
        target: selected,
        ..Default::default()
    };
    for (entity, config, at, normal) in targets {
        let selected = selected == Some(entity);
        let tool = config.kind != 0 || flame;
        let eligible =
            selected && !returning && hands_free && tool && (held || config.preview_held);
        if let Some(w) = ctx.world.get_mut::<WorkTarget>(entity) {
            if w.tick(ctx.simulation.fixed_delta_seconds, eligible) {
                frame.completed.push(entity);
            }
            if selected {
                frame.prompt = if w.complete {
                    format!("{} — complete", w.prompt)
                } else if !hands_free {
                    "Finish the current interaction first".into()
                } else if !tool {
                    "Ignite the lighter first".into()
                } else {
                    format!("Hold E — {} · {:.0}%", w.prompt, w.progress * 100.0)
                };
            }
            if w.contact > 0.0 && (selected || !eligible) && frame.hand.is_none() {
                // Retraction may finish after the cursor leaves the original target.
                frame.hand = Some((
                    at,
                    w.contact * w.contact * (3.0 - 2.0 * w.contact),
                    w.kind == 0,
                ));
                frame.hand_normal = normal;
            }
        }
    }
    frame
}
fn in_contact_reach(
    work: &WorkTarget,
    eye: Vec3,
    forward: Vec3,
    at: Vec3,
    normal: Vec3,
    contact_eligible: &mut impl FnMut(u32, Vec3, Vec3) -> bool,
) -> bool {
    let delta = at - eye;
    work.enabled
        && !work.complete
        && delta.length() <= work.reach
        && delta.normalize_or_zero().dot(forward) > 0.9
        && contact_eligible(work.kind, at, normal)
}
fn visible(ctx: &EngineContext, eye: Vec3, at: Vec3) -> bool {
    use somnium_physics::animation::CapsuleSweep;
    let displacement = at - eye;
    let distance = displacement.length();
    distance < 0.03
        || ctx
            .physics
            .cast_capsule(CapsuleSweep {
                position: eye,
                rotation: Quat::IDENTITY,
                displacement,
                half_height: 0.002,
                radius: 0.008,
                ignore_body: None,
            })
            .is_ok_and(|h| h.is_none_or(|h| h.fraction * distance >= distance - 0.12))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rig_contact_gate_prevents_remote_repair_and_can_distinguish_burning() {
        let mut work = WorkTarget::default();
        let eye = Vec3::new(0.0, 1.7, 0.0);
        let at = eye + Vec3::NEG_Z;
        let mut eligibility = |kind, _, _| kind == 0;
        let reachable = in_contact_reach(&work, eye, Vec3::NEG_Z, at, Vec3::Z, &mut eligibility);
        assert!(!reachable);
        for _ in 0..100 {
            assert!(!work.tick(0.1, reachable));
        }
        assert_eq!(work.progress, 0.0);
        assert_eq!(work.contact, 0.0);
        work.kind = 0;
        assert!(in_contact_reach(
            &work,
            eye,
            Vec3::NEG_Z,
            at,
            Vec3::Z,
            &mut eligibility
        ));
        work.complete = true;
        assert!(!in_contact_reach(
            &work,
            eye,
            Vec3::NEG_Z,
            at,
            Vec3::Z,
            &mut eligibility
        ));
    }
    #[test]
    fn repair_retains_progress_and_completes_only_once_after_contact() {
        let mut w = WorkTarget {
            seconds: 1.0,
            ..Default::default()
        };
        w.tick(0.1, true);
        assert_eq!(w.progress, 0.0);
        for _ in 0..6 {
            w.tick(0.1, true);
        }
        let saved = w.progress;
        assert!(saved > 0.0 && saved < 1.0);
        for _ in 0..20 {
            assert!(!w.tick(0.1, false));
        }
        assert_eq!(w.progress, saved);
        assert_eq!(w.contact, 0.0);
        let mut events = 0;
        for _ in 0..50 {
            events += usize::from(w.tick(0.1, true));
        }
        assert_eq!(events, 1);
        assert!(w.complete);
    }
    #[test]
    fn invalid_time_and_disabled_or_reset_work_cannot_commit() {
        let mut w = WorkTarget {
            seconds: 0.25,
            retain_progress: false,
            ..Default::default()
        };
        for dt in [f32::NAN, 0.0, -1.0] {
            assert!(!w.tick(dt, true));
        }
        for _ in 0..3 {
            w.tick(0.1, true);
        }
        assert!(w.progress > 0.0);
        w.tick(0.1, false);
        assert_eq!(w.progress, 0.0);
        w.enabled = false;
        for _ in 0..20 {
            assert!(!w.tick(0.1, true));
        }
        w.reset();
        assert!(!w.complete);
        assert_eq!(w.contact, 0.0);
    }
}
