//! A realm's places as data: every town's spots, lanes and roads in one file,
//! written by a game's tool from its settlements, and adopted by a host as the
//! view comes near ([`LifeTable::near`], then
//! [`crate::LifeWorld::adopt_place`] with [`TablePlace::key`]), so the people
//! of a town far off cost nothing until the player is on the way there.

use crate::Pose;
use crate::place::Place;
use crate::routine::Role;
use crate::sim::BeastKind;
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Every place of a realm.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LifeTable {
    /// Format version (1).
    #[serde(default)]
    pub version: u32,
    /// The places.
    pub places: Vec<TablePlace>,
    /// How its people and beasts look, if the game has bodies for them.
    #[serde(default)]
    pub looks: Option<TableLooks>,
}

/// How a table's people and beasts look: skinned bodies and their clips (a
/// game's assets; paths relative to its assets folder, with `<name>_lod1.glb`
/// and `<name>_lod2.glb` beside a body as its lighter levels).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TableLooks {
    /// Bodies by role name ([`Role::NAMES`]); `default` for any role not
    /// named. Each person keeps one of their role's, by who they are.
    #[serde(default)]
    pub villagers: BTreeMap<String, Vec<String>>,
    /// A body by beast kind ([`BeastKind::NAMES`]).
    #[serde(default)]
    pub beasts: BTreeMap<String, String>,
    /// Clips by pose name ([`Pose::name`]). A clip may list alternatives,
    /// `A|B`: a body plays the first its file has.
    #[serde(default)]
    pub poses: BTreeMap<String, String>,
    /// Children's own clips, over `poses`.
    #[serde(default)]
    pub child_poses: BTreeMap<String, String>,
    /// Elders' own clips, over `poses`.
    #[serde(default)]
    pub elder_poses: BTreeMap<String, String>,
    /// Beasts' clips by pose name (`Graze`, `Walk`, `Run`, `Idle`).
    #[serde(default)]
    pub beast_poses: BTreeMap<String, String>,
    /// How fast the walk clip moves, m/s: villagers walk at it.
    #[serde(default)]
    pub walk_speed: Option<f32>,
    /// How fast the run clip moves, m/s: villagers run from danger at it.
    #[serde(default)]
    pub run_speed: Option<f32>,
    /// A role's own clips by pose (a smith at work hammers), over `poses`
    /// and the children's and elders' own.
    #[serde(default)]
    pub role_poses: BTreeMap<String, BTreeMap<String, String>>,
    /// What a clip holds, by clip name: pieces on joint sockets (a game's
    /// gear table: the hoe in `Hoe_Loop`).
    #[serde(default)]
    pub gear: BTreeMap<String, Vec<GearPiece>>,
    /// Each body's height in metres, by its path: gear is sized to its holder.
    #[serde(default)]
    pub heights: BTreeMap<String, f32>,
}

/// One piece a clip holds: meshes of a file, on a joint's socket.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GearPiece {
    /// What it is (`hoe`).
    #[serde(default)]
    pub piece: String,
    /// Its file, relative to the assets folder.
    pub file: String,
    /// The file's mesh nodes to draw.
    #[serde(default)]
    pub nodes: Vec<u32>,
    /// The joint it rides (`weapon_r`).
    pub joint: String,
    /// Its offset from the joint, for a body of `ref_height_m`.
    #[serde(default)]
    pub offset: [f32; 3],
    /// The height the piece and offset are sized for.
    #[serde(default = "ref_height")]
    pub ref_height_m: f32,
}

fn ref_height() -> f32 {
    1.76
}

impl TableLooks {
    /// The body person `who` wears, of their role's.
    #[must_use]
    pub fn body(&self, role: Role, who: u32) -> Option<&str> {
        static NONE: BTreeMap<String, Vec<String>> = BTreeMap::new();
        self.body_in(&NONE, role, who)
    }

