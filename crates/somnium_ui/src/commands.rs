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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
            EDIT,
            selection
        ),
        command!(
            "editor.edit.duplicate",
            "Duplicate",
            "Edit",
            ctrl('D'),
            "Duplicate the selected entity.",
            A::DuplicateSelected,
            EDIT,
            selection
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
    REGISTRY.get_or_init(|| CommandRegistry::new(declarations()))
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
