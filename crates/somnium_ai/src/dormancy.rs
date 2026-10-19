//! Simulation LOD for groups: a bandit camp, a wolf pack, a herd. Far from
//! every watcher (the player, a camera) a group is only a record, with no
//! bodies and no brains; near one, the host builds it, and once every watcher
//! has gone well away it puts it back to sleep. Records sit in a uniform grid,
//! so an update looks only at the cells around each watcher and at the groups
//! already awake, whatever the size of the world.
//!
//! ```text
//!  asleep ── a watcher within `wake` of its edge ──▶ awake (the host builds it)
//!  awake  ── every watcher past `sleep` of its edge ──▶ asleep (the host saves and removes it)
//! ```
//!
//! `sleep` is wider than `wake`, so one walking the line does not wake and
//! sleep a camp on alternate frames; at most `wakes_per_update` groups wake in
//! one update, the nearest first, so a valley of camps never costs one frame.
use glam::Vec3;
use std::collections::{BTreeMap, BTreeSet};

/// A group's handle, stable for the set's life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroupId(pub u32);

/// What an update asks the host to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Build this group.
    Wake(GroupId),
    /// Save and remove this group.
    Sleep(GroupId),
}

/// Distances and budget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DormancySettings {
    /// A group wakes when a watcher comes within this of its edge (metres,
    /// on the ground plane).
    pub wake: f32,
    /// It sleeps only once every watcher is past this from its edge; wider
    /// than `wake`.
    pub sleep: f32,
    /// Grid cell size, metres.
    pub cell: f32,
    /// Most groups woken in one update, the nearest first; the rest wait.
    pub wakes_per_update: usize,
}

impl Default for DormancySettings {
    fn default() -> Self {
        Self {
            wake: 120.0,
            sleep: 170.0,
            cell: 64.0,
            wakes_per_update: 2,
        }
    }
}

struct Group<T> {
    at: Vec3,
    extent: f32,
    cell: (i32, i32),
    awake: bool,
    data: T,
}

/// Groups that sleep until a watcher nears, each carrying the host's `T`
/// (what it is, and what it should remember while asleep: who fell, who is
/// hurt).
pub struct Dormancy<T> {
    settings: DormancySettings,
    groups: Vec<Group<T>>,
    grid: BTreeMap<(i32, i32), Vec<u32>>,
    awake: BTreeSet<u32>,
    max_extent: f32,
}

impl<T> Dormancy<T> {
    /// An empty set. `sleep` below `wake` is raised to it, and a cell under a
    /// metre to one.
    pub fn new(settings: DormancySettings) -> Self {
        let mut settings = settings;
        settings.wake = settings.wake.max(0.0);
        settings.sleep = settings.sleep.max(settings.wake);
        settings.cell = settings.cell.max(1.0);
        Self {
            settings,
            groups: Vec::new(),
            grid: BTreeMap::new(),
            awake: BTreeSet::new(),
            max_extent: 0.0,
        }
    }

    /// The settings in force.
    pub fn settings(&self) -> DormancySettings {
        self.settings
    }

    fn cell_of(&self, at: Vec3) -> (i32, i32) {
        let c = self.settings.cell;
        ((at.x / c).floor() as i32, (at.z / c).floor() as i32)
    }

    /// Add a sleeping group centred at `at` and spreading `extent` metres
    /// round it.
    pub fn add(&mut self, at: Vec3, extent: f32, data: T) -> GroupId {
        let id = self.groups.len() as u32;
        let extent = if extent.is_finite() {
            extent.max(0.0)
        } else {
            0.0
        };
        let cell = self.cell_of(at);
        self.max_extent = self.max_extent.max(extent);
        self.groups.push(Group {
            at,
            extent,
            cell,
            awake: false,
            data,
        });
        self.grid.entry(cell).or_default().push(id);
        GroupId(id)
    }

    /// Groups held, awake or asleep.
    pub fn len(&self) -> usize {
        self.groups.len()
    }

