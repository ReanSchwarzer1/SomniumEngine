//! PERSONA-H: causal interaction timing shared by editor controls.
//! Repeated draws observe a destination; only a changed destination starts work.
//! Initial model state is settled, so a populated inspector never animates values
//! merely because it was constructed. All geometry returned here is paint-only.
use super::{Animator, Easing, MotionKey, MotionProperty};
use crate::theme;

fn state(animator: &mut Animator, key: MotionKey, target: f32, ms: f32, easing: Easing) -> f32 {
    let previous = animator.target_or(key, f32::NAN);
    if previous.is_nan() {
        animator.set_immediate(key, target);
    } else if ms <= 0.0 {
        if animator.value_or(key, target) != target || previous != target {
            animator.set_immediate(key, target);
        }
    } else if previous != target {
        animator.start(key, target, target, ms, easing);
    }
    animator.value_or(key, target)
}

/// Shared hover policy. Content tiles explicitly opt out to avoid hover trails.
pub fn hover(animator: &mut Animator, key: MotionKey, on: bool, animate: bool) -> f32 {
    let t = theme::active().motion;
    let ms = if !animate {
        0.0
    } else if on {
        t.hover_ms as f32
    } else {
        t.press_ms as f32
    };
    state(
        animator,
        key,
        if on { 1.0 } else { 0.0 },
        ms,
        Easing::Decelerate,
    )
}

/// Press wash and face compression share one bounded, non-overshooting curve.
/// Labels, layout and pointer targets stay stationary.
pub fn press(animator: &mut Animator, node: u32, down: bool, enabled: bool) -> (f32, f32) {
    let t = theme::active().motion;
    let ms = if enabled { t.press_ms as f32 } else { 0.0 };
    let wash = state(
        animator,
        MotionKey::new(node, MotionProperty::PressWash),
        if down && enabled { 1.0 } else { 0.0 },
        ms,
        Easing::Spring,
    );
    let scale = state(
        animator,
        MotionKey::new(node, MotionProperty::ScaleY),
        if down && enabled { t.press_scale } else { 1.0 },
        ms,
        Easing::Spring,
    );
    (wash, scale)
}

/// Toggle glyph only: the underlying boolean, mixed state and input route are
/// immediate. Turning off reverses from the current opacity without a pop.
pub fn toggle(animator: &mut Animator, node: u32, checked: bool, animate: bool) -> (f32, f32) {
    let t = theme::active().motion;
    let ms = if animate { t.toggle_ms as f32 } else { 0.0 };
    let opacity = state(
        animator,
        MotionKey::new(node, MotionProperty::Opacity),
        if checked { 1.0 } else { 0.0 },
        ms,
        Easing::Decelerate,
    );
    let scale = state(
        animator,
        MotionKey::new(node, MotionProperty::Scale),
        if checked { 1.0 } else { t.toggle_scale },
        ms,
        Easing::Decelerate,
    );
    (opacity, scale)
}

