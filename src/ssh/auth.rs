//! User authentication, asking for secrets inside the terminal.

use russh::MethodKind;
use russh::client::{AuthResult, Handle, KeyboardInteractiveAuthResponse};

use super::{BoxError, Handler, Keys, Output, Target};

const PASSWORD_TRIES: usize = 3;

/// Returns whether the server accepted the user. `public_key_only` is set
/// after the user accepted a changed host key: the host may be an impostor,
/// and only public key authentication keeps the secret from reaching it.
pub async fn authenticate(
    handle: &mut Handle<Handler>,
    target: &Target,
    keys: &mut Keys,
    out: &Output,
    public_key_only: bool,
) -> Result<bool, BoxError> {
    let user = target.user.as_str();
    // "none" both succeeds on open servers and tells which methods exist.
    let mut methods = match handle.authenticate_none(user).await? {
        AuthResult::Success => return Ok(true),
        AuthResult::Failure {
            remaining_methods, ..
        } => remaining_methods,
    };
    // Each method is tried once, but the server may ask for several in turn
    // (e.g. `publickey,password`), so the list is refreshed after each one.
    let mut tried = Vec::new();
    loop {
        let next = next_method(&methods, &tried, public_key_only);
        let Some(method) = next else {
            if public_key_only {
                out.print(
                    "Password authentication is disabled to avoid man-in-the-middle attacks.\n",
                );
            }
            return Ok(false);
        };
        tried.push(method);
        let result = match method {
            // No key to offer leaves the list as it was.
            MethodKind::PublicKey => {
                match super::public_key::authenticate(handle, user, keys, out).await? {
                    Some(result) => result,
                    None => continue,
                }
            }
            MethodKind::Password => match password(handle, target, keys, out).await? {
                Some(result) => result,
                None => return Ok(false),
            },
            _ => match keyboard_interactive(handle, user, keys, out).await? {
                Some(result) => result,
                None => return Ok(false),
            },
        };
        match result {
            AuthResult::Success => return Ok(true),
            AuthResult::Failure {
                remaining_methods,
                partial_success,
            } => {
                // Password and keyboard-interactive both ask the user for
                // the same secret; after one is refused, the other is not
                // tried, as before.
                if !partial_success && method != MethodKind::PublicKey {
                    return Ok(false);
                }
                methods = remaining_methods;
            }
        }
    }
}

/// The first method the server still accepts that has not been tried, in
/// the order `ssh` tries them.
fn next_method(
    methods: &[MethodKind],
    tried: &[MethodKind],
    public_key_only: bool,
) -> Option<MethodKind> {
    [
        MethodKind::PublicKey,
        MethodKind::Password,
        MethodKind::KeyboardInteractive,
    ]
    .into_iter()
    .find(|m| {
        methods.contains(m)
            && !tried.contains(m)
            && (!public_key_only || *m == MethodKind::PublicKey)
    })
}

/// `None` when the user cancels.
async fn password(
    handle: &mut Handle<Handler>,
    target: &Target,
    keys: &mut Keys,
    out: &Output,
) -> Result<Option<AuthResult>, BoxError> {
    let user = target.user.as_str();
    let mut last = None;
    for _ in 0..PASSWORD_TRIES {
        let prompt = format!("{user}@{}'s password: ", target.host);
        let Some(password) = keys.read_line(out, &prompt, false).await else {
            return Ok(None);
        };
        match handle.authenticate_password(user, password).await? {
            result @ (AuthResult::Success
            | AuthResult::Failure {
                partial_success: true,
                ..
            }) => return Ok(Some(result)),
            result => {
                out.print("Permission denied, please try again.\n");
                last = Some(result);
            }
        }
    }
    Ok(last)
}

/// `None` when the user cancels.
async fn keyboard_interactive(
    handle: &mut Handle<Handler>,
    user: &str,
    keys: &mut Keys,
    out: &Output,
) -> Result<Option<AuthResult>, BoxError> {
    let mut response = handle
        .authenticate_keyboard_interactive_start(user, None)
        .await?;
    loop {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(Some(AuthResult::Success)),
            KeyboardInteractiveAuthResponse::Failure {
                remaining_methods,
                partial_success,
            } => {
                return Ok(Some(AuthResult::Failure {
                    remaining_methods,
                    partial_success,
                }));
            }
            KeyboardInteractiveAuthResponse::InfoRequest {
                name,
                instructions,
                prompts,
            } => {
                for text in [name, instructions] {
                    if !text.is_empty() {
                        out.print(&format!("{text}\n"));
                    }
                }
                let mut answers = Vec::with_capacity(prompts.len());
                for p in prompts {
                    let Some(answer) = keys.read_line(out, &p.prompt, p.echo).await else {
                        return Ok(None);
                    };
                    answers.push(answer);
                }
                response = handle
                    .authenticate_keyboard_interactive_respond(answers)
                    .await?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use MethodKind::{KeyboardInteractive, Password, PublicKey};

    #[test]
    fn methods_are_tried_in_order_once_each() {
        let all = [Password, KeyboardInteractive, PublicKey];
        assert_eq!(next_method(&all, &[], false), Some(PublicKey));
        assert_eq!(next_method(&all, &[PublicKey], false), Some(Password));
    }

    #[test]
    fn a_method_the_server_asks_for_after_a_partial_success_is_tried() {
        // `password,publickey`: "none" lists only password; once it
        // partially succeeds the server asks for publickey.
        assert_eq!(next_method(&[Password], &[], false), Some(Password));
        assert_eq!(
            next_method(&[PublicKey], &[Password], false),
            Some(PublicKey)
        );
        assert_eq!(
            next_method(&[PublicKey], &[Password, PublicKey], false),
            None
        );
    }

    #[test]
    fn only_public_keys_are_used_after_a_changed_host_key() {
        let all = [Password, KeyboardInteractive, PublicKey];
        assert_eq!(next_method(&all, &[], true), Some(PublicKey));
        assert_eq!(next_method(&all, &[PublicKey], true), None);
        assert_eq!(
            next_method(&[Password, KeyboardInteractive], &[], true),
            None
        );
    }
}
