//! Authoritative editor command declarations (CONTROL-A2).
//!
//! A command is declared once here. Menus, toolbars, context menus, keyboard
//! dispatch, the command palette, and the Help index consume this registry.

use crate::{CreateKind, Workspace};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    path::PathBuf,
    sync::OnceLock,
};
use winit::keyboard::KeyCode;

/// A platform-neutral key used by an editor shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CommandKey {
    /// A letter key.
    Letter(char),
    /// Space.
    Space,
    /// Delete.
    Delete,
    /// A function key.
    Function(u8),
}

impl fmt::Display for CommandKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Letter(letter) => write!(f, "{}", letter.to_ascii_uppercase()),
            Self::Space => f.write_str("Space"),
            Self::Delete => f.write_str("Del"),
            Self::Function(number) => write!(f, "F{number}"),
        }
    }
}

/// One key plus its required modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Chord {
    /// Primary key.
    pub key: CommandKey,
    /// Control modifier.
    pub ctrl: bool,
    /// Shift modifier.
    pub shift: bool,
    /// Alt/Option modifier.
    pub alt: bool,
    /// Command/Super modifier.
    pub command: bool,
}

impl Chord {
    /// Whether this chord carries any modifier.
    ///
    /// The line between a shortcut that may fire while the viewport is flying
    /// and one that may not. `Ctrl+S` is unambiguous and should still save;
    /// bare `S` is also "move backward", and a dispatcher that fires it eats
    /// the camera's key.
    #[must_use]
    pub const fn has_modifier(self) -> bool {
        self.ctrl || self.shift || self.alt || self.command
    }

    /// Start a chord with no modifiers.
    pub const fn press(key: CommandKey) -> Self {
        Self {
            key,
            ctrl: false,
            shift: false,
            alt: false,
            command: false,
        }
    }

    /// Require Control.
    #[must_use]
    pub const fn ctrl(mut self) -> Self {
        self.ctrl = true;
        self
    }

    /// Require Shift.
    #[must_use]
    pub const fn shift(mut self) -> Self {
        self.shift = true;
        self
    }

    /// Require Alt/Option.
    #[must_use]
    pub const fn alt(mut self) -> Self {
        self.alt = true;
        self
    }

    /// Require Command/Super.
    #[must_use]
    pub const fn command(mut self) -> Self {
        self.command = true;
        self
    }

    /// Convert a winit key and modifier snapshot into a chord.
    pub fn from_winit(
        code: KeyCode,
        ctrl: bool,
        shift: bool,
        alt: bool,
        command: bool,
    ) -> Option<Self> {
        let key = match code {
            KeyCode::KeyA => CommandKey::Letter('A'),
            KeyCode::KeyB => CommandKey::Letter('B'),
            KeyCode::KeyC => CommandKey::Letter('C'),
            KeyCode::KeyD => CommandKey::Letter('D'),
            KeyCode::KeyE => CommandKey::Letter('E'),
            KeyCode::KeyF => CommandKey::Letter('F'),
            KeyCode::KeyG => CommandKey::Letter('G'),
            KeyCode::KeyH => CommandKey::Letter('H'),
            KeyCode::KeyI => CommandKey::Letter('I'),
            KeyCode::KeyJ => CommandKey::Letter('J'),
            KeyCode::KeyK => CommandKey::Letter('K'),
            KeyCode::KeyL => CommandKey::Letter('L'),
            KeyCode::KeyM => CommandKey::Letter('M'),
            KeyCode::KeyN => CommandKey::Letter('N'),
            KeyCode::KeyO => CommandKey::Letter('O'),
            KeyCode::KeyP => CommandKey::Letter('P'),
            KeyCode::KeyQ => CommandKey::Letter('Q'),
            KeyCode::KeyR => CommandKey::Letter('R'),
            KeyCode::KeyS => CommandKey::Letter('S'),
            KeyCode::KeyT => CommandKey::Letter('T'),
            KeyCode::KeyU => CommandKey::Letter('U'),
            KeyCode::KeyV => CommandKey::Letter('V'),
            KeyCode::KeyW => CommandKey::Letter('W'),
            KeyCode::KeyX => CommandKey::Letter('X'),
            KeyCode::KeyY => CommandKey::Letter('Y'),
            KeyCode::KeyZ => CommandKey::Letter('Z'),
            KeyCode::Space => CommandKey::Space,
            KeyCode::Delete => CommandKey::Delete,
            KeyCode::F1 => CommandKey::Function(1),
            KeyCode::F2 => CommandKey::Function(2),
            KeyCode::F3 => CommandKey::Function(3),
            KeyCode::F4 => CommandKey::Function(4),
            KeyCode::F5 => CommandKey::Function(5),
            KeyCode::F6 => CommandKey::Function(6),
            KeyCode::F7 => CommandKey::Function(7),
            KeyCode::F8 => CommandKey::Function(8),
            KeyCode::F9 => CommandKey::Function(9),
            KeyCode::F10 => CommandKey::Function(10),
            KeyCode::F11 => CommandKey::Function(11),
            KeyCode::F12 => CommandKey::Function(12),
            _ => return None,
        };
        Some(Self {
            key,
            ctrl,
            shift,
            alt,
            command,
        })
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        if self.shift {
            f.write_str("Shift+")?;
        }
        if self.command {
            f.write_str("Cmd+")?;
        }
        self.key.fmt(f)
    }
}

/// Why a command may or may not run in the current editor state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enablement {
    /// The command may run.
    Enabled,
    /// The command is unavailable, with a user-facing explanation.
    Disabled(&'static str),
}

impl Enablement {
    /// Whether the command may run.
    pub const fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }

    /// Explanation suitable for a tooltip or palette row.
    pub const fn reason(self) -> Option<&'static str> {
        match self {
            Self::Enabled => None,
            Self::Disabled(reason) => Some(reason),
        }
    }
}

/// State consulted by command enablement predicates.
#[derive(Clone, Copy, Debug, Default)]
pub struct EditorCtx {
    /// An entity is selected.
    pub has_selection: bool,
    /// Undo history is available.
    pub can_undo: bool,
    /// Redo history is available.
    pub can_redo: bool,
    /// A Content Drawer item is the context-menu subject.
    pub has_content_target: bool,
    /// The entity clipboard holds something to paste.
    pub has_clipboard: bool,
}

/// Application menu that owns a command row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Menu {
    File,
    Edit,
    Create,
    View,
    Window,
    Help,
}

/// UI surfaces generated from the registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommandSurface {
    Menu(Menu),
    Toolbar,
    ContentContext,
    /// The Outliner's right-click menu. CONTROL-F: built from the registry
    /// like every other surface, so a command added once appears here too.
    OutlinerContext,
}

