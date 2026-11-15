// SPDX-License-Identifier: AGPL-3.0-only
//! Passwords of password-protected ZIPs.
//!
//! As in Windows 11 File Explorer, a password-protected ZIP is browsed
//! without a password: its names and sizes are not encrypted. Opening a
//! file in it, Extract all, Extract here and copying or dragging items out
//! ask for the password with "Password needed" when a file to read is
//! encrypted; a wrong password asks again with the reason, and Cancel
//! stops without a message. Nothing is written before the password is
//! right (ox-core checks it first).
//!
//! The password that worked is remembered for that archive, in memory
//! only, until the window closes, so opening a second file does not ask
//! again. It is never saved, logged or shown.

use std::future::Future;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::archive::{ArchiveError, ArchivePassword};

use super::BrowserWindow;
use crate::dialog::Dialog;
use crate::window::ButtonStyle;

/// The dialog's title.
const TITLE: &str = crate::i18n::message_id("Password needed");
/// The label of the password field.
const PASSWORD_LABEL: &str = crate::i18n::message_id("Password:");
/// The error line after a wrong password.
const WRONG_PASSWORD: &str = crate::i18n::message_id("The password is incorrect. Try again.");

/// The dialog's message for the archive named `name`.
pub(super) fn password_question(name: &str) -> String {
    ox_core::i18n::format_message(
        "“{name}” is password protected. Enter the password below.",
        &[("name", name)],
    )
}

impl BrowserWindow {
    /// Runs `attempt` with the password known for the archive at
    /// `archive_uri`, if any, and while it needs one or the one given is
    /// wrong, asks for it and runs `attempt` again. The password that
    /// worked is remembered for the archive. Cancel in the dialog gives
    /// [`ArchiveError::Cancelled`], which callers pass over in silence.
    ///
    /// # Errors
    ///
    /// `attempt`'s error other than a missing or wrong password, or
    /// [`ArchiveError::Cancelled`].
    pub(super) async fn with_archive_password<T, Attempt, Outcome>(
        &self,
        archive_uri: &str,
        mut attempt: Attempt,
    ) -> Result<T, ArchiveError>
    where
        Attempt: FnMut(Option<ArchivePassword>) -> Outcome,
        Outcome: Future<Output = Result<T, ArchiveError>>,
    {
        let mut password = self.known_archive_password(archive_uri);
        loop {
            match attempt(password.clone()).await {
                Ok(value) => {
                    if let Some(password) = password {
                        self.remember_archive_password(archive_uri, password);
                    }
                    return Ok(value);
                }
                Err(error @ (ArchiveError::PasswordRequired | ArchiveError::WrongPassword)) => {
                    // A remembered password that stopped working is not
                    // reported as the user's mistake.
                    let was_typed = password.is_some() && !self.is_remembered(archive_uri, password.as_ref());
                    self.forget_archive_password(archive_uri);
                    let is_retry = error == ArchiveError::WrongPassword && was_typed;
                    let name = self.imp().locations.borrow().base_name(archive_uri);
                    match self.ask_archive_password(&name, is_retry).await {
                        Some(typed) => password = Some(typed),
                        None => return Err(ArchiveError::Cancelled),
                    }
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Asks for the password of the archive named `name`; `is_retry` says
    /// the last one was wrong. `None` when the user cancelled.
    pub(super) async fn ask_archive_password(&self, name: &str, is_retry: bool) -> Option<ArchivePassword> {
        let dialog = Dialog::new(
            self,
            ox_core::i18n::gettext_static(TITLE),
            &password_question(name),
        );
        let field = gtk::PasswordEntry::builder()
            .show_peek_icon(true)
            .activates_default(true)
            .build();
        field.update_property(&[gtk::accessible::Property::Label(ox_core::i18n::gettext_static(
            PASSWORD_LABEL,
        ))]);
        dialog.add_labelled(ox_core::i18n::gettext_static(PASSWORD_LABEL), &field);
        dialog.add_cancel_button();
        let ok = dialog.add_button(&ox_core::i18n::gettext("OK"), ButtonStyle::Accent);
        if is_retry {
            dialog.show_error(ox_core::i18n::gettext_static(WRONG_PASSWORD));
        }
        dialog.open_focusing(&field);
        let answer = dialog.next_response().await;
        let typed = ArchivePassword::from(field.text().as_str());
        // No password stays in a closed dialog.
        field.set_text("");
        if answer == Some(ok) {
            dialog.finish();
            Some(typed)
        } else {
            None
        }
    }

    /// The password that last worked for the archive at `uri`.
    fn known_archive_password(&self, uri: &str) -> Option<ArchivePassword> {
        self.imp().archive_passwords.borrow().get(uri).cloned()
    }

    /// True when `password` is the one remembered for `uri`.
    fn is_remembered(&self, uri: &str, password: Option<&ArchivePassword>) -> bool {
        let remembered = self.imp().archive_passwords.borrow();
        password.is_some() && remembered.get(uri) == password
    }

    /// Remembers `password` for the archive at `uri` until the window
    /// closes.
    fn remember_archive_password(&self, uri: &str, password: ArchivePassword) {
        self.imp()
            .archive_passwords
            .borrow_mut()
            .insert(uri.to_owned(), password);
    }

    /// Forgets the password of the archive at `uri`.
    fn forget_archive_password(&self, uri: &str) {
        self.imp().archive_passwords.borrow_mut().remove(uri);
    }

    /// Forgets every archive password, as the window closes.
    pub(super) fn forget_archive_passwords(&self) {
        self.imp().archive_passwords.borrow_mut().clear();
    }
}
