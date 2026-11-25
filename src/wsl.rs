//! The WSL distributions installed on Windows, offered as profiles.

use std::sync::{Once, OnceLock};

/// The names `wsl.exe -l -q` lists, asked once per process. Empty off
/// Windows, without WSL, or when `wsl.exe` fails.
static DISTROS: OnceLock<Vec<String>> = OnceLock::new();
static STARTED: Once = Once::new();

/// The distributions if already listed, else empty while a background
/// thread asks `wsl.exe`, which has no timeout and can take seconds to
/// start WSL; the UI thread must not wait on it.
pub fn distros() -> &'static [String] {
    in_background(&DISTROS, &STARTED, list)
}

/// The distributions, waiting for the background `wsl.exe` if they are not
/// listed yet. For a profile name the config does not have, which may be one
/// of them.
pub fn distros_wait() -> &'static [String] {
    distros();
    DISTROS.wait()
}

fn in_background(
    cell: &'static OnceLock<Vec<String>>,
    started: &'static Once,
    list: fn() -> Vec<String>,
) -> &'static [String] {
    if let Some(names) = cell.get() {
        return names;
    }
    started.call_once(|| {
        std::thread::spawn(move || {
            cell.get_or_init(list);
        });
    });
    &[]
}

#[cfg(windows)]
fn list() -> Vec<String> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    // A GUI-subsystem process would otherwise flash a console window.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = Command::new("wsl.exe")
        .args(["-l", "-q"])
        // WSL_UTF8=1 switches the output to UTF-8; removing it keeps the
        // one encoding `parse_list` reads.
        .env_remove("WSL_UTF8")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match output {
        Ok(output) if output.status.success() => parse_list(&output.stdout),
        _ => Vec::new(),
    }
}

#[cfg(not(windows))]
fn list() -> Vec<String> {
    Vec::new()
}

/// Reads `wsl.exe -l -q` output: UTF-16LE, maybe with a BOM, one name per
/// CRLF-terminated line.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_list(bytes: &[u8]) -> Vec<String> {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| u16::from_le_bytes(pair))
        .collect();
    String::from_utf16_lossy(&units)
        .lines()
        .map(|line| line.trim_matches(|c: char| c == '\u{feff}' || c == '\0' || c.is_whitespace()))
        // Docker Desktop's own distributions run its engine, not a shell
        // the user would open; Windows Terminal hides them too.
        .filter(|name| {
            !name.is_empty() && !matches!(*name, "docker-desktop" | "docker-desktop-data")
        })
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16le(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn the_list_is_empty_until_the_background_lookup_finishes() {
        static CELL: OnceLock<Vec<String>> = OnceLock::new();
        static STARTED: Once = Once::new();
        fn slow() -> Vec<String> {
            std::thread::sleep(std::time::Duration::from_millis(200));
            vec!["Ubuntu".to_owned()]
        }
        assert!(in_background(&CELL, &STARTED, slow).is_empty());
        assert!(in_background(&CELL, &STARTED, slow).is_empty());
        while CELL.get().is_none() {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(in_background(&CELL, &STARTED, slow), ["Ubuntu"]);
    }

    #[test]
    fn lists_one_distribution_per_line() {
        let bytes = utf16le("Ubuntu\r\nDebian\r\n");
        assert_eq!(parse_list(&bytes), ["Ubuntu", "Debian"]);
    }

    #[test]
    fn ignores_bom_blank_lines_and_nul_padding() {
        let bytes = utf16le("\u{feff}Ubuntu-22.04\r\n\r\nkali-linux\0\r\n");
        assert_eq!(parse_list(&bytes), ["Ubuntu-22.04", "kali-linux"]);
    }

    #[test]
    fn keeps_non_ascii_names() {
        assert_eq!(parse_list(&utf16le("開発用\r\n")), ["開発用"]);
    }

    #[test]
    fn hides_docker_desktop_distributions() {
        let bytes = utf16le("docker-desktop\r\nUbuntu\r\ndocker-desktop-data\r\n");
        assert_eq!(parse_list(&bytes), ["Ubuntu"]);
    }

    #[test]
    fn nothing_installed_or_odd_trailing_byte_is_handled() {
        assert!(parse_list(b"").is_empty());
        let mut bytes = utf16le("Ubuntu\r\n");
        bytes.push(0);
        assert_eq!(parse_list(&bytes), ["Ubuntu"]);
    }
}
