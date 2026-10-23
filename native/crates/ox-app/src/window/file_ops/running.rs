// SPDX-License-Identifier: AGPL-3.0-only
//! Starting exclusive operations and reporting each transfer job
//! through its panel, cancellation and completion message (OPS-019,
//! OPS-022, OPS-023, OPS-024).
//!
//! Ports `runOperation`, `updateTransfer` and the `cancel` request of
//! `v2.0.0:desktop/ui/app.js`. The operation's blocking work runs on GIO's worker
//! threads; its progress reports cross to the main loop through a
//! channel, and the panel shows them until the user cancels. When the
//! operation ends, the panel hides, the folder is listed again with the
//! items the operation created selected (the Python app cleared the
//! selection) and Undo remembers how to reverse it. As in Windows
//! Explorer, nothing more is said when every item succeeded; otherwise
//! the "Operation result" dialog says what happened.

use gtk::subclass::prelude::*;
use ox_core::ops::{
    run_transfer, starting_label, summarize, OperationContext, OperationSummary, OpsError, TransferOutcome,
    TransferRequest, UndoRecord, RESULT_TITLE, STOPPED_TITLE,
};
use ox_core::transfer::{Cancellation, Progress, TransferMode};

use crate::app_context::WriteHold;

use super::unfinished::mark_unfinished;
use crate::dialog;
use crate::search::changed_folders;
use crate::window::loading::LoadMode;
use crate::window::transfer_panel::{TransferKind, TransferPanel};
use crate::window::BrowserWindow;

/// Why files are not written while a drive is formatted.
const FORMATTING_A_DRIVE: &str =
    crate::i18n::message_id("Wait until the drive is formatted, then try again.");

/// What a finished operation leaves behind.
#[derive(Debug)]
pub(super) struct FinishedOperation {
    /// What happened, for the report.
    pub(super) summary: OperationSummary,
    /// How Undo reverses it, when it can.
    pub(super) undo: Option<UndoRecord>,
    /// The items to select once the folder is listed again: where its new
    /// or moved items are now, or after a deletion the item that followed
    /// the removed ones.
    pub(super) select_after: Vec<String>,
}

impl FinishedOperation {
    /// What a finished `mode` run of the transfer engine leaves behind.
    pub(super) fn of_transfer(mode: TransferMode, outcome: TransferOutcome) -> Self {
        Self {
            summary: summarize(mode, &outcome.result),
            undo: outcome.undo,
            select_after: outcome.created,
        }
    }
}

impl BrowserWindow {
    /// The primary panel, used by an exclusive operation or one transfer job.
    /// Concurrent jobs use additional panels managed by `jobs`.
    pub(in crate::window) fn transfer_panel(&self) -> &TransferPanel {
        &self.imp().transfer_panel
    }

    /// Whether this window writes files now: a file operation runs or is
    /// being planned, or an extraction, compression or restored copy
    /// runs. Exclusive writes, Sign out, Disconnect, moving a tab and an
    /// update's restart wait (OPS-024). Additional transfers must pass
    /// `begin_transfer`'s job limit and overlapping-path checks.
    pub(crate) fn is_writing_files(&self) -> bool {
        let is_operating = !self.imp().file_operations.borrow().is_idle();
        is_operating || self.transfer_panel().is_busy()
    }

    /// True, after saying why, once an application update installed files
    /// and waits for its restart (UPD-006), or while a drive is formatted
    /// (DEV-012). Every writer asks before it starts: file operations,
    /// extraction, compression, a restored version, rename and new items,
    /// as the Python app refused every file request during an update.
    pub(in crate::window) fn refuses_writes(&self) -> bool {
        let Some(refusal) = self.write_refusal() else {
            return false;
        };
        self.show_message(&refusal);
        true
    }

    /// Why no write may start now (see [`Self::refuses_writes`]), or
    /// `None` when one may.
    pub(in crate::window) fn write_refusal(&self) -> Option<String> {
        if self.context().formats_a_drive() {
            return Some(ox_core::i18n::gettext_static(FORMATTING_A_DRIVE).to_owned());
        }
        self.context().updates().file_refusal()
    }

