//! Authored combat data on entities, with schemas so Details edits it, scenes
//! save it and scripts can read it.
//!
//! The host turns these into a [`crate::CombatWorld`] when play starts
//! ([`CombatantComponent::desc`], [`CombatSurfaceComponent`] painted over the
//! entity's footprint) and writes the live readouts back each frame (only when
//! they change, so caches keyed on the component stay warm).
use crate::ai::{Archetype, BrainDesc};
use crate::environment::Surface;
use crate::schools::{SwordSchool, Swordsmanship};
use crate::spell::{CastSpeed, Discipline, Magic};
use crate::weapon::WeaponKind;
use crate::world::{Body, CombatantDesc};
use crate::{Rank, Resistances, TeamId, Vitals};
use glam::Vec3;
use somnium_ecs::Component;
use somnium_ecs::reflect::{FieldFlags, FieldType, TypeRegistry};

/// A fighter.
#[derive(Clone, Debug, PartialEq)]
pub struct CombatantComponent {
    /// Joins the fight in Play.
    pub enabled: bool,
    /// Team number (0 is the protagonist's party).
    pub team: u32,
    /// Driven by the player's mouse and keyboard rather than AI.
    pub player: bool,
    /// AI style, by [`Archetype::ALL`] index.
    pub archetype: u32,
    /// Weapon, by [`WeaponKind::ALL`] index.
    pub weapon: u32,
    /// Sword school: 0 none, then [`SwordSchool::ALL`] order.
    pub school: u32,
    /// Forms of the school learned, in teaching order.
    pub forms: u32,
    /// Sword and magic rank, by [`Rank::ALL`] index.
    pub rank: u32,
    /// Hit points.
    pub max_health: f32,
    /// Stamina.
    pub max_stamina: f32,
    /// Capacity in lamps (maximum mana).
    pub capacity: f32,
    /// Poise.
    pub poise: f32,
    /// Physical armour.
    pub armor: f32,
    /// Walking speed, metres per second.
    pub speed: f32,
    /// Spell ids on the hotbar (or in the AI's book), comma-separated.
    pub spells: String,
    /// Knows the base-twelve numbers (measures take exact values).
    pub numbers: bool,
    /// Live: health.
    pub health: f32,
    /// Live: stamina.
    pub stamina: f32,
    /// Live: mana.
    pub mana: f32,
    /// Live: what it is doing.
    pub state: String,
}

impl Component for CombatantComponent {}

impl Default for CombatantComponent {
    fn default() -> Self {
        Self {
            enabled: true,
            team: 1,
            player: false,
            archetype: 0,
            weapon: 0,
            school: 0,
            forms: 0,
            rank: 1,
            max_health: 100.0,
            max_stamina: 100.0,
            capacity: 5.0,
            poise: 40.0,
            armor: 0.0,
            speed: 3.6,
            spells: String::new(),
            numbers: false,
            health: 0.0,
            stamina: 0.0,
            mana: 0.0,
            state: String::new(),
        }
    }
}

impl CombatantComponent {
    /// The protagonist: a sword, magic, the player's hands.
    #[must_use]
    pub fn protagonist() -> Self {
        Self {
            team: 0,
            player: true,
            archetype: Archetype::Companion as u32,
            school: 2,
            forms: 4,
            rank: Rank::Advanced as u32,
            max_health: 160.0,
            max_stamina: 120.0,
            capacity: 600.0,
            poise: 50.0,
            speed: 4.0,
            spells: "ice_lance,ember_bolt,rime_bloom,ice_wall,frost_step,gust,water_bolt,close_wound,shield,fireburst".into(),
            numbers: true,
            ..Self::default()
        }
    }

    /// The archetype.
    #[must_use]
    pub fn archetype(&self) -> Archetype {
        Archetype::ALL[(self.archetype as usize).min(Archetype::ALL.len() - 1)]
    }

    /// The weapon kind.
    #[must_use]
    pub fn weapon_kind(&self) -> WeaponKind {
        WeaponKind::ALL[(self.weapon as usize).min(WeaponKind::ALL.len() - 1)]
    }

    /// The rank.
    #[must_use]
    pub fn rank(&self) -> Rank {
        Rank::ALL[(self.rank as usize).min(Rank::ALL.len() - 1)]
    }

    /// The spell ids.
    pub fn spell_ids(&self) -> impl Iterator<Item = &str> {
        self.spells
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }

