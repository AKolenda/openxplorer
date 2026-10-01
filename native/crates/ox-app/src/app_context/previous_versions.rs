// SPDX-License-Identifier: AGPL-3.0-only
//! The previous-versions service every window shares, and the write
//! protection it gives file operations.
//!
//! The Python app kept one `PreviousVersions` service for the whole
//! application (`v2.0.0:desktop/winspace.py`): the snapshot collections a lookup
//! finds in one window are read-only in every window for the rest of the
//! session (PROP-024), and every write asks it first (XFER-020).

use std::sync::Arc;

use gtk::subclass::prelude::*;
use ox_core::ops::WriteProtection;
use ox_core::versions::PreviousVersions;

use super::AppContext;

impl AppContext {
    /// The previous-versions service.
    pub(crate) fn previous_versions(&self) -> &Arc<PreviousVersions> {
        self.imp()
            .previous_versions
            .get()
            .expect("AppContext::new creates the previous-versions service")
    }

    /// The protection every file operation runs with: previous versions
    /// (snapshots and backups) are never changed in place
    /// (`PreviousVersions.assert_writable` in `v2.0.0:desktop/previous_versions.py`).
    pub(crate) fn write_protection(&self) -> WriteProtection {
        WriteProtection::new(self.previous_versions().write_guard())
    }
}
