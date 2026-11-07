// SPDX-License-Identifier: AGPL-3.0-only
//! One result per operation for the interface, from the operations that
//! handle items outside the transfer engine (the Recycle Bin, renaming
//! back) or run it once per folder (Duplicate, the undo of a move). They
//! record in the engine's form, [`TransferResult`].
//!
//! As in Windows Explorer, which has no summary of an operation, these
//! operations ask about each failed item while it happens, in the progress
//! window, with the engine's question (OPS-047): [`ItemQuestions`].

use super::context::{FailureAsker, OperationContext};
use super::error::OpsError;
use crate::transfer::{FailedItem, FailureAnswer, FailureCause, TransferMode, TransferResult};

/// The questions about failed items of an operation that handles items
/// outside the transfer engine, worded as for a run in `mode`: Try Again
/// runs the item again, Skip leaves it out, Skip with "Do this for all
/// current items" leaves out every later failing item without asking, and
/// Cancel stops the operation, keeping what is done. Without a question
/// (a test, or a run without a window), a failure is recorded and the
/// next item runs.
#[derive(Debug)]
pub(crate) struct ItemQuestions {
    asker: Option<FailureAsker>,
    mode: TransferMode,
    skip_all: bool,
}

/// The item a failure is about.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AskedItem<'a> {
    /// Its name, as the question shows it and the result lists it.
    pub(crate) name: &'a str,
    /// Its URI, whose facts the question shows.
    pub(crate) uri: &'a str,
    /// Whether other items follow, so "Do this for all current items"
    /// means something.
    pub(crate) more_items: bool,
}

impl ItemQuestions {
    /// The questions of `context`, worded for a run in `mode`.
    pub(crate) fn new(context: &OperationContext, mode: TransferMode) -> Self {
        Self {
            asker: context.item_failure.clone(),
            mode,
            skip_all: false,
        }
    }

    /// Runs `attempt` for `item` until it succeeds, asking after each
    /// failure: its value, or `None` once the item is left out or the
    /// user cancelled, which `result` then records.
    pub(crate) fn run<T>(
        &mut self,
        result: &mut TransferResult,
        item: AskedItem<'_>,
        mut attempt: impl FnMut() -> Result<T, OpsError>,
    ) -> Option<T> {
        loop {
            let error = match attempt() {
                Ok(value) => return Some(value),
                Err(error) => error,
            };
            match self.ask(item, &error) {
                Some(FailureAnswer::Retry | FailureAnswer::Elevate | FailureAnswer::ElevateAll) => {}
                answer => {
                    record_answered(result, item.name, &error, answer);
                    return None;
                }
            }
        }
    }

    /// Asks about `error`, the failure of `item` that cannot be tried
    /// again, as "Interrupted Action" with its reason, and records it.
    pub(crate) fn tell(&mut self, result: &mut TransferResult, item: AskedItem<'_>, error: &OpsError) {
        if error.is_cancelled() {
            result.cancelled = true;
            return;
        }
        let shown = self.asker.as_ref().map(|asker| {
            asker.ask(&FailedItem {
                cause: FailureCause::NeedsAttention,
                more_items: false,
                ..self.failed_item(item, error)
            })
        });
        record_answered(result, item.name, error, shown.map(|_| FailureAnswer::Skip));
    }

    /// Tells, one by one, about each failure in `result` the user has not
    /// seen in a question, such as one recorded by a step that asks
    /// nothing itself, right after that step; each is then recorded as
    /// seen. A message two failures share is told twice only when neither
    /// was seen.
    pub(crate) fn tell_unseen(&mut self, result: &mut TransferResult) {
        let Some(asker) = self.asker.as_ref() else {
            return;
        };
        for message in unseen(result) {
            let (name, reason) = message.split_once(": ").unwrap_or(("", message.as_str()));
            asker.ask(&FailedItem {
                mode: self.mode,
                name: name.to_owned(),
                error: reason.to_owned(),
                more_items: false,
                cause: FailureCause::NeedsAttention,
                uri: String::new(),
                destination_folder: None,
                needed: None,
            });
            result.presented.push(message);
        }
    }

    /// The user's answer about `error`, `None` without a question; a
    /// cancellation is answered Cancel, and after "Skip all" every failure
    /// Skip.
    fn ask(&mut self, item: AskedItem<'_>, error: &OpsError) -> Option<FailureAnswer> {
        if error.is_cancelled() {
            return Some(FailureAnswer::Cancel);
        }
        let asker = self.asker.as_ref()?;
        if self.skip_all {
            return Some(FailureAnswer::Skip);
        }
        let answer = asker.ask(&self.failed_item(item, error));
        self.skip_all = answer == FailureAnswer::SkipAll;
        Some(answer)
    }

    /// The question about `item`, which failed with `error`.
    fn failed_item(&self, item: AskedItem<'_>, error: &OpsError) -> FailedItem {
        FailedItem {
            mode: self.mode,
            name: item.name.to_owned(),
            error: error.to_string(),
            more_items: item.more_items,
            cause: cause_of(error),
            uri: item.uri.to_owned(),
            destination_folder: None,
            needed: None,
        }
    }
}

/// The errors of `result` no question showed, in order. Each presented
/// message accounts for one error with that message.
fn unseen(result: &TransferResult) -> Vec<String> {
    let mut presented: Vec<&String> = result.presented.iter().collect();
    result
        .errors
        .iter()
        .filter(
            |error| match presented.iter().position(|message| message == error) {
                Some(seen) => {
                    presented.swap_remove(seen);
                    false
                }
                None => true,
            },
        )
        .cloned()
        .collect()
}

