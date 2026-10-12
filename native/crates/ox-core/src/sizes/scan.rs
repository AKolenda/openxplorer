// SPDX-License-Identifier: AGPL-3.0-only
//! The bounded, read-only walk that totals a folder's logical size.
//!
//! Ports `scan_folder` in `v2.0.0:desktop/folder_sizes.py`. The walk reads
//! metadata only, never follows a link, and never enters a nested mount,
//! another filesystem or a snapshot collection (PROP-028). Limits and
//! cancellation are checked between metadata reads, so a read that blocks
//! can take longer to return; GIO reads also receive the cancellation.

use std::collections::HashSet;
use std::fmt;
use std::ops::ControlFlow;
use std::time::{Duration, Instant, SystemTime};

use super::{
    FileIdentity, FolderSize, PartialReason, ScanStatus, SizeEntry, SizeEntryKind, SizeError, SizeProvider,
};
use crate::entry::EntryError;
use crate::location::{is_smb_server, normalise};
use crate::transfer::Cancellation;

/// The most items one scan looks at (`MAX_ENTRIES` in Python).
pub const MAX_ENTRIES: u64 = 1_000_000;

/// The longest one scan runs (`MAX_SECONDS` in Python).
pub const MAX_DURATION: Duration = Duration::from_secs(300);

/// The shortest time between two progress reports while scanning.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);

/// Folder names of snapshot collections (`EXCLUDED` in Python). They hold
/// earlier copies of the same files, which would be counted again.
const SNAPSHOT_COLLECTION_NAMES: [&str; 4] = [".zfs", ".snapshot", "#snapshot", ".snapshots"];

/// When a scan stops early and reports partial totals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanLimits {
    max_entries: u64,
    max_duration: Duration,
}

impl ScanLimits {
    /// Stop after `max_entries` items or `max_duration`, whichever comes
    /// first.
    ///
    /// # Errors
    ///
    /// [`SizeError::InvalidLimits`] when either limit is zero.
    pub fn new(max_entries: u64, max_duration: Duration) -> Result<Self, SizeError> {
        if max_entries == 0 || max_duration.is_zero() {
            return Err(SizeError::InvalidLimits);
        }
        Ok(Self {
            max_entries,
            max_duration,
        })
    }
}

impl Default for ScanLimits {
    /// [`MAX_ENTRIES`] items or [`MAX_DURATION`].
    fn default() -> Self {
        Self {
            max_entries: MAX_ENTRIES,
            max_duration: MAX_DURATION,
        }
    }
}

/// A folder-size scan: the provider it reads through, its limits and the
/// clock that times it. It blocks while it runs; see
/// [`scan_folder_size_in_background`](super::scan_folder_size_in_background).
pub struct FolderSizeScan<'a> {
    provider: &'a dyn SizeProvider,
    limits: ScanLimits,
    clock: &'a dyn Fn() -> Instant,
}

impl<'a> FolderSizeScan<'a> {
    /// A scan through `provider` with the default [`ScanLimits`].
    pub fn new(provider: &'a dyn SizeProvider) -> Self {
        Self {
            provider,
            limits: ScanLimits::default(),
            clock: &Instant::now,
        }
    }

    /// This scan with other limits.
    #[must_use]
    pub fn with_limits(self, limits: ScanLimits) -> Self {
        Self { limits, ..self }
    }

    /// This scan timed by a simulated `clock` instead of [`Instant::now`].
    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_clock(self, clock: &'a dyn Fn() -> Instant) -> Self {
        Self { clock, ..self }
    }

    /// Totals the folder at `uri`. `progress` receives the totals when the
    /// scan starts, at most every 200 ms while it runs, and when it ends.
    /// Problems below the folder are counted, not returned. A scan
    /// cancelled after the metadata of the folder itself was read returns
    /// what it counted, with [`ScanStatus::Cancelled`].
    ///
    /// # Errors
    ///
    /// [`SizeError::Location`] for an address the location rules refuse,
    /// [`SizeError::ServerRoot`] for a whole SMB server,
    /// [`SizeError::NotAFolder`] for a file or a link, and
    /// [`SizeError::Read`] when the folder itself cannot be read. A scan
    /// cancelled before or while the folder's metadata is read returns
    /// [`SizeError::Read`] with [`EntryError::Cancelled`].
    pub fn run(
        &self,
        uri: &str,
        cancel: &Cancellation,
        mut progress: impl FnMut(&FolderSize),
    ) -> Result<FolderSize, SizeError> {
        let uri = normalise(uri)?;
        if is_smb_server(&uri) {
            return Err(SizeError::ServerRoot);
        }
        // Errors on the folder itself reach the caller, which mounts an
        // unmounted share and scans again (PROP-029).
        let root = self.provider.inspect(&uri, cancel)?;
        if root.kind != SizeEntryKind::Folder {
            return Err(SizeError::NotAFolder);
        }
        let mut walk = Walk::new(self, uri, root.filesystem, cancel, &mut progress);
        walk.publish(Publish::Always);
        walk.measure_pending_folders()?;
        Ok(walk.finish())
    }
}

