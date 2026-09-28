// SPDX-License-Identifier: AGPL-3.0-only
//! Ports `AuthTests` of `desktop/tests/test_v05.py`. The Python tests drove
//! a fake operation; these emit `GVfs`'s signals on a real
//! `gio::MountOperation` and read its replies, on a private main context,
//! with an in-memory keyring.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use gio::prelude::*;

use super::*;
use crate::network::credential::{Credential, CredentialScope, Password};
use crate::network::keyring::KeyringCollection;
use crate::network::test_support::{
    with_memory_only_prompts, with_prompts, with_prompts_living, PromptsFixture,
};

/// The flags of the Python fixture's challenge (`7`): a user name and a
/// password are needed and saving is supported.
const SIGN_IN_FLAGS: gio::AskPasswordFlags = gio::AskPasswordFlags::NEED_USERNAME
    .union(gio::AskPasswordFlags::NEED_PASSWORD)
    .union(gio::AskPasswordFlags::SAVING_SUPPORTED);

/// The replies an operation sent to `GVfs`, oldest first.
type Replies = Rc<RefCell<Vec<gio::MountOperationResult>>>;

/// An operation for `uri` whose replies are recorded.
fn operation(fixture: &PromptsFixture, uri: &str) -> (gio::MountOperation, Replies) {
    let operation = fixture.prompts.create(uri).expect("a valid SMB location");
    let replies = Replies::default();
    let recorded = Rc::clone(&replies);
    operation.connect_reply(move |_, result| recorded.borrow_mut().push(result));
    (operation, replies)
}

/// The Python fixture's `prompt()`: `GVfs` asks for a password, nothing is
/// known for the server, so the dialog is shown.
fn prompt(fixture: &PromptsFixture) -> (gio::MountOperation, Replies, ChallengeId) {
    let (operation, replies) = operation(fixture, "smb://nas/a");
    let shown_before = fixture.prompter.shown_count();
    ask_password(&operation, SIGN_IN_FLAGS);
    fixture.wait_until("the sign-in dialog", || {
        fixture.prompter.shown_count() > shown_before
    });
    let id = fixture.prompter.last_shown().id;
    (operation, replies, id)
}

/// Signs in as `sam` with `password` in answer to challenge `id`.
fn answer_as_sam(fixture: &PromptsFixture, id: ChallengeId, password: &str, scope: CredentialScope) {
    let answer = Answer::SignIn(SignIn {
        username: "sam".into(),
        password: Password::from(password),
        scope,
    });
    fixture.prompts.answer(id, answer).expect("a valid answer");
}

/// `GVfs` asks `operation` for a password, as `ask-password` from the
/// daemon does.
fn ask_password(operation: &gio::MountOperation, flags: gio::AskPasswordFlags) {
    operation.emit_by_name::<()>("ask-password", &[&"", &"user", &"WORKGROUP", &flags]);
}

fn sam(password: &str) -> Credential {
    Credential {
        username: "sam".into(),
        domain: String::new(),
        password: Password::from(password),
        scope: CredentialScope::Session,
    }
}

/// Ported from `desktop/tests/test_v05.py::AuthTests::test_unchecked_session_not_never`
///
/// parity: NET-007, NET-011
#[test]
fn unchecked_remember_saves_for_the_session_not_never() {
    with_prompts(|fixture| {
        let (operation, replies, id) = prompt(fixture);

        answer_as_sam(fixture, id, "test", CredentialScope::Session);

        assert_eq!(operation.password_save(), gio::PasswordSave::ForSession);
        assert_eq!(*replies.borrow(), [gio::MountOperationResult::Handled]);
    });
}

