// SPDX-License-Identifier: AGPL-3.0-only
//! What tests of the search cache share: starting it beside a test
//! window's settings, indexing a folder, and waiting for its state.

use gtk::glib;
use ox_core::search::{Caching, IndexRoot, RootStatus};

use super::harness::{wait_until, TestWindow};
use crate::search::CacheLocation;

impl TestWindow {
    /// Starts the search cache in a temporary directory beside the
    /// window's settings, and waits until it has read its status.
    pub(crate) fn start_search_cache(&self) {
        let directory = self.settings_directory().join("cache");
        self.context
            .start_search_cache(CacheLocation::Directory(directory));
        wait_until("the search cache to start", || {
            self.context.search_cache().status().is_some()
        });
    }

    /// Indexes `uri` and waits until its first scan is done.
    pub(crate) fn index_folder(&self, uri: &str) {
        let cache = self.context.search_cache().clone();
        let folder = uri.to_owned();
        glib::spawn_future_local(async move {
            cache
                .set_caching(&folder, Caching::Enabled, "Indexed")
                .await
                .expect("a local folder can be indexed");
        });
        self.wait_for_root(uri, RootStatus::Ready);
    }

    /// Waits until the status lists `uri` as an enabled root in `status`.
    pub(crate) fn wait_for_root(&self, uri: &str, status: RootStatus) {
        wait_until("the folder to be indexed", || {
            self.find_root(uri)
                .is_some_and(|root| root.is_enabled() && root.status == status)
        });
    }

    /// The root `uri` as the search cache last read it.
    pub(crate) fn find_root(&self, uri: &str) -> Option<IndexRoot> {
        let roots = self.context.search_cache().roots();
        roots.into_iter().find(|root| root.uri == uri)
    }
}
