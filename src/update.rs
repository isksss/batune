//! Replaces the running executable with the latest GitHub release.

use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha256};

const REPO: &str = "https://github.com/isksss/batune";
const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// ureq reads at most 10 MB by default, less than the archives.
const MAX_DOWNLOAD: u64 = 256 * 1024 * 1024;

pub enum Outcome {
    UpToDate(String),
    Updated(String),
}

/// Whether this copy may update itself; the Store updates its own package.
pub fn available() -> bool {
    !packaged()
}

/// Asks GitHub for the latest release and installs it if it is newer.
/// The new version runs from the next start.
pub fn run() -> Result<Outcome, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(300)))
        .build()
        .into();
    let latest = latest_version(&agent)?;
    if !is_newer(&latest, CURRENT) {
        return Ok(Outcome::UpToDate(CURRENT.to_owned()));
    }
    let (archive_name, inner) = asset(&latest, cfg!(windows));
    let base = format!("{REPO}/releases/download/v{latest}");
    let sums = String::from_utf8(download(&agent, &format!("{base}/SHA256SUMS"))?)
        .map_err(|_| "SHA256SUMS が読めません".to_owned())?;
    let expected = expected_sum(&sums, &archive_name)
        .ok_or(format!("SHA256SUMS に {archive_name} がありません"))?;
    let archive = download(&agent, &format!("{base}/{archive_name}"))?;
    if !hex(&Sha256::digest(&archive)).eq_ignore_ascii_case(expected) {
        return Err(format!(
            "{archive_name} のハッシュが SHA256SUMS と合いません"
        ));
    }
    let exe = if cfg!(windows) {
        from_zip(&archive, &inner)
    } else {
        from_tar_gz(&archive, &inner)
    }
    .ok_or(format!("{archive_name} に {inner} がありません"))?;
    replace(&exe)?;
    Ok(Outcome::Updated(latest))
}

/// `/releases/latest` redirects to the tag's page, which saves parsing the
/// API's JSON (no JSON parser among the dependencies). It skips prereleases.
fn latest_version(agent: &ureq::Agent) -> Result<String, String> {
    use ureq::ResponseExt;
    let response = agent
        .get(&format!("{REPO}/releases/latest"))
        .call()
        .map_err(|e| format!("最新リリースを確認できません: {e}"))?;
    version_from_url(&response.get_uri().to_string())
        .map(str::to_owned)
        .ok_or("リリースが見つかりません".to_owned())
}

