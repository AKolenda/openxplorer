// SPDX-License-Identifier: AGPL-3.0-only
//! Scanning a folder with the provider its location needs, blocking or on
//! a worker thread.
//!
//! Ports the start of `scan_folder` in `desktop/folder_sizes.py`, which
//! reads `file:` locations with `LocalSizeProvider` and everything else
//! with `GioSizeProvider`, and the `folderSize` request of
//! `desktop/winspace.py`, which runs the scan on a worker thread with a
//! cancellation the window controls.

use super::{FolderSize, FolderSizeScan, GioSizeProvider, LocalSizeProvider, SizeError};
use crate::location::{normalise, split_location, LocationKind};
use crate::transfer::Cancellation;

/// Totals the folder at `uri` with the default limits: local folders
/// through `lstat` ([`LocalSizeProvider`]), shares and other locations
/// through GIO metadata ([`GioSizeProvider`]). `progress` receives the
/// totals as [`FolderSizeScan::run`] describes. This blocks; see
/// [`scan_folder_size_in_background`].
///
/// # Errors
///
/// As [`FolderSizeScan::run`], and [`SizeError::Read`] when the mount
/// table of this process cannot be read for a local folder.
pub fn scan_folder_size(
    uri: &str,
    cancel: &Cancellation,
    progress: impl FnMut(&FolderSize),
) -> Result<FolderSize, SizeError> {
    let uri = normalise(uri)?;
    let location_kind = split_location(&uri)?.kind();
    if location_kind == LocationKind::Local {
        let provider = LocalSizeProvider::new()?;
        FolderSizeScan::new(&provider).run(&uri, cancel, progress)
    } else {
        FolderSizeScan::new(&GioSizeProvider).run(&uri, cancel, progress)
    }
}

/// [`scan_folder_size`] on a GIO worker thread, so the window can await
/// the totals without blocking.
///
/// Cancelling `cancel` stops the scan between two metadata reads and
/// aborts a GIO read in progress. Once the metadata of the scanned folder
/// itself was read, the scan returns what it counted with
/// [`ScanStatus::Cancelled`](super::ScanStatus::Cancelled); before that it
/// returns `Err(SizeError::Read(EntryError::Cancelled))`. Both are the
/// user's Cancel, not an unavailable size.
///
/// `progress` runs on the worker thread. A window forwards the totals to
/// its main context, for example with `glib::MainContext::invoke` and a
/// `glib::SendWeakRef` to the widget that shows them.
///
/// # Errors
///
/// As [`scan_folder_size`].
///
/// # Panics
///
/// Re-raises a panic of the scan on the worker thread, which is a bug.
pub async fn scan_folder_size_in_background(
    uri: String,
    cancel: Cancellation,
    progress: impl FnMut(&FolderSize) + Send + 'static,
) -> Result<FolderSize, SizeError> {
    let worker = gio::spawn_blocking(move || scan_folder_size(&uri, &cancel, progress));
    match worker.await {
        Ok(outcome) => outcome,
        // A panicking scan is a bug; it surfaces where it is awaited.
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
