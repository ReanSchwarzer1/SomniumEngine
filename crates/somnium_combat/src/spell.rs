//! Spells as requests with six parts (MAGIC 2.2).
//!
//! | Part | Here |
//! |---|---|
//! | 1. Address, who is asked | [`Address`]: whose power pays, the caster's well, a held crystal, or a chorus of allies whose capacities add |
//! | 2. Element | [`Element`] |
//! | 3. Shape | [`Shape`]: bolt, lance, wall, wave, step, bloom, ward |
//! | 4. Measure | [`Measure`]: size, range, speed, count, duration; snapped to rote measures unless the caster knows the base-twelve numbers |
//! | 5. Bearing | [`Bearing`]: at the cursor's point, at a target, around self |
//! | 6. Release | [`CastSpeed`]: incantation (slow, reliable, cheaper), short-chant (fast), silent (instant, costs focus) |
//!
//! A [`SpellAsset`] is the data a designer writes: which shapes a spell
//! allows, its measure limits, power, cost and timings. A [`SpellRequest`] is
//! what a caster asks for this time. The same Ice Lance asset makes a
//! needle-thin, hawk-fast lance or a wide slow one; [`SpellAsset::cost`] makes
//! the wide one pay for its width.
use crate::status::StatusKind;
use crate::{CombatantId, DamageType, Rank};
use glam::Vec3;
use serde::{Deserialize, Serialize};

/// The nine disciplines (MAGIC 4), for ranks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Discipline {
    /// *agin*.
    Flame,
    /// *hima*: water and ice.
    Rime,
    /// *vāyu*.
    Gale,
    /// *bhūmi*.
    Stone,
    /// *sanjīvana*.
    Mending,
    /// *raksha*: force.
    Warding,
    /// *āhvāna*: summoning and binding.
    Calling,
    /// *lekha*: written spells.
    Sigilcraft,
    /// *ātma-vidyā*: forbidden.
    Soulcraft,
}

impl Discipline {
    /// Every discipline, in index order.
    pub const ALL: [Self; 9] = [
        Self::Flame,
        Self::Rime,
        Self::Gale,
        Self::Stone,
        Self::Mending,
        Self::Warding,
        Self::Calling,
        Self::Sigilcraft,
        Self::Soulcraft,
    ];

    /// Index into per-discipline tables.
    #[must_use]
    pub fn index(self) -> usize {
        self as usize
    }
}

/// Part 2: the kind of mana-work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Element {
    /// Fire.
    Flame,
    /// Ice and cold (Rime).
    Frost,
    /// Water (Rime): soaks, slams, puts out fire.
    Water,
    /// Wind.
    Gale,
    /// Earth and rock.
    Stone,
    /// Healing.
    Mending,
    /// Force (Warding): shields, force walls, a raw push of the Wheel's power.
    Force,
}

impl Element {
    /// Every element, in declaration order.
    pub const ALL: [Self; 7] = [
        Self::Flame,
        Self::Frost,
        Self::Water,
        Self::Gale,
        Self::Stone,
        Self::Mending,
        Self::Force,
    ];

    /// The discipline whose rank governs this element.
    #[must_use]
    pub fn discipline(self) -> Discipline {
        match self {
            Self::Flame => Discipline::Flame,
            Self::Frost | Self::Water => Discipline::Rime,
            Self::Gale => Discipline::Gale,
            Self::Stone => Discipline::Stone,
            Self::Mending => Discipline::Mending,
            Self::Force => Discipline::Warding,
        }
    }

    /// The harm this element does; `None` for Mending, which heals.
    #[must_use]
    pub fn damage_type(self) -> Option<DamageType> {
        match self {
            Self::Flame => Some(DamageType::Flame),
            Self::Frost => Some(DamageType::Frost),
            Self::Water => Some(DamageType::Blunt),
            Self::Gale => Some(DamageType::Gale),
            Self::Stone => Some(DamageType::Stone),
            Self::Mending => None,
            Self::Force => Some(DamageType::Mana),
        }
    }

    /// The status a hit of this element carries by its nature, before the
    /// asset's own list.
    #[must_use]
    pub fn natural_status(self) -> Option<(StatusKind, f32)> {
        match self {
            Self::Flame => Some((StatusKind::Burn, 4.0)),
            Self::Frost => Some((StatusKind::Chill, 25.0)),
            Self::Water => Some((StatusKind::Wet, 8.0)),
            _ => None,
        }
    }
}

/// Part 3: the form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Shape {
    /// A missile.
    Bolt,
    /// A fast piercing missile that passes through bodies.
    Lance,
    /// A standing wall across the bearing.
    Wall,
    /// A front that sweeps outward in an arc.
    Wave,
    /// The caster moves: a dash, a leap, a skate.
    Step,
    /// A burst at a point, after a breath's warning.
    Bloom,
    /// A shield on the bearer.
    Ward,
}

