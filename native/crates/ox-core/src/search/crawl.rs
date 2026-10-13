// SPDX-License-Identifier: AGPL-3.0-only
//! A full scan of one root.
//!
//! Ports `_run` in `v2.0.0:desktop/index_service.py` (SRCH-024, SRCH-031,
//! SRCH-032). The scan walks the root depth first, stores what it reads
//! under a new generation and, only when every folder was read, prunes
//! what it did not see.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use super::error::{check_cancelled, SearchError};
use super::policy::{IndexScope, RootStorage};
use super::root::{IndexRoot, RootOrigin, ScanGeneration};
use super::scan::{ListedItem, ScanOutcome};
use super::state::Shared;
use super::text::unix_now;

/// Deepest folder level a scan descends to below its root.
const MAX_DEPTH: usize = 128;

/// Reported when folders deeper than [`MAX_DEPTH`] were skipped.
const TOO_DEEP_MESSAGE: &str = "Some directories exceed the 128-level traversal limit.";

/// Put before the first error of a scan that could not read everything.
const UNREADABLE_PREFIX: &str = "Some folders could not be read. ";

/// How often a running scan reports its progress.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(800);

/// A queued full scan.
#[derive(Debug)]
pub(super) struct ScanJob {
    pub(super) root: IndexRoot,
    pub(super) cancellable: gio::Cancellable,
}

/// Runs `job`, records its outcome and ends its bookkeeping.
pub(super) fn run_scan(shared: &Shared, job: &ScanJob) {
    let root = &job.root.uri;
    let storage = RootStorage::current(root);
    scan_and_record(shared, job, storage);
    shared.report_monitoring(root, storage);
    shared.finish_scan_job(root, &job.cancellable);
}

/// Scans the root and stores how the scan ended.
fn scan_and_record(shared: &Shared, job: &ScanJob, storage: RootStorage) {
    let root = &job.root.uri;
    // A root disabled or removed since the scan was queued is not scanned,
    // and nothing is recorded, as in Python.
    let Ok(generation) = shared.index.begin_scan(root) else {
        return;
    };
    shared.forget_watches(root);
    shared.notify();
    let outcome = match walk_root(shared, job, &generation, storage) {
        Ok(outcome) => outcome,
        Err(SearchError::FrequentFolderTooLarge) if skip_frequent_root(shared, root) => return,
        Err(error) => ScanOutcome::Incomplete {
            error: error.to_string(),
        },
    };
    if let Err(error) = shared.index.finish_scan(root, &generation, &outcome) {
        // Python reports a failure to record the outcome as the outcome. If
        // that fails too, the root stays "Indexing" until the next owner
        // marks it interrupted.
        let failure = ScanOutcome::Incomplete {
            error: error.to_string(),
        };
        let _ = shared.index.finish_scan(root, &generation, &failure);
    }
}

/// Removes the root that frequent use added and that turned out too large,
/// and remembers to skip it for a while (see [`super::frequent`]). Returns
/// `false`, changing nothing, when the root is no longer frequent use's,
/// for example because the user chose it meanwhile.
fn skip_frequent_root(shared: &Shared, root: &str) -> bool {
    match shared.index.skip_frequent_root(root, unix_now()) {
        Ok(true) => {
            shared.forget_watches(root);
            true
        }
        Ok(false) | Err(_) => false,
    }
}

/// Reads every folder of the root that its scope admits and says how the
/// scan ended.
fn walk_root(
    shared: &Shared,
    job: &ScanJob,
    generation: &ScanGeneration,
    storage: RootStorage,
) -> Result<ScanOutcome, SearchError> {
    let scope = shared.scope_of(&job.root.uri)?;
    let limit = EntryLimit::of(shared, &job.root)?;
    let mut crawl = Crawl::new(shared, job, generation, scope, storage, limit);
    crawl.run()?;
    Ok(crawl.outcome())
}

/// The most entries one scan may store, and the error it stops with when
/// there are more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EntryLimit {
    entries: usize,
    is_frequent: bool,
}

