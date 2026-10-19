//! People who are not villagers of one place: the dual protagonists' off-screen
//! man (Rimeborn's DP-4, `DUAL_PROTAGONIST_ENGINE_SPECS`) and, later, named
//! cast with fixed habits.
//!
//! A person keeps a [`Schedule`]: blocks of the day, each from its hour until
//! the next, saying where (a place by [`crate::Place::key`], a spot there) and
//! what he does. In a place his schedule has him in he is a villager of it if
//! it is adopted: walking its lanes to his spot and doing it there, drawn as his
//! own body, never [`crate::Tier::Asleep`], sheltering at his spot in an alarm.
//! Moving on to another place he travels off-screen for the pair's travel time
//! ([`LifeWorld::set_travel_hours`], an hour unless set) and turns up there.
//! Places he is not in see him away (indoors, unseen). Time skipped (a large
//! step: sleep, travel, the switch's crossing) puts him where the schedule says
//! without walking. Released (the player has him) he is not stepped at all;
//! resumed, he takes up his day from where he was left. The sim never harms
//! him: healing, Hue and income are the game's sums.

use super::{AgentId, Doing, Goal, LifeWorld, VillagerDesc};
use crate::{Activity, Clock, Place, Pose, Role, SpotKind};
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

/// One block of a hand-written day: from `start_hour` until the next block,
/// be here doing this.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScheduleBlock {
    /// When it starts (`0..24`).
    pub start_hour: f32,
    /// The place, by [`Place::key`] ([`crate::LifeTable::key_of`] finds a
    /// table place's): a key, not an index, because a schedule's far places
    /// are rarely adopted yet.
    pub place: u64,
    /// Where in it.
    pub spot: SpotRef,
    /// What he does there.
    pub activity: Activity,
    /// What the journal and the switch wheel say ("At school in Kaelmar").
    #[serde(default)]
    pub label: String,
}

/// A person's day (one per act, the story swapping them).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Schedule {
    /// Its blocks, in any order.
    pub blocks: Vec<ScheduleBlock>,
}

impl Schedule {
    /// The block holding `hour`: the latest to have started, the day's last
    /// one before the first has.
    #[must_use]
    pub fn at(&self, hour: f32) -> Option<&ScheduleBlock> {
        let h = hour.rem_euclid(24.0);
        let latest = |a: &&ScheduleBlock, b: &&ScheduleBlock| a.start_hour.total_cmp(&b.start_hour);
        let started = self
            .blocks
            .iter()
            .filter(|b| b.start_hour <= h)
            .max_by(latest);
        started.or_else(|| self.blocks.iter().max_by(latest))
    }
}

/// Where a person is and what he is doing.
#[derive(Clone, Debug, PartialEq)]
pub struct PersonAt {
    /// The place he is in (or set out from, travelling).
    pub place: u64,
    /// His block's spot there.
    pub spot: SpotRef,
    /// Where he stands, when his place is adopted (his body there, or else its
    /// spot); `None` in a place not adopted, where the host resolves it against
    /// its table ([`crate::LifeTable::spot_position`]), and on the road.
    pub position: Option<Vec3>,
    /// Which way he faces.
    pub facing: Vec3,
    /// What he does.
    pub activity: Activity,
    /// His block's label.
    pub label: String,
    /// Off-screen on the road between two places, the one he is bound for.
    pub travelling_to: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Where {
    /// Not yet placed: he starts in his block's place.
    Unplaced,
    In(u64),
    Travelling {
        from: u64,
        to: u64,
        arrive: f64,
    },
}

#[derive(Clone, Debug)]
pub(super) struct Person {
    id: AgentId,
    name: String,
    body: Option<String>,
    schedule: Schedule,
    /// The story's block instead of the schedule's, until this many hours of the world's day count.
    over: Option<(ScheduleBlock, f64)>,
    at: Where,
    /// His villager in each adopted place he has been in (villager indices: they never go).
    bodies: HashMap<usize, usize>,
    /// The player has him: where he was taken.
    released: Option<PersonAt>,
}

/// More than this many hours in one step is time skipped: he is put where the schedule says.
const SKIP_HOURS: f64 = 0.25;
/// Travel between two places unless the game says otherwise.
const TRAVEL_HOURS: f32 = 1.0;
/// How near an adopted place's spots a resumed person must be to be in it, metres.
const RESUME_REACH: f32 = 400.0;

impl LifeWorld {
    /// A person with a day of his own (see the module). Returns his id.
    pub fn adopt_person(&mut self, name: &str, schedule: Schedule) -> AgentId {
        let id = self.next();
        self.persons.push(Person {
            id,
            name: name.to_string(),
            body: None,
            schedule,
            over: None,
            at: Where::Unplaced,
            bodies: HashMap::new(),
            released: None,
        });
        id
    }