    /// Everything the world needs to create this fighter.
    #[must_use]
    pub fn desc(&self, name: &str, position: Vec3, facing: Vec3, host: u64) -> CombatantDesc {
        let rank = self.rank();
        let mut desc = CombatantDesc::new(
            name,
            TeamId(self.team as u16),
            position,
            self.weapon_kind().library(),
        );
        desc.facing = facing;
        desc.host = host;
        desc.body = Body {
            speed: self.speed.max(0.5),
            ..Body::default()
        };
        desc.vitals = Vitals::new(
            self.max_health.max(1.0),
            self.max_stamina.max(1.0),
            self.capacity.max(0.0),
            self.poise.max(1.0),
        );
        desc.resist = Resistances::default().with_armor(self.armor.max(0.0));
        if self.school > 0 {
            let school = SwordSchool::ALL[((self.school - 1) as usize).min(3)];
            desc.swordsmanship = Some(Swordsmanship::with_forms(school, self.forms as usize, rank));
        }
        let library = crate::spell::library::all();
        let mut magic = Magic {
            numbers: self.numbers,
            speeds: CastSpeed::Incantation.bit() | CastSpeed::ShortChant.bit(),
            allow_overdraw: self.player,
            ..Magic::default()
        };
        if self.player {
            magic.speeds |= CastSpeed::Silent.bit();
        }
        let ids: Vec<&str> = self.spell_ids().collect();
        for (slot, id) in ids.iter().take(10).enumerate() {
            if let Some(asset) = library.iter().find(|s| s.id == *id) {
                let d = asset.element.discipline();
                let have = magic.rank(d).unwrap_or(Rank::Novice);
                magic = magic.with_rank(d, have.max(rank).max(asset.rank));
                magic.hotbar[slot] = Some(asset.slot());
            }
        }
        // The protagonist reads the machine's grammar: every discipline he
        // casts in at his rank at least.
        if self.player {
            for d in [Discipline::Rime, Discipline::Flame, Discipline::Gale] {
                let have = magic.rank(d).unwrap_or(Rank::Novice);
                magic = magic.with_rank(d, have.max(rank));
            }
        }
        desc.magic = magic;
        if !self.player {
            let mut brain = BrainDesc::new(self.archetype());
            let mut profile = brain.archetype.profile();
            if !ids.is_empty() {
                profile.spells = ids.iter().map(|s| (*s).to_string()).collect();
            }
            brain.profile = Some(profile);
            brain.home = Some(position);
            desc.brain = Some(brain);
        }
        desc
    }
}

/// Reactive ground under the entity's footprint (its scale on X and Z).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatSurfaceComponent {
    /// Surface, by [`Surface::ALL`] index.
    pub surface: u32,
    /// A disc rather than a box.
    pub round: bool,
}

impl Component for CombatSurfaceComponent {}

impl Default for CombatSurfaceComponent {
    fn default() -> Self {
        Self {
            surface: Surface::Grass as u32,
            round: false,
        }
    }
}

impl CombatSurfaceComponent {
    /// The surface.
    #[must_use]
    pub fn surface(&self) -> Surface {
        Surface::ALL[(self.surface as usize).min(Surface::ALL.len() - 1)]
    }
}

/// Moving air: carries fire, bends arrows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindVolumeComponent {
    /// Direction the wind blows (planar).
    pub direction: Vec3,
    /// Speed at the centre, metres per second.
    pub strength: f32,
    /// Radius of influence.
    pub radius: f32,
}

impl Component for WindVolumeComponent {}

impl Default for WindVolumeComponent {
    fn default() -> Self {
        Self {
            direction: Vec3::X,
            strength: 6.0,
            radius: 15.0,
        }
    }
}

const LIVE: FieldFlags = FieldFlags::EDIT.union(FieldFlags::SCRIPT_READ);

