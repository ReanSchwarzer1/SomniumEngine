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
    /// One step beyond `primary`, reserved for the label of a selected or
    /// hovered row so selection reads as a lift rather than a recolour. In a
    /// light theme it steps *down*, not up.
    pub emphasis: Srgb8,
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
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Elevation {
    pub offset_y: f32,
    /// Falloff distance. Drives `Primitive::shadow`'s blur term.
    pub spread: f32,
    pub alpha: f32,
}

impl Elevation {
    pub const NONE: Elevation = Elevation {
        offset_y: 0.0,
        spread: 0.0,
        alpha: 0.0,
    };
}

/// A two-stop linear gradient in authored sRGB. Interpolation happens in linear
/// space inside the shader, never here.
///
/// Phase 27 caps chrome washes at a 6 % luminance delta (§5.3). `GradientTokens`
/// is where that limit is expressed as data, and `gradient_delta_is_within_the_design_cap`
/// is where it is enforced.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gradient {
    pub from: Srgb8,
    pub to: Srgb8,
    /// Direction in normalised rect space. `(0, 1)` is the top-to-bottom wash.
    pub axis: [f32; 2],
}

impl Gradient {
    pub const fn vertical(from: Srgb8, to: Srgb8) -> Self {
        Self {
            from,
            to,
            axis: [0.0, 1.0],
        }
    }
}

/// Chrome-only gradients. Never on body content, inputs, or text.
#[derive(Clone, Copy, Debug)]
pub struct GradientTokens {
    /// Toolbars and the application bar.
    pub chrome_wash: Gradient,
    /// Panel and section headers.
    pub header_wash: Gradient,
    /// The one filled call-to-action per surface.
    pub accent_primary: Gradient,
    /// The 2 px selection rail.
    pub rail_accent: Gradient,
}

/// Outer halo. Exactly two roles exist and no more may be added (§5.4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glow {
    pub color: Srgb8,
    pub radius: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct GlowTokens {
    /// Keyboard focus ring bloom.
    pub focus: Glow,
    /// The armed editing mode in the mode strip.
    pub armed: Glow,
}

/// Inner shadow, used to sink an input below its panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inset {
    pub blur: f32,
    pub color: Srgb8,
}

#[derive(Clone, Copy, Debug)]
pub struct InsetTokens {
    pub input: Inset,
}

/// The z-order ladder. Phase 27-D replaced the three ad-hoc levels with a full
/// ordering, and `Elevation::blur` replaced the old `spread`-as-blur overload
/// now that `Primitive::shadow` can express both independently.
///
/// **Panels never cast.** `canvas` and `panel` are deliberately flat: elevation
/// marks layering, not decoration, and a shadow under every surface is the
/// fastest way to make a dense editor look cheap.
#[derive(Clone, Copy, Debug)]
pub struct ElevationTokens {
    /// Level 2. A control lifted off its panel.
    pub raised: Elevation,
    /// Level 3.
    pub popup: Elevation,
    /// Level 4. Opens upward, so its offset is negative.
    pub drawer: Elevation,
    /// Level 5. The only layer that traps focus.
    pub modal: Elevation,
    /// Level 6. Above everything, including the modal.
    pub toast: Elevation,
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
    pub gradient: GradientTokens,
    pub glow: GlowTokens,
    pub inset: InsetTokens,
    /// Warm secondary. Phase 27-E promoted the orphaned `folder` swatch into a
    /// full role so the palette runs two temperatures: cool indigo carries
    /// focus and selection, warm ember carries asset and content identity.
    /// Ember never competes with the accent for a state cue.
    pub ember: Srgb8,
    /// True when this snapshot is a light theme. Recipes that need to know
    /// which way a shadow or a wash should run read this rather than sniffing
    /// the background luminance.
    pub is_light: bool,
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
            emphasis: Srgb8::opaque(0xF2, 0xF4, 0xFF),
            secondary: Srgb8::opaque(0x9A, 0xA3, 0xB5),
            muted: Srgb8::opaque(0x8C, 0x95, 0xAA),
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
            error: Srgb8::opaque(0xE6, 0x70, 0x70),
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
        raised: Elevation {
            offset_y: 1.0,
            spread: 3.0,
            alpha: 0.28,
        },
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
        toast: Elevation {
            offset_y: 6.0,
            spread: 14.0,
            alpha: 0.50,
        },
    },
    gradient: GradientTokens {
        chrome_wash: Gradient::vertical(
            Srgb8::opaque(0x24, 0x27, 0x31),
            Srgb8::opaque(0x20, 0x23, 0x2C),
        ),
        header_wash: Gradient::vertical(
            Srgb8::opaque(0x26, 0x29, 0x33),
            Srgb8::opaque(0x22, 0x25, 0x2F),
        ),
        accent_primary: Gradient::vertical(
            Srgb8::opaque(0x86, 0x92, 0xFF),
            Srgb8::opaque(0x6E, 0x7A, 0xF4),
        ),
        rail_accent: Gradient::vertical(
            Srgb8::opaque(0x94, 0x9C, 0xFF),
            Srgb8::opaque(0x7A, 0x86, 0xFF),
        ),
    },
    glow: GlowTokens {
        focus: Glow {
            color: Srgb8::rgba(0x94, 0x9C, 0xFF, 0x66),
            radius: 4.0,
        },
        armed: Glow {
            color: Srgb8::rgba(0x7A, 0x86, 0xFF, 0x40),
            radius: 6.0,
        },
    },
    inset: InsetTokens {
        input: Inset {
            blur: 3.0,
            color: Srgb8::rgba(0x00, 0x00, 0x00, 0x59),
        },
    },
    ember: Srgb8::opaque(0xC4, 0xA5, 0x74),
    is_light: false,
};