impl Shape {
    /// Every shape, in declaration order.
    pub const ALL: [Self; 7] = [
        Self::Bolt,
        Self::Lance,
        Self::Wall,
        Self::Wave,
        Self::Step,
        Self::Bloom,
        Self::Ward,
    ];

    /// Cost weight relative to a bolt.
    #[must_use]
    pub fn cost_weight(self) -> f32 {
        match self {
            Self::Bolt => 1.0,
            Self::Lance => 1.5,
            Self::Wall => 2.0,
            Self::Wave => 2.2,
            Self::Step => 0.7,
            Self::Bloom => 2.6,
            Self::Ward => 1.6,
        }
    }

    /// Damage (or healing, or absorption) weight relative to a bolt.
    #[must_use]
    pub fn power_weight(self) -> f32 {
        match self {
            Self::Bolt => 1.0,
            Self::Lance => 1.6,
            Self::Wall => 0.35,
            Self::Wave => 0.6,
            Self::Step => 0.3,
            Self::Bloom => 0.9,
            Self::Ward => 3.0,
        }
    }
}

/// Part 4: how much, how far, how fast, how many, how long.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Measure {
    /// Radius or width, metres.
    pub size: f32,
    /// How far, metres.
    pub range: f32,
    /// How fast, metres per second.
    pub speed: f32,
    /// How many.
    pub count: u8,
    /// How long it stands (walls, wards, burning ground), seconds.
    pub duration: f32,
}

impl Measure {
    /// The reference the cost and power curves are written against.
    pub const REFERENCE: Self = Self {
        size: 1.0,
        range: 20.0,
        speed: 20.0,
        count: 1,
        duration: 4.0,
    };
}

/// Which measure the mouse wheel edits while a spell is being shaped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MeasureAxis {
    /// Size.
    Size,
    /// Range.
    Range,
    /// Speed.
    Speed,
    /// Count.
    Count,
    /// Duration.
    Duration,
}

impl MeasureAxis {
    /// The next axis, for cycling.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Size => Self::Range,
            Self::Range => Self::Speed,
            Self::Speed => Self::Count,
            Self::Count => Self::Duration,
            Self::Duration => Self::Size,
        }
    }
}

/// The measures a caster without numbers can say, smallest first, with their
/// Common names (MAGIC 2.2: most learn fixed measures by rote).
pub const ROTE_SIZE: [(f32, &str); 7] = [
    (0.25, "a hand"),
    (0.5, "a forearm"),
    (1.0, "a span"),
    (2.0, "a spear's length"),
    (4.0, "a cart"),
    (8.0, "a house"),
    (16.0, "a hall"),
];
/// Rote ranges.
pub const ROTE_RANGE: [(f32, &str); 5] = [
    (5.0, "a stride"),
    (12.0, "a stone's throw"),
    (25.0, "a short bowshot"),
    (50.0, "a bowshot"),
    (100.0, "a horn-call"),
];
/// Rote speeds.
pub const ROTE_SPEED: [(f32, &str); 4] = [
    (4.0, "a walker"),
    (12.0, "a horse"),
    (30.0, "a hawk"),
    (60.0, "a falling star"),
];
/// Rote durations.
pub const ROTE_DURATION: [(f32, &str); 4] = [
    (2.0, "a breath"),
    (6.0, "a prayer"),
    (15.0, "a song"),
    (40.0, "a watch"),
];
/// The rote litany counts only to three; more needs the base-twelve numbers.
pub const ROTE_MAX_COUNT: u8 = 3;
/// Base twelve: the most the numbered Measure accepts.
pub const NUMBERED_MAX_COUNT: u8 = 12;

fn snap(value: f32, rote: &[(f32, &str)], lo: f32, hi: f32) -> f32 {
    // Nearest in log space among the rote steps inside the limits; the limit
    // itself when none fits.
    let inside: Vec<f32> = rote
        .iter()
        .map(|r| r.0)
        .filter(|v| *v >= lo && *v <= hi)
        .collect();
    if inside.is_empty() {
        return value.clamp(lo, hi);
    }
    let lv = value.max(1e-3).ln();
    inside
        .into_iter()
        .min_by(|a, b| (a.ln() - lv).abs().total_cmp(&(b.ln() - lv).abs()))
        .unwrap_or(lo)
}

