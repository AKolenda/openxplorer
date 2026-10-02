// SPDX-License-Identifier: AGPL-3.0-only
//! Asking whether to run a program or script that is opened (OPEN-008).
//!
//! Dolphin asks "Open" or "Execute" when an executable file is opened.
//! Here that question is a choice in Settings ("Ask whether to run
//! programs and scripts"), off by default, so `OpenXplorer` keeps its rule
//! that opening never runs anything (OPEN-007): without the setting a
//! program opens in its viewer or editor, as before. There is no setting
//! that runs programs without asking. Running goes through the same
//! checks as dropping files on a program ([`super::file_drop`]): the
//! execute permission is checked again, a program on a share or a
//! removable drive asks again, and a script runs in the terminal.

use ox_core::entry::Entry;

use super::file_drop::{query_program, ProgramTarget};
use super::BrowserWindow;
use super::ButtonStyle;
use crate::dialog::Dialog;

/// What to do with an opened item that may be a program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RunChoice {
    /// Open it in its application, as any file.
    Open,
    /// Run it.
    Run(ProgramTarget),
    /// Do nothing.
    Cancel,
}

impl BrowserWindow {
    /// Whether to run `entry`, open it or do nothing: it opens unless the
    /// settings ask and it is a program the user may run, and then the
    /// user decides.
    pub(super) async fn run_or_open(&self, entry: &Entry) -> RunChoice {
        if !self.context().settings_data().preferences.ask_to_run_programs {
            return RunChoice::Open;
        }
        let Some(program) = query_program(entry).await else {
            return RunChoice::Open;
        };
        let message = ox_core::i18n::format_message(
            "“{name}” is a program or script. Run it, or open it in its application?",
            &[("name", &(program.name).to_string())],
        );
        let dialog = Dialog::new(self, &ox_core::i18n::gettext("Run this program?"), &message);
        dialog.add_cancel_button();
        // Open is first and primary, so Enter never runs it by accident.
        let open = dialog.add_button(&ox_core::i18n::gettext("Open"), ButtonStyle::Accent);
        let run = dialog.add_button(&ox_core::i18n::gettext("Run"), ButtonStyle::Bordered);
        dialog.open();
        let answer = dialog.next_response().await;
        dialog.finish();
        match answer {
            Some(choice) if choice == open => RunChoice::Open,
            Some(choice) if choice == run => RunChoice::Run(program),
            _ => RunChoice::Cancel,
        }
    }
}