/// Ported from `desktop/tests/test_v05.py::AuthTests::test_checked_permanent`
///
/// parity: NET-007, NET-011
#[test]
fn checked_remember_saves_permanently() {
    with_prompts(|fixture| {
        let (operation, _replies, id) = prompt(fixture);

        answer_as_sam(fixture, id, "test", CredentialScope::Permanent);

        assert_eq!(operation.password_save(), gio::PasswordSave::Permanently);
        assert_eq!(operation.username().as_deref(), Some("sam"));
        assert_eq!(operation.domain().as_deref(), Some("WORKGROUP"));
    });
}

/// Ported from `desktop/tests/test_v05.py::AuthTests::test_failed_mount_not_saved`
///
/// parity: NET-001, NET-014, NET-015
#[test]
fn a_failed_mount_saves_nothing() {
    with_prompts(|fixture| {
        let (operation, _replies, id) = prompt(fixture);
        answer_as_sam(fixture, id, "test", CredentialScope::Session);

        fixture.prompts.finish(&operation, MountOutcome::Failed);

        assert!(fixture.keyring.saves().is_empty());
        assert_eq!(fixture.credentials.peek("smb://nas/a"), None);
        assert_eq!(operation.password(), None, "the password is cleared");
    });
}

/// Ported from `desktop/tests/test_v05.py::AuthTests::test_success_saved_after_finish`
///
/// parity: NET-001, NET-014, NET-015
#[test]
fn credentials_are_saved_only_after_the_mount_succeeds() {
    with_prompts(|fixture| {
        let (operation, _replies, id) = prompt(fixture);
        answer_as_sam(fixture, id, "test", CredentialScope::Session);
        assert!(fixture.keyring.saves().is_empty());

        fixture.prompts.finish(&operation, MountOutcome::Mounted);

        fixture.wait_until("the keyring save", || !fixture.keyring.saves().is_empty());
        assert_eq!(
            fixture.keyring.last_saved_collection(),
            Some(KeyringCollection::Session)
        );
        assert!(fixture.prompter.notices().is_empty());
    });
}

/// Ported from `desktop/tests/test_v05.py::AuthTests::test_reuse_without_dialog`
///
/// parity: NET-011, NET-014
#[test]
fn another_share_reuses_the_account_without_a_dialog() {
    with_prompts(|fixture| {
        fixture.credentials.accept_memory("smb://nas/a", &sam("test"));
        let (operation, replies) = operation(fixture, "smb://nas/other");

        ask_password(&operation, SIGN_IN_FLAGS);

        assert_eq!(operation.password().as_deref(), Some("test"));
        assert_eq!(*replies.borrow(), [gio::MountOperationResult::Handled]);
        assert_eq!(fixture.prompter.shown_count(), 0);
    });
}

/// A credential saved in the keyring by an earlier session is reused too,
/// read off the main thread.
///
/// parity: NET-014
#[test]
fn a_keyring_credential_is_reused_without_a_dialog() {
    with_prompts(|fixture| {
        let saved = sam("from-keyring");
        let generation = fixture.credentials.generation("smb://nas/a");
        fixture
            .credentials
            .persist("smb://nas/a", &saved, generation)
            .expect("saved");
        let (operation, replies) = operation(fixture, "smb://nas/b");

        ask_password(&operation, SIGN_IN_FLAGS);

        fixture.wait_until("the keyring lookup", || !replies.borrow().is_empty());
        assert_eq!(operation.password().as_deref(), Some("from-keyring"));
        assert_eq!(fixture.prompter.shown_count(), 0);
        fixture.prompts.finish(&operation, MountOutcome::Mounted);
        assert_eq!(
            fixture.keyring.saves().len(),
            1,
            "a known credential is not saved again"
        );
    });
}

