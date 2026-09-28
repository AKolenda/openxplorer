// SPDX-License-Identifier: AGPL-3.0-only
//! A full scan of one root.
//!
//! Ports `_run` in `desktop/index_service.py` (SRCH-024, SRCH-031,
//! SRCH-032). The scan walks the root depth first, stores what it reads
//! under a new generation and, only when every folder was read, prunes
//! what it did not see.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use super::error::{check_cancelled, SearchError};
use super::policy::{IndexScope, RootStorage};
use super::root::{IndexRoot, ScanGeneration};
use super::scan::{ListedItem, ScanOutcome};
use super::state::Shared;

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

/// Reads every folder of the root that its scope admits and says how the
/// scan ended.
fn walk_root(
    shared: &Shared,
    job: &ScanJob,
    generation: &ScanGeneration,
    storage: RootStorage,
) -> Result<ScanOutcome, SearchError> {
    let scope = shared.scope_of(&job.root.uri)?;
    let mut crawl = Crawl::new(shared, job, generation, scope, storage);
    crawl.run()?;
    Ok(crawl.outcome())
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
    /// Entries stored so far.
    stored: usize,
    /// Why folders could not be read; the scan goes on without them.
    errors: Vec<String>,
    /// Folders read or skipped already.
    visited: HashSet<String>,
    /// Folders still to read; the last is read next, so the walk is depth
    /// first, as in Python.
    pending: Vec<PendingFolder>,
    /// When progress was last reported.
    last_report: Instant,
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
    ) -> Self {
        Self {
            shared,
            job,
            generation,
            scope,
            storage,
            stored: 0,
            errors: Vec::new(),
            visited: HashSet::new(),
            pending: Vec::new(),
            last_report: Instant::now(),
        }
    }
}

impl Crawl<'_> {
    /// Reads every folder of the root. Unreadable folders are recorded
    /// and skipped, except the root itself: without it nothing is known.
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
            self.errors.push(error.to_string());
            if folder.uri == root || self.stored >= self.shared.limits.entries_per_root {
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
    /// [`ServiceLimits::entries_per_root`]): what does not fit is not
    /// stored, and the scan stops with the limit message.
    fn store_batch(&mut self, batch: Vec<ListedItem>, child_depth: usize) -> Result<(), SearchError> {
        check_cancelled(&self.job.cancellable)?;
        let admitted = self.scope.admit(batch)?;
        let limit = self.shared.limits.entries_per_root;
        let room = limit.saturating_sub(self.stored);
        let fitting = &admitted[..admitted.len().min(room)];
        self.stored += self
            .shared
            .index
            .store_scanned(&self.job.root.uri, self.generation, fitting)?;
        self.queue_folders(&admitted, child_depth);
        if admitted.len() > room || self.stored >= limit {
            return Err(SearchError::EntryLimit);
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
        if self.last_report.elapsed() <= PROGRESS_INTERVAL {
            return;
        }
        self.shared.report_monitoring(&self.job.root.uri, self.storage);
        self.shared.notify();
        self.last_report = Instant::now();
    }
}
