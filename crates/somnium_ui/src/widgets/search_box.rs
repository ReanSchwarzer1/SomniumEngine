// SearchBox / Breadcrumb / PropertyRow / Tooltip helpers (Phase 26-B).

use crate::{
    draw::DrawingContext,
    icons::IconId,
    message::{MessageDirection, NodeHandle, TextMessage, UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    theme,
    types::Rect,
    widget::{Widget, WidgetBuilder},
    widgets::text_box::TextBoxBuilder,
};
use glam::Vec2;

#[derive(Debug, Clone)]
pub enum SearchBoxMessage {
    /// Sent `FromWidget` as the user types.
    Query(String),
    /// Sent `ToWidget` to replace the contents — Esc clearing a live filter,
    /// or a workspace restoring one.
    SetText(String),
}

pub struct SearchBox {
    pub query: String,
    pub font_id: u8,
    pub focused: bool,
}

impl Control for SearchBox {
    fn is_keyboard_focusable(&self) -> bool {
        true
    }

    fn measure_override(&self, _widget: &Widget, _ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        Vec2::new(available.x.max(80.0), theme::active().density.row_dense)
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let paint = crate::style::input(crate::style::VisualState::rest().focused(self.focused));
        ctx.push_paint(b, &paint);
        let ic = Rect::new(b.x + 4.0, b.y + 3.0, 16.0, 16.0);
        let (uv, tex) = IconId::Search.draw_quad(ic);
        ctx.push_textured_rect(ic, uv, theme::active().semantic.text.secondary.bytes(), tex);
        let shown = if self.query.is_empty() {
            "Search"
        } else {
            self.query.as_str()
        };
        let color = if self.query.is_empty() {
            theme::active().semantic.text.disabled.bytes()
        } else {
            theme::active().semantic.text.primary.bytes()
        };
        ctx.push_text(
            shown,
            Vec2::new(b.x + 22.0, b.y + 4.0),
            self.font_id,
            12.0,
            color,
        );
    }

    fn is_text_input(&self) -> bool {
        self.focused
    }

    fn cursor_icon(&self, _widget: &Widget, _pos: Vec2) -> crate::node::CursorKind {
        crate::node::CursorKind::Text
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        // Programmatic clear (Esc dropping a live filter). No `Query` is
        // emitted back: the caller that sent this already knows, and echoing
        // would make an Esc that clears two boxes look like one that cleared
        // one.
        if let Some(SearchBoxMessage::SetText(text)) = msg.data::<SearchBoxMessage>() {
            self.query = text.clone();
            widget.invalidate_layout();
            msg.handled = true;
            return;
        }
        if let Some(wmsg) = msg.data::<WidgetMessage>() {
            match wmsg {
                WidgetMessage::Focus => {
                    self.focused = true;
                    msg.handled = true;
                }
                WidgetMessage::Unfocus => {
                    self.focused = false;
                    msg.handled = true;
                }
                WidgetMessage::Text(s) => {
                    self.query.push_str(s);
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        SearchBoxMessage::Query(self.query.clone()),
                    ));
                    msg.handled = true;
                }
                WidgetMessage::KeyDown(crate::message::KeyCode::Backspace, _) => {
                    self.query.pop();
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        SearchBoxMessage::Query(self.query.clone()),
                    ));
                    msg.handled = true;
                }
                WidgetMessage::KeyDown(crate::message::KeyCode::Escape, _) => {
                    self.query.clear();
                    self.focused = false;
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        SearchBoxMessage::Query(String::new()),
                    ));
                    msg.handled = true;
                }
                _ => {}
            }
        }
    }
}

pub struct SearchBoxBuilder {
    widget: WidgetBuilder,
    font_id: u8,
}

impl SearchBoxBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self { widget, font_id: 0 }
    }
    pub fn with_font_id(mut self, id: u8) -> Self {
        self.font_id = id;
        self
    }
    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(SearchBox {
                query: String::new(),
                font_id: self.font_id,
                focused: false,
            }),
        )
    }
}

#[derive(Debug, Clone)]
pub enum BreadcrumbMessage {
    Navigate(usize),
    SetParts(Vec<String>),
}

pub struct Breadcrumb {
    pub parts: Vec<String>,
    pub font_id: u8,
}

