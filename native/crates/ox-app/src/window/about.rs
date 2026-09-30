// SPDX-License-Identifier: AGPL-3.0-only
//! "About this build" in the More options menu and the About settings,
//! and the licence dialog of the About settings.
//!
//! Ports the `About this build` item of the More menu in
//! `desktop/ui/app.js`, which shows a message box describing the build,
//! and `showLicense`. The text says what the native preview is and what it
//! cannot do yet.

use gtk::glib;
use ox_core::update::REPOSITORY;

use super::dialog::show_message;
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

/// The licence dialog's heading (`showLicense` in `desktop/ui/app.js`).
const LICENSE_TITLE: &str = "OpenXplorer · License & source";

/// The AGPL, as the packages install it beside the program.
const AGPL: &str = include_str!("../../../../../LICENSE");

/// What the licence dialog says: the copyright, where the corresponding
/// source is, and the licence (`showLicense`).
fn license_text() -> String {
    let version = env!("CARGO_PKG_VERSION");
    format!(
        "Copyright (c) 2026 OpenXplorer contributors.\nAGPL-3.0-only. No warranty. You may \
         redistribute and modify under the included terms.\n\nComplete corresponding source and \
         build tools: {REPOSITORY}, tag v{version}. The licence texts of the components are \
         installed with the package.\n\n{AGPL}"
    )
}

impl BrowserWindow {
    /// Shows the copyright, the source and the licence in a dialog whose
    /// body scrolls ("Read license & source information").
    pub(super) fn show_license(&self) {
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move { show_message(&window, LICENSE_TITLE, &license_text()).await }
        ));
    }

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