    /// The body person `who` of a place with its own `people` (bodies by role
    /// name, as [`TablePlace::people`]) wears. A role the place names comes
    /// first. A child then takes the table's children and never an adult's
    /// body; an elder the table's elders, then an adult's; anyone else the
    /// place's `default`, then the table's own for the role, then its `default`.
    #[must_use]
    pub fn body_in<'a>(
        &'a self,
        people: &'a BTreeMap<String, Vec<String>>,
        role: Role,
        who: u32,
    ) -> Option<&'a str> {
        let name = Role::NAMES[role as usize];
        let (own, theirs) = (people.get(name), self.villagers.get(name));
        let (adults, all) = (people.get("default"), self.villagers.get("default"));
        let order = match role {
            Role::Child => [own, theirs, None, None],
            Role::Elder => [own, theirs, adults, all],
            _ => [own, adults, theirs, all],
        };
        let list = order.into_iter().flatten().find(|l| !l.is_empty())?;
        // A stable pick, so the same person wears the same body each time.
        let pick = (who.wrapping_mul(2_654_435_761) >> 7) as usize % list.len().max(1);
        list.get(pick).map(String::as_str)
    }

    /// The clip a person of `role` plays in `pose`.
    #[must_use]
    pub fn clip(&self, role: Role, pose: Pose) -> Option<&str> {
        let trade = self
            .role_poses
            .get(Role::NAMES[role as usize])
            .and_then(|p| p.get(pose.name()));
        let own = match role {
            Role::Child => self.child_poses.get(pose.name()),
            Role::Elder => self.elder_poses.get(pose.name()),
            _ => None,
        };
        trade
            .or(own)
            .or_else(|| self.poses.get(pose.name()))
            .map(String::as_str)
    }

    /// What `clip` holds (its first alternative, `A|B`, the one a body plays
    /// when its file has it), and the scale each piece takes on `body` (its
    /// height over the piece's reference height).
    pub fn gear_for<'a>(
        &'a self,
        clip: &str,
        body: &str,
    ) -> impl Iterator<Item = (&'a GearPiece, f32)> + 'a {
        let height = self.heights.get(body).copied();
        let first = clip.split('|').next().unwrap_or_default().trim();
        self.gear.get(first).into_iter().flatten().map(move |g| {
            let scale = height.map_or(1.0, |h| h / g.ref_height_m.max(0.01));
            (g, scale)
        })
    }

    /// The body a beast of `kind` wears.
    #[must_use]
    pub fn beast_body(&self, kind: BeastKind) -> Option<&str> {
        self.beasts
            .get(BeastKind::NAMES[kind as usize])
            .map(String::as_str)
    }

    /// The clip a beast plays in `pose`.
    #[must_use]
    pub fn beast_clip(&self, pose: Pose) -> Option<&str> {
        self.beast_poses.get(pose.name()).map(String::as_str)
    }
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
    /// Its own people: bodies by role name (and `default` for adults of any
    /// role), over the table's looks, so each town wears its own folk.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub people: BTreeMap<String, Vec<String>>,
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
            people: BTreeMap::new(),
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
            looks: None,
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

    /// Looks: a person keeps one body of their role's, children and elders
    /// play their own clips where they have them, beasts theirs.
    #[test]
    fn looks_dress_people_and_beasts_by_role_and_pose() {
        let looks: TableLooks = serde_json::from_str(
            r#"{
              "villagers": {"Farmer": ["a.glb", "b.glb"], "default": ["c.glb"]},
              "beasts": {"Sheep": "sheep.glb"},
              "poses": {"Walk": "Loco_Walk_F_Loop", "Idle": "Idle_Neutral_Loop"},
              "child_poses": {"Walk": "Child_Walk_F_Loop"},
              "beast_poses": {"Graze": "Graze_Loop"},
              "walk_speed": 0.86
            }"#,
        )
        .unwrap();
        let farmer = looks.body(Role::Farmer, 7).unwrap();
        assert!(farmer == "a.glb" || farmer == "b.glb");
        assert_eq!(
            looks.body(Role::Farmer, 7),
            Some(farmer),
            "the same each time"
        );
        assert_eq!(looks.body(Role::Smith, 7), Some("c.glb"), "the default");
        assert_eq!(
            looks.clip(Role::Child, Pose::Walk),
            Some("Child_Walk_F_Loop")
        );
        assert_eq!(
            looks.clip(Role::Child, Pose::Idle),
            Some("Idle_Neutral_Loop")
        );
        assert_eq!(
            looks.clip(Role::Farmer, Pose::Walk),
            Some("Loco_Walk_F_Loop")
        );
        assert_eq!(looks.clip(Role::Farmer, Pose::Pray), None);
        assert_eq!(looks.beast_body(BeastKind::Sheep), Some("sheep.glb"));
        assert_eq!(looks.beast_clip(Pose::Graze), Some("Graze_Loop"));
        assert_eq!(looks.walk_speed, Some(0.86));
        // A town with its own folk: its adults and smiths wear them; its
        // children, having none of their own, the table's children.
        let people: BTreeMap<String, Vec<String>> = serde_json::from_str(
            r#"{"default": ["kael_00.glb"], "Smith": ["smith_kael.glb"], "Child": []}"#,
        )
        .unwrap();
        assert_eq!(looks.body_in(&people, Role::Farmer, 7), Some("kael_00.glb"));
        assert_eq!(
            looks.body_in(&people, Role::Smith, 7),
            Some("smith_kael.glb")
        );
        assert_eq!(looks.body_in(&people, Role::Priest, 7), Some("kael_00.glb"));
        assert_eq!(
            looks.body_in(&people, Role::Elder, 7),
            Some("kael_00.glb"),
            "an elder with no elders' bodies wears an adult's"
        );
        assert_eq!(
            looks.body_in(&people, Role::Child, 7),
            None,
            "never an adult's body"
        );
        let mut kids = looks.clone();
        kids.villagers
            .insert("Child".into(), vec!["child.glb".into()]);
        assert_eq!(kids.body_in(&people, Role::Child, 7), Some("child.glb"));
    }

    /// A trade's own clips come first; a clip's gear is sized to its holder.
    #[test]
    fn trades_have_their_own_clips_and_gear_fits_the_hand() {
        let looks: TableLooks = serde_json::from_str(
            r#"{
              "poses": {"Work": "Bench_Work_Loop", "Farm": "Hoe_Loop"},
              "role_poses": {"Smith": {"Work": "Hammer_Anvil_Loop"}},
              "gear": {"Hoe_Loop": [{"piece": "hoe", "file": "characters/gear/hoe.glb", "nodes": [0, 1],
                                     "joint": "weapon_r", "offset": [0, 0.1, 0], "ref_height_m": 1.76}]},
              "heights": {"short.glb": 1.32}
            }"#,
        )
        .unwrap();
        assert_eq!(
            looks.clip(Role::Smith, Pose::Work),
            Some("Hammer_Anvil_Loop")
        );
        assert_eq!(
            looks.clip(Role::Labourer, Pose::Work),
            Some("Bench_Work_Loop")
        );
        let (hoe, scale) = looks
            .gear_for("Hoe_Loop", "short.glb")
            .next()
            .expect("a hoe");
        assert_eq!(
            (hoe.piece.as_str(), hoe.joint.as_str(), hoe.nodes.len()),
            ("hoe", "weapon_r", 2)
        );
        assert!((scale - 0.75).abs() < 1e-6, "1.32 m of 1.76: {scale}");
        assert_eq!(
            looks.gear_for("Hoe_Loop", "unknown.glb").next().unwrap().1,
            1.0
        );
        assert!(
            looks
                .gear_for("Bench_Work_Loop", "short.glb")
                .next()
                .is_none()
        );
        assert!(
            looks
                .gear_for("Hoe_Loop|Bench_Work_Loop", "short.glb")
                .next()
                .is_some(),
            "the first alternative's gear"
        );
    }
}
