//! ColorSwatch + ColorPicker (Phase 26-F Iris).
//!
//! Linear RGB(A) storage. The swatch and spectrum use the standard sRGB transfer.
//! Cancel restores the colour captured when the picker opened.

use crate::{
    color::{hex_to_linear, hsv_to_linear, linear_rgba_to_srgb_u8, linear_to_hex, linear_to_hsv},
    draw::DrawingContext,
    message::{KeyCode, MessageDirection, UiMessage, WidgetMessage},
    node::{Control, CursorKind, LayoutCtx, UiNode},
    theme,
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

#[derive(Debug, Clone, PartialEq)]
pub enum ColorSwatchMessage {
    Clicked([f32; 4]),
    SetColor([f32; 4]),
    SetLocked(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ColorPickerMessage {
    Changing([f32; 4]),
    Changed([f32; 4]),
    Cancelled([f32; 4]),
    SetColor([f32; 4]),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DragPart {
    None,
    Sv,
    Hue,
}

pub struct ColorSwatch {
    pub color: [f32; 4],
    pub locked: bool,
}

impl Control for ColorSwatch {
    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, _available: Vec2) -> Vec2 {
        Vec2::new(36.0, theme::ROW_HEIGHT)
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let sw = Rect::new(b.x + 4.0, b.y + 3.0, 28.0, (b.h - 6.0).max(12.0));
        // Checker so alpha is visible.
        ctx.push_rect_filled(sw, theme::BG_RAISED);
        let display = linear_rgba_to_srgb_u8(self.color);
        ctx.push_rect_filled(sw, display);
        ctx.push_rect_border(sw, 1.0, theme::BORDER_MEDIUM);
        if self.locked {
            ctx.push_rect_filled(Rect::new(sw.x, sw.y, sw.w, 3.0), theme::STATUS_WARN);
        }
    }

    fn cursor_icon(&self, _widget: &Widget, _pos: Vec2) -> CursorKind {
        if self.locked {
            CursorKind::Default
        } else {
            CursorKind::Pointer
        }
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if let Some(m) = msg.data::<ColorSwatchMessage>() {
            match m {
                ColorSwatchMessage::SetColor(c) => {
                    self.color = *c;
                    msg.handled = true;
                }
                ColorSwatchMessage::SetLocked(v) => {
                    self.locked = *v;
                    msg.handled = true;
                }
                ColorSwatchMessage::Clicked(_) => {}
            }
            return;
        }
        if let Some(WidgetMessage::MouseDown { .. }) = msg.data::<WidgetMessage>() {
            if !self.locked {
                emit.push(UiMessage::new(
                    widget.handle,
                    MessageDirection::FromWidget,
                    ColorSwatchMessage::Clicked(self.color),
                ));
                msg.handled = true;
            }
        }
    }
}

pub struct ColorSwatchBuilder {
    widget: WidgetBuilder,
    color: [f32; 4],
    locked: bool,
}

impl ColorSwatchBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            color: [1.0, 1.0, 1.0, 1.0],
            locked: false,
        }
    }
    pub fn with_color(mut self, c: [f32; 4]) -> Self {
        self.color = c;
        self
    }
    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(ColorSwatch {
                color: self.color,
                locked: self.locked,
            }),
        )
    }
}

pub struct ColorPicker {
    linear: [f32; 4],
    original: [f32; 4],
    hsv: (f32, f32, f32),
    hex: String,
    hex_focus: bool,
    show_alpha: bool,
    drag: DragPart,
    recent: [[f32; 4]; 8],
    recent_count: usize,
    font_id: u8,
}

impl ColorPicker {
    fn set_linear(&mut self, rgba: [f32; 4], sync_hex: bool) {
        self.linear = rgba;
        self.hsv = linear_to_hsv([rgba[0], rgba[1], rgba[2]]);
        if sync_hex {
            self.hex = linear_to_hex([rgba[0], rgba[1], rgba[2]]);
        }
    }

