// SPDX-License-Identifier: AGPL-3.0-only
//! The name of the Delete command (CMD-003).
//!
//! As in Windows Explorer, the command bar, the right-click menu and its
//! compact strip name Delete plainly, "Delete", wherever it is. What Delete
//! does is decided per item when it runs (`plan_delete`): items that can go
//! to the Trash go there, and only items that cannot are deleted
//! permanently, after a question that says so. In the Recycle Bin, whose
//! items can only be deleted for good, it reads "Delete permanently".
//!
//! The Python app (`deleteLabel` in `v2.0.0:desktop/ui/app.js`) named it
//! "Move to Trash" or "Delete permanently" after asking GIO about one item
//! per folder, which could name the command wrongly for the item actually
//! selected; the native app no longer asks.

use ox_core::location::{same_location, TRASH_URI};

use crate::window::BrowserWindow;

/// The label of Delete outside the Recycle Bin.
const DELETE: &str = crate::i18n::message_id("Delete");

/// The label of Delete in the Recycle Bin, whose items can only be
/// deleted for good.
const DELETE_PERMANENTLY: &str = crate::i18n::message_id("Delete permanently");

/// The label of Delete in the folder at `folder_uri`.
fn delete_label_in(folder_uri: &str) -> &'static str {
    if same_location(folder_uri, TRASH_URI) {
        ox_core::i18n::gettext_static(DELETE_PERMANENTLY)
    } else {
        ox_core::i18n::gettext_static(DELETE)
    }
}

impl BrowserWindow {
    /// The label of Delete in the folder shown.
    pub(crate) fn delete_label(&self) -> &'static str {
        delete_label_in(&self.current_uri().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: CMD-003
    #[test]
    fn delete_reads_delete_except_in_the_recycle_bin() {
        assert_eq!(delete_label_in("file:///home/user"), "Delete");
        assert_eq!(delete_label_in("smb://nas/share"), "Delete");
        assert_eq!(delete_label_in("trash:///"), "Delete permanently");
    }
}
