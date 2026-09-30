// SPDX-License-Identifier: AGPL-3.0-only
//! Keeping the session from logging out or suspending while files are
//! written (INT-028).
//!
//! A copy, move, delete, extraction or restore holds a
//! [`WriteInhibitor`] from its start to its end, cancelled or not. GTK
//! passes it to the session manager (`org.gnome.SessionManager.Inhibit`,
//! or the Inhibit portal), so GNOME's logout and suspend dialogs say that
//! `OpenXplorer` is busy. This complements the refusal to close a window
//! or quit while files are written.

use gtk::prelude::*;

/// What the logout and suspend dialogs show.
pub(crate) const REASON: &str = "Copying files";

/// An inhibitor held for as long as this value lives.
#[derive(Debug)]
pub(crate) struct WriteInhibitor {
    /// The application that holds it.
    app: gtk::Application,
    /// GTK's cookie; 0 when the session could not be asked, as on a
    /// desktop without a session manager or portal.
    cookie: u32,
}

impl WriteInhibitor {
    /// Asks the session not to log out or suspend on behalf of the window
    /// `widget` is in; `None` for a widget outside an application window.
    pub(crate) fn hold(widget: &impl IsA<gtk::Widget>) -> Option<Self> {
        let window = widget.root()?.downcast::<gtk::Window>().ok()?;
        let app = window.application()?;
        let flags = gtk::ApplicationInhibitFlags::LOGOUT | gtk::ApplicationInhibitFlags::SUSPEND;
        let cookie = app.inhibit(Some(&window), flags, Some(REASON));
        Some(Self { app, cookie })
    }
}

impl Drop for WriteInhibitor {
    fn drop(&mut self) {
        if self.cookie != 0 {
            self.app.uninhibit(self.cookie);
        }
    }
}
