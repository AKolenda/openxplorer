// SPDX-License-Identifier: AGPL-3.0-only
//! Sidebar composition without network I/O or automatic bookmarking.
//!
//! Two sidebar sections are composed here from snapshots the caller
//! supplies; building the sidebar never mounts a share or saves anything:
//!
//! - `quick_access`: the standard folders and the user's pins, ported from
//!   `environment` in `desktop/winspace.py`.
//! - `network`: saved shares, active mounts and visited servers merged into
//!   one Network list, ported from `desktop/network_locations.py`.

mod network;
mod quick_access;

pub use network::{
    merge_network_locations, network_key, NetworkKey, NetworkKind, NetworkLocation, NetworkMount, SavedShare,
    StableMount,
};
pub use quick_access::{compose_quick_access, known_folders, quick_access, FolderGlyph, Place};
