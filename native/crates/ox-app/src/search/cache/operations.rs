// SPDX-License-Identifier: AGPL-3.0-only
//! What the windows and Settings ask of the search cache: searching, and
//! choosing, refreshing, stopping and clearing indexed folders.
//!
//! Ports the `search`, `cacheSet`, `cacheRefresh`, `cacheStop` and
//! `cacheClear` bridge operations of `v2.0.0:desktop/winspace.py`. Each runs on a
//! GIO worker thread once the index service has started; one that changes
//! the cache reads the status again, so every window shows its effect.

use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::search::{
    Caching, HiddenItems, IndexService, PinIndexing, SearchError, SearchQuery, SearchResults,
};
use ox_core::settings::Bookmark;
use ox_core::LOG_DOMAIN;

use super::{SearchCache, StatusReading};
use crate::search::error::CacheError;
use crate::search::indexer::{self, Indexer};

impl SearchCache {
    /// Searches the cache for `query` (the `search` bridge operation). A
    /// newer search cancels `cancellable`, which stops this one.
    ///
    /// # Errors
    ///
    /// The cache's errors, such as a query that is too long, and
    /// [`SearchError::Cancelled`] once cancelled.
    pub(crate) async fn search(
        &self,
        query: SearchQuery,
        cancellable: gio::Cancellable,
    ) -> Result<SearchResults, CacheError> {
        self.run(move |service| service.index().search(&query, Some(&cancellable)))
            .await
    }

    /// Chooses whether `uri` is indexed, under `label` (`cacheSet`,
    /// SRCH-019).
    ///
    /// # Errors
    ///
    /// The cache's errors, such as a phone or a server's share list.
    pub(crate) async fn set_caching(
        &self,
        uri: &str,
        caching: Caching,
        label: &str,
    ) -> Result<(), CacheError> {
        let uri = uri.to_owned();
        let label = label.to_owned();
        self.change(move |service| service.configure(&uri, caching, &label, HiddenItems::Skip))
            .await
    }

    /// Rescans `root`, or every enabled root for `None` (`cacheRefresh`,
    /// SRCH-023).
    ///
    /// # Errors
    ///
    /// The cache's errors.
    pub(crate) async fn refresh(&self, root: Option<&str>) -> Result<(), CacheError> {
        let root = root.map(str::to_owned);
        self.change(move |service| match root {
            Some(root) => service.refresh(&root),
            None => service.refresh_all(),
        })
        .await
    }

    /// Stops the running scan of `root` (`cacheStop`, "Stop indexing").
    ///
    /// # Errors
    ///
    /// The cache's errors.
    pub(crate) async fn stop(&self, root: &str) -> Result<(), CacheError> {
        let root = root.to_owned();
        self.change(move |service| service.stop(&root)).await
    }

    /// Deletes the cached names of `root` and keeps it as "Not indexed"
    /// (`cacheClear`).
    ///
    /// # Errors
    ///
    /// The cache's errors.
    pub(crate) async fn clear(&self, root: &str) -> Result<(), CacheError> {
        let root = root.to_owned();
        self.change(move |service| service.clear(&root)).await
    }

    /// The "Index pinned folders automatically" switch as last set.
    ///
    /// # Errors
    ///
    /// The cache's errors.
    pub(crate) async fn pin_indexing(&self) -> Result<PinIndexing, CacheError> {
        self.run(|service| service.index().pin_indexing()).await
    }

    /// Turns the switch on or off, indexing or un-indexing `pins`.
    ///
    /// # Errors
    ///
    /// The cache's errors.
    pub(crate) async fn set_pin_indexing(
        &self,
        pins: Vec<Bookmark>,
        indexing: PinIndexing,
    ) -> Result<(), CacheError> {
        self.change(move |service| service.set_pin_indexing(&pins, indexing))
            .await
    }

    /// Pauses indexing of SMB server `host` while the user signs out of it
    /// (`pause_server` in `index_service.py`): its running scans stop and it
    /// is skipped until [`Self::resume_server`].
    pub(crate) fn pause_server(&self, host: &str) {
        let host = host.to_owned();
        self.change_if_started("pause indexing the server", move |service| {
            service.pause_server(&host)
        });
    }

