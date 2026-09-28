// SPDX-License-Identifier: AGPL-3.0-only
//! The mounts in progress and their open challenges.
//!
//! Ports the `operations` and `pending` tables of `MountPrompts` in
//! `desktop/auth_bridge.py` and the methods that change them (`create`,
//! `_ask_question`, `_show_processes`, `_new`, `_consume`, `_dismiss`,
//! `_expire`, `finish` and `close`). Password requests are answered in
//! `password`. A successful SMB mount is reported to the window, which
//! resumes the server's indexing as `mount` in `desktop/winspace.py` does.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gio::prelude::*;

use super::challenge::{Challenge, ChallengeId, ChallengeKind, QuestionChallenge, SignInError};
use super::operation::{send_reply, MountReply, PasswordRequest, QuestionSource};
use super::{MountOutcome, SignInPrompter, KEYRING_SAVE_NOTICE};
use crate::network::credential::Credential;
use crate::network::credential_store::CredentialGeneration;
use crate::network::server::{host_name, smb_host};
use crate::network::session_credentials::SessionCredentials;

/// The shared state of one window's prompts. The operations' signal
/// handlers hold it weakly, and the `RefCell` is borrowed only between
/// calls into GIO or the interface, which may call back.
pub(super) struct Inner {
    /// Where known accounts come from and entered ones are saved.
    pub(super) credentials: Arc<SessionCredentials>,
    /// The window's sign-in dialog.
    prompter: Rc<dyn SignInPrompter>,
    /// How long a challenge stays open.
    challenge_lifetime: Duration,
    /// The mounts in progress and their open challenges.
    pub(super) state: RefCell<State>,
    /// Called with the host of every SMB server a mount reached.
    server_mounted: RefCell<Vec<ServerMountedHandler>>,
}

/// A handler of [`MountPrompts::connect_server_mounted`](super::MountPrompts::connect_server_mounted).
pub(super) type ServerMountedHandler = Rc<dyn Fn(&str)>;

/// The mutable part of [`Inner`].
#[derive(Default)]
pub(super) struct State {
    /// Set by `close`: no further dialog is shown.
    pub(super) is_closed: bool,
    /// The id of the latest challenge; ids are never reused.
    last_challenge: u64,
    /// The challenges shown and not yet answered.
    pending: BTreeMap<ChallengeId, PendingChallenge>,
    /// The mounts in progress.
    pub(super) operations: HashMap<gio::MountOperation, OperationRecord>,
}

/// A challenge shown and not yet answered.
struct PendingChallenge {
    operation: gio::MountOperation,
    request: PendingRequest,
    /// Aborts the mount when the challenge expires.
    expiry: glib::JoinHandle<()>,
}

/// What an answer to a pending challenge is checked against.
#[derive(Debug, Clone)]
pub(super) enum PendingRequest {
    /// The sign-in dialog, for this password request.
    Password(PasswordRequest),
    /// A question with this many buttons.
    Question { choice_count: usize },
}

/// One mount in progress.
pub(super) struct OperationRecord {
    /// The canonical location being mounted.
    pub(super) uri: String,
    /// How often `GVfs` asked for a password.
    pub(super) attempts: u32,
    /// The server's sign-out generation when the mount started.
    generation: CredentialGeneration,
    /// The latest password request.
    pub(super) request: Option<PasswordRequest>,
    /// The account given to `GVfs`, kept if the mount succeeds.
    candidate: Option<Candidate>,
}

/// An account given to `GVfs`.
pub(super) struct Candidate {
    pub(super) credential: Credential,
    pub(super) source: CandidateSource,
}

impl Candidate {
    /// The account typed into the dialog that `reply` signs in with;
    /// `None` for a guest sign-in, which keeps no account.
    pub(super) fn entered_in(reply: &MountReply) -> Option<Self> {
        let MountReply::Credential(credential) = reply else {
            return None;
        };
        Some(Self {
            credential: credential.clone(),
            source: CandidateSource::Entered,
        })
    }
}

/// Where a candidate account came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CandidateSource {
    /// Memory or the keyring: already saved.
    Known,
    /// Typed into the dialog: saved once the mount succeeds.
    Entered,
}

impl Inner {
    pub(super) fn new(
        credentials: Arc<SessionCredentials>,
        prompter: Rc<dyn SignInPrompter>,
        challenge_lifetime: Duration,
    ) -> Self {
        Self {
            credentials,
            prompter,
            challenge_lifetime,
            state: RefCell::default(),
            server_mounted: RefCell::default(),
        }
    }

