use super::*;
use crate::style::{self, ButtonVariant, Interaction, VisualState};

#[test]
fn exported_metrics_match_both_densities_and_themes() {
    for (sheet, t) in [
        (
            include_str!("../assets/tokens/nocturne.tokens.json"),
            NOCTURNE,
        ),
        (include_str!("../assets/tokens/dawn.tokens.json"), DAWN),
    ] {
        let sheet: serde_json::Value = serde_json::from_str(sheet).unwrap();
        for (key, d) in [
            ("density", t.density),
            ("density_comfortable", COMFORTABLE_DENSITY),
        ] {
            macro_rules! density { ($($field:ident),*) => { $(
                assert_eq!(sheet[key][stringify!($field)].as_f64().unwrap() as f32, d.$field);
            )* }; }
            density!(
                row_dense,
                row_tree,
                row_chrome,
                titlebar,
                menu,
                toolbar,
                status,
                icon_row,
                icon_toolbar,
                icon_action,
                hit_min
            );
        }
        let g = t.geometry;
        for (group, key, value) in [
            ("space", "base", g.space_base),
            ("space", "inset_panel", g.inset_panel),
            ("space", "gap_group", g.gap_group),
            ("space", "gap_section", g.gap_section),
            ("radius", "input", g.radius_input),
            ("radius", "chrome", g.radius_chrome),
            ("radius", "popup", g.radius_popup),
            ("radius", "modal", g.radius_modal),
            ("radius", "tile", g.radius_tile),
            ("stroke", "hairline", g.stroke_hairline),
            ("stroke", "focus", g.stroke_focus),
            ("stroke", "rail", g.stroke_rail),
        ] {
            assert_eq!(
                sheet[group][key].as_f64().unwrap() as f32,
                value,
                "{group}.{key}"
            );
        }
        let m = t.motion;
        for (key, v) in [
            ("press", m.press_ms),
            ("hover", m.hover_ms),
            ("popup", m.popup_ms),
            ("drawer", m.drawer_ms),
            ("tooltip_delay", m.tooltip_delay_ms),
        ] {
            assert_eq!(sheet["motion_ms"][key].as_u64(), Some(v));
        }
        let ty = t.typography;
        for (key, v, w) in [
            ("display", ty.display, 600),
            ("title", ty.title, 600),
            ("section", ty.section, 600),
            ("body", ty.body, 400),
            ("body_strong", ty.body_strong, 500),
            ("label", ty.label, 500),
            ("caption", ty.caption, 400),
            ("mono", ty.mono, 400),
            ("mono_strong", ty.mono_strong, 500),
        ] {
            assert_eq!(sheet["typography"][key][0].as_f64().unwrap() as f32, v);
            assert_eq!(sheet["typography"][key][1].as_u64(), Some(w));
        }
    }
}

#[test]
fn theme_density_and_contrast_are_independent_and_readable() {
    for id in [ThemeId::Nocturne, ThemeId::Dawn] {
        set_active(id);
        for density in [DensityId::Compact, DensityId::Comfortable] {
            set_density(density);
            for hc in [false, true] {
                set_high_contrast(hc);
                let t = active();
                assert_eq!(active_id(), id);
                assert_eq!(density_id(), density);
                assert_eq!(
                    crate::widgets::property_row::row_metrics(340.0).height,
                    t.density.row_dense
                );
                assert_eq!(
                    crate::typography::text_style(crate::typography::TextRole::Label).color,
                    t.semantic.text.secondary.bytes()
                );
                for bg in [
                    t.semantic.surface.panel,
                    t.semantic.surface.input,
                    t.semantic.surface.raised,
                    t.semantic.surface.hover,
                    t.semantic.surface.selected,
                ] {
                    for fg in [
                        t.semantic.text.primary,
                        t.semantic.text.secondary,
                        t.semantic.text.muted,
                    ] {
                        assert!(
                            contrast_ratio(fg, bg) >= if hc { 6.99 } else { 4.5 },
                            "{id:?}/{density:?}/{hc}: {:?} on {:?}",
                            fg,
                            bg
                        );
                    }
                }
                assert!(contrast_ratio(t.semantic.border.control, t.semantic.surface.input) >= 3.0);
            }
        }
    }
    set_active(ThemeId::Nocturne);
    set_density(DensityId::Compact);
    set_high_contrast(false);
}