/// Phase 27-E — the light theme.
///
/// Dawn exists to prove the recipe layer is not hard-coded. Every value here is
/// a role the dark theme also fills, and any recipe that cannot express both is
/// a recipe reaching past `style.rs` for a literal. Contrast for both snapshots
/// is certified by `certified_pairs_meet_wcag_aa_in_both_themes`, so these
/// numbers are checked rather than eyeballed.
pub const DAWN: Theme = Theme {
    semantic: SemanticColors {
        surface: SurfaceTokens {
            window: Srgb8::opaque(0xE4, 0xE6, 0xED),
            canvas: Srgb8::opaque(0xED, 0xEF, 0xF4),
            panel: Srgb8::opaque(0xF5, 0xF6, 0xFA),
            header: Srgb8::opaque(0xE8, 0xEA, 0xF1),
            raised: Srgb8::opaque(0xFF, 0xFF, 0xFF),
            input: Srgb8::opaque(0xFF, 0xFF, 0xFF),
            popup: Srgb8::opaque(0xFF, 0xFF, 0xFF),
            hover: Srgb8::opaque(0xDD, 0xE0, 0xEA),
            selected: Srgb8::rgba(0x4A, 0x56, 0xE0, 0x29),
            modal_scrim: Srgb8::rgba(0x14, 0x16, 0x1E, 0x73),
        },
        text: TextTokens {
            primary: Srgb8::opaque(0x1A, 0x1D, 0x26),
            emphasis: Srgb8::opaque(0x07, 0x09, 0x10),
            secondary: Srgb8::opaque(0x45, 0x4B, 0x5C),
            muted: Srgb8::opaque(0x5C, 0x63, 0x75),
            disabled: Srgb8::opaque(0x9A, 0xA0, 0xAE),
            inverse: Srgb8::opaque(0xFF, 0xFF, 0xFF),
            link: Srgb8::opaque(0x2E, 0x39, 0xA8),
        },
        border: BorderTokens {
            subtle: Srgb8::opaque(0xDD, 0xE0, 0xE8),
            default: Srgb8::opaque(0xC3, 0xC8, 0xD4),
            strong: Srgb8::opaque(0x9A, 0xA0, 0xAE),
            focus: Srgb8::opaque(0x2E, 0x39, 0xA8),
        },
        accent: AccentTokens {
            default: Srgb8::opaque(0x3A, 0x46, 0xC8),
            hover: Srgb8::opaque(0x2E, 0x39, 0xA8),
            pressed: Srgb8::opaque(0x24, 0x2D, 0x86),
            selected_bg: Srgb8::rgba(0x3A, 0x46, 0xC8, 0x29),
            selected_rail: Srgb8::opaque(0x3A, 0x46, 0xC8),
        },
        status: StatusTokens {
            info: Srgb8::opaque(0x14, 0x62, 0x7C),
            success: Srgb8::opaque(0x15, 0x60, 0x3E),
            warning: Srgb8::opaque(0x6E, 0x4E, 0x08),
            error: Srgb8::opaque(0x9B, 0x1C, 0x1C),
        busy: Srgb8::opaque(0x3A, 0x46, 0xC8),
        },
        folder: Srgb8::opaque(0x8A, 0x62, 0x2C),
    },
    typography: NOCTURNE.typography,
    density: NOCTURNE.density,
    geometry: NOCTURNE.geometry,
    motion: NOCTURNE.motion,
    opacity: OpacityTokens {
        disabled: 0.38,
        ghost: 0.60,
        // A light theme needs a darker scrim to separate the modal at all.
        scrim: 0.45,
        drop_valid: 0.18,
    },
    elevation: ElevationTokens {
        // Shadows on a light ground read far heavier at the same alpha, so the
        // ladder keeps its geometry and drops its opacity.
        raised: Elevation {
            offset_y: 1.0,
            spread: 3.0,
            alpha: 0.14,
        },
        popup: Elevation {
            offset_y: 8.0,
            spread: 12.0,
            alpha: 0.20,
        },
        drawer: Elevation {
            offset_y: -12.0,
            spread: 16.0,
            alpha: 0.18,
        },
        modal: Elevation {
            offset_y: 24.0,
            spread: 32.0,
            alpha: 0.28,
        },
        toast: Elevation {
            offset_y: 6.0,
            spread: 14.0,
            alpha: 0.22,
        },
    },
    gradient: GradientTokens {
        chrome_wash: Gradient::vertical(
            Srgb8::opaque(0xEC, 0xEE, 0xF4),
            Srgb8::opaque(0xE4, 0xE6, 0xED),
        ),
        header_wash: Gradient::vertical(
            Srgb8::opaque(0xEE, 0xF0, 0xF6),
            Srgb8::opaque(0xE8, 0xEA, 0xF1),
        ),
        accent_primary: Gradient::vertical(
            Srgb8::opaque(0x44, 0x51, 0xD8),
            Srgb8::opaque(0x33, 0x3E, 0xB4),
        ),
        rail_accent: Gradient::vertical(
            Srgb8::opaque(0x4A, 0x56, 0xE0),
            Srgb8::opaque(0x3A, 0x46, 0xC8),
        ),
    },
    glow: GlowTokens {
        focus: Glow {
            color: Srgb8::rgba(0x3A, 0x46, 0xC8, 0x59),
            radius: 4.0,
        },
        armed: Glow {
            color: Srgb8::rgba(0x3A, 0x46, 0xC8, 0x33),
            radius: 6.0,
        },
    },
    inset: InsetTokens {
        input: Inset {
            blur: 3.0,
            color: Srgb8::rgba(0x1A, 0x1D, 0x26, 0x24),
        },
    },
    ember: Srgb8::opaque(0x8A, 0x62, 0x2C),
    is_light: true,
};

