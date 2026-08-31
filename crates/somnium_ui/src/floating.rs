//! A panel living in its own OS window.
//!
//! MORROWIND-J step 2. Step 1 built a dock tree that can express where a panel
//! sits; this is the case it cannot express, because the panel is not in the
//! window at all.
//!
//! # What a floating panel actually needs
//!
//! Not a widget. A second OS window is a second **surface**, and a surface is
//! the thing a [`crate::pass::UiPass`] renders into, so a floating panel owns a
//! parallel stack:
//!
//! ```text
//!   main window          floating window
//!   ───────────          ───────────────
//!   winit::Window        winit::Window        ┐ the host's (somnium_core)
//!   wgpu::Surface        wgpu::Surface        ┘
//!   UserInterface        UserInterface        ┐ this module's
//!   UiPass               UiPass               ┘
//! ```
//!
//! # Why a panel is rebuilt rather than moved
//!
//! A widget tree owns its handles: they index one [`crate::ui::UserInterface`]'s
//! pool, and a node cannot be re-parented across two of them. Detaching a panel
//! therefore means **building it again** in the new tree from the same data,
//! which is only possible when the panel's content is a *store* rather than a
//! pile of widgets.
//!
//! That is the whole rule for which panels can float. [`crate::log::OutputLog`]
//! is a store and the Outliner's rows are one, so both float. The Details panel
//! generates its rows from reflected schemas against the live selection, and the
//! viewport is a hole the renderer draws through. Neither has a store to rebuild
//! from yet, and giving them one is their own piece of work.

use crate::{
    log::{LogSeverity, OutputLog},
    message::{MessageDirection, NodeHandle, UiMessage},
    theme,
    types::{Rect, Thickness},
    typography::{TextRole, text_style},
    ui::UserInterface,
    widget::WidgetBuilder,
    widgets::{
        border::BorderBuilder,
        scroll_viewer::ScrollViewerBuilder,
        stack_panel::{Orientation, StackPanelBuilder},
        text::TextBuilder,
        tree_view::{TreeItem, TreeViewBuilder, TreeViewMessage},
    },
};

/// Which panel a floating window is showing.
///
/// An enum rather than a `NodeHandle`, because the window outlives any tree: it
/// is closed and reopened, and what it has to remember across that is *which
/// panel it is*, not where the widgets were.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FloatingKind {
    /// The Output Log.
    OutputLog,
    /// The Outliner's entity tree.
    Outliner,
}

impl FloatingKind {
    /// Every panel that can float, in menu order.
    pub const ALL: [Self; 2] = [Self::Outliner, Self::OutputLog];

    /// The panel `SOMNIUM_FLOAT` asks to open at startup, if it asks for one.
    ///
    /// A window only a menu can open is a window no automated run can look at,
    /// and this one has a GPU surface of its own.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        match std::env::var("SOMNIUM_FLOAT").ok()?.trim() {
            "log" => Some(Self::OutputLog),
            "outliner" => Some(Self::Outliner),
            other => {
                tracing::warn!("SOMNIUM_FLOAT={other} is not a panel name; ignoring");
                None
            }
        }
    }

    /// The name as it reads in a menu.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::OutputLog => "Output Log",
            Self::Outliner => "Outliner",
        }
    }

    /// The window title.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::OutputLog => "Output Log - Somnium",
            Self::Outliner => "Outliner - Somnium",
        }
    }

    /// The size the window opens at, in logical pixels.
    #[must_use]
    pub const fn default_size(self) -> (u32, u32) {
        match self {
            // Wide and short: a log is read a line at a time and the lines are
            // long. Tall and narrow: an outliner is a list of short names.
            Self::OutputLog => (900, 420),
            Self::Outliner => (360, 720),
        }
    }
}

/// What the host hands a panel so it can rebuild itself.
///
/// One method takes this rather than one method per panel, so adding a panel
/// widens an enum instead of widening the interface every caller has to learn.
pub enum PanelData<'a> {
    /// Lines for the Output Log.
    Log(&'a OutputLog),
    /// Rows for the Outliner, already filtered and projected.
    Outliner {
        items: &'a [TreeItem],
        selected: Option<u32>,
    },
}

/// Something the user did in a floating window that the editor has to act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelEvent {
    /// A row was clicked. The editor owns what selection means.
    SelectEntity(u32),
}

/// The widgets one kind of panel is made of.
enum Content {
    Log {
        stack: NodeHandle,
        rows: Vec<NodeHandle>,
        /// The newest entry this tree was built for.
        built_for: Option<u64>,
    },
    Outliner {
        tree: NodeHandle,
    },
}

