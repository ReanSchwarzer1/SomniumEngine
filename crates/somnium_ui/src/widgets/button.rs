// Port of: example_repo/fyrox/Fyrox-master/fyrox-ui/src/button.rs
// Button: captures mouse on MouseDown; emits ButtonMessage::Click on MouseUp within bounds.
// Hover / press / selected fills so chrome controls read as buttons, not dead labels.

use crate::motion::{Easing, MotionKey, MotionProperty, lerp_color};
use crate::{
    draw::DrawingContext,
    message::{MessageDirection, UiMessage, WidgetMessage},
    node::{Control, LayoutCtx, UiNode},
    style::{Interaction, VisualState, button as style_button, icon_button as style_icon_button},
    theme,
    types::Rect,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;
use std::time::{Duration, Instant};

const DOUBLE_CLICK: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, PartialEq)]
pub enum ButtonMessage {
    Click,
    DoubleClick,
    SetSelected(bool),
}

impl ButtonMessage {
    pub fn set_selected(dest: crate::message::NodeHandle, selected: bool) -> UiMessage {
        UiMessage::new(
            dest,
            MessageDirection::ToWidget,
            Self::SetSelected(selected),
        )
    }
}

pub struct Button {
    pub is_pressed: bool,
    pub hovered: bool,
    pub selected: bool,
    /// Keyboard focus — draws the 1 px ring without changing the fill.
    pub focused: bool,
    last_up: Option<Instant>,
}

impl Control for Button {
    // MORROWIND-I. A button's accessible name is its label if it has one and
    // its tooltip otherwise — `UserInterface::a11y_probe` supplies the second,
    // which is why this returns `None` rather than an empty string for an
    // icon-only button. An empty name would *shadow* the tooltip.
    fn role(&self) -> crate::a11y::Role {
        crate::a11y::Role::Button
    }

    fn is_keyboard_focusable(&self) -> bool {
        true
    }

    fn measure_override(&self, widget: &Widget, ctx: &mut LayoutCtx, available: Vec2) -> Vec2 {
        let mut desired = Vec2::ZERO;
        for &ch in &widget.children {
            ctx.measure_child(ch, available);
            let ds = ctx.desired_size(ch);
            if ds.x > desired.x {
                desired.x = ds.x;
            }
            if ds.y > desired.y {
                desired.y = ds.y;
            }
        }
        desired
    }

    fn arrange_override(&self, widget: &Widget, ctx: &mut LayoutCtx, final_size: Vec2) -> Vec2 {
        let ox = widget.actual_local_position.x;
        let oy = widget.actual_local_position.y;
        let rect = Rect::new(ox, oy, final_size.x, final_size.y);
        for &ch in &widget.children {
            ctx.arrange_child(ch, rect);
        }
        final_size
    }

    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        let b = widget.screen_bounds();
        // Zeta-C: paint comes from the shared recipes. A button whose caller
        // gave it no background is a chrome/icon button — it must stay
        // transparent at rest so a command band reads as one surface — while a
        // button with an explicit background keeps it as its rest fill.
        let ghost = widget.background[3] == 0;
        let state = VisualState::with(if !widget.enabled {
            Interaction::Disabled
        } else if self.is_pressed {
            Interaction::Pressed
        } else if self.selected {
            Interaction::Selected
        } else if self.hovered {
            Interaction::Hover
        } else {
            Interaction::Rest
        })
        .focused(self.focused);
        let paint = if ghost {
            style_icon_button(state)
        } else {
            style_button(state)
        };
        // Phase 27-D. An explicit caller background still wins at rest, but
        // everything else — radius, the chrome wash, the elevation lift, the
        // focus glow and the selection rail — now comes from the recipe and is
        // rendered in one call so the layer order cannot be got wrong here.
        let mut paint = paint;
        if !ghost && state.interaction == Interaction::Rest && widget.background[3] > 0 {
            paint.background = widget.background;
            // The caller picked the *hue*, not the flatness. Re-derive the wash
            // from that base so the button still reads as a lit chrome surface.
            // Suppressing it here is what left the shell looking unchanged after
            // the recipes already described the depth.
            paint.gradient = Some(theme::wash_from(theme::Srgb8(widget.background)));
        }

        // Phase 27-C. Cross-fade the hover wash instead of snapping to it. The
        // track is keyed on this node, so two buttons hovered in sequence do not
        // share state, and it retires the moment it completes.
        let t = theme::active();
        let key = MotionKey::new(widget.handle.index(), MotionProperty::HoverWash);
        let target = if state.interaction == Interaction::Hover {
            1.0
        } else {
            0.0
        };
        ctx.motion
            .start(key, 0.0, target, t.motion.hover_ms as f32, Easing::Standard);
        let wash = ctx.motion.value_or(key, target);
        if wash > 0.0 && wash < 1.0 && state.interaction != Interaction::Pressed {
            let rest = if ghost {
                style_icon_button(VisualState::rest())
            } else {
                style_button(VisualState::rest())
            };
            let hovered = if ghost {
                style_icon_button(VisualState::with(Interaction::Hover))
            } else {
                style_button(VisualState::with(Interaction::Hover))
            };
            paint.background = lerp_color(rest.background, hovered.background, wash);
            paint.foreground = lerp_color(rest.foreground, hovered.foreground, wash);
        }

