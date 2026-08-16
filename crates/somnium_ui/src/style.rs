//! Phase 26-Zeta-C — component style recipes.
//!
//! [`crate::theme`] holds the *tokens*; this module holds the *recipes* that
//! turn a component plus an interaction state into concrete paint. Widgets ask
//! for `style::button(state)` instead of choosing between `BG_RAISED` and
//! `BG_HOVER` themselves, which is what stops the token layer from being
//! cosmetic (phase_26_Zeta §6.1, risk row "theme constants remain cosmetic").
//!
//! The state grammar is the design package's, and deliberately small — four
//! cues carry every surface in the editor:
//!
//! * a **hover wash** (never a border change, which would reflow the row);
//! * a **1 px focus ring** in `focus.ring`;
//! * a **2 px selection rail** in `accent.selected_rail`, always paired with
//!   the translucent selection fill so selection never relies on colour alone;
//! * a **gutter dot** for modified.

use crate::theme::{self, Color};

/// Interaction state of one component instance.
///
/// Ordering matters when several apply at once: `Disabled` wins over
/// everything, then `Pressed`, `Selected`, `Hover`, `Rest`. Focus is not a
/// member because it composes — a control can be focused *and* hovered — so it
/// is carried separately by [`VisualState::focused`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Interaction {
    #[default]
    Rest,
    Hover,
    Pressed,
    /// Selected row, active tab, armed mode button.
    Selected,
    Disabled,
}

/// Everything a recipe needs to resolve paint for one instance.
#[derive(Clone, Copy, Debug, Default)]
pub struct VisualState {
    pub interaction: Interaction,
    /// Keyboard focus. Draws a ring; does not change the fill.
    pub focused: bool,
    /// Value differs from the component default — draws the gutter dot.
    pub modified: bool,
    /// Validation failed. Recolours the outline, never the fill.
    pub invalid: bool,
}

impl VisualState {
    pub fn rest() -> Self {
        Self::default()
    }

    pub fn with(interaction: Interaction) -> Self {
        Self {
            interaction,
            ..Self::default()
        }
    }

    pub fn focused(mut self, on: bool) -> Self {
        self.focused = on;
        self
    }

    pub fn modified(mut self, on: bool) -> Self {
        self.modified = on;
        self
    }

    pub fn invalid(mut self, on: bool) -> Self {
        self.invalid = on;
        self
    }

    pub fn is_disabled(&self) -> bool {
        self.interaction == Interaction::Disabled
    }
}

/// Resolved paint for one component instance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paint {
    pub background: Color,
    pub foreground: Color,
    /// Outline colour. Equal to `background` where the recipe has no outline.
    pub border: Color,
    pub border_thickness: f32,
    /// 2 px left strip; `None` unless the instance is selected or errored.
    pub rail: Option<Color>,
    pub radius: f32,
}

impl Paint {
    fn flat(background: Color, foreground: Color) -> Self {
        Self {
            background,
            foreground,
            border: background,
            border_thickness: 0.0,
            rail: None,
            radius: theme::NOCTURNE.geometry.radius_chrome,
        }
    }

    fn outlined(background: Color, foreground: Color, border: Color) -> Self {
        Self {
            border,
            border_thickness: theme::NOCTURNE.geometry.stroke_hairline,
            ..Self::flat(background, foreground)
        }
    }

    /// Apply the shared focus and disabled overrides every recipe honours.
    fn finish(mut self, state: &VisualState) -> Self {
        if state.invalid {
            self.border = theme::STATUS_ERROR;
            self.border_thickness = theme::NOCTURNE.geometry.stroke_hairline;
        }
        if state.focused {
            self.border = theme::NOCTURNE.semantic.border.focus.bytes();
            self.border_thickness = theme::NOCTURNE.geometry.stroke_focus;
        }
        if state.is_disabled() {
            self.foreground = theme::TEXT_DISABLED;
        }
        self
    }
}

/// `Button` and `MenuItem`: a raised chrome surface that washes on hover and
/// darkens (rather than moves) on press.
pub fn button(state: VisualState) -> Paint {
    let s = &theme::NOCTURNE.semantic;
    let base = match state.interaction {
        Interaction::Rest => Paint::flat(s.surface.raised.bytes(), s.text.primary.bytes()),
        Interaction::Hover => Paint::flat(s.surface.hover.bytes(), s.text.primary.bytes()),
        Interaction::Pressed => Paint::flat(s.surface.input.bytes(), s.text.primary.bytes()),
        Interaction::Selected => Paint {
            rail: Some(s.accent.selected_rail.bytes()),
            ..Paint::flat(
                theme::flatten(s.surface.raised.bytes(), s.accent.selected_bg.bytes()),
                theme::TEXT_PRIMARY,
            )
        },
        Interaction::Disabled => Paint::flat(s.surface.panel.bytes(), s.text.disabled.bytes()),
    };
    base.finish(&state)
}

