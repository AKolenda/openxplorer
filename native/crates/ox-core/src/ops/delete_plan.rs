// SPDX-License-Identifier: AGPL-3.0-only
//! Deciding what Delete does with each item, and the confirmation that
//! says so (OPS-015, OPS-017, OPS-018, CMD-003).
//!
//! Ports `trash_support` in `v2.0.0:desktop/gio_backend.py` and `trashScope`,
//! `trashSupported`, `deleteLabel` and the confirmation of `trash` in
//! `v2.0.0:desktop/ui/app.js`. Each item is decided by what GIO reports for the
//! item itself (`access::can-trash`): where it can go to the Trash it goes
//! there, elsewhere (SMB shares, most remote backends, tmpfs) it is deleted
//! permanently, and the confirmation says which. GIO answers for an item
//! from the folder that holds it, so the folder is never asked about
//! itself: that answer is about the folder's own parent, and a folder
//! directly in the home folder, whose parent `/home` only root may change,
//! would wrongly count as having no Trash.

use std::collections::HashMap;

use gio::prelude::*;

use super::context::{on_worker, unless_cancelled};
use super::error::OpsError;
use crate::gio_node::GioNode;
use crate::location::{normalise, parent_location};
use crate::transfer::{Cancellation, Node, TransferError};

/// Whether items in the folder at `folder_uri` go to the Trash: what GIO
/// reports for the first item listed in it (`access::can-trash`). An
/// empty folder has nothing to ask about and answers `None`, so the
/// question is asked again once it holds items.
///
/// # Errors
///
/// An address that is not a supported location, [`OpsError::NotMounted`]
/// when the share must be mounted first (mount it and ask again), or
/// [`OpsError::Cancelled`]. Every other failure answers `false`, as
/// `trash_support` in `v2.0.0:desktop/gio_backend.py` does.
pub async fn trash_support(folder_uri: &str, cancel: &Cancellation) -> Result<Option<bool>, OpsError> {
    let folder = GioNode::new(&normalise(folder_uri)?);
    let cancel = cancel.clone();
    on_worker(move || {
        let first_item = unless_cancelled(&cancel, || first_item_in(&folder.uri(), &cancel))?;
        let Some(item) = first_item? else {
            return Ok(None);
        };
        let can_trash = unless_cancelled(&cancel, || item.can_trash(Some(&cancel)))?;
        Ok(Some(can_trash?))
    })
    .await
}

/// The first item listed in the folder at `folder_uri`, `None` when it is
/// empty or cannot be listed.
///
/// # Errors
///
/// [`TransferError::NotMounted`] when the share must be mounted first.
fn first_item_in(folder_uri: &str, cancel: &Cancellation) -> Result<Option<GioNode>, TransferError> {
    let folder = gio::File::for_uri(folder_uri);
    let cancellable = Some(cancel.cancellable());
    let listed = folder.enumerate_children(
        "standard::name",
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        cancellable,
    );
    let children = match listed {
        Ok(children) => children,
        Err(error) if error.matches(gio::IOErrorEnum::NotMounted) => return Err(error.into()),
        Err(_) => return Ok(None),
    };
    let first = children.next_file(cancellable).ok().flatten();
    // Closing only releases the listing; nothing is lost when it fails.
    let _ = children.close(cancellable);
    Ok(first.map(|info| GioNode::from_file(folder.child(info.name()))))
}

/// The Delete command's label for an item in a folder whose Trash support
/// is `trash_support` (`None` while unknown): "Delete permanently" only
/// where the folder is known to have no Trash.
pub fn delete_command_label(trash_support: Option<bool>) -> &'static str {
    match trash_support {
        Some(false) => crate::i18n::gettext_static("Delete permanently"),
        Some(true) | None => crate::i18n::gettext_static("Move to Trash"),
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

/// Decides what Delete does with each of `items`, asking GIO about each
/// item (`access::can-trash`).
///
/// GIO's answer comes from the folder that holds the item, so an item
/// that can go to the Trash answers for the rest of its folder, which is
/// then not asked again. An item that cannot answers only for itself in
/// a local folder: it may be one that vanished since the listing, or
/// another user's file in a shared folder such as `/tmp`. On a remote
/// location the first answer stands for the folder either way, so a large
/// selection on a share costs one query, as before.
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
    let mut trash_by_folder: HashMap<String, bool> = HashMap::new();
    let mut plan = DeletePlan {
        to_trash: Vec::new(),
        to_delete: Vec::new(),
        selection_text: selection_text(items),
    };
    for item in items {
        cancel.check()?;
        let folder = trash_scope(&item.uri);
        let has_trash = if let Some(&known) = trash_by_folder.get(&folder) {
            known
        } else {
            let answer = item_has_trash(&item.uri, cancel)?;
            if answer || !is_local(&folder) {
                trash_by_folder.insert(folder, answer);
            }
            answer
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

/// True for a `file:` location.
fn is_local(uri: &str) -> bool {
    gio::File::for_uri(uri).has_uri_scheme("file")
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
    fn delete_says_permanently_only_where_no_trash_is_known() {
        assert_eq!(delete_command_label(Some(false)), "Delete permanently");
        assert_eq!(delete_command_label(Some(true)), "Move to Trash");
        assert_eq!(delete_command_label(None), "Move to Trash");
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

    #[test]
    fn an_item_uses_the_trash_of_its_own_folder() {
        assert_eq!(trash_scope("file:///home/user/a.txt"), "file:///home/user");
        assert_eq!(trash_scope("smb://nas/share/a"), "smb://nas/share");
        assert_eq!(trash_scope("file:///"), "file:///");
    }
}
