// SPDX-License-Identifier: AGPL-3.0-only
//! What the Search & indexing settings ask of the search cache, and what
//! the window's message line says about it.
//!
//! Ports the handlers of `renderSettingsCache` and `renderSettingsPage` in
//! `desktop/ui/app.js` (`setCache`, `cacheRefresh`, `cacheStop`,
//! `cacheClear`, the Add button and "Refresh all"; SET-006, SET-007,
//! SRCH-019, SRCH-023). Each runs off the main thread; a failure is shown
//! in the window's message line, as the Python toasts were.

use gtk::glib;
use gtk::prelude::*;
use ox_core::location::{file_uri, normalise_location};
use ox_core::search::Caching;

use crate::search::CacheError;
use crate::settings_page::{SettingsPage, MESSAGE};

/// What the window says after a folder was added to the index
/// (`setCache` in app.js).
const CACHING_STARTED: &str =
    "Caching filenames and paths in the background. No file contents are downloaded.";

/// A request of the Search & indexing settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IndexCommand {
    /// Index the folder `uri`, listed under `label` (`cacheSet` on).
    Index {
        /// The folder.
        uri: String,
        /// Its name in the list.
        label: String,
    },
    /// Index the folder typed into "Add a folder", relative to `base`.
    IndexTyped {
        /// What was typed: a path, a relative path or a share.
        typed: String,
        /// The folder a relative path starts from: the one shown before
        /// Settings (`state.settingsOrigin`), or the home folder.
        base: Option<String>,
    },
    /// Rescan the folder (`cacheRefresh`).
    Refresh(String),
    /// Rescan every indexed folder ("Refresh all").
    RefreshAll,
    /// Stop the folder's running scan (`cacheStop`).
    Stop(String),
    /// Delete the folder's cached names but keep it (`cacheClear`).
    Clear(String),
    /// Stop indexing the folder and delete its names (`cacheSet` off).
    StopIndexing(String),
}

impl SettingsPage {
    /// Runs `command` on the search cache off the main thread; a failure
    /// is shown in the window's message line.
    pub(crate) fn run_index_command(&self, command: IndexCommand) {
        let cache = self.context().search_cache().clone();
        let page = self.downgrade();
        glib::spawn_future_local(async move {
            let outcome = run(&cache, command).await;
            let Some(page) = page.upgrade() else {
                return;
            };
            match outcome {
                Ok(Some(message)) => page.emit_by_name::<()>(MESSAGE, &[&message.to_owned()]),
                Ok(None) => {}
                Err(error) => page.emit_by_name::<()>(MESSAGE, &[&error.to_string()]),
            }
        });
    }
}

/// Why an index command failed.
#[derive(Debug, thiserror::Error)]
enum CommandError {
    /// The typed address is not a location.
    #[error(transparent)]
    Location(#[from] ox_core::location::LocationError),
    /// The cache refused or failed.
    #[error(transparent)]
    Cache(#[from] CacheError),
}

/// Runs `command`; returns what the window says on success, if anything.
async fn run(
    cache: &crate::search::SearchCache,
    command: IndexCommand,
) -> Result<Option<&'static str>, CommandError> {
    match command {
        IndexCommand::Index { uri, label } => {
            cache.set_caching(&uri, Caching::Enabled, &label).await?;
            return Ok(Some(CACHING_STARTED));
        }
        IndexCommand::IndexTyped { typed, base } => {
            let uri = typed_folder(&typed, base.as_deref())?;
            cache.set_caching(&uri, Caching::Enabled, "").await?;
            return Ok(Some(CACHING_STARTED));
        }
        IndexCommand::Refresh(uri) => cache.refresh(Some(&uri)).await?,
        IndexCommand::RefreshAll => cache.refresh(None).await?,
        IndexCommand::Stop(uri) => cache.stop(&uri).await?,
        IndexCommand::Clear(uri) => cache.clear(&uri).await?,
        IndexCommand::StopIndexing(uri) => cache.set_caching(&uri, Caching::Disabled, "").await?,
    }
    Ok(None)
}

/// The folder `typed` names, relative to `base` or the home folder
/// (`normalise` with `base:state.settingsOrigin||state.env.home`).
fn typed_folder(typed: &str, base: Option<&str>) -> Result<String, ox_core::location::LocationError> {
    let home = glib::home_dir();
    let base = base.map_or_else(|| file_uri(&home), str::to_owned);
    normalise_location(typed, Some(&base), &home)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ported from the Add button of `renderSettingsPage` in
    /// `desktop/ui/app.js`: a relative path starts at the folder shown
    /// before Settings, and a UNC path names a share.
    ///
    /// parity: SET-007
    #[test]
    fn a_typed_folder_is_read_relative_to_the_folder_shown_before() {
        let base = "file:///home/demo/Work";

        assert_eq!(
            typed_folder("Projects", Some(base)).unwrap(),
            "file:///home/demo/Work/Projects"
        );
        assert_eq!(
            typed_folder("\\\\nas\\share", Some(base)).unwrap(),
            "smb://nas/share"
        );
        assert!(typed_folder("", Some(base)).is_err());
    }
}
