// SPDX-License-Identifier: AGPL-3.0-only
//! Bounded concurrent transfers. Conflicting source/destination trees stay exclusive.
//! Each job is an action of the application's progress window.
use crate::window::transfer_panel::{Action, TransferKind, TransferPanel, Verb};
use crate::window::BrowserWindow;
use gtk::gio;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::ops::OperationContext;
use ox_core::transfer::Cancellation;

pub(in crate::window) const MAX_JOBS: usize = 4;

#[derive(Debug)]
pub(super) struct Job {
    cancel: Cancellation,
    pub(super) panel: TransferPanel,
    locations: Vec<String>,
}

fn overlaps(left: &str, right: &str) -> bool {
    let left = gio::File::for_uri(left);
    let right = gio::File::for_uri(right);
    left == right || left.has_prefix(&right) || right.has_prefix(&left)
}

impl BrowserWindow {
    /// Admits a transfer with its own panel and cancellation token, up to
    /// `MAX_JOBS`. Exclusive writes, update locks and overlapping source or
    /// destination paths refuse new jobs.
    pub(super) fn begin_transfer(
        &self,
        label: &str,
        uris: &[String],
        destination: Option<&str>,
    ) -> Option<OperationContext> {
        if self.refuses_writes() {
            return None;
        }
        let mut operations = self.imp().file_operations.borrow_mut();
        if operations.is_busy()
            || operations.jobs.len() >= MAX_JOBS
            || (operations.jobs.is_empty() && self.transfer_panel().is_busy())
        {
            return None;
        }
        let locations: Vec<String> = uris
            .iter()
            .cloned()
            .chain(destination.map(str::to_owned))
            .collect();
        if operations.jobs.iter().any(|job| {
            job.locations
                .iter()
                .any(|active| locations.iter().any(|next| overlaps(active, next)))
        }) {
            drop(operations);
            self.show_message(ox_core::i18n::gettext_static(
                "Wait for the operation using these files or folders to finish.",
            ));
            return None;
        }
        let mut context = OperationContext::new(self.context().write_protection());
        context.unstorable = Some(self.unstorable_asker());
        context.move_by_copying = Some(self.move_by_copying_asker());
        context.item_failure = Some(self.failure_asker(&context.cancel));
        context.replaced_files = self.replaced_files();
        let panel = if self.transfer_panel().is_busy() {
            TransferPanel::for_window(self)
        } else {
            self.transfer_panel().clone()
        };
        panel.start(TransferKind::Files, label, context.cancel.clone());
        operations.jobs.push(Job {
            cancel: context.cancel.clone(),
            panel,
            locations,
        });
        drop(operations);
        self.update_file_commands();
        Some(context)
    }

    pub(super) fn panel_for_operation(&self, cancel: &Cancellation) -> TransferPanel {
        self.imp()
            .file_operations
            .borrow()
            .jobs
            .iter()
            .find(|job| job.cancel.cancellable() == cancel.cancellable())
            .map_or_else(|| self.transfer_panel().clone(), |job| job.panel.clone())
    }

    pub(super) fn end_transfer(&self, cancel: &Cancellation) {
        let mut operations = self.imp().file_operations.borrow_mut();
        if let Some(index) = operations
            .jobs
            .iter()
            .position(|job| job.cancel.cancellable() == cancel.cancellable())
        {
            let job = operations.jobs.remove(index);
            job.panel.finish();
        }
        drop(operations);
        self.update_file_commands();
    }

    /// Says what the operation that `cancel` stops does, in its action.
    pub(in crate::window) fn describe_operation(&self, cancel: &Cancellation, action: Option<Action>) {
        if let Some(action) = action {
            self.panel_for_operation(cancel).describe(action);
        }
    }

    /// Says that the operation `cancel` stops extracts `request`'s
    /// archive into its folder.
    pub(in crate::window) fn describe_extraction(
        &self,
        cancel: &Cancellation,
        request: &ox_core::archive::ExtractionRequest,
    ) {
        let archive = [request.archive_uri.clone()];
        let action = Action::of(Verb::Extracting, &archive, Some(&request.destination_uri));
        self.describe_operation(cancel, action);
    }

    /// Says that the operation `cancel` stops compresses `request`'s
    /// items.
    pub(in crate::window) fn describe_compression(
        &self,
        cancel: &Cancellation,
        request: &ox_core::archive::CompressionRequest,
    ) {
        let action = Action::of(Verb::Compressing, &request.uris, Some(&request.destination_uri));
        self.describe_operation(cancel, action);
    }

    /// Ends the window's actions in the progress window when it goes:
    /// their operations are cancelled, and closing asked first while they
    /// ran (TAB-049).
    pub(in crate::window) fn end_progress_actions(&self) {
        let jobs: Vec<TransferPanel> = self
            .imp()
            .file_operations
            .borrow()
            .jobs
            .iter()
            .map(|job| job.panel.clone())
            .collect();
        let primary = self.imp().transfer_panel.get().cloned();
        for panel in jobs.into_iter().chain(primary) {
            panel.cancel();
            panel.finish();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::harness::TestWindow;
    /// parity: OPS-025, OPS-033
    #[gtk::test]
    fn separate_jobs_have_independent_cancel_and_release_their_slots() {
        let test = TestWindow::without_tabs();
        let first = test
            .window
            .begin_transfer("Copying", &["file:///tmp/one".into()], Some("file:///tmp/first"))
            .unwrap();
        let second = test
            .window
            .begin_transfer("Moving", &["file:///tmp/two".into()], Some("file:///tmp/second"))
            .unwrap();
        assert!(test
            .window
            .begin_transfer("Copying", &["file:///tmp/one/child".into()], None)
            .is_none());
        test.window.panel_for_operation(&first.cancel).cancel();
        assert!(first.cancel.is_cancelled());
        assert!(!second.cancel.is_cancelled());
        let third = test
            .window
            .begin_transfer("Copying", &["file:///tmp/three".into()], None)
            .unwrap();
        let fourth = test
            .window
            .begin_transfer("Copying", &["file:///tmp/four".into()], None)
            .unwrap();
        assert!(test
            .window
            .begin_transfer("Copying", &["file:///tmp/five".into()], None)
            .is_none());
        test.window.end_transfer(&third.cancel);
        test.window.end_transfer(&fourth.cancel);
        test.window.end_transfer(&first.cancel);
        assert!(test.window.is_writing_files());
        assert!(test.window.begin_operation("Exclusive archive").is_none());
        test.window.end_transfer(&second.cancel);
        assert!(!test.window.is_writing_files());
    }
}
