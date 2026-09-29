// SPDX-License-Identifier: AGPL-3.0-only
//! Progress for the transfer panel (OPS-019): the label shown before the
//! first report, and the rate at which reports reach the interface.
//!
//! Ports the `progress` closure of the `operate` branch of `dispatch` in
//! `desktop/winspace.py`, which forwards a report only when 80 ms have
//! passed since the last one or the report says the work is complete, and
//! the starting labels of `runOperation` in `desktop/ui/app.js`.

use std::time::{Duration, Instant};

use crate::transfer::{Progress, TransferMode};

/// The shortest time between two progress reports.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(80);

/// The panel's label from the start of an operation until the first
/// progress report arrives.
pub fn starting_label(mode: TransferMode) -> &'static str {
    match mode {
        TransferMode::Trash => "Moving to Trash…",
        TransferMode::Delete => "Deleting items…",
        TransferMode::Move => "Moving items…",
        TransferMode::Copy => "Preparing copy…",
    }
}

/// Lets through at most one progress report per [`PROGRESS_INTERVAL`],
/// and always the report that says the work is complete.
#[derive(Debug, Default)]
pub(crate) struct ProgressThrottle {
    last_delivery: Option<Instant>,
}

impl ProgressThrottle {
    /// True when `progress`, reported at `now`, should reach the interface.
    pub(crate) fn admits(&mut self, progress: &Progress, now: Instant) -> bool {
        let interval_has_passed = self
            .last_delivery
            .is_none_or(|last| now.duration_since(last) >= PROGRESS_INTERVAL);
        let is_complete = progress.fraction >= 1.0;
        if !(interval_has_passed || is_complete) {
            return false;
        }
        self.last_delivery = Some(now);
        true
    }
}

/// `sink`, receiving only the reports [`ProgressThrottle`] admits.
pub(crate) fn throttled(
    mut sink: impl FnMut(Progress) + Send + 'static,
) -> impl FnMut(Progress) + Send + 'static {
    let mut throttle = ProgressThrottle::default();
    move |progress: Progress| {
        if throttle.admits(&progress, Instant::now()) {
            sink(progress);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(fraction: f64) -> Progress {
        Progress {
            label: String::from("Copying a"),
            fraction,
            scope: crate::transfer::ProgressScope::File,
        }
    }

    #[test]
    fn reports_are_spaced_by_the_interval_but_completion_always_arrives() {
        let start = Instant::now();
        let mut throttle = ProgressThrottle::default();

        let first = throttle.admits(&progress(0.1), start);
        let too_soon = throttle.admits(&progress(0.2), start + Duration::from_millis(40));
        let complete = throttle.admits(&progress(1.0), start + Duration::from_millis(50));
        let later = throttle.admits(&progress(0.3), start + Duration::from_millis(130));

        assert!(first);
        assert!(!too_soon);
        assert!(complete);
        assert!(later);
    }

    #[test]
    fn the_panel_names_the_operation_before_the_first_report() {
        assert_eq!(starting_label(TransferMode::Trash), "Moving to Trash…");
        assert_eq!(starting_label(TransferMode::Delete), "Deleting items…");
        assert_eq!(starting_label(TransferMode::Move), "Moving items…");
        assert_eq!(starting_label(TransferMode::Copy), "Preparing copy…");
    }
}
