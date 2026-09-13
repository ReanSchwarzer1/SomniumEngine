//! Designer-authored interaction intent and interruption-safe hand timing.
//! Games consume the single commit event to perform their own action.
use glam::Vec3;
use somnium_ecs::{Component, component_schema, reflect::TypeRegistry};

#[derive(Clone, Debug, PartialEq)]
/// Authored reach, prompt and action settings shared by native Details and games.
pub struct Interactable {
    /// 0 door, 1 collect, 2 inspect, 3 operate, 4 equip light.
    pub kind: u32,
    /// Verb displayed when the object can be targeted.
    pub prompt: String,
    /// Game-defined event emitted at contact.
    pub trigger: String,
    /// Whether the object may be targeted.
    pub enabled: bool,
    /// Show the prompt but reject the action.
    pub locked: bool,
    /// Maximum eye-to-contact distance in metres.
    pub reach: f32,
    /// Reach and return timing in seconds.
    pub duration: f32,
    /// Contact point in the entity local frame.
    pub anchor: Vec3,
    /// Signed hinge travel in degrees for a door.
    pub open_angle: f32,
    /// One-shot native preview request, consumed by the game.
    pub preview_requested: bool,
}
impl Component for Interactable {}
impl Default for Interactable {
    fn default() -> Self {
        Self {
            kind: 3,
            prompt: "Operate".into(),
            trigger: String::new(),
            enabled: true,
            locked: false,
            reach: 2.0,
            duration: 0.65,
            anchor: Vec3::ZERO,
            open_angle: 90.0,
            preview_requested: false,
        }
    }
}
/// Register editable fields for persistence, Details and authoring.
pub fn register_schema(registry: &mut TypeRegistry) {
    let mut schema = component_schema! {
        Interactable as "somnium.Interactable", display "Interaction", version 1,
        fields {
            kind { group:"Action", min:0.0,max:4.0,doc:"0 Door, 1 Collect, 2 Inspect, 3 Operate, 4 Equip light." },
            prompt { group:"Action",doc:"Player-facing verb shown while looking at this object." },
            trigger { group:"Action",doc:"Game event emitted once at hand contact." },
            enabled { group:"Availability" }, locked { group:"Availability" },
            reach { group:"Contact",min:0.2,max:4.0,unit:"m" },
            duration { group:"Contact",min:0.2,max:3.0,unit:"s" },
            anchor { group:"Contact",unit:"m",doc:"Contact position relative to the entity." },
            open_angle { group:"Door",min:-170.0,max:170.0,unit:"deg" },
            preview_requested { group:"Preview",doc:"Request the same interaction during Play." },
        }
    };
    if let Some(field) = schema.fields.iter_mut().find(|field| field.name == "kind") {
        field.ty = somnium_ecs::reflect::FieldType::Enum(&[
            "Door",
            "Collect",
            "Inspect",
            "Operate",
            "Equip Light",
        ]);
    }
    registry.register(schema);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Events emitted once by the interaction clock.
pub enum InteractionEvent {
    /// Perform the game action at hand contact.
    Contact,
    /// Release the hand and restore any held inspection object.
    Returned,
}
#[derive(Clone, Debug, Default)]
/// A single hand interaction with smooth, idempotent cancellation.
pub struct InteractionSession {
    time: f32,
    duration: f32,
    committed: bool,
    returning: bool,
    return_weight: f32,
    /// The contact pose is held until cancellation.
    pub holding: bool,
    active: bool,
}
impl InteractionSession {
    /// Begin an available action; reject overlapping or invalid requests.
    pub fn begin(&mut self, config: &Interactable) -> Result<(), &'static str> {
        if self.active {
            return Err("Hands are busy");
        }
        if !config.enabled || config.locked {
            return Err("Unavailable or locked");
        }
        if config.kind > 4
            || !config.duration.is_finite()
            || !(0.2..=3.0).contains(&config.duration)
        {
            return Err("Invalid interaction settings");
        }
        *self = Self {
            duration: config.duration,
            active: true,
            ..Self::default()
        };
        Ok(())
    }
    /// Whether the hand is reaching, holding or returning.
    pub fn busy(&self) -> bool {
        self.active
    }
    /// Smooth weight applied to the contact pose.
    pub fn contact_weight(&self) -> f32 {
        if !self.active {
            return 0.0;
        }
        let x = if self.returning {
            1.0 - self.time / self.duration
        } else {
            (self.time / (self.duration * 0.5)).min(1.0)
        }
        .clamp(0.0, 1.0);
        let smooth = x * x * (3.0 - 2.0 * x);
        if self.returning {
            self.return_weight * smooth
        } else {
            smooth
        }
    }
    /// Return from the current weight without repeating or committing the action.
    pub fn cancel(&mut self) {
        if self.active && !self.returning {
            let weight = self.contact_weight();
            self.returning = true;
            self.holding = false;
            self.return_weight = weight;
            self.time = 0.0;
        }
    }
    /// Advance simulation time; emit at most one transition per call.
    pub fn tick(&mut self, dt: f32, hold_at_contact: bool) -> Option<InteractionEvent> {
        if !self.active || !dt.is_finite() || dt <= 0.0 || self.holding {
            return None;
        }
        self.time += dt.min(0.1);
        if self.returning {
            if self.time >= self.duration {
                *self = Self::default();
                return Some(InteractionEvent::Returned);
            }
        } else if !self.committed && self.time >= self.duration * 0.5 {
            self.committed = true;
            self.holding = hold_at_contact;
            if !hold_at_contact {
                self.returning = true;
                self.return_weight = 1.0;
                self.time = 0.0;
            }
            return Some(InteractionEvent::Contact);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_reach_never_commits_and_releases_hands() {
        let mut session = InteractionSession::default();
        session.begin(&Interactable::default()).unwrap();
        session.tick(0.1, false);
        session.cancel();
        let mut contacts = 0;
        for _ in 0..40 {
            contacts += usize::from(session.tick(0.05, false) == Some(InteractionEvent::Contact));
        }
        assert_eq!(contacts, 0);
        assert!(!session.busy());
        assert_eq!(session.contact_weight(), 0.0);
    }
    #[test]
    fn repeated_focus_loss_cancel_is_continuous_and_finishes() {
        let mut session = InteractionSession::default();
        session.begin(&Interactable::default()).unwrap();
        session.tick(0.1, false);
        let weight = session.contact_weight();
        session.cancel();
        assert!((session.contact_weight() - weight).abs() < 1e-6);
        for _ in 0..100 {
            session.cancel();
            assert_ne!(session.tick(0.016, false), Some(InteractionEvent::Contact));
        }
        assert!(!session.busy());
    }
    #[test]
    fn held_inspection_commits_once_and_returns_on_cancel() {
        let mut session = InteractionSession::default();
        session.begin(&Interactable::default()).unwrap();
        let mut contacts = 0;
        for _ in 0..100 {
            contacts += usize::from(session.tick(0.05, true) == Some(InteractionEvent::Contact));
        }
        assert_eq!(contacts, 1);
        assert!(session.holding);
        assert!(session.begin(&Interactable::default()).is_err());
        session.cancel();
        for _ in 0..40 {
            session.tick(0.05, true);
        }
        assert!(!session.busy());
    }
    #[test]
    fn invalid_and_locked_actions_do_not_acquire_hands() {
        let mut session = InteractionSession::default();
        let mut config = Interactable::default();
        config.locked = true;
        assert!(session.begin(&config).is_err());
        config.locked = false;
        config.duration = f32::NAN;
        assert!(session.begin(&config).is_err());
        assert!(!session.busy());
    }
}