#[test]
fn focused_invalid_field_keeps_both_strokes_within_its_clip() {
    use crate::{draw::DrawingContext, types::Rect};
    let t = active();
    let paint = style::input(VisualState::rest().invalid(true).focused(true));
    let rect = Rect::new(10.0, 10.0, 120.0, 28.0);
    let mut ctx = DrawingContext::new(160.0, 60.0);
    ctx.push_clip_rect(rect);
    ctx.push_paint(rect, &paint);
    for color in [
        t.semantic.status.error.bytes(),
        t.semantic.border.focus.bytes(),
    ] {
        assert!(ctx.instances.iter().any(|p| p.border_color == color
            && p.border_width > 0.0
            && p.rect[0] >= rect.x
            && p.rect[1] >= rect.y
            && p.rect[0] + p.rect[2] <= rect.x + rect.w
            && p.rect[1] + p.rect[3] <= rect.y + rect.h));
    }
}

#[test]
fn inactive_selection_and_action_hierarchy_keep_distinct_cues() {
    let selected = VisualState::with(Interaction::Selected);
    let active = style::tree_row(selected);
    let inactive = style::tree_row(selected.inactive(true));
    assert_ne!(active.background, inactive.background);
    assert!(inactive.rail.is_some());
    assert_ne!(
        super::active().semantic.modified,
        super::active().semantic.accent.default
    );
    for variant in [
        ButtonVariant::Secondary,
        ButtonVariant::Quiet,
        ButtonVariant::Toggle,
        ButtonVariant::Destructive,
    ] {
        let paint = style::action_button(variant, VisualState::rest());
        assert!(paint.gradient.is_none() && paint.elevation.is_none());
    }
    assert!(
        style::action_button(ButtonVariant::Primary, VisualState::rest())
            .gradient
            .is_some()
    );
    assert!(
        style::input(VisualState::with(Interaction::Disabled).focused(true))
            .focus_ring
            .is_none()
    );
}

#[test]
fn action_label_color_is_scoped_to_its_button_subtree() {
    use crate::{
        draw::DrawingContext,
        node::{Control, UiNode},
        ui::UserInterface,
        widget::{Widget, WidgetBuilder},
        widgets::button::ButtonBuilder,
    };
    struct LabelProbe;
    impl Control for LabelProbe {
        fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
            ctx.push_rect_filled(
                widget.screen_bounds(),
                ctx.inherited_foreground.unwrap_or([1, 2, 3, 255]),
            );
        }
    }
    let mut ui = UserInterface::new(240.0, 100.0);
    let root = ui.root();
    let button = ui.add_node(
        ButtonBuilder::new(WidgetBuilder::new())
            .with_variant(ButtonVariant::Primary)
            .build(),
        root,
    );
    ui.add_node(
        UiNode::new(WidgetBuilder::new().build(), Box::new(LabelProbe)),
        button,
    );
    ui.add_node(
        UiNode::new(WidgetBuilder::new().build(), Box::new(LabelProbe)),
        root,
    );
    ui.update();
    ui.perform_layout();
    ui.draw();
    assert!(
        ui.draw_ctx
            .instances
            .iter()
            .any(|p| p.fill_a == active().semantic.text.inverse.bytes())
    );
    assert_eq!(ui.draw_ctx.instances.last().unwrap().fill_a, [1, 2, 3, 255]);
    assert!(ui.draw_ctx.inherited_foreground.is_none());
}