fn download(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, String> {
    agent
        .get(url)
        .call()
        .and_then(|r| {
            r.into_body()
                .with_config()
                .limit(MAX_DOWNLOAD)
                .read_to_vec()
        })
        .map_err(|e| format!("{url} を取得できません: {e}"))
}

fn version_from_url(url: &str) -> Option<&str> {
    url.rsplit_once("/releases/tag/v")
        .map(|(_, v)| v)
        .filter(|v| !v.is_empty())
}

/// Semver order without build metadata: a prerelease sorts before its
/// release; two prereleases of one version are treated as equal.
fn is_newer(latest: &str, current: &str) -> bool {
    fn parse(v: &str) -> Option<([u64; 3], bool)> {
        let (core, pre) = match v.split_once('-') {
            Some((core, _)) => (core, true),
            None => (v, false),
        };
        let mut parts = core.split('.').map(|p| p.parse().ok());
        let triple = [parts.next()??, parts.next()??, parts.next()??];
        parts.next().is_none().then_some((triple, pre))
    }
    match (parse(latest), parse(current)) {
        (Some((l, l_pre)), Some((c, c_pre))) => l > c || (l == c && c_pre && !l_pre),
        _ => false,
    }
}

/// The archive `release.yml` makes for this platform and the executable's
/// path inside it.
fn asset(version: &str, windows: bool) -> (String, String) {
    let (os, ext, exe) = if windows {
        ("windows", "zip", "batune.exe")
    } else {
        ("linux", "tar.gz", "batune")
    };
    let stem = format!("batune-v{version}-x86_64-{os}");
    (format!("{stem}.{ext}"), format!("{stem}/{exe}"))
}

/// `sha256sum` output: `<hex>  <name>`, or `<hex> *<name>` in binary mode.
fn expected_sum<'a>(sums: &'a str, name: &str) -> Option<&'a str> {
    sums.lines().find_map(|line| {
        let (sum, file) = line.split_once(' ')?;
        (file.trim_start_matches([' ', '*']) == name && sum.len() == 64).then_some(sum)
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Only plain ustar/GNU entries with names under 100 bytes, which is all
/// `release.yml` makes; the `tar` crate would bring filetime and xattr for
/// unpacking to disk, which this does not do.
fn from_tar_gz(archive: &[u8], path: &str) -> Option<Vec<u8>> {
    let mut tar = Vec::new();
    flate2::read::GzDecoder::new(archive)
        .read_to_end(&mut tar)
        .ok()?;
    let mut rest = &tar[..];
    while rest.len() >= 512 && rest[..512].iter().any(|&b| b != 0) {
        let header = &rest[..512];
        let name = header[..100].split(|&b| b == 0).next()?;
        let size = std::str::from_utf8(&header[124..136]).ok()?;
        let size = usize::from_str_radix(size.trim_matches(['\0', ' ']), 8).ok()?;
        let data = rest.get(512..512 + size)?;
        if name == path.as_bytes() && matches!(header[156], b'0' | b'\0') {
            return Some(data.to_vec());
        }
        rest = rest.get(512 + size.div_ceil(512) * 512..)?;
    }
    None
}

/// Reads the central directory, not the local headers, which may leave
/// the sizes to a trailing data descriptor. No zip64: the archive is far
/// under 4 GB. The `zip` crate would be a dozen crates for this.
fn from_zip(archive: &[u8], path: &str) -> Option<Vec<u8>> {
    let u16_at =
        |at: usize| Some(u16::from_le_bytes(archive.get(at..at + 2)?.try_into().ok()?) as usize);
    let u32_at =
        |at: usize| Some(u32::from_le_bytes(archive.get(at..at + 4)?.try_into().ok()?) as usize);
    let end = (0..archive.len().saturating_sub(21))
        .rev()
        .find(|&at| archive[at..].starts_with(b"PK\x05\x06"))?;
    let mut at = u32_at(end + 16)?;
    for _ in 0..u16_at(end + 10)? {
        if !archive.get(at..)?.starts_with(b"PK\x01\x02") {
            return None;
        }
        let name_len = u16_at(at + 28)?;
        let next = at + 46 + name_len + u16_at(at + 30)? + u16_at(at + 32)?;
        if archive.get(at + 46..at + 46 + name_len)? == path.as_bytes() {
            let method = u16_at(at + 10)?;
            let size = u32_at(at + 20)?;
            let local = u32_at(at + 42)?;
            let start = local + 30 + u16_at(local + 26)? + u16_at(local + 28)?;
            let data = archive.get(start..start + size)?;
            return match method {
                0 => Some(data.to_vec()),
                8 => {
                    let mut out = Vec::new();
                    flate2::read::DeflateDecoder::new(data)
                        .read_to_end(&mut out)
                        .ok()?;
                    Some(out)
                }
                _ => None,
            };
        }
        at = next;
    }
    None
}

/// Writes beside the executable and renames over it, so the running file
/// is never written to and a failure leaves it in place.
fn replace(new: &[u8]) -> Result<(), String> {
    let exe =
        std::env::current_exe().map_err(|e| format!("実行ファイルの場所が分かりません: {e}"))?;
    let dir = exe.parent().ok_or("実行ファイルの場所が分かりません")?;
    let tmp = dir.join(format!(".batune-update-{}", std::process::id()));
    let result = write_executable(&tmp, &exe, new).and_then(|()| swap(&tmp, &exe));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|e| format!("{} を置き換えられません: {e}", exe.display()))
}

fn write_executable(tmp: &Path, exe: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = fs::File::create(tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::set_permissions(tmp, fs::metadata(exe)?.permissions())
}

#[cfg(not(windows))]
fn swap(tmp: &Path, exe: &Path) -> std::io::Result<()> {
    fs::rename(tmp, exe)
}

/// Windows cannot replace or delete a running executable, but can rename it.
#[cfg(windows)]
fn swap(tmp: &Path, exe: &Path) -> std::io::Result<()> {
    let old = exe.with_extension("exe.old");
    let _ = fs::remove_file(&old);
    fs::rename(exe, &old)?;
    fs::rename(tmp, exe).inspect_err(|_| {
        let _ = fs::rename(&old, exe);
    })
}

/// Deletes the executable an earlier update moved aside, which could not be
/// deleted while it was running.
#[cfg(windows)]
pub fn remove_old() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = fs::remove_file(exe.with_extension("exe.old"));
    }
}

/// A process started from an MSIX package has a package identity; the
/// install directory is not checked instead, since it can be moved.
#[cfg(windows)]
fn packaged() -> bool {
    use windows_sys::Win32::Foundation::APPMODEL_ERROR_NO_PACKAGE;
    use windows_sys::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
    let mut len = 0;
    // SAFETY: a zero length with a null buffer only asks for the length.
    unsafe {
        GetCurrentPackageFullName(&mut len, std::ptr::null_mut()) != APPMODEL_ERROR_NO_PACKAGE
    }
}

