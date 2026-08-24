//! Screen-space game canvas (Phase 26-G).
//!
//! Wraps [`UserInterface`] + [`UiPass`] without `UiManager` editor chrome so a
//! game can build a widget tree (HUD, pause, menus) and draw it through the
//! same GPU pass the editor uses. Nine-slice lives on [`DrawingContext`].

use crate::{
    message::NodeHandle,
    pass::UiPass,
    ui::UserInterface,
    widget::WidgetBuilder,
    widgets::{border::BorderBuilder, text::TextBuilder},
};
use glam::Vec2;
use winit::event::WindowEvent;
use winit::window::Window;

/// Retained screen-space UI owned by a game, not the editor shell.
pub struct UiCanvas {
    ui: UserInterface,
    pass: Option<UiPass>,
    font_id: u8,
}

impl UiCanvas {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            ui: UserInterface::new(width, height),
            pass: None,
            font_id: 0,
        }
    }

    pub fn ui(&self) -> &UserInterface {
        &self.ui
    }

    pub fn ui_mut(&mut self) -> &mut UserInterface {
        &mut self.ui
    }

    pub fn font_id(&self) -> u8 {
        self.font_id
    }

    pub fn add_font(&mut self, bytes: &[u8]) -> Result<u8, &'static str> {
        let id = self.ui.add_font(bytes)?;
        self.font_id = id;
        Ok(id)
    }

    pub fn process_os_event(&mut self, event: &WindowEvent) -> bool {
        self.ui.process_os_event(event)
    }

    /// Layout + draw. Creates the GPU pass on first call.
    ///
    /// A game canvas lays out in logical units exactly like the editor shell
    /// does, so the HUD keeps its apparent size on a HiDPI display. The scale
    /// is read from the window each frame rather than cached, because a canvas
    /// is cheap to lay out and a game may be dragged between monitors.
    pub fn render(
        &mut self,
        window: &Window,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        output_format: wgpu::TextureFormat,
    ) {
        let ui_scale = window.scale_factor() as f32;
        self.ui.set_ui_scale(ui_scale);
        self.ui.draw_ctx.font_atlas.set_render_scale(ui_scale);
        self.ui.draw_ctx.icon_atlas.set_render_scale(ui_scale);

        let _ = self.ui.update();
        self.ui.perform_layout();
        self.ui.draw();

        let logical_w = self.ui.screen_size.x.max(1.0);
        let logical_h = self.ui.screen_size.y.max(1.0);
        let physical = window.inner_size();
        let (phys_w, phys_h) = (physical.width.max(1), physical.height.max(1));

        let pass = self
            .pass
            .get_or_insert_with(|| UiPass::new(device, queue, output_format));
        pass.prepare(
            device,
            queue,
            &mut self.ui.draw_ctx,
            crate::pass::UiSurface::new((logical_w, logical_h), (phys_w, phys_h)),
        );
        pass.render(encoder, view);
    }

    /// Optional pause/HUD stub: a centred label on a dim panel.
    pub fn add_pause_banner(&mut self, text: &str) -> NodeHandle {
        let root = self.ui.root();
        let panel = BorderBuilder::new(
            WidgetBuilder::new()
                .with_width(280.0)
                .with_height(64.0)
                .with_desired_position(Vec2::new(40.0, 40.0))
                .with_background(crate::theme::BG_HEADER)
                .with_foreground(crate::theme::BORDER_DARK),
        )
        .with_stroke_thickness(crate::types::Thickness::uniform(1.0))
        .build();
        let panel_h = self.ui.add_node(panel, root);
        let label = TextBuilder::new(
            WidgetBuilder::new().with_margin(crate::types::Thickness::uniform(16.0)),
        )
        .with_text(text)
        .with_font_id(self.font_id)
        .with_font_size(16.0)
        .with_color(crate::theme::TEXT_PRIMARY)
        .build();
        self.ui.add_node(label, panel_h);
        panel_h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::text::TextBuilder;

    #[test]
    fn canvas_builds_a_widget_tree_without_editor_chrome() {
        let mut canvas = UiCanvas::new(640.0, 360.0);
        let root = canvas.ui().root();
        let t = TextBuilder::new(WidgetBuilder::new())
            .with_text("HUD")
            .build();
        let h = canvas.ui_mut().add_node(t, root);
        assert!(h.is_some());
        canvas.ui_mut().perform_layout();
        canvas.ui_mut().draw();
    }
}