/// Stable execution meaning of a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandAction {
    NewScene,
    SaveScene,
    ImportModel,
    Undo,
    Redo,
    DeleteSelected,
    DuplicateSelected,
    CopySelected,
    PasteClipboard,
    SelectAll,
    FocusSelection,
    RenameSelected,
    Play,
    Pause,
    Stop,
    ToggleProfiler,
    ToggleDrawer,
    TogglePalette,
    OpenHelp(u8),
    ReloadScripts,
    ToggleShadingMode,
    SetGizmoMode(u8),
    ToggleTerrainEdit,
    ToggleFoliage,
    ToggleImmersiveViewport,
    OpenOutputLog,
    CreateEntity(CreateKind),
    DockContentDrawer,
    SetWorkspace(Workspace),
    ResetWorkspace,
    ContentNewFolder,
    ContentNewScript,
    ContentNewMaterial,
    ContentRename,
    ContentShowInFolder,
    ContentRefresh,
    OpenPreferences,
    OpenProjectPicker,
    /// Select a named debug visualisation. Mutually exclusive with the others.
    SetDebugView(&'static str),
    /// Flip one named renderer pipeline switch.
    ToggleRenderSwitch(&'static str),
    /// Point the editor camera down a world axis.
    ViewPreset(u8),
    /// Store the current camera pose in a numbered slot.
    SetBookmark(u8),
    /// Recall a numbered camera pose.
    RecallBookmark(u8),
    /// Orbit the camera around the selection rather than around itself.
    ToggleOrbitSelection,
    /// CONTROL-L: jump the day cycle's clock to a named hour. The payload is
    /// the hour itself rather than an index into a table, so a rearranged
    /// preset list cannot silently change what a persisted keybinding does.
    SetTimeOfDay(&'static str),
    /// CONTROL-M: apply a named sky preset. Id, not an index, for the same
    /// reason as above.
    SetSkyPreset(&'static str),
    /// CONTROL-N: apply a named weather state.
    SetWeatherPreset(&'static str),
}

/// One editor command declaration.
#[derive(Clone, Copy)]
pub struct Command {
    /// Stable path-like identifier used for dispatch and persistence.
    pub id: &'static str,
    /// User-facing label.
    pub label: &'static str,
    /// Search/menu category.
    pub category: &'static str,
    /// Shipped shortcut.
    pub default_binding: Option<Chord>,
    /// One-line Help and tooltip copy.
    pub help: &'static str,
    /// Stable execution meaning.
    pub action: CommandAction,
    /// Surfaces on which the command is explicitly shown. Every command is
    /// automatically present in the palette and Help index.
    pub surfaces: &'static [CommandSurface],
    enabled: fn(&EditorCtx) -> Enablement,
}

impl Command {
    /// Evaluate this command for an editor state.
    pub fn enabled(self, ctx: &EditorCtx) -> Enablement {
        (self.enabled)(ctx)
    }

    /// Label plus the rendered authoritative shortcut.
    pub fn menu_label(self) -> String {
        self.default_binding.map_or_else(
            || self.label.to_string(),
            |chord| format!("{}    {chord}", self.label),
        )
    }
}

fn always(_: &EditorCtx) -> Enablement {
    Enablement::Enabled
}
fn selection(ctx: &EditorCtx) -> Enablement {
    if ctx.has_selection {
        Enablement::Enabled
    } else {
        Enablement::Disabled("Select an entity first")
    }
}
fn undo(ctx: &EditorCtx) -> Enablement {
    if ctx.can_undo {
        Enablement::Enabled
    } else {
        Enablement::Disabled("Nothing to undo")
    }
}
fn redo(ctx: &EditorCtx) -> Enablement {
    if ctx.can_redo {
        Enablement::Enabled
    } else {
        Enablement::Disabled("Nothing to redo")
    }
}
fn clipboard(ctx: &EditorCtx) -> Enablement {
    if ctx.has_clipboard {
        Enablement::Enabled
    } else {
        Enablement::Disabled("Copy something first")
    }
}

fn content_target(ctx: &EditorCtx) -> Enablement {
    if ctx.has_content_target {
        Enablement::Enabled
    } else {
        Enablement::Disabled("Choose a content item first")
    }
}

const FILE: &[CommandSurface] = &[CommandSurface::Menu(Menu::File)];
const EDIT: &[CommandSurface] = &[CommandSurface::Menu(Menu::Edit)];
const CREATE: &[CommandSurface] = &[CommandSurface::Menu(Menu::Create)];
const VIEW: &[CommandSurface] = &[CommandSurface::Menu(Menu::View)];
const WINDOW: &[CommandSurface] = &[CommandSurface::Menu(Menu::Window)];
const HELP: &[CommandSurface] = &[CommandSurface::Menu(Menu::Help)];
const TOOLBAR: &[CommandSurface] = &[CommandSurface::Toolbar];
const VIEW_TOOLBAR: &[CommandSurface] =
    &[CommandSurface::Menu(Menu::View), CommandSurface::Toolbar];
const CONTENT: &[CommandSurface] = &[CommandSurface::ContentContext];
const VIEW_MENU: &[CommandSurface] = &[CommandSurface::Menu(Menu::View)];
const EDIT_OUTLINER: &[CommandSurface] = &[
    CommandSurface::Menu(Menu::Edit),
    CommandSurface::OutlinerContext,
];
const VIEW_OUTLINER: &[CommandSurface] = &[
    CommandSurface::Menu(Menu::View),
    CommandSurface::OutlinerContext,
];
const CREATE_CONTENT: &[CommandSurface] = &[
    CommandSurface::Menu(Menu::Create),
    CommandSurface::ContentContext,
];
const PALETTE_ONLY: &[CommandSurface] = &[];

macro_rules! command {
    ($id:literal, $label:literal, $category:literal, $binding:expr, $help:literal, $action:expr, $surfaces:expr, $enabled:expr) => {{
        const _: () = assert!(!$help.is_empty(), "command help must not be empty");
        Command {
            id: $id,
            label: $label,
            category: $category,
            default_binding: $binding,
            help: $help,
            action: $action,
            surfaces: $surfaces,
            enabled: $enabled,
        }
    }};
}

const fn ctrl(letter: char) -> Option<Chord> {
    Some(Chord::press(CommandKey::Letter(letter)).ctrl())
}

/// CONTROL-G's view-mode menu, generated from the renderer's own tables.
///
/// Written as a loop rather than fifty `command!` blocks on purpose: the whole
/// claim of this sub-phase is that a debug visualisation costs a *row*, not a
/// feature, and a hand-copied list would be the first thing to drift from
/// `somnium_renderer::debug`. Every id, label and Help line here is the
/// renderer's, so the menu cannot describe a view the shader does not have.
fn view_mode_commands() -> Vec<Command> {
    let mut commands = Vec::new();
    for view in crate::debug::DEBUG_VIEWS {
        commands.push(Command {
            id: leak_id("editor.view.debug.", view.id),
            label: view.label,
            category: "View Mode",
            default_binding: None,
            help: view.help,
            action: CommandAction::SetDebugView(view.id),
            surfaces: VIEW_MENU,
            enabled: always,
        });
    }
    for toggle in crate::debug::TOGGLES {
        commands.push(Command {
            id: leak_id("editor.view.pipeline.", toggle.id),
            label: toggle.label,
            category: "Pipeline",
            default_binding: None,
            help: toggle.help,
            action: CommandAction::ToggleRenderSwitch(toggle.id),
            surfaces: VIEW_MENU,
            enabled: always,
        });
    }
    commands
}

/// CONTROL-L's named times: `(id, label, hour)`.
///
/// One table, in the registry, because the label a user reads and the hour the
/// sun moves to must be one declaration. `somnium_core::time_of_day`
/// re-exports it and looks hours up by id, so a preset added here is a preset
/// the driver understands with no second edit.
pub const TIME_PRESETS: [(&str, &str, f32); 6] = [
    ("dawn", "Dawn", 5.5),
    ("sunrise", "Sunrise", 7.0),
    ("noon", "Noon", 12.0),
    ("golden_hour", "Golden Hour", 19.0),
    ("dusk", "Dusk", 20.5),
    ("night", "Night", 1.0),
];

/// CONTROL-M's named skies: `(id, label)`.
///
/// Ids only. The *values* a preset writes are engine data and live in
/// `somnium_core::sky`, which resolves an id from this list; a test there
/// asserts that every id here resolves, so a menu row cannot name a preset the
/// engine does not have.
pub const SKY_PRESETS: [(&str, &str); 4] = [
    ("clear", "Clear"),
    ("scattered", "Scattered"),
    ("overcast", "Overcast"),
    ("storm", "Storm"),
];

/// CONTROL-N's named weather states: `(id, label)`.
///
/// Ids only, for the same reason as [`SKY_PRESETS`]: the values are engine
/// data and live in `somnium_core::weather`.
pub const WEATHER_PRESETS: [(&str, &str); 4] = [
    ("clear", "Clear"),
    ("drizzle", "Drizzle"),
    ("storm", "Storm"),
    ("snow", "Snow"),
];

/// The command family built from [`TIME_PRESETS`].
fn time_of_day_commands() -> Vec<Command> {
    TIME_PRESETS
        .iter()
        .map(|(id, label, hour)| Command {
            id: leak_id("editor.time.", id),
            label: Box::leak(format!("Time: {label}").into_boxed_str()),
            category: "Environment",
            default_binding: None,
            help: Box::leak(
                format!("Set the scene's day cycle to {label} ({hour:.2} h).").into_boxed_str(),
            ),
            action: CommandAction::SetTimeOfDay(id),
            surfaces: VIEW_MENU,
            enabled: always,
        })
        .collect()
}

/// CONTROL-M's sky presets, from [`SKY_PRESETS`].
fn sky_commands() -> Vec<Command> {
    SKY_PRESETS
        .iter()
        .map(|(id, label)| Command {
            id: leak_id("editor.sky.", id),
            label: Box::leak(format!("Sky: {label}").into_boxed_str()),
            category: "Environment",
            default_binding: None,
            help: Box::leak(format!("Set the scene's cloud layer to {label}.").into_boxed_str()),
            action: CommandAction::SetSkyPreset(id),
            surfaces: VIEW_MENU,
            enabled: always,
        })
        .collect()
}

/// CONTROL-N's weather presets, from [`WEATHER_PRESETS`].
fn weather_commands() -> Vec<Command> {
    WEATHER_PRESETS
        .iter()
        .map(|(id, label)| Command {
            id: leak_id("editor.weather.", id),
            label: Box::leak(format!("Weather: {label}").into_boxed_str()),
            category: "Environment",
            default_binding: None,
            help: Box::leak(format!("Transition the scene's weather to {label}.").into_boxed_str()),
            action: CommandAction::SetWeatherPreset(id),
            surfaces: VIEW_MENU,
            enabled: always,
        })
        .collect()
}

/// Ids are `&'static str` throughout the registry, and the registry is built
/// once per process. Leaking the handful of generated ids is cheaper and
/// clearer than threading a lifetime through every command surface for
/// strings that live until the editor exits anyway.
fn leak_id(prefix: &str, suffix: &str) -> &'static str {
    Box::leak(format!("{prefix}{suffix}").into_boxed_str())
}

/// Camera bookmarks and view presets, also generated: nine slots and four
/// presets are a table, and writing them out would be thirteen near-identical
/// blocks that only a diff could tell apart.
fn camera_commands() -> Vec<Command> {
    const PRESETS: [(&str, &str, u8); 4] = [
        ("top", "Top", 0),
        ("front", "Front", 1),
        ("side", "Side", 2),
        ("perspective", "Perspective", 3),
    ];
    let mut commands = Vec::new();
    for (id, label, index) in PRESETS {
        commands.push(Command {
            id: leak_id("editor.view.preset.", id),
            label: Box::leak(format!("{label} View").into_boxed_str()),
            category: "Camera",
            default_binding: None,
            help: Box::leak(
                format!(
                    "Point the editor camera along the {} axis.",
                    label.to_lowercase()
                )
                .into_boxed_str(),
            ),
            action: CommandAction::ViewPreset(index),
            surfaces: VIEW_MENU,
            enabled: always,
        });
    }
    for slot in 1..=9u8 {
        commands.push(Command {
            id: leak_id("editor.view.bookmark.set.", &slot.to_string()),
            label: Box::leak(format!("Set Bookmark {slot}").into_boxed_str()),
            category: "Camera",
            default_binding: Some(Chord::press(CommandKey::Letter((b'0' + slot) as char)).ctrl()),
            help: Box::leak(
                format!("Store the current camera pose in slot {slot}.").into_boxed_str(),
            ),
            action: CommandAction::SetBookmark(slot),
            surfaces: &[],
            enabled: always,
        });
        commands.push(Command {
            id: leak_id("editor.view.bookmark.recall.", &slot.to_string()),
            label: Box::leak(format!("Recall Bookmark {slot}").into_boxed_str()),
            category: "Camera",
            default_binding: Some(Chord::press(CommandKey::Letter((b'0' + slot) as char))),
            help: Box::leak(format!("Return the camera to slot {slot}.").into_boxed_str()),
            action: CommandAction::RecallBookmark(slot),
            surfaces: &[],
            enabled: always,
        });
    }
    commands
}

fn declarations() -> Vec<Command> {
    use CommandAction as A;
    use CreateKind as C;
    use Workspace as W;
    vec![
        command!(
            "editor.scene.new",
            "New Scene",
            "Scene",
            ctrl('N'),
            "Create a new empty scene.",
            A::NewScene,
            FILE,
            always
        ),
        command!(
            "editor.scene.save",
            "Save Scene",
            "Scene",
            ctrl('S'),
            "Save the current scene.",
            A::SaveScene,
            FILE,
            always
        ),
        command!(
            "editor.asset.import_model",
            "Import Model…",
            "Asset",
            None,
            "Import a model into project content.",
            A::ImportModel,
            FILE,
            always
        ),
        command!(
            "editor.edit.undo",
            "Undo",
            "Edit",
            ctrl('Z'),
            "Undo the most recent editor change.",
            A::Undo,
            EDIT,
            undo
        ),
        command!(
            "editor.edit.redo",
            "Redo",
            "Edit",
            ctrl('Y'),
            "Redo the most recently undone change.",
            A::Redo,
            EDIT,
            redo
        ),
        command!(
            "editor.edit.delete",
            "Delete",
            "Edit",
            Some(Chord::press(CommandKey::Delete)),
            "Delete the selected entity.",
            A::DeleteSelected,
            EDIT_OUTLINER,
            selection
        ),
        command!(
            "editor.edit.duplicate",
            "Duplicate",
            "Edit",
            ctrl('D'),
            "Duplicate the selected entity.",
            A::DuplicateSelected,
            EDIT_OUTLINER,
            selection
        ),
        command!(
            "editor.edit.copy",
            "Copy",
            "Edit",
            ctrl('C'),
            "Copy the selected entities and everything beneath them.",
            A::CopySelected,
            EDIT_OUTLINER,
            selection
        ),
        command!(
            "editor.edit.paste",
            "Paste",
            "Edit",
            ctrl('V'),
            "Paste the clipboard under the selection.",
            A::PasteClipboard,
            EDIT_OUTLINER,
            clipboard
        ),
        command!(
            "editor.edit.select_all",
            "Select All",
            "Edit",
            ctrl('A'),
            "Select every entity in the scene.",
            A::SelectAll,
            EDIT,
            always
        ),
        command!(
            "editor.edit.rename",
            "Rename",
            "Edit",
            Some(Chord::press(CommandKey::Function(2))),
            "Rename the selected entity in place.",
            A::RenameSelected,
            EDIT_OUTLINER,
            selection
        ),
        command!(
            "editor.view.focus_selection",
            "Focus Selection",
            "View",
            Some(Chord::press(CommandKey::Letter('F'))),
            "Move the editor camera to frame the selection.",
            A::FocusSelection,
            VIEW_OUTLINER,
            selection
        ),
        command!(
            "editor.window.preferences",
            "Preferences",
            "Window",
            Some(Chord::press(CommandKey::Letter(',')).ctrl()),
            "Open editor preferences, project settings and the keyboard map.",
            A::OpenPreferences,
            WINDOW,
            always
        ),
        command!(
            "editor.file.open_project",
            "Open Project...",
            "File",
            None,
            "Choose a different project folder to work in.",
            A::OpenProjectPicker,
            FILE,
            always
        ),
        command!(
            "editor.simulation.play",
            "Play",
            "Simulation",
            None,
            "Start the play simulation.",
            A::Play,
            TOOLBAR,
            always
        ),
        command!(
            "editor.simulation.pause",
            "Pause",
            "Simulation",
            None,
            "Pause or resume the play simulation.",
            A::Pause,
            TOOLBAR,
            always
        ),
        command!(
            "editor.simulation.stop",
            "Stop",
            "Simulation",
            None,
            "Stop the play simulation and restore the edit scene.",
            A::Stop,
            TOOLBAR,
            always
        ),
        command!(
            "editor.view.profiler",
            "Toggle Profiler",
            "View",
            None,
            "Show or hide the profiler overlay.",
            A::ToggleProfiler,
            VIEW_TOOLBAR,
            always
        ),
        command!(
            "editor.view.content_drawer",
            "Content Drawer",
            "View",
            Some(Chord::press(CommandKey::Space).ctrl()),
            "Show or hide the Content Drawer.",
            A::ToggleDrawer,
            VIEW,
            always
        ),
        command!(
            "editor.search.commands",
            "Command Palette",
            "Window",
            ctrl('P'),
            "Search and run every registered editor command.",
            A::TogglePalette,
            PALETTE_ONLY,
            always
        ),
        command!(
            "editor.help.index",
            "Help Overlay",
            "Help",
            Some(Chord::press(CommandKey::Function(1))),
            "Open the editor Help index.",
            A::OpenHelp(0),
            HELP,
            always
        ),
        command!(
            "editor.help.shortcuts",
            "Shortcuts",
            "Help",
            None,
            "Show editor keyboard shortcuts.",
            A::OpenHelp(2),
            HELP,
            always
        ),
        command!(
            "editor.help.about",
            "About",
            "Help",
            None,
            "Show engine version and attribution information.",
            A::OpenHelp(4),
            HELP,
            always
        ),
        command!(
            "editor.script.reload",
            "Reload Scripts",
            "Script",
            Some(Chord::press(CommandKey::Function(5))),
            "Reload project scripts without ending the play session.",
            A::ReloadScripts,
            PALETTE_ONLY,
            always
        ),
        command!(
            "editor.view.shading_mode",
            "Cycle Shading Mode",
            "View",
            None,
            "Cycle the viewport shading mode.",
            A::ToggleShadingMode,
            VIEW,
            always
        ),
        command!(
            "editor.gizmo.translate",
            "Translate Tool",
            "Tools",
            Some(Chord::press(CommandKey::Letter('T'))),
            "Use the translate gizmo.",
            A::SetGizmoMode(0),
            TOOLBAR,
            always
        ),
        command!(
            "editor.gizmo.rotate",
            "Rotate Tool",
            "Tools",
            Some(Chord::press(CommandKey::Letter('R'))),
            "Use the rotate gizmo.",
            A::SetGizmoMode(1),
            TOOLBAR,
            always
        ),
        command!(
            "editor.gizmo.scale",
            "Scale Tool",
            "Tools",
            Some(Chord::press(CommandKey::Letter('S'))),
            "Use the scale gizmo.",
            A::SetGizmoMode(2),
            TOOLBAR,
            always
        ),
        command!(
            "editor.terrain.edit",
            "Landscape Mode",
            "Tools",
            Some(Chord::press(CommandKey::Function(6))),
            "Enter or leave terrain editing mode.",
            A::ToggleTerrainEdit,
            TOOLBAR,
            always
        ),
        command!(
            "editor.foliage.edit",
            "Foliage Mode",
            "Tools",
            Some(Chord::press(CommandKey::Function(8))),
            "Enter or leave foliage painting mode.",
            A::ToggleFoliage,
            TOOLBAR,
            always
        ),
        command!(
            "editor.viewport.immersive",
            "Immersive Viewport",
            "View",
            None,
            "Toggle the viewport-only immersive layout.",
            A::ToggleImmersiveViewport,
            TOOLBAR,
            always
        ),
        command!(
            "editor.window.output_log",
            "Output Log",
            "Window",
            None,
            "Open or close the Output Log.",
            A::OpenOutputLog,
            WINDOW,
            always
        ),
        command!(
            "editor.window.dock_content",
            "Show Content Drawer",
            "Window",
            None,
            "Dock and show the Content Drawer.",
            A::DockContentDrawer,
            WINDOW,
            always
        ),
        command!(
            "editor.workspace.layout",
            "Workspace: Layout",
            "Window",
            None,
            "Switch to the Layout workspace.",
            A::SetWorkspace(W::Layout),
            WINDOW,
            always
        ),
        command!(
            "editor.workspace.terrain",
            "Workspace: Terrain",
            "Window",
            None,
            "Switch to the Terrain workspace.",
            A::SetWorkspace(W::Terrain),
            WINDOW,
            always
        ),
        command!(
            "editor.workspace.foliage",
            "Workspace: Foliage",
            "Window",
            None,
            "Switch to the Foliage workspace.",
            A::SetWorkspace(W::Foliage),
            WINDOW,
            always
        ),
        command!(
            "editor.workspace.lighting",
            "Workspace: Lighting",
            "Window",
            None,
            "Switch to the Lighting workspace.",
            A::SetWorkspace(W::Lighting),
            WINDOW,
            always
        ),
        command!(
            "editor.workspace.materials",
            "Workspace: Materials",
            "Window",
            None,
            "Switch to the Materials workspace.",
            A::SetWorkspace(W::Materials),
            WINDOW,
            always
        ),
        command!(
            "editor.workspace.animation",
            "Workspace: Animation",
            "Window",
            None,
            "Open the animation graph and state-machine authoring surface.",
            A::SetWorkspace(W::Animation),
            WINDOW,
            always
        ),
        command!(
            "editor.workspace.debug",
            "Workspace: Debug",
            "Window",
            None,
            "Switch to the Debug workspace.",
            A::SetWorkspace(W::Debug),
            WINDOW,
            always
        ),
        command!(
            "editor.workspace.play",
            "Workspace: Play",
            "Window",
            None,
            "Switch to the Play workspace.",
            A::SetWorkspace(W::Play),
            WINDOW,
            always
        ),
        command!(
            "editor.workspace.reset",
            "Reset Workspace",
            "Window",
            None,
            "Restore the current workspace's shipped layout.",
            A::ResetWorkspace,
            WINDOW,
            always
        ),
        command!(
            "editor.create.cube",
            "Create Cube",
            "Create",
            None,
            "Create a cube entity.",
            A::CreateEntity(C::Cube),
            CREATE,
            always
        ),
        command!(
            "editor.create.sphere",
            "Create Sphere",
            "Create",
            None,
            "Create a sphere entity.",
            A::CreateEntity(C::Sphere),
            CREATE,
            always
        ),
        command!(
            "editor.create.plane",
            "Create Plane",
            "Create",
            None,
            "Create a plane entity.",
            A::CreateEntity(C::Plane),
            CREATE,
            always
        ),
        command!(
            "editor.create.cylinder",
            "Create Cylinder",
            "Create",
            None,
            "Create a cylinder entity.",
            A::CreateEntity(C::Cylinder),
            CREATE,
            always
        ),
        command!(
            "editor.create.directional_light",
            "Create Directional Light",
            "Create",
            None,
            "Create a directional light entity.",
            A::CreateEntity(C::DirectionalLight),
            CREATE,
            always
        ),
        command!(
            "editor.create.point_light",
            "Create Point Light",
            "Create",
            None,
            "Create a point light entity.",
            A::CreateEntity(C::PointLight),
            CREATE,
            always
        ),
        command!(
            "editor.create.spot_light",
            "Create Spot Light",
            "Create",
            None,
            "Create a spot light entity.",
            A::CreateEntity(C::SpotLight),
            CREATE,
            always
        ),
        command!(
            "editor.create.area_light",
            "Create Area Light",
            "Create",
            None,
            "Create an area light entity.",
            A::CreateEntity(C::RectLight),
            CREATE,
            always
        ),
        command!(
            "editor.create.disc_light",
            "Create Disc Light",
            "Create",
            None,
            "Create a disc light entity.",
            A::CreateEntity(C::DiscLight),
            CREATE,
            always
        ),
        command!(
            "editor.create.tube_light",
            "Create Tube Light",
            "Create",
            None,
            "Create a tube light entity.",
            A::CreateEntity(C::TubeLight),
            CREATE,
            always
        ),
        command!(
            "editor.create.particle",
            "Create Particle Emitter",
            "Create",
            None,
            "Create a particle emitter entity.",
            A::CreateEntity(C::Particle),
            CREATE,
            always
        ),
        command!(
            "editor.create.terrain",
            "Create Terrain",
            "Create",
            None,
            "Create a terrain entity.",
            A::CreateEntity(C::Terrain),
            CREATE,
            always
        ),
        command!(
            "editor.create.voxel_terrain",
            "Create Voxel Terrain",
            "Create",
            None,
            "Create a voxel terrain entity.",
            A::CreateEntity(C::VoxelTerrain),
            CREATE,
            always
        ),
        command!(
            "editor.create.environment",
            "Create Environment",
            "Create",
            None,
            "Create the scene's day cycle, sky and weather.",
            A::CreateEntity(C::Environment),
            CREATE,
            always
        ),
        command!(
            "editor.asset.new_material",
            "New Material…",
            "Create",
            None,
            "Create an editable Somnium material in the current content folder.",
            A::ContentNewMaterial,
            CREATE_CONTENT,
            always
        ),
        command!(
            "editor.content.new_folder",
            "New Folder…",
            "Content",
            None,
            "Create a folder in the current Content Drawer location.",
            A::ContentNewFolder,
            CONTENT,
            always
        ),
        command!(
            "editor.content.new_script",
            "New Script…",
            "Content",
            None,
            "Create a Luau script in the current Content Drawer location.",
            A::ContentNewScript,
            CONTENT,
            always
        ),
        command!(
            "editor.content.rename",
            "Rename…",
            "Content",
            None,
            "Rename the chosen content item.",
            A::ContentRename,
            CONTENT,
            content_target
        ),
        command!(
            "editor.content.show_in_folder",
            "Show in Folder",
            "Content",
            None,
            "Reveal the chosen content item in the operating system.",
            A::ContentShowInFolder,
            CONTENT,
            content_target
        ),
        command!(
            "editor.content.refresh",
            "Refresh",
            "Content",
            None,
            "Refresh the Content Drawer index.",
            A::ContentRefresh,
            CONTENT,
            always
        ),
    ]
}

/// The single indexed command store.
pub struct CommandRegistry {
    commands: Vec<Command>,
    by_id: HashMap<&'static str, usize>,
}

impl CommandRegistry {
    fn new(commands: Vec<Command>) -> Self {
        let by_id = commands
            .iter()
            .enumerate()
            .map(|(index, command)| (command.id, index))
            .collect();
        Self { commands, by_id }
    }

    /// Every registered command, in declaration order.
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    /// Resolve a stable command id.
    pub fn get(&self, id: &str) -> Option<&Command> {
        self.by_id.get(id).map(|index| &self.commands[*index])
    }

    /// Commands explicitly assigned to an application menu.
    pub fn menu(&self, menu: Menu) -> Vec<&Command> {
        self.commands
            .iter()
            .filter(|command| command.surfaces.contains(&CommandSurface::Menu(menu)))
            .collect()
    }

    /// Commands explicitly assigned to a surface.
    pub fn surface(&self, surface: CommandSurface) -> Vec<&Command> {
        self.commands
            .iter()
            .filter(|command| command.surfaces.contains(&surface))
            .collect()
    }

    /// Resolve an authoritative default binding.
    pub fn binding(&self, chord: Chord) -> Option<&Command> {
        self.commands
            .iter()
            .find(|command| command.default_binding == Some(chord))
    }

    /// F1 command index, derived from non-empty Help declarations.
    pub fn help_index(&self) -> Vec<(&'static str, &'static str, &'static str)> {
        self.commands
            .iter()
            .filter(|command| !command.help.is_empty())
            .map(|command| (command.id, command.label, command.help))
            .collect()
    }
}

/// Process-wide immutable editor registry.
pub fn registry() -> &'static CommandRegistry {
    static REGISTRY: OnceLock<CommandRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        // Hand-written declarations first, then the two generated families.
        // The order matters only for the menu, and "the things a person
        // reaches for" belongs above "every visualisation the shader has".
        let mut commands = declarations();
        commands.extend(view_mode_commands());
        commands.extend(camera_commands());
        commands.extend(time_of_day_commands());
        commands.extend(sky_commands());
        commands.extend(weather_commands());
        CommandRegistry::new(commands)
    })
}

