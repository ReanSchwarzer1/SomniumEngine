//! Native PERSONA state sheet. Enabled only by the existing audit startup hook.
//! Recipes are the same ones used by widgets; the bottom strip contains live controls.
use crate::{
    draw::DrawingContext,
    message::UiMessage,
    node::{Control, LayoutCtx, UiNode},
    style::{self, ButtonVariant, Interaction, VisualState},
    theme,
    types::{HorizontalAlignment, Rect, VerticalAlignment},
    typography::{self, TextRole},
    ui::UserInterface,
    widget::{Widget, WidgetBuilder},
};
use glam::Vec2;

struct StateSheet {
    high_contrast: bool,
}

fn label(ctx: &mut DrawingContext, text: &str, x: f32, y: f32, role: TextRole) {
    let t = typography::text_style(role);
    ctx.push_text(text, Vec2::new(x, y), t.font_id(), t.px, t.color);
}

impl Control for StateSheet {
    fn measure_override(&self, _: &Widget, _: &mut LayoutCtx, available: Vec2) -> Vec2 {
        available
    }
    fn arrange_override(&self, _: &Widget, _: &mut LayoutCtx, size: Vec2) -> Vec2 {
        size
    }
    fn handle_routed_message(&mut self, _: &mut Widget, _: &mut UiMessage, _: &mut Vec<UiMessage>) {
    }
    fn draw(&self, widget: &Widget, ctx: &mut DrawingContext) {
        theme::set_high_contrast(self.high_contrast);
        let t = theme::active();
        let b = widget.screen_bounds();
        ctx.push_rect_filled(b, t.semantic.surface.panel.bytes());
        label(
            ctx,
            "Nocturne Atelier / PERSONA",
            32.0,
            24.0,
            TextRole::Title,
        );
        let mode = format!(
            "{:?}  /  {:?}  /  {}",
            theme::active_id(),
            theme::density_id(),
            if self.high_contrast {
                "High contrast"
            } else {
                "Standard contrast"
            }
        );
        label(ctx, &mode, 32.0, 54.0, TextRole::Body);
        label(
            ctx,
            "Native paint states. Focus and validation compose; density is independent of DPI.",
            32.0,
            82.0,
            TextRole::Caption,
        );
        let states = [
            ("Rest", VisualState::rest()),
            ("Hover", VisualState::with(Interaction::Hover)),
            ("Pressed", VisualState::with(Interaction::Pressed)),
            ("Selected", VisualState::with(Interaction::Selected)),
            (
                "Inactive",
                VisualState::with(Interaction::Selected).inactive(true),
            ),
            ("Focus", VisualState::rest().focused(true)),
            ("Invalid", VisualState::rest().invalid(true)),
            (
                "Focus + error",
                VisualState::rest().focused(true).invalid(true),
            ),
            ("Disabled", VisualState::with(Interaction::Disabled)),
        ];
        let pitch = (b.w - 200.0) / states.len() as f32;
        let height = t.density.row_chrome;
        for (i, (name, _)) in states.iter().enumerate() {
            label(
                ctx,
                name,
                180.0 + i as f32 * pitch,
                123.0,
                TextRole::Caption,
            );
        }
        for (row, name) in [
            "Primary",
            "Secondary",
            "Quiet",
            "Toggle",
            "Destructive",
            "Input",
            "Tree row",
            "Asset tile",
        ]
        .into_iter()
        .enumerate()
        {
            let y = 150.0 + row as f32 * 48.0;
            label(ctx, name, 32.0, y + 7.0, TextRole::Label);
            for (col, (_, state)) in states.iter().enumerate() {
                let paint = match row {
                    0 => style::action_button(ButtonVariant::Primary, *state),
                    1 => style::action_button(ButtonVariant::Secondary, *state),
                    2 => style::action_button(ButtonVariant::Quiet, *state),
                    3 => style::action_button(ButtonVariant::Toggle, *state),
                    4 => style::action_button(ButtonVariant::Destructive, *state),
                    5 => style::input(*state),
                    6 => style::tree_row(*state),
                    _ => style::asset_tile(*state),
                };
                let r = Rect::new(180.0 + col as f32 * pitch, y, pitch - 12.0, height);
                ctx.push_paint(r, &paint);
                ctx.push_text(
                    if row == 5 { "1.250 m" } else { "Sample" },
                    Vec2::new(r.x + 9.0, r.y + 7.0),
                    typography::font_id(typography::FontRole::UiRegular),
                    12.0,
                    paint.foreground,
                );
            }
        }
        label(ctx, "Live controls", 32.0, 554.0, TextRole::Section);
        label(
            ctx,
            "Tab / type / scrub. Mixed values remain untouched until edited. Modified uses a separate warm cue.",
            32.0,
            581.0,
            TextRole::Caption,
        );
        ctx.push_round_rect(
            Rect::new(34.0, 665.0, 6.0, 6.0),
            3.0,
            t.semantic.modified.bytes(),
        );
        label(ctx, "Modified from default", 50.0, 660.0, TextRole::Caption);
        label(
            ctx,
            "Long names: Coastal environment / Sun direction and atmospheric scattering",
            260.0,
            660.0,
            TextRole::Caption,
        );
    }
}

