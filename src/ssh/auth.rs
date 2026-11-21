//! User authentication, asking for secrets inside the terminal.

use russh::MethodKind;
use russh::client::{AuthResult, Handle, KeyboardInteractiveAuthResponse};

use super::{BoxError, Handler, Keys, Output, Target};

const PASSWORD_TRIES: usize = 3;

/// Returns whether the server accepted the user.
pub async fn authenticate(
    handle: &mut Handle<Handler>,
    target: &Target,
    keys: &mut Keys,
    out: &Output,
) -> Result<bool, BoxError> {
    let user = target.user.as_str();
    // "none" both succeeds on open servers and tells which methods exist.
    let methods = match handle.authenticate_none(user).await? {
        AuthResult::Success => return Ok(true),
        AuthResult::Failure {
            remaining_methods, ..
        } => remaining_methods,
    };
    if methods.contains(&MethodKind::PublicKey)
        && super::public_key::authenticate(handle, user, keys, out).await?
    {
        return Ok(true);
    }
    if methods.contains(&MethodKind::Password) {
        for _ in 0..PASSWORD_TRIES {
            let prompt = format!("{user}@{}'s password: ", target.host);
            let Some(password) = keys.read_line(out, &prompt, false).await else {
                return Ok(false);
            };
            match handle.authenticate_password(user, password).await? {
                AuthResult::Success => return Ok(true),
                AuthResult::Failure { .. } => out.print("Permission denied, please try again.\n"),
            }
        }
    } else if methods.contains(&MethodKind::KeyboardInteractive) {
        return keyboard_interactive(handle, user, keys, out).await;
    }
    Ok(false)
}

async fn keyboard_interactive(
    handle: &mut Handle<Handler>,
    user: &str,
    keys: &mut Keys,
    out: &Output,
) -> Result<bool, BoxError> {
    let mut response = handle
        .authenticate_keyboard_interactive_start(user, None)
        .await?;
    loop {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
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
                        return Ok(false);
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
