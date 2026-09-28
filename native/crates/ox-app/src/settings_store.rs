// SPDX-License-Identifier: AGPL-3.0-only
//! The shared settings file, read and written off the GTK main thread.
//!
//! Holds what the Python application kept in `settings_store` (the
//! `Settings` of `desktop/core.py`, shared through `desktop/winspace.py`).
//! Every window reads the same [`Settings`] handle. Changes and re-reads
//! are queued and run one after another on a GIO worker thread: the file
//! lock can wait for the Python app, and nothing may block the interface
//! while it does. A change that fails leaves the data as last read, as
//! `Settings`' own change methods do.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use gtk::{gio, glib};
use ox_core::settings::{Settings, SettingsData, SettingsError};

/// A change applied to a copy of the settings on a worker thread.
pub(crate) type Change = Box<dyn FnOnce(&mut Settings) -> Result<(), SettingsError> + Send>;

/// Called on the main thread with the outcome of a [`Change`].
pub(crate) type Reply = Box<dyn FnOnce(Result<(), SettingsError>)>;

/// A change waiting for the worker, and who hears its outcome.
struct QueuedChange {
    change: Change,
    reply: Reply,
}

/// The settings every window shares, with a queue of pending changes.
pub(crate) struct SettingsStore {
    /// The data as last read or successfully changed.
    settings: RefCell<Settings>,
    /// Changes waiting for the one on the worker to finish.
    pending: RefCell<VecDeque<QueuedChange>>,
    /// A change is running on the worker thread.
    worker_busy: Cell<bool>,
}

impl std::fmt::Debug for SettingsStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SettingsStore")
            .field("settings", &self.settings)
            .field("pending", &self.pending.borrow().len())
            .field("worker_busy", &self.worker_busy.get())
            .finish()
    }
}

impl SettingsStore {
    /// Shares `settings`, as read at startup.
    pub(crate) fn new(settings: Settings) -> Rc<Self> {
        Rc::new(Self {
            settings: RefCell::new(settings),
            pending: RefCell::new(VecDeque::new()),
            worker_busy: Cell::new(false),
        })
    }

    /// A copy of the data as last read or changed.
    pub(crate) fn data(&self) -> SettingsData {
        self.settings.borrow().data().clone()
    }

    /// Why the last read fell back to defaults, if it did.
    pub(crate) fn warning(&self) -> Option<String> {
        self.settings.borrow().warning().map(str::to_owned)
    }

    /// Queues `change`; `reply` hears its outcome once every earlier change
    /// has finished.
    pub(crate) fn change(self: &Rc<Self>, change: Change, reply: Reply) {
        self.pending
            .borrow_mut()
            .push_back(QueuedChange { change, reply });
        self.run_next();
    }

    /// Queues a re-read of the file, for changes the Python app or another
    /// process made. `reply` hears whether the data changed.
    pub(crate) fn reload(self: &Rc<Self>, reply: impl FnOnce(bool) + 'static) {
        let before = self.data();
        let store = Rc::downgrade(self);
        let reload: Change = Box::new(|settings| {
            settings.reload();
            Ok(())
        });
        let after_reload: Reply = Box::new(move |_| {
            let changed = store.upgrade().is_some_and(|store| store.data() != before);
            reply(changed);
        });
        self.change(reload, after_reload);
    }

    /// Starts the oldest queued change unless one is running already.
    fn run_next(self: &Rc<Self>) {
        if self.worker_busy.get() {
            return;
        }
        let Some(queued) = self.pending.borrow_mut().pop_front() else {
            return;
        };
        self.worker_busy.set(true);
        // Data safety: the change works on a copy, which replaces the data
        // only when the change succeeds, so a failed change leaves the
        // data as last read.
        let copy = self.settings.borrow().clone();
        let store = Rc::clone(self);
        glib::spawn_future_local(async move {
            let (changed, result) = apply_on_worker(copy, queued.change).await;
            if result.is_ok() {
                store.settings.replace(changed);
            }
            store.worker_busy.set(false);
            (queued.reply)(result);
            store.run_next();
        });
    }
}

