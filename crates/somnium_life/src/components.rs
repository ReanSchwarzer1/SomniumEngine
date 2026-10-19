//! Authored life on entities, with schemas so Details edits it and scenes save
//! it.
//!
//! A settlement scene marks a root with **Life Place** and its doors, greens,
//! fields and pens with **Life Spot** (the entity's position; its scale on X
//! and Z sizes an open spot). With Populate on, the host houses a household in
//! every home itself; **Villager**, **Livestock** and **Cart** place people,
//! beasts and carts by hand (a villager keeps to the nearest place, its home
//! the nearest home; a beast to the nearest pen; a cart to the nearest road).

use crate::place::SpotKind;
use crate::routine::Role;
use crate::sim::BeastKind;
use somnium_ecs::Component;
use somnium_ecs::reflect::{FieldFlags, FieldType, TypeRegistry};

/// A settlement (or a quarter): its spots are those under it.
#[derive(Clone, Debug, PartialEq)]
pub struct LifePlaceComponent {
    /// Lives in Play.
    pub enabled: bool,
    /// House a household in every home on its own.
    pub populate: bool,
    /// Seed for who lives where.
    pub seed: u32,
    /// Live: people, beasts and carts in it.
    pub population: u32,
}

impl Component for LifePlaceComponent {}

impl Default for LifePlaceComponent {
    fn default() -> Self {
        Self {
            enabled: true,
            populate: true,
            seed: 1,
            population: 0,
        }
    }
}

/// A door, a green, a field, a pen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LifeSpotComponent {
    /// What for, by [`SpotKind::ALL`] index.
    pub kind: u32,
}

impl Component for LifeSpotComponent {}

impl LifeSpotComponent {
    /// The kind.
    #[must_use]
    pub fn kind(&self) -> SpotKind {
        SpotKind::ALL[(self.kind as usize).min(SpotKind::ALL.len() - 1)]
    }
}

/// A villager placed by hand.
#[derive(Clone, Debug, PartialEq)]
pub struct VillagerComponent {
    /// Lives in Play.
    pub enabled: bool,
    /// Trade, by [`Role::ALL`] index.
    pub role: u32,
    /// Live: what it is about.
    pub doing: String,
}

impl Component for VillagerComponent {}

impl Default for VillagerComponent {
    fn default() -> Self {
        Self {
            enabled: true,
            role: 0,
            doing: String::new(),
        }
    }
}

impl VillagerComponent {
    /// The role.
    #[must_use]
    pub fn role(&self) -> Role {
        Role::ALL[(self.role as usize).min(Role::ALL.len() - 1)]
    }
}

/// A beast placed by hand.
#[derive(Clone, Debug, PartialEq)]
pub struct LivestockComponent {
    /// Lives in Play.
    pub enabled: bool,
    /// Kind, by [`BeastKind::ALL`] index.
    pub kind: u32,
}

impl Component for LivestockComponent {}

impl Default for LivestockComponent {
    fn default() -> Self {
        Self {
            enabled: true,
            kind: 0,
        }
    }
}

impl LivestockComponent {
    /// The kind.
    #[must_use]
    pub fn kind(&self) -> BeastKind {
        BeastKind::ALL[(self.kind as usize).min(BeastKind::ALL.len() - 1)]
    }
}

/// A cart placed by hand.
#[derive(Clone, Debug, PartialEq)]
pub struct CartComponent {
    /// Lives in Play.
    pub enabled: bool,
}

impl Component for CartComponent {}

impl Default for CartComponent {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// A realm's places from a file ([`crate::LifeTable`]): each town comes to
/// life as the view comes within reach of it, and lives on, cheaply, after.
#[derive(Clone, Debug, PartialEq)]
pub struct LifeTableComponent {
    /// Lives in Play.
    pub enabled: bool,
    /// The table, relative to the project.
    pub file: String,
    /// How near the view comes to a town's edge before its people are housed,
    /// metres (a streamed world's cell reach and its margin).
    pub reach: f32,
    /// Live: towns housed so far.
    pub adopted: u32,
}

impl Component for LifeTableComponent {}

impl Default for LifeTableComponent {
    fn default() -> Self {
        Self {
            enabled: true,
            file: String::new(),
            reach: 700.0,
            adopted: 0,
        }
    }
}

const LIVE: FieldFlags = FieldFlags::EDIT.union(FieldFlags::SCRIPT_READ);

/// Register the schemas.
pub fn register(registry: &mut TypeRegistry) {
    registry.register(somnium_ecs::component_schema! {
        LifePlaceComponent as "somnium.LifePlace", display "Life Place", version 1,
        fields {
            enabled { group: "Life" },
            populate { group: "Life", doc: "House a household in every home under this place, with a trade for each by its workplaces, livestock in its pens and carts on its roads." },
            seed { group: "Life", doc: "Who lives where." },
            population { group: "Live", read_only: true, flags: LIVE },
        }
    });
    let mut spot = somnium_ecs::component_schema! {
        LifeSpotComponent as "somnium.LifeSpot", display "Life Spot", version 1,
        fields {
            kind { doc: "A door (home, work, inn, chapel: people go inside) or an open spot (market, field, pen, green, well: its scale on X and Z is its size)." },
        }
    };
    if let Some(field) = spot.fields.iter_mut().find(|f| f.name == "kind") {
        field.ty = FieldType::Enum(&SpotKind::NAMES);
    }
    registry.register(spot);
    let mut villager = somnium_ecs::component_schema! {
        VillagerComponent as "somnium.Villager", display "Villager", version 1,
        fields {
            enabled { group: "Life" },
            role { group: "Life", doc: "Trade: the day it keeps." },
            doing { group: "Live", read_only: true, flags: LIVE },
        }
    };
    if let Some(field) = villager.fields.iter_mut().find(|f| f.name == "role") {
        field.ty = FieldType::Enum(&Role::NAMES);
    }
    registry.register(villager);
    let mut beast = somnium_ecs::component_schema! {
        LivestockComponent as "somnium.Livestock", display "Livestock", version 1,
        fields {
            enabled { group: "Life" },
            kind { group: "Life" },
        }
    };
    if let Some(field) = beast.fields.iter_mut().find(|f| f.name == "kind") {
        field.ty = FieldType::Enum(&BeastKind::NAMES);
    }
    registry.register(beast);
    registry.register(somnium_ecs::component_schema! {
        CartComponent as "somnium.Cart", display "Cart", version 1,
        fields {
            enabled { group: "Life", doc: "Drives the nearest road (a Spline) by day." },
        }
    });
    registry.register(somnium_ecs::component_schema! {
        LifeTableComponent as "somnium.LifeTable", display "Life Table", version 1,
        fields {
            enabled { group: "Life" },
            file { group: "Life", doc: "A realm's places (spots, lanes, roads), relative to the project: a game's tool writes it from its settlements." },
            reach { group: "Life", min: 0.0, unit: "m", doc: "How near the view comes to a town's edge before its people are housed; they live on, cheaply, after." },
            adopted { group: "Live", read_only: true, flags: LIVE },
        }
    });
}