    fn person_mut(&mut self, id: AgentId) -> Option<&mut Person> {
        self.persons.iter_mut().find(|p| p.id == id)
    }

    /// The glb he is drawn as wherever he is a villager ([`crate::Villager::body`]).
    pub fn set_person_body(&mut self, id: AgentId, body: &str) -> bool {
        let Some(p) = self.persons.iter_mut().find(|p| p.id == id) else {
            return false;
        };
        p.body = Some(body.to_string());
        for &v in p.bodies.values() {
            self.villagers[v].body = Some(body.to_string());
        }
        true
    }

    /// A new day for him (the next act's).
    pub fn set_schedule(&mut self, id: AgentId, schedule: Schedule) -> bool {
        self.person_mut(id).map(|p| p.schedule = schedule).is_some()
    }

    /// The story's block instead of the schedule's until `until_hour` comes
    /// round (in hospital, under arrest); then the schedule again.
    pub fn override_block(&mut self, id: AgentId, block: ScheduleBlock, until_hour: f32) -> bool {
        let now = self.person_hours;
        let hour = self.last_hour.unwrap_or(0.0);
        let ahead = f64::from((until_hour - hour).rem_euclid(24.0));
        self.person_mut(id)
            .map(|p| p.over = Some((block, now + ahead)))
            .is_some()
    }

    /// How long the road between places `a` and `b` (keys) takes, game hours, either way.
    pub fn set_travel_hours(&mut self, a: u64, b: u64, hours: f32) {
        self.travel.insert((a.min(b), a.max(b)), hours.max(0.0));
    }

    fn travel_hours(&self, a: u64, b: u64) -> f32 {
        self.travel
            .get(&(a.min(b), a.max(b)))
            .copied()
            .unwrap_or(TRAVEL_HOURS)
    }