impl Control for Breadcrumb {
    fn measure_override(&self, _widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let w: f32 = self
            .parts
            .iter()
            .map(|p| ctx.measure_text(p, 11.0, self.font_id).x + 16.0)
            .sum();
        Vec2::new(available.x.max(w), 20.0)
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let mut x = b.x + 4.0;
        for (i, part) in self.parts.iter().enumerate() {
            ctx.push_text(
                part,
                Vec2::new(x, b.y + 3.0),
                self.font_id,
                11.0,
                theme::TEXT_LINK,
            );
            x += 8.0 + part.len() as f32 * 6.5;
            if i + 1 < self.parts.len() {
                ctx.push_text(
                    "/",
                    Vec2::new(x, b.y + 3.0),
                    self.font_id,
                    11.0,
                    theme::active().semantic.text.secondary.bytes(),
                );
                x += 12.0;
            }
        }
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if let Some(BreadcrumbMessage::SetParts(parts)) = msg.data::<BreadcrumbMessage>() {
            self.parts = parts.clone();
            widget.invalidate_layout();
            msg.handled = true;
            return;
        }
        if let Some(WidgetMessage::MouseDown { pos, .. }) = msg.data::<WidgetMessage>() {
            let b = widget.screen_bounds();
            let mut x = b.x + 4.0;
            for (i, part) in self.parts.iter().enumerate() {
                let w = 8.0 + part.len() as f32 * 6.5;
                if pos.x >= x && pos.x < x + w {
                    emit.push(UiMessage::new(
                        widget.handle,
                        MessageDirection::FromWidget,
                        BreadcrumbMessage::Navigate(i),
                    ));
                    msg.handled = true;
                    return;
                }
                x += w + 12.0;
            }
        }
    }
}

pub struct BreadcrumbBuilder {
    widget: WidgetBuilder,
    parts: Vec<String>,
    font_id: u8,
}

impl BreadcrumbBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            parts: Vec::new(),
            font_id: 0,
        }
    }
    pub fn with_parts(mut self, p: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.parts = p.into_iter().map(Into::into).collect();
        self
    }
    pub fn with_font_id(mut self, id: u8) -> Self {
        self.font_id = id;
        self
    }
    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(Breadcrumb {
                parts: self.parts,
                font_id: self.font_id,
            }),
        )
    }
}

/// Label + control row used by Details.
/// Build one Details row.
///
/// Phase 26-Zeta routes this through [`crate::widgets::property_row`] so every
/// inspector row shares the measured label/value grammar from the approved
/// redline instead of a hand-placed 110 px label. `font_id` is ignored — the
/// label's face comes from [`crate::typography::TextRole::Label`] — and is kept
/// in the signature so the ~120 existing call sites did not all have to change
/// in the same commit.
pub fn build_property_row(
    ui: &mut crate::ui::UserInterface,
    parent: NodeHandle,
    label: &str,
    font_id: u8,
    control: UiNode,
) -> (NodeHandle, NodeHandle) {
    let _ = font_id;
    let row = crate::widgets::property_row::PropertyRowBuilder::new(
        WidgetBuilder::new()
            .with_clip_to_bounds(false)
            .with_background(theme::TRANSPARENT),
    )
    .with_label(label)
    .build();
    let row_h = ui.add_node(row, parent);
    let control_h = ui.add_node(control, row_h);
    (row_h, control_h)
}

pub fn build_search_field(
    ui: &mut crate::ui::UserInterface,
    parent: NodeHandle,
    font_id: u8,
) -> NodeHandle {
    let node =
        SearchBoxBuilder::new(WidgetBuilder::new().with_height(theme::active().density.row_dense))
            .with_font_id(font_id)
            .build();
    ui.add_node(node, parent)
}

/// Unused TextBox helper kept so TextBox stays in the editor toolkit.
#[allow(dead_code)]
pub fn build_labelled_text_box(
    ui: &mut crate::ui::UserInterface,
    parent: NodeHandle,
    font_id: u8,
) -> NodeHandle {
    let box_ =
        TextBoxBuilder::new(WidgetBuilder::new().with_height(theme::active().density.row_dense))
            .build();
    let _ = font_id;
    ui.add_node(box_, parent)
}

pub struct Tooltip {
    pub text: String,
    pub font_id: u8,
}

impl Control for Tooltip {
    fn measure_override(&self, _widget: &Widget, ctx: &mut LayoutCtx, _available: Vec2) -> Vec2 {
        ctx.measure_text(&self.text, 11.0, self.font_id) + Vec2::new(12.0, 8.0)
    }
    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        let t = theme::active();
        // A tooltip floats over content, so it takes the popup rung rather than
        // sitting flat on whatever it happens to cover.
        let paint = crate::style::popup();
        ctx.push_paint(b, &paint);
        let _ = t;
        ctx.push_text(
            &self.text,
            Vec2::new(b.x + 6.0, b.y + 4.0),
            self.font_id,
            11.0,
            theme::active().semantic.text.primary.bytes(),
        );
    }
    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        _emit: &mut Vec<UiMessage>,
    ) {
        if let Some(TextMessage::SetText(s)) = msg.data::<TextMessage>() {
            self.text = s.clone();
            widget.invalidate_layout();
            msg.handled = true;
        }
    }
}

