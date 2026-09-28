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
searches folders and the search index, shows Properties, previous \
versions and folder sizes, opens and extracts ZIP files, and has the \
Settings page, desktop integration (default apps, Show in folder, Open \
with, Open in Terminal, Brave's download folder) and Check for updates. \
What it still lacks, the installed OpenXplorer keeps doing.";

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