impl EntryLimit {
    /// The limit of a scan of `root`: [`ServiceLimits::entries_per_root`],
    /// and for a root that frequent use added, also
    /// [`ServiceLimits::entries_per_frequent_root`] and what is left of
    /// [`ServiceLimits::frequent_entries`] beside its other roots.
    ///
    /// [`ServiceLimits::entries_per_root`]: super::limits::ServiceLimits::entries_per_root
    /// [`ServiceLimits::entries_per_frequent_root`]: super::limits::ServiceLimits::entries_per_frequent_root
    /// [`ServiceLimits::frequent_entries`]: super::limits::ServiceLimits::frequent_entries
    fn of(shared: &Shared, root: &IndexRoot) -> Result<Self, SearchError> {
        let limits = &shared.limits;
        if root.origin != RootOrigin::Frequent {
            return Ok(Self {
                entries: limits.entries_per_root,
                is_frequent: false,
            });
        }
        let others: u64 = shared
            .index
            .roots()?
            .iter()
            .filter(|other| other.origin == RootOrigin::Frequent && other.uri != root.uri)
            .map(|other| other.entry_count)
            .sum();
        let others = usize::try_from(others).unwrap_or(usize::MAX);
        let entries = limits
            .entries_per_root
            .min(limits.entries_per_frequent_root)
            .min(limits.frequent_entries.saturating_sub(others));
        Ok(Self {
            entries,
            is_frequent: true,
        })
    }

    /// The error a scan that reached the limit stops with.
    fn error(self) -> SearchError {
        if self.is_frequent {
            SearchError::FrequentFolderTooLarge
        } else {
            SearchError::EntryLimit
        }
    }
}

/// A folder waiting to be read, with its depth below the root.
#[derive(Debug)]
struct PendingFolder {
    uri: String,
    depth: usize,
}

/// One running scan.
struct Crawl<'a> {
    shared: &'a Shared,
    job: &'a ScanJob,
    generation: &'a ScanGeneration,
    scope: IndexScope,
    storage: RootStorage,
    /// The most entries the scan may store.
    limit: EntryLimit,
    /// Entries stored so far.
    stored: usize,
    /// Why folders could not be read; the scan goes on without them.
    errors: Vec<String>,
    /// Folders read or skipped already.
    visited: HashSet<String>,
    /// Folders still to read; the last is read next, so the walk is depth
    /// first, as in Python.
    pending: Vec<PendingFolder>,
    /// Spaces the progress reports.
    progress: ProgressPacer,
}

impl<'a> Crawl<'a> {
    /// A scan of `job`'s root within `scope` that stores under
    /// `generation`.
    fn new(
        shared: &'a Shared,
        job: &'a ScanJob,
        generation: &'a ScanGeneration,
        scope: IndexScope,
        storage: RootStorage,
        limit: EntryLimit,
    ) -> Self {
        Self {
            shared,
            job,
            generation,
            scope,
            storage,
            limit,
            stored: 0,
            errors: Vec::new(),
            visited: HashSet::new(),
            pending: Vec::new(),
            progress: ProgressPacer::new(Instant::now()),
        }
    }
}