/// Applies `change` to `settings` on a GIO worker thread, where waiting
/// for the settings lock cannot block the interface, and hands both back.
async fn apply_on_worker(mut settings: Settings, change: Change) -> (Settings, Result<(), SettingsError>) {
    let worker = gio::spawn_blocking(move || {
        let result = change(&mut settings);
        (settings, result)
    });
    match worker.await {
        Ok(outcome) => outcome,
        // A panicking change is a bug; it surfaces on the main thread.
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use ox_core::settings::PreferencesUpdate;

    use super::*;
    use crate::test_support::harness::wait_until;

    /// A store over a fresh settings directory, which the caller keeps.
    fn store_in(directory: &tempfile::TempDir) -> Rc<SettingsStore> {
        SettingsStore::new(Settings::open(directory.path()))
    }

    /// Writes a text size to the settings file, as another window or the
    /// Python app would.
    fn save_text_size_elsewhere(directory: &tempfile::TempDir, percent: u32) {
        let update = PreferencesUpdate {
            text_size: Some(percent),
            ..PreferencesUpdate::default()
        };
        Settings::open(directory.path())
            .update_preferences(&update)
            .expect("the test directory accepts settings");
    }

    #[gtk::test]
    fn a_failed_change_leaves_the_data_as_last_read() {
        let directory = tempfile::tempdir().expect("a settings directory");
        let store = store_in(&directory);
        save_text_size_elsewhere(&directory, 125);
        let outcome = Rc::new(RefCell::new(None));
        let heard = Rc::clone(&outcome);
        let reload_then_fail: Change = Box::new(|settings| {
            settings.reload();
            Err(SettingsError::Invalid("refused".to_owned()))
        });
        store.change(
            reload_then_fail,
            Box::new(move |result| {
                heard.replace(Some(result.is_ok()));
            }),
        );
        wait_until("the change to finish", || outcome.borrow().is_some());
        assert_eq!(*outcome.borrow(), Some(false), "the reply hears the failure");
        assert_eq!(store.data().preferences.text_size, 100);
    }

    /// parity: SIDE-022
    #[gtk::test]
    fn a_reload_reports_whether_another_process_changed_the_file() {
        let directory = tempfile::tempdir().expect("a settings directory");
        let store = store_in(&directory);
        let changes = Rc::new(RefCell::new(Vec::new()));
        let heard = Rc::clone(&changes);
        store.reload(move |changed| heard.borrow_mut().push(changed));
        wait_until("the first reload", || changes.borrow().len() == 1);
        save_text_size_elsewhere(&directory, 150);
        let heard = Rc::clone(&changes);
        store.reload(move |changed| heard.borrow_mut().push(changed));
        wait_until("the second reload", || changes.borrow().len() == 2);
        assert_eq!(*changes.borrow(), [false, true]);
        assert_eq!(store.data().preferences.text_size, 150);
    }

    /// parity: VIEW-045
    #[gtk::test]
    fn changes_run_one_after_another_in_the_order_queued() {
        let directory = tempfile::tempdir().expect("a settings directory");
        let store = store_in(&directory);
        let started = Arc::new(Mutex::new(Vec::new()));
        let replies = Rc::new(RefCell::new(Vec::new()));
        for number in 1..=3 {
            let started = Arc::clone(&started);
            let change: Change = Box::new(move |_| {
                // The first change is the slowest: were changes run side by
                // side, a later one would start before it finished.
                std::thread::sleep(Duration::from_millis(30 / number));
                started.lock().expect("no test thread panics").push(number);
                Ok(())
            });
            let heard = Rc::clone(&replies);
            store.change(change, Box::new(move |_| heard.borrow_mut().push(number)));
        }
        wait_until("every change to finish", || replies.borrow().len() == 3);
        assert_eq!(*started.lock().expect("no test thread panics"), [1, 2, 3]);
        assert_eq!(*replies.borrow(), [1, 2, 3]);
    }
}