/// One panel, detached: its own widget tree and its own draw list.
///
/// Deliberately does **not** own the window or the surface, so it can be built
/// and driven in a test with no GPU and no event loop. That is what keeps the
/// rebuild logic, which is where the bugs are, testable.
pub struct FloatingPanel {
    kind: FloatingKind,
    /// The panel's own widget tree.
    pub ui: UserInterface,
    font_id: u8,
    /// The viewport the content scrolls inside.
    scroll: NodeHandle,
    content: Content,
    events: Vec<PanelEvent>,
}

impl FloatingPanel {
    /// Build the panel's tree at a logical size.
    ///
    /// `fonts` loads the same faces the editor uses. A second tree has a second
    /// atlas, and text in a floating window that fell back to a system face
    /// would be visibly not the editor's.
    pub fn new(
        kind: FloatingKind,
        logical: (f32, f32),
        fonts: impl FnOnce(&mut UserInterface) -> u8,
    ) -> Self {
        let mut ui = UserInterface::new(logical.0.max(1.0), logical.1.max(1.0));
        let font_id = fonts(&mut ui);
        let root = ui.root();

        let panel = BorderBuilder::new(
            WidgetBuilder::new()
                .with_background(theme::BG_PANEL)
                .with_foreground(theme::TRANSPARENT),
        )
        .with_stroke_thickness(Thickness::ZERO)
        .build();
        let panel = ui.add_node(panel, root);

        let scroll =
            ScrollViewerBuilder::new(WidgetBuilder::new().with_background(theme::BG_DARK)).build();
        let scroll = ui.add_node(scroll, panel);

        let content = match kind {
            FloatingKind::OutputLog => {
                let stack = StackPanelBuilder::new(
                    WidgetBuilder::new().with_background(theme::TRANSPARENT),
                )
                .with_orientation(Orientation::Vertical)
                .build();
                Content::Log {
                    stack: ui.add_node(stack, scroll),
                    rows: Vec::new(),
                    built_for: None,
                }
            }
            FloatingKind::Outliner => {
                let tree =
                    TreeViewBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
                        .with_font_id(font_id)
                        .build();
                Content::Outliner {
                    tree: ui.add_node(tree, scroll),
                }
            }
        };

        Self {
            kind,
            ui,
            font_id,
            scroll,
            content,
            events: Vec::new(),
        }
    }

    #[must_use]
    pub fn kind(&self) -> FloatingKind {
        self.kind
    }

    /// Feed the window's input to the panel's tree.
    ///
    /// Returns whether the tree consumed it. Everything is forwarded, pointer
    /// motion included, because a wheel event carries a position and the tree
    /// routes it to whatever sits under that position.
    pub fn on_event(&mut self, event: &winit::event::WindowEvent) -> bool {
        self.ui.process_os_event(event)
    }

    /// Lay the tree out for a new window size, in logical pixels.
    pub fn resize(&mut self, logical: (f32, f32)) {
        self.ui.resize(logical.0.max(1.0), logical.1.max(1.0));
    }

    /// Rebuild the content if the data has moved on.
    pub fn sync(&mut self, data: PanelData<'_>) {
        match (&mut self.content, data) {
            (
                Content::Log {
                    stack,
                    rows,
                    built_for,
                },
                PanelData::Log(log),
            ) => {
                // Keyed on the newest id, not the row count: a log at capacity
                // loses a line for every line it gains, so a length-keyed cache
                // would freeze on the first full buffer.
                let newest = log.entries().last().map(|entry| entry.id);
                if *built_for == newest && !rows.is_empty() {
                    return;
                }
                *built_for = newest;
                let stack = *stack;
                let old = std::mem::take(rows);
                let font_id = self.font_id;
                let style = text_style(TextRole::Body);
                let mut fresh = Vec::new();
                for entry in log.visible() {
                    let colour = match entry.severity {
                        LogSeverity::Error => theme::active().semantic.status.error.bytes(),
                        LogSeverity::Warn => theme::active().semantic.status.warning.bytes(),
                        _ => theme::TEXT_PRIMARY,
                    };
                    let row = TextBuilder::new(
                        WidgetBuilder::new().with_margin(Thickness::axes(8.0, 1.0)),
                    )
                    .with_text(&entry.rendered())
                    .with_font_id(font_id)
                    .with_font_size(style.px)
                    .with_color(colour)
                    .build();
                    fresh.push(self.ui.add_node(row, stack));
                }
                for row in old {
                    self.ui.remove_node(row);
                }
                if let Content::Log { rows, .. } = &mut self.content {
                    *rows = fresh;
                }
                self.ui.invalidate_ancestors(stack);
            }
            (Content::Outliner { tree }, PanelData::Outliner { items, selected }) => {
                // A `TreeView` is one widget that owns its rows, so this is a
                // message rather than a rebuild. The items come from the same
                // projection the docked panel uses, so the two cannot disagree
                // about what the scene contains.
                let tree = *tree;
                self.ui
                    .send(TreeViewMessage::set_items(tree, items.to_vec()));
                self.ui.send(UiMessage::new(
                    tree,
                    MessageDirection::ToWidget,
                    TreeViewMessage::SetSelected(selected),
                ));
            }
            // A panel handed the wrong data is a host bug, and a silent no-op
            // would present as a window that never updates.
            (content, _) => {
                tracing::warn!(
                    "floating {} was handed data for another panel",
                    match content {
                        Content::Log { .. } => "log",
                        Content::Outliner { .. } => "outliner",
                    }
                );
            }
        }
    }