    /// Deletes the cached names of every indexed folder on SMB server
    /// `host`, for "Also clear cached filenames for this server" (NET-022).
    pub(crate) fn clear_server(&self, host: &str) {
        let host = host.to_owned();
        self.change_if_started("clear the server's cached names", move |service| {
            service.clear_server(&host)
        });
    }

    /// Indexes SMB server `host` again after a successful mount of it, so
    /// a pinned share that needed sign-in is indexed now (SRCH-040).
    pub(crate) fn resume_server(&self, host: &str) {
        let host = host.to_owned();
        self.change_if_started("resume indexing the server", move |service| {
            service.resume_server(&host)
        });
    }

    /// Whether indexing of SMB server `host` is paused for a sign-out.
    ///
    /// # Errors
    ///
    /// [`CacheError::NotStarted`] before [`Self::start`].
    #[cfg(test)]
    pub(crate) async fn is_server_paused(&self, host: &str) -> Result<bool, CacheError> {
        let host = host.to_owned();
        self.run(move |service| Ok(service.is_server_paused(&host))).await
    }

    /// Runs `change` in the background once the index service has
    /// started; before that there is nothing to change.
    fn change_if_started(
        &self,
        what: &'static str,
        change: impl FnOnce(&IndexService) -> Result<(), SearchError> + Send + 'static,
    ) {
        if self.imp().indexer.borrow().is_some() {
            self.change_in_background(what, change);
        }
    }

    /// Tells the service that the app wrote into `folders`, so every
    /// indexed folder that holds one reads it again (SRCH-033). Local
    /// folders follow their live watches as well; a share has none, so
    /// only this shows the app's own changes there before the next check.
    pub(crate) fn folders_written(&self, folders: Vec<String>) {
        if folders.is_empty() || self.imp().indexer.borrow().is_none() {
            return;
        }
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = cache)]
            self,
            async move {
                #[cfg(test)]
                let reported = folders.clone();
                let outcome = cache
                    .run(move |service| {
                        let mut changed = folders.iter();
                        changed.try_for_each(|folder| service.folder_changed(folder))
                    })
                    .await;
                if let Err(error) = &outcome {
                    glib::g_warning!(LOG_DOMAIN, "Could not update the search cache: {error}");
                }
                #[cfg(test)]
                if outcome.is_ok() {
                    cache.imp().written.borrow_mut().extend(reported);
                }
            }
        ));
    }

    /// The folders the service read again after the app wrote into them,
    /// for tests.
    #[cfg(test)]
    pub(crate) fn written_folders(&self) -> Vec<String> {
        self.imp().written.borrow().clone()
    }

    /// Runs `change` like [`Self::run`] and reads the status again after it
    /// succeeded, so every window shows its effect and re-runs a shown
    /// search (`setCache` in app.js).
    pub(super) async fn change(
        &self,
        change: impl FnOnce(&IndexService) -> Result<(), SearchError> + Send + 'static,
    ) -> Result<(), CacheError> {
        self.run(change).await?;
        self.read_status(StatusReading::AfterChange);
        Ok(())
    }

    /// Runs `change` in the background, logging its failure as `what`
    /// failed, for changes no control waits for.
    pub(super) fn change_in_background(
        &self,
        what: &'static str,
        change: impl FnOnce(&IndexService) -> Result<(), SearchError> + Send + 'static,
    ) {
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = cache)]
            self,
            async move {
                if let Err(error) = cache.change(change).await {
                    glib::g_warning!(LOG_DOMAIN, "Could not {what}: {error}");
                }
            }
        ));
    }

    /// Runs `operation` on the index service on a GIO worker thread, once
    /// the service has started.
    ///
    /// # Errors
    ///
    /// [`CacheError::NotStarted`] before [`Self::start`], and the
    /// operation's errors.
    pub(super) async fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&IndexService) -> Result<T, SearchError> + Send + 'static,
    ) -> Result<T, CacheError> {
        let service = self
            .imp()
            .indexer
            .borrow()
            .as_ref()
            .map(Indexer::service)
            .ok_or(CacheError::NotStarted)?;
        let outcome = gio::spawn_blocking(move || {
            let service = indexer::started(&service)?;
            operation(service).map_err(CacheError::from)
        })
        .await;
        outcome.unwrap_or(Err(CacheError::WorkerLost))
    }
}