/// Persisted command-palette usage history.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandHistory {
    clock: u64,
    last_used: BTreeMap<String, u64>,
}

impl CommandHistory {
    /// Recency tick for an id, or zero when it has never run.
    pub fn recency(&self, id: &str) -> u64 {
        self.last_used.get(id).copied().unwrap_or(0)
    }

    /// Record successful dispatch and return the new tick.
    pub fn record(&mut self, id: &str) -> u64 {
        self.clock = self.clock.saturating_add(1);
        self.last_used.insert(id.to_string(), self.clock);
        self.clock
    }

    /// Decode history; malformed state safely starts fresh.
    pub fn from_json(json: &str) -> Self {
        serde_json::from_str(json).unwrap_or_default()
    }

    /// Encode history for persistence.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Load editor history from the per-user config directory.
    pub fn load() -> Self {
        std::fs::read_to_string(history_path())
            .map_or_else(|_| Self::default(), |json| Self::from_json(&json))
    }

    /// Persist editor history. Failure is intentionally non-fatal.
    pub fn save(&self) {
        let path = history_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, self.to_json());
    }
}

/// User overrides of the registry's default chords.
///
/// A separate, sparse store rather than a mutable registry: the declarations
/// stay `const`, "reset to default" is a *removal* rather than a remembered
/// second value, and a build that renames a command simply stops matching a
/// stale override instead of resurrecting a binding for something that no
/// longer exists.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyBindings {
    /// `command id -> chord`. An id absent here uses its declared default; an
    /// id present with `None` has been deliberately unbound.
    #[serde(default)]
    overrides: BTreeMap<String, Option<Chord>>,
}

