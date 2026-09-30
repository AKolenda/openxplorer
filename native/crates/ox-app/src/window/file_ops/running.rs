// SPDX-License-Identifier: AGPL-3.0-only
//! One file operation at a time: its start, its progress in the transfer
//! panel, Cancel, and what the user is told when it ends (OPS-019,
//! OPS-022, OPS-023, OPS-024).
//!
//! Ports `runOperation`, `updateTransfer` and the `cancel` request of
//! `desktop/ui/app.js`. The operation's blocking work runs on GIO's worker
//! threads; its progress reports cross to the main loop through a
//! channel, and the panel shows them until the user cancels. When the
//! operation ends, the panel hides, the folder is listed again with the
//! items the operation created selected (the Python app cleared the
//! selection), Undo remembers how to reverse it, and a toast or the
//! "Operation result" dialog says what happened.

use gtk::glib;
use gtk::subclass::prelude::*;
use ox_core::ops::{
    run_transfer, starting_label, summarize, OperationContext, OperationSummary, OpsError, TransferOutcome,
    TransferRequest, UndoRecord, RESULT_TITLE, STOPPED_TITLE,
};
use ox_core::transfer::{Cancellation, Progress, TransferMode};

use crate::window::dialog;
use crate::window::loading::LoadMode;
use crate::window::transfer_panel::TransferPanel;
use crate::window::BrowserWindow;

/// What a finished operation leaves behind.
#[derive(Debug)]
pub(super) struct FinishedOperation {
    /// The toast or report that says what happened.
    pub(super) summary: OperationSummary,
    /// How Undo reverses it, when it can.
    pub(super) undo: Option<UndoRecord>,
    /// Where its new or moved items are now, to select them.
    pub(super) created: Vec<String>,
}

impl FinishedOperation {
    /// What a finished `mode` run of the transfer engine leaves behind.
    pub(super) fn of_transfer(mode: TransferMode, outcome: TransferOutcome) -> Self {
        Self {
            summary: summarize(mode, &outcome.result),
            undo: outcome.undo,
            created: outcome.created,
        }
    }
}

impl BrowserWindow {
    /// The panel of the running operation.
    pub(super) fn transfer_panel(&self) -> &TransferPanel {
        &self.imp().transfer_panel
    }

    /// Whether this window writes files now: a file operation runs or is
    /// being planned, or an extraction, compression or restored copy
    /// runs. Data safety (OPS-024): no other write starts meanwhile, and
    /// Sign out, Disconnect, moving a tab and an update's restart wait.
    pub(in crate::window) fn is_writing_files(&self) -> bool {
        let is_operating = !self.imp().file_operations.borrow().is_idle();
        is_operating || self.operation_panel().is_busy()
    }

    /// Starts an operation whose panel reads `label` until the first
    /// progress report. Returns its context, or `None` while another
    /// operation runs (OPS-024: `if(state.operation)return` in app.js),
    /// an archive operation included, or once an application update
    /// waits for its restart, which the message line says (UPD-006).
    pub(in crate::window) fn begin_operation(&self, label: &str) -> Option<OperationContext> {
        if self.operation_panel().is_busy() {
            return None;
        }
        if let Some(refusal) = self.context().updates().file_refusal() {
            self.show_message(&refusal);
            return None;
        }
        let context = OperationContext::new(self.context().write_protection());
        {
            let mut operations = self.imp().file_operations.borrow_mut();
            if operations.is_running() {
                return None;
            }
            operations.running = Some(context.cancel.clone());
        }
        self.transfer_panel().start(label);
        self.update_file_commands();
        Some(context)
    }

    /// Forgets the running operation and hides its panel.
    pub(in crate::window) fn end_operation(&self) {
        self.imp().file_operations.borrow_mut().running = None;
        self.transfer_panel().finish();
        self.update_file_commands();
    }