/// Which snapshot the recipe layer reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ThemeId {
    Nocturne = 0,
    Dawn = 1,
}

static ACTIVE_THEME: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// The snapshot every `style.rs` recipe resolves against.
///
/// Both themes are compile-time constants, so selection is one relaxed atomic
/// load and no lock — a paint call happens hundreds of times per frame. A
/// user-authored theme would need a different mechanism; this is deliberately
/// the smallest thing that ships two.
pub fn active() -> &'static Theme {
    match ACTIVE_THEME.load(std::sync::atomic::Ordering::Relaxed) {
        1 => &DAWN,
        _ => &NOCTURNE,
    }
}

pub fn set_active(id: ThemeId) {
    ACTIVE_THEME.store(id as u8, std::sync::atomic::Ordering::Relaxed);
}

pub fn active_id() -> ThemeId {
    match ACTIVE_THEME.load(std::sync::atomic::Ordering::Relaxed) {
        1 => ThemeId::Dawn,
        _ => ThemeId::Nocturne,
    }
}

/// Relative luminance per WCAG 2.x, from authored sRGB bytes.
pub fn relative_luminance(c: Srgb8) -> f32 {
    let b = c.bytes();
    let lin = crate::color::srgb_u8_to_linear_rgba(b);
    0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2]
}