/// The Secret Service may show an unlock prompt, so the keyring is read on
/// a worker thread and the window stays responsive meanwhile.
///
/// parity: PERF-003, NET-014
#[test]
fn the_keyring_is_read_off_the_main_thread() {
    with_prompts(|fixture| {
        let (reader, read_on) = mpsc::channel();
        fixture
            .keyring
            .set_lookup_hook(move || reader.send(thread::current().id()).expect("the test waits"));
        let (operation, _replies) = operation(fixture, "smb://nas/a");

        ask_password(&operation, SIGN_IN_FLAGS);
        // Nothing is saved, so the lookup ends with the sign-in dialog.
        fixture.wait_until("the sign-in dialog", || fixture.prompter.shown_count() == 1);

        let lookup_thread = read_on
            .recv_timeout(Duration::from_secs(1))
            .expect("the keyring was asked");
        assert_ne!(lookup_thread, thread::current().id());
    });
}

/// Without a keyring the entered account still signs in the other shares
/// of this session, and the user is told it could not be saved.
///
/// parity: NET-015, SAFE-011
#[test]
fn a_credential_the_keyring_cannot_save_still_works_and_the_user_is_told() {
    with_memory_only_prompts(|fixture| {
        let (operation, _replies, id) = prompt(fixture);
        answer_as_sam(fixture, id, "test", CredentialScope::Session);

        fixture.prompts.finish(&operation, MountOutcome::Mounted);

        fixture.wait_until("the keyring notice", || !fixture.prompter.notices().is_empty());
        assert_eq!(fixture.prompter.notices(), [KEYRING_SAVE_NOTICE]);
        let reused = fixture.credentials.peek("smb://nas/other");
        assert_eq!(
            reused.map(|credential| credential.username),
            Some("sam".to_owned())
        );
    });
}

/// Ported from `desktop/tests/test_v05.py::AuthTests::test_rejected_reuse_shows_prompt_not_loop`
///
/// parity: NET-011, NET-014
#[test]
fn a_rejected_known_credential_shows_the_dialog_instead_of_looping() {
    with_prompts(|fixture| {
        fixture.credentials.accept_memory("smb://nas/a", &sam("test"));
        let (operation, replies) = operation(fixture, "smb://nas/a");

        ask_password(&operation, SIGN_IN_FLAGS);
        ask_password(&operation, SIGN_IN_FLAGS);

        assert_eq!(fixture.prompter.shown_count(), 1);
        assert_eq!(replies.borrow().len(), 1);
        let ChallengeKind::Password(dialog) = fixture.prompter.last_shown().kind else {
            panic!("a sign-in dialog");
        };
        assert!(dialog.is_retry, "the dialog says the previous sign-in failed");
    });
}

/// Ported from `desktop/tests/test_v05.py::AuthTests::test_password_not_sent_back_to_ui`
///
/// parity: NET-008, NET-011, SAFE-011
#[test]
fn the_password_is_never_sent_back_to_the_window() {
    with_prompts(|fixture| {
        let (_operation, _replies, id) = prompt(fixture);

        answer_as_sam(fixture, id, "unique-secret-value", CredentialScope::Session);

        let events = format!("{:?}", fixture.prompter);
        assert!(!events.contains("unique-secret-value"), "{events}");
    });
}

/// Ported from `desktop/tests/test_v05.py::AuthTests::test_cancelled_challenge_dismisses`
///
/// parity: NET-011, NET-012, NET-013
#[test]
fn cancelling_aborts_the_mount_and_dismisses_the_dialog() {
    with_prompts(|fixture| {
        let (_operation, replies, id) = prompt(fixture);

        fixture
            .prompts
            .answer(id, Answer::Cancel)
            .expect("a pending challenge");

        assert_eq!(replies.borrow().last(), Some(&gio::MountOperationResult::Aborted));
        assert!(fixture.prompter.is_dismissed(id));
        assert_eq!(
            fixture.prompts.answer(id, Answer::Cancel),
            Err(SignInError::Expired)
        );
    });
}

