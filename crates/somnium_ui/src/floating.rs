//! A panel living in its own OS window.
//!
//! MORROWIND-J step 2. Step 1 built a dock tree that can express where a panel
//! sits; this is the case it cannot express, because the panel is not in the
//! window at all.
//!
//! # What a floating panel actually needs
//!
//! Not a widget. A second OS window is a second **surface**, and a surface is
//! the thing a [`crate::pass::UiPass`] renders into — so a floating panel owns
//! a whole parallel stack:
//!
//! ```text
//!   main window          floating window
//!   ───────────          ───────────────
//!   winit::Window        winit::Window        ← the host owns both
//!   wgpu::Surface        wgpu::Surface        ← and both configurations
//!   UserInterface        UserInterface        ← here
//!   UiPass               UiPass               ← here
//! ```
//!
//! The split matters: everything above the line is the host's (`somnium_core`
//! creates windows and surfaces; this crate has never known what a `Surface`
//! is), and everything below is a self-contained widget tree that happens not
//! to be the editor's.
//!
//! # Why the panel is rebuilt rather than moved
//!
//! A widget tree belongs to one [`crate::ui::UserInterface`]: handles are
//! indices into its pool, and a node cannot be re-parented across two of them.
//! So detaching a panel means *building it again* in the new tree from the same
//! data — which is only possible because the panel's content is a **store**
//! rather than a pile of widgets. [`crate::log::OutputLog`] is that store, and
//! it is the reason the Output Log is the panel that floats first.

use crate::{
    log::{LogSeverity, OutputLog},
    message::NodeHandle,
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
    },
};

/// Which panel a floating window is showing.
///
/// An enum rather than a `NodeHandle` because the window outlives any tree: it
/// is closed and reopened, and what it must remember across that is *which
/// panel it is*, not where the widgets were.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FloatingKind {
    /// The Output Log.
    OutputLog,
}

impl FloatingKind {
    /// The panel `SOMNIUM_FLOAT` asks to open at startup, if it asks for one.
    ///
    /// A window that only a menu can open is a window no automated run can
    /// look at, and this one has a GPU surface of its own — the part most worth
    /// exercising outside a human's hands.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        match std::env::var("SOMNIUM_FLOAT").ok()?.trim() {
            "log" => Some(Self::OutputLog),
            other => {
                tracing::warn!("SOMNIUM_FLOAT={other} is not a panel name; ignoring");
                None
            }
        }
    }

    /// The window title.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::OutputLog => "Output Log — Somnium",
        }
    }

    /// The size the window opens at, in logical pixels.
    #[must_use]
    pub const fn default_size(self) -> (u32, u32) {
        match self {
            // Wide and short: a log is read a line at a time, and the lines are
            // long.
            Self::OutputLog => (900, 420),
        }
    }
}

/// One panel, detached: its own widget tree and its own draw list.
///
/// Deliberately does **not** own the window or the surface. This type can be
/// built and driven in a test with no GPU and no event loop, which is what
/// keeps the rebuild logic — the part with the bugs — testable.
pub struct FloatingPanel {
    kind: FloatingKind,
    /// The panel's own widget tree.
    pub ui: UserInterface,
    font_id: u8,
    /// The viewport the rows scroll inside.
    scroll: NodeHandle,
    /// Where rows are appended.
    stack: NodeHandle,
    /// Rows currently built, so a refresh replaces rather than appends.
    rows: Vec<NodeHandle>,
    /// The log revision this tree was built for.
    ///
    /// A log grows every frame in a busy editor, and rebuilding a thousand rows
    /// per frame to add one is how a second window costs more than the editor.
    built_for: Option<u64>,
}

impl FloatingPanel {
    /// Build the panel's tree at a logical size.
    ///
    /// `fonts` loads the same faces the editor uses; a second tree has a second
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
        let stack =
            StackPanelBuilder::new(WidgetBuilder::new().with_background(theme::TRANSPARENT))
                .with_orientation(Orientation::Vertical)
                .build();
        let stack = ui.add_node(stack, scroll);