    /// Whether it holds none.
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// The host's record for a group.
    pub fn data(&self, id: GroupId) -> Option<&T> {
        self.groups.get(id.0 as usize).map(|g| &g.data)
    }

    /// The host's record, to change (what to remember while it sleeps).
    pub fn data_mut(&mut self, id: GroupId) -> Option<&mut T> {
        self.groups.get_mut(id.0 as usize).map(|g| &mut g.data)
    }

    /// Where a group is.
    pub fn position(&self, id: GroupId) -> Option<Vec3> {
        self.groups.get(id.0 as usize).map(|g| g.at)
    }

    /// Whether a group is awake.
    pub fn is_awake(&self, id: GroupId) -> bool {
        self.awake.contains(&id.0)
    }

    /// The awake groups, in id order.
    pub fn awake(&self) -> impl Iterator<Item = GroupId> + '_ {
        self.awake.iter().map(|&i| GroupId(i))
    }

    /// Move a group's centre: an awake pack roams, and it should sleep, and
    /// wake again, where it went.
    pub fn move_to(&mut self, id: GroupId, at: Vec3) {
        if !at.is_finite() {
            return;
        }
        let cell = self.cell_of(at);
        let Some(g) = self.groups.get_mut(id.0 as usize) else {
            return;
        };
        g.at = at;
        if g.cell != cell {
            let old = std::mem::replace(&mut g.cell, cell);
            if let Some(ids) = self.grid.get_mut(&old) {
                ids.retain(|&i| i != id.0);
                if ids.is_empty() {
                    self.grid.remove(&old);
                }
            }
            self.grid.entry(cell).or_default().push(id.0);
        }
    }

    /// Distance from the nearest watcher to a group's edge on the ground plane.
    fn gap(g: &Group<T>, watchers: &[Vec3]) -> f32 {
        watchers
            .iter()
            .map(|w| (Vec3::new(w.x - g.at.x, 0.0, w.z - g.at.z)).length() - g.extent)
            .fold(f32::INFINITY, f32::min)
    }

    /// Sleep the awake groups every watcher has left, and wake the sleeping
    /// ones a watcher has come near (at most `wakes_per_update`, the nearest
    /// first). Returns what the host should do, sleeps first, in a fixed
    /// order. No watchers: everything sleeps.
    pub fn update(&mut self, watchers: &[Vec3]) -> Vec<Change> {
        let watchers: Vec<Vec3> = watchers.iter().copied().filter(|w| w.is_finite()).collect();
        let mut changes = Vec::new();
        let sleepers: Vec<u32> = self
            .awake
            .iter()
            .copied()
            .filter(|&i| Self::gap(&self.groups[i as usize], &watchers) > self.settings.sleep)
            .collect();
        for i in sleepers {
            self.awake.remove(&i);
            self.groups[i as usize].awake = false;
            changes.push(Change::Sleep(GroupId(i)));
        }
        // Only the cells a watcher's wake disk (grown by the widest group)
        // touches can hold a group to wake.
        let reach = self.settings.wake + self.max_extent;
        let mut near: Vec<(f32, u32)> = Vec::new();
        let mut seen = BTreeSet::new();
        for w in &watchers {
            let (x0, z0) = self.cell_of(*w - Vec3::new(reach, 0.0, reach));
            let (x1, z1) = self.cell_of(*w + Vec3::new(reach, 0.0, reach));
            for x in x0..=x1 {
                for z in z0..=z1 {
                    for &i in self.grid.get(&(x, z)).into_iter().flatten() {
                        let g = &self.groups[i as usize];
                        if g.awake || !seen.insert(i) {
                            continue;
                        }
                        let gap = Self::gap(g, &watchers);
                        if gap <= self.settings.wake {
                            near.push((gap, i));
                        }
                    }
                }
            }
        }
        near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        for (_, i) in near.into_iter().take(self.settings.wakes_per_update) {
            self.awake.insert(i);
            self.groups[i as usize].awake = true;
            changes.push(Change::Wake(GroupId(i)));
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set<T>() -> Dormancy<T> {
        Dormancy::new(DormancySettings {
            wakes_per_update: usize::MAX,
            ..DormancySettings::default()
        })
    }

    #[test]
    fn a_camp_wakes_near_and_sleeps_only_well_away() {
        let mut d = set();
        let camp = d.add(Vec3::ZERO, 10.0, "camp");
        let at = |x: f32| [Vec3::new(x, 0.0, 0.0)];
        assert!(d.update(&at(200.0)).is_empty(), "far: asleep");
        assert_eq!(
            d.update(&at(125.0)),
            [Change::Wake(camp)],
            "115 m from its edge"
        );
        assert!(
            d.update(&at(150.0)).is_empty(),
            "between the radii: stays awake"
        );
        assert!(d.update(&at(-160.0)).is_empty());
        assert_eq!(
            d.update(&at(190.0)),
            [Change::Sleep(camp)],
            "180 m from its edge"
        );
        assert!(!d.is_awake(camp));
        assert_eq!(d.update(&[]), [], "no watchers, nothing to wake");
        d.update(&at(0.0));
        assert_eq!(
            d.update(&[]),
            [Change::Sleep(camp)],
            "no watchers: everything sleeps"
        );
        assert_eq!(*d.data(camp).unwrap(), "camp");
    }

    #[test]
    fn wakes_come_a_few_at_a_time_nearest_first() {
        let mut d: Dormancy<u32> = Dormancy::new(DormancySettings::default());
        let ids: Vec<GroupId> = (0..5)
            .map(|i| d.add(Vec3::new(10.0 * i as f32, 0.0, 0.0), 2.0, i))
            .collect();
        let here = [Vec3::ZERO];
        assert_eq!(
            d.update(&here),
            [Change::Wake(ids[0]), Change::Wake(ids[1])]
        );
        assert_eq!(
            d.update(&here),
            [Change::Wake(ids[2]), Change::Wake(ids[3])]
        );
        assert_eq!(d.update(&here), [Change::Wake(ids[4])]);
        assert!(d.update(&here).is_empty());
        assert_eq!(d.awake().count(), 5);
    }

    #[test]
    fn a_pack_sleeps_and_wakes_where_it_roamed() {
        let mut d = set();
        let pack = d.add(Vec3::ZERO, 5.0, "pack");
        d.update(&[Vec3::ZERO]);
        // It ran 2 km east while awake; the host moves its record.
        d.move_to(pack, Vec3::new(2000.0, 0.0, 0.0));
        assert_eq!(d.update(&[Vec3::ZERO]), [Change::Sleep(pack)]);
        assert!(d.update(&[Vec3::ZERO]).is_empty(), "not where it was");
        assert_eq!(
            d.update(&[Vec3::new(1950.0, 0.0, 30.0)]),
            [Change::Wake(pack)]
        );
        assert_eq!(d.position(pack), Some(Vec3::new(2000.0, 0.0, 0.0)));
    }

    /// Three thousand groups across a 10 km square, one watcher walking
    /// straight across it: exactly the groups near it are awake at each step,
    /// and an update stays cheap.
    #[test]
    fn three_thousand_camps_across_ten_kilometres_stay_cheap() {
        let mut d: Dormancy<u32> = set();
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f32 / (1u64 << 53) as f32
        };
        for i in 0..3000 {
            let at = Vec3::new(next() * 10_000.0, 0.0, next() * 10_000.0);
            d.add(at, 5.0 + next() * 20.0, i);
        }
        let start = std::time::Instant::now();
        let steps = 500;
        for s in 0..steps {
            let w = Vec3::new(s as f32 * 20.0, 0.0, 5000.0);
            d.update(&[w]);
            for id in d.awake() {
                let at = d.position(id).unwrap();
                assert!(Vec3::new(at.x - w.x, 0.0, at.z - w.z).length() <= 170.0 + 25.0 + 20.0);
            }
        }
        let per = start.elapsed().as_secs_f64() * 1e6 / f64::from(steps);
        assert!(d.awake().count() < 40, "{} awake", d.awake().count());
        assert!(per < 2000.0, "{per:.1} microseconds an update");
    }
}
