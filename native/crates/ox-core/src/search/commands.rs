// SPDX-License-Identifier: AGPL-3.0-only
//! What the Search & indexing settings, file operations and sign-in ask
//! of the index service.
//!
//! Ports the `cacheSet`, `cacheRefresh`, `cacheStop`, `cacheClear` and
//! `cacheRemove` bridge operations and `invalidate_cache_for_write` in
//! `desktop/winspace.py`, and `cancel`, `pause_server` and
//! `resume_server` in `desktop/index_service.py` (SRCH-019 to SRCH-023,
//! SRCH-033). A process that does not own the index passes each request
//! on to the owner through the database.

use std::time::Instant;

use super::error::SearchError;
use super::index::indexable_root;
use super::requests::IndexRequest;
use super::root::{Caching, HiddenItems, RootStatus};
use super::service::{IndexService, ScanTrigger};
use super::text::{host_of, is_at_or_below};
use crate::location::normalise;

impl IndexService {
    /// Chooses whether `uri` is indexed (`cacheSet`, SRCH-019). Enabling
    /// starts a scan; disabling stops a running one and deletes the root's
    /// cached entries. An empty `label` becomes the location's display
    /// path. The choice makes the root the user's: unpinning its folder no
    /// longer removes it (SRCH-040).
    ///
    /// # Errors
    ///
    /// [`SearchError::Location`] for an invalid address,
    /// [`SearchError::DeviceLocation`] for a phone or camera,
    /// [`SearchError::ServerList`] for an SMB server's share list, and
    /// database errors.
    pub fn configure(
        &self,
        uri: &str,
        caching: Caching,
        label: &str,
        hidden_items: HiddenItems,
    ) -> Result<(), SearchError> {
        let uri = indexable_root(uri)?;
        if caching == Caching::Disabled {
            self.stop(&uri)?;
        }
        self.index().configure(&uri, caching, label, hidden_items)?;
        if caching == Caching::Enabled {
            self.refresh(&uri)?;
        }
        Ok(())
    }

    /// Rescans `root` now, or asks the owning process to (`cacheRefresh`,
    /// SRCH-023). A server paused for sign-out is resumed, because the
    /// user asked.
    ///
    /// # Errors
    ///
    /// [`SearchError::Location`] for an invalid address, and database
    /// errors.
    pub fn refresh(&self, root: &str) -> Result<(), SearchError> {
        let root = normalise(root)?;
        self.start_scan(&root, ScanTrigger::User)?;
        Ok(())
    }

    /// Rescans every enabled root (`cacheRefresh` without a location,
    /// "Refresh all").
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn refresh_all(&self) -> Result<(), SearchError> {
        let roots = self.index().roots()?;
        for root in roots.iter().filter(|root| root.is_enabled()) {
            self.start_scan(&root.uri, ScanTrigger::User)?;
        }
        Ok(())
    }

    /// Stops the running scan and live updates of `root`, here or in the
    /// owning process (`cacheStop`, "Stop indexing"). What was cached so
    /// far stays.
    ///
    /// # Errors
    ///
    /// [`SearchError::Location`] for an invalid address, and database
    /// errors.
    pub fn stop(&self, root: &str) -> Result<(), SearchError> {
        let root = normalise(root)?;
        if !self.is_owner() {
            self.index()
                .enqueue(&IndexRequest::Cancel { root: root.clone() })?;
        }
        self.shared.state().cancel_root(&root);
        Ok(())
    }

    /// Stops indexing `root` and deletes its cached entries; the root
    /// stays, as "Not indexed" (`cacheClear`, SRCH-023).
    ///
    /// # Errors
    ///
    /// [`SearchError::Location`] for an invalid address, and database
    /// errors.
    pub fn clear(&self, root: &str) -> Result<(), SearchError> {
        self.stop(root)?;
        self.index().clear(root)
    }

    /// Stops indexing `root` and deletes it with its cached entries
    /// (`cacheRemove`, SRCH-023).
    ///
    /// # Errors
    ///
    /// [`SearchError::Location`] for an invalid address, and database
    /// errors.
    pub fn remove(&self, root: &str) -> Result<(), SearchError> {
        self.stop(root)?;
        self.index().remove(root)
    }

    /// Re-reads `folder` in every enabled root that holds it, after the
    /// app created, renamed, pasted, deleted or extracted something there
    /// (`invalidate_cache_for_write`, SRCH-033). Pass the folders whose
    /// contents changed: the destination and the parents of the changed
    /// items.
    ///
    /// Python also passed the changed items themselves, so a renamed or
    /// deleted file was read as a folder, which failed and reported the
    /// root as offline; passing folders only avoids that.
    ///
    /// # Errors
    ///
    /// [`SearchError::Location`] for an invalid address, and database
    /// errors.
    pub fn folder_changed(&self, folder: &str) -> Result<(), SearchError> {
        let folder = normalise(folder)?;
        let roots = self.index().roots()?;
        let holding_roots = roots
            .iter()
            .filter(|root| root.is_enabled() && is_at_or_below(&folder, &root.uri));
        for root in holding_roots {
            self.record_change(&root.uri, &folder, Instant::now())?;
        }
        Ok(())
    }

    /// Stops indexing SMB server `host` while the user signs out: its
    /// scans are cancelled, and it is skipped until resumed.
    ///
    /// # Errors
    ///
    /// Database errors while passing the request on to the owner.
    pub fn pause_server(&self, host: &str) -> Result<(), SearchError> {
        let host = host.to_lowercase();
        let is_owner = {
            let mut state = self.shared.state();
            state.paused_hosts.insert(host.clone());
            state.cancel_scans_on_host(&host);
            state.ownership.is_owner()
        };
        if !is_owner {
            self.index().enqueue(&IndexRequest::PauseServer { host })?;
        }
        Ok(())
    }

    /// Deletes the cached entries of every root on SMB server `host`, for a
    /// sign-out with "Also clear cached filenames for this server"
    /// (NET-022). The roots stay, as "Not indexed".
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn clear_server(&self, host: &str) -> Result<(), SearchError> {
        let host = host.to_lowercase();
        let roots = self.index().roots()?;
        let on_host = roots
            .iter()
            .filter(|root| host_of(&root.uri).as_deref() == Some(host.as_str()));
        for root in on_host {
            self.clear(&root.uri)?;
        }
        Ok(())
    }

    /// Indexes SMB server `host` again after the user signed in to it.
    ///
    /// Its enabled roots whose last scan did not complete are scanned
    /// again, so a pinned share that needed sign-in is indexed once the
    /// user signs in (SRCH-040). The Python app waited for the next
    /// network check instead.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn resume_server(&self, host: &str) -> Result<(), SearchError> {
        let host = host.to_lowercase();
        self.shared.state().paused_hosts.remove(&host);
        if !self.is_owner() {
            return self.index().enqueue(&IndexRequest::ResumeServer { host });
        }
        self.rescan_unfinished_roots(&host)
    }

    /// Scans again the enabled roots on `host` whose last scan did not
    /// complete.
    fn rescan_unfinished_roots(&self, host: &str) -> Result<(), SearchError> {
        let roots = self.index().roots()?;
        let unfinished = roots.iter().filter(|root| {
            let is_on_host = host_of(&root.uri).as_deref() == Some(host);
            is_on_host && root.is_enabled() && root.status != RootStatus::Ready
        });
        for root in unfinished {
            self.start_scan(&root.uri, ScanTrigger::Automatic)?;
        }
        Ok(())
    }
}
