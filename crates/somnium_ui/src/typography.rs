//! Phase 26-Zeta-D — typography roles.
//!
//! Before Zeta-D the editor had exactly one face (`Inter-Regular`) and every
//! call site passed the same `font_id`, so hierarchy could only be expressed
//! with size and colour. That is the single largest reason the shell reads as
//! a hobby tool: a professional editor separates a section header from a
//! property label with *weight*, not with two points of size.
//!
//! This module owns two token layers:
//!
//! * [`FontRole`] — which bundled face a run of text uses. The concrete
//!   `font_id` values are assigned by [`crate::font::FontAtlas::add_font`] at
//!   startup, so they are resolved through a process-wide [`FontRegistry`]
//!   installed once by `UiManager::new` rather than threaded through the
//!   several hundred builder call sites in `lib.rs`.
//! * [`TextRole`] — the semantic style (`section`, `body`, `label`, …) named
//!   by the approved Nocturne Atelier token sheet. A [`TextStyle`] resolves a
//!   role into the size, face, colour and tracking a widget actually needs.
//!
//! Numerics deserve a note. `fontdue` does not apply OpenType features, so the
//! `tnum` figure set the token sheet asks for cannot be switched on in the
//! proportional face. [`TextRole::MonoStrong`] instead routes numeric fields to
//! JetBrains Mono, whose figures are tabular by construction — the redline's
//! requirement ("a scrub never shifts the row") is met by the face choice
//! rather than by a feature flag.

use crate::theme::{self, Color};
use std::sync::OnceLock;

/// A bundled face. Static cuts only — no variable-font instancing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontRole {
    /// Inter Regular — body copy, menu rows, log lines.
    UiRegular,
    /// Inter Medium — selected/dirty rows, property labels, chrome buttons.
    UiMedium,
    /// Inter SemiBold — section headers, panel titles, display text.
    UiSemiBold,
    /// JetBrains Mono Regular — log timestamps, paths, console.
    Mono,
    /// JetBrains Mono Medium — numeric field values (tabular by construction).
    MonoMedium,
}

pub const FONT_ROLE_COUNT: usize = 5;

impl FontRole {
    pub const ALL: [FontRole; FONT_ROLE_COUNT] = [
        FontRole::UiRegular,
        FontRole::UiMedium,
        FontRole::UiSemiBold,
        FontRole::Mono,
        FontRole::MonoMedium,
    ];
}

/// Maps every [`FontRole`] onto a concrete atlas `font_id`.
///
/// Roles collapse onto whichever face actually loaded: if the Mono cut is
/// missing the registry points its roles at the UI face rather than at an
/// unloaded id, so text degrades to the wrong weight instead of vanishing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontRegistry {
    ids: [u8; FONT_ROLE_COUNT],
}

impl FontRegistry {
    /// Every role resolves to one face. Used before install and by unit tests
    /// that build layouts without a GPU.
    pub const fn uniform(id: u8) -> Self {
        Self {
            ids: [id; FONT_ROLE_COUNT],
        }
    }

    pub const fn new(ids: [u8; FONT_ROLE_COUNT]) -> Self {
        Self { ids }
    }

    pub const fn id(&self, role: FontRole) -> u8 {
        self.ids[role as usize]
    }

    /// Point one role at another role's face (used when a cut fails to load).
    pub fn alias(&mut self, role: FontRole, to: FontRole) {
        self.ids[role as usize] = self.ids[to as usize];
    }

    pub fn set(&mut self, role: FontRole, id: u8) {
        self.ids[role as usize] = id;
    }
}

impl Default for FontRegistry {
    fn default() -> Self {
        Self::uniform(0)
    }
}

static REGISTRY: OnceLock<FontRegistry> = OnceLock::new();

/// Publish the resolved face table. Called once, from `UiManager::new`.
/// Later calls are ignored so a second editor instance cannot repoint the
/// faces of an existing widget tree.
pub fn install_fonts(registry: FontRegistry) {
    let _ = REGISTRY.set(registry);
}

pub fn fonts() -> FontRegistry {
    REGISTRY.get().copied().unwrap_or_default()
}

/// Resolve a role to the atlas id to pass to `TextBuilder::with_font_id`.
pub fn font_id(role: FontRole) -> u8 {
    fonts().id(role)
}

/// Semantic text styles from the approved token sheet (§03 / TYPE ROLES).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextRole {
    /// 22 / 600 — splash and empty states.
    Display,
    /// 16 / 600 — modal and panel titles.
    Title,
    /// 13 / 600 — inspector section headers in sentence case.
    Section,
    /// 11 / 600 uppercase, 0.08em tracking — panel and drawer headers.
    SectionCaps,
    /// 13 / 400 — rows, menus, log.
    Body,
    /// 13 / 500 — selected and dirty rows.
    BodyStrong,
    /// 12 / 500 — property labels.
    Label,
    /// 11 / 400 — status text and metadata.
    Caption,
    /// 12 / 400 mono — log, console, paths.
    Mono,
    /// 12 / 500 mono — numeric values; tabular by face.
    MonoStrong,
}

