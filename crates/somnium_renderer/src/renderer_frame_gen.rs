//! FSR 3 frame generation's part of the frame (`pass/frame_gen.rs` has the how
//! and why). A child module of the renderer so these hooks reach its passes;
//! `renderer.rs` only calls them, from `render_with_game_ui` and at the split
//! points in `record_scene_view`.

use super::SomniumRenderer as Renderer;
use crate::context::RenderContext;
use crate::pass::frame_gen::{FrameGenCadence, SPLIT_START};
use winit::window::Window;

impl Renderer {
    /// Turn AMD FSR 3 frame generation on or off. Takes effect from the next frame;
    /// it runs only in the game's own view (immersive, one view, not floated).
    pub fn set_frame_generation(&mut self, on: bool) {
        self.frame_gen.set_requested(on);
    }

    /// Whether frame generation is asked for.
    pub fn frame_generation_requested(&self) -> bool {
        self.frame_gen.requested()
    }

    /// Whether the last frame generated one.
    pub fn frame_generation_active(&self) -> bool {
        self.frame_gen.active()
    }

    /// Presented-frame cadence over the last few seconds (GPU timestamps).
    pub fn frame_generation_cadence(&self) -> FrameGenCadence {
        self.frame_gen.cadence()
    }

    /// A cut the camera test cannot see (a cinematic's cut to a nearby angle):
    /// the next frame is not interpolated from the one before.
    pub fn reset_frame_generation(&mut self) {
        self.frame_gen.request_reset();
    }

    /// The start of the frame. While generating, draws last frame's picture with
    /// this frame's UI into a swap chain image held for the split point, and
    /// returns the picture the scene is drawn into.
    pub(super) fn frame_gen_begin<'o>(&mut self, ctx: &RenderContext, window: &Window,
        game_ui: Option<&mut (dyn somnium_ui::GameUi + 'o)>, eligible: bool) -> Option<wgpu::TextureView> {
        let size = [ctx.config.width, ctx.config.height];
        let render = [self.render_width, self.render_height];
        if !self.frame_gen.begin(&ctx.device, &ctx.queue, ctx.config.format, size, render, eligible) {
            return None;
        }
        if self.frame_gen.has_previous() {
            let texture = match ctx.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => Some(t),
                _ => None,
            };
            if let Some(texture) = texture {
                self.profiler.cpu_begin("Frame generation: real frame");
                let view = texture.texture.create_view(&Default::default());
                let mut encoder = ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Frame generation: real frame"),
                });
                self.frame_gen.blit_previous(&mut encoder, &ctx.device, &view);
                if let Some(game_ui) = game_ui {
                    let mut frame = somnium_ui::GameUiFrame::new(window, &ctx.device, &ctx.queue, &mut encoder, &view,
                        ctx.config.format);
                    game_ui.draw_ui(&mut frame);
                }
                self.frame_gen.hold_real(texture);
                let now = self.frame_gen.take_split(SPLIT_START);
                if now {
                    self.frame_gen.stamp_real(&mut encoder);
                }
                ctx.queue.submit([encoder.finish()]);
                if now {
                    self.frame_gen.present_real(&ctx.queue);
                }
                self.profiler.cpu_end();
            } else {
                tracing::warn!("frame generation: no swap chain image for the real frame");
            }
        }
        self.frame_gen.picture_target()
    }

    /// A split point: if the real frame is presented here, the work recorded so
    /// far is submitted ahead of its present and recording goes on in a new encoder.
    pub(super) fn frame_gen_split(&mut self, ctx: &RenderContext, encoder: &mut wgpu::CommandEncoder, point: usize) {
        if self.frame_gen.take_split(point) {
            self.frame_gen_present_real_now(ctx, encoder);
        }
    }

    fn frame_gen_present_real_now(&mut self, ctx: &RenderContext, encoder: &mut wgpu::CommandEncoder) {
        self.frame_gen.stamp_real(encoder);
        let before = std::mem::replace(encoder, ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Main Render Encoder (after the real frame)"),
        }));
        ctx.queue.submit([before.finish()]);
        self.frame_gen.present_real(&ctx.queue);
    }

    /// After the scene: the real frame if it is still waiting (split "after
    /// scene"), then the generated frame into a fresh swap chain image, returned
    /// for the UI to draw over and the frame to present. `None`: no image.
    pub(super) fn frame_gen_finish(&mut self, ctx: &RenderContext, encoder: &mut wgpu::CommandEncoder)
        -> Option<(wgpu::SurfaceTexture, wgpu::TextureView)> {
        if self.frame_gen.take_split(crate::pass::frame_gen::SPLIT_AFTER_SCENE) {
            self.frame_gen_present_real_now(ctx, encoder);
        }
        let output = match ctx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => {
                tracing::warn!("Failed to acquire surface texture");
                return None;
            }
        };
        let view = output.texture.create_view(&Default::default());
        self.profiler.cpu_begin("Frame generation");
        self.profiler.begin(encoder, "Frame generation");
        self.frame_gen.generate(&ctx.device, &ctx.queue, encoder, &view, &self.vis_pass.depth_texture,
            self.velocity_pass.texture(), [self.render_width, self.render_height], self.view_matrix, self.proj_matrix);
        self.profiler.end(encoder);
        self.profiler.cpu_end();
        Some((output, view))
    }
}
