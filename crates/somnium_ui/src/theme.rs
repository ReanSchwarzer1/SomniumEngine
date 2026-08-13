// Nocturne — Somnium editor visual identity (Phase 26-A Metaphor).
// Cool night interior, lunar indigo accent, Inter. Not Unreal Starship.
// Reference: dev records/phase_26.md §2.4. Old Phase 12 names remain as aliases.

pub type Color = [u8; 4]; // RGBA, matches Widget background/foreground fields

pub const TRANSPARENT: Color = [0x00, 0x00, 0x00, 0x00];
pub const BLACK: Color = [0x00, 0x00, 0x00, 0xFF];
pub const WHITE: Color = [0xFF, 0xFF, 0xFF, 0xFF];

// Surfaces (cool blue-black)
pub const BG_VOID: Color = [0x14, 0x16, 0x1C, 0xFF];
pub const BG_CONTENT: Color = [0x18, 0x1A, 0x20, 0xFF];
pub const BG_PANEL: Color = [0x1C, 0x1E, 0x26, 0xFF];
pub const BG_HEADER: Color = [0x25, 0x28, 0x30, 0xFF];
pub const BG_RAISED: Color = [0x2A, 0x2D, 0x38, 0xFF];
pub const BG_INPUT: Color = [0x12, 0x14, 0x1A, 0xFF];
pub const BG_HOVER: Color = [0x34, 0x38, 0x48, 0xFF];

/// Phase 12 alias — maps to the deepest well.
pub const BG_DARK: Color = BG_VOID;

// Borders
pub const BORDER_DARK: Color = [0x0E, 0x10, 0x14, 0xFF];
pub const BORDER_MEDIUM: Color = [0x3A, 0x3E, 0x4A, 0xFF];
pub const BORDER_LIGHT: Color = [0x50, 0x54, 0x62, 0xFF];
pub const BORDER_FOCUS: Color = [0x7A, 0x86, 0xFF, 0xFF];

// Lunar indigo accent
pub const ACCENT: Color = [0x7A, 0x86, 0xFF, 0xFF];
pub const ACCENT_HOVER: Color = [0x94, 0x9C, 0xFF, 0xFF];
pub const ACCENT_PRESSED: Color = [0x5C, 0x68, 0xE0, 0xFF];
/// Selection fill behind a row (~22% of ACCENT over BG_PANEL).
pub const ACCENT_DIM: Color = [0x32, 0x36, 0x52, 0xFF];
/// Phase 12 alias.
pub const ACCENT_BLUE: Color = ACCENT;

/// Folder tint in the Content Drawer (warm sand on cool panels).
pub const FOLDER_SAND: Color = [0xC4, 0xA5, 0x74, 0xFF];

// Type
pub const TEXT_PRIMARY: Color = [0xD8, 0xDC, 0xE8, 0xFF];
pub const TEXT_SECONDARY: Color = [0x9A, 0xA3, 0xB5, 0xFF];
pub const TEXT_DISABLED: Color = [0x5C, 0x64, 0x78, 0xFF];
pub const TEXT_LINK: Color = ACCENT;

// Status
pub const STATUS_OK: Color = [0x5D, 0xCE, 0x9A, 0xFF];
pub const STATUS_WARN: Color = [0xE6, 0xB0, 0x4A, 0xFF];
pub const STATUS_ERROR: Color = [0xE0, 0x5A, 0x5A, 0xFF];

/// Chrome corner radius in pixels (tighter than Starship's 4).
pub const RADIUS_CHROME: f32 = 2.0;
pub const RADIUS_POPUP: f32 = 3.0;

/// Inspector / outliner row height.
pub const ROW_HEIGHT: f32 = 24.0;
pub const TOOLBAR_HEIGHT: f32 = 32.0;
pub const MENU_HEIGHT: f32 = 28.0;
pub const STATUS_HEIGHT: f32 = 24.0;
pub const SPLITTER_THICKNESS: f32 = 6.0;
pub const TOOLTIP_DELAY_MS: u64 = 400;

/// Toolbar / status-bar icon size (atlas cells are 32 px).
pub const ICON_TOOL: f32 = 24.0;
/// Outliner / tree / menu chevron size.
pub const ICON_TREE: f32 = 20.0;
/// Check-box tick box.
pub const ICON_CHECK: f32 = 16.0;
