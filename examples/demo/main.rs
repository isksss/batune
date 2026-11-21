//! Opens a window and draws a fixed frame.
//!
//! `cargo run -p batune-render --example demo -- [block|underline|bar|hollow] [font-size]`

mod frame;

use std::sync::Arc;

use batune::render::{CursorShape, FontConfig, Renderer};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

struct Gpu {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
}

struct App {
    cursor: CursorShape,
    font_size: f32,
    gpu: Option<Gpu>,
}

impl App {
    fn init(&self, event_loop: &ActiveEventLoop) -> Gpu {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("batune-render demo")
                        .with_inner_size(LogicalSize::new(760.0, 280.0)),
                )
                .expect("create window"),
        );
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no GPU adapter");
        eprintln!("adapter: {:?}", adapter.get_info());
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).expect("device");
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("surface unsupported by adapter");
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);
        let mut renderer = Renderer::new(
            &device,
            config.format,
            FontConfig {
                size: self.font_size,
                ..Default::default()
            },
            window.scale_factor(),
        );
        renderer.set_padding(8.0);
        Gpu {
            window,
            surface,
            device,
            queue,
            config,
            renderer,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_none() {
            self.gpu = Some(self.init(event_loop));
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                gpu.config.width = size.width.max(1);
                gpu.config.height = size.height.max(1);
                gpu.surface.configure(&gpu.device, &gpu.config);
                gpu.window.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                gpu.renderer.set_scale_factor(scale_factor);
                gpu.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let texture = match gpu.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(t)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
                    other => {
                        eprintln!("surface: {other:?}");
                        gpu.surface.configure(&gpu.device, &gpu.config);
                        gpu.window.request_redraw();
                        return;
                    }
                };
                let size = (gpu.config.width, gpu.config.height);
                let (cols, rows) = gpu.renderer.grid_size(size.0, size.1);
                let frame = frame::demo_frame(cols, rows, self.cursor);
                let view = texture.texture.create_view(&Default::default());
                gpu.renderer
                    .render(&gpu.device, &gpu.queue, &view, &frame, size);
                gpu.window.pre_present_notify();
                gpu.queue.present(texture);
            }
            _ => {}
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let cursor = match args.next().as_deref() {
        None | Some("block") => CursorShape::Block,
        Some("underline") => CursorShape::Underline,
        Some("bar") => CursorShape::Bar,
        Some("hollow") => CursorShape::HollowBlock,
        Some(other) => panic!("unknown cursor shape {other:?}"),
    };
    let font_size = args.next().map_or(16.0, |s| s.parse().expect("font size"));
    let event_loop = EventLoop::new().expect("event loop");
    event_loop
        .run_app(&mut App {
            cursor,
            font_size,
            gpu: None,
        })
        .expect("run");
}
