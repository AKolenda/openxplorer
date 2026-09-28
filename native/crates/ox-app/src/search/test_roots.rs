// SPDX-License-Identifier: AGPL-3.0-only
//! Indexed folders as the cache status lists them, for the unit tests of
//! the search module.

use ox_core::search::{Caching, HiddenItems, IndexRoot, RootOrigin, RootStatus, UpdateMode};

/// An enabled root at `uri` that the user chose, scanned and ready.
pub(crate) fn enabled_root(uri: &str) -> IndexRoot {
    IndexRoot {
        uri: uri.to_owned(),
        label: String::new(),
        caching: Caching::Enabled,
        status: RootStatus::Ready,
        updated: Some(1_700_000_000),
        scanned: 17,
        error: None,
        generation: None,
        hidden_items: HiddenItems::Skip,
        update_mode: UpdateMode::LiveLocalEvents,
        watch_count: 3,
        watch_error: None,
        last_event: None,
        entry_count: 17,
        origin: RootOrigin::User,
    }
}
