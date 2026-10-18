//! `somnium.CastMember`: a character in the story's cast, performing what the story's
//! [`Director`](somnium_story::actor::Director) says.
//!
//! The game owns the story and its director; each frame it calls [`sync`] and the component's
//! read-only outputs (talking, jaw, lids, expression, gesture, mark, look) follow, for the Details
//! panel and scripts. A rig with an `Animation` component on the same entity also gets its Look At
//! aimed, when `drive_look` is on. Only what changed is written.
//!
//! Not `StoryActor`: scripts reach a component by the last part of its id, and The Somnus Fracture's
//! `tsf.StoryActor` already answers to `storyActor`.

use crate::Name;
use somnium_ecs::{Component, Entity, FieldFlags, TypeRegistry, World, component_schema};
use somnium_story::actor::{Director, Look};

/// A cast member on an entity: its settings, and the director's performance as read-only outputs.
#[derive(Clone, Debug, PartialEq)]
pub struct CastMemberComponent {
    /// The cast id in the story manifest.
    pub speaker: String,
    /// Aim the rig's Look At (an `Animation` component on this entity) where the director says.
    pub drive_look: bool,
    /// Look At weight while the actor looks at someone.
    pub look_weight: f32,
    /// Speaking a line now.
    pub talking: bool,
    /// How far the jaw is open, 0 to 1.
    pub jaw_open: f32,
    /// How far the lids are shut, 0 to 1.
    pub lids_shut: f32,
    /// The face's expression from the last line spoken (the portrait's names).
    pub expression: String,
    /// Whom the actor looks at: empty (ahead), `player`, a cast id or a mark.
    pub look_at: String,
    /// A gesture waiting to be played.
    pub gesture: String,
    /// A mark the scene sent the actor to.
    pub mark: String,
}

impl Component for CastMemberComponent {}

impl Default for CastMemberComponent {
    fn default() -> Self {
        Self {
            speaker: String::new(),
            drive_look: true,
            look_weight: 0.8,
            talking: false,
            jaw_open: 0.0,
            lids_shut: 0.0,
            expression: String::new(),
            look_at: String::new(),
            gesture: String::new(),
            mark: String::new(),
        }
    }
}

/// Register `somnium.CastMember`.
pub fn register(registry: &mut TypeRegistry) {
    let out = FieldFlags::EDIT.union(FieldFlags::SCRIPT_READ);
    registry.register(component_schema! {
        CastMemberComponent as "somnium.CastMember", display "Cast Member", version 1,
        fields {
            speaker { group: "Cast", doc: "The cast id in the story manifest (`cast[].id`)." },
            drive_look { group: "Look", doc: "Aim this entity's Animation Look At at whom the scene says." },
            look_weight { min: 0.0, max: 1.0, step: 0.05, group: "Look" },
            talking { group: "Performance", read_only: true, flags: out },
            jaw_open { group: "Performance", read_only: true, flags: out },
            lids_shut { group: "Performance", read_only: true, flags: out },
            expression { group: "Performance", read_only: true, flags: out },
            look_at { group: "Performance", read_only: true, flags: out },
            gesture { group: "Performance", read_only: true, flags: out },
            mark { group: "Performance", read_only: true, flags: out },
        }
    });
}

