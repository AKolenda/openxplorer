// SPDX-License-Identifier: AGPL-3.0-only
//! Searching: the search cache every window shares and what a window's
//! search box does with it.
//!
//! Ports the search parts of `desktop/ui/app.js` (`queueSearch`,
//! `runSearch`, `renderSearchInfo`, `refreshCacheStatus`, `setCache`) on
//! top of ox-core's port of `desktop/search_index.py` and
//! `desktop/index_service.py`. The window's search box searches the
//! current folder: a folder no indexed folder covers is filtered as it is
//! listed, and an indexed one is searched in the cache of names and
//! paths, with the folder's own matches first.
//!
//! | Module | Responsibility | Ports |
//! |---|---|---|
//! | `cache` | The shared [`SearchCache`] GObject: status, operations, signals | `winspace.py`, `app.js` |
//! | `indexer` | The thread that starts and ticks the index service | `winspace.py` |
//! | `pin_sync` | Which pins appeared and disappeared (SRCH-040) | new |
//! | `folder_search` | One window's search and its runs | `app.js` |
//! | `source` | The scope, and where a search looks | `app.js` |
//! | `results` | The rows of a cached search | `app.js` |
//! | `report` | The strip's, status bar's and empty page's wording | `app.js` |
//! | `info_strip` | The [`SearchInfoStrip`] widget | `app.js`, `style.css` |
//! | `error` | [`CacheError`] | both |

mod cache;
mod error;
mod folder_search;
mod indexer;
mod info_strip;
mod pin_sync;
mod report;
mod results;
mod source;

#[cfg(test)]
pub(crate) mod test_roots;

pub(crate) use cache::SearchCache;
pub(crate) use error::CacheError;
pub(crate) use folder_search::{FolderSearch, SearchRun};
pub(crate) use indexer::{CacheLocation, IndexerStart};
pub(crate) use info_strip::SearchInfoStrip;
pub(crate) use pin_sync::SettingsReading;
pub(crate) use report::{SearchCount, RESULT_LIMIT};
pub(crate) use results::{merge_results, Listing};
pub(crate) use source::{related_roots, SearchScope};
