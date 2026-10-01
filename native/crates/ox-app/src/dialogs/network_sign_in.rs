// SPDX-License-Identifier: AGPL-3.0-only
//! The network sign-in dialog: "Enter network credentials", or a server's
//! question under "Network connection".
//!
//! Ports `renderAuth`, `submitAuth`, `answerAuth` and `authKeys` in
//! `v2.0.0:desktop/ui/app.js`. The layout is the template
//! `resources/ui/network-sign-in.ui`. One dialog shows one ox-core
//! [`Challenge`] and reports the user's [`Answer`] to the handler of
//! [`SignInDialog::connect_answered`]; the window's
//! [`SignInQueue`](crate::network::SignInQueue) decides what happens next.
//!
//! The rules of `renderAuth`, each enforced where noted:
//!
//! - Privacy rule (NET-008, SAFE-011): the password is read and its field
//!   emptied before the answer leaves the dialog, and emptied again when
//!   the dialog goes; the field keeps it in non-pageable memory
//!   (`GtkPasswordEntryBuffer`).
//! - "Remember my credentials" is checked, and can be changed, only where
//!   the server can save a password (NET-007, NET-011).
//! - The dialog is modal, so it traps Tab and blocks the window's
//!   shortcuts; Escape, × and Cancel answer [`Answer::Cancel`] for this
//!   challenge only, and Enter in a field presses Connect (NET-009).

use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use ox_core::network::{
    Answer, Challenge, ChallengeKind, CredentialScope, Password, PasswordChallenge, QuestionChallenge, SignIn,
};

use crate::icons::{self, Icon};

/// The size of the caption bar's shield (`icon('shield',17)`).
const CAPTION_GLYPH: i32 = 17;
/// The size of the ×.
const CLOSE_GLYPH: i32 = 12;
/// The size of the computer in the badge (`icon('desktop',29)`).
const DEVICE_GLYPH: i32 = 29;
/// The size of the glyph before the server's name (`icon('network',16)`).
const TARGET_GLYPH: i32 = 16;
/// The size of the eye (`.auth-eye svg`).
const REVEAL_GLYPH: i32 = 17;
/// The size of the note's shield (`icon('shield',14)`).
const NOTE_GLYPH: i32 = 13;

/// What the description says on a first sign-in.
const FIRST_SIGN_IN: &str = "Enter the credentials for this computer or network storage.";
/// What it says after the server refused the previous account.
const RETRY_SIGN_IN: &str = "The previous sign-in was not accepted. Check your username and password.";
/// The note while "Remember my credentials" is checked.
const SAVED_PERMANENTLY: &str = "Saved in your system keyring for future sign-ins.";
/// The note while it is unchecked, or cannot be checked.
const SAVED_FOR_SESSION: &str =
    "Reused for this server during your Linux login session. Not saved permanently.";
/// Connect's label while the answer is checked.
const CONNECTING: &str = "Connecting…";

/// A handler of [`SignInDialog::connect_answered`].
type AnswerHandler = Rc<dyn Fn(&SignInDialog, Answer)>;

mod imp;

glib::wrapper! {
    /// The dialog that asks one network sign-in or question.
    pub(crate) struct SignInDialog(ObjectSubclass<imp::SignInDialog>)
        @extends gtk::Window, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native,
            gtk::Root, gtk::ShortcutManager;
}

impl SignInDialog {
    /// A dialog over `parent` that asks what `challenge` asks. Present it
    /// once a handler hears its answers.
    pub(crate) fn new(parent: &impl IsA<gtk::Window>, challenge: &Challenge) -> Self {
        let dialog: Self = glib::Object::builder().property("transient-for", parent).build();
        match &challenge.kind {
            ChallengeKind::Password(fields) => dialog.ask_for_account(&challenge.host, fields),
            ChallengeKind::Question(question) => dialog.ask_question(question),
        }
        dialog
    }

    /// Calls `on_answer` with each answer: Connect, Connect as guest, a
    /// question's button, or Cancel (the Cancel button, ×, Escape or
    /// closing the window). The dialog stays open until
    /// [`Self::dismiss`].
    pub(crate) fn connect_answered(&self, on_answer: impl Fn(&Self, Answer) + 'static) {
        self.imp().answered.replace(Some(Rc::new(on_answer)));
    }

    /// Shows why the answer was refused, and lets the user try again.
    pub(crate) fn show_error(&self, message: &str) {
        let imp = self.imp();
        imp.error_label.set_text(message);
        imp.error_label.set_visible(true);
        imp.connect_button.set_sensitive(true);
        imp.connect_button.set_label("Connect");
    }

    /// Shows "Connecting…" on a disabled Connect while the answer is
    /// checked.
    fn show_connecting(&self) {
        let connect = &self.imp().connect_button;
        connect.set_sensitive(false);
        connect.set_label(CONNECTING);
    }

