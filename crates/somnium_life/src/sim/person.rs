//! People who are not villagers of one place: the dual protagonists' off-screen
//! man (Rimeborn's DP-4) and anyone else the game gives a day of its own.
//!
//! A person has a [`Schedule`]: for each part of the day, a place (by its
//! [`crate::Place::key`]), a spot there and what he does. Wherever that place
//! is adopted he is a villager of it, walking to his spot and doing it, drawn
//! as his own body; in any other place his schedule took him to he is away
//! (indoors, unseen). Where his place is not adopted, [`LifeWorld::person_at`]
//! still says where the schedule has him, for the game to resolve against its
//! table ([`crate::LifeTable::spot_position`]). Nobody dies off-screen: the sim
//! never harms him; healing and the rest are the game's facts.

use super::{AgentId, Goal, LifeWorld, VillagerDesc};
use crate::{Activity, Clock, Place, Role, SpotKind};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Which spot of a place.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SpotRef {
    /// The place's spot by index.
    Index(usize),
    /// The place's first spot of a kind (its inn, its chapel).
    Kind(SpotKind),
}

impl SpotRef {
    /// The spot it names in `place`.
    #[must_use]
    pub fn resolve(&self, place: &Place) -> Option<usize> {
        match self {
            Self::Index(i) => (*i < place.spots.len()).then_some(*i),
            Self::Kind(k) => place.spots_of(*k).next(),
        }
    }
}

/// A part of a person's day.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScheduleEntry {
    /// From this hour (`0..24`) to `to_hour`; past midnight when `to_hour` is the smaller.
    pub from_hour: f32,
    /// Until this hour.
    pub to_hour: f32,
    /// The place, by [`Place::key`] ([`crate::LifeTable::key_of`] finds a table place's).
    pub place: u64,
    /// Where in it.
    pub spot: SpotRef,
    /// What he does there.
    pub activity: Activity,
}

impl ScheduleEntry {
    fn holds(&self, hour: f32) -> bool {
        let h = hour.rem_euclid(24.0);
        if self.from_hour <= self.to_hour {
            (self.from_hour..self.to_hour).contains(&h)
        } else {
            h >= self.from_hour || h < self.to_hour
        }
    }
}

/// A person's day, the first entry that holds the hour winning (one per act, the game swapping them).
pub type Schedule = Vec<ScheduleEntry>;

/// Where a person's schedule has him at a moment.
#[derive(Clone, Debug, PartialEq)]
pub struct PersonAt {
    /// The place, by its key.
    pub place: u64,
    /// The spot there.
    pub spot: SpotRef,
    /// Where he stands, when his place is adopted (his body there, or else its spot).
    pub position: Option<Vec3>,
}

#[derive(Clone, Debug)]
pub(super) struct Person {
    id: AgentId,
    name: String,
    body: Option<String>,
    schedule: Schedule,
    /// His villager in each adopted place his schedule took him to (villager indices: they never go).
    bodies: HashMap<usize, usize>,
}

impl Person {
    fn entry(&self, hour: f32) -> Option<&ScheduleEntry> {
        self.schedule.iter().find(|e| e.holds(hour))
    }
}

impl LifeWorld {
    /// A person with a day of his own (see the module): `body`, if any, is the
    /// glb he is drawn as wherever he is a villager. Returns his id.
    pub fn adopt_person(&mut self, name: &str, body: Option<&str>, schedule: Schedule) -> AgentId {
        let id = self.next();
        self.persons.push(Person {
            id,
            name: name.to_string(),
            body: body.map(str::to_string),
            schedule,
            bodies: HashMap::new(),
        });
        id
    }

    /// A new day for a person (the next act's).
    pub fn reschedule_person(&mut self, id: AgentId, schedule: Schedule) -> bool {
        self.persons
            .iter_mut()
            .find(|p| p.id == id)
            .map(|p| p.schedule = schedule)
            .is_some()
    }

    /// Where `id`'s schedule has him at `clock`.
    #[must_use]
    pub fn person_at(&self, id: AgentId, clock: Clock) -> Option<PersonAt> {
        let p = self.persons.iter().find(|p| p.id == id)?;
        let e = p.entry(clock.hour)?;
        let position = self.keyed.get(&e.place).and_then(|&place| {
            p.bodies
                .get(&place)
                .map(|&v| self.villagers[v].position)
                .or_else(|| {
                    let placed = &self.places[place].place;
                    Some(placed.spots[e.spot.resolve(placed)?].position)
                })
        });
        Some(PersonAt {
            place: e.place,
            spot: e.spot.clone(),
            position,
        })
    }

    /// The player takes him: he is nobody's to move now, and every body of his
    /// in the sim goes away. False if there was no such person.
    pub fn release_person(&mut self, id: AgentId) -> bool {
        let Some(i) = self.persons.iter().position(|p| p.id == id) else {
            return false;
        };
        let p = self.persons.remove(i);
        for &v in p.bodies.values() {
            self.villagers[v].goal = Goal::Away;
            self.villagers[v].visible = false;
        }
        true
    }

    /// Each person to where his schedule has him this hour: a villager of that
    /// place if it is adopted (made the first time), away in any other.
    pub(super) fn follow_schedules(&mut self, clock: Clock) {
        for i in 0..self.persons.len() {
            let (here, spot, activity) = {
                let p = &self.persons[i];
                match p.entry(clock.hour) {
                    Some(e) => match self.keyed.get(&e.place) {
                        Some(&place) => (
                            Some(place),
                            e.spot.resolve(&self.places[place].place),
                            e.activity,
                        ),
                        None => (None, None, e.activity),
                    },
                    None => (None, None, Activity::Sleep),
                }
            };
            for (&place, &v) in &self.persons[i].bodies {
                if Some(place) != here {
                    self.villagers[v].goal = Goal::Away;
                }
            }
            let (Some(place), Some(spot)) = (here, spot) else {
                continue;
            };
            let v = match self.persons[i].bodies.get(&place) {
                Some(&v) => v,
                None => {
                    let desc = VillagerDesc {
                        name: self.persons[i].name.clone(),
                        role: Role::Labourer,
                        place,
                        home: spot,
                        work: Some(spot),
                    };
                    if self.spawn_villager(desc).is_none() {
                        continue;
                    }
                    let v = self.villagers.len() - 1;
                    self.villagers[v].body.clone_from(&self.persons[i].body);
                    self.persons[i].bodies.insert(place, v);
                    v
                }
            };
            self.villagers[v].goal = Goal::At(spot, activity);
        }
    }
}
