// SPDX-License-Identifier: AGPL-3.0-only
//! Watching a folder for changes without blocking the interface.
//!
//! Ports `watch` and the directory monitor with its 350 ms debounce in
//! `desktop/winspace.py`. Creating a monitor can block: for a phone, a
//! camera or a network share, `GVfs` answers the request over D-Bus, and a
//! busy backend (an MTP copy in another tab) can take seconds. The Python
//! app therefore creates monitors off the GTK thread, and so does this
//! module.
//!
//! Each [`Watch`] has a monitor thread with a main context of its own. The
//! monitor is created, runs and is dropped on that thread, so it never
//! crosses threads. Its events reach the main context as plain watch ids;
//! the main thread keeps each watch's callback in `SUBSCRIBERS`. Dropping
//! the `Watch` cancels a creation still in progress and ends the thread.

use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::LOG_DOMAIN;

/// Quiet time before a burst of change notifications triggers a refresh.
const CHANGE_DEBOUNCE: Duration = Duration::from_millis(350);

/// Hands out watch ids; an id is never reused in one process.
static NEXT_WATCH_ID: AtomicU64 = AtomicU64::new(0);

/// Identifies a live [`Watch`] across threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct WatchId(u64);

impl WatchId {
    /// An id no other watch of this process has had.
    fn next() -> Self {
        Self(NEXT_WATCH_ID.fetch_add(1, Ordering::Relaxed))
    }
}

/// The change callback of one watch and its pending debounce timer.
struct Subscriber {
    on_change: Rc<dyn Fn()>,
    /// Runs `on_change` once the changes have been quiet for
    /// [`CHANGE_DEBOUNCE`]; restarted by every change.
    debounce_timer: Option<glib::SourceId>,
}

thread_local! {
    /// The callbacks of the live watches, on the main thread. A monitor
    /// thread cannot hold them (they are not `Send`), so it sends only the
    /// watch id and the callback is looked up here.
    static SUBSCRIBERS: RefCell<HashMap<WatchId, Subscriber>> = RefCell::new(HashMap::new());
}

/// True for the monitor events that can change what the folder lists.
fn changes_listing(event: gio::FileMonitorEvent) -> bool {
    matches!(
        event,
        gio::FileMonitorEvent::Created
            | gio::FileMonitorEvent::Deleted
            | gio::FileMonitorEvent::MovedIn
            | gio::FileMonitorEvent::MovedOut
            | gio::FileMonitorEvent::Renamed
            | gio::FileMonitorEvent::ChangesDoneHint
            | gio::FileMonitorEvent::AttributeChanged
    )
}

/// On the main thread: a watched folder changed. Restarts the debounce
/// timer, so a burst of changes refreshes once. A watch dropped after its
/// monitor thread sent the change is no longer registered and is skipped.
fn folder_changed(id: WatchId) {
    let is_watched = SUBSCRIBERS.with_borrow(|subscribers| subscribers.contains_key(&id));
    if !is_watched {
        return;
    }
    let timer = glib::timeout_add_local_once(CHANGE_DEBOUNCE, move || debounce_elapsed(id));
    let previous_timer = SUBSCRIBERS.with_borrow_mut(|subscribers| {
        let subscriber = subscribers.get_mut(&id)?;
        subscriber.debounce_timer.replace(timer)
    });
    if let Some(previous_timer) = previous_timer {
        previous_timer.remove();
    }
}

/// On the main thread: the changes settled. The callback runs outside the
/// registry borrow, because it may start or stop watches.
fn debounce_elapsed(id: WatchId) {
    let on_change = SUBSCRIBERS.with_borrow_mut(|subscribers| {
        let subscriber = subscribers.get_mut(&id)?;
        // The timer has fired; removing it again would be a GLib error.
        subscriber.debounce_timer = None;
        Some(Rc::clone(&subscriber.on_change))
    });
    if let Some(on_change) = on_change {
        on_change();
    }
}

/// Watches one folder for changes. Dropping it stops watching.
///
/// A watch lives on the main thread: dropping it removes its callback
/// from that thread's `SUBSCRIBERS` and its debounce timer from that
/// thread's main context. It is therefore neither `Send` nor `Sync`.
#[derive(Debug)]
pub(crate) struct Watch {
    uri: String,
    id: WatchId,
    /// Cancelled when the watch is dropped, which ends the monitor thread.
    stop: gio::Cancellable,
    /// The monitor thread's own main context.
    monitor_context: glib::MainContext,
    /// Main-thread-only rule: this `Rc` marker keeps the compiler from
    /// letting a watch move to, or be dropped on, another thread, where
    /// its callback would stay registered and keep running.
    _main_thread_only: PhantomData<Rc<()>>,
}

impl Watch {
    /// The folder being watched.
    pub(crate) fn uri(&self) -> &str {
        &self.uri
    }

    /// Tells watches apart, for tests that a watch was kept.
    #[cfg(test)]
    pub(crate) fn id(&self) -> WatchId {
        self.id
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        // Cancelling interrupts a monitor still being created; the wakeup
        // ends the monitor thread's wait for events.
        self.stop.cancel();
        self.monitor_context.wakeup();
        let subscriber = SUBSCRIBERS.with_borrow_mut(|subscribers| subscribers.remove(&self.id));
        if let Some(timer) = subscriber.and_then(|subscriber| subscriber.debounce_timer) {
            timer.remove();
        }
    }
}

