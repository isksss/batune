//! Public key authentication: the keys an SSH agent holds, then OpenSSH's
//! default key files in `~/.ssh`, asking for their passphrase in the
//! terminal.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use russh::client::{AuthResult, Handle};
use russh::keys::agent::client::{AgentClient, AgentStream};
use russh::keys::{self, HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey};

use super::{BoxError, Handler, Keys, Output};

/// The files `ssh` tries when no `-i` is given, in its order. DSA is left out:
/// russh is built without it and OpenSSH has dropped it.
const KEY_FILES: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];

const PASSPHRASE_TRIES: usize = 3;

/// The server's answer to the first key it accepted, fully or partially,
/// or else to the last key offered; `None` when no key was offered.
pub async fn authenticate(
    handle: &mut Handle<Handler>,
    user: &str,
    keys: &mut Keys,
    out: &Output,
) -> Result<Option<AuthResult>, BoxError> {
    let rsa_hash = handle.best_supported_rsa_hash().await?.flatten();
    let mut tried = Vec::new();
    let mut last = None;
    if let Some(result) = with_agent(handle, user, rsa_hash, &mut tried, &mut last).await? {
        return Ok(Some(result));
    }
    let Some(dir) = std::env::home_dir().map(|home| home.join(".ssh")) else {
        return Ok(last);
    };
    for path in KEY_FILES.map(|name| dir.join(name)) {
        // Asking for the passphrase of a key the agent already offered would
        // only annoy; the `.pub` next to it tells which key it is.
        if public_key_of(&path).is_some_and(|key| tried.contains(&key)) {
            continue;
        }
        let Some(key) = load(&path, keys, out).await else {
            continue;
        };
        if tried.contains(key.public_key()) {
            continue;
        }
        tried.push(key.public_key().clone());
        let key = PrivateKeyWithHashAlg::new(Arc::new(key), rsa_hash);
        let result = handle.authenticate_publickey(user, key).await?;
        if accepted(&result) {
            return Ok(Some(result));
        }
        last = Some(result);
    }
    Ok(last)
}

/// A partial success means the key was good but the server wants another
/// method too; trying more keys would not help.
fn accepted(result: &AuthResult) -> bool {
    matches!(
        result,
        AuthResult::Success
            | AuthResult::Failure {
                partial_success: true,
                ..
            }
    )
}

type Agent = AgentClient<Box<dyn AgentStream + Send + Unpin>>;

/// An agent that is missing or misbehaves is skipped rather than failing the
/// connection, since the key files and passwords may still get the user in.
async fn with_agent(
    handle: &mut Handle<Handler>,
    user: &str,
    rsa_hash: Option<HashAlg>,
    tried: &mut Vec<PublicKey>,
    last: &mut Option<AuthResult>,
) -> Result<Option<AuthResult>, BoxError> {
    let Some(mut agent) = connect_agent().await else {
        return Ok(None);
    };
    let Ok(identities) = agent.request_identities().await else {
        return Ok(None);
    };
    for identity in identities {
        let key = identity.public_key().into_owned();
        if tried.contains(&key) {
            continue;
        }
        tried.push(key.clone());
        match handle
            .authenticate_publickey_with(user, key, rsa_hash, &mut agent)
            .await
        {
            Ok(result) if accepted(&result) => return Ok(Some(result)),
            Ok(result) => *last = Some(result),
            // The agent refused to sign (e.g. the user declined a
            // confirmation); the next key may still work.
            Err(_) => {}
        }
    }
    Ok(None)
}

#[cfg(unix)]
async fn connect_agent() -> Option<Agent> {
    Some(AgentClient::connect_env().await.ok()?.dynamic())
}

/// Windows' OpenSSH agent, then Pageant (PuTTY's agent).
#[cfg(windows)]
async fn connect_agent() -> Option<Agent> {
    let pipe =
        std::env::var_os("SSH_AUTH_SOCK").unwrap_or_else(|| r"\\.\pipe\openssh-ssh-agent".into());
    if let Ok(agent) = AgentClient::connect_named_pipe(pipe).await {
        return Some(agent.dynamic());
    }
    Some(AgentClient::connect_pageant().await.ok()?.dynamic())
}

fn public_key_of(path: &Path) -> Option<PublicKey> {
    let mut pub_path = PathBuf::from(path);
    pub_path.as_mut_os_string().push(".pub");
    PublicKey::read_openssh_file(&pub_path).ok()
}

/// `None` when the file is missing or unreadable, or the passphrase was not
/// given; these skip the key as `ssh` does.
async fn load(path: &Path, keys: &mut Keys, out: &Output) -> Option<PrivateKey> {
    let text = std::fs::read_to_string(path).ok()?;
    match keys::decode_secret_key(&text, None) {
        Ok(key) => return Some(key),
        Err(keys::Error::KeyIsEncrypted) => {}
        Err(e) => {
            out.print(&format!("Load key \"{}\": {e}\n", path.display()));
            return None;
        }
    }
    let prompt = format!("Enter passphrase for key '{}': ", path.display());
    for _ in 0..PASSPHRASE_TRIES {
        let passphrase = keys.read_line(out, &prompt, false).await?;
        // An empty passphrase means "skip this key", as in `ssh`.
        if passphrase.is_empty() {
            return None;
        }
        match keys::decode_secret_key(&text, Some(&passphrase)) {
            Ok(key) => return Some(key),
            Err(_) => out.print("Bad passphrase, try again.\n"),
        }
    }
    None
}