pub struct TooltipBuilder {
    widget: WidgetBuilder,
    text: String,
    font_id: u8,
}

impl TooltipBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self {
            widget,
            text: String::new(),
            font_id: 0,
        }
    }
    pub fn with_text(mut self, t: impl Into<String>) -> Self {
        self.text = t.into();
        self
    }
    pub fn with_font_id(mut self, id: u8) -> Self {
        self.font_id = id;
        self
    }
    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget
                .with_hit_test_visibility(false)
                // Left/Top, not the default Stretch. A tooltip measures itself
                // from its text — see `measure_override` above — but a
                // stretched widget is *arranged* to fill whatever it was given,
                // and this one is parented to the root. The measure was
                // correct and ignored, so every tooltip in the editor painted
                // as a slab from the cursor to the bottom-right corner of the
                // window. Alignment is what makes arrange honour the measure.
                .with_horizontal_alignment(crate::types::HorizontalAlignment::Left)
                .with_vertical_alignment(crate::types::VerticalAlignment::Top)
                .build(),
            Box::new(Tooltip {
                text: self.text,
                font_id: self.font_id,
            }),
        )
    }
}

/// The size a tooltip will take for `text`, before it is placed.
///
/// Mirrors [`Tooltip::measure_override`]; kept beside it so the placement
/// arithmetic and the measurement cannot drift apart.
#[must_use]
pub fn tooltip_size(measured_text: Vec2) -> Vec2 {
    measured_text + Vec2::new(12.0, 8.0)
}

/// Keep a tooltip on screen.
///
/// Below-right of the pointer by default, because that is where every
/// desktop toolkit puts one and because it leaves the thing being described
/// unobscured. Near an edge it flips to the other side rather than being
/// clamped flush against it, which would cover the control the pointer is
/// resting on.
#[must_use]
pub fn place_tooltip(cursor: Vec2, size: Vec2, window: Vec2) -> Vec2 {
    const GAP: Vec2 = Vec2::new(12.0, 18.0);
    let mut x = cursor.x + GAP.x;
    if x + size.x > window.x {
        x = (cursor.x - GAP.x - size.x).max(0.0);
    }
    let mut y = cursor.y + GAP.y;
    if y + size.y > window.y {
        y = (cursor.y - GAP.y - size.y).max(0.0);
    }
    Vec2::new(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tooltip is as big as its words. It stretched to fill the window for
    /// as long as this widget existed, because `WidgetBuilder`'s default
    /// alignment is `Stretch` and nothing here overrode it — the measure was
    /// right and arrange threw it away.
    #[test]
    fn a_tooltip_is_not_stretched_to_its_parent() {
        let node = TooltipBuilder::new(WidgetBuilder::new())
            .with_text("Show or hide the profiler overlay.")
            .build();
        assert_eq!(
            node.widget.horizontal_alignment,
            crate::types::HorizontalAlignment::Left
        );
        assert_eq!(
            node.widget.vertical_alignment,
            crate::types::VerticalAlignment::Top
        );
    }

    /// Below-right of the pointer when there is room.
    #[test]
    fn a_tooltip_sits_below_and_right_of_the_pointer() {
        let placed = place_tooltip(
            Vec2::new(400.0, 300.0),
            Vec2::new(220.0, 24.0),
            Vec2::new(1920.0, 1080.0),
        );
        assert_eq!(placed, Vec2::new(412.0, 318.0));
    }

    /// And flips rather than hanging off the edge. Clamping flush to the
    /// border instead would park the tooltip on top of the control the
    /// pointer is resting on, which is the one thing it must not cover.
    #[test]
    fn a_tooltip_near_an_edge_flips_to_the_other_side() {
        let window = Vec2::new(1000.0, 700.0);
        let size = Vec2::new(240.0, 24.0);
        let placed = place_tooltip(Vec2::new(980.0, 690.0), size, window);
        assert!(placed.x + size.x <= window.x, "stayed inside horizontally");
        assert!(placed.y + size.y <= window.y, "and vertically");
        assert!(placed.x < 980.0 && placed.y < 690.0, "flipped, not clamped");
    }

    /// A tooltip wider than the whole window is pinned to the left rather
    /// than pushed off it, which is the degenerate case the `max(0.0)` is for.
    #[test]
    fn an_over_wide_tooltip_starts_at_the_edge() {
        let placed = place_tooltip(
            Vec2::new(50.0, 50.0),
            Vec2::new(2000.0, 24.0),
            Vec2::new(800.0, 600.0),
        );
        assert_eq!(placed.x, 0.0);
    }

    #[test]
    fn search_starts_empty() {
        let s = SearchBox {
            query: String::new(),
            font_id: 0,
            focused: false,
        };
        assert!(s.query.is_empty());
    }
}
