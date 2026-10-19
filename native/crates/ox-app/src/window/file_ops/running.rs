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
//! selection), Undo remembers how to reverse it, and a toast or the
//! "Operation result" dialog says what happened.

use gtk::subclass::prelude::*;
use ox_core::location::TRASH_URI;
use ox_core::ops::{
    run_transfer, starting_label, summarize, trashed_items_since, OperationContext, OperationSummary,
    OpsError, TransferOutcome, TransferRequest, UndoRecord, RESULT_TITLE, STOPPED_TITLE,
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

/// Why files are not written while a drive is formatted.
const FORMATTING_A_DRIVE: &str =
    crate::i18n::message_id("Wait until the drive is formatted, then try again.");

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
        let refusal = if self.context().formats_a_drive() {
            Some(ox_core::i18n::gettext_static(FORMATTING_A_DRIVE).to_owned())
        } else {
            self.context().updates().file_refusal()
        };
        let Some(refusal) = refusal else {
            return false;
        };
        self.show_message(&refusal);
        true
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
    /// The notification's Show opens what [`Self::removed_destination`]
    /// says, never `next`.
    pub(super) async fn run_deletion(&self, request: &TransferRequest, next: Option<&str>) {
        let origin = self.current_uri();
        let Some(outcome) = self.run_request(request).await else {
            return;
        };
        let finished = outcome.map(|outcome| FinishedOperation {
            select_after: next.map(str::to_owned).into_iter().collect(),
            ..FinishedOperation::of_transfer(request.mode, outcome)
        });
        // Undo knows the deletion before the notice's lookup is awaited:
        // Ctrl+Z meanwhile must take back this deletion, not the action
        // before it.
        let recorded = match &finished {
            Ok(FinishedOperation {
                undo: Some(record), ..
            }) => {
                self.context().record_operation(record.clone());
                true
            }
            _ => false,
        };
        let shown = match &finished {
            Ok(finished) => {
                self.removed_destination(request.mode, finished.undo.as_ref())
                    .await
            }
            Err(_) => Destination::default(),
        };
        self.conclude_recorded_operation_in(finished, origin.as_deref(), Some(shown), recorded)
            .await;
    }

    /// What the Show button of a deletion's notification opens: the
    /// Recycle Bin with the items `undo` says a Move to Trash put there
    /// selected, or the Recycle Bin alone when they cannot be found. A
    /// permanent delete leaves nothing to show, so Show only brings the
    /// window back, as for any operation with no destination (INT-026).
    /// The Recycle Bin is listed only while no window has focus, the only
    /// time a notification is sent.
    async fn removed_destination(&self, mode: TransferMode, undo: Option<&UndoRecord>) -> Destination {
        let trashed = match (mode, undo) {
            (
                TransferMode::Trash,
                Some(UndoRecord::Trash {
                    original_paths,
                    trashed_since,
                }),
            ) => (original_paths.clone(), *trashed_since),
            _ => return Destination::default(),
        };
        if !self.is_in_background() {
            return Destination::folder(TRASH_URI);
        }
        let (original_paths, since) = trashed;
        let items = trashed_items_since(original_paths, since, &Cancellation::new())
            .await
            .unwrap_or_default();
        Destination {
            folder: Some(TRASH_URI.to_owned()),
            items,
        }
    }

    /// Concludes an ended operation: Undo remembers it, the folder is
    /// listed again with its items selected, and the user is told what
    /// happened ("Operation stopped" for a request refused before it
    /// started). The toast of an operation Undo can reverse has an Undo
    /// button (OPS-032).
    pub(super) async fn conclude_operation(&self, outcome: Result<FinishedOperation, OpsError>) {
        let origin = self.current_uri();
        self.conclude_operation_in(outcome, origin.as_deref(), None).await;
    }

    /// Completion must not replace a selection made while browsing another
    /// folder. The notification's Show opens `shown`, or by default the
    /// items the operation selects afterwards.
    pub(super) async fn conclude_operation_in(
        &self,
        outcome: Result<FinishedOperation, OpsError>,
        origin: Option<&str>,
        shown: Option<Destination>,
    ) {
        self.conclude_recorded_operation_in(outcome, origin, shown, false)
            .await;
    }

    /// [`Self::conclude_operation_in`]; `recorded` when Undo already
    /// remembers the operation.
    async fn conclude_recorded_operation_in(
        &self,
        outcome: Result<FinishedOperation, OpsError>,
        origin: Option<&str>,
        shown: Option<Destination>,
        recorded: bool,
    ) {
        let still_here = self.current_uri().as_deref() == origin;
        match outcome {
            Ok(finished) => {
                let is_undoable = finished.undo.is_some();
                if let Some(record) = finished.undo.filter(|_| !recorded) {
                    self.context().record_operation(record);
                }
                let destination = shown.unwrap_or_else(|| Destination::items(finished.select_after.clone()));
                if still_here {
                    self.reload_selecting(finished.select_after);
                }
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
                if still_here {
                    self.reload_selecting(Vec::new());
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
            OperationSummary::Report(text) => {
                dialog::show_message(self, ox_core::i18n::gettext_static(RESULT_TITLE), &text).await;
            }
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
