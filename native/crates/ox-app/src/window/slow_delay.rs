// SPDX-License-Identifier: AGPL-3.0-only
//! When a pane's location counts as slow: once it has taken longer than
//! [`APPEARANCE_DELAY`] to open or list. Only then does the file area
//! say "Working on it..." and the status bar "Loading…", so a location
//! that answers sooner flashes nothing (ui-spec.md M06). The progress
//! itself shows in the address bar from the first moment, as in Windows
//! Explorer (VIEW-068); nothing is drawn over the items.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;

/// How long a location takes before it counts as slow. Shorter listings,
/// which are most of them, show nothing in the file area: Dolphin waits
/// as long before it says a folder is loading.
pub(crate) const APPEARANCE_DELAY: Duration = Duration::from_millis(300);

/// What a [`SlowDelay`] shares with its timer.
#[derive(Default)]
struct State {
    /// The timer that will mark the location slow, while one runs. It
    /// clears itself when it fires, so it is never removed twice.
    pending: RefCell<Option<glib::SourceId>>,
    /// The running location has taken longer than the delay.
    slow: Cell<bool>,
    /// Called each time a location becomes slow.
    on_slow: RefCell<Vec<Box<dyn Fn()>>>,
}

/// Tells when the location of one pane has taken longer than the delay.
#[derive(Default)]
pub(crate) struct SlowDelay(Rc<State>);

impl std::fmt::Debug for SlowDelay {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SlowDelay")
            .field("slow", &self.0.slow.get())
            .finish_non_exhaustive()
    }
}

impl Drop for SlowDelay {
    fn drop(&mut self) {
        self.cancel_pending();
    }
}

impl SlowDelay {
    /// Starts the delay when a location starts loading, or ends it at
    /// once when it is over.
    pub(crate) fn set_running(&self, running: bool) {
        if !running {
            self.cancel_pending();
            self.0.slow.set(false);
            return;
        }
        let already_on_its_way = self.0.slow.get() || self.0.pending.borrow().is_some();
        if already_on_its_way {
            return;
        }
        let state = Rc::downgrade(&self.0);
        let timer = glib::timeout_add_local_once(APPEARANCE_DELAY, move || {
            let Some(state) = state.upgrade() else { return };
            state.pending.take();
            state.slow.set(true);
            for on_slow in state.on_slow.borrow().iter() {
                on_slow();
            }
        });
        self.0.pending.replace(Some(timer));
    }

    /// Whether the running location has taken longer than the delay.
    pub(crate) fn is_slow(&self) -> bool {
        self.0.slow.get()
    }

    /// Calls `on_slow` each time a location becomes slow.
    pub(crate) fn connect_slow(&self, on_slow: impl Fn() + 'static) {
        self.0.on_slow.borrow_mut().push(Box::new(on_slow));
    }

    fn cancel_pending(&self) {
        if let Some(timer) = self.0.pending.take() {
            timer.remove();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::test_support::harness::{wait_for, wait_until};

    /// parity: VIEW-047
    #[gtk::test]
    fn a_location_counts_as_slow_once_it_has_run_for_the_delay() {
        let delay = SlowDelay::default();
        let told = Rc::new(Cell::new(0));
        let counted = Rc::clone(&told);
        delay.connect_slow(move || counted.set(counted.get() + 1));
        let started = Instant::now();
        delay.set_running(true);
        assert!(!delay.is_slow(), "a location that just started is not slow");
        wait_until("the delay", || delay.is_slow());
        assert!(started.elapsed() >= APPEARANCE_DELAY);
        assert_eq!(told.get(), 1);
        delay.set_running(false);
        assert!(!delay.is_slow(), "the end of the location ends it");
    }

    #[gtk::test]
    fn a_location_that_ends_at_once_never_counts_as_slow() {
        let delay = SlowDelay::default();
        delay.set_running(true);
        delay.set_running(false);
        wait_for(APPEARANCE_DELAY * 2);
        assert!(!delay.is_slow());
    }
}
