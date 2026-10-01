// SPDX-License-Identifier: AGPL-3.0-only
//! The app's search cache: the index service every window shares, the
//! cache status as last read, and the operations the windows and Settings
//! ask for.
//!
//! Ports what `v2.0.0:desktop/winspace.py` did with its `IndexService` (the
//! `cache*` and `search` bridge operations and the `cacheChanged` event)
//! and `refreshCacheStatus` in `v2.0.0:desktop/ui/app.js`. [`SearchCache`] is a
//! `GObject` owned by the app context. Every operation blocks on SQLite, so
//! it runs on a GIO worker thread and the caller awaits it; the main
//! thread never waits for the cache. The status is read again 200 ms
//! after the service reports a change (SRCH-018), after every operation
//! that changes the cache, and every five seconds while a window is
//! shown, for changes another process's index owner made. The operations
//! themselves are in [`operations`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::search::{CacheStatus, IndexRoot, IndexSettings};
use ox_core::settings::Bookmark;
use ox_core::LOG_DOMAIN;

mod operations;

use super::error::CacheError;
use super::indexer::{Indexer, IndexerStart};
use super::pin_sync::{PinChanges, SettingsReading};

/// Emitted when the cache status was read again.
const STATUS_CHANGED: &str = "status-changed";
/// Emitted when the cached names may have changed, so a search shown now
/// may be out of date.
const CONTENTS_CHANGED: &str = "contents-changed";

/// How long a burst of change reports is gathered before the status is
/// read (`cacheTimer` in app.js).
const CHANGE_SETTLE: Duration = Duration::from_millis(200);
/// How often the status is read while a window is shown (`setInterval` of
/// `refreshCacheStatus` in app.js).
const STATUS_POLL: Duration = Duration::from_secs(5);

/// Why the status is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatusReading {
    /// To show the latest counts and states.
    Refresh,
    /// The cached names changed too, so shown searches run again.
    AfterChange,
}

mod imp {
    use std::cell::{Cell, RefCell};
    use std::sync::OnceLock;

    use gtk::glib;
    use gtk::glib::subclass::Signal;
    use gtk::subclass::prelude::*;
    use ox_core::search::CacheStatus;

    use super::super::indexer::Indexer;
    use super::super::pin_sync::PinnedFolders;
    use super::{CONTENTS_CHANGED, STATUS_CHANGED};

    /// Private state of [`super::SearchCache`].
    #[derive(Debug, Default)]
    pub(crate) struct SearchCache {
        /// The running index service; `None` until started.
        pub(super) indexer: RefCell<Option<Indexer>>,
        /// The status as last read; `None` until first read.
        pub(super) status: RefCell<Option<CacheStatus>>,
        /// The number of the latest status reading started; an older
        /// reading that finishes later is dropped.
        pub(super) latest_reading: Cell<u64>,
        /// Set while a reading after a change has not been shown yet; the
        /// reading that is shown announces the change, even when a newer
        /// reading replaced the one asked after the change.
        pub(super) has_unannounced_change: Cell<bool>,
        /// The pending read after a burst of changes.
        pub(super) change_timer: RefCell<Option<glib::SourceId>>,
        /// The five-second status poll.
        pub(super) poll_timer: RefCell<Option<glib::SourceId>>,
        /// The Quick access pins as last read (SRCH-040).
        pub(super) pinned: RefCell<PinnedFolders>,
        /// The folders the service read again after the app wrote into
        /// them, for tests (SRCH-033).
        #[cfg(test)]
        pub(super) written: RefCell<Vec<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SearchCache {
        const NAME: &'static str = "OxSearchCache";
        type Type = super::SearchCache;
    }

    impl ObjectImpl for SearchCache {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder(STATUS_CHANGED).build(),
                    Signal::builder(CONTENTS_CHANGED).build(),
                ]
            })
        }

        fn dispose(&self) {
            self.obj().shut_down();
        }
    }
}

glib::wrapper! {
    /// The search cache every window of the app shares.
    pub(crate) struct SearchCache(ObjectSubclass<imp::SearchCache>);
}

impl Default for SearchCache {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl SearchCache {
    /// Starts the index service as `start` says. Until it is started, every
    /// operation fails with [`CacheError::NotStarted`] and no folder counts
    /// as indexed.
    pub(crate) fn start(&self, start: IndexerStart) {
        let imp = self.imp();
        if imp.indexer.borrow().is_some() {
            return;
        }
        imp.pinned
            .borrow_mut()
            .update(&start.pins, SettingsReading::Sound);
        match Indexer::start(start, self.change_listener()) {
            Ok(indexer) => {
                imp.indexer.replace(Some(indexer));
                // The first reading tells the windows what the cache holds,
                // so a search typed before it ran runs again with it.
                self.read_status(StatusReading::AfterChange);
                self.poll_while_shown();
            }
            Err(error) => glib::g_warning!(LOG_DOMAIN, "The search cache could not start: {error}"),
        }
    }

    /// Stops the index service at once, as the app quits (`close` in
    /// Python): scans stop, watches end, and ownership passes to another
    /// process once the running jobs notice. Operations then fail with
    /// [`CacheError::NotStarted`].
    pub(crate) fn shut_down(&self) {
        let imp = self.imp();
        let timers = [imp.change_timer.take(), imp.poll_timer.take()];
        for timer in timers.into_iter().flatten() {
            timer.remove();
        }
        // Dropping the indexer closes the service and ends its thread.
        imp.indexer.take();
    }