    /// A progress sink for the worker thread of the operation that
    /// `cancel` stops. Its reports reach the panel on the main loop until
    /// the user cancels; after that the panel keeps saying "Cancelling…".
    pub(super) fn progress_reporter(&self, cancel: &Cancellation) -> impl FnMut(Progress) + Send + 'static {
        let (reports, report_queue) = async_channel::unbounded::<Progress>();
        let panel = self.transfer_panel().clone();
        let cancel = cancel.clone();
        glib::spawn_future_local(glib::clone!(
            #[weak]
            panel,
            async move {
                // The loop ends when the worker drops its sender.
                while let Ok(progress) = report_queue.recv().await {
                    if !cancel.is_cancelled() {
                        panel.show_progress(&progress.label, progress.fraction);
                    }
                }
            }
        ));
        move |progress| {
            // Fails only once the window has gone and stopped listening.
            let _ = reports.try_send(progress);
        }
    }

    /// Cancel on the transfer panel: stops the running operation between
    /// steps; what is finished stays finished (OPS-022).
    pub(super) fn cancel_operation(&self) {
        let running = self.imp().file_operations.borrow().running.clone();
        let Some(cancel) = running else {
            return;
        };
        cancel.cancel();
        self.transfer_panel().show_cancelling();
    }

    /// Runs `request` on the transfer engine as the window's one
    /// operation (`runOperation`); `None` when another one runs. The panel
    /// has hidden when this returns; conclude with
    /// [`Self::conclude_operation`].
    pub(super) async fn run_request(
        &self,
        request: &TransferRequest,
    ) -> Option<Result<TransferOutcome, OpsError>> {
        let context = self.begin_operation(starting_label(request.mode))?;
        let progress = self.progress_reporter(&context.cancel);
        let outcome = run_transfer(request, &context, progress).await;
        self.end_operation();
        Some(outcome)
    }

    /// Runs `request` and concludes it: [`Self::run_request`], then
    /// [`Self::conclude_operation`].
    pub(super) async fn run_and_conclude(&self, request: &TransferRequest) {
        let Some(outcome) = self.run_request(request).await else {
            return;
        };
        let finished = outcome.map(|outcome| FinishedOperation::of_transfer(request.mode, outcome));
        self.conclude_operation(finished).await;
    }

    /// Concludes an ended operation: Undo remembers it, the folder is
    /// listed again with its items selected, and the user is told what
    /// happened ("Operation stopped" for a request refused before it
    /// started).
    pub(super) async fn conclude_operation(&self, outcome: Result<FinishedOperation, OpsError>) {
        match outcome {
            Ok(finished) => {
                if let Some(record) = finished.undo {
                    self.context().record_operation(record);
                }
                self.reload_selecting(finished.created);
                self.report(finished.summary).await;
            }
            Err(error) => {
                self.reload_selecting(Vec::new());
                dialog::show_message(self, STOPPED_TITLE, &error.to_string()).await;
            }
        }
    }

    /// Shows `summary`: a toast for complete success, otherwise the
    /// "Operation result" dialog.
    /// A desktop notification says it too while no window has focus
    /// (INT-026).
    pub(super) async fn report(&self, summary: OperationSummary) {
        self.notify_if_in_background(&summary);
        match summary {
            OperationSummary::Toast(text) => self.show_message(&text),
            OperationSummary::Report(text) => dialog::show_message(self, RESULT_TITLE, &text).await,
        }
    }

    /// Lists the active folder again, then selects `uris` in it (the
    /// items an operation created or moved there; none clears the
    /// selection, as app.js does after every operation).
    pub(super) fn reload_selecting(&self, uris: Vec<String>) {
        let Some(id) = self.imp().session.borrow().active_id() else {
            return;
        };
        if let Some(tab) = self.imp().session.borrow_mut().tab_mut(id) {
            tab.selected = uris;
        }
        self.load_tab(id, LoadMode::Reload);
    }
}