    /// Pump, lay out and draw.
    ///
    /// The pump is not optional and its absence is not visible.
    /// [`crate::ui::UserInterface::process_os_event`] **queues** a `UiMessage`;
    /// nothing dispatches it until `update` runs. A panel that laid out and
    /// painted without pumping took every wheel event, put it on the queue and
    /// threw it away next frame, so the window looked right and would not
    /// scroll.
    ///
    /// Twice, for the reason the shell does it twice: a handler can send
    /// further messages, and a scroll that only invalidated layout on the
    /// following frame lags the pointer by one.
    pub fn draw(&mut self) {
        for _ in 0..2 {
            for message in self.ui.update() {
                if let Some(TreeViewMessage::Select(id)) = message.data::<TreeViewMessage>() {
                    self.events.push(PanelEvent::SelectEntity(*id));
                }
            }
        }
        self.ui.perform_layout();
        self.ui.draw();
    }

    /// Take what the user did in this window, for the editor to act on.
    pub fn take_events(&mut self) -> Vec<PanelEvent> {
        std::mem::take(&mut self.events)
    }

    /// The scroll viewport's handle, so a caller can address it directly.
    #[must_use]
    pub fn scroll_handle(&self) -> NodeHandle {
        self.scroll
    }

    /// Where the content sits, for a test that needs to see it move.
    #[must_use]
    pub fn content_origin(&self) -> f32 {
        let handle = match &self.content {
            Content::Log { stack, .. } => *stack,
            Content::Outliner { tree } => *tree,
        };
        self.ui.screen_bounds(handle).y
    }

    /// How many log rows the tree currently holds.
    #[must_use]
    pub fn row_count(&self) -> usize {
        match &self.content {
            Content::Log { rows, .. } => rows.len(),
            Content::Outliner { .. } => 0,
        }
    }

    /// The rectangle the tree was laid out for.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        self.ui.screen_bounds(self.ui.root())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icons::IconId;

    /// No faces: a tree with no font still has to build, because a floating
    /// window that panicked when the atlas was empty would take the editor with
    /// it.
    fn panel() -> FloatingPanel {
        FloatingPanel::new(FloatingKind::OutputLog, (900.0, 420.0), |_| 0)
    }

    fn outliner() -> FloatingPanel {
        FloatingPanel::new(FloatingKind::Outliner, (360.0, 720.0), |_| 0)
    }

    fn item(id: u32, label: &str) -> TreeItem {
        TreeItem {
            id,
            label: label.to_string(),
            depth: 0,
            icon: IconId::Cube,
            has_children: false,
            expanded: false,
            hidden: false,
            locked: false,
            script_error: false,
        }
    }

    #[test]
    fn a_detached_panel_builds_without_a_window_or_a_gpu() {
        // The reason this type does not own the surface. If it did, none of the
        // tests below could exist.
        let mut panel = panel();
        panel.draw();
        assert_eq!(panel.kind(), FloatingKind::OutputLog);
        assert_eq!(panel.row_count(), 0, "an empty log has no rows");
    }

    #[test]
    fn the_wheel_scrolls_the_rows() {
        // `process_os_event` queues a `UiMessage`, and nothing dispatches it
        // until `update` runs. A panel that laid out and painted without
        // pumping accepted every wheel event, queued it, and dropped it on the
        // next frame: the window looked correct and would not scroll, and a
        // screenshot could not tell the difference because the log kept growing
        // underneath.
        let mut log = OutputLog::default();
        for i in 0..200 {
            log.append(i as f64, &format!("line {i}"));
        }
        let mut panel = panel();
        panel.sync(PanelData::Log(&log));
        panel.draw();
        let top = panel.content_origin();

        panel.ui.send(UiMessage::new(
            panel.scroll_handle(),
            MessageDirection::ToWidget,
            crate::message::WidgetMessage::MouseWheel {
                pos: glam::Vec2::new(400.0, 200.0),
                delta: -600.0,
                mods: crate::message::Modifiers::default(),
            },
        ));
        panel.draw();

        assert!(
            panel.content_origin() < top,
            "the rows did not move: {top} then {}",
            panel.content_origin()
        );
    }