/// Register the schemas.
pub fn register(registry: &mut TypeRegistry) {
    let mut combatant = somnium_ecs::component_schema! {
        CombatantComponent as "somnium.Combatant", display "Combatant", version 1,
        fields {
            enabled { group: "Combat" },
            player { group: "Combat", display_name: "Player Controlled", doc: "Mouse and keyboard: WASD, click to strike, right click to guard, 1-0 for spells." },
            team { group: "Combat", max: 64.0 },
            archetype { group: "Combat", display_name: "AI Style" },
            weapon { group: "Combat" },
            school { group: "Sword", display_name: "Sword School" },
            forms { group: "Sword", max: 5.0, display_name: "Forms Learned" },
            rank { group: "Sword" },
            max_health { group: "Body", min: 1.0, display_name: "Health" },
            max_stamina { group: "Body", min: 1.0, display_name: "Stamina" },
            capacity { group: "Body", min: 0.0, unit: "lamps", doc: "Maximum mana. A commoner holds about 5, a trained mage 200, a Saint 2,000." },
            poise { group: "Body", min: 1.0 },
            armor { group: "Body", min: 0.0 },
            speed { group: "Body", min: 0.5, max: 12.0, unit: "m/s" },
            spells { group: "Magic", doc: "Spell ids, comma-separated: the hotbar for the player, the book for AI." },
            numbers { group: "Magic", display_name: "Knows Numbers", doc: "Measures take exact values instead of rote steps." },
            health { group: "Live", read_only: true, flags: LIVE },
            stamina { group: "Live", read_only: true, flags: LIVE },
            mana { group: "Live", read_only: true, flags: LIVE },
            state { group: "Live", read_only: true, flags: LIVE },
        }
    };
    for field in &mut combatant.fields {
        field.ty = match field.name {
            "archetype" => FieldType::Enum(&Archetype::NAMES),
            "weapon" => FieldType::Enum(&WeaponKind::NAMES),
            "school" => {
                FieldType::Enum(&["None", "Aetherlance", "Meridian", "Night Fang", "Gale Edge"])
            }
            "rank" => FieldType::Enum(&[
                "Novice",
                "Adept",
                "Advanced",
                "Saint",
                "King",
                "Emperor",
                "Sovereign",
            ]),
            _ => continue,
        };
    }
    registry.register(combatant);
    let mut surface = somnium_ecs::component_schema! {
        CombatSurfaceComponent as "somnium.CombatSurface", display "Combat Surface", version 1,
        fields {
            surface { doc: "What the ground under this entity's footprint is: grass burns, water freezes." },
            round { doc: "A disc of the footprint's width instead of a box." },
        }
    };
    if let Some(field) = surface.fields.iter_mut().find(|f| f.name == "surface") {
        field.ty = FieldType::Enum(&Surface::NAMES);
    }
    registry.register(surface);
    registry.register(somnium_ecs::component_schema! {
        WindVolumeComponent as "somnium.WindVolume", display "Wind Volume", version 1,
        fields {
            direction { doc: "Which way the air moves." },
            strength { min: 0.0, max: 40.0, unit: "m/s" },
            radius { min: 0.5, unit: "m" },
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use somnium_ecs::reflect::StableId;

    #[test]
    fn schemas_register_with_enums_and_live_fields_unsaved() {
        let mut registry = TypeRegistry::new();
        register(&mut registry);
        let schema = registry
            .by_stable_id(StableId::new("somnium.Combatant"))
            .unwrap();
        let weapon = schema.field_by_name("weapon").unwrap();
        assert!(
            matches!(weapon.ty, FieldType::Enum(names) if names.len() == WeaponKind::ALL.len())
        );
        let health = schema.field_by_name("health").unwrap();
        assert!(
            !health.flags.contains(FieldFlags::SERIALIZE),
            "live readouts are not saved"
        );
        assert!(
            schema
                .field_by_name("max_health")
                .unwrap()
                .flags
                .contains(FieldFlags::SERIALIZE)
        );
        assert!(
            registry
                .by_stable_id(StableId::new("somnium.CombatSurface"))
                .is_some()
        );
        assert!(
            registry
                .by_stable_id(StableId::new("somnium.WindVolume"))
                .is_some()
        );
    }

    #[test]
    fn the_protagonist_has_his_hotbar_and_school() {
        let me = CombatantComponent::protagonist();
        let desc = me.desc("Protagonist", Vec3::ZERO, Vec3::Z, 7);
        assert!(desc.brain.is_none(), "the player has no AI");
        assert_eq!(desc.host, 7);
        let sword = desc.swordsmanship.unwrap();
        assert_eq!(sword.school, SwordSchool::Meridian);
        assert!(sword.knows(crate::schools::Form::Noon));
        assert!(desc.magic.hotbar.iter().all(Option::is_some));
        assert!(desc.magic.numbers);
        assert!(desc.magic.rank(Discipline::Rime).unwrap() >= Rank::Advanced);
        let raider = CombatantComponent {
            archetype: Archetype::Archer as u32,
            weapon: WeaponKind::Bow as u32,
            ..CombatantComponent::default()
        };
        let desc = raider.desc("Raider", Vec3::ZERO, Vec3::Z, 8);
        assert_eq!(desc.brain.unwrap().archetype, Archetype::Archer);
        assert_eq!(desc.weapon.kind, WeaponKind::Bow);
    }
}
