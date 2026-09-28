// SPDX-License-Identifier: AGPL-3.0-only
//! The sign-in dialog and its queue, against `renderAuth`, `submitAuth`,
//! `receiveAuth` and `dismissAuth` in `desktop/ui/app.js` and the
//! `desktop/tests/ui_release.py` checks of the dialog. ox-core's real
//! prompts ask through the queue: each test emits `GVfs`'s signals on a
//! `gio::MountOperation` and reads its replies, with an in-memory keyring,
//! so nothing is mounted.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gtk::{gio, glib};
use ox_core::network::{
    Challenge, ChallengeKind, CredentialStore, MountOutcome, MountPrompts, PasswordChallenge,
    SessionCredentials, SignInPrompter,
};

use super::*;
use crate::dialogs::{NetworkFormDialog, SignInDialog};
use crate::test_support::harness::{descendants, settle, wait_until};

/// What `GVfs` asks for on a first sign-in to `nas` (the Python fixture's
/// flags, `7`): a user name and a password, which the server can save.
const SIGN_IN_FLAGS: gio::AskPasswordFlags = gio::AskPasswordFlags::NEED_USERNAME
    .union(gio::AskPasswordFlags::NEED_PASSWORD)
    .union(gio::AskPasswordFlags::SAVING_SUPPORTED);

/// The replies an operation sent to `GVfs`, oldest first.
type Replies = Rc<RefCell<Vec<gio::MountOperationResult>>>;

/// A window whose sign-in queue answers ox-core's prompts, as a browser
/// window's does.
struct PromptsFixture {
    window: gtk::Window,
    queue: Rc<SignInQueue>,
    prompts: MountPrompts,
}

impl PromptsFixture {
    fn new() -> Self {
        let window = gtk::Window::new();
        window.present();
        let queue = SignInQueue::new(&window);
        let credentials = Arc::new(SessionCredentials::new(Arc::new(CredentialStore::memory_only())));
        let prompts = MountPrompts::new(credentials, Rc::clone(&queue) as Rc<dyn SignInPrompter>);
        let answering = prompts.clone();
        queue.answer_with(move |id, answer| answering.answer(id, answer));
        Self {
            window,
            queue,
            prompts,
        }
    }

    /// A mount of `smb://nas/share` whose replies are recorded.
    fn mount(&self) -> (gio::MountOperation, Replies) {
        let operation = self.prompts.create("smb://nas/share").expect("an SMB share");
        let replies = Replies::default();
        let recorded = Rc::clone(&replies);
        operation.connect_reply(move |_, result| recorded.borrow_mut().push(result));
        (operation, replies)
    }

    /// `GVfs` asks `operation` for a password with `flags`, and the
    /// dialog it leads to is shown.
    fn ask_password(&self, operation: &gio::MountOperation, flags: gio::AskPasswordFlags) -> SignInDialog {
        operation.emit_by_name::<()>("ask-password", &[&"", &"sam", &"WORKGROUP", &flags]);
        wait_until("the sign-in dialog", || self.queue.shown_dialog().is_some());
        self.shown()
    }

    /// The dialog on screen.
    fn shown(&self) -> SignInDialog {
        self.queue.shown_dialog().expect("a sign-in dialog is shown")
    }
}

impl Drop for PromptsFixture {
    fn drop(&mut self) {
        // Closing the prompts dismisses every dialog; the queue lets go
        // of the prompts it answered through.
        self.prompts.close();
        self.queue.answer_with(|_, _| Err(SignInError::Expired));
        self.window.close();
        settle();
    }
}

/// Every text `widget` shows.
fn texts_of(widget: &impl IsA<gtk::Widget>) -> Vec<String> {
    let labels = descendants::<gtk::Label>(widget);
    let shown = labels.iter().filter(|label| label.is_mapped());
    shown.map(|label| label.text().to_string()).collect()
}

/// Asserts that `widget` shows each of `expected`.
fn assert_shows(widget: &impl IsA<gtk::Widget>, expected: &[&str]) {
    let texts = texts_of(widget);
    for text in expected {
        assert!(texts.iter().any(|shown| shown == text), "{text} in {texts:?}");
    }
}

