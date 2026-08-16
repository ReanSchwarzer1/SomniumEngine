//! Nocturne Atelier design tokens (Phase 26-Zeta).
//!
//! Colours in this module are authored sRGB bytes. [`UiPass`](crate::pass::UiPass)
//! decodes their RGB channels to linear exactly once before compositing into an
//! sRGB swapchain. Alpha is straight (unassociated) throughout the widget and
//! vertex APIs; the render pipeline applies the straight-alpha blend equation.
//!
//! `NOCTURNE` is the immutable theme snapshot for a frame. The legacy constants
//! below are aliases into that snapshot so existing widgets can migrate without
//! changing their event or layout behaviour.

/// Existing widget paint type. Its RGB channels are authored sRGB bytes and its
/// alpha channel is straight.
pub type Color = [u8; 4];

/// Explicit authored-sRGB colour used by the theme service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct Srgb8(pub Color);

impl Srgb8 {
    pub const fn opaque(r: u8, g: u8, b: u8) -> Self {
        Self([r, g, b, 0xFF])
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self([r, g, b, a])
    }

    pub const fn bytes(self) -> Color {
        self.0
    }
}

/// Linear colour representation for renderer-facing tests and conversions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearRgba(pub [f32; 4]);

impl LinearRgba {
    pub fn from_srgb(color: Srgb8) -> Self {
        Self(crate::color::srgb_u8_to_linear_rgba(color.0))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SurfaceTokens {
    pub window: Srgb8,
    pub canvas: Srgb8,
    pub panel: Srgb8,
    pub header: Srgb8,
    pub raised: Srgb8,
    pub input: Srgb8,
    pub popup: Srgb8,
    pub hover: Srgb8,
    pub selected: Srgb8,
    pub modal_scrim: Srgb8,
}

#[derive(Clone, Copy, Debug)]
pub struct TextTokens {
    pub primary: Srgb8,
    pub secondary: Srgb8,
    pub muted: Srgb8,
    pub disabled: Srgb8,
    pub inverse: Srgb8,
    pub link: Srgb8,
}

#[derive(Clone, Copy, Debug)]
pub struct BorderTokens {
    pub subtle: Srgb8,
    pub default: Srgb8,
    pub strong: Srgb8,
    pub focus: Srgb8,
}

#[derive(Clone, Copy, Debug)]
pub struct AccentTokens {
    pub default: Srgb8,
    pub hover: Srgb8,
    pub pressed: Srgb8,
    pub selected_bg: Srgb8,
    pub selected_rail: Srgb8,
}

#[derive(Clone, Copy, Debug)]
pub struct StatusTokens {
    pub info: Srgb8,
    pub success: Srgb8,
    pub warning: Srgb8,
    pub error: Srgb8,
    pub busy: Srgb8,
}

#[derive(Clone, Copy, Debug)]
pub struct SemanticColors {
    pub surface: SurfaceTokens,
    pub text: TextTokens,
    pub border: BorderTokens,
    pub accent: AccentTokens,
    pub status: StatusTokens,
    pub folder: Srgb8,
}

#[derive(Clone, Copy, Debug)]
pub struct TypographyTokens {
    pub display: f32,
    pub title: f32,
    pub section: f32,
    pub body: f32,
    pub body_strong: f32,
    pub label: f32,
    pub caption: f32,
    pub mono: f32,
    pub mono_strong: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct DensityTokens {
    pub row_dense: f32,
    pub row_tree: f32,
    pub row_chrome: f32,
    pub titlebar: f32,
    pub menu: f32,
    pub toolbar: f32,
    pub status: f32,
    pub icon_row: f32,
    pub icon_toolbar: f32,
    pub icon_action: f32,
    pub hit_min: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct GeometryTokens {
    pub space_base: f32,
    pub inset_panel: f32,
    pub gap_group: f32,
    pub gap_section: f32,
    pub radius_input: f32,
    pub radius_chrome: f32,
    pub radius_popup: f32,
    pub radius_modal: f32,
    pub radius_tile: f32,
    pub stroke_hairline: f32,
    pub stroke_focus: f32,
    pub stroke_rail: f32,
}

/// Opacity ladder. Straight alpha, expressed 0..1 and applied to a token's
/// alpha channel rather than to a whole widget subtree.
#[derive(Clone, Copy, Debug)]
pub struct OpacityTokens {
    pub disabled: f32,
    pub ghost: f32,
    pub scrim: f32,
    pub drop_valid: f32,
}

/// Elevation marks z-order, never decoration — panels never cast. Each level
/// is a vertical offset, a spread and a peak alpha; [`crate::draw::DrawingContext`]
/// approximates the blur with concentric translucent rings, which is enough at
/// these radii and costs no extra bind group.
#[derive(Clone, Copy, Debug)]
pub struct Elevation {
    pub offset_y: f32,
    pub spread: f32,
    pub alpha: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct ElevationTokens {
    pub popup: Elevation,
    pub drawer: Elevation,
    pub modal: Elevation,
}

#[derive(Clone, Copy, Debug)]
pub struct MotionTokens {
    pub press_ms: u64,
    pub hover_ms: u64,
    pub popup_ms: u64,
    pub drawer_ms: u64,
    pub tooltip_delay_ms: u64,
}

/// Immutable theme snapshot consumed by editor and runtime canvases.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub semantic: SemanticColors,
    pub typography: TypographyTokens,
    pub density: DensityTokens,
    pub geometry: GeometryTokens,
    pub motion: MotionTokens,
    pub opacity: OpacityTokens,
    pub elevation: ElevationTokens,
}

pub const NOCTURNE: Theme = Theme {
    semantic: SemanticColors {
        surface: SurfaceTokens {
            window: Srgb8::opaque(0x14, 0x16, 0x1C),
            canvas: Srgb8::opaque(0x18, 0x1A, 0x20),
            panel: Srgb8::opaque(0x1C, 0x1E, 0x26),
            header: Srgb8::opaque(0x22, 0x25, 0x2F),
            raised: Srgb8::opaque(0x28, 0x2B, 0x36),
            input: Srgb8::opaque(0x12, 0x14, 0x1A),
            popup: Srgb8::opaque(0x22, 0x25, 0x2F),
            hover: Srgb8::opaque(0x31, 0x35, 0x43),
            selected: Srgb8::rgba(0x7A, 0x86, 0xFF, 0x29),
            modal_scrim: Srgb8::rgba(0x0A, 0x0B, 0x0F, 0x9E),
        },
        text: TextTokens {
            primary: Srgb8::opaque(0xD8, 0xDC, 0xE8),
            secondary: Srgb8::opaque(0x9A, 0xA3, 0xB5),
            muted: Srgb8::opaque(0x7E, 0x86, 0x98),
            disabled: Srgb8::opaque(0x5C, 0x64, 0x78),
            inverse: Srgb8::opaque(0x0E, 0x10, 0x14),
            link: Srgb8::opaque(0x94, 0x9C, 0xFF),
        },
        border: BorderTokens {
            subtle: Srgb8::opaque(0x22, 0x25, 0x2F),
            default: Srgb8::opaque(0x31, 0x35, 0x43),
            strong: Srgb8::opaque(0x4A, 0x4F, 0x5E),
            focus: Srgb8::opaque(0x94, 0x9C, 0xFF),
        },
        accent: AccentTokens {
            default: Srgb8::opaque(0x7A, 0x86, 0xFF),
            hover: Srgb8::opaque(0x94, 0x9C, 0xFF),
            pressed: Srgb8::opaque(0x5C, 0x68, 0xE0),
            selected_bg: Srgb8::rgba(0x7A, 0x86, 0xFF, 0x29),
            selected_rail: Srgb8::opaque(0x7A, 0x86, 0xFF),
        },
        status: StatusTokens {
            info: Srgb8::opaque(0x59, 0xB8, 0xD6),
            success: Srgb8::opaque(0x5D, 0xCE, 0x9A),
            warning: Srgb8::opaque(0xE6, 0xB0, 0x4A),
            error: Srgb8::opaque(0xE0, 0x5A, 0x5A),
            busy: Srgb8::opaque(0x7A, 0x86, 0xFF),
        },
        folder: Srgb8::opaque(0xC4, 0xA5, 0x74),
    },
    typography: TypographyTokens {
        display: 22.0,
        title: 16.0,
        section: 13.0,
        body: 13.0,
        body_strong: 13.0,
        label: 12.0,
        caption: 11.0,
        mono: 12.0,
        mono_strong: 12.0,
    },
    density: DensityTokens {
        row_dense: 24.0,
        row_tree: 26.0,
        row_chrome: 30.0,
        titlebar: 36.0,
        menu: 28.0,
        toolbar: 32.0,
        status: 26.0,
        icon_row: 16.0,
        icon_toolbar: 20.0,
        icon_action: 24.0,
        hit_min: 24.0,
    },
    geometry: GeometryTokens {
        space_base: 4.0,
        inset_panel: 8.0,
        gap_group: 12.0,
        gap_section: 16.0,
        radius_input: 2.0,
        radius_chrome: 2.0,
        radius_popup: 4.0,
        radius_modal: 6.0,
        radius_tile: 3.0,
        stroke_hairline: 1.0,
        stroke_focus: 1.0,
        stroke_rail: 2.0,
    },
    motion: MotionTokens {
        press_ms: 90,
        hover_ms: 120,
        popup_ms: 140,
        drawer_ms: 200,
        tooltip_delay_ms: 400,
    },
    opacity: OpacityTokens {
        disabled: 0.38,
        ghost: 0.60,
        scrim: 0.62,
        drop_valid: 0.18,
    },
    elevation: ElevationTokens {
        popup: Elevation {
            offset_y: 8.0,
            spread: 12.0,
            alpha: 0.45,
        },
        drawer: Elevation {
            offset_y: -12.0,
            spread: 16.0,
            alpha: 0.40,
        },
        modal: Elevation {
            offset_y: 24.0,
            spread: 32.0,
            alpha: 0.60,
        },
    },
};

pub const fn frame_theme() -> &'static Theme {
    &NOCTURNE
}

pub const TRANSPARENT: Color = [0x00, 0x00, 0x00, 0x00];
pub const BLACK: Color = [0x00, 0x00, 0x00, 0xFF];
pub const WHITE: Color = [0xFF, 0xFF, 0xFF, 0xFF];

// Compatibility aliases. New component recipes should read from NOCTURNE.
pub const BG_VOID: Color = NOCTURNE.semantic.surface.window.bytes();
pub const BG_CONTENT: Color = NOCTURNE.semantic.surface.canvas.bytes();
pub const BG_PANEL: Color = NOCTURNE.semantic.surface.panel.bytes();
pub const BG_HEADER: Color = NOCTURNE.semantic.surface.header.bytes();
pub const BG_RAISED: Color = NOCTURNE.semantic.surface.raised.bytes();
pub const BG_INPUT: Color = NOCTURNE.semantic.surface.input.bytes();
pub const BG_HOVER: Color = NOCTURNE.semantic.surface.hover.bytes();
pub const BG_DARK: Color = BG_VOID;

pub const BORDER_DARK: Color = NOCTURNE.semantic.border.subtle.bytes();
pub const BORDER_MEDIUM: Color = NOCTURNE.semantic.border.default.bytes();
pub const BORDER_LIGHT: Color = NOCTURNE.semantic.border.strong.bytes();
pub const BORDER_FOCUS: Color = NOCTURNE.semantic.border.focus.bytes();

pub const ACCENT: Color = NOCTURNE.semantic.accent.default.bytes();
pub const ACCENT_HOVER: Color = NOCTURNE.semantic.accent.hover.bytes();
pub const ACCENT_PRESSED: Color = NOCTURNE.semantic.accent.pressed.bytes();
pub const ACCENT_DIM: Color = NOCTURNE.semantic.accent.selected_bg.bytes();
pub const ACCENT_BLUE: Color = ACCENT;
pub const FOLDER_SAND: Color = NOCTURNE.semantic.folder.bytes();

/// Palette `moon` — the one step above `text.primary`, reserved for the label
/// of a selected or hovered row so selection reads as a lift, not a recolour.
pub const MOON: Color = [0xF2, 0xF4, 0xFF, 0xFF];

pub const TEXT_PRIMARY: Color = NOCTURNE.semantic.text.primary.bytes();
pub const TEXT_SECONDARY: Color = NOCTURNE.semantic.text.secondary.bytes();
pub const TEXT_MUTED: Color = NOCTURNE.semantic.text.muted.bytes();
pub const TEXT_DISABLED: Color = NOCTURNE.semantic.text.disabled.bytes();
pub const TEXT_LINK: Color = NOCTURNE.semantic.text.link.bytes();

pub const STATUS_INFO: Color = NOCTURNE.semantic.status.info.bytes();
pub const STATUS_OK: Color = NOCTURNE.semantic.status.success.bytes();
pub const STATUS_WARN: Color = NOCTURNE.semantic.status.warning.bytes();
pub const STATUS_ERROR: Color = NOCTURNE.semantic.status.error.bytes();

pub const RADIUS_CHROME: f32 = NOCTURNE.geometry.radius_chrome;
pub const RADIUS_POPUP: f32 = NOCTURNE.geometry.radius_popup;
pub const ROW_HEIGHT: f32 = NOCTURNE.density.row_dense;
pub const TREE_ROW_HEIGHT: f32 = NOCTURNE.density.row_tree;
pub const TOOLBAR_HEIGHT: f32 = NOCTURNE.density.toolbar;
pub const MENU_HEIGHT: f32 = NOCTURNE.density.menu;
pub const TITLEBAR_HEIGHT: f32 = NOCTURNE.density.titlebar;
pub const STATUS_HEIGHT: f32 = NOCTURNE.density.status;
pub const BOTTOM_DRAWER_HEIGHT: f32 = 220.0;
pub const SPLITTER_THICKNESS: f32 = 6.0;
pub const TOOLTIP_DELAY_MS: u64 = NOCTURNE.motion.tooltip_delay_ms;
pub const ICON_TOOL: f32 = NOCTURNE.density.icon_toolbar;
pub const ICON_TREE: f32 = NOCTURNE.density.icon_row;
/// The engine mark. Larger than `icon_action` because it is a brand element,
/// not a control: at 24 px it read as one more toolbar glyph beside the
/// wordmark instead of as the thing the wordmark belongs to.
pub const ICON_MARK: f32 = 30.0;
pub const ICON_CHECK: f32 = 16.0;
pub const ICON_DRAWER: f32 = 80.0;

/// Replace a token's alpha channel. Alpha stays straight — the pipeline
/// premultiplies at blend time (see [`crate::pass::UiPass`]).
pub const fn with_alpha(color: Color, alpha: u8) -> Color {
    [color[0], color[1], color[2], alpha]
}

/// Scale a token's alpha by a factor from [`OpacityTokens`].
pub fn scaled_alpha(color: Color, factor: f32) -> Color {
    let a = (color[3] as f32 * factor.clamp(0.0, 1.0)).round();
    with_alpha(color, a as u8)
}

/// Blend `top` over `bottom` in authored sRGB space using `top`'s straight
/// alpha. Used where a translucent wash has to be flattened into one opaque
/// fill because the widget below it is drawn in the same pass.
pub fn flatten(bottom: Color, top: Color) -> Color {
    let a = top[3] as f32 / 255.0;
    let ch = |i: usize| ((top[i] as f32 * a) + (bottom[i] as f32 * (1.0 - a))).round() as u8;
    [ch(0), ch(1), ch(2), bottom[3]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_token_is_the_approved_authored_srgb_value() {
        assert_eq!(BG_PANEL, [0x1C, 0x1E, 0x26, 0xFF]);
    }

    #[test]
    fn selected_fill_keeps_straight_alpha() {
        assert_eq!(ACCENT_DIM, [0x7A, 0x86, 0xFF, 0x29]);
    }
}
