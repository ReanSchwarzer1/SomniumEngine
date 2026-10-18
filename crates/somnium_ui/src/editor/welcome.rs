//! The welcome screen: the projects this machine has opened, Open Project…, and the way into Help.
//!
//! A modal card like Preferences, drawn in the authoring panel's recipe: a glyph-led title, caps section headers with
//! counts, icon buttons. It owns no files. The host reads the Recent Projects list, says which folders are gone and
//! which have a picture, and gets back what the designer chose as a [`WelcomeAction`].
//!
//! There is no New Project button: the engine cannot create a project from a template yet, and a button that opened
//! a folder dialog under another name would be a fake.

use std::path::PathBuf;

use crate::{
    authoring_panel::{glyph, icon_button, label, row, section_header},
    icons::IconId,
    message::{MessageDirection, NodeHandle, UiMessage},
    theme,
    types::{HorizontalAlignment, Thickness, VerticalAlignment},
    typography::TextRole,
    ui::UserInterface,
    widget::WidgetBuilder,
    widgets::{
        border::{BorderBuilder, Surface},
        button::{ButtonBuilder, ButtonMessage},
        grid::{Column, GridBuilder, Row},
        image::ImageBuilder,
        popup::{PopupBuilder, PopupPlacement},
        scroll_viewer::ScrollViewerBuilder,
        stack_panel::{Orientation, StackPanelBuilder},
        text::TextBuilder,
    },
};

use super::parts::window_chrome_button;

/// One row of the Recent Projects list, as the host finds it now.
#[derive(Clone, Debug, PartialEq)]
pub struct WelcomeProject {
    /// The project's name when it was last opened.
    pub name: String,
    /// The project folder, the one holding `game.project.json`.
    pub path: PathBuf,
    /// When it was last opened, in words: "2 hours ago".
    pub opened: String,
    /// The project's picture, `thumbnail.png` in its folder, when there is one. The host delivers its pixels to the
    /// thumbnail atlas under this path; until then the row shows a folder.
    pub thumbnail: Option<PathBuf>,
    /// The folder, or its `game.project.json`, is gone.
    pub missing: bool,
    /// The project open in this editor.
    pub current: bool,
}

/// What a click on the welcome screen asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WelcomeAction {
    /// Open this project folder in its editor.
    Open(PathBuf),
    /// Take this folder off the list. The folder itself is not touched.
    Forget(PathBuf),
    /// Choose a project folder on disk.
    OpenPicker,
    /// Open this Help page.
    Help(u8),
    Close,
}

pub(crate) struct WelcomeScreen {
    /// The modal popup.
    pub overlay: NodeHandle,
    /// Open Project…, which also takes the keyboard focus when the screen opens.
    pub open_project: NodeHandle,
    close: NodeHandle,
    /// Holds the Recent Projects heading, rebuilt with its count.
    recent_header: NodeHandle,
    /// Holds the project rows, rebuilt whenever the list changes.
    list: NodeHandle,
    help: Vec<(NodeHandle, u8)>,
    /// Row buttons that open a project. Missing projects have none.
    rows: Vec<(NodeHandle, PathBuf)>,
    forget: Vec<(NodeHandle, PathBuf)>,
    projects: Option<Vec<WelcomeProject>>,
}

