// SPDX-License-Identifier: AGPL-3.0-only
//! The `gio::MountOperation` side of the sign-in prompts: its signals in,
//! its replies out.
//!
//! Ports `MountPrompts.create`, `_ask_password`, `_ask_question`,
//! `_show_processes` and the `op.set_*` / `op.reply` calls of
//! `v2.0.0:desktop/auth_bridge.py`. A plain `gio::MountOperation` is used, never
//! GTK's, so `GVfs`'s challenges reach `OpenXplorer`'s own dialog instead of a
//! GNOME Shell prompt.

use std::rc::{Rc, Weak};

use gio::prelude::*;

use super::challenge::QuestionChallenge;
use super::state::Inner;
use crate::network::credential::Credential;

/// What `GVfs` asked for in an `ask-password` signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PasswordRequest {
    /// The user name to prefill.
    pub(super) username: String,
    /// The server's default domain, used when the user types no domain.
    pub(super) domain: String,
    /// What the server needs and offers.
    pub(super) flags: gio::AskPasswordFlags,
}

/// The answer a mount operation passes back to `GVfs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MountReply {
    /// Abort the mount.
    Abort,
    /// Sign in with an account.
    Credential(Credential),
    /// Sign in anonymously; `GVfs` saves nothing.
    Guest,
    /// The index of the chosen answer to a question.
    Choice(i32),
}

/// Sends `reply` to `GVfs` through `operation`.
pub(super) fn send_reply(operation: &gio::MountOperation, reply: MountReply) {
    let result = match reply {
        MountReply::Abort => gio::MountOperationResult::Aborted,
        MountReply::Credential(credential) => {
            operation.set_anonymous(false);
            operation.set_username(Some(&credential.username));
            operation.set_domain(Some(&credential.domain));
            operation.set_password(Some(credential.password.as_str()));
            operation.set_password_save(credential.scope.password_save());
            gio::MountOperationResult::Handled
        }
        MountReply::Guest => {
            operation.set_anonymous(true);
            operation.set_password_save(gio::PasswordSave::Never);
            gio::MountOperationResult::Handled
        }
        MountReply::Choice(choice) => {
            operation.set_choice(choice);
            gio::MountOperationResult::Handled
        }
    };
    operation.reply(result);
}

/// Routes the signals of `operation` to `prompts`. The handlers hold the
/// prompts weakly, so an operation outliving its window is aborted
/// instead of keeping the window's prompts alive.
pub(super) fn connect_signals(operation: &gio::MountOperation, prompts: &Rc<Inner>) {
    connect_ask_password(operation, Rc::downgrade(prompts));
    connect_questions(operation, Rc::downgrade(prompts));
    let aborted = Rc::downgrade(prompts);
    operation.connect_aborted(move |operation| {
        if let Some(prompts) = aborted.upgrade() {
            prompts.on_aborted(operation);
        }
    });
}

fn connect_ask_password(operation: &gio::MountOperation, prompts: Weak<Inner>) {
    operation.connect_ask_password(move |operation, _message, username, domain, flags| {
        // GIO's default handler replies UNHANDLED from an idle callback,
        // which would fail the mount before the user answers.
        operation.stop_signal_emission_by_name("ask-password");
        let Some(prompts) = prompts.upgrade() else {
            send_reply(operation, MountReply::Abort);
            return;
        };
        let request = PasswordRequest {
            username: username.to_owned(),
            domain: domain.to_owned(),
            flags,
        };
        prompts.on_ask_password(operation, request);
    });
}

/// `ask-question` (certificate or host choices) and `show-processes`
/// (programs keeping a mount busy) both become a question. GIO's Rust
/// bindings do not wrap these two signals, so their arguments are read
/// from the signal values.
fn connect_questions(operation: &gio::MountOperation, prompts: Weak<Inner>) {
    let asking = prompts.clone();
    operation.connect_local("ask-question", false, move |values| {
        let (operation, message) = operation_and_message(values)?;
        operation.stop_signal_emission_by_name("ask-question");
        let question = QuestionChallenge::bounded(&message, strings_at(values, 2));
        match asking.upgrade() {
            Some(prompts) => prompts.on_question(&operation, question, QuestionSource::Server),
            None => send_reply(&operation, MountReply::Abort),
        }
        None
    });
    operation.connect_local("show-processes", false, move |values| {
        let (operation, message) = operation_and_message(values)?;
        operation.stop_signal_emission_by_name("show-processes");
        // Explains a busy mount; never kills a process or forces an unmount.
        let question = QuestionChallenge::bounded(&message, strings_at(values, 3));
        match prompts.upgrade() {
            Some(prompts) => prompts.on_question(&operation, question, QuestionSource::BusyMount),
            None => send_reply(&operation, MountReply::Abort),
        }
        None
    });
}

/// Who asks a question, which decides the host the dialog names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QuestionSource {
    /// The server, for example about its certificate.
    Server,
    /// GIO, listing the programs that keep a mount busy.
    BusyMount,
}

/// The emitting operation and the message of a question signal.
fn operation_and_message(values: &[glib::Value]) -> Option<(gio::MountOperation, String)> {
    let operation = values.first()?.get::<gio::MountOperation>().ok()?;
    let message = values.get(1)?.get::<Option<String>>().ok()?.unwrap_or_default();
    Some((operation, message))
}

/// The string list at `index` of a signal's values, empty if absent.
fn strings_at(values: &[glib::Value], index: usize) -> Vec<String> {
    let strings = values.get(index).and_then(|value| value.get::<glib::StrV>().ok());
    strings.map_or_else(Vec::new, |strings| {
        strings.iter().map(ToString::to_string).collect()
    })
}
