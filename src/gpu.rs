//! Surface, device and renderer for one window.

use std::sync::Arc;

use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::window::Window;

use crate::render::{BackgroundImage, FontConfig, Frame, Renderer};
use crate::tab_bar;
use crate::ui::Painted;

pub struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    /// Paints egui (the tab bar) over the grid.
    egui: egui_wgpu::Renderer,
    retries: Retries,
}

/// Immediate retries after `Lost`/`Outdated` before waiting instead; a
/// reconfigure that does not help would otherwise redraw in a tight loop.
const MAX_RETRIES: u32 = 3;

/// Frames in a row that could not get a texture right after a reconfigure.
#[derive(Debug, Default)]
struct Retries(u32);

impl Retries {
    /// After a frame was lost and the surface reconfigured.
    fn failed(&mut self) -> Rendered {
        self.0 = self.0.saturating_add(1);
        if self.0 > MAX_RETRIES {
            Rendered::RetryLater
        } else {
            Rendered::RetryNow
        }
    }

    /// Whether this failure is the first to give up on retrying at once; the
    /// log says so once instead of on every later attempt.
    fn just_gave_up(&self) -> bool {
        self.0 == MAX_RETRIES + 1
    }

    fn succeeded(&mut self) {
        self.0 = 0;
    }
}

/// What the caller should do after [`Gpu::render`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rendered {
    Presented,
    /// The surface was reconfigured; redraw at once.
    RetryNow,
    /// No frame now (timeout, or reconfiguring did not help); redraw after
    /// a while.
    RetryLater,
    /// The window is hidden; it is redrawn when it shows again.
    Hidden,
}