impl WelcomeScreen {
    pub(crate) fn build(ui: &mut UserInterface, root: NodeHandle) -> Self {
        let tokens = theme::active().semantic;
        let overlay = ui.add_node(
            PopupBuilder::new(WidgetBuilder::new().with_background(tokens.surface.modal_scrim.bytes()))
                .with_placement(PopupPlacement::Center)
                .build(),
            root,
        );
        let card = ui.add_node(
            BorderBuilder::new(
                WidgetBuilder::new()
                    .with_width(900.0)
                    .with_height(580.0)
                    .with_horizontal_alignment(HorizontalAlignment::Center)
                    .with_vertical_alignment(VerticalAlignment::Center)
                    .with_background(tokens.surface.panel.bytes())
                    .with_foreground(tokens.border.subtle.bytes()),
            )
            .with_surface(Surface::Modal)
            .with_stroke_thickness(Thickness::uniform(1.0))
            .build(),
            overlay,
        );
        let grid = ui.add_node(
            GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
                .add_row(Row::strict(68.0))
                .add_row(Row::stretch())
                .add_column(Column::stretch())
                .build(),
            card,
        );

        // ── header: the mark, the greeting, close ───────────────────────────
        let header = ui.add_node(
            BorderBuilder::new(
                WidgetBuilder::new()
                    .with_row(0)
                    .with_background(theme::TRANSPARENT)
                    .with_foreground(tokens.border.subtle.bytes()),
            )
            .with_stroke_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
            .build(),
            grid,
        );
        let header_grid = ui.add_node(
            GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
                .add_row(Row::stretch())
                .add_column(Column::stretch())
                .add_column(Column::auto())
                .build(),
            header,
        );
        let title_row = ui.add_node(
            StackPanelBuilder::new(
                WidgetBuilder::new()
                    .with_column(0)
                    .with_margin(Thickness::axes(16.0, 0.0))
                    .with_vertical_alignment(VerticalAlignment::Center)
                    .with_background(theme::TRANSPARENT),
            )
            .with_orientation(Orientation::Horizontal)
            .build(),
            header_grid,
        );
        glyph(ui, title_row, IconId::EngineMark, 32.0, tokens.accent.default.bytes());
        let words = ui.add_node(
            StackPanelBuilder::new(
                WidgetBuilder::new()
                    .with_margin(Thickness::new(12.0, 0.0, 0.0, 0.0))
                    .with_vertical_alignment(VerticalAlignment::Center)
                    .with_background(theme::TRANSPARENT),
            )
            .with_orientation(Orientation::Vertical)
            .build(),
            title_row,
        );
        text(ui, words, "Welcome to Somnium", TextRole::Title, None);
        text(
            ui,
            words,
            "Open a project to begin, or pick up where you left off.",
            TextRole::Caption,
            Some(tokens.text.secondary.bytes()),
        );
        let close_col = ui.add_node(
            StackPanelBuilder::new(
                WidgetBuilder::new()
                    .with_column(1)
                    .with_vertical_alignment(VerticalAlignment::Top)
                    .with_background(theme::TRANSPARENT),
            )
            .with_orientation(Orientation::Horizontal)
            .build(),
            header_grid,
        );
        let close = window_chrome_button(ui, close_col, IconId::Close, "Close");

        // ── body: recent projects, and beside them Open and Help ────────────
        let body = ui.add_node(
            GridBuilder::new(WidgetBuilder::new().with_row(1).with_background(theme::TRANSPARENT))
                .add_row(Row::stretch())
                .add_column(Column::stretch())
                .add_column(Column::strict(270.0))
                .build(),
            grid,
        );
        let left = ui.add_node(
            GridBuilder::new(
                WidgetBuilder::new()
                    .with_column(0)
                    .with_margin(Thickness::new(10.0, 4.0, 10.0, 10.0))
                    .with_background(theme::TRANSPARENT),
            )
            .add_row(Row::auto())
            .add_row(Row::stretch())
            .add_row(Row::auto())
            .add_column(Column::stretch())
            .build(),
            body,
        );
        let recent_header = ui.add_node(
            StackPanelBuilder::new(WidgetBuilder::new().with_row(0).with_background(theme::TRANSPARENT))
                .with_orientation(Orientation::Vertical)
                .build(),
            left,
        );
        let scroll = ui.add_node(
            ScrollViewerBuilder::new(
                WidgetBuilder::new()
                    .with_row(1)
                    .with_margin(Thickness::new(0.0, 4.0, 0.0, 4.0))
                    .with_background(theme::TRANSPARENT),
            )
            .build(),
            left,
        );
        let list = ui.add_node(
            StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
                .with_orientation(Orientation::Vertical)
                .build(),
            scroll,
        );
        let foot = ui.add_node(
            StackPanelBuilder::new(WidgetBuilder::new().with_row(2).with_background(theme::TRANSPARENT))
                .with_orientation(Orientation::Vertical)
                .build(),
            left,
        );
        label(
            ui,
            foot,
            "Projects appear here as you open them. File > Save Project Thumbnail gives the open one its picture.",
            TextRole::Caption,
        );

        let side = ui.add_node(
            BorderBuilder::new(
                WidgetBuilder::new()
                    .with_column(1)
                    .with_background(tokens.surface.header.bytes())
                    .with_foreground(tokens.border.subtle.bytes()),
            )
            .with_stroke_thickness(Thickness::new(1.0, 0.0, 0.0, 0.0))
            .build(),
            body,
        );
        let side_scroll = ui.add_node(
            ScrollViewerBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT)).build(),
            side,
        );
        let side_stack = ui.add_node(
            StackPanelBuilder::new(
                WidgetBuilder::new()
                    .with_margin(Thickness::new(8.0, 12.0, 8.0, 8.0))
                    .with_background(theme::TRANSPARENT),
            )
            .with_orientation(Orientation::Vertical)
            .build(),
            side_scroll,
        );
        let open_row = row(ui, side_stack, Thickness::axes(4.0, 0.0));
        let open_project = icon_button(
            ui,
            open_row,
            IconId::FolderOpen,
            "Open Project…",
            true,
            "Choose a folder that holds a game.project.json. The project opens in its own editor; this one stays open.",
        );
        label(
            ui,
            side_stack,
            "Opens a game folder in the editor built for it.",
            TextRole::Caption,
        );
        let titles = crate::metaphor::help_titles();
        section_header(ui, side_stack, IconId::Docs, "Learn", titles.len());
        let mut help = Vec::with_capacity(titles.len());
        for (page, title) in titles.iter().enumerate() {
            // Page 0 is the index: it lists every topic and every command.
            let (text, icon) = if page == 0 {
                ("Help index", IconId::HelpCircle)
            } else {
                (*title, topic_icon(title))
            };
            help.push((link(ui, side_stack, icon, text), page as u8));
        }

        ui.set_visibility(overlay, true);
        Self {
            overlay,
            open_project,
            close,
            recent_header,
            list,
            help,
            rows: Vec::new(),
            forget: Vec::new(),
            projects: None,
        }
    }

    /// Show this list. Rebuilds the rows only when it differs from what is showing.
    pub(crate) fn set_projects(&mut self, ui: &mut UserInterface, projects: &[WelcomeProject]) {
        if self.projects.as_deref() == Some(projects) {
            return;
        }
        self.projects = Some(projects.to_vec());
        ui.clear_children(self.recent_header);
        ui.clear_children(self.list);
        self.rows.clear();
        self.forget.clear();
        section_header(ui, self.recent_header, IconId::History, "Recent projects", projects.len());
        if projects.is_empty() {
            label(
                ui,
                self.list,
                "No recent projects yet. Open one, and it will be waiting here next time.",
                TextRole::Body,
            );
        }
        for project in projects {
            let (open, forget) = project_row(ui, self.list, project);
            if let Some(open) = open {
                self.rows.push((open, project.path.clone()));
            }
            self.forget.push((forget, project.path.clone()));
        }
        ui.invalidate_ancestors(self.list);
    }

    /// What a message from one of this screen's controls asks for.
    pub(crate) fn event(&self, message: &UiMessage) -> Option<WelcomeAction> {
        if message.direction != MessageDirection::FromWidget
            || !matches!(message.data::<ButtonMessage>(), Some(ButtonMessage::Click))
        {
            return None;
        }
        let target = message.destination;
        if target == self.close {
            return Some(WelcomeAction::Close);
        }
        if target == self.open_project {
            return Some(WelcomeAction::OpenPicker);
        }
        if let Some(&(_, page)) = self.help.iter().find(|(h, _)| *h == target) {
            return Some(WelcomeAction::Help(page));
        }
        if let Some((_, path)) = self.forget.iter().find(|(h, _)| *h == target) {
            return Some(WelcomeAction::Forget(path.clone()));
        }
        self.rows
            .iter()
            .find(|(h, _)| *h == target)
            .map(|(_, path)| WelcomeAction::Open(path.clone()))
    }
}

