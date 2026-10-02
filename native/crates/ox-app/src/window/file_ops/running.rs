// SPDX-License-Identifier: AGPL-3.0-only
//! One file operation at a time: its start, its progress in the transfer
//! panel, Cancel, and what the user is told when it ends (OPS-019,
//! OPS-022, OPS-023, OPS-024).
//!
//! Ports `runOperation`, `updateTransfer` and the `cancel` request of
//! `v2.0.0:desktop/ui/app.js`. The operation's blocking work runs on GIO's worker
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

use super::unfinished::mark_unfinished;
use crate::dialog;
use crate::search::changed_folders;
use crate::window::background_notice::Destination;
use crate::window::loading::LoadMode;
use crate::window::transfer_panel::{TransferKind, TransferPanel};
use crate::window::window_action::WindowAction;
use crate::window::BrowserWindow;

/// The label of the toast's button that reverses the operation it reports.
const UNDO_BUTTON: &str = "Undo";

/// What a finished operation leaves behind.
#[derive(Debug)]
pub(super) struct FinishedOperation {
    /// The toast or report that says what happened.
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
    /// The panel of the running operation.
    pub(in crate::window) fn transfer_panel(&self) -> &TransferPanel {
        &self.imp().transfer_panel
    }

    /// Whether this window writes files now: a file operation runs or is
    /// being planned, or an extraction, compression or restored copy
    /// runs. Data safety (OPS-024): no other write starts meanwhile, and
    /// Sign out, Disconnect, moving a tab and an update's restart wait.
    pub(crate) fn is_writing_files(&self) -> bool {
        let is_operating = !self.imp().file_operations.borrow().is_idle();
        is_operating || self.transfer_panel().is_busy()
    }

    /// True, after saying why, once an application update installed files
    /// and waits for its restart (UPD-006). Every writer asks before it
    /// starts: file operations, extraction, compression, a restored
    /// version, rename and new items, as the Python app refused every
    /// file request in that state.
    pub(in crate::window) fn refuses_writes_during_update(&self) -> bool {
        let Some(refusal) = self.context().updates().file_refusal() else {
            return false;
        };
        self.show_message(&refusal);
        true
    }

    /// Starts an operation whose panel reads `label` until the first
    /// progress report. Returns its context, or `None` while another
    /// operation runs (OPS-024: `if(state.operation)return` in app.js),
    /// an archive operation included, or once an application update
    /// waits for its restart, which the message line says (UPD-006).
    pub(crate) fn begin_operation(&self, label: &str) -> Option<OperationContext> {
        if self.is_writing_files() {
            return None;
        }
        if self.refuses_writes_during_update() {
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
    pub(super) fn progress_reporter(&self, cancel: &Cancellation) -> impl FnMut(Progress) + Send + 'static {
        let (reports, report_queue) = async_channel::unbounded::<Progress>();
        let panel = self.panel_for_operation(cancel);
        let cancel = cancel.clone();
        glib::spawn_future_local(glib::clone!(
            #[weak]
            panel,
            async move {
                // The loop ends when the worker drops its sender.
                while let Ok(progress) = report_queue.recv().await {
                    if !cancel.is_cancelled() {
                        panel.show_progress(&progress);
                    }
                }
            }
        ));
        move |progress| {
            // Fails only once the window has gone and stopped listening.
            let _ = reports.try_send(progress);
        }
    }

    /// Cancel operation: stops the running file operation between steps,
    /// as the transfer panel's Cancel does; what is finished stays
    /// finished (OPS-022).
    pub(in crate::window) fn cancel_operation(&self) {
        self.transfer_panel().cancel();
        for job in &self.imp().file_operations.borrow().jobs {
            job.panel.cancel();
        }
    }

    /// Runs `request` on the transfer engine as the window's one
    /// operation (`runOperation`); `None` when another one runs. The panel
    /// has hidden when this returns; conclude with
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
        let Some(outcome) = self.run_request(request).await else {
            return;
        };
        let finished = outcome.map(|outcome| FinishedOperation {
            select_after: next.map(str::to_owned).into_iter().collect(),
            ..FinishedOperation::of_transfer(request.mode, outcome)
        });
        self.conclude_operation(finished).await;
    }

    /// Concludes an ended operation: Undo remembers it, the folder is
    /// listed again with its items selected, and the user is told what
    /// happened ("Operation stopped" for a request refused before it
    /// started). The toast of an operation Undo can reverse has an Undo
    /// button (OPS-032).
    pub(super) async fn conclude_operation(&self, outcome: Result<FinishedOperation, OpsError>) {
        match outcome {
            Ok(finished) => {
                let is_undoable = finished.undo.is_some();
                if let Some(record) = finished.undo {
                    self.context().record_operation(record);
                }
                let destination = Destination::items(finished.select_after.clone());
                self.reload_selecting(finished.select_after);
                match finished.summary {
                    OperationSummary::Toast(text) if is_undoable => {
                        // The toast has Undo; the desktop hears it too while
                        // no window has focus (INT-026).
                        self.notify_if_in_background(&OperationSummary::Toast(text.clone()), destination);
                        self.show_message_with_undo(&text);
                    }
                    summary => self.report(summary, destination).await,
                }
            }
            Err(error) => {
                self.reload_selecting(Vec::new());
                dialog::show_message(self, STOPPED_TITLE, &error.to_string()).await;
            }
        }
    }

    /// Shows `text` in the toast with an Undo button, which reverses the
    /// newest operation while it still is the one the toast is about: a
    /// later change to the undo journal takes the button away (OPS-032).
    pub(in crate::window) fn show_message_with_undo(&self, text: &str) {
        self.imp()
            .toast
            .show_with_action(text, UNDO_BUTTON, WindowAction::Undo);
    }

    /// Takes the toast's Undo button away once the operation it would
    /// reverse is no longer the newest.
    pub(super) fn withdraw_toast_undo(&self) {
        self.imp().toast.withdraw_action();
    }

    /// Shows `summary`: a toast for complete success, otherwise the
    /// "Operation result" dialog.
    /// A desktop notification says it too while no window has focus
    /// (INT-026), whose Show button opens `destination`.
    pub(super) async fn report(&self, summary: OperationSummary, destination: Destination) {
        self.notify_if_in_background(&summary, destination);
        match summary {
            OperationSummary::Toast(text) => self.show_message(&text),
            OperationSummary::Report(text) => dialog::show_message(self, RESULT_TITLE, &text).await,
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
