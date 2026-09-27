// SPDX-License-Identifier: AGPL-3.0-only
//! "About this build" in the More options menu.
//!
//! Ports the `About this build` item of the More menu in
//! `desktop/ui/app.js`, which shows a message box describing the build.
//! The text says what the native preview is and what it cannot do yet.

use super::BrowserWindow;

/// The heading of the message box.
const HEADING: &str = concat!("OpenXplorer ", env!("CARGO_PKG_VERSION"), " native preview");

/// What the message box says about the build.
const DETAIL: &str = "An independent Windows 11–inspired file manager for Zorin.\n\n\
Desktop: GTK 4 + GIO/GVfs.\n\n\
This preview browses folders, network locations and devices. File operations, \
cached search, the Settings page and network sign-in are not ported yet; the \
installed OpenXplorer keeps doing those.";

impl BrowserWindow {
    /// Shows the build's description in a message box over the window.
    pub(super) fn show_about(&self) {
        let dialog = gtk::AlertDialog::builder()
            .message(HEADING)
            .detail(DETAIL)
            .modal(true)
            .build();
        dialog.show(Some(self));
    }
}
