// SPDX-License-Identifier: AGPL-3.0-only
//! The handle of a listing that runs on its own worker thread.
//!
//! The result can be awaited from the GTK main loop (the handle is a
//! [`Future`]) or waited for on another thread. It does not depend on any
//! main context, so tests can use it directly.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

use crate::entry::{EnumerateError, EnumerationSummary};

/// The outcome of a listing.
type Outcome = Result<EnumerationSummary, EnumerateError>;

/// Shown when the worker ended without a result, for example because the
/// batch callback panicked or the thread could not start.
const STOPPED_EARLY: &str = "The folder listing stopped before it finished.";

/// A listing running on a worker thread. See [`super::enumerate`].
#[derive(Debug)]
pub struct EnumerationTask {
    shared: Arc<Shared>,
}

impl EnumerationTask {
    /// Runs `work` on a new, detached worker thread.
    pub(super) fn spawn(work: impl FnOnce() -> Outcome + Send + 'static) -> Self {
        let task = Self::new();
        let completion = Completion {
            shared: Arc::clone(&task.shared),
        };
        // If the thread cannot start, the closure is dropped, and with it
        // the `Completion`, which finishes the task with an error.
        let _detached = std::thread::Builder::new()
            .name("ox-enumerate".into())
            .spawn(move || completion.finish(work()));
        task
    }

    fn new() -> Self {
        Self {
            shared: Arc::new(Shared::default()),
        }
    }

    /// Blocks the calling thread until the listing ends. Never call this on
    /// the GTK main thread; await the task there instead.
    pub fn wait(self) -> Outcome {
        let mut state = self.shared.lock();
        loop {
            if let Some(result) = state.result.take() {
                return result;
            }
            state = self
                .shared
                .finished
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

impl Future for EnumerationTask {
    type Output = Outcome;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.shared.lock();
        match state.result.take() {
            Some(result) => Poll::Ready(result),
            None => {
                state.waker = Some(context.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// State shared between the worker and the [`EnumerationTask`].
#[derive(Debug, Default)]
struct Shared {
    state: Mutex<TaskState>,
    finished: Condvar,
}

#[derive(Debug, Default)]
struct TaskState {
    result: Option<Outcome>,
    finished: bool,
    waker: Option<Waker>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, TaskState> {
        // The lock is never held across code that can panic, so a poisoned
        // mutex still holds consistent state.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records the first result only, then wakes whoever is waiting.
    fn finish(&self, result: Outcome) {
        let waker = {
            let mut state = self.lock();
            if state.finished {
                return;
            }
            state.finished = true;
            state.result = Some(result);
            state.waker.take()
        };
        self.finished.notify_all();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

/// Owned by the worker. If the worker ends without a result, dropping this
/// still completes the task, so nobody waits forever.
struct Completion {
    shared: Arc<Shared>,
}

impl Completion {
    fn finish(self, result: Outcome) {
        self.shared.finish(result);
    }
}

impl Drop for Completion {
    fn drop(&mut self) {
        self.shared
            .finish(Err(EnumerateError::Other(STOPPED_EARLY.to_string())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_result_is_kept() {
        let task = EnumerationTask::new();
        let completion = Completion {
            shared: Arc::clone(&task.shared),
        };
        completion.finish(Err(EnumerateError::Cancelled));
        assert_eq!(task.wait(), Err(EnumerateError::Cancelled));
    }

    #[test]
    fn a_dropped_worker_still_completes_the_task() {
        let task = EnumerationTask::new();
        drop(Completion {
            shared: Arc::clone(&task.shared),
        });
        assert_eq!(task.wait(), Err(EnumerateError::Other(STOPPED_EARLY.into())));
    }

    #[test]
    fn a_panicking_worker_still_completes_the_task() {
        let task = EnumerationTask::spawn(|| panic!("batch callback failed"));
        assert_eq!(task.wait(), Err(EnumerateError::Other(STOPPED_EARLY.into())));
    }

    #[test]
    fn the_task_can_be_awaited() {
        let summary = EnumerationSummary {
            uri: "file:///tmp".into(),
            count: 3,
        };
        let expected = summary.clone();
        let task = EnumerationTask::spawn(move || Ok(summary));
        let result = glib::MainContext::new().block_on(task);
        assert_eq!(result, Ok(expected));
    }
}