    /// The listener the service calls, on a worker thread, whenever the
    /// cache may have changed. It passes the news to the main loop once
    /// per burst.
    fn change_listener(&self) -> impl Fn() + Send + Sync + 'static {
        let cache = glib::SendWeakRef::from(self.downgrade());
        let is_reported = Arc::new(AtomicBool::new(false));
        move || {
            if is_reported.swap(true, Ordering::AcqRel) {
                return;
            }
            let cache = cache.clone();
            let is_reported = Arc::clone(&is_reported);
            glib::MainContext::default().invoke(move || {
                is_reported.store(false, Ordering::Release);
                if let Some(cache) = cache.upgrade() {
                    cache.schedule_change_reading();
                }
            });
        }
    }

    /// Reads the status once the current burst of changes has settled.
    fn schedule_change_reading(&self) {
        let imp = self.imp();
        if let Some(pending) = imp.change_timer.take() {
            pending.remove();
        }
        let read = glib::clone!(
            #[weak(rename_to = cache)]
            self,
            move || {
                cache.imp().change_timer.take();
                cache.read_status(StatusReading::AfterChange);
            }
        );
        let timer = glib::timeout_add_local_once(CHANGE_SETTLE, read);
        imp.change_timer.replace(Some(timer));
    }

    /// Reads the status every five seconds while a window of the app is
    /// shown (PERF-005: polling pauses while every window is hidden).
    fn poll_while_shown(&self) {
        let poll = glib::clone!(
            #[weak(rename_to = cache)]
            self,
            #[upgrade_or]
            glib::ControlFlow::Break,
            move || {
                if is_any_window_shown() {
                    cache.read_status(StatusReading::Refresh);
                }
                glib::ControlFlow::Continue
            }
        );
        let timer = glib::timeout_add_local(STATUS_POLL, poll);
        self.imp().poll_timer.replace(Some(timer));
    }

    /// Reads the status again now, as a window does when it gains focus.
    pub(crate) fn refresh_status(&self) {
        self.read_status(StatusReading::Refresh);
    }

    /// Reads the status off the main thread, keeps it and tells the
    /// windows; `reading` says whether shown searches run again.
    fn read_status(&self, reading: StatusReading) {
        let imp = self.imp();
        let number = imp.latest_reading.get() + 1;
        imp.latest_reading.set(number);
        if reading == StatusReading::AfterChange {
            imp.has_unannounced_change.set(true);
        }
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = cache)]
            self,
            async move {
                let status = cache.run(|service| service.index().status()).await;
                if cache.imp().latest_reading.get() != number {
                    return;
                }
                match status {
                    Ok(status) => cache.show_status(status),
                    Err(CacheError::NotStarted) => {}
                    Err(error) => glib::g_warning!(LOG_DOMAIN, "Could not read the search cache: {error}"),
                }
            }
        ));
    }

    /// Keeps `status` and tells the windows, and whether the cached names
    /// changed since the status shown before.
    fn show_status(&self, status: CacheStatus) {
        let imp = self.imp();
        imp.status.replace(Some(status));
        self.emit_by_name::<()>(STATUS_CHANGED, &[]);
        if imp.has_unannounced_change.replace(false) {
            self.emit_by_name::<()>(CONTENTS_CHANGED, &[]);
        }
    }

    /// The status as last read; `None` until the cache has started.
    pub(crate) fn status(&self) -> Option<CacheStatus> {
        self.imp().status.borrow().clone()
    }

    /// The indexed folders as last read; none until the cache has started.
    pub(crate) fn roots(&self) -> Vec<IndexRoot> {
        let status = self.imp().status.borrow();
        status
            .as_ref()
            .map(|status| status.roots.clone())
            .unwrap_or_default()
    }

    /// Calls `callback` whenever the status was read again.
    pub(crate) fn connect_status_changed(&self, callback: impl Fn() + 'static) -> glib::SignalHandlerId {
        self.connect_local(STATUS_CHANGED, false, move |_| {
            callback();
            None
        })
    }

    /// Calls `callback` whenever the cached names may have changed.
    pub(crate) fn connect_contents_changed(&self, callback: impl Fn() + 'static) -> glib::SignalHandlerId {
        self.connect_local(CONTENTS_CHANGED, false, move |_| {
            callback();
            None
        })
    }

    /// Applies the user's Search & indexing options and tells the service
    /// about changed Quick access pins (SRCH-040).
    pub(crate) fn follow_settings(
        &self,
        settings: IndexSettings,
        pins: &[Bookmark],
        reading: SettingsReading,
    ) {
        if let Some(indexer) = self.imp().indexer.borrow().as_ref() {
            indexer.set_settings(settings);
        }
        let changes = self.imp().pinned.borrow_mut().update(pins, reading);
        if !changes.is_empty() {
            self.index_pin_changes(changes);
        }
    }

    /// Tells the service about pins that appeared and disappeared.
    fn index_pin_changes(&self, changes: PinChanges) {
        self.change_in_background("index the pinned folders", move |service| {
            let indexing = service.index().pin_indexing()?;
            for pin in &changes.added {
                service.pin_added(pin, indexing)?;
            }
            for uri in &changes.removed {
                service.pin_removed(uri)?;
            }
            Ok(())
        });
    }
}

/// Whether a window of the app is on screen, not minimised or hidden.
fn is_any_window_shown() -> bool {
    let windows = gtk::Window::list_toplevels();
    windows
        .iter()
        .filter_map(|window| window.downcast_ref::<gtk::Window>())
        .any(|window| window.is_visible() && !window.is_suspended())
}
