// Splitter: two-pane resizable container (Phase 26-A).
// Horizontal = left | right. Vertical = top / bottom.

use crate::{
    draw::DrawingContext,
    message::{MessageDirection, MouseButton, UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    theme,
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitterOrientation {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone)]
pub enum SplitterMessage {
    /// First pane size in pixels after a drag.
    Changed(f32),
    /// Apply a persisted width/height without emitting Changed.
    SetFirstSize(f32),
}

pub struct Splitter {
    pub orientation: SplitterOrientation,
    pub first_size: f32,
    pub min_first: f32,
    pub min_second: f32,
    dragging: bool,
    drag_origin: Option<(f32, f32)>,
}

impl Control for Splitter {
    fn measure_override(&self, widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let bar = theme::SPLITTER_THICKNESS;
        match self.orientation {
            SplitterOrientation::Horizontal => {
                let first = self.first_size.clamp(
                    self.min_first,
                    (available.x - bar - self.min_second).max(self.min_first),
                );
                let second = (available.x - bar - first).max(self.min_second);
                if let Some(&a) = widget.children.first() {
                    ctx.measure_child(a, Vec2::new(first, available.y));
                }
                if let Some(&b) = widget.children.get(1) {
                    ctx.measure_child(b, Vec2::new(second, available.y));
                }
            }
            SplitterOrientation::Vertical => {
                let first = self.first_size.clamp(
                    self.min_first,
                    (available.y - bar - self.min_second).max(self.min_first),
                );
                let second = (available.y - bar - first).max(self.min_second);
                if let Some(&a) = widget.children.first() {
                    ctx.measure_child(a, Vec2::new(available.x, first));
                }
                if let Some(&b) = widget.children.get(1) {
                    ctx.measure_child(b, Vec2::new(available.x, second));
                }
            }
        }
        available
    }

    fn arrange_override(&self, widget: &Widget, ctx: &mut LayoutCtx, final_size: Vec2) -> Vec2 {
        let bar = theme::SPLITTER_THICKNESS;
        let ox = widget.actual_local_position.x;
        let oy = widget.actual_local_position.y;
        match self.orientation {
            SplitterOrientation::Horizontal => {
                let first = self.first_size.clamp(
                    self.min_first,
                    (final_size.x - bar - self.min_second).max(self.min_first),
                );
                if let Some(&a) = widget.children.first() {
                    ctx.arrange_child(a, Rect::new(ox, oy, first, final_size.y));
                }
                if let Some(&b) = widget.children.get(1) {
                    ctx.arrange_child(
                        b,
                        Rect::new(
                            ox + first + bar,
                            oy,
                            (final_size.x - first - bar).max(0.0),
                            final_size.y,
                        ),
                    );
                }
            }
            SplitterOrientation::Vertical => {
                let first = self.first_size.clamp(
                    self.min_first,
                    (final_size.y - bar - self.min_second).max(self.min_first),
                );
                if let Some(&a) = widget.children.first() {
                    ctx.arrange_child(a, Rect::new(ox, oy, final_size.x, first));
                }
                if let Some(&b) = widget.children.get(1) {
                    ctx.arrange_child(
                        b,
                        Rect::new(
                            ox,
                            oy + first + bar,
                            final_size.x,
                            (final_size.y - first - bar).max(0.0),
                        ),
                    );
                }
            }
        }
        final_size
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let bar = theme::SPLITTER_THICKNESS;
        let rect = match self.orientation {
            SplitterOrientation::Horizontal => {
                let first = self.first_size.clamp(
                    self.min_first,
                    (b.w - bar - self.min_second).max(self.min_first),
                );
                Rect::new(b.x + first, b.y, bar, b.h)
            }
            SplitterOrientation::Vertical => {
                let first = self.first_size.clamp(
                    self.min_first,
                    (b.h - bar - self.min_second).max(self.min_first),
                );
                Rect::new(b.x, b.y + first, b.w, bar)
            }
        };
        ctx.push_rect_filled(rect, theme::BORDER_MEDIUM);
    }

    fn cursor_icon(&self, widget: &Widget, pos: Vec2) -> crate::node::CursorKind {
        if self.dragging || self.hit_rect(widget).contains(pos) {
            match self.orientation {
                SplitterOrientation::Horizontal => crate::node::CursorKind::ColResize,
                SplitterOrientation::Vertical => crate::node::CursorKind::RowResize,
            }
        } else {
            crate::node::CursorKind::Default
        }
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if let Some(SplitterMessage::SetFirstSize(v)) = msg.data::<SplitterMessage>() {
            self.first_size = *v;
            widget.invalidate_layout();
            msg.handled = true;
            return;
        }
        let Some(wmsg) = msg.data::<WidgetMessage>() else {
            return;
        };
        match wmsg {
            WidgetMessage::MouseDown { pos, button } if *button == MouseButton::Left => {
                if self.hit_rect(widget).contains(*pos) {
                    self.dragging = true;
                    self.drag_origin = Some((
                        match self.orientation {
                            SplitterOrientation::Horizontal => pos.x,
                            SplitterOrientation::Vertical => pos.y,
                        },
                        self.first_size,
                    ));
                    msg.handled = true;
                }
            }
            WidgetMessage::MouseMove { pos } if self.dragging => {
                if let Some((origin, start)) = self.drag_origin {
                    let delta = match self.orientation {
                        SplitterOrientation::Horizontal => pos.x - origin,
                        SplitterOrientation::Vertical => pos.y - origin,
                    };
                    self.first_size = (start + delta).max(self.min_first);
                    widget.invalidate_layout();
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        SplitterMessage::Changed(self.first_size),
                    ));
                    msg.handled = true;
                }
            }
            WidgetMessage::MouseUp { .. } => {
                self.dragging = false;
                self.drag_origin = None;
            }
            _ => {}
        }
    }
}