/// Written by hand because the clock is a closure.
impl fmt::Debug for FolderSizeScan<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FolderSizeScan")
            .field("provider", &self.provider)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

/// Whether a progress report waits for [`PROGRESS_INTERVAL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Publish {
    Always,
    WhenDue,
}

/// The state of one running scan.
struct Walk<'w> {
    provider: &'w dyn SizeProvider,
    limits: ScanLimits,
    clock: &'w dyn Fn() -> Instant,
    cancel: &'w Cancellation,
    progress: &'w mut dyn FnMut(&FolderSize),
    /// The totals so far.
    size: FolderSize,
    /// The filesystem of the scanned folder; subfolders on another one are
    /// skipped.
    root_filesystem: Option<String>,
    /// Folders still to measure, as a stack.
    pending: Vec<String>,
    /// Folders already measured.
    visited: HashSet<String>,
    /// Files already counted, so a hard-linked file counts once.
    counted_files: HashSet<FileIdentity>,
    started: Instant,
    last_published: Instant,
}

impl<'w> Walk<'w> {
    fn new(
        scan: &FolderSizeScan<'w>,
        uri: String,
        root_filesystem: Option<String>,
        cancel: &'w Cancellation,
        progress: &'w mut dyn FnMut(&FolderSize),
    ) -> Self {
        let started = (scan.clock)();
        Self {
            provider: scan.provider,
            limits: scan.limits,
            clock: scan.clock,
            cancel,
            progress,
            pending: vec![uri.clone()],
            size: FolderSize::new(uri),
            root_filesystem,
            visited: HashSet::new(),
            counted_files: HashSet::new(),
            started,
            last_published: started,
        }
    }

    /// How long the scan has run.
    fn elapsed(&self) -> Duration {
        (self.clock)().saturating_duration_since(self.started)
    }

    /// Measures folders from the stack until it is empty or the scan
    /// stops.
    fn measure_pending_folders(&mut self) -> Result<(), SizeError> {
        while self.size.status == ScanStatus::Scanning {
            let Some(folder) = self.pending.pop() else {
                break;
            };
            if let Some(stopped) = self.stop_between_folders() {
                self.size.status = stopped;
                break;
            }
            self.measure_folder(&folder)?;
            self.publish(Publish::WhenDue);
        }
        Ok(())
    }

    /// The status to stop with before the next folder: cancelled, or out
    /// of time.
    fn stop_between_folders(&self) -> Option<ScanStatus> {
        if self.cancel.is_cancelled() {
            return Some(ScanStatus::Cancelled);
        }
        if self.elapsed() >= self.limits.max_duration {
            return Some(ScanStatus::Partial(PartialReason::TimeLimitReached));
        }
        None
    }

    /// Counts the items of `folder` and stacks its subfolders.
    fn measure_folder(&mut self, folder: &str) -> Result<(), SizeError> {
        if !self.visited.insert(folder.to_owned()) {
            self.size.skipped += 1;
            return Ok(());
        }
        let provider = self.provider;
        let cancel = self.cancel;
        let listed = provider.visit_children(folder, cancel, &mut |entry| self.count(entry));
        match listed {
            Ok(()) => Ok(()),
            Err(error) => self.record_listing_error(folder, error),
        }
    }

    /// Handles a folder that could not be listed, or not to the end.
    fn record_listing_error(&mut self, folder: &str, error: EntryError) -> Result<(), SizeError> {
        if self.cancel.is_cancelled() {
            self.size.status = ScanStatus::Cancelled;
            return Ok(());
        }
        // A share may need mounting at its first listing rather than at
        // its first query, so an error on the scanned folder before
        // anything was counted reaches the caller's mount-and-retry.
        if folder == self.size.uri && self.size.entries == 0 {
            return Err(SizeError::Read(error));
        }
        // Safety rule PROP-028 ("failed subtrees are not presented as
        // empty directories" in folder_sizes.py): a subfolder that could
        // not be read is counted as an error.
        self.size.errors += 1;
        Ok(())
    }

    /// Counts one item, unless the scan must stop first.
    fn count(&mut self, entry: SizeEntry) -> ControlFlow<()> {
        if self.cancel.is_cancelled() {
            self.size.status = ScanStatus::Cancelled;
            return ControlFlow::Break(());
        }
        let is_out_of_time = self.elapsed() >= self.limits.max_duration;
        if self.size.entries >= self.limits.max_entries || is_out_of_time {
            self.size.status = ScanStatus::Partial(PartialReason::ScanLimitReached);
            return ControlFlow::Break(());
        }
        self.size.entries += 1;
        self.add(entry);
        self.publish(Publish::WhenDue);
        ControlFlow::Continue(())
    }