    fn apply_hsv(&mut self) {
        let rgb = hsv_to_linear(self.hsv.0, self.hsv.1, self.hsv.2);
        self.linear[0] = rgb[0];
        self.linear[1] = rgb[1];
        self.linear[2] = rgb[2];
        self.hex = linear_to_hex(rgb);
    }

    fn sv_rect(b: Rect) -> Rect {
        Rect::new(b.x + 10.0, b.y + 10.0, 220.0, 140.0)
    }
    fn hue_rect(b: Rect) -> Rect {
        Rect::new(b.x + 10.0, b.y + 156.0, 220.0, 14.0)
    }
    fn cancel_rect(b: Rect) -> Rect {
        Rect::new(b.x + 10.0, b.y + b.h - 28.0, 64.0, 20.0)
    }
}

impl Control for ColorPicker {
    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, _available: Vec2) -> Vec2 {
        Vec2::new(240.0, if self.show_alpha { 292.0 } else { 268.0 })
    }

    fn is_text_input(&self) -> bool {
        self.hex_focus
    }

    fn cursor_icon(&self, widget: &Widget, pos: Vec2) -> CursorKind {
        let b = widget.screen_bounds();
        if Self::hue_rect(b).contains(pos) {
            CursorKind::EwResize
        } else if Self::sv_rect(b).contains(pos) || Self::cancel_rect(b).contains(pos) {
            CursorKind::Pointer
        } else {
            CursorKind::Default
        }
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        ctx.push_rect_filled(b, theme::BG_HEADER);
        ctx.push_rect_border(b, 1.0, theme::BORDER_DARK);

        let sv = Self::sv_rect(b);
        // SV square: sample a coarse grid in the current hue.
        const STEPS: i32 = 16;
        let cell_w = sv.w / STEPS as f32;
        let cell_h = sv.h / STEPS as f32;
        for y in 0..STEPS {
            for x in 0..STEPS {
                let s = (x as f32 + 0.5) / STEPS as f32;
                let v = 1.0 - (y as f32 + 0.5) / STEPS as f32;
                let rgb = hsv_to_linear(self.hsv.0, s, v);
                let c = linear_rgba_to_srgb_u8([rgb[0], rgb[1], rgb[2], 1.0]);
                ctx.push_rect_filled(
                    Rect::new(
                        sv.x + x as f32 * cell_w,
                        sv.y + y as f32 * cell_h,
                        cell_w + 0.5,
                        cell_h + 0.5,
                    ),
                    c,
                );
            }
        }
        ctx.push_rect_border(sv, 1.0, theme::BORDER_LIGHT);
        let cx = sv.x + self.hsv.1 * sv.w;
        let cy = sv.y + (1.0 - self.hsv.2) * sv.h;
        ctx.push_rect_border(Rect::new(cx - 4.0, cy - 4.0, 8.0, 8.0), 1.0, theme::WHITE);

        let hue = Self::hue_rect(b);
        for i in 0..24 {
            let h = i as f32 / 24.0 * 360.0;
            let rgb = hsv_to_linear(h, 1.0, 1.0);
            let c = linear_rgba_to_srgb_u8([rgb[0], rgb[1], rgb[2], 1.0]);
            let x = hue.x + i as f32 * hue.w / 24.0;
            ctx.push_rect_filled(Rect::new(x, hue.y, hue.w / 24.0 + 0.5, hue.h), c);
        }
        let hx = hue.x + (self.hsv.0 / 360.0).clamp(0.0, 1.0) * hue.w;
        ctx.push_rect_filled(
            Rect::new(hx - 1.0, hue.y - 1.0, 3.0, hue.h + 2.0),
            theme::WHITE,
        );

        let preview = Rect::new(b.x + 10.0, b.y + 178.0, 36.0, 22.0);
        ctx.push_rect_filled(preview, linear_rgba_to_srgb_u8(self.linear));
        ctx.push_rect_border(preview, 1.0, theme::BORDER_MEDIUM);

        ctx.push_text(
            &self.hex,
            Vec2::new(b.x + 52.0, b.y + 182.0),
            self.font_id,
            12.0,
            theme::TEXT_PRIMARY,
        );
        let rgb8 = linear_rgba_to_srgb_u8(self.linear);
        ctx.push_text(
            &format!("R {}  G {}  B {}", rgb8[0], rgb8[1], rgb8[2]),
            Vec2::new(b.x + 10.0, b.y + 206.0),
            self.font_id,
            11.0,
            theme::TEXT_SECONDARY,
        );

        for i in 0..self.recent_count.min(8) {
            let r = Rect::new(b.x + 10.0 + i as f32 * 22.0, b.y + 226.0, 18.0, 18.0);
            ctx.push_rect_filled(r, linear_rgba_to_srgb_u8(self.recent[i]));
            ctx.push_rect_border(r, 1.0, theme::BORDER_DARK);
        }

        let cancel = Self::cancel_rect(b);
        ctx.push_rect_filled(cancel, theme::BG_RAISED);
        ctx.push_rect_border(cancel, 1.0, theme::BORDER_MEDIUM);
        ctx.push_text(
            "Cancel",
            Vec2::new(cancel.x + 10.0, cancel.y + 4.0),
            self.font_id,
            11.0,
            theme::TEXT_PRIMARY,
        );
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if let Some(ColorPickerMessage::SetColor(c)) = msg.data::<ColorPickerMessage>() {
            self.original = *c;
            self.set_linear(*c, true);
            self.hex_focus = false;
            msg.handled = true;
            return;
        }

        let Some(wmsg) = msg.data::<WidgetMessage>() else {
            return;
        };
        let b = widget.screen_bounds();
        match wmsg.clone() {
            WidgetMessage::MouseDown { pos, .. } => {
                if Self::cancel_rect(b).contains(pos) {
                    self.set_linear(self.original, true);
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        ColorPickerMessage::Cancelled(self.original),
                    ));
                    msg.handled = true;
                    return;
                }
                for i in 0..self.recent_count.min(8) {
                    let r = Rect::new(b.x + 10.0 + i as f32 * 22.0, b.y + 226.0, 18.0, 18.0);
                    if r.contains(pos) {
                        self.set_linear(self.recent[i], true);
                        emit.push(UiMessage::new(
                            widget.handle,
                            MessageDirection::FromWidget,
                            ColorPickerMessage::Changing(self.linear),
                        ));
                        msg.handled = true;
                        return;
                    }
                }
                let hex_r = Rect::new(b.x + 52.0, b.y + 178.0, 170.0, 22.0);
                if hex_r.contains(pos) {
                    self.hex_focus = true;
                    msg.handled = true;
                    return;
                }
                self.hex_focus = false;
                if Self::sv_rect(b).contains(pos) {
                    self.drag = DragPart::Sv;
                    self.pick_sv(pos, b, emit, widget);
                    msg.handled = true;
                } else if Self::hue_rect(b).contains(pos) {
                    self.drag = DragPart::Hue;
                    self.pick_hue(pos, b, emit, widget);
                    msg.handled = true;
                }
            }
            WidgetMessage::MouseMove { pos } => match self.drag {
                DragPart::Sv => {
                    self.pick_sv(pos, b, emit, widget);
                    msg.handled = true;
                }
                DragPart::Hue => {
                    self.pick_hue(pos, b, emit, widget);
                    msg.handled = true;
                }
                DragPart::None => {}
            },
            WidgetMessage::MouseUp { .. } => {
                if self.drag != DragPart::None {
                    self.drag = DragPart::None;
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        ColorPickerMessage::Changed(self.linear),
                    ));
                    msg.handled = true;
                }
            }
            WidgetMessage::Text(s) if self.hex_focus => {
                for ch in s.chars() {
                    if ch.is_ascii_hexdigit() || ch == '#' {
                        if self.hex.len() < 7 {
                            self.hex.push(ch.to_ascii_uppercase());
                        }
                    }
                }
                if let Some(rgb) = hex_to_linear(&self.hex) {
                    self.linear[0] = rgb[0];
                    self.linear[1] = rgb[1];
                    self.linear[2] = rgb[2];
                    self.hsv = linear_to_hsv(rgb);
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        ColorPickerMessage::Changing(self.linear),
                    ));
                }
                msg.handled = true;
            }
            WidgetMessage::KeyDown(KeyCode::Backspace) if self.hex_focus => {
                self.hex.pop();
                msg.handled = true;
            }
            WidgetMessage::KeyDown(KeyCode::Enter | KeyCode::NumpadEnter) if self.hex_focus => {
                self.hex_focus = false;
                emit.push(UiMessage::new(
                    widget.handle,
                    MessageDirection::FromWidget,
                    ColorPickerMessage::Changed(self.linear),
                ));
                msg.handled = true;
            }
            _ => {}
        }
    }
}

