// SPDX-License-Identifier: AGPL-3.0-only
//! Coalesces worker progress so small-file transfers cannot queue an
//! unbounded number of labels or keep GTK busy replaying old reports.
//! Batch and file progress drive separate bars, so keep the latest of
//! each until the main loop can show them. Reports contain cumulative
//! byte totals; skipped intermediate reports do not lose transferred bytes.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gtk::glib;
use ox_core::transfer::{Cancellation, Progress, ProgressScope};

use crate::window::transfer_panel::TransferPanel;

/// At most twenty progress refreshes per second, yielding the main loop
/// between them even when the worker produces reports continuously.
const UPDATE_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Default)]
struct PendingProgress {
    batch: Option<Progress>,
    file: Option<Progress>,
}

impl PendingProgress {
    fn push(&mut self, progress: Progress) {
        match progress.scope {
            ProgressScope::Batch => {
                self.batch = Some(progress);
                // A new batch report hides the preceding file's bar.
                self.file = None;
            }
            ProgressScope::File => self.file = Some(progress),
        }
    }
}

/// Latest reports and one wake-up, regardless of how many were produced.
struct ProgressUpdates {
    pending: Arc<Mutex<PendingProgress>>,
    ready: async_channel::Receiver<()>,
}

impl ProgressUpdates {
    async fn next(&self) -> Option<PendingProgress> {
        self.ready.recv().await.ok()?;
        glib::timeout_future(UPDATE_INTERVAL).await;
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Some(std::mem::take(&mut *pending))
    }
}

fn progress_channel() -> (impl FnMut(Progress) + Send, ProgressUpdates) {
    let pending = Arc::new(Mutex::new(PendingProgress::default()));
    let updates = Arc::clone(&pending);
    let (notify, ready) = async_channel::bounded(1);
    let report = move |progress| {
        pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(progress);
        // A full channel already has the only wake-up the latest reports
        // need. Reporting never waits for the GTK thread to catch up.
        let _ = notify.try_send(());
    };
    (
        report,
        ProgressUpdates {
            pending: updates,
            ready,
        },
    )
}

/// A worker's progress sink for the panel that owns `cancel`.
pub(super) fn report_to(
    panel: &TransferPanel,
    cancel: Cancellation,
) -> impl FnMut(Progress) + Send + 'static {
    let (report, updates) = progress_channel();
    glib::spawn_future_local(glib::clone!(
        #[weak]
        panel,
        async move {
            while let Some(progress) = updates.next().await {
                if !cancel.is_cancelled() && panel.tracks(&cancel) {
                    for report in progress.batch.into_iter().chain(progress.file) {
                        panel.show_progress(&report);
                    }
                }
            }
        }
    ));
    report
}

#[cfg(test)]
mod tests {
    use ox_core::transfer::ByteProgress;

    use super::*;
    use crate::test_support::harness::wait_until;
    use crate::window::transfer_panel::TransferKind;

    fn batch(label: &str, fraction: f64) -> Progress {
        Progress {
            label: label.to_owned(),
            fraction,
            scope: ProgressScope::Batch,
            bytes: None,
        }
    }

    fn file(number: u64) -> Progress {
        Progress {
            label: format!("Copying sample {number}.txt"),
            fraction: 0.5,
            scope: ProgressScope::File,
            bytes: Some(ByteProgress {
                file_written: 5,
                file_size: 10,
                batch_written: number * 10 + 5,
                batch_size: None,
            }),
        }
    }

    #[test]
    fn a_busy_main_loop_keeps_only_the_latest_batch_and_file_reports() {
        let (mut report, updates) = progress_channel();
        std::thread::spawn(move || {
            for number in 0..100_000 {
                report(batch(&format!("Copying sample {number}"), 0.25));
                report(file(number));
            }
        })
        .join()
        .unwrap();

        assert_eq!(updates.ready.len(), 1, "one wake-up for 200,000 reports");
        let context = glib::MainContext::new();
        let received = context.block_on(updates.next()).unwrap();
        assert_eq!(received.batch, Some(batch("Copying sample 99999", 0.25)));
        assert_eq!(received.file, Some(file(99_999)));
        assert!(context.block_on(updates.next()).is_none(), "the sender closed");
    }

    #[test]
    fn completing_a_batch_discards_its_old_file_progress() {
        let (mut report, updates) = progress_channel();
        report(file(1));
        report(batch("Finished", 1.0));
        drop(report);

        let received = glib::MainContext::new().block_on(updates.next()).unwrap();

        assert_eq!(received.batch, Some(batch("Finished", 1.0)));
        assert!(received.file.is_none());
    }

    #[gtk::test]
    fn queued_reports_preserve_both_bars_and_do_not_replace_a_new_job() {
        let panel: TransferPanel = glib::Object::new();
        let cancel = Cancellation::new();
        panel.start(TransferKind::Files, "Copying", cancel.clone());
        let mut report = report_to(&panel, cancel);
        report(batch("Batch", 0.25));
        report(file(1));
        wait_until("the latest file report", || panel.status_text() == file(1).label);
        assert_eq!(panel.fractions(), (0.25, Some(0.5)));

        report(file(2));
        panel.finish();
        panel.start(TransferKind::Files, "New operation", Cancellation::new());
        drop(report);
        glib::MainContext::default().block_on(glib::timeout_future(UPDATE_INTERVAL * 3));
        assert_eq!(panel.status_text(), "New operation");
        panel.finish();
    }
}