/// Ported from `desktop/tests/ui_release.py::Credentials default to remembered`
/// and `desktop/tests/ui_release.py::No separate domain field`
///
/// parity: NET-007
#[gtk::test]
fn the_dialog_names_the_server_and_preselects_remember() {
    let fixture = PromptsFixture::new();
    let (operation, _) = fixture.mount();
    let flags = SIGN_IN_FLAGS | gio::AskPasswordFlags::ANONYMOUS_SUPPORTED;

    let dialog = fixture.ask_password(&operation, flags);

    assert_shows(
        &dialog,
        &[
            "OpenXplorer Security",
            "Enter network credentials",
            "Connect to nas",
            "Enter the credentials for this computer or network storage.",
            "Username",
            "Password",
            "Remember my credentials",
            "Saved in your system keyring for future sign-ins.",
            "Connect as guest",
            "Cancel",
            "Connect",
        ],
    );
    let remember = descendants::<gtk::CheckButton>(&dialog);
    assert!(remember[0].is_active() && remember[0].is_sensitive());
    let entries = descendants::<gtk::Entry>(&dialog);
    assert_eq!(entries.len(), 2, "a user name and a password, no domain field");
    assert_eq!(entries[0].text().as_str(), "sam", "the user name is prefilled");
    assert!(!EntryExt::is_visible(&entries[1]), "the password is hidden");
    fixture.prompts.finish(&operation, MountOutcome::Failed);
}

/// Connect hands `GVfs` the account typed, with the password field
/// emptied before the answer leaves the dialog, and the dialog goes.
///
/// Ported from `desktop/tests/ui_release.py::Session-only credential form submits and clears its DOM`
///
/// parity: NET-008, NET-011
#[gtk::test]
fn connect_hands_gvfs_the_account_and_empties_the_password() {
    let fixture = PromptsFixture::new();
    let (operation, replies) = fixture.mount();
    let dialog = fixture.ask_password(&operation, SIGN_IN_FLAGS);

    dialog.type_account("OFFICE\\sam", "not-a-real-password");
    dialog.press_connect();

    assert_eq!(*replies.borrow(), [gio::MountOperationResult::Handled]);
    assert_eq!(operation.username().as_deref(), Some("sam"));
    assert_eq!(operation.domain().as_deref(), Some("OFFICE"));
    assert_eq!(operation.password_save(), gio::PasswordSave::Permanently);
    assert_eq!(dialog.password_text(), "");
    assert!(fixture.queue.shown_dialog().is_none(), "the dialog is dismissed");
    fixture.prompts.finish(&operation, MountOutcome::Failed);
}

/// parity: NET-008, NET-011
#[gtk::test]
fn a_refused_answer_keeps_the_dialog_open_with_the_reason() {
    let fixture = PromptsFixture::new();
    let (operation, replies) = fixture.mount();
    let dialog = fixture.ask_password(&operation, SIGN_IN_FLAGS);

    dialog.type_account("  ", "not-a-real-password");
    dialog.press_connect();
    assert_eq!(dialog.error_text().as_deref(), Some("Enter your username."));
    dialog.type_account("\\sam", "not-a-real-password");
    dialog.press_connect();

    assert_eq!(
        dialog.error_text().as_deref(),
        Some("Enter a username, or use DOMAIN\\username.")
    );
    assert!(replies.borrow().is_empty(), "GVfs heard nothing yet");
    assert_eq!(fixture.queue.shown_dialog().as_ref(), Some(&dialog));
    assert_eq!(
        dialog.connect_label(),
        ("Connect".to_owned(), true),
        "Connect works again"
    );
    fixture.prompts.finish(&operation, MountOutcome::Failed);
}