    pub(super) fn connect_server_mounted(&self, handler: ServerMountedHandler) {
        self.server_mounted.borrow_mut().push(handler);
    }

    pub(super) fn credentials(&self) -> &Arc<SessionCredentials> {
        &self.credentials
    }

    /// Starts tracking `operation`, which mounts the canonical `uri`.
    pub(super) fn track(&self, operation: &gio::MountOperation, uri: String) {
        let record = OperationRecord {
            generation: self.credentials.generation(&uri),
            uri,
            attempts: 0,
            request: None,
            candidate: None,
        };
        self.state
            .borrow_mut()
            .operations
            .insert(operation.clone(), record);
    }

    /// Records the account `operation` signs in with; false when the
    /// operation is no longer tracked.
    pub(super) fn set_candidate(
        &self,
        operation: &gio::MountOperation,
        candidate: Option<Candidate>,
    ) -> bool {
        let mut state = self.state.borrow_mut();
        let Some(record) = state.operations.get_mut(operation) else {
            return false;
        };
        record.candidate = candidate;
        true
    }

    /// Handles a question GIO asks through `operation`.
    pub(super) fn on_question(
        self: &Rc<Self>,
        operation: &gio::MountOperation,
        question: QuestionChallenge,
        source: QuestionSource,
    ) {
        let host = match source {
            QuestionSource::Server => self.host_of_operation(operation),
            QuestionSource::BusyMount => String::new(),
        };
        let request = PendingRequest::Question {
            choice_count: question.choices.len(),
        };
        self.present(operation, host, ChallengeKind::Question(question), request);
    }

    /// The host of the location `operation` mounts, or empty.
    fn host_of_operation(&self, operation: &gio::MountOperation) -> String {
        let state = self.state.borrow();
        let record = state.operations.get(operation);
        record
            .and_then(|record| host_name(&record.uri))
            .unwrap_or_default()
    }

    /// Shows a new challenge for `operation`, replacing its previous one.
    pub(super) fn present(
        self: &Rc<Self>,
        operation: &gio::MountOperation,
        host: String,
        kind: ChallengeKind,
        request: PendingRequest,
    ) {
        // NET-012: a retry supersedes only this mount's challenge, never
        // another server's.
        self.dismiss_operation(operation);
        let id = self.next_challenge_id();
        let pending = PendingChallenge {
            operation: operation.clone(),
            request,
            expiry: self.schedule_expiry(id),
        };
        self.state.borrow_mut().pending.insert(id, pending);
        self.prompter.show_challenge(&Challenge { id, host, kind });
    }

    fn next_challenge_id(&self) -> ChallengeId {
        let mut state = self.state.borrow_mut();
        state.last_challenge += 1;
        ChallengeId(state.last_challenge)
    }

    /// Aborts the mount of challenge `id` once it has been open for the
    /// challenge lifetime (NET-012).
    fn schedule_expiry(self: &Rc<Self>, id: ChallengeId) -> glib::JoinHandle<()> {
        let prompts = Rc::downgrade(self);
        let lifetime = self.challenge_lifetime;
        glib::MainContext::ref_thread_default().spawn_local(async move {
            glib::timeout_future(lifetime).await;
            if let Some(prompts) = prompts.upgrade() {
                prompts.expire(id);
            }
        })
    }

    fn expire(&self, id: ChallengeId) {
        // Taken without aborting its expiry task: that task is running.
        let Some(pending) = self.state.borrow_mut().pending.remove(&id) else {
            return;
        };
        self.prompter.dismiss_challenge(id);
        send_reply(&pending.operation, MountReply::Abort);
    }

    /// The operation and request of pending challenge `id`.
    ///
    /// # Errors
    ///
    /// [`SignInError::Expired`] when the challenge was answered, cancelled
    /// or dismissed already.
    pub(super) fn pending_challenge(
        &self,
        id: ChallengeId,
    ) -> Result<(gio::MountOperation, PendingRequest), SignInError> {
        let state = self.state.borrow();
        let pending = state.pending.get(&id).ok_or(SignInError::Expired)?;
        Ok((pending.operation.clone(), pending.request.clone()))
    }

