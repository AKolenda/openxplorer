// SPDX-License-Identifier: AGPL-3.0-only
//! "About this build" and "License & source" in the More options menu and
//! the About settings.
//!
//! Ports the `About this build` item of the More menu and `showLicense`
//! in `v2.0.0:desktop/ui/app.js`: message boxes with one OK button. About names
//! the native stack and what the app does not do (UPD-015); License &
//! source keeps the AGPL's "Appropriate Legal Notices" reachable: the
//! copyright, where the corresponding source is, the full AGPL-3.0 text
//! and the original MIT notice (UPD-016). Both texts are compiled in from
//! the repository's licence files, which every package installs too.
//! About also offers "Report an issue" and the website, which open in the
//! browser only when pressed (CMD-033, SAFE-002).

use gtk::glib;
use ox_core::update::REPOSITORY;

use super::help::REPORT_ISSUE;
use super::BrowserWindow;
use super::ButtonStyle;
use crate::config::{BUILD_NAME, IS_PREVIEW};
use crate::dialog::{Dialog, DialogButton};

/// What About this build says above the channel: the description and the
/// platform line, as the Python box had them.
const ABOUT_INTRODUCTION: &str = "An independent Windows 11–inspired file manager for Zorin.\n\n\
Desktop: Rust + GTK 4 + GIO/GVfs.";

/// What About this build says after the channel: the limitations.
const ABOUT_LIMITS: &str = "Replacing existing files requires confirmation; locations without a \
Recycle Bin offer a confirmed permanent delete. Cached filename/path search is opt-in. Thumbnails \
are not implemented. ZIP files can be browsed read-only and extracted. Open folders update \
through GIO file monitors.";

/// The text of About this build, whose channel sentence says which build
/// this is.
fn about_text() -> String {
    let channel = if IS_PREVIEW {
        "Native preview build: it runs beside the stable release."
    } else {
        "Stable release."
    };
    format!("{ABOUT_INTRODUCTION}\n\n{channel} {ABOUT_LIMITS}")
}

/// The project's website.
const WEBSITE: &str = "https://openxplorer.app";

/// The heading of License & source.
const LICENSE_TITLE: &str = "OpenXplorer · License & source";

/// The facts above the licence text (`showLicense`), with where this
/// build's corresponding source is: its release tag in the repository,
/// and the source archive published with each release.
fn license_text() -> String {
    let version = env!("CARGO_PKG_VERSION");
    format!(
        "Copyright (c) 2026 OpenXplorer contributors.\nAGPL-3.0-only. No warranty. You may \
         redistribute and modify under the included terms.\n\nComplete corresponding source and \
         build tools: {REPOSITORY}, tag v{version}, and the source archive published with each \
         release at {REPOSITORY}/releases."
    )
}

/// The GNU Affero General Public License, version 3, word for word.
const AGPL: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../LICENSE"));

/// The notice of the MIT-licensed Winspace code `OpenXplorer` started from.
const WINSPACE_NOTICE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../licenses/Winspace-MIT.txt"
));

/// How tall the scrolled licence text is, in pixels.
const LICENSE_TEXT_HEIGHT: i32 = 320;

impl BrowserWindow {
    /// Shows About this build over the window; its link buttons open
    /// the issue tracker or the website.
    pub(super) fn show_about(&self) {
        let (dialog, [report, website]) = self.about_dialog();
        dialog.open();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let answer = dialog.next_response().await;
                dialog.finish();
                if answer == Some(report) {
                    window.open_issue_tracker();
                } else if answer == Some(website) {
                    window.open_web_page(WEBSITE);
                }
            }
        ));
    }

    /// Shows License & source over the window.
    pub(super) fn show_license(&self) {
        show_until_dismissed(self.license_dialog());
    }

    /// About this build, and its "Report an issue" and "Website"
    /// buttons.
    fn about_dialog(&self) -> (Dialog, [DialogButton; 2]) {
        let dialog = Dialog::new(self, BUILD_NAME, &about_text());
        let report = dialog.add_button(REPORT_ISSUE, ButtonStyle::Bordered);
        let website = dialog.add_button("Website", ButtonStyle::Bordered);
        dialog.add_button("OK", ButtonStyle::Accent);
        (dialog, [report, website])
    }

    fn license_dialog(&self) -> Dialog {
        let dialog = Dialog::new(self, LICENSE_TITLE, &license_text());
        let notices = format!("{AGPL}\n\nOriginal notice:\n\n{WINSPACE_NOTICE}");
        dialog.add_scrolled_text(&notices, LICENSE_TEXT_HEIGHT);
        dialog.add_button("OK", ButtonStyle::Accent);
        dialog
    }
}

/// Opens `dialog` and closes it once any answer arrives.
fn show_until_dismissed(dialog: Dialog) {
    dialog.open();
    glib::spawn_future_local(async move {
        dialog.next_response().await;
        dialog.finish();
    });
}

#[cfg(test)]
mod tests {
    use gtk::prelude::*;

    use super::*;
    use crate::test_support::harness::{Fixture, TestWindow};

    /// About names the version, the native stack and what is not
    /// implemented, without the Python box's stale "no permanent-delete
    /// fallback".
    ///
    /// parity: UPD-015
    #[gtk::test]
    fn about_names_the_native_stack_and_its_limits() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let (dialog, _) = test.window.about_dialog();
        assert_eq!(dialog.title_text(), BUILD_NAME);
        let text = dialog.message_text();
        let channel = if IS_PREVIEW {
            "Native preview build"
        } else {
            "Stable release."
        };
        assert!(text.contains(channel), "{text}");
        assert!(text.contains("Desktop: Rust + GTK 4 + GIO/GVfs."), "{text}");
        assert!(text.contains("Thumbnails are not implemented."), "{text}");
        assert!(!text.contains("no permanent-delete fallback"), "{text}");
        assert_eq!(dialog.button_labels(), [REPORT_ISSUE, "Website", "OK"]);
        dialog.finish();
    }

    /// License & source shows the copyright, the source, the whole AGPL and
    /// the original MIT notice, and More options and Settings can open it.
    ///
    /// parity: UPD-016
    #[gtk::test]
    fn the_license_dialog_holds_every_notice() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let dialog = test.window.license_dialog();
        assert_eq!(dialog.title_text(), LICENSE_TITLE);
        assert!(dialog.message_text().contains("AGPL-3.0-only. No warranty."));
        assert!(dialog.message_text().contains(&format!("{REPOSITORY}/releases")));
        let notices = dialog.scrolled_text();
        for part in [
            "GNU AFFERO GENERAL PUBLIC LICENSE",
            "13. Remote Network Interaction",
            "END OF TERMS AND CONDITIONS",
            "Copyright (c) 2026 Winspace contributors",
        ] {
            assert!(notices.contains(part), "{part}");
        }
        dialog.finish();
        let license = crate::window::WindowAction::License.name();
        assert!(
            test.window.is_action_enabled(license),
            "License & source can be opened"
        );
    }
}
