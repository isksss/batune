use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::pty::Command;

/// The user's interactive shell with the terminal environment set up:
/// `$SHELL` (falling back to `/bin/sh`) on Unix, and the first of
/// `pwsh.exe`, `powershell.exe`, `cmd.exe` found on `PATH` on Windows.
pub fn default_shell() -> Command {
    let program = shell_program(
        std::env::var_os("SHELL"),
        std::env::var_os("PATH"),
        cfg!(windows),
    );
    with_terminal_env(Command::new(program))
}

/// Sets the variables programs use to detect what the terminal supports.
pub fn with_terminal_env(command: Command) -> Command {
    command
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
}

fn shell_program(shell: Option<OsString>, path: Option<OsString>, windows: bool) -> OsString {
    if windows {
        // `$SHELL` is ignored here: when set on Windows it usually comes from
        // MSYS/Git Bash and names a Unix path ConPTY cannot start.
        let candidates = ["pwsh.exe", "powershell.exe"];
        candidates
            .iter()
            .find_map(|name| find_in_path(name, path.as_deref()))
            .map(PathBuf::into_os_string)
            // cmd.exe ships with every Windows, so it is the last resort even
            // when PATH is broken.
            .unwrap_or_else(|| "cmd.exe".into())
    } else {
        shell
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/bin/sh".into())
    }
}

fn find_in_path(name: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    std::env::split_paths(path?)
        .map(|dir| dir.join(name))
        .find(|candidate| is_file(candidate))
}

fn is_file(path: &Path) -> bool {
    path.metadata().is_ok_and(|m| m.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_uses_shell_env() {
        assert_eq!(
            shell_program(Some("/bin/zsh".into()), None, false),
            "/bin/zsh"
        );
    }

    #[test]
    fn unix_falls_back_to_bin_sh() {
        assert_eq!(shell_program(None, None, false), "/bin/sh");
        assert_eq!(shell_program(Some("".into()), None, false), "/bin/sh");
    }

    #[test]
    fn windows_prefers_pwsh_then_powershell_on_path() {
        let dir = std::env::temp_dir().join(format!("batune-pty-shell-{}", std::process::id()));
        let pwsh_dir = dir.join("pwsh");
        let ps_dir = dir.join("ps");
        std::fs::create_dir_all(&pwsh_dir).unwrap();
        std::fs::create_dir_all(&ps_dir).unwrap();
        std::fs::write(pwsh_dir.join("pwsh.exe"), "").unwrap();
        std::fs::write(ps_dir.join("powershell.exe"), "").unwrap();

        let both = std::env::join_paths([&ps_dir, &pwsh_dir]).unwrap();
        assert_eq!(
            shell_program(None, Some(both), true),
            pwsh_dir.join("pwsh.exe").into_os_string()
        );
        let only_ps = std::env::join_paths([&ps_dir]).unwrap();
        assert_eq!(
            shell_program(None, Some(only_ps), true),
            ps_dir.join("powershell.exe").into_os_string()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn windows_falls_back_to_cmd_and_ignores_shell_env() {
        assert_eq!(
            shell_program(Some("/usr/bin/bash".into()), None, true),
            "cmd.exe"
        );
    }

    #[test]
    fn terminal_env_advertises_256_colors_and_truecolor() {
        let env = with_terminal_env(Command::new("sh")).env;
        assert!(env.contains(&("TERM".into(), "xterm-256color".into())));
        assert!(env.contains(&("COLORTERM".into(), "truecolor".into())));
    }
}