pub(crate) fn show(ui: &mut UserInterface) {
    ui.audit_component_gallery = true;
    use crate::widgets::{
        button::ButtonBuilder, canvas::CanvasBuilder, numeric_field::NumericFieldBuilder,
        text::TextBuilder, text_box::TextBoxBuilder,
    };
    let hc = std::env::var("SOMNIUM_AUDIT_HIGH_CONTRAST").as_deref() == Ok("1");
    theme::set_high_contrast(hc);
    let mut root = WidgetBuilder::new().with_background(theme::TRANSPARENT);
    root.widget.z_index = 10_000;
    let host = ui.add_node(CanvasBuilder::new(root).build(), ui.root());
    ui.add_node(
        UiNode::new(
            WidgetBuilder::new()
                .with_width(ui.screen_size.x)
                .with_height(ui.screen_size.y)
                .build(),
            Box::new(StateSheet { high_contrast: hc }),
        ),
        host,
    );
    let h = theme::active().density.row_chrome;
    let at = |x| {
        WidgetBuilder::new()
            .with_desired_position(Vec2::new(x, 612.0))
            .with_width(210.0)
            .with_height(h)
            .with_horizontal_alignment(HorizontalAlignment::Left)
            .with_vertical_alignment(VerticalAlignment::Top)
    };
    ui.add_node(
        TextBoxBuilder::new(at(32.0))
            .with_text("Coastal lighting")
            .build(),
        host,
    );
    ui.add_node(
        NumericFieldBuilder::new(at(260.0))
            .with_value(1.25)
            .with_unit("m")
            .build(),
        host,
    );
    ui.add_node(
        NumericFieldBuilder::new(at(488.0)).with_mixed(true).build(),
        host,
    );
    let button = ui.add_node(
        ButtonBuilder::new(at(716.0))
            .with_variant(ButtonVariant::Secondary)
            .build(),
        host,
    );
    ui.add_node(
        TextBuilder::new(WidgetBuilder::new())
            .with_text("Keyboard action")
            .with_role(TextRole::Body)
            .build(),
        button,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_sheet_has_finite_full_window_bounds_and_state_cells() {
        let mut ui = UserInterface::new(1280.0, 720.0);
        show(&mut ui);
        ui.set_axis_widget(
            Rect::new(0.0, 0.0, 1280.0, 720.0),
            [Vec2::X, Vec2::Y, -Vec2::X],
        );
        ui.update();
        ui.perform_layout();
        ui.draw();
        assert!(
            !ui.draw_ctx
                .instances
                .iter()
                .any(|p| p.fill_a == [0x5A, 0xD0, 0x6A, 255])
        );
        assert!(
            ui.draw_ctx
                .instances
                .iter()
                .any(|p| p.rect == [0.0, 0.0, 1280.0, 720.0]
                    && p.fill_a == theme::active().semantic.surface.panel.bytes())
        );
        assert!(
            ui.draw_ctx
                .instances
                .iter()
                .filter(|p| p.rect[0] >= 180.0
                    && p.rect[1] >= 150.0
                    && p.rect[1] < 535.0
                    && p.rect[2] > 80.0
                    && p.rect[2].is_finite())
                .count()
                >= 60
        );
    }
}