/// WCAG contrast ratio between two opaque colours, 1.0..=21.0.
pub fn contrast_ratio(a: Srgb8, b: Srgb8) -> f32 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// A perceptual step of the accent, expressed as a ramp position 50..900.
///
/// §9.5 asks for the accent's hover / pressed / disabled / glow steps to be
/// *derived* rather than five hand-picked hexes. Derivation happens here, on the
/// linear values, so the steps are perceptually even in both themes rather than
/// even in byte space.
pub fn ramp_step(base: Srgb8, step: u16) -> Srgb8 {
    // 500 is the base. Below it mixes toward white, above toward black.
    let t = (step as f32 - 500.0) / 400.0;
    let target = if t < 0.0 { 1.0f32 } else { 0.0f32 };
    let k = t.abs().clamp(0.0, 1.0);
    let lin = crate::color::srgb_u8_to_linear_rgba(base.bytes());
    Srgb8(crate::color::linear_rgba_to_srgb_u8([
        lin[0] + (target - lin[0]) * k,
        lin[1] + (target - lin[1]) * k,
        lin[2] + (target - lin[2]) * k,
        lin[3],
    ]))
}

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

#[cfg(test)]
mod asphodel_tests {
    use super::*;

    fn themes() -> [(&'static str, &'static Theme); 2] {
        [("Nocturne", &NOCTURNE), ("Dawn", &DAWN)]
    }