    /// Admits a write outside the operation panel (a new folder, file or
    /// link, a rename or a permission change) right before it starts,
    /// after its questions: a hold that counts it as writing until
    /// dropped, so no drive is formatted meanwhile (DEV-012). The error
    /// says why it may not start.
    pub(crate) fn hold_write(&self) -> Result<WriteHold, String> {
        match self.write_refusal() {
            Some(refusal) => Err(refusal),
            None => Ok(self.context().hold_write()),
        }
    }

    /// [`Self::hold_write`], with the context the write runs with.
    pub(in crate::window) fn admit_write(&self) -> Result<(OperationContext, WriteHold), String> {
        let hold = self.hold_write()?;
        Ok((OperationContext::new(self.context().write_protection()), hold))
    }

    /// Starts an exclusive operation whose panel reads `label` until the first
    /// progress report. Returns its context, or `None` while another
    /// operation runs (OPS-024: `if(state.operation)return` in app.js),
    /// an archive operation included, or once an application update
    /// waits for its restart, which the message line says (UPD-006).
    pub(crate) fn begin_operation(&self, label: &str) -> Option<OperationContext> {
        if self.is_writing_files() {
            return None;
        }
        if self.refuses_writes() {
            return None;
        }
        let mut context = OperationContext::new(self.context().write_protection());
        // XFER-028: names and links the destination cannot store are asked
        // about in a dialog.
        context.unstorable = Some(self.unstorable_asker());
        // XFER-011 and XFER-013: a move the location cannot do natively is
        // finished by copying only when the user agrees.
        context.move_by_copying = Some(self.move_by_copying_asker());
        // OPS-047: an item that fails asks whether to retry, skip or
        // cancel.
        context.item_failure = Some(self.failure_asker());
        {
            let mut operations = self.imp().file_operations.borrow_mut();
            if operations.is_running() {
                return None;
            }
            operations.running = Some(context.cancel.clone());
        }
        self.transfer_panel()
            .start(TransferKind::Files, label, context.cancel.clone());
        self.update_file_commands();
        Some(context)
    }

    /// Forgets the running operation and hides its panel.
    pub(crate) fn end_operation(&self) {
        self.imp().file_operations.borrow_mut().running = None;
        self.transfer_panel().finish();
        self.update_file_commands();
    }