/// Unchecked, the note says the account is kept for the login session,
/// and `GVfs` is asked to keep it for exactly that.
///
/// Ported from `desktop/tests/test_v05.py::AuthTests::test_unchecked_session_not_never`
///
/// parity: NET-007, NET-011
#[gtk::test]
fn unchecking_remember_keeps_the_account_for_the_session_only() {
    let fixture = PromptsFixture::new();
    let (operation, _) = fixture.mount();
    let dialog = fixture.ask_password(&operation, SIGN_IN_FLAGS);

    dialog.set_remember(false);
    assert_shows(
        &dialog,
        &["Reused for this server during your Linux login session. Not saved permanently."],
    );
    dialog.type_account("sam", "not-a-real-password");
    dialog.press_connect();

    assert_eq!(operation.password_save(), gio::PasswordSave::ForSession);
    fixture.prompts.finish(&operation, MountOutcome::Failed);
}

/// A rejected account shows the dialog again, saying why; a server that
/// cannot save offers no Remember.
///
/// parity: NET-007
#[gtk::test]
fn a_retry_says_why_and_a_server_that_cannot_save_offers_no_remember() {
    let fixture = PromptsFixture::new();
    let (operation, _) = fixture.mount();
    let flags = gio::AskPasswordFlags::NEED_USERNAME | gio::AskPasswordFlags::NEED_PASSWORD;
    let first = fixture.ask_password(&operation, flags);
    first.type_account("sam", "wrong-password");
    first.press_connect();

    let retry = fixture.ask_password(&operation, flags);

    assert_shows(
        &retry,
        &["The previous sign-in was not accepted. Check your username and password."],
    );
    let remember = descendants::<gtk::CheckButton>(&retry);
    assert!(!remember[0].is_active() && !remember[0].is_sensitive());
    fixture.prompts.finish(&operation, MountOutcome::Failed);
}

/// A server's question shows its message and one button per answer; the
/// chosen one reaches `GVfs`.
///
/// parity: NET-013
#[gtk::test]
fn a_question_offers_its_answers_and_cancel() {
    let fixture = PromptsFixture::new();
    let (operation, replies) = fixture.mount();
    let choices = glib::StrV::from(vec!["Connect anyway", "Stop"]);

    operation.emit_by_name::<()>("ask-question", &[&"The server's identity changed.", &choices]);
    let dialog = fixture.shown();
    assert_shows(
        &dialog,
        &[
            "Network connection",
            "The server's identity changed.",
            "Connect anyway",
            "Stop",
            "Cancel",
        ],
    );
    dialog.press_choice(1);

    assert_eq!(*replies.borrow(), [gio::MountOperationResult::Handled]);
    assert_eq!(operation.choice(), 1);
    fixture.prompts.finish(&operation, MountOutcome::Failed);
}

/// Challenges wait their turn and a duplicate is ignored; a waiting one
/// that `GVfs` gives up on leaves the queue, and closing the one on
/// screen shows the next.
///
/// parity: NET-010
#[gtk::test]
fn challenges_are_shown_one_at_a_time_in_order() {
    let fixture = PromptsFixture::new();
    let [first, second, third] = [fixture.mount().0, fixture.mount().0, fixture.mount().0];
    let shown_first = fixture.ask_password(&first, SIGN_IN_FLAGS);
    for waiting in [&second, &third] {
        waiting.emit_by_name::<()>("ask-password", &[&"", &"sam", &"WORKGROUP", &SIGN_IN_FLAGS]);
    }
    wait_until("two challenges to wait", || fixture.queue.waiting_count() == 2);

    fixture.queue.show_challenge(&duplicate_of_the_shown(&fixture));
    assert_eq!(fixture.queue.waiting_count(), 2, "the duplicate is ignored");
    fixture.prompts.finish(&second, MountOutcome::Failed);
    assert_eq!(
        fixture.queue.waiting_count(),
        1,
        "a dismissed challenge leaves the queue"
    );
    shown_first.close();

    let shown_next = fixture.shown();
    assert_ne!(shown_first, shown_next);
    assert_eq!(fixture.queue.waiting_count(), 0);
    fixture.prompts.finish(&first, MountOutcome::Failed);
    fixture.prompts.finish(&third, MountOutcome::Failed);
}

