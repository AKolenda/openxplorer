// SPDX-License-Identifier: AGPL-3.0-only
//! "About this build" in the More options menu and the About settings.
//!
//! Ports the `About this build` item of the More menu in
//! `desktop/ui/app.js`, which shows a message box describing the build.
//! The text says what the native preview is and what it cannot do yet.

use super::BrowserWindow;
use crate::config::BUILD_NAME;

/// What the message box says about the build, under its name.
const DETAIL: &str = "An independent Windows 11–inspired file manager for Zorin.\n\n\
Desktop: GTK 4 + GIO/GVfs.\n\n\
This preview browses folders, copies, moves, renames and deletes files, \
signs in to network shares, connects and removes drives and devices, \
searches folders and the search index, and has the Settings page. Desktop \
integration is not ported yet; the installed \
OpenXplorer keeps doing those.";

impl BrowserWindow {
    /// Shows the build's description in a message box over the window.
    pub(super) fn show_about(&self) {
        let dialog = gtk::AlertDialog::builder()
            .message(BUILD_NAME)
            .detail(DETAIL)
            .modal(true)
            .build();
        dialog.show(Some(self));
    }
}