/// A resolved style: everything `TextBuilder` needs for one run.
#[derive(Clone, Copy, Debug)]
pub struct TextStyle {
    pub px: f32,
    pub font: FontRole,
    pub color: Color,
    /// Extra advance per glyph, in logical pixels. Zero for everything except
    /// the uppercase header role.
    pub tracking: f32,
    /// Whether the role renders its text uppercased.
    pub uppercase: bool,
}

impl TextStyle {
    pub fn font_id(&self) -> u8 {
        font_id(self.font)
    }

    /// Apply the role's case transform to a label.
    pub fn transform(&self, text: &str) -> String {
        if self.uppercase {
            text.to_uppercase()
        } else {
            text.to_string()
        }
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }
}

/// Resolve a semantic role against the immutable Nocturne snapshot.
pub fn text_style(role: TextRole) -> TextStyle {
    let t = &theme::active().typography;
    let c = &theme::active().semantic.text;
    let plain = |px: f32, font: FontRole, color: Color| TextStyle {
        px,
        font,
        color,
        tracking: 0.0,
        uppercase: false,
    };
    match role {
        TextRole::Display => plain(t.display, FontRole::UiSemiBold, c.primary.bytes()),
        TextRole::Title => plain(t.title, FontRole::UiSemiBold, c.primary.bytes()),
        TextRole::Section => plain(t.section, FontRole::UiSemiBold, c.primary.bytes()),
        TextRole::SectionCaps => TextStyle {
            px: t.caption,
            font: FontRole::UiSemiBold,
            color: c.primary.bytes(),
            tracking: t.caption * 0.08,
            uppercase: true,
        },
        TextRole::Body => plain(t.body, FontRole::UiRegular, c.primary.bytes()),
        TextRole::BodyStrong => plain(t.body_strong, FontRole::UiMedium, c.primary.bytes()),
        TextRole::Label => plain(t.label, FontRole::UiMedium, c.secondary.bytes()),
        TextRole::Caption => plain(t.caption, FontRole::UiRegular, c.muted.bytes()),
        TextRole::Mono => plain(t.mono, FontRole::Mono, c.secondary.bytes()),
        TextRole::MonoStrong => plain(t.mono_strong, FontRole::MonoMedium, c.primary.bytes()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uninstalled_registry_collapses_every_role_onto_face_zero() {
        // Layout unit tests build widget trees without a GPU or a loaded atlas;
        // they must still resolve to a usable id rather than panic.
        assert_eq!(FontRegistry::default().id(FontRole::MonoMedium), 0);
    }

    #[test]
    fn alias_points_a_missing_cut_at_a_loaded_one() {
        let mut reg = FontRegistry::new([0, 1, 2, 3, 4]);
        reg.alias(FontRole::MonoMedium, FontRole::UiRegular);
        assert_eq!(reg.id(FontRole::MonoMedium), 0);
        assert_eq!(reg.id(FontRole::Mono), 3);
    }

    #[test]
    fn roles_match_the_approved_token_sheet() {
        let section = text_style(TextRole::SectionCaps);
        assert_eq!(section.px, 11.0);
        assert!(section.uppercase);
        assert!((section.tracking - 0.88).abs() < 1e-6);

        let body = text_style(TextRole::Body);
        assert_eq!(body.px, 13.0);
        assert_eq!(body.font, FontRole::UiRegular);

        let strong = text_style(TextRole::BodyStrong);
        assert_eq!(strong.px, 13.0);
        assert_eq!(strong.font, FontRole::UiMedium);

        let label = text_style(TextRole::Label);
        assert_eq!(label.px, 12.0);
        assert_eq!(label.color, theme::TEXT_SECONDARY);
    }

    #[test]
    fn numeric_values_use_a_monospaced_face_so_scrubbing_cannot_shift_a_row() {
        // The token sheet asks for tnum; fontdue applies no OpenType features,
        // so the tabular guarantee comes from the face itself.
        assert_eq!(text_style(TextRole::MonoStrong).font, FontRole::MonoMedium);
    }

    #[test]
    fn section_caps_uppercases_its_label() {
        assert_eq!(
            text_style(TextRole::SectionCaps).transform("Transform"),
            "TRANSFORM"
        );
        assert_eq!(
            text_style(TextRole::Body).transform("Transform"),
            "Transform"
        );
    }
}
