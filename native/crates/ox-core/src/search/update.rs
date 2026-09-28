// SPDX-License-Identifier: AGPL-3.0-only
//! A live update: re-reading the folders that changed below a root.
//!
//! Ports `_read` and `_update` in `desktop/index_service.py` (SRCH-028,
//! SRCH-030, SRCH-032). Only the changed folder and the folders that are
//! new below it are read; the rest of the cache stays as it is.

use std::collections::HashSet;

use super::error::{check_cancelled, SearchError};
use super::policy::{IndexScope, RootStorage};
use super::root::IndexRoot;
use super::scan::ListedItem;
use super::state::{FolderKey, Shared};

/// A queued live update.
#[derive(Debug)]
pub(super) struct UpdateJob {
    pub(super) root: IndexRoot,
    pub(super) folder: String,
    pub(super) cancellable: gio::Cancellable,
}

impl UpdateJob {
    /// The key the service tracks this update under.
    pub(super) fn key(&self) -> FolderKey {
        FolderKey {
            root: self.root.uri.clone(),
            folder: self.folder.clone(),
        }
    }
}

/// How an update that did not fail ended.
enum UpdateEnd {
    /// Every folder was read.
    Finished,
    /// The service closed or the server is being signed out of.
    Abandoned,
}

/// Runs `job`, records the result and ends its bookkeeping.
pub(super) fn run_update(shared: &Shared, job: &UpdateJob) {
    let root = &job.root.uri;
    let storage = RootStorage::current(root);
    match update_folders(shared, job, storage) {
        Ok(UpdateEnd::Finished) => shared.report_monitoring(root, storage),
        Ok(UpdateEnd::Abandoned) => {}
        // Safety rule "a failed check keeps the last good data" (SRCH-030):
        // only the status changes; nothing cached is removed.
        Err(error) => shared.report_failed_check(root, &error.to_string()),
    }
    shared.finish_update_job(&job.key());
}

/// Re-reads the changed folder and every folder that appeared below it.
fn update_folders(shared: &Shared, job: &UpdateJob, storage: RootStorage) -> Result<UpdateEnd, SearchError> {
    let root = &job.root.uri;
    let scope = shared.scope_of(root)?;
    let mut pending = vec![job.folder.clone()];
    let mut seen = HashSet::new();
    while let Some(folder) = pending.pop() {
        if is_abandoned(shared, root) {
            return Ok(UpdateEnd::Abandoned);
        }
        check_cancelled(&job.cancellable)?;
        if seen.contains(&folder) || !scope.admits(&folder) {
            continue;
        }
        seen.insert(folder.clone());
        // Safety rule "at most 10,000 folders per live update" (SRCH-032,
        // `ServiceLimits::folders_per_update`): a larger change needs a
        // full scan, which the user starts with Refresh.
        if seen.len() > shared.limits.folders_per_update {
            return Err(SearchError::TooManyNewFolders);
        }
        shared.watch_folder(root, &folder, storage);
        // Safety rule "never prune after a failed read" (`replace_directory`
        // in `search_index.py`): a read that fails returns here, before the
        // cached children are replaced by an incomplete listing.
        let items = read_whole_folder(shared, job, &folder, &scope)?;
        let new_folders = shared.index.replace_folder_contents(root, &folder, &items)?;
        pending.extend(new_folders);
    }
    Ok(UpdateEnd::Finished)
}

/// Whether the update must stop without a result.
fn is_abandoned(shared: &Shared, root: &str) -> bool {
    let state = shared.state();
    state.closed || state.is_paused(root)
}

/// Every indexable item of `folder` (`_read` in Python). The listing must
/// be complete, because the cached children are replaced by it.
///
/// Safety rule "at most a million entries per folder" (SRCH-032,
/// `ServiceLimits::entries_per_folder`): a larger folder is not stored at
/// all, because its cached children are replaced only by a whole listing.
fn read_whole_folder(
    shared: &Shared,
    job: &UpdateJob,
    folder: &str,
    scope: &IndexScope,
) -> Result<Vec<ListedItem>, SearchError> {
    let mut items = Vec::new();
    let mut receive = |batch| {
        check_cancelled(&job.cancellable)?;
        items.extend(scope.admit(batch)?);
        if items.len() > shared.limits.entries_per_folder {
            return Err(SearchError::FolderTooLarge);
        }
        Ok(())
    };
    shared
        .reader
        .read_folder(folder, job.root.hidden_items, &job.cancellable, &mut receive)?;
    Ok(items)
}
