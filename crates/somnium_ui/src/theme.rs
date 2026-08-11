// UE5 dark editor theme — color constants for Phase 12 native UI.
// Reference: UE5 editor palette (see ATTRIBUTION.md §13.9).

pub type Color = [u8; 4]; // RGBA, matches Widget background/foreground fields

pub const TRANSPARENT: Color = [0x00, 0x00, 0x00, 0x00];
pub const BLACK: Color = [0x00, 0x00, 0x00, 0xFF];
pub const WHITE: Color = [0xFF, 0xFF, 0xFF, 0xFF];

// Backgrounds
pub const BG_DARK: Color = [0x1E, 0x1E, 0x1E, 0xFF]; // main panels
pub const BG_HEADER: Color = [0x28, 0x28, 0x28, 0xFF]; // toolbar / header bars
pub const BG_CONTENT: Color = [0x21, 0x21, 0x21, 0xFF]; // content areas
pub const BG_RAISED: Color = [0x2E, 0x2E, 0x2E, 0xFF]; // buttons, cards, inputs
pub const BG_HOVER: Color = [0x3A, 0x3A, 0x3A, 0xFF]; // hover state

// Borders
pub const BORDER_DARK: Color = [0x1A, 0x1A, 0x1A, 0xFF]; // deep/dark borders
pub const BORDER_MEDIUM: Color = [0x30, 0x30, 0x30, 0xFF]; // standard separator
pub const BORDER_LIGHT: Color = [0x50, 0x50, 0x50, 0xFF]; // subtle borders

// Accent / selection
pub const ACCENT_BLUE: Color = [0x1A, 0x75, 0xD2, 0xFF]; // selection / focus ring
pub const ACCENT_HOVER: Color = [0x22, 0x88, 0xFF, 0xFF]; // hovered accent
pub const ACCENT_PRESSED: Color = [0x0E, 0x5A, 0xA8, 0xFF]; // pressed accent

// Text
pub const TEXT_PRIMARY: Color = [0xFF, 0xFF, 0xFF, 0xFF]; // main labels
pub const TEXT_SECONDARY: Color = [0xAA, 0xAA, 0xAA, 0xFF]; // secondary / muted
pub const TEXT_DISABLED: Color = [0x66, 0x66, 0x66, 0xFF]; // disabled state

// Status
pub const STATUS_OK: Color = [0x3C, 0xB3, 0x71, 0xFF]; // success / green
pub const STATUS_WARN: Color = [0xE6, 0x9B, 0x2D, 0xFF]; // warning / yellow
pub const STATUS_ERROR: Color = [0xC0, 0x39, 0x2B, 0xFF]; // error / red
