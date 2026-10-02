// SPDX-License-Identifier: AGPL-3.0-only
//! Help: the offline user manual that F1 and More options > Help open,
//! at the topic of what the window shows, and "Report an issue"
//! (CMD-033).
//!
//! The manual, `resources/manual.md`, is compiled into the program, so
//! every package installs it and it opens without a network. Each `## `
//! heading is a topic of the dialog's Topic list. Nothing goes online
//! until the user presses "Report an issue", which opens the project's
//! issue tracker in the default browser (SAFE-002).

use gtk::glib;
use gtk::prelude::*;
use ox_core::location::is_smb_location;
use ox_core::update::REPOSITORY;

use super::BrowserWindow;
use super::ButtonStyle;
use crate::dialog::{Dialog, DialogButton};
use crate::locations::Page;

/// The manual, with an HTML comment before its first topic.
const MANUAL: &str = include_str!("../../resources/manual.md");

/// The label of the button that opens the issue tracker.
pub(super) const REPORT_ISSUE: &str = crate::i18n::message_id("Report an issue");

/// The topic the manual opens at when nothing more specific applies.
const FIRST_TOPIC: &str = crate::i18n::message_id("Getting started");

/// The project's issue tracker.
pub(super) fn issue_tracker() -> String {
    format!("{REPOSITORY}/issues")
}

/// The manual's topics in order: each heading and the text under it.
fn topics() -> Vec<(&'static str, String)> {
    MANUAL
        .split("\n## ")
        .skip(1)
        .map(|topic| {
            let (heading, body) = topic.split_once('\n').unwrap_or((topic, ""));
            (heading.trim(), body.trim().to_owned())
        })
        .collect()
}

impl BrowserWindow {
    /// The manual's topic for what the window shows now.
    fn help_topic(&self) -> &'static str {
        let uri = self.current_uri().unwrap_or_default();
        if self.shows_settings() {
            "Settings and privacy"
        } else if self.is_searching() {
            "Search"
        } else if self.command_facts().folder.is_recycle_bin {
            "Recycle Bin"
        } else if Page::from_uri(&uri) == Some(Page::Network) || is_smb_location(&uri) {
            "Network locations"
        } else if Page::from_uri(&uri) == Some(Page::ThisPc) {
            "Drives and phones"
        } else {
            ox_core::i18n::gettext_static(FIRST_TOPIC)
        }
    }

    /// Opens the manual at the topic of what the window shows.
    pub(super) fn show_help(&self) {
        let (dialog, report_button) = self.help_dialog(self.help_topic());
        dialog.open();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let report = dialog.next_response().await == Some(report_button);
                dialog.finish();
                if report {
                    window.open_issue_tracker();
                }
            }
        ));
    }

    /// The Help dialog, showing `topic`, and its "Report an issue"
    /// button.
    pub(super) fn help_dialog(&self, topic: &str) -> (Dialog, DialogButton) {
        let topics = topics();
        let dialog = Dialog::new(
            self,
            &ox_core::i18n::gettext("OpenXplorer Help"),
            &ox_core::i18n::gettext(""),
        );
        let headings: Vec<&str> = topics.iter().map(|(heading, _)| *heading).collect();
        let chooser = gtk::DropDown::from_strings(&headings);
        dialog.add_labelled("Topic", &chooser);
        let text = dialog.add_hint("");
        let show = move |chooser: &gtk::DropDown| {
            let index = usize::try_from(chooser.selected()).unwrap_or_default();
            if let Some((_, body)) = topics.get(index) {
                text.set_text(body);
            }
        };
        chooser.connect_selected_notify(show.clone());
        let position = headings
            .iter()
            .position(|heading| *heading == topic)
            .unwrap_or_default();
        chooser.set_selected(u32::try_from(position).unwrap_or_default());
        show(&chooser);
        let report_button =
            dialog.add_button(ox_core::i18n::gettext_static(REPORT_ISSUE), ButtonStyle::Bordered);
        dialog.add_button(&ox_core::i18n::gettext("Close"), ButtonStyle::Accent);
        (dialog, report_button)
    }

    /// Opens the project's issue tracker in the default browser.
    pub(super) fn open_issue_tracker(&self) {
        self.open_web_page(&issue_tracker());
    }

    /// Opens `url` in the default browser, saying so when it cannot.
    pub(super) fn open_web_page(&self, url: &str) {
        let window = self.downgrade();
        self.context().open_uri(url, self.upcast_ref(), move |error| {
            if let Some(window) = window.upgrade() {
                window.show_message(&error.to_string());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::{Fixture, TestWindow};

    /// F1 opens the compiled-in manual at the topic of what the window
    /// shows, and "Report an issue" opens the tracker only when pressed.
    ///
    /// parity: CMD-033
    #[gtk::test]
    fn f1_opens_the_offline_manual_and_report_an_issue_opens_the_tracker() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.context.record_launches();
        assert!(topics().len() >= 8, "the manual has its topics");

        let (dialog, _) = test.window.help_dialog(test.window.help_topic());

        assert_eq!(dialog.title_text(), "OpenXplorer Help");
        assert!(dialog.texts().iter().any(|text| text.contains("tabbed window")));
        assert_eq!(dialog.button_labels(), [REPORT_ISSUE, "Close"]);
        assert!(
            test.context.recorded_launches().is_empty(),
            "nothing opens by itself"
        );
        dialog.finish();
        test.window.open_issue_tracker();
        assert_eq!(test.context.recorded_launches(), [issue_tracker()]);
        let app = test.window.application().expect("the window has its application");
        assert_eq!(app.accels_for_action("win.help"), ["F1"]);
    }
}