/// The rote name of a value, when it is one.
#[must_use]
pub fn rote_name(value: f32, rote: &[(f32, &'static str)]) -> Option<&'static str> {
    rote.iter()
        .find(|r| (r.0 - value).abs() < 1e-3)
        .map(|r| r.1)
}

/// Part 5: where.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Bearing {
    /// At a point on the ground (the cursor's).
    Cursor(#[serde(with = "crate::serde_vec3")] Vec3),
    /// At a combatant.
    Target(CombatantId),
    /// Centred on the caster.
    AroundSelf,
}

/// Part 6: the release, which is also the casting speed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CastSpeed {
    /// The full verse: slow, reliable (only a stagger breaks it), cheaper.
    Incantation,
    /// The six working words: fast; a hard blow breaks it.
    ShortChant,
    /// Intent alone: instant, dearer, and it costs focus.
    Silent,
}

impl CastSpeed {
    /// Every speed, in declaration order.
    pub const ALL: [Self; 3] = [Self::Incantation, Self::ShortChant, Self::Silent];

    fn index(self) -> usize {
        self as usize
    }

    /// Bit in [`Magic::speeds`].
    #[must_use]
    pub fn bit(self) -> u8 {
        1 << self.index()
    }

    fn mana_weight(self) -> f32 {
        match self {
            Self::Incantation => 0.85,
            Self::ShortChant => 1.0,
            Self::Silent => 1.4,
        }
    }
}

/// Part 1: who is asked, which here means whose power pays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Address {
    /// The caster's own well.
    Well,
    /// A crystal the caster holds ([`Magic::crystal`]).
    Crystal,
    /// A chorus: the caster and these allies chant one verse; the cost is
    /// shared by capacity and the limits grow with the chorus (MAGIC 2.4).
    Chorus(Vec<CombatantId>),
}

/// A whole request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpellRequest {
    /// The asset this request uses.
    pub spell: String,
    /// 1. Address.
    pub address: Address,
    /// 2. Element (must be the asset's).
    pub element: Element,
    /// 3. Shape.
    pub shape: Shape,
    /// 4. Measure.
    pub measure: Measure,
    /// 5. Bearing.
    pub bearing: Bearing,
    /// 6. Release.
    pub release: CastSpeed,
}

/// Measure limits of an asset at its own rank.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeasureLimits {
    /// Size, min and max.
    pub size: [f32; 2],
    /// Range, min and max.
    pub range: [f32; 2],
    /// Speed, min and max.
    pub speed: [f32; 2],
    /// Most allowed.
    pub count: u8,
    /// Duration, min and max.
    pub duration: [f32; 2],
}

/// A spell as authored data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpellAsset {
    /// Stable id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// The element; the discipline follows from it.
    pub element: Element,
    /// Rank needed in the discipline.
    pub rank: Rank,
    /// Shapes this spell takes.
    pub shapes: Vec<Shape>,
    /// The shape, measure and release a hotbar slot starts with.
    pub default_shape: Shape,
    /// Default measure.
    pub default_measure: Measure,
    /// Default release.
    pub default_release: CastSpeed,
    /// Limits at the asset's rank; each rank above widens size and range by a quarter.
    pub limits: MeasureLimits,
    /// Damage (or healing, or absorption) per hit at the reference measure, for a bolt.
    pub power: f32,
    /// Lamps at the reference measure, for a bolt, by short-chant.
    pub cost: f32,
    /// Seconds of gathering per [`CastSpeed`].
    pub gather: [f32; 3],
    /// Seconds after release before the caster acts again.
    pub recover: f32,
    /// Seconds before the spell may be cast again.
    pub cooldown: f32,
    /// Lamps per second to hold a channel; zero means it cannot be channelled.
    pub channel_cost: f32,
    /// Status build-up on a hit, after the element's own.
    pub status: Vec<(StatusKind, f32)>,
    /// Poise damage per hit.
    pub poise: f32,
    /// Push, metres per second.
    pub knockback: f32,
    /// How far the casting is heard (the full verse is loud).
    pub loudness: f32,
}

/// What a request will cost and take.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpellCost {
    /// Lamps.
    pub mana: f32,
    /// Seconds of gathering.
    pub gather: f32,
    /// Seconds of recovery.
    pub recover: f32,
    /// Focus spent (silent casting).
    pub focus: f32,
    /// Damage, healing or absorption per hit.
    pub power: f32,
}

/// Why a request was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpellError {
    /// No asset with that id.
    UnknownSpell,
    /// The request's element is not the asset's.
    WrongElement,
    /// The asset does not take that shape.
    ShapeNotAllowed,
    /// The caster's rank in the discipline is too low.
    RankTooLow,
    /// The caster has not learned that release.
    SpeedNotLearned,
    /// A measure is outside what this caster may ask for.
    MeasureOutOfBounds(MeasureAxis),
    /// A chorus member is missing, dead or not an ally.
    BadChorus,
    /// Still cooling down.
    Cooldown,
    /// Not enough mana and overdraw is off (or past the floor).
    NoMana,
    /// Not enough focus for a silent cast.
    NoFocus,
    /// The bearing names nobody, or is out of range.
    BadBearing,
    /// The caster cannot act now (frozen, staggered, down, dead, busy).
    Busy,
}