/// The one primary action on a surface — Save scene, Save and continue.
/// There is never more than one visible at a time.
pub fn primary_button(state: VisualState) -> Paint {
    let a = &theme::NOCTURNE.semantic.accent;
    let fill = match state.interaction {
        Interaction::Hover => a.hover.bytes(),
        Interaction::Pressed => a.pressed.bytes(),
        Interaction::Disabled => theme::NOCTURNE.semantic.surface.panel.bytes(),
        _ => a.default.bytes(),
    };
    let fg = if state.is_disabled() {
        theme::TEXT_DISABLED
    } else {
        theme::NOCTURNE.semantic.text.inverse.bytes()
    };
    Paint::flat(fill, fg).finish(&state)
}

/// Icon-only toolbar control. Transparent at rest so the command bands read as
/// one surface; the 30 px hit box is a layout concern, not a paint one.
pub fn icon_button(state: VisualState) -> Paint {
    let s = &theme::NOCTURNE.semantic;
    let base = match state.interaction {
        Interaction::Rest => Paint::flat(theme::TRANSPARENT, s.text.secondary.bytes()),
        Interaction::Hover => Paint::flat(s.surface.hover.bytes(), s.text.primary.bytes()),
        Interaction::Pressed => Paint::flat(s.surface.input.bytes(), s.text.primary.bytes()),
        // Active mode is the one place an icon takes the accent as foreground.
        Interaction::Selected => Paint {
            rail: Some(s.accent.selected_rail.bytes()),
            ..Paint::flat(s.accent.selected_bg.bytes(), s.accent.hover.bytes())
        },
        Interaction::Disabled => Paint::flat(theme::TRANSPARENT, s.text.disabled.bytes()),
    };
    base.finish(&state)
}

/// Recessed text / numeric / search field.
pub fn input(state: VisualState) -> Paint {
    let s = &theme::NOCTURNE.semantic;
    let base = match state.interaction {
        Interaction::Disabled => Paint::outlined(
            s.surface.panel.bytes(),
            s.text.disabled.bytes(),
            s.surface.raised.bytes(),
        ),
        Interaction::Hover => Paint::outlined(
            s.surface.input.bytes(),
            s.text.primary.bytes(),
            s.border.strong.bytes(),
        ),
        _ => Paint::outlined(
            s.surface.input.bytes(),
            s.text.primary.bytes(),
            s.border.default.bytes(),
        ),
    };
    Paint {
        radius: theme::NOCTURNE.geometry.radius_input,
        ..base
    }
    .finish(&state)
}

/// Outliner / content-tree row. Selection is fill **and** rail together, so it
/// survives a colour-vision simulation.
pub fn tree_row(state: VisualState) -> Paint {
    let s = &theme::NOCTURNE.semantic;
    let base = match state.interaction {
        Interaction::Rest => Paint::flat(theme::TRANSPARENT, s.text.primary.bytes()),
        Interaction::Hover => Paint::flat(s.surface.hover.bytes(), theme::MOON),
        Interaction::Pressed | Interaction::Selected => Paint {
            rail: Some(s.accent.selected_rail.bytes()),
            ..Paint::flat(s.accent.selected_bg.bytes(), theme::MOON)
        },
        // "Hidden" and "locked" both arrive here: dimmed, never removed.
        Interaction::Disabled => Paint::flat(theme::TRANSPARENT, s.text.disabled.bytes()),
    };
    let mut paint = Paint {
        radius: 0.0,
        ..base
    }
    .finish(&state);
    if state.invalid {
        paint.background = theme::with_alpha(theme::STATUS_ERROR, 0x1A);
        paint.foreground = theme::STATUS_ERROR;
        paint.rail = Some(theme::STATUS_ERROR);
        paint.border_thickness = 0.0;
    }
    paint
}

/// Content Browser tile.
pub fn asset_tile(state: VisualState) -> Paint {
    let s = &theme::NOCTURNE.semantic;
    let base = match state.interaction {
        Interaction::Hover => Paint::outlined(
            s.surface.canvas.bytes(),
            s.text.primary.bytes(),
            s.border.strong.bytes(),
        ),
        Interaction::Selected | Interaction::Pressed => Paint::outlined(
            s.surface.header.bytes(),
            theme::MOON,
            s.accent.default.bytes(),
        ),
        Interaction::Disabled => Paint::outlined(
            s.surface.canvas.bytes(),
            s.text.disabled.bytes(),
            s.border.subtle.bytes(),
        ),
        Interaction::Rest => Paint::outlined(
            s.surface.canvas.bytes(),
            s.text.primary.bytes(),
            s.border.default.bytes(),
        ),
    };
    Paint {
        radius: theme::NOCTURNE.geometry.radius_tile,
        ..base
    }
    .finish(&state)
}

