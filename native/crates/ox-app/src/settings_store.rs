// SPDX-License-Identifier: AGPL-3.0-only
//! The shared settings file, read and written off the GTK main thread.
//!
//! Every window reads the same [`Settings`] handle. Changes and re-reads
//! are queued and run one after another on a GIO worker thread, like the
//! Python app's settings queue: the file lock can wait for the Python app,
//! and nothing may block the interface while it does. A change that fails
//! leaves the data as last read, as `Settings`' own change methods do.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use gtk::{gio, glib};
use ox_core::settings::{Settings, SettingsData, SettingsError};

/// A change applied to a copy of the settings on a worker thread.
pub(crate) type Change = Box<dyn FnOnce(&mut Settings) -> Result<(), SettingsError> + Send>;

/// Called on the main thread with the outcome of a [`Change`].
pub(crate) type Reply = Box<dyn FnOnce(Result<(), SettingsError>)>;

/// The settings every window shares, with a queue of pending changes.
pub(crate) struct SettingsStore {
    settings: RefCell<Settings>,
    pending: RefCell<VecDeque<(Change, Reply)>>,
    writing: Cell<bool>,
}

impl std::fmt::Debug for SettingsStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SettingsStore")
            .field("settings", &self.settings)
            .field("pending", &self.pending.borrow().len())
            .field("writing", &self.writing.get())
            .finish()
    }
}

impl SettingsStore {
    /// Shares `settings`, as read at startup.
    pub fn new(settings: Settings) -> Rc<Self> {
        Rc::new(Self {
            settings: RefCell::new(settings),
            pending: RefCell::new(VecDeque::new()),
            writing: Cell::new(false),
        })
    }

    /// A copy of the data as last read or changed.
    pub fn data(&self) -> SettingsData {
        self.settings.borrow().data().clone()
    }

    /// Why the last read fell back to defaults, if it did.
    pub fn warning(&self) -> Option<String> {
        self.settings.borrow().warning().map(str::to_owned)
    }

    /// Queues `change`; `reply` hears its outcome once every earlier change
    /// has finished.
    pub fn change(self: &Rc<Self>, change: Change, reply: Reply) {
        self.pending.borrow_mut().push_back((change, reply));
        self.run_next();
    }

    /// Queues a re-read of the file, for changes the Python app or another
    /// process made. `reply` hears whether the data changed.
    pub fn reload(self: &Rc<Self>, reply: impl FnOnce(bool) + 'static) {
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

    fn run_next(self: &Rc<Self>) {
        if self.writing.get() {
            return;
        }
        let Some((change, reply)) = self.pending.borrow_mut().pop_front() else {
            return;
        };
        self.writing.set(true);
        let mut copy = self.settings.borrow().clone();
        let store = Rc::clone(self);
        glib::spawn_future_local(async move {
            let worker = gio::spawn_blocking(move || {
                let result = change(&mut copy);
                (copy, result)
            });
            let (copy, result) = match worker.await {
                Ok(outcome) => outcome,
                Err(panic) => std::panic::resume_unwind(panic),
            };
            if result.is_ok() {
                store.settings.replace(copy);
            }
            store.writing.set(false);
            reply(result);
            store.run_next();
        });
    }
}
