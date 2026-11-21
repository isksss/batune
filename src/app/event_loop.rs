//! winit's callbacks: timers, user events and window events.

use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::window::WindowId;

use super::{App, EXIT_POLL, UserEvent};

impl ApplicationHandler<UserEvent> for App {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        let StartCause::ResumeTimeReached { .. } = cause else {
            return;
        };
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let now = Instant::now();
        if std::mem::take(&mut active.redraw_later) {
            active.window.request_redraw();
        }
        if active.bell.is_some_and(|end| now >= end) {
            active.bell = None;
            active.window.request_redraw();
        }
        let timing = active.blink_timing(&self.config);
        if active.blink.update(now, timing) {
            active.window.request_redraw();
        }
        if now >= active.exit_poll {
            active.exit_poll = now + EXIT_POLL;
            active.close_exited(&self.config);
        }
        self.exit_if_empty(event_loop);
    }

    /// Wakes for whichever comes first: the exit poll, the end of the
    /// bell's flash, or the next blink.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        let now = Instant::now();
        let timing = active.blink_timing(&self.config);
        let wake = [active.bell, active.blink.deadline(now, timing)]
            .into_iter()
            .flatten()
            .fold(active.exit_poll, Instant::min);
        event_loop.set_control_flow(ControlFlow::WaitUntil(wake));
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.active.is_none() {
            self.active = Some(self.open(event_loop));
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        let id = match event {
            UserEvent::ConfigChanged => return self.reload_config(),
            UserEvent::Output(id) => id,
        };
        let Some(active) = self.active.as_mut() else {
            return;
        };
        // The tab may have been closed since the wake was posted.
        let Some(index) = active.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let taken = active.tabs[index].output.take();
        if !taken.bytes.is_empty() {
            active.on_output(index, &taken.bytes, &self.config);
        }
        if taken.closed {
            active.close_tab(index, &self.config);
        }
        self.exit_if_empty(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let typing = active.settings.is_some();
        let ui_redraw = active.ui.on_window_event(&active.window, &event, typing);
        if ui_redraw && active.settings.is_some() {
            active.window.request_redraw();
        }
        let mut clicked = None;
        match event {
            WindowEvent::CloseRequested => {
                for index in (0..active.tabs.len()).rev() {
                    active.close_tab(index, &self.config);
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => active.set_modifiers(modifiers.state()),
            WindowEvent::KeyboardInput { event, .. } if !typing => {
                clicked = active.on_key(&event, &self.config);
            }
            WindowEvent::CursorMoved { position, .. } => {
                // An open menu hangs over the grid; it is the bar's too. The
                // settings screen takes the whole window, so a click beside
                // it does not start a selection behind it.
                let over_bar = position.y < f64::from(active.gpu.top())
                    || active.ui.menu_open()
                    || active.settings.is_some();
                // egui asks for a redraw on every move; the grid is redrawn
                // with it, so only moves that can change the bar's hover.
                if ui_redraw && (over_bar || active.over_bar) {
                    active.window.request_redraw();
                }
                active.over_bar = over_bar;
                // Still passed on while over the bar, so a drag that leaves
                // the grid keeps extending the selection.
                active.on_cursor_moved(position);
            }
            WindowEvent::CursorLeft { .. } => active.over_bar = false,
            // Presses and the wheel over the bar are egui's alone. Releases
            // always reach the terminal, to end a drag begun on the grid.
            WindowEvent::MouseInput { state, button, .. } => {
                if ui_redraw {
                    active.window.request_redraw();
                }
                if !(active.over_bar && state == ElementState::Pressed) {
                    active.on_mouse_button(state, button, &self.config);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if !active.over_bar {
                    active.on_wheel(delta, &self.config);
                }
            }
            WindowEvent::Ime(event) if !typing => active.on_ime(event, &self.config),
            // Minimizing on Windows reports 0x0; fitting the grid to it would
            // truncate every tab to one cell, and there is no reflow back.
            WindowEvent::Resized(size) if size.width == 0 || size.height == 0 => {}
            WindowEvent::Resized(size) => {
                active.gpu.resize(size);
                active.fit_grid();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                active.gpu.set_scale_factor(scale_factor);
                active.fit_grid();
            }
            WindowEvent::Occluded(false) => active.window.request_redraw(),
            WindowEvent::Focused(focused) => {
                active.focused = focused;
                active
                    .blink
                    .reset(Instant::now(), active.blink_timing(&self.config));
                active.window.request_redraw();
            }
            WindowEvent::RedrawRequested => clicked = active.redraw(&self.palette, &self.config),
            _ => {}
        }
        if let Some(clicked) = clicked {
            self.on_clicked(clicked);
        }
        self.exit_if_empty(event_loop);
    }
}