/// Which of Windows' questions `error` gets.
fn cause_of(error: &OpsError) -> FailureCause {
    match error {
        OpsError::PermissionDenied(_) => FailureCause::AccessDenied,
        OpsError::NotFound(_) => FailureCause::NotFound,
        _ => FailureCause::Other,
    }
}

/// Records the failure of the item `name` with `error` after `answer`:
/// the user's cancellation stops the run, and a failure the user saw is
/// recorded as answered.
fn record_answered(result: &mut TransferResult, name: &str, error: &OpsError, answer: Option<FailureAnswer>) {
    if error.is_cancelled() {
        result.cancelled = true;
        return;
    }
    let message = format!("{name}: {error}");
    if answer.is_some() {
        result.presented.push(message.clone());
    }
    result.cancelled |= answer == Some(FailureAnswer::Cancel);
    result.errors.push(message);
}

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
    total.recovery.extend(part.recovery);
    total.presented.extend(part.presented);
    total.cancelled |= part.cancelled;
    total.landed.extend(part.landed);
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn a_cancellation_stops_the_run_and_other_failures_are_listed_by_name() {
        let mut result = TransferResult::default();

        record_failure(&mut result, "a.txt", &OpsError::NotFound("Gone.".into()));
        record_failure(&mut result, "b.txt", &OpsError::Cancelled);

        assert_eq!(result.errors, ["a.txt: Gone."]);
        assert!(result.cancelled);
    }

    /// A context whose question answers each of `answers` in turn, and
    /// the questions it was asked.
    fn answering(answers: &[FailureAnswer]) -> (OperationContext, Arc<Mutex<Vec<FailedItem>>>) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&asked);
        let answers = Mutex::new(answers.to_vec());
        let context = OperationContext {
            item_failure: Some(FailureAsker::new(move |item: &FailedItem| {
                log.lock().unwrap().push(item.clone());
                answers.lock().unwrap().remove(0)
            })),
            ..OperationContext::default()
        };
        (context, asked)
    }

    fn item(name: &str) -> AskedItem<'_> {
        AskedItem {
            name,
            uri: "trash:///a.txt",
            more_items: true,
        }
    }

    /// parity: OPS-047, OPS-023
    #[test]
    fn a_failed_item_is_asked_about_and_try_again_runs_it_again() {
        let (context, asked) = answering(&[FailureAnswer::Retry]);
        let mut questions = ItemQuestions::new(&context, TransferMode::Delete);
        let mut result = TransferResult::default();
        let mut attempts = 0;

        let value = questions.run(&mut result, item("a.txt"), || {
            attempts += 1;
            if attempts == 1 {
                Err(OpsError::NotFound("Gone.".into()))
            } else {
                Ok(7)
            }
        });

        assert_eq!(value, Some(7));
        assert!(result.errors.is_empty());
        let asked = asked.lock().unwrap();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].cause, FailureCause::NotFound);
        assert_eq!(asked[0].mode, TransferMode::Delete);
        assert_eq!(asked[0].name, "a.txt");
    }

    /// parity: OPS-047, OPS-023
    #[test]
    fn skip_all_answers_every_later_failure_and_cancel_stops() {
        let (context, asked) = answering(&[FailureAnswer::SkipAll]);
        let mut questions = ItemQuestions::new(&context, TransferMode::Move);
        let mut result = TransferResult::default();
        let denied = || Err::<(), _>(OpsError::PermissionDenied("Denied.".into()));

        questions.run(&mut result, item("a.txt"), denied);
        questions.run(&mut result, item("b.txt"), denied);

        assert_eq!(asked.lock().unwrap().len(), 1, "Skip all asks once");
        assert_eq!(result.errors, ["a.txt: Denied.", "b.txt: Denied."]);
        assert_eq!(result.presented, result.errors, "both were the user's answer");
        assert!(!result.cancelled);

        let (context, _) = answering(&[FailureAnswer::Cancel]);
        let mut questions = ItemQuestions::new(&context, TransferMode::Move);
        let mut cancelled = TransferResult::default();
        questions.run(&mut cancelled, item("a.txt"), denied);
        assert!(cancelled.cancelled);
        assert_eq!(cancelled.presented, cancelled.errors);
    }

    /// parity: OPS-023
    #[test]
    fn a_failure_no_step_asked_about_is_told_once() {
        let (context, asked) = answering(&[FailureAnswer::Skip, FailureAnswer::Skip]);
        let mut questions = ItemQuestions::new(&context, TransferMode::Copy);
        let mut result = TransferResult {
            errors: vec!["a: seen".into(), "b: kept in the Recycle Bin".into()],
            presented: vec!["a: seen".into()],
            ..TransferResult::default()
        };

        questions.tell_unseen(&mut result);
        questions.tell_unseen(&mut result);

        let asked = asked.lock().unwrap();
        assert_eq!(asked.len(), 1, "{asked:?}");
        assert_eq!(asked[0].cause, FailureCause::NeedsAttention);
        assert_eq!(asked[0].name, "b");
        assert_eq!(asked[0].error, "kept in the Recycle Bin");
        assert_eq!(result.presented, result.errors);
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
            errors: vec!["c: failed".into(), "d: backup kept".into()],
            recovery: vec!["d: backup kept".into()],
            cancelled: true,
            ..TransferResult::default()
        };

        merge_results(&mut total, first);
        merge_results(&mut total, second);

        assert_eq!(total.done, ["file:///a"]);
        assert_eq!(total.skipped, ["file:///b"]);
        assert_eq!(total.errors, ["c: failed", "d: backup kept"]);
        assert_eq!(total.recovery, ["d: backup kept"]);
        assert!(total.cancelled);
    }
}