        Self {
            kind,
            ui,
            font_id,
            scroll,
            stack,
            rows: Vec::new(),
            built_for: None,
        }
    }

    #[must_use]
    pub fn kind(&self) -> FloatingKind {
        self.kind
    }

    /// Feed the window's input to the panel's tree.
    ///
    /// Returns whether the tree consumed it. Without this the panel is a
    /// picture: the scroll viewer never sees a wheel event, so a log longer
    /// than the window is a log you can only read the top of.
    ///
    /// Everything is forwarded, including the pointer motion. A wheel event
    /// carries a position, and the tree routes it to whatever is under that
    /// position, so a window that forwarded only wheels would scroll whichever
    /// widget the pointer was last over in some other window.
    pub fn on_event(&mut self, event: &winit::event::WindowEvent) -> bool {
        self.ui.process_os_event(event)
    }

    /// Lay the tree out for a new window size, in logical pixels.
    pub fn resize(&mut self, logical: (f32, f32)) {
        self.ui.resize(logical.0.max(1.0), logical.1.max(1.0));
    }

    /// Rebuild the rows if the log has moved on.
    ///
    /// Keyed on the log's newest id rather than on its length: a log at
    /// capacity drops from the front as it gains at the back, so the length
    /// stops changing long before the content does.
    pub fn sync(&mut self, log: &OutputLog) {
        let newest = log.entries().last().map(|entry| entry.id);
        if self.built_for == newest && !self.rows.is_empty() {
            return;
        }
        self.built_for = newest;

        for row in std::mem::take(&mut self.rows) {
            self.ui.remove_node(row);
        }
        let style = text_style(TextRole::Body);
        for entry in log.visible() {
            let colour = match entry.severity {
                LogSeverity::Error => theme::active().semantic.status.error.bytes(),
                LogSeverity::Warn => theme::active().semantic.status.warning.bytes(),
                _ => theme::TEXT_PRIMARY,
            };
            let row = TextBuilder::new(WidgetBuilder::new().with_margin(Thickness::axes(8.0, 1.0)))
                .with_text(&entry.rendered())
                .with_font_id(self.font_id)
                .with_font_size(style.px)
                .with_color(colour)
                .build();
            self.rows.push(self.ui.add_node(row, self.stack));
        }
        self.ui.invalidate_ancestors(self.stack);
    }

    /// How many rows the tree currently holds.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Pump, lay out and draw.
    ///
    /// The pump is not optional and its absence is not visible.
    /// [`crate::ui::UserInterface::process_os_event`] **queues** a `UiMessage`;
    /// nothing dispatches it until `update` runs. A panel that laid out and
    /// painted without pumping took every wheel event, put it on the queue, and
    /// threw it away next frame, so the window looked right and would not
    /// scroll.
    ///
    /// Twice, for the reason the shell does it twice: a handler can send
    /// further messages, and a scroll that only invalidated layout on the
    /// following frame lags the pointer by one.
    pub fn draw(&mut self) {
        let _ = self.ui.update();
        let _ = self.ui.update();
        self.ui.perform_layout();
        self.ui.draw();
    }

    /// Where the rows sit, for a test that needs to see them move.
    #[must_use]
    pub fn rows_origin(&self) -> f32 {
        self.ui.screen_bounds(self.stack).y
    }

    /// The scroll viewport's handle, so a caller can address it directly.
    #[must_use]
    pub fn scroll_handle(&self) -> NodeHandle {
        self.scroll
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

    /// No faces: a tree with no font still has to build, because a floating
    /// window that panicked when the atlas was empty would take the editor with
    /// it.
    fn panel() -> FloatingPanel {
        FloatingPanel::new(FloatingKind::OutputLog, (900.0, 420.0), |_| 0)
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
        // The bug this pins: `process_os_event` *queues* a `UiMessage`, and
        // nothing dispatches it until `update` runs. A panel that laid out and
        // painted without pumping accepted every wheel event, put it on the
        // queue, and dropped it on the next frame. The window looked correct
        // and would not scroll, and a screenshot could not tell the difference
        // because the log kept growing under it.
        let mut log = OutputLog::default();
        for i in 0..200 {
            log.append(i as f64, &format!("line {i}"));
        }
        let mut panel = panel();
        panel.sync(&log);
        panel.draw();
        let top = panel.rows_origin();

        panel.ui.send(crate::message::UiMessage::new(
            panel.scroll_handle(),
            crate::message::MessageDirection::ToWidget,
            crate::message::WidgetMessage::MouseWheel {
                pos: glam::Vec2::new(400.0, 200.0),
                // Negative is downward: the viewer subtracts the delta.
                delta: -600.0,
                mods: crate::message::Modifiers::default(),
            },
        ));
        panel.draw();

        assert!(
            panel.rows_origin() < top,
            "the rows did not move: {} then {}",
            top,
            panel.rows_origin()
        );
    }

    #[test]
    fn a_log_shorter_than_the_window_does_not_scroll() {
        // The other half. A scroll viewer clamps to its content, so two lines
        // in a 420 px window stay put however hard the wheel is turned.
        let mut log = OutputLog::default();
        log.append(0.0, "one");
        log.append(1.0, "two");
        let mut panel = panel();
        panel.sync(&log);
        panel.draw();
        let top = panel.rows_origin();

        panel.ui.send(crate::message::UiMessage::new(
            panel.scroll_handle(),
            crate::message::MessageDirection::ToWidget,
            crate::message::WidgetMessage::MouseWheel {
                pos: glam::Vec2::new(400.0, 200.0),
                delta: -600.0,
                mods: crate::message::Modifiers::default(),
            },
        ));
        panel.draw();

        assert_eq!(panel.rows_origin(), top);
    }

    #[test]
    fn rows_follow_the_log() {
        let mut log = OutputLog::default();
        for i in 0..5 {
            log.append(i as f64, &format!("line {i}"));
        }
        let mut panel = panel();
        panel.sync(&log);
        assert_eq!(panel.row_count(), 5);

        log.append(5.0, "one more");
        panel.sync(&log);
        assert_eq!(panel.row_count(), 6);
    }

    #[test]
    fn an_unchanged_log_does_not_rebuild_the_rows() {
        // A busy editor logs every frame, and a second window that rebuilt a
        // thousand rows to add one costs more than the editor it is beside.
        let mut log = OutputLog::default();
        log.append(0.0, "hello");
        let mut panel = panel();
        panel.sync(&log);
        let first = panel.rows.clone();

        panel.sync(&log);
        assert_eq!(panel.rows, first, "the same log rebuilt the same rows");
    }

    #[test]
    fn a_log_that_drops_from_the_front_still_rebuilds() {
        // The reason the cache key is the newest id and not the row count. A
        // log at capacity loses a line for every line it gains, so a
        // length-keyed cache would freeze the window on the first full buffer.
        let mut log = OutputLog::with_capacity(4);
        for i in 0..4 {
            log.append(i as f64, &format!("line {i}"));
        }
        let mut panel = panel();
        panel.sync(&log);
        let before = panel.row_count();

        log.append(9.0, "pushes one out");
        panel.sync(&log);
        assert_eq!(panel.row_count(), before, "still full");
        assert_ne!(panel.built_for, Some(3), "but rebuilt for the newer entry");
    }

    #[test]
    fn resizing_relays_out_rather_than_rebuilding() {
        let mut log = OutputLog::default();
        log.append(0.0, "hello");
        let mut panel = panel();
        panel.sync(&log);
        let rows = panel.rows.clone();

        panel.resize((400.0, 300.0));
        panel.draw();
        assert_eq!(panel.rows, rows, "a resize is a layout, not a rebuild");
        let bounds = panel.bounds();
        assert!(bounds.w > 0.0 && bounds.h > 0.0, "{bounds:?}");
    }
}
