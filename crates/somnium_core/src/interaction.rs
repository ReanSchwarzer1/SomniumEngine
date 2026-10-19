//! Designer-authored interaction intent and interruption-safe hand timing.
//! Games consume the single commit event to perform their own action.
use glam::Vec3;
use somnium_ecs::{Component, component_schema, reflect::TypeRegistry};

/// A world-space anatomical contact, consumed by a game's hand IK layer.
#[derive(Clone, Copy, Debug)]
pub struct HandContact {
    /// Surface position, not the wrist position.
    pub position: Vec3,
    /// Surface normal pointing away from the object.
    pub normal: Vec3,
    /// Reach/return blend from the interaction owner.
    pub weight: f32,
    /// Finger closure: zero is an open palm.
    pub grip: f32,
}

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
    /// Outward normal in the entity frame; zero derives it from the approach.
    pub contact_normal: Vec3,
    /// Finger closure at contact: -1 uses action defaults, 0 open, 1 closed.
    pub hand_grip: f32,
    /// Signed hinge travel in degrees for a door.
    pub open_angle: f32,
    /// One-shot native preview request, consumed by the game.
    pub preview_requested: bool,
    // Added for the isometric layer (`crate::interact`, `crate::door`; Rimeborn, 19 Oct 2026). Nothing that existed
    // before reads them, so TSF's hand interactions behave as they did.
    /// Half the angle, degrees, either side of the player's facing it can be picked in (isometric targeting).
    pub cone: f32,
    /// A world-state condition the game evaluates; while false the action is refused. Empty: only `locked` locks.
    pub requires: String,
    /// A door's leaf, metres: along the entity's +X from the hinge (its origin) to the free edge.
    pub leaf_width: f32,
    /// A door's leaf, up from the hinge.
    pub leaf_height: f32,
    /// A door's leaf, through it (for its box).
    pub leaf_thickness: f32,
    /// A static box fills a closed door's leaf.
    pub blocks: bool,
    /// Entities under a door's parent whose names start with this swing with it (a building's `Door / ` parts).
    pub leaves: String,
    /// Seconds a door takes to swing fully open or shut.
    pub swing_duration: f32,
    /// Seconds a door stays open before closing by itself, once nobody is in its swing; 0 stays open.
    pub close_after: f32,
    /// A door is open: how it starts, and its state while the door system runs.
    pub open: bool,
    /// An operated thing (a lever, a gate) swings its children and `leaves` by `open_angle` about its origin, as a
    /// door does, but blocks nothing. Doors always swing.
    pub swings: bool,
    /// A world-state condition; while it holds the thing is gone (taken): removed as it appears. A take removes it
    /// at once. Empty: only the take removes it, and it comes back with its cell.
    pub gone_when: String,
    /// The prompt while refused (locked or `requires` false). Empty says "Locked".
    pub refused_prompt: String,
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
            contact_normal: Vec3::ZERO,
            hand_grip: -1.0,
            open_angle: 90.0,
            preview_requested: false,
            cone: 75.0,
            requires: String::new(),
            leaf_width: 0.9,
            leaf_height: 2.0,
            leaf_thickness: 0.06,
            blocks: true,
            leaves: String::new(),
            swing_duration: 0.9,
            close_after: 0.0,
            open: false,
            swings: false,
            gone_when: String::new(),
            refused_prompt: String::new(),
        }
    }
}
/// Register editable fields for persistence, Details and authoring.
pub fn register_schema(registry: &mut TypeRegistry) {
    let mut schema = component_schema! {
        Interactable as "somnium.Interactable", display "Interaction", version 1,
        fields {
            kind { group:"Action", min:0.0,max:6.0,doc:"0 Door, 1 Collect, 2 Inspect, 3 Operate, 4 Equip light, 5 Talk, 6 Sit." },
            prompt { group:"Action",doc:"Player-facing verb shown while looking at this object." },
            trigger { group:"Action",doc:"Game event emitted once at hand contact." },
            enabled { group:"Availability" }, locked { group:"Availability" },
            reach { group:"Contact",min:0.2,max:4.0,unit:"m" },
            duration { group:"Contact",min:0.2,max:3.0,unit:"s" },
            anchor { group:"Contact",unit:"m",doc:"Contact position relative to the entity." },
            contact_normal { group:"Contact",doc:"Local outward surface normal. Zero uses the player's approach; set for angled surfaces and handles." },
            hand_grip { group:"Contact",min:-1.0,max:1.0,doc:"-1 uses the action default; 0 is a flat palm, 1 a closed grip." },
            open_angle { group:"Door",min:-170.0,max:170.0,unit:"deg" },
            preview_requested { group:"Preview",doc:"Request the same interaction during Play." },
            cone { group:"Isometric",min:5.0,max:180.0,unit:"deg",doc:"Either side of the player's facing it can be picked in by the isometric layer. 180: any direction." },
            requires { group:"Availability",doc:"A world-state condition (the game's: a story fact or expression). While false the action is refused and the prompt says Locked. Empty: only Locked locks." },
            leaf_width { group:"Door",min:0.1,max:6.0,unit:"m",doc:"The leaf along +X from the hinge (this entity's origin) to its free edge." },
            leaf_height { group:"Door",min:0.1,max:8.0,unit:"m" },
            leaf_thickness { group:"Door",min:0.01,max:0.5,unit:"m" },
            blocks { group:"Door",doc:"A static box fills the leaf while the door is closed." },
            leaves { group:"Door",doc:"Entities under this door's parent whose names start with this swing with it (a building's 'Door / ' parts). Its children always swing." },
            swing_duration { group:"Door",min:0.1,max:5.0,unit:"s" },
            close_after { group:"Door",min:0.0,max:60.0,unit:"s",doc:"Closes by itself this long after opening, once nobody is in its swing. 0 stays open." },
            open { group:"Door",doc:"Open now: how it starts, and its state while playing." },
            swings { group:"Door",doc:"An operated thing (a lever, a gate) swings like a door when used, blocking nothing. Doors always swing." },
            gone_when { group:"Availability",doc:"A world-state condition; while it holds the thing is gone (taken) and is removed as it appears. A take removes it at once." },
            refused_prompt { group:"Availability",doc:"The prompt while refused. Empty says Locked." },
        }
    };
    if let Some(field) = schema.fields.iter_mut().find(|field| field.name == "kind") {
        field.ty = somnium_ecs::reflect::FieldType::Enum(&[
            "Door",
            "Collect",
            "Inspect",
            "Operate",
            "Equip Light",
            "Talk",
            "Sit",
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
        if config.kind > 6
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