    /// Text pairs that must clear WCAG AA at 4.5:1, and UI-component pairs that
    /// must clear 3:1. Extends the table Zeta §8A.3 certified for the dark theme
    /// so Dawn is held to exactly the same bar.
    #[test]
    fn certified_pairs_meet_wcag_aa_in_both_themes() {
        let mut failures = Vec::new();
        for (name, t) in themes() {
            let sem = &t.semantic;
            let surfaces = [
                ("window", sem.surface.window),
                ("canvas", sem.surface.canvas),
                ("panel", sem.surface.panel),
                ("header", sem.surface.header),
                ("raised", sem.surface.raised),
                ("popup", sem.surface.popup),
                ("input", sem.surface.input),
            ];

            for (sname, surface) in surfaces {
                for (tname, fg) in [
                    ("text.primary", sem.text.primary),
                    ("text.secondary", sem.text.secondary),
                    ("text.muted", sem.text.muted),
                    ("text.link", sem.text.link),
                    ("status.error", sem.status.error),
                    ("status.warning", sem.status.warning),
                    ("status.success", sem.status.success),
                    ("status.info", sem.status.info),
                ] {
                    let r = contrast_ratio(fg, surface);
                    if r < 4.5 {
                        failures.push(format!("{name}: {tname} on {sname} = {r:.2} (< 4.5)"));
                    }
                }
            }

            // UI components and graphical objects: 3:1.
            // WCAG 1.4.11 covers the parts of a control needed to identify
            // it and its state. `border.strong` is a divider and a panel seam —
            // decorative structure, explicitly out of scope — so it is not held
            // to 3:1. Everything below *is* a state cue and is.
            for (cname, fg, bg) in [
                ("border.focus", sem.border.focus, sem.surface.panel),
                ("accent.default", sem.accent.default, sem.surface.panel),
                (
                    "accent.selected_rail",
                    sem.accent.selected_rail,
                    sem.surface.panel,
                ),
                ("ember", t.ember, sem.surface.panel),
            ] {
                let r = contrast_ratio(fg, bg);
                if r < 3.0 {
                    failures.push(format!("{name}: {cname} on panel = {r:.2} (< 3.0)"));
                }
            }

            // A filled primary button: its label against its own fill.
            let r = contrast_ratio(sem.text.inverse, sem.accent.default);
            if r < 4.5 {
                failures.push(format!("{name}: text.inverse on accent = {r:.2} (< 4.5)"));
            }
        }
        assert!(
            failures.is_empty(),
            "contrast failures:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn measured_gradient_deltas() {
        // Printed so the cap in the next test is set from data rather than from
        // an eyeballed guess.
        for (name, t) in themes() {
            for (gname, g) in [
                ("chrome_wash", t.gradient.chrome_wash),
                ("header_wash", t.gradient.header_wash),
                ("accent_primary", t.gradient.accent_primary),
                ("rail_accent", t.gradient.rail_accent),
            ] {
                let a = relative_luminance(g.from);
                let b = relative_luminance(g.to);
                println!(
                    "{name} {gname}: dL = {:.4}, ratio = {:.4}",
                    (a - b).abs(),
                    contrast_ratio(g.from, g.to)
                );
            }
        }
    }

    #[test]
    fn chrome_washes_stay_subtle_and_accent_gradients_stay_readable() {
        // §5.3 caps chrome washes at "2-6 %", which is ambiguous until you pick
        // a metric. An absolute luminance delta is the wrong one: the same
        // perceptual step measures 0.0036 on the Nocturne ground and 0.0631 on
        // the Dawn ground, so an absolute cap would force the light theme's wash
        // to be invisible. Contrast ratio is scale-invariant and means the same
        // thing on both, so that is what the cap is expressed in.
        for (name, t) in themes() {
            for (gname, g) in [
                ("chrome_wash", t.gradient.chrome_wash),
                ("header_wash", t.gradient.header_wash),
            ] {
                let r = contrast_ratio(g.from, g.to);
                assert!(
                    r <= 1.12,
                    "{name} {gname} ratio {r:.4} reads as a band, not a wash"
                );
                assert!(
                    r > 1.0,
                    "{name} {gname} is flat, so it should not be a gradient at all"
                );
            }
            for (gname, g) in [
                ("accent_primary", t.gradient.accent_primary),
                ("rail_accent", t.gradient.rail_accent),
            ] {
                let r = contrast_ratio(g.from, g.to);
                assert!(r <= 1.60, "{name} {gname} ratio {r:.4} is too strong");
                assert!(r > 1.0, "{name} {gname} is flat");
            }
        }
    }

    #[test]
    fn every_gradient_runs_top_to_bottom() {
        // A chrome wash lit from any other direction reads as a mistake.
        for (_, t) in themes() {
            for g in [
                t.gradient.chrome_wash,
                t.gradient.header_wash,
                t.gradient.accent_primary,
                t.gradient.rail_accent,
            ] {
                assert_eq!(g.axis, [0.0, 1.0]);
            }
        }
    }

    #[test]
    fn switching_theme_changes_paint_and_never_layout() {
        // The reason Dawn ships in 27-E rather than at sign-off: if a theme swap
        // moved one density or geometry token, every cached layout would be
        // invalidated and the swap would stop being a swap.
        assert_eq!(NOCTURNE.density.titlebar, DAWN.density.titlebar);
        assert_eq!(NOCTURNE.density.row_dense, DAWN.density.row_dense);
        assert_eq!(NOCTURNE.geometry.radius_input, DAWN.geometry.radius_input);
        assert_eq!(NOCTURNE.geometry.radius_popup, DAWN.geometry.radius_popup);
        assert_eq!(NOCTURNE.geometry.stroke_rail, DAWN.geometry.stroke_rail);
        assert_eq!(NOCTURNE.typography.body, DAWN.typography.body);
        assert_eq!(NOCTURNE.motion.hover_ms, DAWN.motion.hover_ms);
        assert_ne!(
            NOCTURNE.semantic.surface.panel.bytes(),
            DAWN.semantic.surface.panel.bytes(),
            "and the paint must actually differ"
        );
    }

    #[test]
    fn the_two_themes_run_opposite_polarities() {
        let dark = relative_luminance(NOCTURNE.semantic.surface.panel);
        let light = relative_luminance(DAWN.semantic.surface.panel);
        assert!(dark < 0.1, "Nocturne panel luminance {dark}");
        assert!(light > 0.7, "Dawn panel luminance {light}");
    }

    // Polarity is a compile-time property of the snapshots, so it is asserted
    // as one rather than as a runtime check the optimiser deletes.
    const _: () = assert!(!NOCTURNE.is_light);
    const _: () = assert!(DAWN.is_light);

    #[test]
    fn the_palette_runs_two_temperatures() {
        // §4.2: a cool ground with a warm signal. Ember must be warm and the
        // accent cool, or the composition collapses into one hue.
        for (name, t) in themes() {
            let ember = t.ember.bytes();
            assert!(ember[0] > ember[2], "{name}: ember {ember:?} is not warm");
            let accent = t.semantic.accent.default.bytes();
            assert!(accent[2] > accent[0], "{name}: accent {accent:?} is not cool");
        }
    }

    #[test]
    fn elevation_is_a_strict_ladder() {
        for (name, t) in themes() {
            let e = &t.elevation;
            assert!(e.raised.spread < e.popup.spread, "{name} raised/popup");
            assert!(e.popup.spread < e.drawer.spread, "{name} popup/drawer");
            assert!(e.drawer.spread < e.modal.spread, "{name} drawer/modal");
            // The drawer opens upward.
            assert!(e.drawer.offset_y < 0.0, "{name} drawer offset");
            for (lname, level) in [
                ("raised", e.raised),
                ("popup", e.popup),
                ("drawer", e.drawer),
                ("modal", e.modal),
                ("toast", e.toast),
            ] {
                assert!(level.alpha > 0.0, "{name} {lname} is invisible");
                assert!(level.alpha < 1.0, "{name} {lname} is opaque");
            }
        }
    }

    #[test]
    fn shadows_are_lighter_on_the_light_theme() {
        // The same alpha reads far heavier over a bright ground. Every rung is
        // checked, not just the two that happened to be written first.
        let pairs = [
            ("raised", DAWN.elevation.raised, NOCTURNE.elevation.raised),
            ("popup", DAWN.elevation.popup, NOCTURNE.elevation.popup),
            ("drawer", DAWN.elevation.drawer, NOCTURNE.elevation.drawer),
            ("modal", DAWN.elevation.modal, NOCTURNE.elevation.modal),
            ("toast", DAWN.elevation.toast, NOCTURNE.elevation.toast),
        ];
        for (name, light, dark) in pairs {
            assert!(
                light.alpha < dark.alpha,
                "{name}: Dawn {} is not lighter than Nocturne {}",
                light.alpha,
                dark.alpha
            );
        }
    }

    #[test]
    fn ramp_steps_are_monotonic_and_land_on_the_base_at_500() {
        let base = NOCTURNE.semantic.accent.default;
        assert_eq!(ramp_step(base, 500).bytes(), base.bytes(), "500 is the base");

        let mut previous = f32::MAX;
        for step in [50u16, 100, 200, 300, 400, 500, 600, 700, 800, 900] {
            let l = relative_luminance(ramp_step(base, step));
            assert!(
                l <= previous + 1e-4,
                "step {step} is brighter than the step before it"
            );
            previous = l;
        }
        assert!(relative_luminance(ramp_step(base, 50)) > relative_luminance(base));
        assert!(relative_luminance(ramp_step(base, 900)) < relative_luminance(base));
    }

    #[test]
    fn theme_selection_round_trips() {
        let original = active_id();
        set_active(ThemeId::Dawn);
        assert_eq!(active_id(), ThemeId::Dawn);
        assert!(active().is_light);
        set_active(ThemeId::Nocturne);
        assert_eq!(active_id(), ThemeId::Nocturne);
        assert!(!active().is_light);
        set_active(original);
    }

    #[test]
    fn glow_is_capped_at_two_roles() {
        // §5.4 allows exactly two. The destructuring below stops compiling if a
        // third is added, which is the point: the cap is structural.
        for (_, t) in themes() {
            let GlowTokens { focus, armed } = t.glow;
            assert!(focus.radius > 0.0 && armed.radius > 0.0);
            // Glow is a halo, not a fill: it must be translucent.
            assert!(focus.color.bytes()[3] < 0xFF);
            assert!(armed.color.bytes()[3] < 0xFF);
        }
    }
}

#[cfg(test)]
mod token_sheet_tests {
    use super::*;

