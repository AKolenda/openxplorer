// SPDX-License-Identifier: AGPL-3.0-only
//! What a sign-in dialog shows, and what the user answers.
//!
//! Ports the `auth` event data and the `authReply` validation of
//! `v2.0.0:desktop/auth_bridge.py` (`MountPrompts._show_password`,
//! `_ask_question`, `answer` and `split_identity`).

use std::fmt;

use crate::location::python_strip;
use crate::network::credential::{CredentialScope, Password};

/// Longest accepted user name, in characters.
const MAX_USERNAME_CHARS: usize = 512;
/// Longest accepted password, in characters.
const MAX_PASSWORD_CHARS: usize = 16_384;
/// Longest question text shown, in characters.
const MAX_QUESTION_CHARS: usize = 4000;
/// Most answer buttons shown for a question.
const MAX_CHOICES: usize = 12;

/// Identifies one challenge until it is answered or dismissed. Ids are
/// never reused, so an answer to a dismissed challenge is recognised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChallengeId(pub(crate) u64);

impl fmt::Display for ChallengeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "sign-in-{}", self.0)
    }
}

/// A request for the user, shown in `OpenXplorer`'s secure dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Challenge {
    /// Identifies the challenge in [`MountPrompts::answer`](super::MountPrompts::answer).
    pub id: ChallengeId,
    /// The server asking (`Connect to <host>`); empty when unknown.
    pub host: String,
    /// What is asked.
    pub kind: ChallengeKind,
}

/// What a challenge asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChallengeKind {
    /// "Enter network credentials".
    Password(PasswordChallenge),
    /// "Network connection": a question with one button per choice.
    Question(QuestionChallenge),
}

/// The fields of the "Enter network credentials" dialog. The server's
/// default domain is deliberately not included: it stays in the backend.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is one of GIO's AskPasswordFlags or the retry state, which the dialog reads separately"
)]
pub struct PasswordChallenge {
    /// The location being mounted.
    pub uri: String,
    /// The user name to prefill.
    pub username: String,
    /// A user name is required unless signing in as guest.
    pub needs_username: bool,
    /// The server asks for a password.
    pub needs_password: bool,
    /// "Remember my credentials" is offered (and checked by default).
    pub can_save: bool,
    /// "Connect as guest" is offered.
    pub can_sign_in_as_guest: bool,
    /// The previous sign-in was not accepted ("The previous sign-in was
    /// not accepted. Check your username and password.").
    pub is_retry: bool,
}

/// A question from the backend, such as a certificate or host choice, or
/// the programs that keep a mount busy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionChallenge {
    /// The question, at most 4000 characters.
    pub message: String,
    /// One button per choice, at most 12.
    pub choices: Vec<String>,
}

impl QuestionChallenge {
    /// A question bounded to what the dialog shows.
    pub(crate) fn bounded(message: &str, choices: impl IntoIterator<Item = String>) -> Self {
        Self {
            message: message.chars().take(MAX_QUESTION_CHARS).collect(),
            choices: choices.into_iter().take(MAX_CHOICES).collect(),
        }
    }
}

/// The user's answer to a challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Cancel or ×: aborts the mount that asked.
    Cancel,
    /// "Connect" in the sign-in dialog.
    SignIn(SignIn),
    /// "Connect as guest": signs in anonymously and saves nothing.
    Guest,
    /// The index of the chosen button of a question.
    Choice(usize),
}

/// The fields of the sign-in dialog when the user presses Connect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignIn {
    /// The user name, optionally `DOMAIN\user`.
    pub username: String,
    /// The password.
    pub password: Password,
    /// "Remember my credentials": [`CredentialScope::Permanent`] when
    /// checked. Ignored where saving is not supported.
    pub scope: CredentialScope,
}

/// A user name split into user and domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The user name without the domain.
    pub username: String,
    /// The domain, or the server's default domain.
    pub domain: String,
}

/// Why an answer was not accepted. The dialog stays open and shows the
/// message, except for [`SignInError::Expired`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SignInError {
    /// The challenge was answered, cancelled, superseded or timed out.
    #[error("This sign-in request expired or was cancelled. Try connecting again.")]
    Expired,
    /// Too long, or contains NUL, CR or LF.
    #[error("Enter a valid username.")]
    InvalidUsername,
    /// `DOMAIN\user` with an empty part or a second backslash.
    #[error("Enter a username, or use DOMAIN\\username.")]
    MalformedDomainUsername,
    /// The server needs a user name and none was entered.
    #[error("Enter your username.")]
    MissingUsername,
    /// Too long, or contains NUL.
    #[error("The password is not valid.")]
    InvalidPassword,
    /// Not one of the question's choices.
    #[error("Choose one of the offered actions.")]
    InvalidChoice,
}