/// Drop-target feedback for a drag in flight. Valid outlines indigo over an
/// 18 % fill; invalid outlines rose and the reason goes to the status bar.
pub fn drop_target(valid: bool) -> Paint {
    let accent = if valid {
        theme::NOCTURNE.semantic.accent.default.bytes()
    } else {
        theme::STATUS_ERROR
    };
    let fill = theme::with_alpha(
        accent,
        (255.0 * theme::NOCTURNE.opacity.drop_valid).round() as u8,
    );
    Paint {
        background: fill,
        foreground: accent,
        border: accent,
        border_thickness: theme::NOCTURNE.geometry.stroke_rail,
        rail: None,
        radius: theme::NOCTURNE.geometry.radius_tile,
    }
}

/// Menu, combo list, tooltip and palette share one popup surface.
pub fn popup() -> Paint {
    let s = &theme::NOCTURNE.semantic;
    Paint {
        radius: theme::NOCTURNE.geometry.radius_popup,
        ..Paint::outlined(
            s.surface.popup.bytes(),
            s.text.primary.bytes(),
            s.border.default.bytes(),
        )
    }
}

/// Status colour for a log or notification severity, in one place so a status
/// never becomes a raw hex at a call site.
pub fn status(kind: StatusKind) -> Color {
    match kind {
        StatusKind::Info => theme::STATUS_INFO,
        StatusKind::Success => theme::STATUS_OK,
        StatusKind::Warning => theme::STATUS_WARN,
        StatusKind::Error => theme::STATUS_ERROR,
        StatusKind::Busy => theme::NOCTURNE.semantic.status.busy.bytes(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusKind {
    Info,
    Success,
    Warning,
    Error,
    Busy,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_is_never_carried_by_colour_alone() {
        // Acceptance matrix §10.3: no required state is communicated by colour
        // alone. Every selected recipe must also produce the rail.
        for paint in [
            tree_row(VisualState::with(Interaction::Selected)),
            button(VisualState::with(Interaction::Selected)),
            icon_button(VisualState::with(Interaction::Selected)),
        ] {
            assert!(paint.rail.is_some());
        }
    }

    #[test]
    fn hover_changes_the_wash_not_the_outline() {
        // A border change on hover reflows the row; the design forbids it.
        let rest = tree_row(VisualState::rest());
        let hover = tree_row(VisualState::with(Interaction::Hover));
        assert_ne!(rest.background, hover.background);
        assert_eq!(rest.border_thickness, hover.border_thickness);
    }

    #[test]
    fn focus_draws_the_ring_over_any_interaction() {
        for interaction in [Interaction::Rest, Interaction::Hover, Interaction::Selected] {
            let paint = button(VisualState::with(interaction).focused(true));
            assert_eq!(paint.border, theme::BORDER_FOCUS);
            assert_eq!(
                paint.border_thickness,
                theme::NOCTURNE.geometry.stroke_focus
            );
        }
    }

    #[test]
    fn invalid_recolours_the_outline_and_keeps_the_fill() {
        let rest = input(VisualState::rest());
        let bad = input(VisualState::rest().invalid(true));
        assert_eq!(rest.background, bad.background);
        assert_eq!(bad.border, theme::STATUS_ERROR);
    }

    #[test]
    fn disabled_beats_every_other_interaction_for_foreground() {
        assert_eq!(
            button(VisualState::with(Interaction::Disabled)).foreground,
            theme::TEXT_DISABLED
        );
        assert_eq!(
            input(VisualState::with(Interaction::Disabled)).foreground,
            theme::TEXT_DISABLED
        );
    }

    #[test]
    fn drop_targets_separate_valid_from_invalid_by_hue_and_by_reason() {
        assert_eq!(drop_target(true).border, theme::ACCENT);
        assert_eq!(drop_target(false).border, theme::STATUS_ERROR);
        assert_eq!(drop_target(true).border_thickness, 2.0);
    }

    #[test]
    fn error_rows_override_the_selection_rail_with_the_error_hue() {
        let row = tree_row(VisualState::rest().invalid(true));
        assert_eq!(row.rail, Some(theme::STATUS_ERROR));
    }
}
