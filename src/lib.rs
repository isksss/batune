//! `batune`: a native terminal window running the local shell, or
//! a remote one with `ssh user@host[:port]`.

pub mod pty;
pub mod render;
pub mod vt;

mod app;
mod bindings;
mod blink;
mod chrome;
mod clipboard;
mod config;
mod config_edit;
mod gpu;
mod icon;
mod ime;
mod input;
mod links;
mod mouse;
mod output;
mod palette;
mod search;
mod selection;
mod settings;
mod ssh;
mod tab_bar;
mod theme;
mod ui;
mod update;
mod watch;
mod wsl;

use std::path::Path;

use winit::event_loop::EventLoop;

use crate::app::Launch;
use crate::config::Config;

const USAGE: &str =
    "usage: batune [--profile <name> | ssh [user@]host[:port] | --print-default-config]";

/// Parses the command line and runs the terminal window until it closes.
pub fn run() {
    #[cfg(all(windows, not(debug_assertions)))]
    report_panics_in_message_box();
    #[cfg(windows)]
    update::remove_old();

    let args: Vec<String> = std::env::args().skip(1).collect();
    // `None` is a local session; which profile needs the config to tell.
    let (ssh, profile) = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => (None, None),
        ["--profile", name] => (None, Some(name)),
        ["ssh", target] => match target.parse() {
            Ok(target) => (Some(target), None),
            Err(e) => {
                eprintln!("batune: {e}");
                std::process::exit(2);
            }
        },
        ["--print-default-config"] => {
            print!("{}", config::DEFAULT_CONFIG);
            return;
        }
        ["-h" | "--help"] => {
            println!("{USAGE}");
            return;
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };

    let path = config::path();
    let config = path.as_deref().map(load_config).unwrap_or_default();
    let launch = match ssh {
        Some(target) => {
            // The local branch starts this through `startup_profile`; without
            // it the first new-tab menu would wait on `wsl.exe`.
            crate::wsl::distros();
            Launch::Ssh(target)
        }
        None => match config.startup_profile(profile) {
            Ok(profile) => Launch::Local(profile),
            Err(e) => {
                eprintln!("batune: --profile: {e}");
                std::process::exit(2);
            }
        },
    };
    let event_loop = EventLoop::with_user_event()
        .build()
        .expect("create event loop");
    let mut app = app::App::new(event_loop.create_proxy(), path, config, launch);
    event_loop.run_app(&mut app).expect("run event loop");
}

/// Shows a panic (e.g. no GPU adapter, shell failed to start) in a message box,
/// since a GUI-subsystem process has no stderr to print it to.
// Not `windows-sys`: one `MessageBoxW` from user32, which winit already links,
// does not warrant a new dependency.
#[cfg(all(windows, not(debug_assertions)))]
fn report_panics_in_message_box() {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(
            hwnd: *mut std::ffi::c_void,
            text: *const u16,
            caption: *const u16,
            kind: u32,
        ) -> i32;
    }
    const MB_ICONERROR: u32 = 0x10;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    std::panic::set_hook(Box::new(|info| {
        let text = wide(&info.to_string());
        let caption = wide("batune");
        // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call.
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                caption.as_ptr(),
                MB_ICONERROR,
            );
        }
    }));
}

/// A broken file must not keep the terminal from opening, so errors fall
/// back to the defaults.
fn load_config(path: &Path) -> Config {
    config::load(path).unwrap_or_else(|e| {
        eprintln!("batune: {}: {e}; using the defaults", path.display());
        Config::default()
    })
}
