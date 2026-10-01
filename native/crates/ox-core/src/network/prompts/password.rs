// SPDX-License-Identifier: AGPL-3.0-only
//! Answering `GVfs`'s password requests: a known account first, else the
//! sign-in dialog.
//!
//! Ports `_ask_password`, `_reuse` and `_show_password` of `MountPrompts`
//! in `v2.0.0:desktop/auth_bridge.py`.

use std::rc::Rc;
use std::sync::Arc;

use super::challenge::{ChallengeKind, PasswordChallenge};
use super::operation::{send_reply, MountReply, PasswordRequest};
use super::state::{Candidate, CandidateSource, Inner, OperationRecord, PendingRequest};
use crate::network::credential::Credential;
use crate::network::server::host_name;

impl Inner {
    /// Handles `GVfs` asking `operation` for a password.
    pub(super) fn on_ask_password(
        self: &Rc<Self>,
        operation: &gio::MountOperation,
        request: PasswordRequest,
    ) {
        let Some((attempts, uri)) = self.record_attempt(operation, request) else {
            send_reply(operation, MountReply::Abort);
            return;
        };
        // Safety rule (NET-014): a rejected attempt shows the dialog
        // instead of reusing the same credential in a loop.
        if attempts > 1 {
            self.show_password(operation);
            return;
        }
        match self.credentials.peek(&uri) {
            Some(known) => self.reuse(operation, known),
            None => self.load_known_credential(operation, uri),
        }
    }

    /// Counts a password request of `operation`; returns the attempt number
    /// and location, or `None` for an operation these prompts do not track.
    fn record_attempt(
        &self,
        operation: &gio::MountOperation,
        request: PasswordRequest,
    ) -> Option<(u32, String)> {
        let mut state = self.state.borrow_mut();
        let record = state.operations.get_mut(operation)?;
        record.attempts += 1;
        record.request = Some(request);
        Some((record.attempts, record.uri.clone()))
    }

    /// Looks the server's credential up in the keyring off the main thread,
    /// then reuses it or shows the dialog.
    fn load_known_credential(self: &Rc<Self>, operation: &gio::MountOperation, uri: String) {
        let credentials = Arc::clone(&self.credentials);
        let prompts = Rc::downgrade(self);
        let operation = operation.clone();
        glib::MainContext::ref_thread_default().spawn_local(async move {
            // PERF-003: the Secret Service may show an unlock prompt, so
            // the main thread never waits for it.
            let loaded = gio::spawn_blocking(move || credentials.load(&uri)).await;
            let Some(prompts) = prompts.upgrade() else {
                return;
            };
            // A keyring error or a failed worker shows the dialog instead.
            let known = loaded.ok().and_then(Result::ok).flatten();
            prompts.deliver_known_credential(&operation, known);
        });
    }

    fn deliver_known_credential(self: &Rc<Self>, operation: &gio::MountOperation, known: Option<Credential>) {
        let is_tracked = {
            let state = self.state.borrow();
            !state.is_closed && state.operations.contains_key(operation)
        };
        if !is_tracked {
            return;
        }
        match known {
            Some(credential) => self.reuse(operation, credential),
            None => self.show_password(operation),
        }
    }

    /// Signs `operation` in with a known credential, without a dialog.
    fn reuse(&self, operation: &gio::MountOperation, credential: Credential) {
        let candidate = Candidate {
            credential: credential.clone(),
            source: CandidateSource::Known,
        };
        let reply = if self.set_candidate(operation, Some(candidate)) {
            MountReply::Credential(credential)
        } else {
            MountReply::Abort
        };
        send_reply(operation, reply);
    }

    /// Shows the sign-in dialog for the latest password request of
    /// `operation`, or aborts the mount once the prompts are closed.
    fn show_password(self: &Rc<Self>, operation: &gio::MountOperation) {
        let dialog = {
            let state = self.state.borrow();
            let Some(record) = state.operations.get(operation) else {
                return;
            };
            match &record.request {
                Some(request) if !state.is_closed => Some(PasswordDialog::for_request(record, request)),
                _ => None,
            }
        };
        let Some(dialog) = dialog else {
            send_reply(operation, MountReply::Abort);
            return;
        };
        let request = PendingRequest::Password(dialog.request);
        self.present(operation, dialog.host, dialog.kind, request);
    }
}

/// A sign-in dialog about to be shown.
struct PasswordDialog {
    /// The server asking.
    host: String,
    /// The dialog's fields.
    kind: ChallengeKind,
    /// The request the answer is checked against.
    request: PasswordRequest,
}

impl PasswordDialog {
    /// The dialog for `request`, the latest password request of the
    /// mount `record`.
    fn for_request(record: &OperationRecord, request: &PasswordRequest) -> Self {
        let flags = request.flags;
        let fields = PasswordChallenge {
            uri: record.uri.clone(),
            username: request.username.clone(),
            needs_username: flags.contains(gio::AskPasswordFlags::NEED_USERNAME),
            needs_password: flags.contains(gio::AskPasswordFlags::NEED_PASSWORD),
            can_save: flags.contains(gio::AskPasswordFlags::SAVING_SUPPORTED),
            can_sign_in_as_guest: flags.contains(gio::AskPasswordFlags::ANONYMOUS_SUPPORTED),
            is_retry: record.attempts > 1,
        };
        Self {
            host: host_name(&record.uri).unwrap_or_default(),
            kind: ChallengeKind::Password(fields),
            request: request.clone(),
        }
    }
}
