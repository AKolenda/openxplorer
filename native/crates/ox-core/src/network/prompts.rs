// SPDX-License-Identifier: AGPL-3.0-only
//! Network sign-in: `GVfs`'s password and question prompts, answered by the
//! window's own dialog.
//!
//! Ports `MountPrompts` in `desktop/auth_bridge.py`. `OpenXplorer` owns the
//! prompt, not a credential database: `GVfs` keeps remembered passwords in
//! the keyring, and [`SessionCredentials`] keeps the account so other
//! shares on the server sign in without asking again.
//!
//! A mount gets its `gio::MountOperation` from [`MountPrompts::create`]
//! and reports the result to [`MountPrompts::finish`]. When `GVfs` asks for a
//! password, the first attempt reuses a known credential (from memory, else
//! from the keyring off the main thread); otherwise, and after a rejected
//! attempt, the window's [`SignInPrompter`] shows a [`Challenge`] and the
//! user's [`Answer`] comes back through [`MountPrompts::answer`].
//!
//! The rules of the Python module, each named where it is enforced:
//!
//! - Passwords are never passed back to the interface: a [`Challenge`]
//!   holds none, and [`Password`](super::Password) hides itself from
//!   `Debug` (SAFE-011).
//! - A rejected saved credential shows the dialog instead of looping
//!   (NET-014).
//! - Credentials are saved only after a successful mount (NET-015), and
//!   not after Sign out raced the mount (SAFE-012).
//! - An unanswered challenge expires after 180 seconds; a new challenge
//!   from the same mount replaces the old one (NET-012).
//!
//! Unlike Python, [`MountPrompts::close`] leaves the [`SessionCredentials`]
//! memory alone: every window shares it, so the app clears it when its
//! last window closes.
//!
//! | Module | Responsibility |
//! |---|---|
//! | `challenge` | What the dialog shows and what the user answers |
//! | `answer` | Checking an answer against the challenge it answers |
//! | `operation` | The `gio::MountOperation` signals and replies |
//! | `password` | A known account first, else the sign-in dialog |
//! | `state` | The mounts in progress and their open challenges |

mod answer;
mod challenge;
mod operation;
mod password;
mod state;
#[cfg(test)]
mod tests;

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

pub use challenge::{
    split_identity, Answer, Challenge, ChallengeId, ChallengeKind, Identity, PasswordChallenge,
    QuestionChallenge, SignIn, SignInError,
};

use self::answer::{accept_choice, accept_sign_in};
use self::operation::{connect_signals, send_reply, MountReply};
use self::state::{Candidate, Inner, PendingRequest};
use super::session_credentials::SessionCredentials;
use crate::location::{normalise, LocationError};

/// How long an unanswered challenge stays open before its mount is
/// aborted.
pub(super) const CHALLENGE_LIFETIME: Duration = Duration::from_secs(180);

/// Shown when a credential that worked could not be saved.
pub const KEYRING_SAVE_NOTICE: &str =
    "Credentials work in this window, but could not be saved in the system \
     keyring. Session reuse after closing OpenXplorer is not available until the keyring works.";

/// The window's sign-in dialog, supplied by the interface.
pub trait SignInPrompter {
    /// Shows `challenge`. Several may be open; the interface queues them.
    fn show_challenge(&self, challenge: &Challenge);
    /// Removes the dialog of `id`: it was answered, replaced, expired or
    /// its mount was aborted.
    fn dismiss_challenge(&self, id: ChallengeId);
    /// Shows a passing notice, such as [`KEYRING_SAVE_NOTICE`].
    fn show_notice(&self, message: &str);
}

/// Whether the mount that used an operation succeeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountOutcome {
    /// The location is mounted (or already was).
    Mounted,
    /// The mount failed, was cancelled or was aborted.
    Failed,
}

/// The sign-in prompts of one window. Cloning gives another handle to the
/// same prompts.
///
/// Lives on the main thread: GIO emits the operations' signals there.
#[derive(Clone)]
pub struct MountPrompts {
    inner: Rc<Inner>,
}

impl MountPrompts {
    /// Prompts that reuse and save credentials in `credentials` and ask
    /// through `prompter`.
    pub fn new(credentials: Arc<SessionCredentials>, prompter: Rc<dyn SignInPrompter>) -> Self {
        Self::with_challenge_lifetime(credentials, prompter, CHALLENGE_LIFETIME)
    }

    /// Prompts whose challenges expire after `challenge_lifetime`, for
    /// tests that cannot wait three minutes.
    pub(super) fn with_challenge_lifetime(
        credentials: Arc<SessionCredentials>,
        prompter: Rc<dyn SignInPrompter>,
        challenge_lifetime: Duration,
    ) -> Self {
        let inner = Inner::new(credentials, prompter, challenge_lifetime);
        Self {
            inner: Rc::new(inner),
        }
    }

    /// A mount operation for mounting or unmounting `uri`, answered by
    /// these prompts. Pass it to GIO, then call [`MountPrompts::finish`].
    ///
    /// # Errors
    ///
    /// The [`LocationError`] of an address that is not a supported location.
    pub fn create(&self, uri: &str) -> Result<gio::MountOperation, LocationError> {
        let uri = normalise(uri)?;
        let operation = gio::MountOperation::new();
        self.inner.track(&operation, uri);
        connect_signals(&operation, &self.inner);
        Ok(operation)
    }

    /// Answers the challenge `id`. On success the dialog is dismissed and
    /// the mount continues (or is aborted, for [`Answer::Cancel`]).
    ///
    /// # Errors
    ///
    /// [`SignInError::Expired`] when the challenge is gone; the other
    /// [`SignInError`]s when the answer is not valid, in which case the
    /// challenge stays open.
    pub fn answer(&self, id: ChallengeId, answer: Answer) -> Result<(), SignInError> {
        let (operation, request) = self.inner.pending_challenge(id)?;
        let reply = match (answer, request) {
            (Answer::Cancel, _) => MountReply::Abort,
            (answer, PendingRequest::Password(request)) => {
                let reply = accept_sign_in(answer, &request)?;
                self.inner
                    .set_candidate(&operation, Candidate::entered_in(&reply));
                reply
            }
            (Answer::Choice(choice), PendingRequest::Question { choice_count }) => {
                accept_choice(choice, choice_count)?
            }
            (_, PendingRequest::Question { .. }) => return Err(SignInError::InvalidChoice),
        };
        self.inner.consume(id);
        send_reply(&operation, reply);
        Ok(())
    }

    /// Ends the mount that used `operation`: dismisses its challenges, and
    /// after a successful mount keeps the account it signed in with.
    pub fn finish(&self, operation: &gio::MountOperation, outcome: MountOutcome) {
        self.inner.finish(operation, outcome);
    }

    /// Closes the window's prompts: every open challenge is dismissed and
    /// its mount aborted, and no password stays on an operation.
    pub fn close(&self) {
        self.inner.close();
    }

    /// The credential store these prompts reuse and save accounts in.
    pub(crate) fn credentials(&self) -> &Arc<SessionCredentials> {
        self.inner.credentials()
    }
}

impl std::fmt::Debug for MountPrompts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MountPrompts")
            .field("state", &self.inner)
            .finish()
    }
}