    /// Removes challenge `id` and dismisses its dialog.
    pub(super) fn consume(&self, id: ChallengeId) {
        let Some(pending) = self.state.borrow_mut().pending.remove(&id) else {
            return;
        };
        pending.expiry.abort();
        self.prompter.dismiss_challenge(id);
    }

    /// Removes every challenge of `operation`.
    fn dismiss_operation(&self, operation: &gio::MountOperation) {
        let ids: Vec<ChallengeId> = {
            let state = self.state.borrow();
            let of_operation = state
                .pending
                .iter()
                .filter(|(_, pending)| &pending.operation == operation);
            of_operation.map(|(id, _)| *id).collect()
        };
        for id in ids {
            self.consume(id);
        }
    }

    pub(super) fn on_aborted(&self, operation: &gio::MountOperation) {
        self.finish(operation, MountOutcome::Failed);
    }

    pub(super) fn finish(&self, operation: &gio::MountOperation, outcome: MountOutcome) {
        self.dismiss_operation(operation);
        let record = self.state.borrow_mut().operations.remove(operation);
        // Privacy rule (SAFE-011): no password stays on a finished
        // operation.
        operation.set_password(None);
        let Some(record) = record else {
            return;
        };
        // Safety rule (NET-015): credentials are kept only after a
        // successful mount.
        if outcome != MountOutcome::Mounted {
            return;
        }
        self.keep_signed_in_account(&record);
        self.report_server_mounted(&record.uri);
    }

    /// Keeps the account that the successful mount `record` signed in
    /// with, and saves it in the keyring if it was typed into the dialog.
    fn keep_signed_in_account(&self, record: &OperationRecord) {
        let Some(candidate) = &record.candidate else {
            return;
        };
        // Safety rule (SAFE-012): Sign out during the mount wins.
        if self.credentials.generation(&record.uri) != record.generation {
            return;
        }
        self.credentials.accept_memory(&record.uri, &candidate.credential);
        if candidate.source == CandidateSource::Entered {
            let credential = candidate.credential.clone();
            self.save_in_background(record.uri.clone(), credential, record.generation);
        }
    }

    /// Tells the server-mounted handlers that the mount of `uri`
    /// succeeded, when it is on an SMB server (`mount` in `winspace.py`
    /// resumes that server's indexing).
    fn report_server_mounted(&self, uri: &str) {
        let Some(host) = smb_host(uri) else {
            return;
        };
        // Cloned, so that a handler may connect another one.
        let handlers = self.server_mounted.borrow().clone();
        for handler in handlers {
            handler(&host);
        }
    }

    /// Saves an entered credential in the keyring off the main thread,
    /// with a notice if that fails.
    fn save_in_background(&self, uri: String, credential: Credential, generation: CredentialGeneration) {
        let credentials = Arc::clone(&self.credentials);
        let prompter = Rc::downgrade(&self.prompter);
        glib::MainContext::ref_thread_default().spawn_local(async move {
            let saved = gio::spawn_blocking(move || credentials.persist(&uri, &credential, generation)).await;
            let is_saved = matches!(saved, Ok(Ok(())));
            if let Some(prompter) = prompter.upgrade().filter(|_| !is_saved) {
                prompter.show_notice(KEYRING_SAVE_NOTICE);
            }
        });
    }

    pub(super) fn close(&self) {
        let (pending, operations) = {
            let mut state = self.state.borrow_mut();
            state.is_closed = true;
            (
                std::mem::take(&mut state.pending),
                std::mem::take(&mut state.operations),
            )
        };
        for (id, challenge) in pending {
            challenge.expiry.abort();
            self.prompter.dismiss_challenge(id);
            send_reply(&challenge.operation, MountReply::Abort);
        }
        for operation in operations.keys() {
            operation.set_password(None);
        }
        // Privacy rule (SAFE-011, TAB-050): a closed window keeps no
        // password in memory (`close` in auth_bridge.py).
        self.credentials.clear_memory();
        // A handler that holds the window must not keep it alive.
        self.server_mounted.borrow_mut().clear();
    }
}

impl fmt::Debug for Inner {
    /// Shows which challenges are open and how many mounts run, never an
    /// account.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.borrow();
        formatter
            .debug_struct("Inner")
            .field("is_closed", &state.is_closed)
            .field("pending", &state.pending.keys().collect::<Vec<_>>())
            .field("operations", &state.operations.len())
            .finish_non_exhaustive()
    }
}