/// One command's binding, with everything the editor needs to draw the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindingRow {
    /// The command this row binds.
    pub command: &'static str,
    /// The chord in force, or `None` when unbound.
    pub chord: Option<Chord>,
    /// Whether it differs from the declaration.
    pub customised: bool,
    /// Whether another command holds the same chord.
    pub conflicted: bool,
}

impl KeyBindings {
    /// The chord in force for `id`.
    #[must_use]
    pub fn chord_for(&self, id: &str, default: Option<Chord>) -> Option<Chord> {
        match self.overrides.get(id) {
            Some(chord) => *chord,
            None => default,
        }
    }

    /// Whether `id` has been changed from its declaration.
    #[must_use]
    pub fn is_customised(&self, id: &str) -> bool {
        self.overrides.contains_key(id)
    }

    /// Bind `id` to `chord`, or to nothing.
    ///
    /// Deliberately does **not** unbind whatever else holds the chord. A
    /// conflict is reported, not silently resolved: quietly stealing a binding
    /// is how a person loses a shortcut they never touched.
    pub fn bind(&mut self, id: &str, chord: Option<Chord>) {
        self.overrides.insert(id.to_string(), chord);
    }

    /// Drop the override so the declared default applies again.
    pub fn reset(&mut self, id: &str) {
        self.overrides.remove(id);
    }

