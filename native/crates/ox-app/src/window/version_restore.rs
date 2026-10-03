// SPDX-License-Identifier: AGPL-3.0-only
//! Restore a copy of a previous version (PROP-025).
//!
//! Ports `restoreVersion` and the `runOperation('copy', …, 'keep-both')`
//! it ends with in `v2.0.0:desktop/ui/app.js`: the Restore dialog checks the
//! destination, then the transfer engine copies the version there with
//! Keep both, with progress and Cancel in the transfer panel, so neither
//! the live original nor the snapshot is replaced. The end is reported
//! as every copy's is: a toast, or the Operation result dialog.

use gtk::glib;
use gtk::subclass::prelude::*;
use ox_core::ops::{
    run_transfer, starting_label, summarize, OperationContext, OperationSummary, TransferRequest,
    RESULT_TITLE, STOPPED_TITLE,
};
use ox_core::transfer::{ConflictPolicy, TransferMode};

use crate::properties::RestoreRequest;

use super::background_notice::Destination;
use super::transfer_panel::TransferKind;
use super::BrowserWindow;

/// Shown when Restore a copy is asked for while a write runs, as the
/// Python bridge refused a second `operate`.
const OPERATION_RUNNING: &str = crate::i18n::message_id("Another file operation is still running.");

impl BrowserWindow {
    /// Asks where to restore a copy of a previous version, then copies it
    /// there.
    pub(super) fn ask_restore_destination(&self, request: &RestoreRequest) {
        let home = self.imp().locations.borrow().home_uri();
        let versions = self.context().previous_versions().clone();
        let source = request.version_uri.clone();
        let frame = request.dialog(
            versions,
            &home,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |destination| window.restore_copy(source.clone(), destination)
            ),
        );
        self.present_window_dialog(&frame);
    }

    /// Copies the version at `source` into `destination` with Keep both,
    /// then says how it went.
    fn restore_copy(&self, source: String, destination: String) {
        // One operation at a time (OPS-024).
        if self.is_writing_files() {
            self.show_message(ox_core::i18n::gettext_static(OPERATION_RUNNING));
            return;
        }
        if self.refuses_writes_during_update() {
            return;
        }
        let request = TransferRequest {
            mode: TransferMode::Copy,
            uris: vec![source],
            destination_folder: Some(destination.clone()),
            policy: ConflictPolicy::KeepBoth,
        };
        let operation = OperationContext::new(self.context().write_protection());
        let panel = self.transfer_panel();
        panel.start(
            TransferKind::Files,
            starting_label(TransferMode::Copy),
            operation.cancel.clone(),
        );
        let progress = self.operation_progress_sender();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let outcome = run_transfer(&request, &operation, progress).await;
                window.finish_archive_operation();
                // Listing a folder hides the toast, so the report comes last.
                window.reload_tabs_showing(&destination);
                match outcome {
                    Ok(outcome) => window.report_transfer(
                        &summarize(TransferMode::Copy, &outcome.result),
                        Destination::items(outcome.created),
                    ),
                    Err(error) => window
                        .show_result_dialog(ox_core::i18n::gettext_static(STOPPED_TITLE), &error.to_string()),
                }
            }
        ));
    }

    /// Shows how a copy ended: a toast for complete success, else the
    /// Operation result dialog.
    fn report_transfer(&self, summary: &OperationSummary, destination: Destination) {
        self.notify_if_in_background(summary, destination);
        match summary {
            OperationSummary::Toast(text) => self.show_message(text),
            OperationSummary::Report(text) => {
                self.show_result_dialog(ox_core::i18n::gettext_static(RESULT_TITLE), text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::test_support::harness::{wait_for, Fixture, TestWindow};

    /// parity: OPS-024
    #[gtk::test]
    fn no_version_is_restored_while_a_file_operation_runs() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let running = test.window.begin_operation("Preparing copy…");

        test.window
            .restore_copy(fixture.uri_of("Notes 2.txt"), fixture.uri_of("Documents"));
        wait_for(Duration::from_millis(200));

        assert!(running.is_some());
        assert_eq!(test.window.shown_message_text(), OPERATION_RUNNING);
        assert!(!fixture.path("Documents/Notes 2.txt").exists());
        test.window.end_operation();
    }
}