    /// Adds one item to the totals, or to what was left out.
    fn add(&mut self, entry: SizeEntry) {
        match entry.kind {
            SizeEntryKind::Unreadable => self.size.errors += 1,
            SizeEntryKind::Folder if self.is_excluded_folder(&entry) => self.size.skipped += 1,
            SizeEntryKind::Folder => {
                self.size.folders += 1;
                self.pending.push(entry.uri);
            }
            SizeEntryKind::File => self.add_file(&entry),
            // Safety rule PROP-028: links are never followed and special
            // files never opened.
            SizeEntryKind::Symlink | SizeEntryKind::Other => self.size.skipped += 1,
        }
    }

    /// True for a subfolder the scan must not enter.
    ///
    /// Safety rule PROP-028: a snapshot collection would count earlier
    /// copies of the same files, and a nested mount or another filesystem
    /// is not part of this folder's storage. A snapshot folder chosen as
    /// the scanned folder is still measured: only subfolders are checked.
    fn is_excluded_folder(&self, entry: &SizeEntry) -> bool {
        let is_snapshot_collection = SNAPSHOT_COLLECTION_NAMES.contains(&entry.name.as_str());
        let is_on_other_filesystem = match (&self.root_filesystem, &entry.filesystem) {
            (Some(root), Some(filesystem)) => root != filesystem,
            _ => false,
        };
        is_snapshot_collection || entry.is_mount_point || is_on_other_filesystem
    }

    /// Adds a regular file to the totals.
    fn add_file(&mut self, entry: &SizeEntry) {
        // Safety rule PROP-028: an unknown size is never counted as 0.
        let Some(bytes) = entry.size else {
            self.size.skipped += 1;
            return;
        };
        // A hard-linked file is counted once, however many names it has.
        if let Some(identity) = entry.identity {
            if !self.counted_files.insert(identity) {
                return;
            }
        }
        self.size.bytes = self.size.bytes.saturating_add(bytes);
        self.size.allocated = self
            .size
            .allocated
            .zip(entry.allocated)
            .map(|(total, allocated)| total.saturating_add(allocated));
        self.size.files += 1;
    }

    /// Reports the totals to `progress`.
    fn publish(&mut self, when: Publish) {
        let now = (self.clock)();
        let since_last = now.saturating_duration_since(self.last_published);
        if when == Publish::WhenDue && since_last < PROGRESS_INTERVAL {
            return;
        }
        self.last_published = now;
        self.size.elapsed = now.saturating_duration_since(self.started);
        (self.progress)(&self.size);
    }

    /// Settles the status, stamps the end and reports the final totals.
    fn finish(mut self) -> FolderSize {
        if self.size.status == ScanStatus::Scanning {
            // Safety rule PROP-028: a total that left anything out says so.
            let left_out = self.size.errors > 0 || self.size.skipped > 0;
            self.size.status = if left_out {
                ScanStatus::Partial(PartialReason::EntriesExcluded)
            } else {
                ScanStatus::Complete
            };
        }
        self.size.finished_at = Some(SystemTime::now());
        self.publish(Publish::Always);
        self.size
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::location::file_uri;
    use crate::sizes::LocalSizeProvider;

    /// Ported from `v2.0.0:desktop/tests/test_v06.py::FolderSizeTests::test_max_seconds`:
    /// a clock that advances one second each time it is read.
    ///
    /// parity: PROP-029
    #[test]
    fn the_time_limit_marks_the_result_partial() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("a"), b"abc").unwrap();
        let start = Instant::now();
        let seconds_read = Cell::new(0);
        let clock = || {
            seconds_read.set(seconds_read.get() + 1);
            start + Duration::from_secs(seconds_read.get())
        };
        let provider = LocalSizeProvider::with_mount_points([]);
        let limits = ScanLimits::new(MAX_ENTRIES, Duration::from_secs(1)).unwrap();

        let size = FolderSizeScan::new(&provider)
            .with_limits(limits)
            .with_clock(&clock)
            .run(&file_uri(folder.path()), &Cancellation::new(), |_| {})
            .unwrap();

        assert_eq!(size.status, ScanStatus::Partial(PartialReason::TimeLimitReached));
        assert_eq!(size.status.reason(), "Time limit reached");
    }

    /// Progress is reported at the start and the end, and in between at
    /// most once per [`PROGRESS_INTERVAL`] of the clock.
    ///
    /// parity: PROP-026, PERF-006
    #[test]
    fn progress_between_start_and_end_waits_for_the_interval() {
        let folder = tempfile::tempdir().unwrap();
        for name in ["a", "b", "c"] {
            std::fs::write(folder.path().join(name), b"x").unwrap();
        }
        let start = Instant::now();
        let clock = || start;
        let provider = LocalSizeProvider::with_mount_points([]);
        let mut reports = 0;

        FolderSizeScan::new(&provider)
            .with_clock(&clock)
            .run(&file_uri(folder.path()), &Cancellation::new(), |_| reports += 1)
            .unwrap();

        assert_eq!(reports, 2);
    }
}