/// Copy the director's performance onto every story actor. `target` places the player and marks
/// (`Look::Player`, `Look::Mark`) for the Look At; other actors are found by their speaker.
pub fn sync(world: &mut World, director: &Director, target: impl Fn(&Look) -> Option<Entity>) {
    let actors: Vec<(Entity, String)> = world
        .iter_with::<CastMemberComponent>()
        .map(|(entity, actor)| (entity, actor.speaker.clone()))
        .collect();
    for (entity, speaker) in &actors {
        let Some(state) = director.state(speaker) else {
            continue;
        };
        let look_at = match &state.look {
            Look::Ahead => String::new(),
            Look::Player => "player".to_owned(),
            Look::Actor(id) | Look::Mark(id) => id.clone(),
        };
        let next = CastMemberComponent {
            talking: state.talking,
            jaw_open: state.jaw,
            lids_shut: state.lids,
            expression: state.expression.clone(),
            look_at,
            gesture: state.gesture.clone().unwrap_or_default(),
            mark: state.mark.clone().unwrap_or_default(),
            ..world
                .get::<CastMemberComponent>(*entity)
                .cloned()
                .unwrap_or_default()
        };
        let (drive, weight) = (next.drive_look, next.look_weight);
        if world.get::<CastMemberComponent>(*entity) != Some(&next) {
            if let Some(actor) = world.get_mut::<CastMemberComponent>(*entity) {
                *actor = next;
            }
        }
        if !drive {
            continue;
        }
        let aim = match &state.look {
            Look::Ahead => None,
            Look::Actor(id) => actors.iter().find(|(_, s)| s == id).map(|(e, _)| *e),
            look => target(look),
        };
        let (look_target, look_weight) = aim.map_or((Entity::DANGLING, 0.0), |e| (e, weight));
        let rig = world.get::<crate::animation_authoring::AnimationAuthoring>(*entity);
        if rig.is_some_and(|r| r.look_target != look_target || r.look_weight != look_weight) {
            if let Some(rig) =
                world.get_mut::<crate::animation_authoring::AnimationAuthoring>(*entity)
            {
                rig.look_target = look_target;
                rig.look_weight = look_weight;
            }
        }
    }
}

/// A mark by entity name: the usual `target` for [`sync`] when marks are named entities.
#[must_use]
pub fn named(world: &World, name: &str) -> Option<Entity> {
    world
        .iter_with::<Name>()
        .find(|(_, n)| n.as_str() == name)
        .map(|(e, _)| e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation_authoring::AnimationAuthoring;
    use somnium_ecs::ComponentId;
    use somnium_story::conversation::LineBeat;
    use somnium_story::manifest::Side;

    fn actor(world: &mut World, speaker: &str) -> Entity {
        world.spawn((
            CastMemberComponent {
                speaker: speaker.into(),
                ..CastMemberComponent::default()
            },
            AnimationAuthoring::default(),
        ))
    }

    #[test]
    fn actors_follow_the_director_and_quiet_frames_write_nothing() {
        let mut world = World::new();
        let oren = actor(&mut world, "oren");
        let wren = actor(&mut world, "wren");
        let player = world.spawn((Name::new("Player"),));
        let mut director = Director::new();
        director.add("oren");
        director.add("wren");
        director.line(&LineBeat {
            dialogue: "d".into(),
            node: "n".into(),
            speaker: "oren".into(),
            name: "Oren".into(),
            portrait: String::new(),
            expression: "tired".into(),
            side: Side::Right,
            color: None,
            text: "Three lamps have gone out.".into(),
            language: String::new(),
            voice: String::new(),
            tags: vec!["gesture:point".into()],
            seen: false,
            ambient: false,
            menu_next: false,
        });
        director.tick(100);
        let place = |look: &Look| (*look == Look::Player).then_some(player);
        sync(&mut world, &director, place);
        let o = world.get::<CastMemberComponent>(oren).unwrap();
        assert!(o.talking && o.jaw_open > 0.0 && o.expression == "tired" && o.gesture == "point");
        assert_eq!(o.look_at, "player");
        assert_eq!(
            world.get::<AnimationAuthoring>(oren).unwrap().look_target,
            player
        );
        assert_eq!(
            world.get::<CastMemberComponent>(wren).unwrap().look_at,
            "oren"
        );
        assert_eq!(
            world.get::<AnimationAuthoring>(wren).unwrap().look_target,
            oren
        );
        assert!((world.get::<AnimationAuthoring>(wren).unwrap().look_weight - 0.8).abs() < 1e-6);

        director.end();
        director.tick(1000);
        sync(&mut world, &director, place);
        let ids = [
            ComponentId::of::<CastMemberComponent>(),
            ComponentId::of::<AnimationAuthoring>(),
        ];
        let settled = world.change_signature(&ids);
        // Lids may blink between frames; hold the clock still to prove an idle sync writes nothing.
        sync(&mut world, &director, place);
        assert_eq!(world.change_signature(&ids), settled);
        assert_eq!(
            world.get::<AnimationAuthoring>(oren).unwrap().look_weight,
            0.0
        );
    }
}