impl ColorPicker {
    fn pick_sv(&mut self, pos: Vec2, b: Rect, emit: &mut Vec<UiMessage>, widget: &Widget) {
        let sv = Self::sv_rect(b);
        self.hsv.1 = ((pos.x - sv.x) / sv.w).clamp(0.0, 1.0);
        self.hsv.2 = (1.0 - (pos.y - sv.y) / sv.h).clamp(0.0, 1.0);
        self.apply_hsv();
        emit.push(UiMessage::new(
            widget.handle,
            MessageDirection::FromWidget,
            ColorPickerMessage::Changing(self.linear),
        ));
    }

    fn pick_hue(&mut self, pos: Vec2, b: Rect, emit: &mut Vec<UiMessage>, widget: &Widget) {
        let hue = Self::hue_rect(b);
        self.hsv.0 = ((pos.x - hue.x) / hue.w).clamp(0.0, 1.0) * 360.0;
        self.apply_hsv();
        emit.push(UiMessage::new(
            widget.handle,
            MessageDirection::FromWidget,
            ColorPickerMessage::Changing(self.linear),
        ));
    }

    pub fn push_recent(&mut self, c: [f32; 4]) {
        if self.recent.iter().take(self.recent_count).any(|r| {
            (r[0] - c[0]).abs() < 1e-3 && (r[1] - c[1]).abs() < 1e-3 && (r[2] - c[2]).abs() < 1e-3
        }) {
            return;
        }
        self.recent.rotate_right(1);
        self.recent[0] = c;
        self.recent_count = (self.recent_count + 1).min(8);
    }
}