/// What a monitor thread needs; everything here may cross threads.
struct MonitorThread {
    uri: String,
    id: WatchId,
    stop: gio::Cancellable,
    /// The thread's own context, which the monitor reports on.
    monitor_context: glib::MainContext,
    /// The GTK thread's context, which runs [`folder_changed`].
    main_context: glib::MainContext,
}

impl MonitorThread {
    /// Monitors until the watch is dropped. A location that cannot be
    /// monitored ends the thread at once; F5 still refreshes it.
    fn run(self) {
        let monitor_context = self.monitor_context.clone();
        // A monitor reports on the thread-default context of the thread that
        // creates it, so this thread's own context must be the default.
        // Only this thread iterates it, so acquiring it should not fail; if
        // it does, the folder is not watched and F5 still refreshes it.
        let watched = monitor_context.with_thread_default(|| self.monitor_until_stopped());
        if let Err(error) = watched {
            glib::g_warning!(LOG_DOMAIN, "Could not watch {} for changes: {error}", self.uri);
        }
    }

    /// Creates the monitor and forwards its changes to the GTK thread
    /// until the watch is dropped.
    fn monitor_until_stopped(&self) {
        let folder = gio::File::for_uri(&self.uri);
        let flags = gio::FileMonitorFlags::WATCH_MOVES;
        let Ok(monitor) = folder.monitor_directory(flags, Some(&self.stop)) else {
            return;
        };
        let id = self.id;
        let main_context = self.main_context.clone();
        monitor.connect_changed(move |_, _, _, event| {
            if changes_listing(event) {
                main_context.invoke(move || folder_changed(id));
            }
        });
        while !self.stop.is_cancelled() {
            self.monitor_context.iteration(true);
        }
        monitor.cancel();
    }
}

/// Calls `on_change` on the main thread after changes in `uri` settle.
///
/// The monitor is created on a thread of its own. Until it exists, and for
/// locations that cannot be monitored at all, changes are not seen.
pub(crate) fn watch_folder(uri: &str, on_change: impl Fn() + 'static) -> Watch {
    let id = WatchId::next();
    let subscriber = Subscriber {
        on_change: Rc::new(on_change),
        debounce_timer: None,
    };
    SUBSCRIBERS.with_borrow_mut(|subscribers| subscribers.insert(id, subscriber));
    let watch = Watch {
        uri: uri.to_owned(),
        id,
        stop: gio::Cancellable::new(),
        monitor_context: glib::MainContext::new(),
        _main_thread_only: PhantomData,
    };
    let thread = MonitorThread {
        uri: watch.uri.clone(),
        id,
        stop: watch.stop.clone(),
        monitor_context: watch.monitor_context.clone(),
        main_context: glib::MainContext::default(),
    };
    let spawned = std::thread::Builder::new()
        .name("folder-watch".to_owned())
        .spawn(move || thread.run());
    if let Err(error) = spawned {
        glib::g_warning!(LOG_DOMAIN, "Could not watch {uri} for changes: {error}");
    }
    watch
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fs;

    use ox_core::location::file_uri;

    use super::*;
    use crate::test_support::harness::{wait_for, wait_until};

    /// The monitor threads of this process that are running now.
    fn monitor_threads() -> usize {
        let tasks = fs::read_dir("/proc/self/task").expect("Linux lists a process's threads");
        let names = tasks
            .filter_map(Result::ok)
            .filter_map(|task| fs::read_to_string(task.path().join("comm")).ok());
        names.filter(|name| name.trim() == "folder-watch").count()
    }

    #[test]
    fn only_changes_to_the_listing_trigger_a_refresh() {
        assert!(changes_listing(gio::FileMonitorEvent::Created));
        assert!(changes_listing(gio::FileMonitorEvent::MovedOut));
        assert!(!changes_listing(gio::FileMonitorEvent::PreUnmount));
        assert!(!changes_listing(gio::FileMonitorEvent::Unmounted));
    }

    /// parity: PERF-003
    #[gtk::test]
    fn a_watch_monitors_on_a_thread_of_its_own_until_dropped() {
        let folder = tempfile::tempdir().expect("the test home has room for a folder");
        let before = monitor_threads();
        let watch = watch_folder(&file_uri(folder.path()), || {});
        wait_until("the monitor thread", || monitor_threads() == before + 1);
        drop(watch);
        wait_until("the monitor thread to end", || monitor_threads() == before);
    }

    /// parity: VIEW-055, PERF-006
    #[gtk::test]
    fn a_burst_of_changes_is_reported_once_it_settles() {
        let folder = tempfile::tempdir().expect("the test home has room for a folder");
        let reports = Rc::new(Cell::new(0));
        let counter = Rc::clone(&reports);
        let _watch = watch_folder(&file_uri(folder.path()), move || counter.set(counter.get() + 1));
        // Give the monitor thread time to start watching before the burst.
        wait_for(Duration::from_millis(200));
        for name in ["one.txt", "two.txt", "three.txt"] {
            fs::write(folder.path().join(name), b"burst").expect("fixture file");
        }
        wait_until("the change report", || reports.get() > 0);
        wait_for(CHANGE_DEBOUNCE * 2);
        assert_eq!(reports.get(), 1, "one report for the whole burst");
    }
}