/// A caster's magical learning.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Magic {
    /// Rank per [`Discipline::index`]; `None` is untrained.
    pub ranks: [Option<Rank>; 9],
    /// Knows the base-twelve number words (measures take numbers).
    pub numbers: bool,
    /// Learned releases, as [`CastSpeed::bit`]s.
    pub speeds: u8,
    /// Spells on the number keys 1 to 0.
    pub hotbar: [Option<HotbarSlot>; 10],
    /// Whether casting past empty is allowed.
    pub allow_overdraw: bool,
    /// Lamps in a held crystal, for [`Address::Crystal`].
    pub crystal: f32,
}

impl Default for Magic {
    fn default() -> Self {
        Self {
            ranks: [None; 9],
            numbers: false,
            speeds: CastSpeed::Incantation.bit(),
            hotbar: Default::default(),
            allow_overdraw: false,
            crystal: 0.0,
        }
    }
}

impl Magic {
    /// Rank in a discipline.
    #[must_use]
    pub fn rank(&self, discipline: Discipline) -> Option<Rank> {
        self.ranks[discipline.index()]
    }

    /// Builder: a rank.
    #[must_use]
    pub fn with_rank(mut self, discipline: Discipline, rank: Rank) -> Self {
        self.ranks[discipline.index()] = Some(rank);
        self
    }

    /// Whether a release is learned.
    #[must_use]
    pub fn knows_speed(&self, speed: CastSpeed) -> bool {
        self.speeds & speed.bit() != 0
    }
}

/// A spell on a number key: the asset and the shape it was last left in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HotbarSlot {
    /// Asset id.
    pub spell: String,
    /// Shape.
    pub shape: Shape,
    /// Measure.
    pub measure: Measure,
    /// Release.
    pub release: CastSpeed,
}

impl SpellAsset {
    /// The hotbar slot this asset starts as.
    #[must_use]
    pub fn slot(&self) -> HotbarSlot {
        HotbarSlot {
            spell: self.id.clone(),
            shape: self.default_shape,
            measure: self.default_measure,
            release: self.default_release,
        }
    }

    fn widened(&self, caster_rank: Rank, chorus: usize) -> MeasureLimits {
        let above = caster_rank.steps_above(self.rank) as f32;
        let widen = 1.25f32.powf(above) * (1.0 + 0.5 * chorus as f32);
        MeasureLimits {
            size: [self.limits.size[0], self.limits.size[1] * widen],
            range: [self.limits.range[0], self.limits.range[1] * widen],
            ..self.limits
        }
    }

    /// Fit a measure to what a caster can say: clamp to the limits, and snap
    /// to rote steps unless the caster knows the numbers.
    #[must_use]
    pub fn fit(&self, measure: Measure, magic: &Magic, chorus: usize) -> Measure {
        let rank = magic
            .rank(self.element.discipline())
            .unwrap_or(Rank::Novice);
        let l = self.widened(rank, chorus);
        let max_count = if magic.numbers {
            l.count.min(NUMBERED_MAX_COUNT)
        } else {
            l.count.min(ROTE_MAX_COUNT)
        }
        .max(1);
        let mut m = Measure {
            size: measure.size.clamp(l.size[0], l.size[1]),
            range: measure.range.clamp(l.range[0], l.range[1]),
            speed: measure.speed.clamp(l.speed[0], l.speed[1]),
            count: measure.count.clamp(1, max_count),
            duration: measure.duration.clamp(l.duration[0], l.duration[1]),
        };
        if !magic.numbers {
            m.size = snap(m.size, &ROTE_SIZE, l.size[0], l.size[1]);
            m.range = snap(m.range, &ROTE_RANGE, l.range[0], l.range[1]);
            m.speed = snap(m.speed, &ROTE_SPEED, l.speed[0], l.speed[1]);
            m.duration = snap(m.duration, &ROTE_DURATION, l.duration[0], l.duration[1]);
        }
        m
    }

