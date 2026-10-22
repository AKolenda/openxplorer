// SPDX-License-Identifier: AGPL-3.0-only
//! Deciding what Delete does with each item, and the confirmation that
//! says so (OPS-015, OPS-017, OPS-018).
//!
//! Ports `trash_support` in `v2.0.0:desktop/gio_backend.py` and `trashScope`,
//! `trashSupported` and the confirmation of `trash` in
//! `v2.0.0:desktop/ui/app.js`. Each item is decided by what GIO reports for the
//! item itself (`access::can-trash`): where it can go to the Trash it goes
//! there, elsewhere (SMB shares, most remote backends, tmpfs) it is deleted
//! permanently, and the confirmation says which. GIO answers for an item
//! from the folder that holds it, so the folder is never asked about
//! itself: that answer is about the folder's own parent, and a folder
//! directly in the home folder, whose parent `/home` only root may change,
//! would wrongly count as having no Trash.

use std::collections::{HashMap, HashSet};

use super::context::{on_worker, unless_cancelled};
use super::error::OpsError;
use crate::gio_node::GioNode;
use crate::location::parent_location;
use crate::transfer::{Cancellation, Node};

/// One selected item, as the confirmation names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteItem {
    /// The item's URI.
    pub uri: String,
    /// The name the listing shows for it.
    pub name: String,
    /// `access::can-trash` as the folder's listing reported it; `None`
    /// when the item was not listed (a drop, the folder tree) or the
    /// backend does not report it.
    pub can_trash: Option<bool>,
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

/// Decides what Delete does with each of `items`, by `access::can-trash`
/// of each item.
///
/// A listed item carries the answer its folder's listing reported
/// ([`DeleteItem::can_trash`]), which decides for it alone with no query,
/// so another user's file in `/tmp` or one whose query failed never
/// decides for its siblings. An item without a listed answer (a drop, the
/// folder tree, a backend that does not report it) is asked about once
/// per folder: an item that can go to the Trash, listed or asked, answers
/// for the folder's other unlisted items, and so does the first asked
/// item's "no Trash", so a large selection on a share without a Trash
/// costs at most one query per folder.
///
/// OPS-017: an item on a share that is not mounted, which GIO cannot be
/// asked about, counts as going to the Trash, so that failure never turns
/// into a permanent delete; the Trash attempt then fails visibly instead.
/// Any other failed query answers "no Trash", as `trash_support` in
/// `v2.0.0:desktop/gio_backend.py` does, and the confirmation then says that the
/// item is deleted permanently.
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
    plan_with(items, cancel, |uri| item_has_trash(uri, cancel))
}