impl Crawl<'_> {
    /// Reads every folder of the root. Unreadable folders are recorded
    /// and skipped, except the root itself: without it nothing is known.
    /// A root that frequent use added and that holds too many entries ends
    /// the scan with [`SearchError::FrequentFolderTooLarge`].
    fn run(&mut self) -> Result<(), SearchError> {
        let root = self.job.root.uri.clone();
        self.pending.push(PendingFolder {
            uri: root.clone(),
            depth: 0,
        });
        while let Some(folder) = self.pending.pop() {
            check_cancelled(&self.job.cancellable)?;
            if !self.visited.insert(folder.uri.clone()) {
                continue;
            }
            if folder.depth > MAX_DEPTH {
                self.errors.push(TOO_DEEP_MESSAGE.to_owned());
                continue;
            }
            self.shared.watch_folder(&root, &folder.uri, self.storage);
            let Err(error) = self.read_folder(&folder) else {
                continue;
            };
            // A read that failed because of the cancellation reports the
            // cancellation.
            check_cancelled(&self.job.cancellable)?;
            if matches!(error, SearchError::FrequentFolderTooLarge) {
                return Err(error);
            }
            self.errors.push(error.to_string());
            if folder.uri == root || self.stored >= self.limit.entries {
                break;
            }
        }
        check_cancelled(&self.job.cancellable)
    }

    /// How the scan ended once every folder was tried.
    fn outcome(&self) -> ScanOutcome {
        match self.errors.first() {
            None => ScanOutcome::Complete,
            Some(first) => ScanOutcome::Incomplete {
                error: format!("{UNREADABLE_PREFIX}{first}"),
            },
        }
    }

    /// Reads one folder and stores its items.
    fn read_folder(&mut self, folder: &PendingFolder) -> Result<(), SearchError> {
        let shared = self.shared;
        let job = self.job;
        let child_depth = folder.depth + 1;
        let mut receive = |batch| self.store_batch(batch, child_depth);
        shared
            .reader
            .read_folder(&folder.uri, job.root.hidden_items, &job.cancellable, &mut receive)
    }

    /// Stores one batch and queues its folders.
    ///
    /// Safety rule "at most a million entries per root" (SRCH-032,
    /// `ServiceLimits::entries_per_root`): what does not fit is not
    /// stored, and the scan stops with the limit message. A root that
    /// frequent use added has the lower limit of [`EntryLimit::of`].
    fn store_batch(&mut self, batch: Vec<ListedItem>, child_depth: usize) -> Result<(), SearchError> {
        check_cancelled(&self.job.cancellable)?;
        let admitted = self.scope.admit(batch)?;
        let limit = self.limit.entries;
        let room = limit.saturating_sub(self.stored);
        let fitting = &admitted[..admitted.len().min(room)];
        self.stored += self
            .shared
            .index
            .store_scanned(&self.job.root.uri, self.generation, fitting)?;
        self.queue_folders(&admitted, child_depth);
        if admitted.len() > room {
            return Err(self.limit.error());
        }
        self.report_progress();
        Ok(())
    }

    /// Queues the unvisited folders among `items`.
    fn queue_folders(&mut self, items: &[ListedItem], depth: usize) {
        let folders = items
            .iter()
            .filter(|item| item.is_dir && !self.visited.contains(&item.uri))
            .map(|item| PendingFolder {
                uri: item.uri.clone(),
                depth,
            });
        self.pending.extend(folders);
    }

    /// Records the watch status and notifies the app, at most every 0.8 s.
    fn report_progress(&mut self) {
        if self.progress.is_due(Instant::now()) {
            self.shared.report_monitoring(&self.job.root.uri, self.storage);
            self.shared.notify();
        }
    }
}

/// Lets a running scan report its progress at most every
/// [`PROGRESS_INTERVAL`] (PERF-006), so a fast scan does not flood the
/// app with status changes.
#[derive(Debug)]
struct ProgressPacer {
    /// When progress was last reported, or the scan started.
    last: Instant,
}

impl ProgressPacer {
    fn new(started: Instant) -> Self {
        Self { last: started }
    }

    /// Whether progress is reported at `now`; when it is, the next report
    /// waits a whole interval again.
    fn is_due(&mut self, now: Instant) -> bool {
        if now.duration_since(self.last) <= PROGRESS_INTERVAL {
            return false;
        }
        self.last = now;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: PERF-006
    #[test]
    fn a_scan_reports_its_progress_at_most_every_800_ms() {
        let started = Instant::now();
        let mut pacer = ProgressPacer::new(started);
        let after = |millis| started + Duration::from_millis(millis);

        let reported: Vec<u64> = (0..=2500)
            .step_by(100)
            .filter(|millis| pacer.is_due(after(*millis)))
            .collect();

        assert_eq!(reported, [900, 1800]);
    }
}