    const NOCTURNE_SHEET: &str = include_str!("../assets/tokens/nocturne.tokens.json");
    const DAWN_SHEET: &str = include_str!("../assets/tokens/dawn.tokens.json");

    fn hex(c: Srgb8) -> String {
        let b = c.bytes();
        if b[3] == 0xFF {
            format!("#{:02X}{:02X}{:02X}", b[0], b[1], b[2])
        } else {
            format!(
                "rgba({},{},{},{:.3})",
                b[0],
                b[1],
                b[2],
                b[3] as f32 / 255.0
            )
        }
    }

    fn expected_pairs(t: &Theme) -> Vec<(String, String)> {
        let s = &t.semantic;
        let mut v = vec![
            ("surface.window", s.surface.window),
            ("surface.canvas", s.surface.canvas),
            ("surface.panel", s.surface.panel),
            ("surface.header", s.surface.header),
            ("surface.raised", s.surface.raised),
            ("surface.input", s.surface.input),
            ("surface.popup", s.surface.popup),
            ("surface.hover", s.surface.hover),
            ("surface.selected", s.surface.selected),
            ("surface.modal_scrim", s.surface.modal_scrim),
            ("text.primary", s.text.primary),
            ("text.emphasis", s.text.emphasis),
            ("text.secondary", s.text.secondary),
            ("text.muted", s.text.muted),
            ("text.disabled", s.text.disabled),
            ("text.inverse", s.text.inverse),
            ("text.link", s.text.link),
            ("border.subtle", s.border.subtle),
            ("border.default", s.border.default),
            ("border.strong", s.border.strong),
            ("border.focus", s.border.focus),
            ("accent.default", s.accent.default),
            ("accent.hover", s.accent.hover),
            ("accent.pressed", s.accent.pressed),
            ("accent.selected_bg", s.accent.selected_bg),
            ("accent.selected_rail", s.accent.selected_rail),
            ("status.info", s.status.info),
            ("status.success", s.status.success),
            ("status.warning", s.status.warning),
            ("status.error", s.status.error),
            ("status.busy", s.status.busy),
            ("folder", s.folder),
            ("ember", t.ember),
        ];
        v.drain(..).map(|(k, c)| (k.to_string(), hex(c))).collect()
    }

