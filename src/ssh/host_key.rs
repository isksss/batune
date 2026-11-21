//! Host key verification against OpenSSH's `known_hosts`.

use std::path::{Path, PathBuf};

use russh::keys::{self, Algorithm, HashAlg, PublicKey, known_hosts};

#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    Known,
    Unknown,
    /// The host has other keys recorded; the first is on `line` (1-based).
    /// Their algorithm need not match: a host that suddenly offers a new
    /// one is as suspicious as a changed key.
    Changed {
        line: usize,
    },
}

pub fn known_hosts_path() -> Option<PathBuf> {
    std::env::home_dir().map(|home| home.join(".ssh").join("known_hosts"))
}

pub fn check(path: &Path, host: &str, port: u16, key: &PublicKey) -> Result<Status, keys::Error> {
    match known_hosts::check_known_hosts_path(host, port, key, path) {
        Ok(true) => Ok(Status::Known),
        // russh reports `KeyChanged` only for a recorded key of the same
        // algorithm, so a key of another one would pass as a first visit.
        Ok(false) => Ok(match recorded(path, host, port)?.first() {
            Some((line, _)) => Status::Changed { line: *line },
            None => Status::Unknown,
        }),
        Err(keys::Error::KeyChanged { line }) => Ok(Status::Changed { line }),
        Err(e) => Err(e),
    }
}

/// Keys recorded for the host, with their 1-based lines.
pub fn recorded(
    path: &Path,
    host: &str,
    port: u16,
) -> Result<Vec<(usize, PublicKey)>, keys::Error> {
    known_hosts::known_host_keys_path(host, port, path)
}

/// `preferred` with the algorithms of `recorded` keys moved to the front,
/// as OpenSSH orders its host key algorithms, so that a host with several
/// keys presents one that can be checked rather than tripping
/// [`Status::Changed`].
pub fn prefer_recorded(preferred: &[Algorithm], recorded: &[PublicKey]) -> Vec<Algorithm> {
    let (mut known, other): (Vec<_>, Vec<_>) = preferred
        .iter()
        .cloned()
        .partition(|a| recorded.iter().any(|k| same_key_type(&k.algorithm(), a)));
    known.extend(other);
    known
}

/// RSA keys are recorded as `ssh-rsa` but negotiated with a hash
/// (`rsa-sha2-512`), so the hash is ignored.
fn same_key_type(a: &Algorithm, b: &Algorithm) -> bool {
    matches!((a, b), (Algorithm::Rsa { .. }, Algorithm::Rsa { .. })) || a == b
}

pub fn learn(path: &Path, host: &str, port: u16, key: &PublicKey) -> Result<(), keys::Error> {
    known_hosts::learn_known_hosts_path(host, port, key, path)
}

/// `SHA256:...` as printed by OpenSSH.
pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIPYniDmNzHoq6kTqdfJQ/N76Ew+MDpLVdOakNHdzKVSG";
    const KEY_B: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKHI9EkesfBNTFWaN2mISrjdheO09ckqrbu6ejpMJ1KY";

    fn key(s: &str) -> PublicKey {
        PublicKey::from_openssh(s).unwrap()
    }

    fn temp_path(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("batune-host-key-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("known_hosts");
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn a_missing_file_means_unknown() {
        let path = temp_path("missing");
        assert_eq!(check(&path, "h", 22, &key(KEY_A)).unwrap(), Status::Unknown);
    }

    #[test]
    fn a_learned_key_is_known_on_the_same_port_only() {
        let path = temp_path("learn");
        learn(&path, "h", 2222, &key(KEY_A)).unwrap();
        assert_eq!(check(&path, "h", 2222, &key(KEY_A)).unwrap(), Status::Known);
        assert_eq!(check(&path, "h", 22, &key(KEY_A)).unwrap(), Status::Unknown);
    }

    #[test]
    fn a_different_key_for_a_known_host_is_reported_with_its_line() {
        let path = temp_path("changed");
        // Written by hand: `learn` starts an empty file with a blank line.
        std::fs::write(&path, format!("other {KEY_B}\nh {KEY_A}\n")).unwrap();
        assert_eq!(
            check(&path, "h", 22, &key(KEY_B)).unwrap(),
            Status::Changed { line: 2 }
        );
    }

    const RSA: &str = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAAAgQC01wxrNnm3Bq2b2iRP9ogF9NB44KMNhchi30/teh79YjYmNzXPZx/Ny+13jYiq+Sch+qQh559Afrh58Im8vcwtAJkXx6/K/Xej9lDJzrTnP9Qltjh+YKpWm5eeAYUDdEvAmdkcFkLwMxrJjI112gFwXvyczOytJSh4cmRT/AFAQw==";

    #[test]
    fn a_key_of_another_algorithm_for_a_known_host_is_reported_as_changed() {
        let path = temp_path("other-algorithm");
        std::fs::write(&path, format!("other {RSA}\nh {RSA}\n")).unwrap();
        assert_eq!(
            check(&path, "h", 22, &key(KEY_A)).unwrap(),
            Status::Changed { line: 2 }
        );
    }

    #[test]
    fn algorithms_of_recorded_keys_are_preferred_in_the_given_order() {
        let preferred = [
            Algorithm::Ed25519,
            Algorithm::Rsa {
                hash: Some(HashAlg::Sha512),
            },
            Algorithm::Rsa {
                hash: Some(HashAlg::Sha256),
            },
        ];
        assert_eq!(
            prefer_recorded(&preferred, &[key(RSA)]),
            [
                preferred[1].clone(),
                preferred[2].clone(),
                preferred[0].clone()
            ]
        );
        assert_eq!(prefer_recorded(&preferred, &[]), preferred);
    }

    #[test]
    fn fingerprints_use_sha256() {
        assert!(fingerprint(&key(KEY_A)).starts_with("SHA256:"));
    }
}