    #[test]
    fn a_log_shorter_than_the_window_does_not_scroll() {
        let mut log = OutputLog::default();
        log.append(0.0, "one");
        log.append(1.0, "two");
        let mut panel = panel();
        panel.sync(PanelData::Log(&log));
        panel.draw();
        let top = panel.content_origin();

        panel.ui.send(UiMessage::new(
            panel.scroll_handle(),
            MessageDirection::ToWidget,
            crate::message::WidgetMessage::MouseWheel {
                pos: glam::Vec2::new(400.0, 200.0),
                delta: -600.0,
                mods: crate::message::Modifiers::default(),
            },
        ));
        panel.draw();
        assert_eq!(panel.content_origin(), top);
    }

    #[test]
    fn rows_follow_the_log() {
        let mut log = OutputLog::default();
        for i in 0..5 {
            log.append(i as f64, &format!("line {i}"));
        }
        let mut panel = panel();
        panel.sync(PanelData::Log(&log));
        assert_eq!(panel.row_count(), 5);

        log.append(5.0, "one more");
        panel.sync(PanelData::Log(&log));
        assert_eq!(panel.row_count(), 6);
    }

    #[test]
    fn an_unchanged_log_does_not_rebuild_the_rows() {
        // A busy editor logs every frame, and a second window that rebuilt a
        // thousand rows to add one costs more than the editor beside it.
        let mut log = OutputLog::default();
        log.append(0.0, "hello");
        let mut panel = panel();
        panel.sync(PanelData::Log(&log));
        let Content::Log { rows, .. } = &panel.content else {
            panic!("built the wrong content")
        };
        let first = rows.clone();

        panel.sync(PanelData::Log(&log));
        let Content::Log { rows, .. } = &panel.content else {
            panic!("built the wrong content")
        };
        assert_eq!(*rows, first, "the same log rebuilt the same rows");
    }

    #[test]
    fn a_log_that_drops_from_the_front_still_rebuilds() {
        // Why the cache key is the newest id and not the row count.
        let mut log = OutputLog::with_capacity(4);
        for i in 0..4 {
            log.append(i as f64, &format!("line {i}"));
        }
        let mut panel = panel();
        panel.sync(PanelData::Log(&log));
        let before = panel.row_count();

        log.append(9.0, "pushes one out");
        panel.sync(PanelData::Log(&log));
        assert_eq!(panel.row_count(), before, "still full");
        let Content::Log { built_for, .. } = &panel.content else {
            panic!("built the wrong content")
        };
        assert_ne!(*built_for, Some(3), "but rebuilt for the newer entry");
    }

    #[test]
    fn a_floating_outliner_reports_the_row_that_was_clicked() {
        // The editor owns what selection means, so the window reports the click
        // and nothing more. Without this the panel is a picture of a tree.
        let mut panel = outliner();
        let items = [item(7, "Terrain"), item(9, "Camera")];
        panel.sync(PanelData::Outliner {
            items: &items,
            selected: None,
        });
        panel.draw();

        let Content::Outliner { tree } = panel.content else {
            panic!("built the wrong content")
        };
        panel.ui.send(UiMessage::new(
            tree,
            MessageDirection::FromWidget,
            TreeViewMessage::Select(9),
        ));
        panel.draw();

        assert_eq!(panel.take_events(), vec![PanelEvent::SelectEntity(9)]);
        assert!(
            panel.take_events().is_empty(),
            "taking the events clears them"
        );
    }

    #[test]
    fn every_floatable_panel_has_a_name_and_a_size() {
        // `ALL` is what the Window menu is written against, so a variant
        // missing from it is a panel nobody can float.
        assert_eq!(FloatingKind::ALL.len(), 2);
        for kind in FloatingKind::ALL {
            assert!(!kind.label().is_empty());
            assert!(kind.title().contains(kind.label()));
            let (w, h) = kind.default_size();
            assert!(w > 100 && h > 100, "{kind:?} opens at {w}x{h}");
        }
    }

    #[test]
    fn resizing_relays_out_rather_than_rebuilding() {
        let mut log = OutputLog::default();
        log.append(0.0, "hello");
        let mut panel = panel();
        panel.sync(PanelData::Log(&log));
        let rows = panel.row_count();

        panel.resize((400.0, 300.0));
        panel.draw();
        assert_eq!(
            panel.row_count(),
            rows,
            "a resize is a layout, not a rebuild"
        );
        let bounds = panel.bounds();
        assert!(bounds.w > 0.0 && bounds.h > 0.0, "{bounds:?}");
    }
}