/// Splits `username` into user and domain: `DOMAIN\user`, or the user with
/// `default_domain`. The dialog has no domain field; advanced accounts use
/// `DOMAIN\user`. Ports `split_identity` in `v2.0.0:desktop/auth_bridge.py`.
///
/// # Errors
///
/// [`SignInError::InvalidUsername`] for a name over 512 characters or with
/// NUL, CR or LF, and [`SignInError::MalformedDomainUsername`] for an empty
/// domain or user part or a second backslash.
pub fn split_identity(username: &str, default_domain: &str) -> Result<Identity, SignInError> {
    let has_line_break_or_nul = username.contains(['\0', '\r', '\n']);
    if username.chars().count() > MAX_USERNAME_CHARS || has_line_break_or_nul {
        return Err(SignInError::InvalidUsername);
    }
    let username = python_strip(username);
    let Some((domain, user)) = username.split_once('\\') else {
        return Ok(Identity {
            username: username.to_owned(),
            domain: default_domain.to_owned(),
        });
    };
    if domain.is_empty() || user.is_empty() || user.contains('\\') {
        return Err(SignInError::MalformedDomainUsername);
    }
    Ok(Identity {
        username: user.to_owned(),
        domain: domain.to_owned(),
    })
}

/// Accepts a password of at most 16,384 characters without NUL.
///
/// # Errors
///
/// [`SignInError::InvalidPassword`] otherwise.
pub(crate) fn validate_password(password: &Password) -> Result<(), SignInError> {
    let text = password.as_str();
    if text.chars().count() > MAX_PASSWORD_CHARS || text.contains('\0') {
        return Err(SignInError::InvalidPassword);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(username: &str, domain: &str) -> Identity {
        Identity {
            username: username.into(),
            domain: domain.into(),
        }
    }

    /// Ported from `v2.0.0:desktop/tests/test_v05.py::CredentialsTests::test_domain_username_supported`
    ///
    /// parity: NET-007, NET-011
    #[test]
    fn a_domain_can_be_given_as_domain_backslash_user() {
        assert_eq!(split_identity("OFFICE\\sam", ""), Ok(identity("sam", "OFFICE")));
    }

    /// parity: NET-011
    #[test]
    fn a_plain_user_name_gets_the_default_domain_and_is_stripped() {
        assert_eq!(
            split_identity("  sam ", "WORKGROUP"),
            Ok(identity("sam", "WORKGROUP"))
        );
        assert_eq!(split_identity("", ""), Ok(identity("", "")));
    }

    struct RefusedName {
        username: String,
        error: SignInError,
    }

    /// parity: NET-011
    #[test]
    fn malformed_user_names_are_refused_in_the_dialog_wording() {
        let cases = [
            RefusedName {
                username: "a".repeat(MAX_USERNAME_CHARS + 1),
                error: SignInError::InvalidUsername,
            },
            RefusedName {
                username: "sam\nroot".into(),
                error: SignInError::InvalidUsername,
            },
            RefusedName {
                username: "\\sam".into(),
                error: SignInError::MalformedDomainUsername,
            },
            RefusedName {
                username: "OFFICE\\".into(),
                error: SignInError::MalformedDomainUsername,
            },
            RefusedName {
                username: "A\\B\\sam".into(),
                error: SignInError::MalformedDomainUsername,
            },
        ];
        for case in cases {
            assert_eq!(
                split_identity(&case.username, ""),
                Err(case.error),
                "{:?}",
                case.username
            );
        }
        assert_eq!(
            SignInError::MalformedDomainUsername.to_string(),
            "Enter a username, or use DOMAIN\\username."
        );
    }

    /// parity: NET-011
    #[test]
    fn passwords_are_bounded_and_free_of_nul() {
        let longest = Password::from("p".repeat(MAX_PASSWORD_CHARS));
        let too_long = Password::from("p".repeat(MAX_PASSWORD_CHARS + 1));
        assert_eq!(validate_password(&longest), Ok(()));
        assert_eq!(validate_password(&too_long), Err(SignInError::InvalidPassword));
        assert_eq!(
            validate_password(&Password::from("a\0b")),
            Err(SignInError::InvalidPassword)
        );
    }

    /// parity: NET-013
    #[test]
    fn questions_are_bounded_to_what_the_dialog_shows() {
        let choices = (0..20).map(|index| format!("Choice {index}"));
        let question = QuestionChallenge::bounded(&"q".repeat(5000), choices);
        assert_eq!(question.message.chars().count(), MAX_QUESTION_CHARS);
        assert_eq!(question.choices.len(), MAX_CHOICES);
    }
}
