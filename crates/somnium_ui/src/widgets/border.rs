// Port of: example_repo/fyrox/Fyrox-master/fyrox-ui/src/border.rs
// Border: background fill + per-side stroke outline, shrinks inner rect by stroke thickness.

use crate::{
    draw::DrawingContext,
    message::UiMessage,
    node::{Control, LayoutCtx, UiNode},
    types::{Rect, Thickness},
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

/// Explicit surface treatment; layout and per-side insets remain unchanged.
#[derive(Clone, Copy, Default)]
pub enum Surface {
    #[default]
    Flat,
    Popup,
    Modal,
}

pub struct Border {
    surface: Surface,
    pub stroke_thickness: Thickness,
}

impl Control for Border {
    fn measure_override(&self, widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let st = self.stroke_thickness;
        let inner = Vec2::new(
            (available.x - st.h()).max(0.0),
            (available.y - st.v()).max(0.0),
        );
        let mut desired = Vec2::ZERO;
        for &ch in &widget.children {
            ctx.measure_child(ch, inner);
            let ds = ctx.desired_size(ch);
            if ds.x > desired.x {
                desired.x = ds.x;
            }
            if ds.y > desired.y {
                desired.y = ds.y;
            }
        }
        desired + Vec2::new(st.h(), st.v())
    }

    fn arrange_override(&self, widget: &Widget, ctx: &mut LayoutCtx, final_size: Vec2) -> Vec2 {
        let st = self.stroke_thickness;
        let ox = widget.actual_local_position.x;
        let oy = widget.actual_local_position.y;
        let rect = Rect::new(
            ox + st.left,
            oy + st.top,
            (final_size.x - st.h()).max(0.0),
            (final_size.y - st.v()).max(0.0),
        );
        for &ch in &widget.children {
            ctx.arrange_child(ch, rect);
        }
        final_size
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        if !matches!(self.surface, Surface::Flat) {
            let t = crate::theme::active();
            let mut paint = crate::style::popup();
            if matches!(self.surface, Surface::Modal) {
                paint.radius = t.geometry.radius_modal;
                paint.background = t.semantic.surface.panel.bytes();
                paint.elevation = Some(t.elevation.modal);
            } else {
                paint.elevation = Some(t.elevation.popup);
            }
            ctx.push_paint(b, &paint);
            return;
        }
        let st = self.stroke_thickness;
        ctx.push_rect_filled(b, widget.background);
        // Per-side stroke (drawn on top)
        let fg = widget.foreground;
        ctx.push_rect_filled(Rect::new(b.x, b.y, b.w, st.top), fg);
        ctx.push_rect_filled(Rect::new(b.x, b.y + b.h - st.bottom, b.w, st.bottom), fg);
        ctx.push_rect_filled(
            Rect::new(
                b.x,
                b.y + st.top,
                st.left,
                (b.h - st.top - st.bottom).max(0.0),
            ),
            fg,
        );
        ctx.push_rect_filled(
            Rect::new(
                b.x + b.w - st.right,
                b.y + st.top,
                st.right,
                (b.h - st.top - st.bottom).max(0.0),
            ),
            fg,
        );
    }

    fn handle_routed_message(
        &mut self,
        _widget: &mut Widget,
        _msg: &mut UiMessage,
        _emit: &mut Vec<UiMessage>,
    ) {
    }
}

pub struct BorderBuilder {
    surface: Surface,
    widget: WidgetBuilder,
    stroke_thickness: Thickness,
}

impl BorderBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            surface: Surface::Flat,
            stroke_thickness: Thickness::uniform(1.0),
        }
    }

    pub fn with_surface(mut self, surface: Surface) -> Self {
        self.surface = surface;
        self.widget = self.widget.with_clip_to_bounds(false);
        self
    }

    pub fn with_stroke_thickness(mut self, t: Thickness) -> Self {
        self.stroke_thickness = t;
        self
    }

    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(Border {
                surface: self.surface,
                stroke_thickness: self.stroke_thickness,
            }),
        )
    }
}
