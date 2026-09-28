// SPDX-License-Identifier: AGPL-3.0-only
//! Restore a copy of a previous version (PROP-025).
//!
//! Ports `restoreVersion` and the `runOperation('copy', …, 'keep-both')`
//! it ends with in `desktop/ui/app.js`: the Restore dialog checks the
//! destination, then the transfer engine copies the version there with
//! Keep both, with progress and Cancel in the operation panel, so neither
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

use super::BrowserWindow;

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
        let request = TransferRequest {
            mode: TransferMode::Copy,
            uris: vec![source],
            destination_folder: Some(destination.clone()),
            policy: ConflictPolicy::KeepBoth,
        };
        let operation = OperationContext::new(self.context().write_protection());
        let panel = self.operation_panel();
        panel.start(starting_label(TransferMode::Copy), operation.cancel.clone());
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
                    Ok(outcome) => window.report_transfer(&summarize(TransferMode::Copy, &outcome.result)),
                    Err(error) => window.show_result_dialog(STOPPED_TITLE, &error.to_string()),
                }
            }
        ));
    }

    /// Shows how a copy ended: a toast for complete success, else the
    /// Operation result dialog.
    fn report_transfer(&self, summary: &OperationSummary) {
        match summary {
            OperationSummary::Toast(text) => self.show_message(text),
            OperationSummary::Report(text) => self.show_result_dialog(RESULT_TITLE, text),
        }
    }
}
