// SPDX-License-Identifier: AGPL-3.0-only
//! Sidebar composition without network I/O or automatic bookmarking.
//!
//! Two sidebar sections are composed here from snapshots the caller
//! supplies; building the sidebar never mounts a share or saves anything:
//!
//! - `quick_access`: the standard folders and the user's pins, ported from
//!   `environment` in `desktop/winspace.py`. The standard folders come from
//!   `known_folders` and `user_dirs`, ported from
//!   `desktop/folder_locations.py`.
//! - `network`: saved shares, active mounts and visited servers merged into
//!   one Network list, ported from `desktop/network_locations.py`.
//! - `desktop_bookmarks`: the pins mirrored into the desktop-wide places
//!   list (SIDE-013), which the app writes only when its pins change.

mod desktop_bookmarks;
mod known_folders;
mod network;
mod quick_access;
mod user_dirs;

pub use desktop_bookmarks::{bookmarks_file, merged_bookmarks, sync_bookmarks};
pub use known_folders::{FolderLocations, KnownFolder, KnownFolderPaths};
pub use network::{
    merge_network_locations, network_key, NetworkKey, NetworkKind, NetworkLocation, NetworkMount, SavedShare,
    StableMount,
};
pub use quick_access::{compose_quick_access, quick_access, Place};