    fn block_of<'a>(&self, p: &'a Person, hour: f32) -> Option<&'a ScheduleBlock> {
        match &p.over {
            Some((block, until)) if self.person_hours < *until => Some(block),
            _ => p.schedule.at(hour),
        }
    }

    /// Where `id` is and what he is doing at `clock` (his block at that hour;
    /// where he stands as of the last step). Released, where he was taken.
    #[must_use]
    pub fn person_at(&self, id: AgentId, clock: Clock) -> Option<PersonAt> {
        let p = self.persons.iter().find(|p| p.id == id)?;
        if let Some(r) = &p.released {
            return Some(r.clone());
        }
        let block = self.block_of(p, clock.hour)?;
        let (place, travelling_to) = match p.at {
            Where::Travelling { from, to, .. } => (from, Some(to)),
            Where::In(k) => (k, None),
            Where::Unplaced => (block.place, None),
        };
        let here = travelling_to
            .is_none()
            .then(|| self.keyed.get(&place).copied())
            .flatten();
        let (position, facing) = match here {
            Some(index) => match p.bodies.get(&index).map(|&v| &self.villagers[v]) {
                Some(v) => (Some(v.position), v.facing),
                None => {
                    let placed = &self.places[index].place;
                    let spot = (place == block.place)
                        .then(|| block.spot.resolve(placed))
                        .flatten();
                    (spot.map(|s| placed.spots[s].position), Vec3::Z)
                }
            },
            None => (None, Vec3::Z),
        };
        Some(PersonAt {
            place,
            spot: block.spot.clone(),
            position,
            facing,
            activity: block.activity,
            label: block.label.clone(),
            travelling_to,
        })
    }

    /// The player takes him: life stops stepping and drawing him, and keeps
    /// where he was (returned). `None` if there was no such person.
    pub fn release_person(&mut self, id: AgentId) -> Option<PersonAt> {
        let clock = Clock::at(self.last_hour.unwrap_or(0.0));
        let at = self.person_at(id, clock)?;
        let p = self.person_mut(id)?;
        p.released = Some(at.clone());
        let bodies: Vec<usize> = p.bodies.values().copied().collect();
        for v in bodies {
            self.villagers[v].goal = Goal::Away;
            self.villagers[v].visible = false;
        }
        Some(at)
    }

    /// The player leaves him at `position`: he takes up his day from there, in
    /// the adopted place nearest (on the road to his block's if none is near).
    pub fn resume_person(&mut self, id: AgentId, position: Vec3, facing: Vec3) -> bool {
        let near = self
            .keyed
            .iter()
            .map(|(&key, &index)| {
                let d = self.places[index]
                    .place
                    .spots
                    .iter()
                    .map(|s| s.position.distance(position))
                    .fold(f32::INFINITY, f32::min);
                (d, key, index)
            })
            .filter(|(d, _, _)| *d < RESUME_REACH)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let hour = self.last_hour.unwrap_or(0.0);
        let hours = self.person_hours;
        let Some(i) = self.persons.iter().position(|p| p.id == id) else {
            return false;
        };
        self.persons[i].released = None;
        match near {
            Some((_, key, index)) => {
                self.persons[i].at = Where::In(key);
                // His body here, where he was left, to walk on from.
                if let Some(v) = self.person_body(i, index, None) {
                    let v = &mut self.villagers[v];
                    v.position = position;
                    v.facing = facing;
                    v.target = None;
                    v.goal = Goal::Routine;
                    v.doing = Doing::At {
                        spot: v.home,
                        shuffle_at: 0.0,
                    };
                }
            }
            None => {
                let to = self.block_of(&self.persons[i], hour).map_or(0, |b| b.place);
                self.persons[i].at = Where::Travelling {
                    from: to,
                    to,
                    arrive: hours + f64::from(TRAVEL_HOURS),
                };
            }
        }
        true
    }

    /// Person `i`'s villager in place `index`, made the first time (at `spot`, else the place's first).
    fn person_body(&mut self, i: usize, index: usize, spot: Option<usize>) -> Option<usize> {
        if let Some(&v) = self.persons[i].bodies.get(&index) {
            return Some(v);
        }
        let home = spot
            .unwrap_or(0)
            .min(self.places[index].place.spots.len().checked_sub(1)?);
        let desc = VillagerDesc {
            name: self.persons[i].name.clone(),
            role: Role::Labourer,
            place: index,
            home,
            work: Some(home),
        };
        self.spawn_villager(desc)?;
        let v = self.villagers.len() - 1;
        self.villagers[v].body.clone_from(&self.persons[i].body);
        self.villagers[v].goal = Goal::Away;
        self.persons[i].bodies.insert(index, v);
        Some(v)
    }

    /// Each person on with his day this step (at the top of [`LifeWorld::step`]).
    pub(super) fn follow_schedules(&mut self, clock: Clock) {
        let hour = clock.hour.rem_euclid(24.0);
        let passed = self
            .last_hour
            .map_or(0.0, |h| f64::from((hour - h).rem_euclid(24.0)));
        self.last_hour = Some(hour);
        self.person_hours += passed;
        let (hours, skipped) = (self.person_hours, passed > SKIP_HOURS);
        for i in 0..self.persons.len() {
            if self.persons[i].released.is_some() {
                continue;
            }
            if self.persons[i]
                .over
                .as_ref()
                .is_some_and(|(_, until)| hours >= *until)
            {
                self.persons[i].over = None;
            }
            let Some(block) = self.block_of(&self.persons[i], hour).cloned() else {
                continue;
            };
            // Where he is: setting out, on the road, arrived.
            let at = match self.persons[i].at {
                Where::Unplaced => Where::In(block.place),
                Where::In(k) if k != block.place => {
                    let road = f64::from(self.travel_hours(k, block.place));
                    if skipped && road <= passed {
                        Where::In(block.place)
                    } else {
                        Where::Travelling {
                            from: k,
                            to: block.place,
                            arrive: hours + road,
                        }
                    }
                }
                Where::Travelling { to, arrive, .. } if hours >= arrive => Where::In(to),
                other => other,
            };
            self.persons[i].at = at;
            let here = match at {
                Where::In(k) => self.keyed.get(&k).copied(),
                _ => None,
            };
            for (&index, &v) in &self.persons[i].bodies {
                if Some(index) != here {
                    self.villagers[v].goal = Goal::Away;
                }
            }
            let Some(index) = here else {
                continue;
            };
            let Some(spot) = block.spot.resolve(&self.places[index].place) else {
                continue;
            };
            let Some(v) = self.person_body(i, index, Some(spot)) else {
                continue;
            };
            let arrived = self.villagers[v].goal == Goal::Away;
            self.villagers[v].goal = Goal::At(spot, block.activity);
            // Skipped time, or just turned up: there already, not walking there.
            if skipped || arrived {
                let s = &self.places[index].place.spots[spot];
                let (to, outdoors) = (s.position, !s.kind.indoor());
                let v = &mut self.villagers[v];
                v.position = to;
                v.target = Some(spot);
                v.activity = block.activity;
                v.doing = Doing::At {
                    spot,
                    shuffle_at: 0.0,
                };
                v.visible = outdoors;
                v.pose = if outdoors {
                    block.activity.pose()
                } else {
                    Pose::Inside
                };
            }
        }
    }
}