/// A missing user name keeps the dialog open with its message; a valid
/// answer then signs in.
///
/// parity: NET-011
#[test]
fn an_invalid_answer_keeps_the_dialog_open() {
    with_prompts(|fixture| {
        let (_operation, replies, id) = prompt(fixture);
        let without_name = Answer::SignIn(SignIn {
            username: "  ".into(),
            password: Password::from("test"),
            scope: CredentialScope::Permanent,
        });

        assert_eq!(
            fixture.prompts.answer(id, without_name),
            Err(SignInError::MissingUsername)
        );
        assert!(replies.borrow().is_empty());
        assert!(!fixture.prompter.is_dismissed(id));

        answer_as_sam(fixture, id, "test", CredentialScope::Permanent);
        assert!(fixture.prompter.is_dismissed(id));
    });
}

/// Guest sign-in is anonymous and saves nothing.
///
/// parity: NET-011
#[test]
fn a_guest_sign_in_saves_nothing() {
    with_prompts(|fixture| {
        let (operation, replies) = operation(fixture, "smb://nas/public");
        ask_password(
            &operation,
            SIGN_IN_FLAGS | gio::AskPasswordFlags::ANONYMOUS_SUPPORTED,
        );
        fixture.wait_until("the sign-in dialog", || fixture.prompter.shown_count() == 1);
        let id = fixture.prompter.last_shown().id;

        fixture
            .prompts
            .answer(id, Answer::Guest)
            .expect("guest is offered");
        fixture.prompts.finish(&operation, MountOutcome::Mounted);

        assert!(operation.is_anonymous());
        assert_eq!(operation.password_save(), gio::PasswordSave::Never);
        assert_eq!(*replies.borrow(), [gio::MountOperationResult::Handled]);
        assert_eq!(fixture.credentials.peek("smb://nas/public"), None);
    });
}

/// Regression: the Python app kept a rejected known credential as the
/// account of a mount that then succeeded as guest, and saved it in the
/// keyring. A guest answer now replaces the rejected account.
///
/// parity: NET-011, NET-015
#[test]
fn a_guest_sign_in_after_a_rejected_credential_saves_nothing() {
    with_prompts(|fixture| {
        fixture.credentials.accept_memory("smb://nas/a", &sam("rejected"));
        let (operation, _replies) = operation(fixture, "smb://nas/a");
        let flags = SIGN_IN_FLAGS | gio::AskPasswordFlags::ANONYMOUS_SUPPORTED;
        ask_password(&operation, flags);
        ask_password(&operation, flags);
        let id = fixture.prompter.last_shown().id;

        fixture
            .prompts
            .answer(id, Answer::Guest)
            .expect("guest is offered");
        fixture.prompts.finish(&operation, MountOutcome::Mounted);

        assert!(fixture.keyring.saves().is_empty());
    });
}

/// A server question is answered with the index of the chosen button.
///
/// parity: NET-013
#[test]
fn a_question_is_answered_with_the_chosen_button() {
    with_prompts(|fixture| {
        let (operation, replies) = operation(fixture, "smb://nas/a");
        let choices = glib::StrV::from(vec!["Connect anyway", "Cancel"]);

        operation.emit_by_name::<()>("ask-question", &[&"Untrusted certificate", &choices]);

        let challenge = fixture.prompter.last_shown();
        assert_eq!(challenge.host, "nas");
        let ChallengeKind::Question(question) = challenge.kind else {
            panic!("a question");
        };
        assert_eq!(question.message, "Untrusted certificate");
        assert_eq!(question.choices, ["Connect anyway", "Cancel"]);
        let refused = fixture.prompts.answer(challenge.id, Answer::Choice(2));
        assert_eq!(refused, Err(SignInError::InvalidChoice));
        fixture
            .prompts
            .answer(challenge.id, Answer::Choice(0))
            .expect("a valid choice");
        assert_eq!(operation.choice(), 0);
        assert_eq!(*replies.borrow(), [gio::MountOperationResult::Handled]);
    });
}

