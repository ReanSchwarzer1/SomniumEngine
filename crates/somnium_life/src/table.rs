//! A realm's places as data: every town's spots, lanes and roads in one file,
//! written by a game's tool from its settlements, and adopted by a host as the
//! view comes near ([`LifeTable::near`], then
//! [`crate::LifeWorld::adopt_place`] with [`TablePlace::key`]), so the people
//! of a town far off cost nothing until the player is on the way there.

use crate::place::Place;
use glam::Vec3;
use serde::{Deserialize, Serialize};

/// Every place of a realm.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LifeTable {
    /// Format version (1).
    #[serde(default)]
    pub version: u32,
    /// The places.
    pub places: Vec<TablePlace>,
}

/// One place of the table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TablePlace {
    /// Its middle, world metres.
    pub centre: [f32; 3],
    /// How far its spots reach from the middle, metres.
    pub radius: f32,
    /// Who lives where.
    #[serde(default)]
    pub seed: u64,
    /// Its spots, lanes and roads.
    pub place: Place,
}

impl TablePlace {
    /// The key it is adopted by: the same every time it comes near.
    #[must_use]
    pub fn key(&self) -> u64 {
        Place::key(&self.place.name, Vec3::from(self.centre))
    }
}

impl LifeTable {
    /// Read a table.
    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }

    /// The places whose edge is within `reach` of `at` on the ground.
    pub fn near(&self, at: Vec3, reach: f32) -> impl Iterator<Item = usize> + '_ {
        self.places.iter().enumerate().filter_map(move |(i, p)| {
            let d = glam::Vec2::new(p.centre[0] - at.x, p.centre[2] - at.z).length();
            (d - p.radius <= reach).then_some(i)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::place::{Spot, SpotKind};
    use crate::{LifeWorld, LodConfig};

    fn table() -> LifeTable {
        let town = |name: &str, x: f32| TablePlace {
            centre: [x, 0.0, 0.0],
            radius: 40.0,
            seed: 7,
            place: Place {
                name: name.into(),
                spots: (0..6)
                    .map(|i| Spot::new(SpotKind::Home, Vec3::new(x + i as f32 * 8.0, 0.0, 0.0)))
                    .chain([Spot::new(SpotKind::Inn, Vec3::new(x, 0.0, 10.0))])
                    .collect(),
                lanes: vec![vec![
                    Vec3::new(x - 30.0, 0.0, 4.0),
                    Vec3::new(x + 60.0, 0.0, 4.0),
                ]],
                roads: Vec::new(),
            },
        };
        LifeTable {
            version: 1,
            places: vec![town("Nearby", 0.0), town("Far Off", 5_000.0)],
        }
    }

    /// A table reads back from JSON, and a town is adopted (once) when the
    /// view comes within reach of its edge, not before.
    #[test]
    fn towns_are_adopted_as_the_view_comes_near() {
        let text = serde_json::to_string(&table()).unwrap();
        let t = LifeTable::from_json(&text).unwrap();
        assert_eq!(t, table());
        assert_eq!(t.near(Vec3::ZERO, 600.0).collect::<Vec<_>>(), vec![0]);
        assert_eq!(
            t.near(Vec3::new(4_400.0, 0.0, 0.0), 600.0)
                .collect::<Vec<_>>(),
            vec![1]
        );
        let mut w = LifeWorld::new(LodConfig::default());
        for _ in 0..3 {
            for i in t.near(Vec3::ZERO, 600.0) {
                let p = &t.places[i];
                w.adopt_place(p.key(), p.place.clone(), Some(p.seed));
            }
        }
        assert_eq!(w.place_count(), 1, "adopted once");
        assert!(w.villagers().len() >= 6, "a household a home");
        assert!(LifeTable::from_json("{").is_err());
    }
}