/// [`plan_delete`] with `has_trash` answering for each item it asks
/// about.
fn plan_with(
    items: &[DeleteItem],
    cancel: &Cancellation,
    mut has_trash: impl FnMut(&str) -> Result<bool, OpsError>,
) -> Result<DeletePlan, OpsError> {
    // What GIO answered for the first unlisted item asked about in each
    // folder, and the folders where a listed item can go to the Trash.
    let mut asked_in_folder: HashMap<String, bool> = HashMap::new();
    let mut folders_with_trash: HashSet<String> = HashSet::new();
    let mut plan = DeletePlan {
        to_trash: Vec::new(),
        to_delete: Vec::new(),
        selection_text: selection_text(items),
    };
    for item in items {
        cancel.check()?;
        let folder = trash_scope(&item.uri);
        let goes_to_trash = match item.can_trash {
            Some(listed) => listed,
            None if folders_with_trash.contains(&folder) => true,
            None => {
                if let Some(&answer) = asked_in_folder.get(&folder) {
                    answer
                } else {
                    let answer = has_trash(&item.uri)?;
                    asked_in_folder.insert(folder.clone(), answer);
                    answer
                }
            }
        };
        if goes_to_trash {
            folders_with_trash.insert(folder);
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

/// Whether the item at `item_uri` goes to the Trash, by the rules of
/// [`plan_delete`].
///
/// # Errors
///
/// [`OpsError::Cancelled`]: a query the user cancelled answers "no Trash",
/// which must never plan a permanent delete.
fn item_has_trash(item_uri: &str, cancel: &Cancellation) -> Result<bool, OpsError> {
    let can_trash = unless_cancelled(cancel, || GioNode::new(item_uri).can_trash(Some(cancel)))?;
    // OPS-017: the only failure `can_trash` reports is a share that is not
    // mounted, and it counts as having a Trash.
    Ok(can_trash.unwrap_or(true))
}

/// The confirmation of Shift+Delete, which deletes `items` permanently
/// even where a Trash exists (OPS-016). It names the item, or counts
/// several, as the Delete confirmation does.
pub fn permanent_delete_confirmation(items: &[DeleteItem]) -> DeleteConfirmation {
    let what = selection_text(items);
    DeleteConfirmation {
        title: crate::i18n::gettext_static("Delete permanently?"),
        body: crate::i18n::format_message(
            "{what}\n\nThe items are deleted permanently, without the Trash, and cannot be recovered.",
            &[("what", &what)],
        ),
        confirm_label: crate::i18n::gettext_static("Delete permanently"),
    }
}

/// One item's name, or `N selected items`.
fn selection_text(items: &[DeleteItem]) -> String {
    match items {
        [only] => only.name.clone(),
        _ => crate::i18n::format_message("{count} selected items", &[("count", &(items.len()).to_string())]),
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
                title: crate::i18n::gettext_static("Delete items?"),
                body: crate::i18n::format_message("{what}\n\n{trash_count} item(s) go to the Trash and can be restored from there.\n{delete_count} item(s) are on a location without Trash and are deleted permanently, without any way to recover them.", &[("what", what), ("trash_count", &(trash_count).to_string()), ("delete_count", &(delete_count).to_string())]),
                confirm_label: crate::i18n::gettext_static("Delete items"),
            };
        }
        if delete_count > 0 {
            return DeleteConfirmation {
                title: crate::i18n::gettext_static("Delete permanently?"),
                body: crate::i18n::format_message("{what}\n\nThis location has no Trash. The items are deleted permanently and cannot be recovered.", &[("what", what)]),
                confirm_label: crate::i18n::gettext_static("Delete permanently"),
            };
        }
        DeleteConfirmation {
            title: crate::i18n::gettext_static("Move to Trash?"),
            body: crate::i18n::format_message(
                "{what}\n\nItems go to the Trash and can be restored from there.",
                &[("what", what)],
            ),
            confirm_label: crate::i18n::gettext_static("Move to Trash"),
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
            can_trash: None,
        }];
        let two = [one[0].clone(), one[0].clone()];

        assert_eq!(selection_text(&one), "report.pdf");
        assert_eq!(selection_text(&two), "2 selected items");
    }

    /// parity: OPS-016
    #[test]
    fn shift_delete_asks_to_delete_the_named_items_permanently() {
        let one = [DeleteItem {
            uri: "file:///tmp/report.pdf".into(),
            name: "report.pdf".into(),
            can_trash: None,
        }];

        let confirmation = permanent_delete_confirmation(&one);

        assert_eq!(confirmation.title, "Delete permanently?");
        assert_eq!(
            confirmation.body,
            "report.pdf\n\nThe items are deleted permanently, without the Trash, and cannot be recovered."
        );
        assert_eq!(confirmation.confirm_label, "Delete permanently");
    }

    #[test]
    fn a_cancelled_trash_query_is_a_cancellation_not_a_permanent_delete() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let item_uri = gio::File::for_path(temp.path()).uri().to_string();
        let cancelled = Cancellation::new();
        cancelled.cancel();

        let answer = item_has_trash(&item_uri, &cancelled);

        assert_eq!(answer, Err(OpsError::Cancelled));
    }

    fn items(uris: &[&str]) -> Vec<DeleteItem> {
        uris.iter()
            .map(|uri| DeleteItem {
                uri: (*uri).to_owned(),
                name: (*uri).to_owned(),
                can_trash: None,
            })
            .collect()
    }

    fn listed(uri: &str, can_trash: bool) -> DeleteItem {
        DeleteItem {
            uri: uri.to_owned(),
            name: uri.to_owned(),
            can_trash: Some(can_trash),
        }
    }

    /// A listed item's own `access::can-trash` decides for it, with no
    /// query: a remote item without a Trash, such as one whose query
    /// failed, does not decide for its siblings, and the one that can goes
    /// to the Trash.
    #[test]
    fn a_remote_item_without_a_trash_does_not_decide_for_its_folder() {
        let selection = [
            listed("sftp://host/dir/gone.txt", false),
            listed("sftp://host/dir/kept.txt", true),
            listed("sftp://host/dir/other.txt", false),
        ];
        let mut asked = Vec::new();

        let plan = plan_with(&selection, &Cancellation::new(), |uri| {
            asked.push(uri.to_owned());
            Ok(true)
        })
        .unwrap();

        assert!(asked.is_empty(), "{asked:?}");
        assert_eq!(plan.to_trash, ["sftp://host/dir/kept.txt"]);
        assert_eq!(
            plan.to_delete,
            ["sftp://host/dir/gone.txt", "sftp://host/dir/other.txt"]
        );
    }

    /// Items with no listed answer on a share without a Trash cost one
    /// query per folder, not one per item.
    #[test]
    fn unlisted_items_without_a_trash_are_asked_about_once_per_folder() {
        let selection = items(&[
            "smb://nas/share/a",
            "smb://nas/share/b",
            "smb://nas/share/c",
            "smb://nas/other/d",
        ]);
        let mut asked = Vec::new();

        let plan = plan_with(&selection, &Cancellation::new(), |uri| {
            asked.push(uri.to_owned());
            Ok(false)
        })
        .unwrap();

        assert_eq!(asked, ["smb://nas/share/a", "smb://nas/other/d"]);
        assert!(plan.to_trash.is_empty());
        assert_eq!(plan.to_delete.len(), 4);
    }

    /// An item that can go to the Trash answers for the rest of its
    /// folder, so a large selection asks once per folder.
    #[test]
    fn an_item_with_a_trash_answers_for_its_folder() {
        let selection = items(&[
            "sftp://host/dir/a",
            "sftp://host/dir/b",
            "file:///tmp/c",
            "file:///tmp/d",
        ]);
        let mut asked = Vec::new();

        let plan = plan_with(&selection, &Cancellation::new(), |uri| {
            asked.push(uri.to_owned());
            Ok(true)
        })
        .unwrap();

        assert_eq!(asked, ["sftp://host/dir/a", "file:///tmp/c"]);
        assert_eq!(plan.to_trash.len(), 4);
    }

    #[test]
    fn an_item_uses_the_trash_of_its_own_folder() {
        assert_eq!(trash_scope("file:///home/user/a.txt"), "file:///home/user");
        assert_eq!(trash_scope("smb://nas/share/a"), "smb://nas/share");
        assert_eq!(trash_scope("file:///"), "file:///");
    }
}
