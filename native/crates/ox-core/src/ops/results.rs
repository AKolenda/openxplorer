// SPDX-License-Identifier: AGPL-3.0-only
//! One result per operation for the interface, from the operations that
//! handle items outside the transfer engine (the Recycle Bin, renaming
//! back) or run it once per folder (Duplicate, the undo of a move). They
//! report in the engine's form, [`TransferResult`], so the interface
//! summarises every operation the same way (`report`).

use super::error::OpsError;
use crate::transfer::TransferResult;

/// Adds one item's failure to `result`: the user's cancellation stops the
/// run, anything else is listed as `name: reason`, the way the engine
/// lists its failures.
pub(crate) fn record_failure(result: &mut TransferResult, name: &str, error: &OpsError) {
    if error.is_cancelled() {
        result.cancelled = true;
    } else {
        result.errors.push(format!("{name}: {error}"));
    }
}

/// Adds the result of one run to the whole operation's.
pub(crate) fn merge_results(total: &mut TransferResult, part: TransferResult) {
    total.done.extend(part.done);
    total.skipped.extend(part.skipped);
    total.errors.extend(part.errors);
    total.cancelled |= part.cancelled;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancellation_stops_the_run_and_other_failures_are_listed_by_name() {
        let mut result = TransferResult::default();

        record_failure(&mut result, "a.txt", &OpsError::NotFound("Gone.".into()));
        record_failure(&mut result, "b.txt", &OpsError::Cancelled);

        assert_eq!(result.errors, ["a.txt: Gone."]);
        assert!(result.cancelled);
    }

    #[test]
    fn results_of_several_runs_add_up() {
        let mut total = TransferResult::default();
        let first = TransferResult {
            done: vec!["file:///a".into()],
            ..TransferResult::default()
        };
        let second = TransferResult {
            skipped: vec!["file:///b".into()],
            errors: vec!["c: failed".into()],
            cancelled: true,
            ..TransferResult::default()
        };

        merge_results(&mut total, first);
        merge_results(&mut total, second);

        assert_eq!(total.done, ["file:///a"]);
        assert_eq!(total.skipped, ["file:///b"]);
        assert_eq!(total.errors, ["c: failed"]);
        assert!(total.cancelled);
    }
}
