// SPDX-License-Identifier: AGPL-3.0-only
//! Folder listing and change monitoring.
//!
//! Ports `enumerate_folder` in `desktop/gio_backend.py` and the directory
//! monitor with its 350 ms debounce in `desktop/winspace.py`. Listing uses
//! GIO's asynchronous enumerator, so the blocking I/O runs on GIO's worker
//! threads and rows arrive in batches: the first batch at once, later ones
//! merged for a quarter second so large folders are not re-sorted per row.
//! Dropping a [`Listing`] cancels it (the GIO futures cancel their
//! `GCancellable` when dropped), and dropping a [`Watch`] stops monitoring.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::{self, Entry};

/// Rows in the first batch, shown before the rest of the folder is read.
const FIRST_BATCH: i32 = 64;
/// Rows requested per enumerator call after the first.
const LATER_BATCH: i32 = 512;
/// How long later batches are merged before they are shown.
const MERGE_WINDOW: Duration = Duration::from_millis(250);
/// Quiet time before a burst of change notifications triggers a refresh.
const CHANGE_DEBOUNCE: Duration = Duration::from_millis(350);

/// Why a listing failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError {
    /// User-facing message from GIO.
    pub message: String,
    /// The location is a file, not a folder.
    pub not_directory: bool,
}

impl LoadError {
    fn from_glib(error: &glib::Error) -> Self {
        Self {
            message: error.message().to_string(),
            not_directory: error.matches(gio::IOErrorEnum::NotDirectory),
        }
    }
}

/// A running listing. Dropping it cancels the listing.
pub struct Listing {
    handle: Option<glib::JoinHandle<()>>,
}

impl Drop for Listing {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

/// Lists `uri`, calling `on_batch` with rows as they arrive and `on_done`
/// once at the end. Neither is called after the [`Listing`] is dropped.
pub fn list_folder(
    uri: &str,
    on_batch: impl Fn(Vec<Entry>) + 'static,
    on_done: impl FnOnce(Result<(), LoadError>) + 'static,
) -> Listing {
    let folder = gio::File::for_uri(uri);
    let handle = glib::spawn_future_local(async move {
        let result = enumerate(&folder, &on_batch).await;
        on_done(result.map_err(|error| LoadError::from_glib(&error)));
    });
    Listing { handle: Some(handle) }
}

async fn enumerate(folder: &gio::File, on_batch: &impl Fn(Vec<Entry>)) -> Result<(), glib::Error> {
    let enumerator = folder
        .enumerate_children_future(
            entry::ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await?;
    let mut pending: Vec<Entry> = Vec::new();
    let mut last_flush = Instant::now();
    let mut request = FIRST_BATCH;
    loop {
        let infos = enumerator
            .next_files_future(request, glib::Priority::DEFAULT)
            .await?;
        if infos.is_empty() {
            break;
        }
        let first = request == FIRST_BATCH;
        request = LATER_BATCH;
        pending.extend(infos.iter().map(|info| {
            let child = enumerator.child(info);
            entry::entry_from_info(&child, info)
        }));
        if first || last_flush.elapsed() >= MERGE_WINDOW {
            on_batch(std::mem::take(&mut pending));
            last_flush = Instant::now();
        }
    }
    if !pending.is_empty() {
        on_batch(pending);
    }
    // Closing is best effort; the enumerator is dropped either way.
    let _ = enumerator.close_future(glib::Priority::LOW).await;
    Ok(())
}

/// Watches a folder for changes. Dropping it stops watching.
pub struct Watch {
    monitor: gio::FileMonitor,
    pending: Rc<RefCell<Option<glib::SourceId>>>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.monitor.cancel();
        if let Some(source) = self.pending.borrow_mut().take() {
            source.remove();
        }
    }
}

/// Calls `on_change` after changes in `uri` settle. `None` when the
/// location cannot be monitored (F5 still refreshes it).
pub fn watch_folder(uri: &str, on_change: impl Fn() + 'static) -> Option<Watch> {
    let folder = gio::File::for_uri(uri);
    let monitor = folder
        .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, None::<&gio::Cancellable>)
        .ok()?;
    let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let on_change = Rc::new(on_change);
    let timer = Rc::clone(&pending);
    monitor.connect_changed(move |_, _, _, event| {
        let relevant = matches!(
            event,
            gio::FileMonitorEvent::Created
                | gio::FileMonitorEvent::Deleted
                | gio::FileMonitorEvent::MovedIn
                | gio::FileMonitorEvent::MovedOut
                | gio::FileMonitorEvent::Renamed
                | gio::FileMonitorEvent::ChangesDoneHint
                | gio::FileMonitorEvent::AttributeChanged
        );
        if !relevant {
            return;
        }
        if let Some(source) = timer.borrow_mut().take() {
            source.remove();
        }
        let callback = Rc::clone(&on_change);
        let slot = Rc::clone(&timer);
        let source = glib::timeout_add_local_once(CHANGE_DEBOUNCE, move || {
            slot.borrow_mut().take();
            callback();
        });
        timer.borrow_mut().replace(source);
    });
    Some(Watch { monitor, pending })
}