/// A project: its picture, name, folder and when it was opened, then a button that takes it off the list.
/// Returns the row's open button (none when the folder is gone) and its remove button.
fn project_row(ui: &mut UserInterface, parent: NodeHandle, project: &WelcomeProject) -> (Option<NodeHandle>, NodeHandle) {
    let tokens = theme::active().semantic;
    let line = ui.add_node(
        GridBuilder::new(
            WidgetBuilder::new()
                .with_margin(Thickness::new(0.0, 0.0, 0.0, 4.0))
                .with_background(theme::TRANSPARENT),
        )
        .add_row(Row::strict(72.0))
        .add_column(Column::stretch())
        .add_column(Column::strict(36.0))
        .build(),
        parent,
    );
    let location = shown_path(&project.path);
    let tooltip = if project.missing {
        format!("{location} is gone, or no longer holds a game.project.json. Restore the folder, or remove it from the list.")
    } else if project.current {
        format!("{location}. This project is open in this editor.")
    } else {
        format!("{location}. Opens in the project's own editor; this one stays open.")
    };
    let open = ui.add_node(
        ButtonBuilder::new(
            WidgetBuilder::new()
                .with_column(0)
                .with_enabled(!project.missing)
                .with_tooltip(tooltip)
                .with_background(tokens.surface.raised.bytes()),
        )
        .build(),
        line,
    );
    let inner = ui.add_node(
        GridBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
            .add_row(Row::stretch())
            .add_column(Column::strict(80.0))
            .add_column(Column::stretch())
            .build(),
        open,
    );
    let (icon, tint) = if project.missing {
        (IconId::Warn, tokens.status.warning.bytes())
    } else {
        (IconId::Folder, tokens.text.muted.bytes())
    };
    let mut picture = ImageBuilder::new(
        WidgetBuilder::new()
            .with_column(0)
            .with_horizontal_alignment(HorizontalAlignment::Center)
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_hit_test_visibility(false),
    )
    .with_icon(icon)
    .with_size(if project.thumbnail.is_some() { 64.0 } else { 32.0 })
    .with_tint(tint);
    if let Some(thumbnail) = &project.thumbnail {
        picture = picture.with_asset(thumbnail.clone());
    }
    ui.add_node(picture.build(), inner);
    // The name and the project's state share the top line, so the folder below has the row's whole width.
    let words = ui.add_node(
        StackPanelBuilder::new(
            WidgetBuilder::new()
                .with_column(1)
                .with_margin(Thickness::new(0.0, 0.0, 12.0, 0.0))
                .with_vertical_alignment(VerticalAlignment::Center)
                .with_clip_to_bounds(true)
                .with_hit_test_visibility(false)
                .with_background(theme::TRANSPARENT),
        )
        .with_orientation(Orientation::Vertical)
        .build(),
        inner,
    );
    let top = ui.add_node(
        GridBuilder::new(WidgetBuilder::new().with_hit_test_visibility(false).with_background(theme::TRANSPARENT))
            .add_row(Row::auto())
            .add_column(Column::stretch())
            .add_column(Column::auto())
            .build(),
        words,
    );
    let name_ink = if project.missing { tokens.text.disabled } else { tokens.text.primary };
    text(ui, top, &project.name, TextRole::BodyStrong, Some(name_ink.bytes()));
    text(ui, words, &location, TextRole::Caption, Some(tokens.text.muted.bytes()));
    let (status, ink) = if project.missing {
        ("Folder not found".to_string(), tokens.status.warning)
    } else if project.current {
        ("Open now".to_string(), tokens.accent.default)
    } else {
        (format!("Opened {}", project.opened), tokens.text.secondary)
    };
    let status_row = ui.add_node(
        StackPanelBuilder::new(
            WidgetBuilder::new()
                .with_column(1)
                .with_margin(Thickness::new(12.0, 0.0, 0.0, 0.0))
                .with_vertical_alignment(VerticalAlignment::Center)
                .with_hit_test_visibility(false)
                .with_background(theme::TRANSPARENT),
        )
        .with_orientation(Orientation::Horizontal)
        .build(),
        top,
    );
    if project.current {
        glyph(ui, status_row, IconId::Ok, 14.0, ink.bytes());
    }
    text_at(ui, status_row, &status, TextRole::Caption, Some(ink.bytes()), 6.0);

    let forget = ui.add_node(
        ButtonBuilder::new(
            WidgetBuilder::new()
                .with_column(1)
                .with_width(32.0)
                .with_height(32.0)
                .with_horizontal_alignment(HorizontalAlignment::Right)
                .with_vertical_alignment(VerticalAlignment::Center)
                .with_tooltip("Remove from this list. The folder is not touched.")
                .with_background(theme::TRANSPARENT),
        )
        .build(),
        line,
    );
    glyph(ui, forget, IconId::Remove, 16.0, tokens.text.secondary.bytes());
    ((!project.missing).then_some(open), forget)
}