    /// Closes the dialog for good: its challenge was answered, replaced,
    /// expired or its mount aborted.
    pub(crate) fn dismiss(&self) {
        let imp = self.imp();
        imp.is_dismissed.set(true);
        // Privacy rule (NET-008): no password stays in a closed dialog.
        imp.password_entry.set_text("");
        imp.answered.take();
        self.close();
    }

    /// Sets the icons, which only [`Icon`] names.
    fn show_icons(&self) {
        let imp = self.imp();
        icons::set_icon(&imp.caption_glyph, Icon::ShieldLock, CAPTION_GLYPH);
        icons::set_icon(&imp.close_glyph, Icon::Dismiss, CLOSE_GLYPH);
        icons::set_icon(&imp.device_glyph, Icon::Desktop, DEVICE_GLYPH);
        icons::set_icon(&imp.target_glyph, Icon::Organization, TARGET_GLYPH);
        icons::set_icon(&imp.reveal_glyph, Icon::Eye, REVEAL_GLYPH);
        icons::set_icon(&imp.note_glyph, Icon::ShieldLock, NOTE_GLYPH);
    }

    /// Connects the buttons, the check box, the eye and Escape.
    fn connect_controls(&self) {
        let imp = self.imp();
        let cancel = glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_: &gtk::Button| dialog.answer(Answer::Cancel)
        );
        imp.close_button.connect_clicked(cancel.clone());
        imp.cancel_button.connect_clicked(cancel);
        imp.connect_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.submit_account()
        ));
        imp.guest_button.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.answer(Answer::Guest)
        ));
        imp.remember_check.connect_toggled(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.describe_saving()
        ));
        imp.reveal_button.connect_toggled(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.follow_the_eye()
        ));
        self.add_controller(self.escape_cancels());
        // The text-size keys still reach the window that owns the dialog
        // (NET-009), as `textSizeKeys` sits ahead of the sign-in layer.
        crate::window::follow_text_size_keys(self);
        self.set_default_widget(Some(&*imp.connect_button));
        self.connect_map(Self::select_username);
    }

    /// Escape answers Cancel for this challenge only.
    fn escape_cancels(&self) -> gtk::ShortcutController {
        let action = gtk::CallbackAction::new(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, _| {
                dialog.answer(Answer::Cancel);
                glib::Propagation::Stop
            }
        ));
        let trigger = gtk::KeyvalTrigger::new(gdk::Key::Escape, gdk::ModifierType::empty());
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(action)));
        shortcuts
    }

    /// Fills the password page for a sign-in to `host`.
    fn ask_for_account(&self, host: &str, fields: &PasswordChallenge) {
        let imp = self.imp();
        self.set_title(Some("Enter network credentials"));
        imp.pages.set_visible_child_name("password");
        imp.host_label.set_text(&format!("Connect to {host}"));
        imp.target_label.set_text(host);
        let description = if fields.is_retry {
            imp.description_label.add_css_class("sign-in-retry");
            RETRY_SIGN_IN
        } else {
            FIRST_SIGN_IN
        };
        imp.description_label.set_text(description);
        imp.username_entry.set_text(&fields.username);
        imp.needs_username.set(fields.needs_username);
        // Remembering is the default, where the server can save at all.
        imp.remember_check.set_active(fields.can_save);
        imp.remember_check.set_sensitive(fields.can_save);
        self.describe_saving();
        imp.guest_button.set_visible(fields.can_sign_in_as_guest);
        GtkWindowExt::set_focus(self, Some(&*imp.username_entry));
    }

    /// Fills the question page: the message, one button per answer and
    /// Cancel, which replace the footer.
    fn ask_question(&self, question: &QuestionChallenge) {
        let imp = self.imp();
        self.set_title(Some("Network connection"));
        imp.pages.set_visible_child_name("question");
        imp.question_label.set_text(&question.message);
        imp.actions.set_visible(false);
        for (index, label) in question.choices.iter().enumerate() {
            let choice = gtk::Button::builder()
                .label(label)
                .css_classes(["bordered"])
                .build();
            choice.connect_clicked(glib::clone!(
                #[weak(rename_to = dialog)]
                self,
                move |_| dialog.answer(Answer::Choice(index))
            ));
            self.append_choice(&choice);
        }
        let cancel = gtk::Button::builder()
            .label("Cancel")
            .css_classes(["bordered"])
            .build();
        cancel.connect_clicked(glib::clone!(
            #[weak(rename_to = dialog)]
            self,
            move |_| dialog.answer(Answer::Cancel)
        ));
        self.append_choice(&cancel);
    }

    /// Adds `button` to the question's answers. Only the button takes
    /// keyboard focus, not the flow box's cell around it.
    fn append_choice(&self, button: &gtk::Button) {
        let choices = &self.imp().choices;
        choices.append(button);
        if let Some(cell) = button.parent() {
            cell.set_focusable(false);
        }
        if GtkWindowExt::focus(self).is_none() {
            GtkWindowExt::set_focus(self, Some(button));
        }
    }

    /// Says where the credentials will be kept, as the check box says.
    fn describe_saving(&self) {
        let imp = self.imp();
        let note = if imp.remember_check.is_active() {
            SAVED_PERMANENTLY
        } else {
            SAVED_FOR_SESSION
        };
        imp.note_label.set_text(note);
    }

    /// Shows the password as text while the eye is pressed, and hides it
    /// again when it is released.
    fn follow_the_eye(&self) {
        let imp = self.imp();
        let revealed = imp.reveal_button.is_active();
        imp.password_entry.set_visibility(revealed);
        let tooltip = if revealed {
            "Hide password"
        } else {
            "Show password"
        };
        imp.reveal_button.set_tooltip_text(Some(tooltip));
    }

    /// Connect: checks that a needed user name was entered, then answers
    /// with the account (`submitAuth`).
    fn submit_account(&self) {
        let imp = self.imp();
        let username = imp.username_entry.text();
        if imp.needs_username.get() && username.trim().is_empty() {
            self.show_error("Enter your username.");
            imp.username_entry.grab_focus();
            return;
        }
        // Privacy rule (NET-008): the field is emptied before the answer
        // leaves the dialog.
        let password = Password::from(imp.password_entry.text().as_str());
        imp.password_entry.set_text("");
        let scope = if imp.remember_check.is_active() {
            CredentialScope::Permanent
        } else {
            CredentialScope::Session
        };
        let sign_in = SignIn {
            username: username.into(),
            password,
            scope,
        };
        self.answer(Answer::SignIn(sign_in));
    }

    /// Hands `answer` to the handler, with Connect disabled meanwhile.
    fn answer(&self, answer: Answer) {
        let handler = self.imp().answered.borrow().clone();
        let Some(handler) = handler else {
            return;
        };
        self.show_connecting();
        handler(self, answer);
    }

    /// Selects the prefilled user name once the dialog is shown, as the
    /// delayed focus of `renderAuth` does, so typing replaces it.
    fn select_username(&self) {
        let username = &*self.imp().username_entry;
        let has_focus = GtkWindowExt::focus(self).is_some_and(|focus| focus.is_ancestor(username));
        if has_focus {
            username.select_region(0, -1);
        }
    }
}

