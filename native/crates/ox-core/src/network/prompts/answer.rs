// SPDX-License-Identifier: AGPL-3.0-only
//! Checking the user's answer against the challenge it answers.
//!
//! Ports the validation in `MountPrompts.answer` of
//! `desktop/auth_bridge.py`. An answer that is not valid leaves the
//! challenge open, so the dialog can show why.

use super::challenge::{split_identity, validate_password, Answer, SignIn, SignInError};
use super::operation::{MountReply, PasswordRequest};
use crate::network::credential::{Credential, CredentialScope, Password};

/// Checks a sign-in answer against its password request; returns the
/// reply for `GVfs`.
///
/// # Errors
///
/// The [`SignInError`] the dialog shows: an invalid password or user name,
/// or a missing user name the server needs.
pub(super) fn accept_sign_in(answer: Answer, request: &PasswordRequest) -> Result<MountReply, SignInError> {
    let wants_guest = answer == Answer::Guest;
    let sign_in = match answer {
        Answer::SignIn(sign_in) => sign_in,
        // A guest or choice answer carries no account, like the empty
        // fields of the Python app's answer.
        Answer::Cancel | Answer::Guest | Answer::Choice(_) => SignIn {
            username: String::new(),
            password: Password::default(),
            scope: CredentialScope::Permanent,
        },
    };
    validate_password(&sign_in.password)?;
    let is_guest = wants_guest && request.flags.contains(gio::AskPasswordFlags::ANONYMOUS_SUPPORTED);
    let identity = split_identity(&sign_in.username, &request.domain)?;
    let needs_username = request.flags.contains(gio::AskPasswordFlags::NEED_USERNAME);
    if needs_username && identity.username.is_empty() && !is_guest {
        return Err(SignInError::MissingUsername);
    }
    if is_guest {
        return Ok(MountReply::Guest);
    }
    // Remembering is the default. Where GVfs cannot save, the account is
    // kept for the session only, never in a plaintext fallback.
    let can_save = request.flags.contains(gio::AskPasswordFlags::SAVING_SUPPORTED);
    let scope = if can_save {
        sign_in.scope
    } else {
        CredentialScope::Session
    };
    Ok(MountReply::Credential(Credential {
        username: identity.username,
        domain: identity.domain,
        password: sign_in.password,
        scope,
    }))
}

/// Checks the chosen answer to a question with `choice_count` buttons.
///
/// # Errors
///
/// [`SignInError::InvalidChoice`] for an index outside the buttons.
pub(super) fn accept_choice(choice: usize, choice_count: usize) -> Result<MountReply, SignInError> {
    if choice >= choice_count {
        return Err(SignInError::InvalidChoice);
    }
    let choice = i32::try_from(choice).map_err(|_| SignInError::InvalidChoice)?;
    Ok(MountReply::Choice(choice))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The flags of the Python fixture's challenge (`7`).
    const SIGN_IN_FLAGS: gio::AskPasswordFlags = gio::AskPasswordFlags::NEED_USERNAME
        .union(gio::AskPasswordFlags::NEED_PASSWORD)
        .union(gio::AskPasswordFlags::SAVING_SUPPORTED);

    fn request(flags: gio::AskPasswordFlags) -> PasswordRequest {
        PasswordRequest {
            username: "user".into(),
            domain: "WORKGROUP".into(),
            flags,
        }
    }

    fn sign_in(username: &str, scope: CredentialScope) -> Answer {
        Answer::SignIn(SignIn {
            username: username.into(),
            password: Password::from("test"),
            scope,
        })
    }

    /// parity: NET-011
    #[test]
    fn a_domain_in_the_user_name_replaces_the_default_domain() {
        let reply = accept_sign_in(
            sign_in("OFFICE\\sam", CredentialScope::Permanent),
            &request(SIGN_IN_FLAGS),
        );

        let expected = Credential {
            username: "sam".into(),
            domain: "OFFICE".into(),
            password: Password::from("test"),
            scope: CredentialScope::Permanent,
        };
        assert_eq!(reply, Ok(MountReply::Credential(expected)));
    }

    /// parity: NET-011
    #[test]
    fn remembering_falls_back_to_the_session_where_saving_is_unsupported() {
        let flags = gio::AskPasswordFlags::NEED_USERNAME | gio::AskPasswordFlags::NEED_PASSWORD;

        let reply = accept_sign_in(sign_in("sam", CredentialScope::Permanent), &request(flags));

        let Ok(MountReply::Credential(credential)) = reply else {
            panic!("a credential reply, got {reply:?}");
        };
        assert_eq!(credential.scope, CredentialScope::Session);
        assert_eq!(credential.domain, "WORKGROUP", "the server's default domain");
    }

    /// parity: NET-011
    #[test]
    fn guest_needs_no_user_name_but_only_where_the_server_offers_it() {
        let offered = SIGN_IN_FLAGS | gio::AskPasswordFlags::ANONYMOUS_SUPPORTED;

        assert_eq!(
            accept_sign_in(Answer::Guest, &request(offered)),
            Ok(MountReply::Guest)
        );
        assert_eq!(
            accept_sign_in(Answer::Guest, &request(SIGN_IN_FLAGS)),
            Err(SignInError::MissingUsername)
        );
    }

    /// parity: NET-011
    #[test]
    fn a_password_with_nul_is_refused() {
        let answer = Answer::SignIn(SignIn {
            username: "sam".into(),
            password: Password::from("a\0b"),
            scope: CredentialScope::Session,
        });

        assert_eq!(
            accept_sign_in(answer, &request(SIGN_IN_FLAGS)),
            Err(SignInError::InvalidPassword)
        );
    }

    /// parity: NET-013
    #[test]
    fn a_choice_must_be_one_of_the_buttons() {
        assert_eq!(accept_choice(1, 2), Ok(MountReply::Choice(1)));
        assert_eq!(accept_choice(2, 2), Err(SignInError::InvalidChoice));
    }
}