    /// Drop every override.
    pub fn reset_all(&mut self) {
        self.overrides.clear();
    }

    /// The command a chord currently runs, honouring overrides. This is the
    /// dispatch path, so an override takes effect the moment it is set.
    #[must_use]
    pub fn command_for(&self, chord: Chord) -> Option<&'static str> {
        registry()
            .commands()
            .iter()
            .find(|command| self.chord_for(command.id, command.default_binding) == Some(chord))
            .map(|command| command.id)
    }

    /// Every command with a binding, in registry order, with conflicts marked.
    #[must_use]
    pub fn rows(&self) -> Vec<BindingRow> {
        let resolved: Vec<_> = registry()
            .commands()
            .iter()
            .map(|command| {
                (
                    command.id,
                    self.chord_for(command.id, command.default_binding),
                )
            })
            .collect();
        resolved
            .iter()
            .map(|(id, chord)| BindingRow {
                command: id,
                chord: *chord,
                customised: self.is_customised(id),
                conflicted: chord.is_some_and(|chord| {
                    resolved
                        .iter()
                        .filter(|(_, other)| *other == Some(chord))
                        .count()
                        > 1
                }),
            })
            .collect()
    }

    /// Every command that would answer to `chord`. One entry is normal; two or
    /// more is the conflict the editor reports before accepting a rebind.
    #[must_use]
    pub fn conflicts_for(&self, chord: Chord, ignoring: &str) -> Vec<&'static str> {
        registry()
            .commands()
            .iter()
            .filter(|command| command.id != ignoring)
            .filter(|command| self.chord_for(command.id, command.default_binding) == Some(chord))
            .map(|command| command.id)
            .collect()
    }

    /// Decode; malformed state safely starts fresh.
    #[must_use]
    pub fn from_json(json: &str) -> Self {
        serde_json::from_str(json).unwrap_or_default()
    }

    /// Encode for persistence.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Load from the per-user config directory.
    #[must_use]
    pub fn load() -> Self {
        std::fs::read_to_string(bindings_path())
            .map_or_else(|_| Self::default(), |json| Self::from_json(&json))
    }

    /// Persist. Failure is intentionally non-fatal.
    pub fn save(&self) {
        let path = bindings_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, self.to_json());
    }
}

