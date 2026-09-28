// SPDX-License-Identifier: AGPL-3.0-only
//! Deciding what Delete does with each item, and the confirmation that
//! says so (OPS-015, OPS-017, OPS-018, CMD-003).
//!
//! Ports `trash_support` in `desktop/gio_backend.py` and `trashScope`,
//! `trashSupported`, `deleteLabel` and the confirmation of `trash` in
//! `desktop/ui/app.js`. Each item is decided by its own folder: where GIO
//! reports a Trash the item goes there, elsewhere (SMB shares, most remote
//! backends) it is deleted permanently, and the confirmation says which.

use std::collections::hash_map::Entry;
use std::collections::HashMap;

use super::context::{on_worker, unless_cancelled};
use super::error::OpsError;
use crate::gio_node::GioNode;
use crate::location::{normalise, parent_location};
use crate::transfer::{Cancellation, Node};

/// Whether the folder at `folder_uri` has a usable Trash
/// (`access::can-trash`).
///
/// # Errors
///
/// An address that is not a supported location, [`OpsError::NotMounted`]
/// when the share must be mounted first (mount it and ask again), or
/// [`OpsError::Cancelled`]. Every other failure answers `false`, as
/// `trash_support` in `desktop/gio_backend.py` does.
pub async fn trash_support(folder_uri: &str, cancel: &Cancellation) -> Result<bool, OpsError> {
    let folder = GioNode::new(&normalise(folder_uri)?);
    let cancel = cancel.clone();
    on_worker(move || {
        let can_trash = unless_cancelled(&cancel, || folder.can_trash(Some(&cancel)))?;
        Ok(can_trash?)
    })
    .await
}

/// The Delete command's label for an item in a folder whose Trash support
/// is `trash_support` (`None` while unknown): "Delete permanently" only
/// where the folder is known to have no Trash.
pub fn delete_command_label(trash_support: Option<bool>) -> &'static str {
    match trash_support {
        Some(false) => "Delete permanently",
        Some(true) | None => "Move to Trash",
    }
}

/// One selected item, as the confirmation names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteItem {
    /// The item's URI.
    pub uri: String,
    /// The name the listing shows for it.
    pub name: String,
}

/// What Delete does with a selection: the items that go to the Trash and
/// the ones deleted permanently, each run as its own operation, Trash
/// first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletePlan {
    /// Items in folders with a Trash.
    pub to_trash: Vec<String>,
    /// Items in folders without a Trash.
    pub to_delete: Vec<String>,
    /// The selection as the confirmation names it: one item's name or
    /// `N selected items`.
    selection_text: String,
}

/// The confirmation dialog for a [`DeletePlan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteConfirmation {
    /// The dialog title.
    pub title: &'static str,
    /// The message, naming the selection and what happens to it.
    pub body: String,
    /// The label of the red confirm button.
    pub confirm_label: &'static str,
}

/// Decides what Delete does with each of `items`, asking each folder about
/// its Trash once.
///
/// OPS-017: a folder on a share that is not mounted, which GIO cannot be
/// asked about, counts as having a Trash, so that failure never turns into
/// a permanent delete; the Trash attempt then fails visibly instead. Any
/// other failed query answers "no Trash", as `trash_support` in
/// `desktop/gio_backend.py` does, and the confirmation then says that the
/// items are deleted permanently.
///
/// # Errors
///
/// Only [`OpsError::Cancelled`], also when the user cancels during the
/// last folder's query, so a cancelled plan never reaches a confirmation.
pub async fn plan_delete(items: &[DeleteItem], cancel: &Cancellation) -> Result<DeletePlan, OpsError> {
    let items = items.to_vec();
    let cancel = cancel.clone();
    on_worker(move || plan_delete_blocking(&items, &cancel)).await
}

/// [`plan_delete`] on the calling thread.
fn plan_delete_blocking(items: &[DeleteItem], cancel: &Cancellation) -> Result<DeletePlan, OpsError> {
    let mut trash_by_folder: HashMap<String, bool> = HashMap::new();
    let mut plan = DeletePlan {
        to_trash: Vec::new(),
        to_delete: Vec::new(),
        selection_text: selection_text(items),
    };
    for item in items {
        cancel.check()?;
        let has_trash = match trash_by_folder.entry(trash_scope(&item.uri)) {
            Entry::Occupied(known) => *known.get(),
            Entry::Vacant(unknown) => {
                let answer = folder_has_trash(unknown.key(), cancel)?;
                *unknown.insert(answer)
            }
        };
        if has_trash {
            plan.to_trash.push(item.uri.clone());
        } else {
            plan.to_delete.push(item.uri.clone());
        }
    }
    Ok(plan)
}

/// The folder whose Trash an item goes to: its parent, or the item itself
/// when it has none (`trashScope` in `app.js`).
fn trash_scope(uri: &str) -> String {
    parent_location(uri).unwrap_or_else(|| uri.to_owned())
}