/// An unanswered challenge expires and aborts its mount.
///
/// parity: NET-012
#[test]
fn an_unanswered_challenge_expires_and_aborts_the_mount() {
    with_prompts_living(Duration::from_millis(20), |fixture| {
        let (_operation, replies, id) = prompt(fixture);

        fixture.wait_until("the expiry", || fixture.prompter.is_dismissed(id));

        assert_eq!(*replies.borrow(), [gio::MountOperationResult::Aborted]);
        assert_eq!(
            fixture.prompts.answer(id, Answer::Cancel),
            Err(SignInError::Expired)
        );
    });
}

/// A new challenge from the same mount replaces the old one; another
/// mount's challenge stays.
///
/// parity: NET-012
#[test]
fn a_retry_replaces_only_its_own_mounts_challenge() {
    with_prompts(|fixture| {
        let (first, _first_replies, first_id) = prompt(fixture);
        let (_second, _second_replies, second_id) = prompt(fixture);

        ask_password(&first, SIGN_IN_FLAGS);

        assert!(fixture.prompter.is_dismissed(first_id));
        assert!(!fixture.prompter.is_dismissed(second_id));
        assert_ne!(fixture.prompter.last_shown().id, first_id);
    });
}

/// `GVfs` aborting the mount dismisses its dialog.
///
/// parity: NET-012
#[test]
fn an_aborted_mount_dismisses_its_dialog() {
    with_prompts(|fixture| {
        let (operation, _replies, id) = prompt(fixture);

        operation.emit_by_name::<()>("aborted", &[]);

        assert!(fixture.prompter.is_dismissed(id));
        assert_eq!(
            fixture.prompts.answer(id, Answer::Cancel),
            Err(SignInError::Expired)
        );
    });
}

/// Closing the window aborts every pending prompt.
///
/// parity: NET-012, TAB-050
#[test]
fn closing_aborts_every_pending_prompt() {
    with_prompts(|fixture| {
        let (_operation, replies, id) = prompt(fixture);

        fixture.prompts.close();

        assert!(fixture.prompter.is_dismissed(id));
        assert_eq!(*replies.borrow(), [gio::MountOperationResult::Aborted]);
    });
}

/// Sign out during a mount wins: the account it signed in with is not
/// kept (NET-021).
///
/// parity: SAFE-012
#[test]
fn sign_out_during_the_mount_keeps_nothing() {
    with_prompts(|fixture| {
        let (operation, _replies, id) = prompt(fixture);
        answer_as_sam(fixture, id, "test", CredentialScope::Session);

        fixture.credentials.forget_memory("smb://nas/a");
        fixture.prompts.finish(&operation, MountOutcome::Mounted);

        assert_eq!(fixture.credentials.peek("smb://nas/a"), None);
        assert!(fixture.keyring.saves().is_empty());
    });
}

/// An operation whose mount has finished is aborted if `GVfs` asks again.
#[test]
fn a_finished_operation_is_aborted_when_asked_again() {
    with_prompts(|fixture| {
        let (operation, replies) = operation(fixture, "smb://nas/a");
        fixture.prompts.finish(&operation, MountOutcome::Failed);

        ask_password(&operation, SIGN_IN_FLAGS);

        assert_eq!(*replies.borrow(), [gio::MountOperationResult::Aborted]);
    });
}

/// The dialog shows the server, prefilled user name and `GVfs`'s offers,
/// but not the default domain.
///
/// parity: NET-007
#[test]
fn the_dialog_shows_the_server_and_what_it_offers() {
    with_prompts(|fixture| {
        let (_operation, _replies, _id) = prompt(fixture);

        let challenge = fixture.prompter.last_shown();

        assert_eq!(challenge.host, "nas");
        let expected = PasswordChallenge {
            uri: "smb://nas/a".into(),
            username: "user".into(),
            needs_username: true,
            needs_password: true,
            can_save: true,
            can_sign_in_as_guest: false,
            is_retry: false,
        };
        assert_eq!(challenge.kind, ChallengeKind::Password(expected));
    });
}
