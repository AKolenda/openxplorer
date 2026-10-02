// SPDX-License-Identifier: AGPL-3.0-only
//! Copies and moves the app never finished (OPS-038).
//!
//! The native counterpart of the Python app's "The interface stopped
//! unexpectedly." dialog: while a copy or move into a folder runs, a mark
//! names its destination ([`ox_core::ops::UnfinishedMarks`]). When the app
//! starts after a run that never ended, the first window lists the private
//! staging and backup items that run left behind, and deletes nothing.

use ox_core::ops::{leftovers_message, UnfinishedMark, UnfinishedMarks, UNFINISHED_TITLE};

use crate::dialog;
use crate::window::BrowserWindow;

/// Marks a copy or move into `destination` as running until the mark is
/// dropped; nothing for Trash and delete, which stage nothing. A mark
/// that cannot be written only loses the report after a crash.
pub(super) fn mark_unfinished(destination: Option<&str>) -> Option<UnfinishedMark> {
    let destination = destination?.to_owned();
    UnfinishedMarks::in_cache_directory().mark(&[destination]).ok()
}

impl BrowserWindow {
    /// Tells the user what copies and moves that never finished left
    /// behind, if anything.
    pub(crate) async fn report_unfinished_operations(&self) {
        let leftovers = UnfinishedMarks::in_cache_directory()
            .collect_leftovers()
            .await
            .unwrap_or_default();
        if !leftovers.is_empty() {
            dialog::show_message(
                self,
                ox_core::i18n::gettext_static(UNFINISHED_TITLE),
                &leftovers_message(&leftovers),
            )
            .await;
        }
    }
}