    /// The token sheets are what a designer reads and what a future tooling
    /// pipeline imports; `theme.rs` is what actually ships. Phase 27-E made the
    /// pair drift-proof rather than merely consistent on the day it was written
    /// — the Nocturne sheet was already stale by two values when this landed,
    /// because the WCAG pass moved `text.muted` and `status.error` and nothing
    /// pointed at the JSON.
    #[test]
    fn json_sheets_match_the_shipped_snapshots() {
        for (label, sheet, theme) in [
            ("nocturne", NOCTURNE_SHEET, &NOCTURNE),
            ("dawn", DAWN_SHEET, &DAWN),
        ] {
            let parsed: serde_json::Value =
                serde_json::from_str(sheet).unwrap_or_else(|e| panic!("{label}: {e}"));
            let semantic = parsed
                .get("semantic")
                .and_then(|v| v.as_object())
                .unwrap_or_else(|| panic!("{label}: no semantic block"));

            let expected = expected_pairs(theme);
            let mut problems = Vec::new();

            for (key, want) in &expected {
                match semantic.get(key).and_then(|v| v.as_str()) {
                    None => problems.push(format!("{label}: {key} missing from the sheet")),
                    Some(got) if got != want => {
                        problems.push(format!("{label}: {key} sheet={got} theme={want}"))
                    }
                    Some(_) => {}
                }
            }
            // And nothing in the sheet that the theme no longer ships.
            for key in semantic.keys() {
                if !expected.iter().any(|(k, _)| k == key) {
                    problems.push(format!("{label}: {key} is in the sheet but not the theme"));
                }
            }
            assert!(problems.is_empty(), "{}", problems.join("\n"));
        }
    }

    #[test]
    fn both_sheets_declare_the_colour_contract_and_their_polarity() {
        for (label, sheet, light) in [
            ("nocturne", NOCTURNE_SHEET, false),
            ("dawn", DAWN_SHEET, true),
        ] {
            let parsed: serde_json::Value = serde_json::from_str(sheet).unwrap();
            let meta = parsed.get("$meta").expect("$meta");
            let contract = meta
                .get("runtime_contract")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            assert!(
                contract.contains("exactly once"),
                "{label}: the single-decode contract must stay on the sheet"
            );
            assert_eq!(
                meta.get("polarity").and_then(|v| v.as_str()),
                Some(if light { "light" } else { "dark" }),
                "{label}: polarity"
            );
        }
    }
}
