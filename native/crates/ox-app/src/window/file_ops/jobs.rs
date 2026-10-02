// SPDX-License-Identifier: AGPL-3.0-only
//! Bounded concurrent transfers. Conflicting source/destination trees stay exclusive.
use crate::window::transfer_panel::{TransferKind, TransferPanel};
use crate::window::BrowserWindow;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::ops::OperationContext;
use ox_core::transfer::Cancellation;

pub(super) const MAX_JOBS: usize = 4;

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
        if self.refuses_writes_during_update() {
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
        context.item_failure = Some(self.failure_asker());
        let panel = if self.transfer_panel().is_busy() {
            let panel: TransferPanel = glib::Object::new();
            self.imp().transfer_panels.append(&panel);
            panel
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
            if &job.panel != self.transfer_panel() {
                self.imp().transfer_panels.remove(&job.panel);
            }
        }
        drop(operations);
        self.update_file_commands();
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