impl Gpu {
    /// `padding` is the space around the grid in logical pixels.
    pub fn new(window: Arc<Window>, font: FontConfig, padding: f32) -> Self {
        let instance = wgpu::Instance::default();
        let size = window.inner_size();
        let scale_factor = window.scale_factor();
        let surface = instance.create_surface(window).expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no GPU adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).expect("GPU device");
        let max = device.limits().max_texture_dimension_2d;
        let mut config = surface
            .get_default_config(
                &adapter,
                size.width.clamp(1, max),
                size.height.clamp(1, max),
            )
            .expect("surface unsupported by adapter");
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);
        let mut renderer = Renderer::new(&device, config.format, font, scale_factor);
        renderer.set_padding(padding);
        let egui = egui_wgpu::Renderer::new(&device, config.format, Default::default());
        Self {
            surface,
            device,
            queue,
            config,
            renderer,
            egui,
            retries: Retries::default(),
        }
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        // A surface wider than the device's texture limit fails validation.
        let max = self.device.limits().max_texture_dimension_2d;
        self.config.width = size.width.clamp(1, max);
        self.config.height = size.height.clamp(1, max);
        self.surface.configure(&self.device, &self.config);
    }

    pub fn set_font(&mut self, font: FontConfig) {
        self.renderer.set_font(font);
    }

    pub fn set_padding(&mut self, padding: f32) {
        self.renderer.set_padding(padding);
    }

    pub fn set_background_image(&mut self, image: Option<&BackgroundImage>) {
        self.renderer
            .set_background_image(&self.device, &self.queue, image);
    }

    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.renderer.set_scale_factor(scale_factor);
    }

    /// Physical pixels above the grid taken by the tab bar. At least one
    /// row of the surface is left to the grid.
    pub fn top(&self) -> u32 {
        let top = (f64::from(tab_bar::HEIGHT) * self.renderer.scale_factor()).round() as u32;
        top.min(self.config.height.saturating_sub(1))
    }

    /// Columns and rows that fit in the current surface.
    pub fn grid_size(&self) -> (u16, u16) {
        self.renderer
            .grid_size(self.config.width, self.config.height - self.top())
    }

    /// Cell (column, row) under a physical-pixel position; positions in the
    /// padding, the tab bar or outside the window clamp to the nearest cell.
    pub fn cell_at(&self, x: f64, y: f64, cols: u16, rows: u16) -> (u16, u16) {
        self.renderer
            .cell_at(x, y - f64::from(self.top()), cols, rows)
    }

    /// Surface size in physical pixels that holds exactly `cols` x `rows`.
    pub fn surface_size_for(&self, cols: u16, rows: u16) -> PhysicalSize<u32> {
        let m = self.renderer.cell_metrics();
        let pad = self.renderer.padding() * 2;
        PhysicalSize::new(
            m.width * u32::from(cols) + pad,
            m.height * u32::from(rows) + pad + self.top(),
        )
    }

    /// Size of one cell in physical pixels.
    pub fn cell_pixels(&self) -> (u32, u32) {
        let m = self.renderer.cell_metrics();
        (m.width, m.height)
    }

    /// Size of the grid area in physical pixels, for programs that ask the
    /// PTY for it.
    pub fn grid_pixels(&self, cols: u16, rows: u16) -> (u16, u16) {
        let m = self.renderer.cell_metrics();
        let clamp = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);
        (
            clamp(m.width * u32::from(cols)),
            clamp(m.height * u32::from(rows)),
        )
    }

    /// Top-left corner and size of the cell at (`col`, `row`) in physical
    /// pixels, relative to the window's inner area.
    pub fn cell_rect(&self, col: u16, row: u16) -> (PhysicalPosition<u32>, PhysicalSize<u32>) {
        let m = self.renderer.cell_metrics();
        let pad = self.renderer.padding();
        (
            PhysicalPosition::new(
                pad + m.width * u32::from(col),
                self.top() + pad + m.height * u32::from(row),
            ),
            PhysicalSize::new(m.width, m.height),
        )
    }

    /// Draws `frame` below the tab bar and `ui` over both, or says when to
    /// try again.
    pub fn render(&mut self, window: &Window, frame: &Frame, ui: &mut Painted) -> Rendered {
        // Before anything can fail: egui sends a texture's change only once,
        // so a skipped frame would leave the bar's glyphs missing for good.
        // A freed texture is not used by this frame's jobs.
        for (id, deltas) in &ui.textures.set {
            for delta in deltas {
                self.egui
                    .update_texture(&self.device, &self.queue, *id, delta);
            }
        }
        for id in &ui.textures.free {
            self.egui.free_texture(id);
        }
        ui.textures.clear();
        let texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Occluded => return Rendered::Hidden,
            failed
            @ (wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated) => {
                self.surface.configure(&self.device, &self.config);
                let next = self.retries.failed();
                if self.retries.just_gave_up() {
                    eprintln!("batune: surface: {failed:?}");
                }
                return next;
            }
            other => {
                eprintln!("batune: surface: {other:?}");
                return Rendered::RetryLater;
            }
        };
        self.retries.succeeded();
        let view = texture.texture.create_view(&Default::default());
        self.paint(&view, frame, ui);
        window.pre_present_notify();
        self.queue.present(texture);
        Rendered::Presented
    }

    /// One pass for both: the grid is drawn into a viewport below the bar,
    /// which moves it down without the render crate knowing about the bar.
    fn paint(&mut self, view: &wgpu::TextureView, frame: &Frame, ui: &Painted) {
        let (width, height) = (self.config.width, self.config.height);
        let top = self.top();
        self.renderer
            .prepare(&self.device, &self.queue, frame, (width, height - top));
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [width, height],
            pixels_per_point: ui.pixels_per_point,
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("batune"),
            });
        let callbacks =
            self.egui
                .update_buffers(&self.device, &self.queue, &mut encoder, &ui.jobs, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("batune"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(self.renderer.clear_color(frame.background)),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            pass.set_viewport(
                0.0,
                top as f32,
                width as f32,
                (height - top) as f32,
                0.0,
                1.0,
            );
            self.renderer.draw(&mut pass);
            pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);
            self.egui.render(&mut pass, &ui.jobs, &screen);
        }
        self.queue
            .submit(callbacks.into_iter().chain([encoder.finish()]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lost_frames_are_retried_at_once_only_a_few_times() {
        let mut retries = Retries::default();
        for _ in 0..MAX_RETRIES {
            assert_eq!(retries.failed(), Rendered::RetryNow);
        }
        assert_eq!(retries.failed(), Rendered::RetryLater);
        assert!(retries.just_gave_up());
        assert_eq!(retries.failed(), Rendered::RetryLater);
        assert!(!retries.just_gave_up());
    }

    #[test]
    fn a_presented_frame_allows_immediate_retries_again() {
        let mut retries = Retries::default();
        for _ in 0..=MAX_RETRIES {
            retries.failed();
        }
        retries.succeeded();
        assert_eq!(retries.failed(), Rendered::RetryNow);
    }
}