/// Whether items in the folder at `folder_uri` go to the Trash, by the
/// rules of [`plan_delete`].
///
/// # Errors
///
/// [`OpsError::Cancelled`]: a query the user cancelled answers "no Trash",
/// which must never plan a permanent delete.
fn folder_has_trash(folder_uri: &str, cancel: &Cancellation) -> Result<bool, OpsError> {
    let can_trash = unless_cancelled(cancel, || GioNode::new(folder_uri).can_trash(Some(cancel)))?;
    // OPS-017: the only failure `can_trash` reports is a share that is not
    // mounted, and it counts as having a Trash.
    Ok(can_trash.unwrap_or(true))
}

/// One item's name, or `N selected items`.
fn selection_text(items: &[DeleteItem]) -> String {
    match items {
        [only] => only.name.clone(),
        _ => format!("{} selected items", items.len()),
    }
}

impl DeletePlan {
    /// The confirmation for this plan, word for word as `app.js` asks it.
    pub fn confirmation(&self) -> DeleteConfirmation {
        let what = &self.selection_text;
        let trash_count = self.to_trash.len();
        let delete_count = self.to_delete.len();
        if trash_count > 0 && delete_count > 0 {
            return DeleteConfirmation {
                title: "Delete items?",
                body: format!(
                    "{what}\n\n{trash_count} item(s) go to the Trash and can be restored from there.\n\
                     {delete_count} item(s) are on a location without Trash and are deleted permanently, \
                     without any way to recover them."
                ),
                confirm_label: "Delete items",
            };
        }
        if delete_count > 0 {
            return DeleteConfirmation {
                title: "Delete permanently?",
                body: format!(
                    "{what}\n\nThis location has no Trash. The items are deleted permanently and cannot be \
                     recovered."
                ),
                confirm_label: "Delete permanently",
            };
        }
        DeleteConfirmation {
            title: "Move to Trash?",
            body: format!("{what}\n\nItems go to the Trash and can be restored from there."),
            confirm_label: "Move to Trash",
        }
    }
}

#[cfg(test)]
mod tests {
    use gio::prelude::*;

    use super::*;

    fn plan(to_trash: &[&str], to_delete: &[&str], selection_text: &str) -> DeletePlan {
        DeletePlan {
            to_trash: to_trash.iter().map(ToString::to_string).collect(),
            to_delete: to_delete.iter().map(ToString::to_string).collect(),
            selection_text: selection_text.to_owned(),
        }
    }

    /// One plan and the confirmation `app.js` shows for it.
    struct ConfirmationCase {
        plan: DeletePlan,
        title: &'static str,
        body: &'static str,
        confirm_label: &'static str,
    }

    #[test]
    fn each_mix_of_trash_and_permanent_delete_has_its_confirmation() {
        let cases = [
            ConfirmationCase {
                plan: plan(&["file:///a"], &[], "a"),
                title: "Move to Trash?",
                body: "a\n\nItems go to the Trash and can be restored from there.",
                confirm_label: "Move to Trash",
            },
            ConfirmationCase {
                plan: plan(&[], &["smb://nas/s/a", "smb://nas/s/b"], "2 selected items"),
                title: "Delete permanently?",
                body: "2 selected items\n\nThis location has no Trash. The items are deleted permanently \
                       and cannot be recovered.",
                confirm_label: "Delete permanently",
            },
            ConfirmationCase {
                plan: plan(&["file:///a"], &["smb://nas/s/b"], "2 selected items"),
                title: "Delete items?",
                body: "2 selected items\n\n1 item(s) go to the Trash and can be restored from there.\n\
                       1 item(s) are on a location without Trash and are deleted permanently, without any \
                       way to recover them.",
                confirm_label: "Delete items",
            },
        ];
        for case in cases {
            let confirmation = case.plan.confirmation();

            assert_eq!(confirmation.title, case.title);
            assert_eq!(confirmation.body, case.body);
            assert_eq!(confirmation.confirm_label, case.confirm_label);
        }
    }

    #[test]
    fn one_item_is_named_and_several_are_counted() {
        let one = [DeleteItem {
            uri: "file:///tmp/report.pdf".into(),
            name: "report.pdf".into(),
        }];
        let two = [one[0].clone(), one[0].clone()];

        assert_eq!(selection_text(&one), "report.pdf");
        assert_eq!(selection_text(&two), "2 selected items");
    }

    #[test]
    fn delete_says_permanently_only_where_no_trash_is_known() {
        assert_eq!(delete_command_label(Some(false)), "Delete permanently");
        assert_eq!(delete_command_label(Some(true)), "Move to Trash");
        assert_eq!(delete_command_label(None), "Move to Trash");
    }

    #[test]
    fn a_cancelled_trash_query_is_a_cancellation_not_a_permanent_delete() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let folder_uri = gio::File::for_path(temp.path()).uri().to_string();
        let cancelled = Cancellation::new();
        cancelled.cancel();

        let answer = folder_has_trash(&folder_uri, &cancelled);

        assert_eq!(answer, Err(OpsError::Cancelled));
    }

    #[test]
    fn an_item_uses_the_trash_of_its_own_folder() {
        assert_eq!(trash_scope("file:///home/user/a.txt"), "file:///home/user");
        assert_eq!(trash_scope("smb://nas/share/a"), "smb://nas/share");
        assert_eq!(trash_scope("file:///"), "file:///");
    }
}