/// Anchored popup presentation. Visibility/input remain owned by Popup; this
/// only supplies a paint envelope. Exit holds scale and fades from the current
/// value, including when an opening is interrupted.
pub fn popup(animator: &mut Animator, node: u32, open: bool) -> (f32, f32) {
    let t = theme::active().motion;
    let opacity_key = MotionKey::new(node, MotionProperty::Opacity);
    let scale_key = MotionKey::new(node, MotionProperty::Scale);
    if animator.target_or(opacity_key, f32::NAN).is_nan() {
        animator.set_immediate(opacity_key, 0.0);
    }
    if open
        && animator.target_or(opacity_key, 0.0) == 0.0
        && animator.value_or(opacity_key, 0.0) == 0.0
    {
        animator.set_immediate(scale_key, t.popup_scale);
    }
    let opacity = state(
        animator,
        opacity_key,
        if open { 1.0 } else { 0.0 },
        if open { t.popup_ms } else { t.popup_close_ms } as f32,
        if open {
            Easing::Decelerate
        } else {
            Easing::Accelerate
        },
    );
    let scale = if open {
        state(
            animator,
            scale_key,
            1.0,
            t.popup_ms as f32,
            Easing::Decelerate,
        )
    } else {
        let current = animator.value_or(scale_key, 1.0);
        animator.set_immediate(scale_key, current);
        current
    };
    (opacity, scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_draws_settle_and_reversal_does_not_snap() {
        let mut a = Animator::new();
        press(&mut a, 1, false, true);
        press(&mut a, 1, true, true);
        a.tick(30.0);
        let midway = press(&mut a, 1, true, true);
        assert!(midway.0 > 0.0 && midway.0 < 1.0);
        assert_eq!(press(&mut a, 1, false, true), midway);
        for _ in 0..20 {
            a.tick(16.0);
            press(&mut a, 1, false, true);
        }
        assert!(a.is_idle());
        assert_eq!(press(&mut a, 1, false, true), (0.0, 1.0));
        press(&mut a, 1, true, true);
        for _ in 0..13 {
            a.tick(16.0);
            press(&mut a, 1, true, true);
        }
        assert!(a.is_idle(), "held feedback must settle within 200 ms");
        assert_eq!(press(&mut a, 1, true, false), (0.0, 1.0));
    }
    #[test]
    fn populated_controls_start_idle_and_reduced_motion_has_exact_endpoints() {
        let mut a = Animator::new();
        assert_eq!(toggle(&mut a, 2, true, true), (1.0, 1.0));
        assert!(a.is_idle());
        toggle(&mut a, 2, false, true);
        a.tick(40.0);
        a.set_reduced_motion(true);
        assert_eq!(
            toggle(&mut a, 2, false, true),
            (0.0, theme::active().motion.toggle_scale)
        );
        assert_eq!(
            press(&mut a, 1, true, true),
            (1.0, theme::active().motion.press_scale)
        );
        assert!(a.is_idle());
    }
    #[test]
    fn content_hover_opt_out_is_immediate_even_mid_transition() {
        let mut a = Animator::new();
        let key = MotionKey::new(3, MotionProperty::HoverWash);
        hover(&mut a, key, false, true);
        hover(&mut a, key, true, true);
        a.tick(30.0);
        assert_eq!(hover(&mut a, key, false, false), 0.0);
        assert!(a.is_idle());
    }
}

#[cfg(test)]
mod widget_tests {
    use super::*;
    use crate::{
        message::{MessageDirection, UiMessage, WidgetMessage},
        ui::UserInterface,
        widget::WidgetBuilder,
        widgets::{
            button::ButtonBuilder,
            check_box::{CheckBoxBuilder, CheckBoxMessage},
        },
    };
    use glam::Vec2;

    #[test]
    fn destruction_clears_children_settled_values_and_recycled_indices() {
        let mut ui = UserInterface::new(200.0, 100.0);
        let parent = ui.add_node(ButtonBuilder::new(WidgetBuilder::new()).build(), ui.root());
        let child = ui.add_node(ButtonBuilder::new(WidgetBuilder::new()).build(), parent);
        let key = MotionKey::new(parent.index(), MotionProperty::ScaleY);
        ui.draw_ctx
            .motion
            .start(key, 1.0, 0.8, 90.0, Easing::Linear);
        let child_key = MotionKey::new(child.index(), MotionProperty::Opacity);
        ui.draw_ctx.motion.set_immediate(child_key, 0.3);
        ui.remove_node(parent);
        assert!(ui.draw_ctx.motion.is_idle());
        assert_eq!(ui.draw_ctx.motion.value_or(child_key, 1.0), 1.0);
        let next = ui.add_node(ButtonBuilder::new(WidgetBuilder::new()).build(), ui.root());
        assert_eq!(
            next.index(),
            parent.index(),
            "test must exercise pool reuse"
        );
        assert_eq!(ui.draw_ctx.motion.value_or(key, 1.0), 1.0);
        ui.draw_ctx
            .motion
            .start(key, 1.0, 0.9, 90.0, Easing::Linear);
        ui.remove_node(parent); // stale generation must not clear the new node
        assert_eq!(ui.draw_ctx.motion.active_count(), 1);
    }

    #[test]
    fn interaction_motion_preserves_hit_bounds_layout_and_checkbox_value() {
        for theme_id in [theme::ThemeId::Nocturne, theme::ThemeId::Dawn] {
            theme::set_active(theme_id);
            for reduced in [false, true] {
                let mut ui = UserInterface::new(200.0, 100.0);
                ui.draw_ctx.motion.set_reduced_motion(reduced);
                let button = ui.add_node(
                    ButtonBuilder::new(WidgetBuilder::new().with_width(100.0).with_height(30.0))
                        .build(),
                    ui.root(),
                );
                ui.perform_layout();
                ui.draw();
                let before = ui.nodes.borrow(button.transmute()).widget.screen_bounds();
                let point = Vec2::new(before.x + 4.0, before.y + 4.0);
                ui.send(UiMessage::new(
                    button,
                    MessageDirection::ToWidget,
                    WidgetMessage::MouseDown {
                        pos: point,
                        button: crate::message::MouseButton::Left,
                        mods: Default::default(),
                    },
                ));
                ui.update();
                ui.draw();
                ui.draw_ctx.motion.tick(45.0);
                ui.draw();
                assert_eq!(ui.hit_test(point), button);
                assert_eq!(
                    ui.nodes.borrow(button.transmute()).widget.screen_bounds(),
                    before
                );
                let scale = ui
                    .draw_ctx
                    .motion
                    .value_or(MotionKey::new(button.index(), MotionProperty::ScaleY), 1.0);
                assert!(scale < 1.0);
                if reduced {
                    assert_eq!(scale, theme::active().motion.press_scale);
                }
                ui.remove_node(button);
                let check = ui.add_node(
                    CheckBoxBuilder::new(WidgetBuilder::new().with_width(100.0).with_height(30.0))
                        .build(),
                    ui.root(),
                );
                ui.perform_layout();
                ui.draw();
                ui.send(CheckBoxMessage::set_checked(check, true));
                ui.update();
                ui.draw();
                assert_eq!(
                    ui.a11y_probe(check).unwrap().toggled,
                    Some(crate::a11y::Toggled::True)
                );
                for _ in 0..15 {
                    ui.draw_ctx.motion.tick(16.0);
                    ui.draw();
                }
                assert!(ui.draw_ctx.motion.is_idle());
            }
        }
        theme::set_active(theme::ThemeId::Nocturne);
    }
}
