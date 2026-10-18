//! Daily routines: what each kind of villager does at each hour.

use crate::Pose;
use crate::place::SpotKind;
use serde::{Deserialize, Serialize};

/// Something to do at a spot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Activity {
    /// Abed at home.
    Sleep,
    /// Up early: fetching water, chores.
    Chores,
    /// At home to eat.
    Eat,
    /// At the trade's workplace.
    Work,
    /// In the fields.
    Farm,
    /// With the livestock.
    Tend,
    /// At the market.
    Market,
    /// At prayer.
    Worship,
    /// At the inn.
    Drink,
    /// About the green.
    Wander,
    /// Playing on the green (children).
    Play,
    /// Indoors until danger passes ([`crate::LifeWorld::alarm`]).
    Shelter,
}

impl Activity {
    /// Where it happens, most wanted first. `Work` means the villager's own
    /// workplace before any other.
    #[must_use]
    pub fn spots(self) -> &'static [SpotKind] {
        use SpotKind::*;
        match self {
            Self::Sleep | Self::Eat | Self::Shelter => &[Home],
            Self::Chores => &[Well, Green, Home],
            Self::Work => &[Work, Market, Green],
            Self::Farm => &[Field, Pen, Green],
            Self::Tend => &[Pen, Field, Green],
            Self::Market => &[Market, Green, Well],
            Self::Worship => &[Chapel, Green],
            Self::Drink => &[Inn, Green, Home],
            Self::Wander => &[Green, Market, Well],
            Self::Play => &[Green, Well, Field],
        }
    }

    /// The pose while at it (outdoors; indoors is out of sight).
    #[must_use]
    pub fn pose(self) -> Pose {
        match self {
            Self::Sleep | Self::Eat | Self::Wander | Self::Chores | Self::Shelter => Pose::Idle,
            Self::Work => Pose::Work,
            Self::Farm => Pose::Farm,
            Self::Tend => Pose::Tend,
            Self::Market => Pose::Trade,
            Self::Worship => Pose::Pray,
            Self::Drink => Pose::Sit,
            Self::Play => Pose::Play,
        }
    }
}

/// Who a villager is, by trade.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Role {
    /// Fields from dawn.
    Farmer,
    /// The forge, the storehouse.
    Smith,
    /// The beasts.
    Herder,
    /// Keeps the inn late.
    Keeper,
    /// The chapel's hours.
    Priest,
    /// Old: the green and the chapel.
    Elder,
    /// Hauls and carries.
    Labourer,
    /// Plays on the green, home by dusk.
    Child,
}

impl Role {
    /// Every role, in declaration order.
    pub const ALL: [Self; 8] = [
        Self::Farmer,
        Self::Smith,
        Self::Herder,
        Self::Keeper,
        Self::Priest,
        Self::Elder,
        Self::Labourer,
        Self::Child,
    ];

    /// Display names, in [`Self::ALL`] order.
    pub const NAMES: [&'static str; 8] = [
        "Farmer", "Smith", "Herder", "Keeper", "Priest", "Elder", "Labourer", "Child",
    ];

    /// The workplace a role needs to have work.
    #[must_use]
    pub fn workplace(self) -> Option<SpotKind> {
        match self {
            Self::Farmer => Some(SpotKind::Field),
            Self::Smith | Self::Labourer => Some(SpotKind::Work),
            Self::Herder => Some(SpotKind::Pen),
            Self::Keeper => Some(SpotKind::Inn),
            Self::Priest => Some(SpotKind::Chapel),
            Self::Elder | Self::Child => None,
        }
    }

    /// The role's day.
    #[must_use]
    pub fn routine(self) -> Routine {
        use Activity::*;
        let blocks: &[(f32, Activity)] = match self {
            Self::Farmer => &[
                (0.0, Sleep),
                (5.5, Chores),
                (6.5, Farm),
                (12.0, Eat),
                (13.0, Farm),
                (17.5, Market),
                (19.0, Drink),
                (21.5, Sleep),
            ],
            Self::Smith | Self::Labourer => &[
                (0.0, Sleep),
                (6.0, Chores),
                (7.0, Work),
                (12.0, Eat),
                (13.0, Work),
                (18.0, Wander),
                (19.5, Drink),
                (22.0, Sleep),
            ],
            Self::Herder => &[
                (0.0, Sleep),
                (5.0, Tend),
                (11.5, Eat),
                (12.5, Tend),
                (17.0, Chores),
                (18.5, Drink),
                (21.0, Sleep),
            ],
            Self::Keeper => &[
                (0.0, Sleep),
                (8.0, Chores),
                (9.0, Market),
                (11.0, Work),
                (23.5, Sleep),
            ],
            Self::Priest => &[
                (0.0, Sleep),
                (5.0, Worship),
                (8.0, Wander),
                (11.0, Worship),
                (13.0, Eat),
                (14.0, Wander),
                (17.5, Worship),
                (20.5, Sleep),
            ],
            Self::Elder => &[
                (0.0, Sleep),
                (7.0, Eat),
                (9.0, Wander),
                (12.0, Eat),
                (14.0, Worship),
                (16.0, Wander),
                (18.5, Drink),
                (21.0, Sleep),
            ],
            Self::Child => &[
                (0.0, Sleep),
                (7.0, Eat),
                (8.0, Play),
                (12.0, Eat),
                (13.0, Play),
                (17.5, Chores),
                (19.0, Sleep),
            ],
        };
        Routine {
            blocks: blocks.to_vec(),
        }
    }
}

/// A day as blocks: from each start hour until the next, one activity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Routine {
    /// `(start hour, activity)`, sorted by hour; the last runs past midnight
    /// into the first.
    pub blocks: Vec<(f32, Activity)>,
}

impl Routine {
    /// What to do at `hour`.
    #[must_use]
    pub fn at(&self, hour: f32) -> Activity {
        let h = hour.rem_euclid(24.0);
        self.blocks
            .iter()
            .rev()
            .find(|(start, _)| *start <= h)
            .or(self.blocks.last())
            .map_or(Activity::Wander, |(_, a)| *a)
    }
}