/// One Help topic, as a quiet row with its glyph.
fn link(ui: &mut UserInterface, parent: NodeHandle, icon: IconId, words: &str) -> NodeHandle {
    let tokens = theme::active().semantic;
    let button = ui.add_node(
        ButtonBuilder::new(
            WidgetBuilder::new()
                .with_height(26.0)
                .with_margin(Thickness::new(4.0, 1.0, 4.0, 1.0))
                .with_tooltip(format!("Open {words} in Help"))
                .with_background(theme::TRANSPARENT),
        )
        .with_variant(crate::style::ButtonVariant::Quiet)
        .build(),
        parent,
    );
    let inner = row(ui, button, Thickness::axes(8.0, 0.0));
    glyph(ui, inner, icon, 14.0, tokens.text.muted.bytes());
    text_at(ui, inner, words, TextRole::Label, Some(tokens.text.link.bytes()), 8.0);
    button
}

/// A line of text that never takes the click from the button it sits in.
fn text(ui: &mut UserInterface, parent: NodeHandle, words: &str, role: TextRole, ink: Option<[u8; 4]>) -> NodeHandle {
    text_at(ui, parent, words, role, ink, 0.0)
}

/// [`text`], set `left` pixels after whatever precedes it in a row.
fn text_at(
    ui: &mut UserInterface,
    parent: NodeHandle,
    words: &str,
    role: TextRole,
    ink: Option<[u8; 4]>,
    left: f32,
) -> NodeHandle {
    let mut builder = TextBuilder::new(
        WidgetBuilder::new()
            .with_margin(Thickness::new(left, 0.0, 0.0, 0.0))
            .with_vertical_alignment(VerticalAlignment::Center)
            .with_hit_test_visibility(false),
    )
    .with_role(role)
    .with_text(words);
    if let Some(ink) = ink {
        builder = builder.with_color(ink);
    }
    ui.add_node(builder.build(), parent)
}