fn bindings_path() -> PathBuf {
    let mut dir = history_path();
    dir.pop();
    dir.push("keybindings.json");
    dir
}

fn history_path() -> PathBuf {
    let mut dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .unwrap_or_else(std::env::temp_dir);
    dir.push("SomniumEngine");
    dir.push("command_history.json");
    dir
}

/// Score a free-text subsequence. Prefixes and word boundaries rank higher.
pub fn fuzzy_score(text: &str, query: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let text = text.to_ascii_lowercase();
    let query = query.to_ascii_lowercase();
    let bytes = text.as_bytes();
    let mut cursor = 0usize;
    let mut score = 0i64;
    let mut previous = None;
    for needle in query.bytes() {
        let offset = bytes
            .get(cursor..)?
            .iter()
            .position(|candidate| *candidate == needle)?;
        let index = cursor + offset;
        score += 10;
        if index == 0 {
            score += 30;
        }
        if index == 0 || !bytes[index - 1].is_ascii_alphanumeric() {
            score += 20;
        }
        if previous.is_some_and(|p| p + 1 == index) {
            score += 12;
        }
        score -= i64::try_from(offset).unwrap_or(i64::MAX / 4);
        previous = Some(index);
        cursor = index + 1;
    }
    Some(score)
}

/// Structured-token command score. Token names are exact; values are exact or
/// prefix matches. Only the free-text remainder is fuzzy.
pub fn command_score(command: &Command, query: &str, recency: u64) -> Option<i64> {
    let mut free = Vec::new();
    for token in query.split_whitespace() {
        let Some((field, value)) = token.split_once(':') else {
            free.push(token);
            continue;
        };
        let value = value.to_ascii_lowercase();
        let haystack = match field {
            "category" => command.category.to_ascii_lowercase(),
            "id" => command.id.to_ascii_lowercase(),
            "binding" => command
                .default_binding
                .map(|binding| binding.to_string().to_ascii_lowercase())
                .unwrap_or_default(),
            _ => return None,
        };
        if !haystack.starts_with(&value) {
            return None;
        }
    }
    let free = free.join(" ");
    let text = format!("{} {} {}", command.label, command.category, command.id);
    fuzzy_score(&text, &free)
        .map(|score| score + i64::try_from(recency.min(10_000)).unwrap_or(10_000))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// CONTROL-G's exit clause: "type 24 into that field" is not required to
    /// reach any debug view. Every code the shader branches on, and every
    /// pipeline switch that used to be shell-only, is a named menu entry with
    /// a Help line.
    #[test]
    fn every_debug_view_and_pipeline_switch_is_a_registered_command() {
        let registry = registry();
        for view in crate::debug::DEBUG_VIEWS {
            let id = format!("editor.view.debug.{}", view.id);
            let command = registry
                .get(&id)
                .unwrap_or_else(|| panic!("{id} is not registered"));
            assert_eq!(command.action, CommandAction::SetDebugView(view.id));
            assert!(!command.help.trim().is_empty());
            assert!(
                command.surfaces.contains(&CommandSurface::Menu(Menu::View)),
                "{id} must appear in the View menu"
            );
        }
        for toggle in crate::debug::TOGGLES {
            let id = format!("editor.view.pipeline.{}", toggle.id);
            let command = registry
                .get(&id)
                .unwrap_or_else(|| panic!("{id} is not registered"));
            assert_eq!(command.action, CommandAction::ToggleRenderSwitch(toggle.id));
        }
    }

    /// Nine bookmarks, set with `command()` and recalled bare — and the two
    /// families must not collide with each other or with anything else.
    #[test]
    fn camera_bookmarks_are_nine_pairs_with_distinct_chords() {
        let registry = registry();
        let mut chords = std::collections::HashSet::new();
        for slot in 1..=9u8 {
            let set = registry
                .get(&format!("editor.view.bookmark.set.{slot}"))
                .expect("set command");
            let recall = registry
                .get(&format!("editor.view.bookmark.recall.{slot}"))
                .expect("recall command");
            assert_ne!(set.default_binding, recall.default_binding);
            assert!(chords.insert(set.default_binding.expect("set has a chord")));
            assert!(chords.insert(recall.default_binding.expect("recall has a chord")));
            assert_eq!(set.action, CommandAction::SetBookmark(slot));
            assert_eq!(recall.action, CommandAction::RecallBookmark(slot));
        }
    }

    /// An override is what dispatch consults; the declaration is the fallback.
    #[test]
    fn a_rebind_takes_effect_and_reset_puts_the_default_back() {
        let save = *registry().get("editor.scene.save").unwrap();
        let default = save.default_binding.expect("Save ships with a chord");
        let rebound = Chord::press(CommandKey::Letter('J')).ctrl().shift();

        let mut bindings = KeyBindings::default();
        assert_eq!(bindings.command_for(default), Some(save.id));

        bindings.bind(save.id, Some(rebound));
        assert_eq!(bindings.command_for(rebound), Some(save.id));
        assert_ne!(
            bindings.command_for(default),
            Some(save.id),
            "the old chord must stop running the rebound command"
        );
        assert!(bindings.is_customised(save.id));

        bindings.reset(save.id);
        assert_eq!(bindings.command_for(default), Some(save.id));
        assert!(!bindings.is_customised(save.id));
    }

    /// A conflict is reported, not silently resolved. Quietly unbinding the
    /// other command is how a person loses a shortcut they never touched.
    #[test]
    fn a_conflicting_rebind_is_reported_and_both_rows_are_marked() {
        let undo = *registry().get("editor.edit.undo").unwrap();
        let save = *registry().get("editor.scene.save").unwrap();
        let chord = undo.default_binding.expect("Undo ships with a chord");

        let mut bindings = KeyBindings::default();
        assert!(bindings.conflicts_for(chord, save.id).contains(&undo.id));
        bindings.bind(save.id, Some(chord));

        let rows = bindings.rows();
        let conflicted: Vec<_> = rows
            .iter()
            .filter(|row| row.conflicted)
            .map(|row| row.command)
            .collect();
        assert!(conflicted.contains(&undo.id));
        assert!(conflicted.contains(&save.id));
        assert!(
            bindings.command_for(chord).is_some(),
            "one of them still answers; neither was silently unbound"
        );
    }

    /// Unbinding is a distinct state from "never customised", so a deliberate
    /// unbind survives a restart instead of reverting on the next launch.
    #[test]
    fn an_explicit_unbind_is_not_the_same_as_a_reset() {
        let save = *registry().get("editor.scene.save").unwrap();
        let mut bindings = KeyBindings::default();
        bindings.bind(save.id, None);
        assert!(bindings.is_customised(save.id));
        assert_eq!(bindings.chord_for(save.id, save.default_binding), None);

        let reloaded = KeyBindings::from_json(&bindings.to_json());
        assert_eq!(reloaded, bindings, "the unbind survives a round trip");

        bindings.reset(save.id);
        assert_eq!(
            bindings.chord_for(save.id, save.default_binding),
            save.default_binding
        );
    }

    /// A stale override for a command this build no longer declares must not
    /// resurrect anything.
    #[test]
    fn an_override_for_an_unknown_command_is_inert() {
        let mut bindings = KeyBindings::default();
        let chord = Chord::press(CommandKey::Letter('Q')).ctrl().alt();
        bindings.bind("editor.deleted.command", Some(chord));
        assert_eq!(bindings.command_for(chord), None);
        assert!(bindings.rows().iter().all(|row| row.chord != Some(chord)));
    }

    /// Malformed state starts fresh rather than refusing to launch.
    #[test]
    fn a_corrupt_bindings_file_starts_fresh() {
        assert_eq!(KeyBindings::from_json("{ not json"), KeyBindings::default());
    }

    /// CONTROL-F: the Outliner's menu is registry-derived, so adding a
    /// command declares it there too. The test names the seven the phase asks
    /// for so a rename cannot silently drop one.
    #[test]
    fn the_outliner_context_menu_is_built_from_the_registry() {
        let ids: Vec<_> = registry()
            .surface(CommandSurface::OutlinerContext)
            .into_iter()
            .map(|command| command.id)
            .collect();
        for wanted in [
            "editor.edit.copy",
            "editor.edit.paste",
            "editor.edit.delete",
            "editor.edit.duplicate",
            "editor.edit.rename",
            "editor.view.focus_selection",
        ] {
            assert!(
                ids.contains(&wanted),
                "{wanted} must reach the Outliner menu"
            );
        }
    }

    /// Paste is disabled until something is copied, and says why.
    #[test]
    fn paste_states_its_own_precondition() {
        let command = *registry().get("editor.edit.paste").unwrap();
        let empty = EditorCtx::default();
        assert_eq!(
            command.enabled(&empty),
            Enablement::Disabled("Copy something first")
        );
        let filled = EditorCtx {
            has_clipboard: true,
            ..EditorCtx::default()
        };
        assert!(command.enabled(&filled).is_enabled());
    }

    #[test]
    fn ids_bindings_help_and_palette_coverage_are_complete() {
        let mut ids = HashSet::new();
        let mut bindings = HashSet::new();
        for command in registry().commands() {
            assert!(
                ids.insert(command.id),
                "duplicate command id: {}",
                command.id
            );
            assert!(
                !command.help.trim().is_empty(),
                "missing help: {}",
                command.id
            );
            if let Some(binding) = command.default_binding {
                assert!(bindings.insert(binding), "duplicate binding: {binding}");
                assert_eq!(
                    registry().binding(binding).map(|found| found.id),
                    Some(command.id)
                );
            }
        }
        assert_eq!(registry().help_index().len(), registry().commands().len());
    }

    #[test]
    fn chord_display_is_derived_from_the_value() {
        assert_eq!(
            Chord::press(CommandKey::Letter('s'))
                .ctrl()
                .shift()
                .to_string(),
            "Ctrl+Shift+S"
        );
        assert_eq!(Chord::press(CommandKey::Delete).to_string(), "Del");
    }

    #[test]
    fn extra_modifiers_do_not_match_a_registered_binding() {
        assert!(
            registry()
                .binding(Chord::press(CommandKey::Letter('S')).ctrl().shift())
                .is_none(),
            "Ctrl+Shift+S must not silently run the Ctrl+S command"
        );
        assert!(
            registry()
                .binding(Chord::press(CommandKey::Delete).shift())
                .is_none(),
            "Shift+Delete must not silently run the unmodified Delete command"
        );
    }

    #[test]
    fn structured_tokens_are_predictable_and_free_text_is_fuzzy() {
        let save = registry().get("editor.scene.save").unwrap();
        assert!(command_score(save, "category:sc svsc", 0).is_some());
        assert!(command_score(save, "category:scene save", 0).is_some());
        assert!(command_score(save, "category:light save", 0).is_none());
        assert!(
            command_score(save, "categroy:scene", 0).is_none(),
            "token names must not fuzzy-match"
        );
        assert!(
            fuzzy_score("Save Scene", "ss").unwrap()
                > fuzzy_score("Toggle Shading", "ss").unwrap_or(i64::MIN)
        );
    }

    #[test]
    fn recency_round_trips_and_breaks_equal_search_scores() {
        let mut history = CommandHistory::default();
        history.record("editor.scene.save");
        history.record("editor.scene.new");
        let decoded = CommandHistory::from_json(&history.to_json());
        assert_eq!(decoded, history);
        assert!(decoded.recency("editor.scene.new") > decoded.recency("editor.scene.save"));
    }

    #[test]
    fn new_material_is_one_declaration_on_both_create_surfaces() {
        let command = registry().get("editor.asset.new_material").unwrap();
        assert_eq!(command.action, CommandAction::ContentNewMaterial);
        assert!(
            registry()
                .menu(Menu::Create)
                .iter()
                .any(|item| item.id == command.id)
        );
        assert!(
            registry()
                .surface(CommandSurface::ContentContext)
                .iter()
                .any(|item| item.id == command.id)
        );
    }
}