#[cfg(not(windows))]
fn packaged() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_compares_numbers_not_text() {
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(is_newer("0.2.1", "0.2.0"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
    }

    #[test]
    fn a_release_is_newer_than_its_prerelease() {
        assert!(is_newer("0.3.0", "0.3.0-rc.1"));
        assert!(!is_newer("0.3.0-rc.1", "0.3.0"));
        assert!(!is_newer("0.3.0-rc.2", "0.3.0-rc.1"));
    }

    #[test]
    fn malformed_versions_are_never_newer() {
        assert!(!is_newer("", "0.2.0"));
        assert!(!is_newer("1.0", "0.2.0"));
        assert!(!is_newer("1.0.0.0", "0.2.0"));
        assert!(!is_newer("x.y.z", "0.2.0"));
    }

    #[test]
    fn reads_the_version_from_the_redirected_tag_url() {
        let url = "https://github.com/isksss/batune/releases/tag/v0.2.0";
        assert_eq!(version_from_url(url), Some("0.2.0"));
        // No release yet: GitHub stays on the releases list.
        assert_eq!(
            version_from_url("https://github.com/isksss/batune/releases"),
            None
        );
    }

    #[test]
    fn names_the_assets_release_yml_makes() {
        assert_eq!(
            asset("0.3.0", false),
            (
                "batune-v0.3.0-x86_64-linux.tar.gz".to_owned(),
                "batune-v0.3.0-x86_64-linux/batune".to_owned()
            )
        );
        assert_eq!(
            asset("0.3.0", true),
            (
                "batune-v0.3.0-x86_64-windows.zip".to_owned(),
                "batune-v0.3.0-x86_64-windows/batune.exe".to_owned()
            )
        );
    }

    #[test]
    fn finds_the_sum_for_one_file_in_either_mode() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let sums = format!("{a}  x.tar.gz\n{b} *x.zip\n");
        assert_eq!(expected_sum(&sums, "x.tar.gz"), Some(a.as_str()));
        assert_eq!(expected_sum(&sums, "x.zip"), Some(b.as_str()));
        assert_eq!(expected_sum(&sums, "x.tar"), None);
        assert_eq!(expected_sum("short  x.zip\n", "x.zip"), None);
    }

    #[test]
    fn hashes_to_lowercase_hex() {
        assert_eq!(
            hex(&Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    fn tar_entry(name: &str, data: &[u8], kind: u8) -> Vec<u8> {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        header[156] = kind;
        let mut entry = header.to_vec();
        entry.extend_from_slice(data);
        entry.resize(512 + data.len().div_ceil(512) * 512, 0);
        entry
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn takes_one_file_out_of_a_tar_gz() {
        let mut tar = tar_entry("d/", b"", b'5');
        tar.extend(tar_entry("d/README.md", &[b'r'; 600], b'0'));
        tar.extend(tar_entry("d/batune", b"\x7fELF", b'0'));
        tar.extend([0; 1024]);
        let archive = gzip(&tar);
        assert_eq!(
            from_tar_gz(&archive, "d/batune").as_deref(),
            Some(&b"\x7fELF"[..])
        );
        assert_eq!(from_tar_gz(&archive, "d/missing"), None);
        assert_eq!(from_tar_gz(b"not gzip", "d/batune"), None);
    }

    /// A zip with one entry per (name, method, stored bytes, original bytes).
    fn zip(entries: &[(&str, u16, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for &(name, method, data) in entries {
            let offset = out.len() as u32;
            out.extend(b"PK\x03\x04");
            out.extend([0; 22]);
            out.extend((name.len() as u16).to_le_bytes());
            out.extend(0u16.to_le_bytes());
            out.extend(name.as_bytes());
            out.extend(data);
            central.extend(b"PK\x01\x02");
            central.extend([0; 6]);
            central.extend(method.to_le_bytes());
            central.extend([0; 8]);
            central.extend((data.len() as u32).to_le_bytes());
            central.extend([0; 4]);
            central.extend((name.len() as u16).to_le_bytes());
            central.extend([0; 12]);
            central.extend(offset.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let central_at = out.len() as u32;
        out.extend(&central);
        out.extend(b"PK\x05\x06");
        out.extend([0; 6]);
        out.extend((entries.len() as u16).to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(central_at.to_le_bytes());
        out.extend([0; 2]);
        out
    }

    #[test]
    fn takes_one_file_out_of_a_zip() {
        let mut deflated =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
        deflated.write_all(b"MZ deflated").unwrap();
        let deflated = deflated.finish().unwrap();
        let archive = zip(&[
            ("d/README.md", 0, b"readme"),
            ("d/batune.exe", 8, &deflated),
            ("d/stored.exe", 0, b"MZ stored"),
        ]);
        assert_eq!(
            from_zip(&archive, "d/batune.exe").as_deref(),
            Some(&b"MZ deflated"[..])
        );
        assert_eq!(
            from_zip(&archive, "d/stored.exe").as_deref(),
            Some(&b"MZ stored"[..])
        );
        assert_eq!(from_zip(&archive, "d/missing.exe"), None);
        assert_eq!(from_zip(b"PK", "d/batune.exe"), None);
    }

    #[test]
    fn replacing_leaves_the_old_file_when_the_new_one_cannot_be_written() {
        let dir = std::env::temp_dir().join(format!("batune-update-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("batune");
        fs::write(&exe, b"old").unwrap();
        // A directory where the temporary file should go makes the write fail.
        let tmp = dir.join("tmp-dir");
        fs::create_dir_all(&tmp).unwrap();
        assert!(
            write_executable(&tmp, &exe, b"new")
                .and_then(|()| swap(&tmp, &exe))
                .is_err()
        );
        assert_eq!(fs::read(&exe).unwrap(), b"old");

        let tmp = dir.join("tmp-file");
        write_executable(&tmp, &exe, b"new").unwrap();
        swap(&tmp, &exe).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"new");
        assert!(!tmp.exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