/// The challenge on screen once more, as a duplicate `authRequest` would
/// bring it.
fn duplicate_of_the_shown(fixture: &PromptsFixture) -> Challenge {
    Challenge {
        id: fixture.queue.shown_id().expect("a challenge on screen"),
        host: "nas".into(),
        kind: ChallengeKind::Password(PasswordChallenge {
            uri: "smb://nas/share".into(),
            username: String::new(),
            needs_username: true,
            needs_password: true,
            can_save: true,
            can_sign_in_as_guest: false,
            is_retry: false,
        }),
    }
}

/// Closing the dialog (×, Escape or the window's close) cancels its
/// challenge only, which aborts that mount.
///
/// Ported from `desktop/tests/test_v05.py::AuthTests::test_cancelled_challenge_dismisses`
///
/// parity: NET-009, NET-012
#[gtk::test]
fn closing_the_dialog_aborts_its_mount() {
    let fixture = PromptsFixture::new();
    let (operation, replies) = fixture.mount();
    let dialog = fixture.ask_password(&operation, SIGN_IN_FLAGS);

    dialog.close();

    assert_eq!(*replies.borrow(), [gio::MountOperationResult::Aborted]);
    assert!(fixture.queue.shown_dialog().is_none());
    fixture.prompts.finish(&operation, MountOutcome::Failed);
}

/// The sign-in dialog opens over the window's own dialog, such as Map
/// network location, which stays open underneath.
///
/// parity: NET-009
#[gtk::test]
fn the_dialog_opens_over_the_windows_own_dialog() {
    let fixture = PromptsFixture::new();
    let map = NetworkFormDialog::new(&fixture.window, "Map network location", "", "Connect");
    map.present();
    let (operation, _) = fixture.mount();

    let dialog = fixture.ask_password(&operation, SIGN_IN_FLAGS);

    assert_eq!(
        dialog.transient_for().as_ref(),
        Some(map.upcast_ref::<gtk::Window>())
    );
    fixture.prompts.finish(&operation, MountOutcome::Failed);
    assert!(map.is_visible(), "Map network location stays open");
    map.close();
}

/// Enter in a field presses Connect; the user name has focus when the
/// dialog opens, and the eye shows and hides the password.
///
/// Ported from `desktop/tests/ui_v093.py::Delayed autofocus does not steal password input`
///
/// parity: NET-007, NET-008, NET-009
#[gtk::test]
fn enter_connects_and_the_eye_reveals_the_password() {
    let fixture = PromptsFixture::new();
    let (operation, replies) = fixture.mount();
    let dialog = fixture.ask_password(&operation, SIGN_IN_FLAGS);
    let entries = descendants::<gtk::Entry>(&dialog);
    assert!(dialog.focus_is_in(&entries[0]), "the user name has focus");
    assert_eq!((entries[0].max_length(), entries[1].max_length()), (512, 16_384));

    assert_eq!(dialog.reveal_password_once(), ("Hide password".to_owned(), true));
    assert_eq!(dialog.reveal_password_once(), ("Show password".to_owned(), false));
    dialog.type_account("sam", "not-a-real-password");
    let password_text = descendants::<gtk::Text>(&entries[1]);
    password_text[0].emit_by_name::<()>("activate", &[]);
    // A button pressed from the keyboard shows its press before it clicks.
    wait_until("Connect", || !replies.borrow().is_empty());

    assert_eq!(*replies.borrow(), [gio::MountOperationResult::Handled]);
    fixture.prompts.finish(&operation, MountOutcome::Failed);
}

/// Connect as guest signs in anonymously, where the server offers it.
///
/// parity: NET-011
#[gtk::test]
fn connect_as_guest_signs_in_anonymously() {
    let fixture = PromptsFixture::new();
    let (operation, replies) = fixture.mount();
    let flags = SIGN_IN_FLAGS | gio::AskPasswordFlags::ANONYMOUS_SUPPORTED;
    let dialog = fixture.ask_password(&operation, flags);

    dialog.press_guest();

    assert_eq!(*replies.borrow(), [gio::MountOperationResult::Handled]);
    assert!(operation.is_anonymous());
    assert_eq!(operation.password_save(), gio::PasswordSave::Never);
    fixture.prompts.finish(&operation, MountOutcome::Failed);
}
