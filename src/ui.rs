//! egui drawn over the window: the tab bar, and screens such as settings
//! that are not part of the terminal grid.

use std::sync::Arc;

use egui::{ClippedPrimitive, Color32, Context, FontData, FontDefinitions, FontFamily, ViewportId};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::window::Window;

use crate::palette::Palette;
use crate::render::{Color, builtin_font};

pub struct Ui {
    ctx: Context,
    state: egui_winit::State,
}

/// One run's output, ready for [`crate::gpu::Gpu::render`] to paint.
pub struct Painted {
    pub jobs: Vec<ClippedPrimitive>,
    pub textures: egui::TexturesDelta,
    pub pixels_per_point: f32,
}

impl Ui {
    pub fn new(window: &Window, palette: &Palette) -> Self {
        let ctx = Context::default();
        // egui is built without its own fonts; the terminal's built-in one
        // also covers Japanese titles.
        let mut fonts = FontDefinitions::empty();
        fonts.font_data.insert(
            "builtin".to_owned(),
            Arc::new(FontData::from_static(builtin_font())),
        );
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push("builtin".to_owned());
        }
        ctx.set_fonts(fonts);
        let state = egui_winit::State::new(
            ctx.clone(),
            ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let ui = Self { ctx, state };
        ui.set_palette(palette);
        ui
    }

    /// Colors the widgets after the terminal.
    pub fn set_palette(&self, palette: &Palette) {
        let (fg, bg) = (palette.foreground, palette.background);
        let luma =
            |c: Color| 0.299 * f32::from(c.r) + 0.587 * f32::from(c.g) + 0.114 * f32::from(c.b);
        let mut visuals = if luma(bg) < 128.0 {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        visuals.panel_fill = color32(bg.mix(fg, 0.08));
        visuals.override_text_color = Some(color32(fg));
        visuals.selection.bg_fill = color32(bg);
        visuals.widgets.hovered.weak_bg_fill = color32(bg.mix(fg, 0.15));
        self.ctx.set_visuals(visuals);
    }

    /// Passes a window event on; `true` when egui wants to be redrawn.
    /// Keys and the IME go to egui only while `typing` (a screen with text
    /// fields is open); otherwise egui would take Tab for its focus or
    /// swallow the terminal's text.
    pub fn on_window_event(&mut self, window: &Window, event: &WindowEvent, typing: bool) -> bool {
        match event {
            WindowEvent::KeyboardInput { .. } | WindowEvent::Ime(_) if !typing => false,
            _ => self.state.on_window_event(window, event).repaint,
        }
    }

    /// Whether a menu is open, which may reach below the bar.
    pub fn menu_open(&self) -> bool {
        egui::Popup::is_any_open(&self.ctx)
    }

    /// Lays out one frame with `build`. The second value is whether egui
    /// asks for another frame at once (an animation in progress).
    pub fn run(
        &mut self,
        window: &Window,
        typing: bool,
        build: impl FnMut(&mut egui::Ui),
    ) -> (Painted, bool) {
        let input = self.state.take_egui_input(window);
        let mut out = self.ctx.run_ui(input, build);
        if typing {
            let mut platform = std::mem::take(&mut out.platform_output);
            // The IME stays on; only its place follows the focused field.
            // Letting egui turn it off and on again would leave X11 (winit
            // 0.30) with a new input context that never gets focus, so
            // nothing could be composed in the field.
            if let Some(ime) = platform.ime.take() {
                let rect = ime.cursor_rect * out.pixels_per_point;
                window.set_ime_cursor_area(
                    PhysicalPosition::new(rect.min.x, rect.min.y),
                    PhysicalSize::new(rect.width(), rect.height()),
                );
            }
            self.state.handle_platform_output(window, platform);
        }
        let again = out
            .viewport_output
            .get(&ViewportId::ROOT)
            .is_some_and(|v| v.repaint_delay.is_zero());
        let painted = Painted {
            jobs: self.ctx.tessellate(out.shapes, out.pixels_per_point),
            textures: out.textures_delta,
            pixels_per_point: out.pixels_per_point,
        };
        (painted, again)
    }
}

fn color32(c: Color) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}
