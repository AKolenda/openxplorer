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
//! Explorer, nothing more is said: every item that failed was asked about
//! in the progress window while it ran (OPS-047).

use std::time::Duration;

use gtk::glib;
use gtk::subclass::prelude::*;
use ox_core::ops::{
    run_transfer, starting_label, OperationContext, OpsError, TransferOutcome, TransferRequest, UndoRecord,
    STOPPED_TITLE,
};
use ox_core::transfer::{Cancellation, Progress};

use crate::app_context::WriteHold;

use super::unfinished::mark_unfinished;
use crate::dialog;
use crate::search::changed_folders;
use crate::window::loading::LoadMode;
use crate::window::transfer_panel::{TransferKind, TransferPanel};
use crate::window::BrowserWindow;

/// How long a quick write runs before its panel shows, as Windows shows
/// progress only for an operation that takes a while.
const QUICK_WRITE_PANEL_DELAY: Duration = Duration::from_secs(1);

/// A write [`BrowserWindow::begin_quick_write`] started.
#[derive(Debug)]
pub(super) struct QuickWrite {
    /// The timer that shows its panel whenever the panel is free.
    panel_timer: glib::SourceId,
}

/// Why files are not written while a drive is formatted.
const FORMATTING_A_DRIVE: &str =
    crate::i18n::message_id("Wait until the drive is formatted, then try again.");

/// What a finished operation leaves behind.
#[derive(Debug)]
pub(super) struct FinishedOperation {
    /// How Undo reverses it, when it can.
    pub(super) undo: Option<UndoRecord>,
    /// The items to select once the folder is listed again: where its new
    /// or moved items are now, or after a deletion the item that followed
    /// the removed ones.
    pub(super) select_after: Vec<String>,
}

impl FinishedOperation {
    /// What a finished run of the transfer engine leaves behind.
    pub(super) fn of_transfer(outcome: TransferOutcome) -> Self {
        Self {
            undo: outcome.undo,
            select_after: outcome.created,
        }
    }
}

impl BrowserWindow {
    /// The primary panel, used by an exclusive operation or one transfer job.
    /// Concurrent jobs use additional panels managed by `jobs`.
    pub(in crate::window) fn transfer_panel(&self) -> &TransferPanel {
        self.imp()
            .transfer_panel
            .get()
            .expect("the window makes its panel when constructed")
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
        context.item_failure = Some(self.failure_asker(&context.cancel));
        if !self.claim_transfer_panel(TransferKind::Files, label, &context.cancel) {
            return None;
        }
        self.imp().file_operations.borrow_mut().running = Some(context.cancel.clone());
        self.update_file_commands();
        Some(context)
    }

    /// Counts a short write, New ▸ Text document, as the window's running
    /// operation, which `cancel` stops: other file commands wait, Cancel
    /// stops it, and closing or Quit asks or waits as for any running
    /// operation (OPS-024, TAB-049). As in Windows, its panel, reading
    /// `label` with its Cancel button, shows only once it has run for
    /// [`QUICK_WRITE_PANEL_DELAY`], so a quick one looks instant, its
    /// progress window then showing at once rather than waiting again, and
    /// once no other write holds the panel: it looks again each
    /// [`QUICK_WRITE_PANEL_DELAY`] while it runs, so it shows again after
    /// an archive operation took the panel and finished. `None`, as
    /// [`Self::begin_operation`] refuses, while any write runs, an
    /// extraction or compression on the panel included. End it with
    /// [`Self::end_quick_write`].
    pub(super) fn begin_quick_write(&self, label: &'static str, cancel: &Cancellation) -> Option<QuickWrite> {
        if self.is_writing_files() {
            return None;
        }
        self.imp().file_operations.borrow_mut().running = Some(cancel.clone());
        // The file and archive commands wait while it writes.
        self.update_file_commands();
        self.update_archive_actions();
        let panel_timer = glib::timeout_add_local(
            QUICK_WRITE_PANEL_DELAY,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[strong]
                cancel,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    // An archive operation that checked before this write
                    // began may hold the panel, or take it from this
                    // write later; it keeps it, and the panel shows this
                    // write again once it is free.
                    let panel = window.transfer_panel();
                    if !panel.is_busy() && !cancel.is_cancelled() {
                        // It waited its moment already: the window shows
                        // at once.
                        panel.start_after_waiting(TransferKind::Create, label, cancel.clone());
                    }
                    glib::ControlFlow::Continue
                }
            ),
        );
        Some(QuickWrite { panel_timer })
    }

    /// Forgets the write of [`Self::begin_quick_write`] and hides its
    /// panel, if it showed and is still its own. The file and archive
    /// commands, which wait while it writes, are enabled again for what
    /// is shown, which may be another tab by now.
    pub(super) fn end_quick_write(&self, write: QuickWrite, cancel: &Cancellation) {
        write.panel_timer.remove();
        if self.transfer_panel().runs(cancel) {
            self.transfer_panel().finish();
        }
        self.imp().file_operations.borrow_mut().running = None;
        self.update_file_commands();
        self.update_archive_actions();
    }

    /// The one way an operation that runs alone (a file operation, an
    /// extraction, a compression or a restored copy) takes the primary
    /// panel. Refuses, leaving the panel to its owner, while this window
    /// writes files; otherwise shows the panel reading `label`, whose
    /// Cancel stops `cancel`. Callers that check and then await must
    /// claim after the await, so nothing started meanwhile loses its
    /// panel. Transfer jobs take their panels in `begin_transfer`, and a
    /// quick write shows its own late in `begin_quick_write`.
    pub(in crate::window) fn claim_transfer_panel(
        &self,
        kind: TransferKind,
        label: &str,
        cancel: &Cancellation,
    ) -> bool {
        if self.is_writing_files() {
            return false;
        }
        self.transfer_panel().start(kind, label, cancel.clone());
        true
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
        let operations = self.imp().file_operations.borrow();
        // A write without a panel has only its token.
        if let Some(running) = &operations.running {
            running.cancel();
        }
        for job in &operations.jobs {
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
        let verb = crate::window::transfer_panel::Verb::of_mode(request.mode);
        let destination = request.destination_folder.as_deref();
        let action = crate::window::transfer_panel::Action::of(verb, &request.uris, destination);
        self.describe_operation(&context.cancel, action);
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
            ..FinishedOperation::of_transfer(outcome)
        });
        self.conclude_operation_in(finished, origin.as_deref()).await;
    }

    /// Concludes an ended operation: Undo remembers it and the folder is
    /// listed again with its items selected. Nothing more is said, as in
    /// Windows Explorer, except "Operation stopped" for a request refused
    /// before it started.
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