impl Splitter {
    fn hit_rect(&self, widget: &Widget) -> Rect {
        let r = self.bar_rect(widget);
        const SLOP: f32 = 4.0;
        match self.orientation {
            SplitterOrientation::Horizontal => Rect::new(r.x - SLOP, r.y, r.w + SLOP * 2.0, r.h),
            SplitterOrientation::Vertical => Rect::new(r.x, r.y - SLOP, r.w, r.h + SLOP * 2.0),
        }
    }

    fn bar_rect(&self, widget: &Widget) -> Rect {
        let b = widget.screen_bounds();
        let bar = theme::SPLITTER_THICKNESS;
        match self.orientation {
            SplitterOrientation::Horizontal => {
                let first = self.first_size.clamp(
                    self.min_first,
                    (b.w - bar - self.min_second).max(self.min_first),
                );
                Rect::new(b.x + first, b.y, bar, b.h)
            }
            SplitterOrientation::Vertical => {
                let first = self.first_size.clamp(
                    self.min_first,
                    (b.h - bar - self.min_second).max(self.min_first),
                );
                Rect::new(b.x, b.y + first, b.w, bar)
            }
        }
    }
}

pub struct SplitterBuilder {
    widget: WidgetBuilder,
    orientation: SplitterOrientation,
    first_size: f32,
    min_first: f32,
    min_second: f32,
}

impl SplitterBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            orientation: SplitterOrientation::Horizontal,
            first_size: 200.0,
            min_first: 40.0,
            min_second: 80.0,
        }
    }

    pub fn with_orientation(mut self, o: SplitterOrientation) -> Self {
        self.orientation = o;
        self
    }
    pub fn with_first_size(mut self, s: f32) -> Self {
        self.first_size = s;
        self
    }
    pub fn with_min_first(mut self, s: f32) -> Self {
        self.min_first = s;
        self
    }
    pub fn with_min_second(mut self, s: f32) -> Self {
        self.min_second = s;
        self
    }

    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(Splitter {
                orientation: self.orientation,
                first_size: self.first_size,
                min_first: self.min_first,
                min_second: self.min_second,
                dragging: false,
                drag_origin: None,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_first_size_is_clamped_by_mins() {
        let s = Splitter {
            orientation: SplitterOrientation::Horizontal,
            first_size: 10.0,
            min_first: 40.0,
            min_second: 80.0,
            dragging: false,
            drag_origin: None,
        };
        assert_eq!(s.min_first, 40.0);
        assert!(s.first_size < s.min_first);
    }
}