        if paint.background[3] > 0 || paint.rail.is_some() || paint.glow.is_some() {
            ctx.push_paint(b, &paint);
        }
    }

    fn cursor_icon(&self, _widget: &Widget, _pos: Vec2) -> crate::node::CursorKind {
        crate::node::CursorKind::Pointer
    }

    fn handle_routed_message(
        &mut self,
        widget: &mut Widget,
        msg: &mut UiMessage,
        emit: &mut Vec<UiMessage>,
    ) {
        if let Some(ButtonMessage::SetSelected(v)) = msg.data::<ButtonMessage>() {
            self.selected = *v;
            msg.handled = true;
            return;
        }
        if let Some(wmsg) = msg.data::<WidgetMessage>() {
            match wmsg {
                WidgetMessage::MouseEnter => {
                    self.hovered = true;
                }
                WidgetMessage::MouseLeave => {
                    self.hovered = false;
                    self.is_pressed = false;
                }
                WidgetMessage::Focus => {
                    self.focused = true;
                }
                WidgetMessage::Unfocus => {
                    self.focused = false;
                }
                WidgetMessage::MouseDown { .. } => {
                    self.is_pressed = true;
                    msg.handled = true;
                }
                WidgetMessage::MouseUp { pos, .. } => {
                    let pos = *pos;
                    if self.is_pressed && widget.screen_bounds().contains(pos) {
                        let now = Instant::now();
                        let double = self
                            .last_up
                            .is_some_and(|t| now.duration_since(t) <= DOUBLE_CLICK);
                        self.last_up = Some(now);
                        emit.push(UiMessage::new(
                            widget.handle,
                            MessageDirection::FromWidget,
                            if double {
                                ButtonMessage::DoubleClick
                            } else {
                                ButtonMessage::Click
                            },
                        ));
                    }
                    self.is_pressed = false;
                    msg.handled = true;
                }
                WidgetMessage::KeyDown(key, _) => {
                    use crate::message::KeyCode;
                    if matches!(key, KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space) {
                        emit.push(UiMessage::new(
                            widget.handle,
                            MessageDirection::FromWidget,
                            ButtonMessage::Click,
                        ));
                        msg.handled = true;
                    }
                }
                _ => {}
            }
        }
    }
}

pub struct ButtonBuilder {
    widget: WidgetBuilder,
}

impl ButtonBuilder {
    pub fn new(widget: WidgetBuilder) -> Self {
        Self { widget }
    }

    pub fn build(self) -> UiNode {
        UiNode::new(
            self.widget.build(),
            Box::new(Button {
                is_pressed: false,
                hovered: false,
                selected: false,
                focused: false,
                last_up: None,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{MouseButton, NodeHandle};

    fn click(button: &mut Button, widget: &mut Widget, emit: &mut Vec<UiMessage>) {
        let pos = Vec2::new(4.0, 4.0);
        let mut down = UiMessage::new(
            widget.handle,
            MessageDirection::FromWidget,
            WidgetMessage::MouseDown {
                pos,
                button: MouseButton::Left,
                mods: crate::message::Modifiers::default(),
            },
        );
        button.handle_routed_message(widget, &mut down, emit);
        let mut up = UiMessage::new(
            widget.handle,
            MessageDirection::FromWidget,
            WidgetMessage::MouseUp {
                pos,
                button: MouseButton::Left,
                mods: crate::message::Modifiers::default(),
            },
        );
        button.handle_routed_message(widget, &mut up, emit);
    }

    #[test]
    fn second_click_within_400ms_emits_double_click() {
        let mut button = Button {
            is_pressed: false,
            hovered: false,
            selected: false,
            focused: false,
            last_up: None,
        };
        let mut widget = Widget::default();
        widget.handle = NodeHandle::NONE;
        widget.actual_local_position = Vec2::ZERO;
        widget.actual_local_size = Vec2::new(32.0, 32.0);
        let mut emit = Vec::new();
        click(&mut button, &mut widget, &mut emit);
        click(&mut button, &mut widget, &mut emit);
        assert!(matches!(
            emit[0].data::<ButtonMessage>(),
            Some(ButtonMessage::Click)
        ));
        assert!(matches!(
            emit[1].data::<ButtonMessage>(),
            Some(ButtonMessage::DoubleClick)
        ));
    }
}