/// The folder as a person writes it: no `\\?\` prefix from canonicalisation.
fn shown_path(path: &std::path::Path) -> String {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => text.strip_prefix(r"\\?\").unwrap_or(&text).to_string(),
    }
}

fn topic_icon(title: &str) -> IconId {
    match title {
        "Viewport" => IconId::Camera,
        "Shortcuts" => IconId::Keyboard,
        "Content Drawer" => IconId::ContentDrawer,
        "About" => IconId::Info,
        "Outliner" => IconId::Hierarchy,
        "Terrain" => IconId::Terrain,
        "Water" => IconId::Water,
        "Lighting" => IconId::Light,
        "Scripting" => IconId::Script,
        "Designer Tools" => IconId::Prefab,
        "Story" => IconId::User,
        _ => IconId::Docs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projects() -> Vec<WelcomeProject> {
        vec![
            WelcomeProject {
                name: "The Somnus Fracture".into(),
                path: PathBuf::from(r"\\?\C:\Games\TheSomnusFracture"),
                opened: "just now".into(),
                thumbnail: None,
                missing: false,
                current: true,
            },
            WelcomeProject {
                name: "Coastal".into(),
                path: PathBuf::from(r"C:\Games\Coastal"),
                opened: "2 hours ago".into(),
                thumbnail: Some(PathBuf::from(r"C:\Games\Coastal\thumbnail.png")),
                missing: false,
                current: false,
            },
            WelcomeProject {
                name: "Old Prototype".into(),
                path: PathBuf::from(r"D:\Gone\Prototype"),
                opened: "3 days ago".into(),
                thumbnail: None,
                missing: true,
                current: false,
            },
        ]
    }

    fn screen(projects: &[WelcomeProject]) -> (UserInterface, WelcomeScreen) {
        let mut ui = UserInterface::new(1280.0, 720.0);
        let root = ui.root();
        let mut screen = WelcomeScreen::build(&mut ui, root);
        screen.set_projects(&mut ui, projects);
        (ui, screen)
    }

    /// Every word on the screen, and every tooltip, in tree order.
    fn words(ui: &UserInterface, from: NodeHandle) -> Vec<String> {
        let node = ui.nodes.borrow(from.transmute());
        let mut out: Vec<String> = node.control.a11y_name().into_iter().collect();
        if !node.widget.tooltip.is_empty() {
            out.push(node.widget.tooltip.clone());
        }
        for child in ui.children(from) {
            out.extend(words(ui, child));
        }
        out
    }

    fn click(target: NodeHandle) -> UiMessage {
        UiMessage::new(target, MessageDirection::FromWidget, ButtonMessage::Click)
    }

    #[test]
    fn each_project_shows_its_name_folder_and_state() {
        let (ui, screen) = screen(&projects());
        let words = words(&ui, screen.overlay);
        for expected in [
            "RECENT PROJECTS  3",
            "The Somnus Fracture",
            r"C:\Games\TheSomnusFracture",
            "Open now",
            "Coastal",
            "Opened 2 hours ago",
            "Old Prototype",
            "Folder not found",
        ] {
            assert!(
                words.iter().any(|w| w.eq_ignore_ascii_case(expected)),
                "{expected:?} missing from {words:#?}"
            );
        }
        assert!(!words.iter().any(|w| w.contains(r"\\?\")), "canonical prefixes are hidden");
        // The missing folder cannot be opened, but it can be removed.
        assert_eq!(screen.rows.len(), 2);
        assert_eq!(screen.forget.len(), 3);
    }

    #[test]
    fn every_control_asks_for_something_real() {
        let list = projects();
        let (ui, screen) = screen(&list);
        assert_eq!(screen.event(&click(screen.close)), Some(WelcomeAction::Close));
        assert_eq!(screen.event(&click(screen.open_project)), Some(WelcomeAction::OpenPicker));
        assert_eq!(screen.help.len(), crate::metaphor::help_titles().len());
        for &(link, page) in &screen.help {
            assert_eq!(screen.event(&click(link)), Some(WelcomeAction::Help(page)));
        }
        for (open, path) in &screen.rows {
            assert_eq!(screen.event(&click(*open)), Some(WelcomeAction::Open(path.clone())));
        }
        for (forget, path) in &screen.forget {
            assert_eq!(screen.event(&click(*forget)), Some(WelcomeAction::Forget(path.clone())));
        }
        // The missing project's row is drawn but disabled, and opens nothing even if a click reached it.
        let missing = &list[2].path;
        assert!(!screen.rows.iter().any(|(_, path)| path == missing));
        let disabled = ui
            .children(screen.list)
            .into_iter()
            .map(|line| ui.children(line)[0])
            .filter(|button| !ui.nodes.borrow(button.transmute()).widget.enabled)
            .count();
        assert_eq!(disabled, 1);
        // Anything that is not a click is ignored.
        let hover = UiMessage::new(screen.close, MessageDirection::ToWidget, ButtonMessage::Click);
        assert_eq!(screen.event(&hover), None);
    }

    #[test]
    fn an_empty_list_says_what_will_appear() {
        let (ui, screen) = screen(&[]);
        let words = words(&ui, screen.overlay);
        assert!(words.iter().any(|w| w == "RECENT PROJECTS  0"), "{words:#?}");
        assert!(words.iter().any(|w| w.starts_with("No recent projects yet")));
        assert!(screen.rows.is_empty() && screen.forget.is_empty());
    }

    #[test]
    fn the_same_list_twice_keeps_the_rows() {
        let list = projects();
        let (mut ui, mut screen) = screen(&list);
        let before = screen.rows.clone();
        screen.set_projects(&mut ui, &list);
        assert_eq!(screen.rows, before, "an unchanged list is not rebuilt");
        screen.set_projects(&mut ui, &list[1..]);
        assert_eq!(screen.forget.len(), 2);
        assert!(words(&ui, screen.overlay).iter().any(|w| w == "RECENT PROJECTS  2"));
    }

    #[test]
    fn help_links_cover_every_topic_and_start_at_the_index() {
        let (ui, screen) = screen(&projects());
        let words = words(&ui, screen.overlay);
        assert!(words.iter().any(|w| w == "Help index"));
        for title in &crate::metaphor::help_titles()[1..] {
            assert!(words.iter().any(|w| w == title), "{title} has no link");
        }
        assert_eq!(screen.help[0].1, 0);
    }

    #[test]
    fn there_is_no_new_project_button_until_the_engine_can_make_one() {
        let (ui, screen) = screen(&projects());
        assert!(!words(&ui, screen.overlay).iter().any(|w| w.contains("New Project")));
    }

    #[test]
    fn nothing_on_the_welcome_screen_names_a_project_phase() {
        let (ui, screen) = screen(&projects());
        for text in words(&ui, screen.overlay) {
            for name in crate::debug::tests::CODENAMES {
                assert!(!text.contains(name), "{text:?} names {name:?}");
            }
        }
    }

    #[test]
    fn the_card_fits_a_small_editor_window() {
        let (mut ui, screen) = screen(&projects());
        ui.send(UiMessage::new(
            screen.overlay,
            MessageDirection::ToWidget,
            crate::widgets::popup::PopupMessage::Open,
        ));
        ui.update();
        ui.perform_layout();
        let card = ui.first_child(screen.overlay);
        let bounds = ui.screen_bounds(card);
        assert!(bounds.w == 900.0 && bounds.h == 580.0, "{bounds:?}");
        assert!(bounds.x >= 0.0 && bounds.y >= 0.0 && bounds.x + bounds.w <= 1280.0 && bounds.y + bounds.h <= 720.0);
        // Every row sits inside the card.
        for (row, _) in &screen.forget {
            let r = ui.screen_bounds(*row);
            assert!(r.x >= bounds.x && r.x + r.w <= bounds.x + bounds.w, "{r:?} outside {bounds:?}");
        }
    }
}