    /// A progress sink for the worker thread of the operation that
    /// `cancel` stops. Its reports reach the panel on the main loop until
    /// the user cancels; after that the panel keeps saying "Cancelling…".
    pub(in crate::window) fn progress_reporter(
        &self,
        cancel: &Cancellation,
    ) -> impl FnMut(Progress) + Send + 'static {
        super::progress::report_to(&self.panel_for_operation(cancel), cancel.clone())
    }

    /// Stops every running operation between steps; each panel's Cancel
    /// button stops only its own job. Finished work remains (OPS-022).
    pub(in crate::window) fn cancel_operation(&self) {
        self.transfer_panel().cancel();
        for job in &self.imp().file_operations.borrow().jobs {
            job.panel.cancel();
        }
    }

    /// Runs `request` as a transfer job with its own panel and cancellation
    /// token. Returns `None` when `begin_transfer` refuses the job because
    /// of an exclusive write, an update, the job limit or overlapping paths.
    /// Its panel is hidden before returning; conclude with
    /// [`Self::conclude_operation`].
    pub(super) async fn run_request(
        &self,
        request: &TransferRequest,
    ) -> Option<Result<TransferOutcome, OpsError>> {
        let context = self.begin_transfer(
            starting_label(request.mode),
            &request.uris,
            request.destination_folder.as_deref(),
        )?;
        let progress = self.progress_reporter(&context.cancel);
        let mark = mark_unfinished(request.destination_folder.as_deref());
        let outcome = run_transfer(request, &context, progress).await;
        drop(mark);
        self.end_transfer(&context.cancel);
        let destination = request.destination_folder.as_deref();
        let changed = changed_folders(destination, request.uris.iter().map(String::as_str));
        self.context().search_cache().folders_written(changed);
        Some(outcome)
    }

    /// Runs `request`, a move to the Trash or a delete, and concludes it:
    /// [`Self::run_request`], then [`Self::conclude_operation`], which
    /// selects `next`, the item that followed the removed ones (SEL-017).
    pub(super) async fn run_deletion(&self, request: &TransferRequest, next: Option<&str>) {
        let origin = self.current_uri();
        let Some(outcome) = self.run_request(request).await else {
            return;
        };
        let finished = outcome.map(|outcome| FinishedOperation {
            select_after: next.map(str::to_owned).into_iter().collect(),
            ..FinishedOperation::of_transfer(request.mode, outcome)
        });
        self.conclude_operation_in(finished, origin.as_deref()).await;
    }

    /// Concludes an ended operation: Undo remembers it, the folder is
    /// listed again with its items selected, and the user is told what
    /// went wrong, if anything ("Operation stopped" for a request refused
    /// before it started).
    pub(super) async fn conclude_operation(&self, outcome: Result<FinishedOperation, OpsError>) {
        let origin = self.current_uri();
        self.conclude_operation_in(outcome, origin.as_deref()).await;
    }

    /// Completion must not replace a selection made while browsing another folder.
    pub(super) async fn conclude_operation_in(
        &self,
        outcome: Result<FinishedOperation, OpsError>,
        origin: Option<&str>,
    ) {
        let still_here = self.current_uri().as_deref() == origin;
        match outcome {
            Ok(finished) => {
                if let Some(record) = finished.undo {
                    self.context().record_operation(record);
                }
                if still_here {
                    self.reload_selecting(finished.select_after);
                }
                self.report(finished.summary).await;
            }
            Err(error) => {
                if still_here {
                    self.reload_selecting(Vec::new());
                }
                // Cancelled before any item ran, as during the free-space
                // check: nothing to say.
                if error.is_cancelled() {
                    return;
                }
                dialog::show_message(
                    self,
                    ox_core::i18n::gettext_static(STOPPED_TITLE),
                    &error.to_string(),
                )
                .await;
            }
        }
    }

    /// Shows the "Operation result" dialog unless every item succeeded,
    /// which Windows Explorer does not mention, or the user cancelled.
    pub(super) async fn report(&self, summary: OperationSummary) {
        if let OperationSummary::Report(text) = summary {
            dialog::show_message(self, ox_core::i18n::gettext_static(RESULT_TITLE), &text).await;
        }
    }

    /// Lists the active folder again, then selects `uris` in it and
    /// scrolls to the first (the items an operation created or moved
    /// there, SEL-016; none clears the selection, as app.js does after
    /// every operation). The search cache reads the folder and the items'
    /// folders again (SRCH-033).
    pub(super) fn reload_selecting(&self, uris: Vec<String>) {
        let Some(id) = self.imp().session.borrow().active_id() else {
            return;
        };
        let folder = self.current_uri();
        let changed = changed_folders(folder.as_deref(), uris.iter().map(String::as_str));
        self.context().search_cache().folders_written(changed);
        if let Some(tab) = self.imp().session.borrow_mut().tab_mut(id) {
            tab.reveals_selection = !uris.is_empty();
            tab.selected = uris;
        }
        self.load_tab(id, LoadMode::Reload);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gtk::glib;
    use ox_core::ops::OpsError;

    use crate::test_support::harness::{wait_for, Fixture, TestWindow};
    use crate::window::tests::file_ops_support::{dialog_over, open_dialog};

    /// A request the user cancelled before any item ran, as during the
    /// free-space check of a large copy, ends without a word, as in
    /// Windows Explorer; a refused one still says why.
    ///
    /// parity: OPS-023
    #[gtk::test]
    fn a_request_cancelled_before_it_started_says_nothing() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let conclude = |error: OpsError| {
            let window = test.window.clone();
            glib::spawn_future_local(async move { window.conclude_operation(Err(error)).await });
        };

        conclude(OpsError::Cancelled);
        wait_for(Duration::from_millis(300));
        assert!(dialog_over(&test).is_none(), "nothing is said after Cancel");

        conclude(OpsError::Failed("Not enough free space.".to_owned()));
        let stopped = open_dialog(&test);
        assert_eq!(stopped.title_text(), "Operation stopped");
        stopped.press("OK");
    }
}
