// SPDX-License-Identifier: AGPL-3.0-only
//! Whether each folder has a Trash, which names the Delete command
//! (CMD-003).
//!
//! Ports `trashKnown`, `refreshTrashSupport` and `deleteLabel` of
//! `v2.0.0:desktop/ui/app.js`. When the window shows a folder it has not asked
//! about, it asks GIO once, off the main thread, and remembers the answer
//! for the window's life. Delete reads "Delete permanently" only where the
//! folder is known to have no Trash (SMB shares, most remote backends);
//! while the answer is unknown it reads "Move to Trash". What Delete then
//! does is decided again per item when it runs (`plan_delete`), so a
//! stale answer can only change a label, never delete an item.

use std::collections::{HashMap, HashSet};

use gtk::glib;
use gtk::subclass::prelude::*;
use ox_core::location::{parent_location, same_location, TRASH_URI};
use ox_core::ops::{delete_command_label, trash_support};
use ox_core::transfer::Cancellation;

use crate::locations::Page;
use crate::window::BrowserWindow;

/// The label of Delete in the Recycle Bin, whose items can only be
/// deleted for good.
const DELETE_PERMANENTLY: &str = "Delete permanently";

/// The answers GIO gave, by folder.
#[derive(Debug, Default)]
pub(crate) struct TrashSupport {
    /// Folders asked about, and whether each has a Trash.
    known: HashMap<String, bool>,
    /// Folders whose answer is still coming.
    asking: HashSet<String>,
}

impl TrashSupport {
    /// Whether `folder` has a Trash; `None` while unknown.
    fn get(&self, folder: &str) -> Option<bool> {
        self.known.get(folder).copied()
    }
}

/// The folder whose Trash an item goes to: its parent, or `fallback`
/// (the folder shown) when it has none (`trashScope` in app.js).
fn trash_scope(uri: &str, fallback: &str) -> String {
    parent_location(uri).unwrap_or_else(|| fallback.to_owned())
}

impl BrowserWindow {
    /// The label of Delete for the first selected item, or for the folder
    /// shown when nothing is selected (`deleteLabel`).
    pub(crate) fn delete_label(&self) -> &'static str {
        let folder = self.current_uri().unwrap_or_default();
        if same_location(&folder, TRASH_URI) {
            return DELETE_PERMANENTLY;
        }
        let first_selected = self.folder_pane().model().selected_uris().into_iter().next();
        let scope = match first_selected {
            Some(uri) => trash_scope(&uri, &folder),
            None => folder,
        };
        let support = self.imp().file_operations.borrow().trash_support.get(&scope);
        delete_command_label(support)
    }

    /// Asks once whether the folder shown has a Trash, and relabels
    /// Delete when the answer comes (`refreshTrashSupport`).
    pub(crate) fn learn_trash_support(&self) {
        let Some(folder) = self.current_uri().filter(|uri| Page::from_uri(uri).is_none()) else {
            return;
        };
        {
            let mut operations = self.imp().file_operations.borrow_mut();
            let support = &mut operations.trash_support;
            if support.known.contains_key(&folder) || !support.asking.insert(folder.clone()) {
                return;
            }
        }
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let answer = trash_support(&folder, &Cancellation::new()).await;
                window.remember_trash_support(folder, answer.ok());
            }
        ));
    }

    /// Records the answer about `folder`; a failed question is asked again
    /// the next time the folder is shown.
    fn remember_trash_support(&self, folder: String, answer: Option<bool>) {
        {
            let mut operations = self.imp().file_operations.borrow_mut();
            let support = &mut operations.trash_support;
            support.asking.remove(&folder);
            let Some(has_trash) = answer else {
                return;
            };
            support.known.insert(folder, has_trash);
        }
        self.update_file_commands();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_is_decided_by_its_own_folder() {
        assert_eq!(
            trash_scope("smb://nas/share/report.pdf", "file:///home/user"),
            "smb://nas/share"
        );
        assert_eq!(trash_scope("file:///", "file:///home/user"), "file:///home/user");
    }

    /// parity: CMD-003
    #[test]
    fn delete_reads_delete_permanently_only_where_no_trash_is_known() {
        let mut support = TrashSupport::default();
        support.known.insert("smb://nas/share".to_owned(), false);
        support.known.insert("file:///home/user".to_owned(), true);

        assert_eq!(
            delete_command_label(support.get("smb://nas/share")),
            "Delete permanently"
        );
        assert_eq!(
            delete_command_label(support.get("file:///home/user")),
            "Move to Trash"
        );
        assert_eq!(
            delete_command_label(support.get("sftp://host/dir")),
            "Move to Trash"
        );
    }
}
