//! The sword schools (MAGIC 10.1) as rules over a sword.
//!
//! A school is not a different weapon: it changes what the same blade does.
//! Each form, once learned, switches on one rule. The four great schools:
//!
//! | School | Forms, in teaching order | What they do here |
//! |---|---|---|
//! | Aetherlance | Frost Step, Shaft Lance, Rimeguard, the Charge, Hrimgard's Tooth | dodges freeze the ground; the heavy looses an Ice Lance along the blade; blocks chill; the running charge; the finisher freezes the blood |
//! | Meridian | Still Water, Returning Tide, the Mirror, Noon | wider parry window; harder ripostes; parried bolts fly back; a parry at the instant of the strike answers on its own |
//! | Night Fang | Lantern-out, Low Moon, the Grieve's Turn, Shadow-step | darkness hides the blade; the opener trips; guard during a windup feints; Hollowborn step behind the lock |
//! | Gale Edge | Wind Blade, the Long Cut, Kulan's Run, Sky-Shrine | the finisher flies as a wind cut; wider arcs; charges from the saddle hit harder; the heavy whistles out as a long cut |
use crate::Rank;
use serde::{Deserialize, Serialize};

/// A sword school.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SwordSchool {
    /// Tyffir's lance-and-Rime school.
    Aetherlance,
    /// Aurelith's "Still Water": counters and parries.
    Meridian,
    /// The Nightfolk's feints and darkness.
    NightFang,
    /// Qarakhal's mounted sabre and wind-cutting.
    GaleEdge,
}

impl SwordSchool {
    /// Every school, in declaration order.
    pub const ALL: [Self; 4] = [
        Self::Aetherlance,
        Self::Meridian,
        Self::NightFang,
        Self::GaleEdge,
    ];

    /// The school's forms in teaching order.
    #[must_use]
    pub fn forms(self) -> &'static [Form] {
        match self {
            Self::Aetherlance => &[
                Form::FrostStep,
                Form::ShaftLance,
                Form::Rimeguard,
                Form::TheCharge,
                Form::HrimgardsTooth,
            ],
            Self::Meridian => &[
                Form::StillWater,
                Form::ReturningTide,
                Form::TheMirror,
                Form::Noon,
            ],
            Self::NightFang => &[
                Form::LanternOut,
                Form::LowMoon,
                Form::GrievesTurn,
                Form::ShadowStep,
            ],
            Self::GaleEdge => &[
                Form::WindBlade,
                Form::LongCut,
                Form::KulansRun,
                Form::SkyShrine,
            ],
        }
    }
}

/// A form of a school.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Form {
    /// Aetherlance: a dodge freezes water underfoot and goes further.
    FrostStep,
    /// Aetherlance: the heavy looses an Ice Lance along the blade (costs mana).
    ShaftLance,
    /// Aetherlance: a blocked attacker is chilled.
    Rimeguard,
    /// Aetherlance: the running charge (Art key, or heavy while sprinting).
    TheCharge,
    /// Aetherlance: the chain's finishing thrust freezes the blood.
    HrimgardsTooth,
    /// Meridian: the parry window widens.
    StillWater,
    /// Meridian: ripostes strike harder.
    ReturningTide,
    /// Meridian: a parried bolt or arrow flies back at its sender.
    TheMirror,
    /// Meridian: a parry at the instant of the strike counters on its own.
    Noon,
    /// Night Fang: darkness gathers around the blade (Art key): watchers see less.
    LanternOut,
    /// Night Fang: the chain's opener is a low cut that trips.
    LowMoon,
    /// Night Fang: raising the guard during a windup feints the blow.
    GrievesTurn,
    /// Night Fang: a Hollowborn's dodge steps behind the locked target.
    ShadowStep,
    /// Gale Edge: the chain's finisher flies as a wind cut.
    WindBlade,
    /// Gale Edge: every cut reaches further.
    LongCut,
    /// Gale Edge: charges and cuts from the saddle strike harder.
    KulansRun,
    /// Gale Edge: the heavy whistles out as a long flying cut.
    SkyShrine,
}

impl Form {
    fn bit(self) -> u32 {
        1 << (self as u32)
    }
}

/// What a fighter has learned of a school.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Swordsmanship {
    /// The school.
    pub school: SwordSchool,
    /// Learned forms, as bits of [`Form`].
    pub forms: u32,
    /// Rank on the seven-rank scale.
    pub rank: Rank,
}

impl Swordsmanship {
    /// A school with its first `count` forms learned.
    #[must_use]
    pub fn with_forms(school: SwordSchool, count: usize, rank: Rank) -> Self {
        let forms = school
            .forms()
            .iter()
            .take(count)
            .fold(0, |bits, f| bits | f.bit());
        Self {
            school,
            forms,
            rank,
        }
    }

    /// Whether a form is known (and belongs to this school).
    #[must_use]
    pub fn knows(&self, form: Form) -> bool {
        self.forms & form.bit() != 0 && self.school.forms().contains(&form)
    }

    /// Learn a form of this school. Returns false for another school's form.
    pub fn learn(&mut self, form: Form) -> bool {
        if self.school.forms().contains(&form) {
            self.forms |= form.bit();
            true
        } else {
            false
        }
    }

    /// Damage multiplier from rank.
    #[must_use]
    pub fn damage_scale(&self) -> f32 {
        self.rank.scale()
    }

    /// Parry window multiplier.
    #[must_use]
    pub fn parry_scale(&self) -> f32 {
        if self.knows(Form::StillWater) {
            1.6
        } else {
            1.0
        }
    }

    /// Riposte multiplier on top of the base.
    #[must_use]
    pub fn riposte_scale(&self) -> f32 {
        if self.knows(Form::ReturningTide) {
            1.5
        } else {
            1.0
        }
    }

    /// Arc reach multiplier.
    #[must_use]
    pub fn reach_scale(&self) -> f32 {
        if self.knows(Form::LongCut) { 1.3 } else { 1.0 }
    }

    /// Dodge distance multiplier.
    #[must_use]
    pub fn dodge_scale(&self) -> f32 {
        if self.knows(Form::FrostStep) {
            1.3
        } else {
            1.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms_are_learned_in_order_and_only_from_their_school() {
        let mut m = Swordsmanship::with_forms(SwordSchool::Meridian, 2, Rank::Adept);
        assert!(m.knows(Form::StillWater));
        assert!(m.knows(Form::ReturningTide));
        assert!(!m.knows(Form::TheMirror));
        assert!(!m.learn(Form::TheCharge), "not this school's form");
        assert!(!m.knows(Form::TheCharge));
        assert!(m.learn(Form::Noon));
        assert!(m.knows(Form::Noon));
        assert!(m.parry_scale() > 1.0 && m.riposte_scale() > 1.0);
        assert_eq!(m.reach_scale(), 1.0);
    }

    #[test]
    fn every_school_teaches_its_own_forms() {
        let mut seen = std::collections::BTreeSet::new();
        for school in SwordSchool::ALL {
            for form in school.forms() {
                assert!(seen.insert(*form), "{form:?} is taught by two schools");
            }
        }
        assert_eq!(seen.len(), 17);
    }
}