    /// Check a request against this asset and the caster's learning. Bearing,
    /// mana, focus, cooldown and chorus membership are the world's to check.
    pub fn validate(&self, request: &SpellRequest, magic: &Magic) -> Result<(), SpellError> {
        if request.spell != self.id {
            return Err(SpellError::UnknownSpell);
        }
        if request.element != self.element {
            return Err(SpellError::WrongElement);
        }
        if !self.shapes.contains(&request.shape) {
            return Err(SpellError::ShapeNotAllowed);
        }
        let rank = magic
            .rank(self.element.discipline())
            .ok_or(SpellError::RankTooLow)?;
        if rank < self.rank {
            return Err(SpellError::RankTooLow);
        }
        if !magic.knows_speed(request.release) {
            return Err(SpellError::SpeedNotLearned);
        }
        let chorus = match &request.address {
            Address::Chorus(members) => members.len(),
            _ => 0,
        };
        let fitted = self.fit(request.measure, magic, chorus);
        let m = request.measure;
        let tolerance = |a: f32, b: f32| (a - b).abs() <= 1e-3 * b.abs().max(1.0);
        for (axis, ok) in [
            (MeasureAxis::Size, tolerance(m.size, fitted.size)),
            (MeasureAxis::Range, tolerance(m.range, fitted.range)),
            (MeasureAxis::Speed, tolerance(m.speed, fitted.speed)),
            (MeasureAxis::Count, m.count == fitted.count),
            (
                MeasureAxis::Duration,
                tolerance(m.duration, fitted.duration),
            ),
        ] {
            if !ok {
                return Err(SpellError::MeasureOutOfBounds(axis));
            }
        }
        Ok(())
    }

    /// The cost, timings and power of a request (assumed valid).
    #[must_use]
    pub fn cost(&self, request: &SpellRequest) -> SpellCost {
        let r = Measure::REFERENCE;
        let m = request.measure;
        let size = (m.size / r.size).max(0.05);
        let lasting = matches!(request.shape, Shape::Wall | Shape::Ward | Shape::Bloom);
        let duration = if lasting {
            (m.duration / r.duration).max(0.1).powf(0.6)
        } else {
            1.0
        };
        let count = f32::from(m.count.max(1));
        let mana = self.cost
            * request.shape.cost_weight()
            * size.powf(1.4)
            * (m.range / r.range).max(0.1).sqrt()
            * (m.speed / r.speed).max(0.1).powf(0.35)
            * count.powf(0.9)
            * duration
            * request.release.mana_weight();
        let chorus = match &request.address {
            Address::Chorus(members) => members.len() as f32,
            _ => 0.0,
        };
        let gather = self.gather[request.release.index()]
            * (1.0 + 0.3 * size.max(1.0).log2() + 0.1 * (count - 1.0))
            + if chorus > 0.0 { 0.6 } else { 0.0 };
        let focus = if request.release == CastSpeed::Silent {
            15.0 + 5.0 * count
        } else {
            0.0
        };
        let power = self.power
            * request.shape.power_weight()
            * size.powf(0.6)
            * (m.speed / r.speed).max(0.1).powf(0.25);
        SpellCost {
            mana,
            gather,
            recover: self.recover,
            focus,
            power,
        }
    }
}

/// Rimeborn's spell spine (MAGIC 4), as defaults a designer starts from.
pub mod library {
    use super::*;