/// What tests read and do, as a user would.
#[cfg(test)]
impl SignInDialog {
    /// Types `username` and `password` into their fields.
    pub(crate) fn type_account(&self, username: &str, password: &str) {
        self.imp().username_entry.set_text(username);
        self.imp().password_entry.set_text(password);
    }

    /// Unchecks or checks "Remember my credentials".
    pub(crate) fn set_remember(&self, remember: bool) {
        self.imp().remember_check.set_active(remember);
    }

    /// Presses Connect.
    pub(crate) fn press_connect(&self) {
        self.imp().connect_button.emit_clicked();
    }

    /// Presses Connect as guest.
    pub(crate) fn press_guest(&self) {
        self.imp().guest_button.emit_clicked();
    }

    /// Presses the eye once; returns its tooltip and whether the password
    /// shows.
    pub(crate) fn reveal_password_once(&self) -> (String, bool) {
        let imp = self.imp();
        let eye = &imp.reveal_button;
        eye.set_active(!eye.is_active());
        let tooltip = eye
            .tooltip_text()
            .map(|text| text.to_string())
            .unwrap_or_default();
        (tooltip, EntryExt::is_visible(&*imp.password_entry))
    }

    /// Whether keyboard focus is in `widget`.
    pub(crate) fn focus_is_in(&self, widget: &impl IsA<gtk::Widget>) -> bool {
        GtkWindowExt::focus(self)
            .is_some_and(|focus| focus.is_ancestor(widget) || &focus == widget.upcast_ref())
    }

    /// Presses the question's button `index`.
    pub(crate) fn press_choice(&self, index: usize) {
        let buttons = crate::test_support::harness::descendants::<gtk::Button>(&*self.imp().choices);
        buttons[index].emit_clicked();
    }

    /// The error shown, if any.
    pub(crate) fn error_text(&self) -> Option<String> {
        let error = &self.imp().error_label;
        error.is_visible().then(|| error.text().to_string())
    }

    /// What the password field holds.
    pub(crate) fn password_text(&self) -> String {
        self.imp().password_entry.text().to_string()
    }

    /// Connect's label and whether it can be pressed.
    pub(crate) fn connect_label(&self) -> (String, bool) {
        let connect = &self.imp().connect_button;
        let label = connect.label().map(|label| label.to_string()).unwrap_or_default();
        (label, connect.is_sensitive())
    }
}