pub struct ColorPickerBuilder {
    widget: WidgetBuilder,
    font_id: u8,
    show_alpha: bool,
}

impl ColorPickerBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            font_id: 0,
            show_alpha: false,
        }
    }
    pub fn with_font_id(mut self, id: u8) -> Self {
        self.font_id = id;
        self
    }
    pub fn with_alpha(mut self, on: bool) -> Self {
        self.show_alpha = on;
        self
    }
    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(ColorPicker {
                linear: [1.0, 1.0, 1.0, 1.0],
                original: [1.0, 1.0, 1.0, 1.0],
                hsv: (0.0, 0.0, 1.0),
                hex: "#FFFFFF".into(),
                hex_focus: false,
                show_alpha: self.show_alpha,
                drag: DragPart::None,
                recent: [[0.0; 4]; 8],
                recent_count: 0,
                font_id: self.font_id,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_restores_the_colour_captured_at_open() {
        let mut p = ColorPicker {
            linear: [0.2, 0.4, 0.6, 1.0],
            original: [1.0, 0.0, 0.0, 1.0],
            hsv: (0.0, 0.0, 1.0),
            hex: String::new(),
            hex_focus: false,
            show_alpha: false,
            drag: DragPart::None,
            recent: [[0.0; 4]; 8],
            recent_count: 0,
            font_id: 0,
        };
        p.set_linear(p.original, true);
        assert!((p.linear[0] - 1.0).abs() < 1e-5);
        assert!(p.linear[1].abs() < 1e-5);
    }

    #[test]
    fn srgb_display_of_linear_mid_grey_is_not_black() {
        let u = linear_rgba_to_srgb_u8([0.18, 0.18, 0.18, 1.0]);
        assert!(u[0] > 80, "mid grey encoded to {}", u[0]);
    }
}
