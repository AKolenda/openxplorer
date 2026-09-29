// SPDX-License-Identifier: AGPL-3.0-only
//! The private, metadata-only filename search cache and the service that
//! keeps it up to date.
//!
//! Ports `desktop/search_index.py` (the SQLite cache),
//! `desktop/index_service.py` (scans, live updates and the one index owner
//! across processes), `desktop/local_watch.py` (inotify), the mount-table
//! reading of `desktop/mount_support.py`, `index_directory` of
//! `desktop/gio_backend.py` and the cache operations of
//! `desktop/winspace.py`. The database, `~/.cache/winspace/search.sqlite3`,
//! is shared with the Python app, so both read each other's cache.
//!
//! Privacy and safety rules that hold throughout:
//!
//! - Each indexed folder (a root) is an explicit choice of the user, or a
//!   Quick access pin while "Index pinned folders automatically" is on.
//! - Only names and metadata are stored. No file is opened, no share or
//!   device is mounted, and symlinks and virtual items are never followed.
//! - The cache is a 0600 file in a 0700 directory, checked again on every
//!   open, because paths can reveal private information.
//! - Only a complete scan prunes; a cancelled or offline scan keeps the
//!   earlier results.
//!
//! Every operation blocks on SQLite or the file system, so the app calls
//! them off the main thread (for example with `gio::spawn_blocking`) and
//! passes a `gio::Cancellable` where one is taken. Scans and live updates
//! run on the [`IndexService`]'s own worker thread.
//!
//! | Module | Responsibility | Ports |
//! |---|---|---|
//! | `index` | Opening the database, roots and the cache status | `search_index.py` |
//! | `schema` | The shared tables and their upgrades | `search_index.py` |
//! | `root` | Roots, their status words and the cache status | `search_index.py`, `index_service.py` |
//! | `query` | Searching | `search_index.py` |
//! | `hit` | A search hit as a listed item | `app.js` |
//! | `scan` | How scans and live updates write | `search_index.py` |
//! | `requests` | Requests other processes leave for the owner | `search_index.py`, `index_service.py` |
//! | `service` | The coordinator, its worker and ownership | `index_service.py` |
//! | `commands` | What the settings, file operations and sign-in ask | `winspace.py`, `index_service.py` |
//! | `tick` | The coordinator's periodic work | `index_service.py` |
//! | `state` | What the coordinator shares with its worker | `index_service.py` |
//! | `ownership` | Electing one owner across processes | `index_service.py` |
//! | `crawl` | A full scan | `index_service.py` |
//! | `update` | A live update of changed folders | `index_service.py` |
//! | `limits` | The entry, folder and watch limits | `index_service.py`, `local_watch.py` |
//! | `policy` | What may be indexed, and network roots | `index_service.py` |
//! | `mounts` | The kernel's mount table | `mount_support.py` |
//! | `watch` | inotify watches | `local_watch.py` |
//! | `reader` | Reading one folder through GIO | `gio_backend.py` |
//! | `pins` | Indexing pinned folders (SRCH-040) | new |
//! | `pattern` | The words and wildcards a name must match | `app.js`, Dolphin |
//! | `text` | Folding, display paths and URI containment | `search_index.py` |
//! | `error` | The error of every operation | both |

mod commands;
mod crawl;
mod error;
mod hit;
mod index;
mod limits;
mod mounts;
mod ownership;
mod pattern;
mod pins;
mod policy;
mod query;
mod reader;
mod requests;
mod root;
mod scan;
mod schema;
mod service;
mod state;
mod text;
mod tick;
mod update;
mod watch;

#[cfg(test)]
mod cache_tests;
#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod service_tests;

pub use error::SearchError;
pub use index::SearchIndex;
pub use pattern::{is_wildcard, NamePattern};
pub use pins::PinIndexing;
pub use query::{
    SearchHit, SearchQuery, SearchResults, DEFAULT_RESULT_LIMIT, MAX_QUERY_CHARS, MAX_RESULT_LIMIT,
};
pub use reader::{FolderReader, GioFolderReader};
pub use root::{
    CacheStatus, Caching, HiddenItems, IndexRoot, RootOrigin, RootStatus, ScanGeneration, SearchEngine,
    UpdateMode,
};
pub use scan::ListedItem;
pub use service::{AutoIndex, IndexService, IndexSettings};
pub use text::display_path;