    fn limits(size: [f32; 2], range: [f32; 2], speed: [f32; 2], count: u8) -> MeasureLimits {
        MeasureLimits {
            size,
            range,
            speed,
            count,
            duration: [2.0, 40.0],
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn asset(
        id: &str,
        name: &str,
        element: Element,
        rank: Rank,
        shapes: &[Shape],
        measure: Measure,
        limits: MeasureLimits,
        power: f32,
        cost: f32,
    ) -> SpellAsset {
        SpellAsset {
            id: id.into(),
            name: name.into(),
            element,
            rank,
            shapes: shapes.to_vec(),
            default_shape: shapes[0],
            default_measure: measure,
            default_release: CastSpeed::Incantation,
            limits,
            power,
            cost,
            gather: [1.4, 0.5, 0.0],
            recover: 0.35,
            cooldown: 0.8,
            channel_cost: 0.0,
            status: Vec::new(),
            poise: 12.0,
            knockback: 1.0,
            loudness: 0.5,
        }
    }

    fn m(size: f32, range: f32, speed: f32, count: u8, duration: f32) -> Measure {
        Measure {
            size,
            range,
            speed,
            count,
            duration,
        }
    }

    /// Every library spell.
    #[must_use]
    pub fn all() -> Vec<SpellAsset> {
        use Element::*;
        use Shape::*;
        let mut list = vec![
            asset(
                "ember_bolt",
                "Ember Bolt",
                Flame,
                Rank::Novice,
                &[Bolt],
                m(0.5, 25.0, 30.0, 1, 2.0),
                limits([0.25, 1.0], [5.0, 30.0], [12.0, 30.0], 3),
                16.0,
                2.0,
            ),
            asset(
                "flame_lance",
                "Flame Lance",
                Flame,
                Rank::Adept,
                &[Lance, Bolt],
                m(0.5, 50.0, 60.0, 1, 2.0),
                limits([0.25, 2.0], [12.0, 50.0], [30.0, 60.0], 3),
                22.0,
                4.0,
            ),
            asset(
                "fireburst",
                "Fireburst",
                Flame,
                Rank::Advanced,
                &[Bloom, Wave],
                m(4.0, 25.0, 12.0, 1, 2.0),
                limits([1.0, 8.0], [5.0, 25.0], [4.0, 30.0], 3),
                26.0,
                6.0,
            ),
            asset(
                "burning_ground",
                "Burning Ground",
                Flame,
                Rank::Advanced,
                &[Wall, Bloom],
                m(4.0, 12.0, 12.0, 1, 6.0),
                limits([1.0, 8.0], [5.0, 25.0], [4.0, 12.0], 1),
                14.0,
                6.0,
            ),
            asset(
                "water_bolt",
                "Water Bolt",
                Water,
                Rank::Novice,
                &[Bolt, Wave],
                m(0.5, 25.0, 30.0, 1, 2.0),
                limits([0.25, 2.0], [5.0, 25.0], [12.0, 30.0], 3),
                10.0,
                1.5,
            ),
            asset(
                "frost_touch",
                "Frost Touch",
                Frost,
                Rank::Novice,
                &[Bolt],
                m(0.25, 5.0, 12.0, 1, 2.0),
                limits([0.25, 0.5], [5.0, 5.0], [12.0, 12.0], 1),
                8.0,
                1.0,
            ),
            asset(
                "ice_lance",
                "Ice Lance",
                Frost,
                Rank::Adept,
                &[Lance, Bolt],
                m(0.5, 50.0, 60.0, 1, 2.0),
                limits([0.25, 2.0], [12.0, 50.0], [12.0, 60.0], 3),
                24.0,
                4.0,
            ),
            asset(
                "frost_step",
                "Frost Step",
                Frost,
                Rank::Adept,
                &[Step],
                m(1.0, 12.0, 12.0, 1, 6.0),
                limits([0.5, 2.0], [5.0, 12.0], [12.0, 30.0], 1),
                6.0,
                2.0,
            ),
            asset(
                "ice_wall",
                "Ice Wall",
                Frost,
                Rank::Advanced,
                &[Wall],
                m(4.0, 12.0, 12.0, 1, 15.0),
                limits([2.0, 8.0], [5.0, 25.0], [4.0, 12.0], 1),
                0.0,
                5.0,
            ),
            asset(
                "rime_bloom",
                "Rime Bloom",
                Frost,
                Rank::Advanced,
                &[Bloom, Wave],
                m(4.0, 25.0, 12.0, 1, 2.0),
                limits([1.0, 8.0], [5.0, 25.0], [4.0, 30.0], 3),
                14.0,
                6.0,
            ),
            asset(
                "gust",
                "Gust",
                Gale,
                Rank::Novice,
                &[Wave, Bolt],
                m(2.0, 12.0, 30.0, 1, 2.0),
                limits([0.5, 4.0], [5.0, 12.0], [12.0, 30.0], 1),
                6.0,
                1.5,
            ),
            asset(
                "wind_blade",
                "Wind Blade",
                Gale,
                Rank::Adept,
                &[Bolt, Wave],
                m(1.0, 25.0, 30.0, 1, 2.0),
                limits([0.5, 2.0], [5.0, 25.0], [12.0, 60.0], 3),
                18.0,
                3.0,
            ),
            asset(
                "gale_wall",
                "Gale Wall",
                Gale,
                Rank::Advanced,
                &[Wall],
                m(4.0, 12.0, 12.0, 1, 15.0),
                limits([2.0, 8.0], [5.0, 25.0], [4.0, 12.0], 1),
                0.0,
                5.0,
            ),
            asset(
                "updraft",
                "Updraft",
                Gale,
                Rank::Advanced,
                &[Step],
                m(1.0, 12.0, 30.0, 1, 2.0),
                limits([0.5, 2.0], [5.0, 12.0], [12.0, 30.0], 1),
                0.0,
                2.0,
            ),
            asset(
                "pebble_bolt",
                "Pebble Bolt",
                Stone,
                Rank::Novice,
                &[Bolt],
                m(0.5, 25.0, 30.0, 1, 2.0),
                limits([0.25, 1.0], [5.0, 25.0], [12.0, 30.0], 3),
                14.0,
                1.5,
            ),
            asset(
                "stone_skin",
                "Stone Skin",
                Stone,
                Rank::Adept,
                &[Ward],
                m(1.0, 5.0, 12.0, 1, 15.0),
                limits([0.5, 2.0], [5.0, 5.0], [12.0, 12.0], 1),
                12.0,
                4.0,
            ),
            asset(
                "raise_wall",
                "Raise Wall",
                Stone,
                Rank::Advanced,
                &[Wall],
                m(4.0, 12.0, 12.0, 1, 15.0),
                limits([2.0, 8.0], [5.0, 25.0], [4.0, 12.0], 1),
                0.0,
                5.0,
            ),
            asset(
                "quake",
                "Quake",
                Stone,
                Rank::Saint,
                &[Bloom, Wave],
                m(8.0, 12.0, 12.0, 1, 2.0),
                limits([2.0, 16.0], [5.0, 25.0], [4.0, 12.0], 1),
                22.0,
                10.0,
            ),
            asset(
                "close_wound",
                "Close Wound",
                Mending,
                Rank::Novice,
                &[Bolt, Bloom],
                m(0.5, 12.0, 30.0, 1, 2.0),
                limits([0.25, 4.0], [5.0, 25.0], [12.0, 30.0], 1),
                30.0,
                3.0,
            ),
            asset(
                "shield",
                "Shield",
                Force,
                Rank::Novice,
                &[Ward],
                m(0.5, 5.0, 12.0, 1, 6.0),
                limits([0.25, 1.0], [5.0, 12.0], [12.0, 12.0], 1),
                8.0,
                2.0,
            ),
            asset(
                "ward",
                "Ward",
                Force,
                Rank::Adept,
                &[Wall, Ward],
                m(4.0, 12.0, 12.0, 1, 15.0),
                limits([1.0, 8.0], [5.0, 25.0], [4.0, 12.0], 1),
                10.0,
                5.0,
            ),
            asset(
                "force_bolt",
                "Force Bolt",
                Force,
                Rank::Adept,
                &[Bolt, Lance],
                m(0.5, 25.0, 30.0, 1, 2.0),
                limits([0.25, 1.0], [5.0, 50.0], [12.0, 60.0], 3),
                15.0,
                3.0,
            ),
        ];
        for spell in &mut list {
            match spell.id.as_str() {
                "fireburst" | "rime_bloom" | "quake" => {
                    if spell.id == "quake" {
                        spell.status = vec![(StatusKind::Knockdown, 1.4)];
                    }
                    if spell.id == "rime_bloom" {
                        // Freeze everything in a circle: a wet body freezes outright.
                        spell.status = vec![(StatusKind::Chill, 40.0)];
                    }
                    spell.poise = 30.0;
                    spell.cooldown = 4.0;
                    spell.gather = [2.2, 0.9, 0.0];
                    spell.loudness = 0.8;
                }
                "ice_wall" | "raise_wall" | "gale_wall" | "ward" | "burning_ground" => {
                    spell.cooldown = 6.0;
                    spell.gather = [1.8, 0.7, 0.0];
                    spell.channel_cost = 1.0;
                }
                "frost_step" | "updraft" => {
                    spell.gather = [0.6, 0.15, 0.0];
                    spell.recover = 0.1;
                    spell.cooldown = 2.0;
                }
                "gust" => {
                    spell.knockback = 9.0;
                    spell.poise = 25.0;
                }
                "water_bolt" => spell.knockback = 3.0,
                "pebble_bolt" => spell.poise = 28.0,
                "ice_lance" => spell.status = vec![(StatusKind::Chill, 15.0)],
                "close_wound" | "shield" | "stone_skin" => spell.cooldown = 3.0,
                _ => {}
            }
        }
        list
    }

    /// One library spell by id.
    #[must_use]
    pub fn get(id: &str) -> Option<SpellAsset> {
        all().into_iter().find(|s| s.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mage(numbers: bool) -> Magic {
        let mut m = Magic::default().with_rank(Discipline::Rime, Rank::Adept);
        m.numbers = numbers;
        m.speeds = CastSpeed::Incantation.bit() | CastSpeed::ShortChant.bit();
        m
    }

    fn request(asset: &SpellAsset, measure: Measure) -> SpellRequest {
        SpellRequest {
            spell: asset.id.clone(),
            address: Address::Well,
            element: asset.element,
            shape: asset.default_shape,
            measure,
            bearing: Bearing::AroundSelf,
            release: CastSpeed::ShortChant,
        }
    }

    #[test]
    fn a_wide_slow_lance_costs_more_than_a_needle() {
        let lance = library::get("ice_lance").unwrap();
        let needle = request(
            &lance,
            Measure {
                size: 0.25,
                speed: 60.0,
                ..lance.default_measure
            },
        );
        let wide = request(
            &lance,
            Measure {
                size: 2.0,
                speed: 12.0,
                ..lance.default_measure
            },
        );
        let (n, w) = (lance.cost(&needle), lance.cost(&wide));
        assert!(w.mana > n.mana * 3.0, "{} vs {}", w.mana, n.mana);
        assert!(w.gather > n.gather, "a bigger verse takes longer");
        assert!(w.power > n.power, "and hits harder");
    }

    #[test]
    fn releases_trade_speed_for_cost_and_focus() {
        let bolt = library::get("water_bolt").unwrap();
        let mut r = request(&bolt, bolt.default_measure);
        let costs: Vec<SpellCost> = CastSpeed::ALL
            .iter()
            .map(|speed| {
                r.release = *speed;
                bolt.cost(&r)
            })
            .collect();
        assert!(costs[0].gather > costs[1].gather && costs[1].gather > costs[2].gather);
        assert!(costs[0].mana < costs[1].mana && costs[1].mana < costs[2].mana);
        assert_eq!(costs[0].focus, 0.0);
        assert!(costs[2].focus > 0.0, "silent casting costs focus");
    }

    #[test]
    fn without_numbers_measures_snap_to_rote_and_count_stops_at_three() {
        let lance = library::get("ice_lance").unwrap();
        let wish = Measure {
            size: 0.6,
            range: 31.0,
            speed: 40.0,
            count: 7,
            duration: 3.0,
        };
        let rote = lance.fit(wish, &mage(false), 0);
        assert_eq!(rote.size, 0.5);
        assert_eq!(rote.range, 25.0);
        assert_eq!(rote.speed, 30.0);
        assert_eq!(rote.count, ROTE_MAX_COUNT);
        assert_eq!(rote_name(rote.size, &ROTE_SIZE), Some("a forearm"));
        let numbered = lance.fit(wish, &mage(true), 0);
        assert_eq!(numbered.size, 0.6);
        assert_eq!(numbered.range, 31.0);
        assert_eq!(numbered.count, 3, "the asset's own limit still applies");
    }

    #[test]
    fn validation_names_what_is_wrong() {
        let lance = library::get("ice_lance").unwrap();
        let magic = mage(true);
        let mut r = request(&lance, lance.default_measure);
        assert_eq!(lance.validate(&r, &magic), Ok(()));
        r.shape = Shape::Wall;
        assert_eq!(lance.validate(&r, &magic), Err(SpellError::ShapeNotAllowed));
        r.shape = Shape::Lance;
        r.measure.size = 9.0;
        assert_eq!(
            lance.validate(&r, &magic),
            Err(SpellError::MeasureOutOfBounds(MeasureAxis::Size))
        );
        r.measure.size = 0.5;
        r.release = CastSpeed::Silent;
        assert_eq!(lance.validate(&r, &magic), Err(SpellError::SpeedNotLearned));
        r.release = CastSpeed::ShortChant;
        let wall = library::get("ice_wall").unwrap();
        let w = request(&wall, wall.default_measure);
        assert_eq!(wall.validate(&w, &magic), Err(SpellError::RankTooLow));
        r.element = Element::Flame;
        assert_eq!(lance.validate(&r, &magic), Err(SpellError::WrongElement));
    }

    #[test]
    fn higher_rank_and_a_chorus_widen_the_limits() {
        let lance = library::get("ice_lance").unwrap();
        let big = Measure {
            size: 3.0,
            ..lance.default_measure
        };
        let adept = mage(true);
        assert_eq!(lance.fit(big, &adept, 0).size, 2.0);
        let saint = mage(true).with_rank(Discipline::Rime, Rank::Saint);
        assert!(lance.fit(big, &saint, 0).size >= 3.0 - 1e-4);
        assert!(lance.fit(big, &adept, 2).size >= 3.0 - 1e-4);
    }

    #[test]
    fn every_library_spell_validates_its_own_defaults() {
        for spell in library::all() {
            let mut magic = Magic::default();
            magic.ranks = [Some(Rank::Sovereign); 9];
            magic.speeds = 0b111;
            magic.numbers = true;
            let r = request(&spell, spell.default_measure);
            assert_eq!(spell.validate(&r, &magic), Ok(()), "{}", spell.id);
            let rote = Magic {
                numbers: false,
                ..magic.clone()
            };
            let fitted = spell.fit(spell.default_measure, &rote, 0);
            assert_eq!(
                fitted, spell.default_measure,
                "{} defaults are rote",
                spell.id
            );
            let json = serde_json::to_string(&spell).unwrap();
            let back: SpellAsset = serde_json::from_str(&json).unwrap();
            assert_eq!(back, spell);
        }
    }
}
