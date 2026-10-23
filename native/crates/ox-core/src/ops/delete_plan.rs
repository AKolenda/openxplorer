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

use gio::prelude::*;

use super::context::{on_worker, unless_cancelled};
use super::error::OpsError;
use crate::gio_node::GioNode;
use crate::location::parent_location;
use crate::transfer::{Cancellation, Node};

/// The unlisted items of one folder, still to be decided once the folder
/// has answered "no Trash", from which the planner reads their answers
/// from one listing of the folder instead of asking about each. Below it,
/// asking each item costs a few queries, while the folder may hold many
/// thousands of items.
const LISTING_MIN_ITEMS: usize = 32;

/// The children a listing reads per selected item before it stops, so a
/// selection in a huge folder never reads all of it; the selected items it
/// did not reach are asked about one by one.
const CHILDREN_READ_PER_ITEM: usize = 64;

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
/// ([`DeleteItem::can_trash`]), which decides for it alone with no query.
/// An item without a listed answer (a drop, the folder tree, a search
/// result) is decided by an item of its folder that can go to the Trash,
/// listed or asked; failing that, by its own answer. "No Trash" always
/// decides for its item alone, so another user's file in `/tmp`, or one
/// that vanished before its query, never sends a sibling that can go to
/// the Trash to a permanent delete. Once a folder has answered "no Trash"
/// for one unlisted item, its other unlisted items are asked about one by
/// one; when there are at least [`LISTING_MIN_ITEMS`] of them, they are
/// first read from one listing of the folder, bounded by
/// [`CHILDREN_READ_PER_ITEM`], so a large selection on a share without a
/// Trash does not cost a query per item.
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
    plan_with(
        items,
        cancel,
        |uri| item_has_trash(uri, cancel),
        |folder, wanted| children_have_trash(folder, wanted, cancel),
    )
}

/// [`plan_delete`] with `has_trash` answering for each item it asks
/// about, and `children_have_trash` for the wanted children of a folder
/// at once.
fn plan_with(
    items: &[DeleteItem],
    cancel: &Cancellation,
    mut has_trash: impl FnMut(&str) -> Result<bool, OpsError>,
    mut children_have_trash: impl FnMut(&str, &HashSet<String>) -> Result<HashMap<String, bool>, OpsError>,
) -> Result<DeletePlan, OpsError> {
    let folders: Vec<String> = items.iter().map(|item| trash_scope(&item.uri)).collect();
    // The folders where an item can go to the Trash, the folders where an
    // unlisted item was asked about and answered "no Trash", the folders
    // considered for a listing, and the answers listings gave, by item.
    let mut folders_with_trash: HashSet<String> = HashSet::new();
    let mut asked_in_folder: HashSet<String> = HashSet::new();
    let mut listed_folders: HashSet<String> = HashSet::new();
    let mut listed_answers: HashMap<String, bool> = HashMap::new();
    let mut plan = DeletePlan {
        to_trash: Vec::new(),
        to_delete: Vec::new(),
        selection_text: selection_text(items),
    };
    for (index, (item, folder)) in items.iter().zip(&folders).enumerate() {
        cancel.check()?;
        let goes_to_trash = match item.can_trash {
            Some(listed) => listed,
            None if folders_with_trash.contains(folder) => true,
            None => {
                if asked_in_folder.contains(folder) && listed_folders.insert(folder.clone()) {
                    let wanted: HashSet<String> = items[index..]
                        .iter()
                        .zip(&folders[index..])
                        .filter(|(other, other_folder)| other.can_trash.is_none() && *other_folder == folder)
                        .map(|(other, _)| other.uri.clone())
                        .collect();
                    if wanted.len() >= LISTING_MIN_ITEMS {
                        listed_answers.extend(children_have_trash(folder, &wanted)?);
                    }
                }
                // An item the listing did not answer for, or one of a
                // folder that was not listed, is asked about itself.
                let answer = match listed_answers.get(&item.uri) {
                    Some(&answer) => answer,
                    None => has_trash(&item.uri)?,
                };
                asked_in_folder.insert(folder.clone());
                answer
            }
        };
        if goes_to_trash {
            folders_with_trash.insert(folder.clone());
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

/// `access::can-trash` of the `wanted` children of the folder at
/// `folder_uri`, by URI, from one listing read by [`selected_answers`]. A
/// folder that cannot be listed answers for none of them, so each is asked
/// about itself.
///
/// # Errors
///
/// [`OpsError::Cancelled`], also when the user cancels during the listing.
fn children_have_trash(
    folder_uri: &str,
    wanted: &HashSet<String>,
    cancel: &Cancellation,
) -> Result<HashMap<String, bool>, OpsError> {
    let folder = gio::File::for_uri(folder_uri);
    let listing = unless_cancelled(cancel, || {
        folder.enumerate_children(
            "standard::name,access::can-trash",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            Some(cancel.cancellable()),
        )
    })?;
    let Ok(listing) = listing else {
        return Ok(HashMap::new());
    };
    let answers = selected_answers(wanted, cancel, || {
        match listing.next_file(Some(cancel.cancellable())) {
            Ok(Some(info)) => {
                let answer = info
                    .has_attribute("access::can-trash")
                    .then(|| info.boolean("access::can-trash"));
                Some((listing.child(&info).uri().to_string(), answer))
            }
            // What the listing read before it failed stays valid.
            Ok(None) | Err(_) => None,
        }
    });
    // Closing only releases the listing; what was read stays valid.
    let _ = listing.close(gio::Cancellable::NONE);
    answers
}

/// The answers for the `wanted` children among those `next_child` reads,
/// each a URI and its answer, `None` where the backend reports none. It
/// stops once every wanted child has its answer, at the end of the listing,
/// or after [`CHILDREN_READ_PER_ITEM`] children per wanted child, and keeps
/// no answer for a child that was not wanted.
///
/// # Errors
///
/// [`OpsError::Cancelled`], also when the user cancels during the listing.
fn selected_answers(
    wanted: &HashSet<String>,
    cancel: &Cancellation,
    mut next_child: impl FnMut() -> Option<(String, Option<bool>)>,
) -> Result<HashMap<String, bool>, OpsError> {
    let mut answers = HashMap::new();
    for _ in 0..wanted.len().saturating_mul(CHILDREN_READ_PER_ITEM) {
        cancel.check()?;
        let Some((child, answer)) = next_child() else {
            break;
        };
        if let Some(answer) = answer.filter(|_| wanted.contains(&child)) {
            answers.insert(child, answer);
            if answers.len() == wanted.len() {
                break;
            }
        }
    }
    cancel.check()?;
    Ok(answers)
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

        let plan = plan_with(
            &selection,
            &Cancellation::new(),
            |uri| {
                asked.push(uri.to_owned());
                Ok(true)
            },
            |folder, _| panic!("{folder} was listed"),
        )
        .unwrap();

        assert!(asked.is_empty(), "{asked:?}");
        assert_eq!(plan.to_trash, ["sftp://host/dir/kept.txt"]);
        assert_eq!(
            plan.to_delete,
            ["sftp://host/dir/gone.txt", "sftp://host/dir/other.txt"]
        );
    }

    /// `count` unlisted items named `0`, `1`, … in `folder`.
    fn numbered(folder: &str, count: usize) -> Vec<DeleteItem> {
        let uris: Vec<String> = (0..count).map(|number| format!("{folder}/{number}")).collect();
        items(&uris.iter().map(String::as_str).collect::<Vec<_>>())
    }

    /// A few unlisted items on a share without a Trash are asked about one
    /// by one, so the folder, which may be huge, is never listed for them.
    #[test]
    fn a_few_unlisted_items_without_a_trash_are_asked_about_without_a_listing() {
        let selection = numbered("smb://nas/share", LISTING_MIN_ITEMS);
        let mut asked = Vec::new();

        let plan = plan_with(
            &selection,
            &Cancellation::new(),
            |uri| {
                asked.push(uri.to_owned());
                Ok(false)
            },
            |folder, _| panic!("{folder} was listed"),
        )
        .unwrap();

        assert_eq!(asked.len(), LISTING_MIN_ITEMS);
        assert_eq!(plan.to_delete.len(), LISTING_MIN_ITEMS);
    }

    /// Many unlisted items on a share without a Trash cost one query and
    /// one listing for the items still to be decided, not a query each.
    #[test]
    fn many_unlisted_items_without_a_trash_are_read_from_one_listing() {
        let mut selection = numbered("smb://nas/share", LISTING_MIN_ITEMS + 1);
        selection.extend(items(&["smb://nas/other/d"]));
        let mut asked = Vec::new();
        let mut listed = Vec::new();

        let plan = plan_with(
            &selection,
            &Cancellation::new(),
            |uri| {
                asked.push(uri.to_owned());
                Ok(false)
            },
            |folder, wanted| {
                listed.push((folder.to_owned(), wanted.clone()));
                Ok(wanted.iter().map(|uri| (uri.clone(), false)).collect())
            },
        )
        .unwrap();

        assert_eq!(asked, ["smb://nas/share/0", "smb://nas/other/d"]);
        let expected: HashSet<String> = selection[1..=LISTING_MIN_ITEMS]
            .iter()
            .map(|item| item.uri.clone())
            .collect();
        assert_eq!(listed, [("smb://nas/share".to_owned(), expected)]);
        assert!(plan.to_trash.is_empty());
        assert_eq!(plan.to_delete.len(), LISTING_MIN_ITEMS + 2);
    }

    /// An unlisted item that vanished answers "no Trash" for itself alone:
    /// its sibling is asked about and goes to the Trash.
    #[test]
    fn a_vanished_unlisted_item_does_not_decide_for_its_folder() {
        let selection = items(&["sftp://host/dir/gone.txt", "sftp://host/dir/kept.txt"]);
        let mut asked = Vec::new();

        let plan = plan_with(
            &selection,
            &Cancellation::new(),
            |uri| {
                asked.push(uri.to_owned());
                Ok(uri != "sftp://host/dir/gone.txt")
            },
            |folder, _| panic!("{folder} was listed"),
        )
        .unwrap();

        assert_eq!(asked, ["sftp://host/dir/gone.txt", "sftp://host/dir/kept.txt"]);
        assert_eq!(plan.to_trash, ["sftp://host/dir/kept.txt"]);
        assert_eq!(plan.to_delete, ["sftp://host/dir/gone.txt"]);
    }

    /// An item its folder's listing did not answer for is asked about
    /// itself.
    #[test]
    fn an_item_the_listing_does_not_answer_for_is_asked_about() {
        let selection = numbered("sftp://host/dir", LISTING_MIN_ITEMS + 1);
        let unanswered = format!("sftp://host/dir/{LISTING_MIN_ITEMS}");
        let mut asked = Vec::new();

        let plan = plan_with(
            &selection,
            &Cancellation::new(),
            |uri| {
                asked.push(uri.to_owned());
                Ok(uri == unanswered)
            },
            |_, wanted| {
                let answered = wanted.iter().filter(|uri| **uri != unanswered);
                Ok(answered.map(|uri| (uri.clone(), false)).collect())
            },
        )
        .unwrap();

        assert_eq!(asked, ["sftp://host/dir/0".to_owned(), unanswered.clone()]);
        assert_eq!(plan.to_trash, [unanswered]);
        assert_eq!(plan.to_delete.len(), LISTING_MIN_ITEMS);
    }

    /// A listing keeps the answers of the selected children only, and
    /// stops reading once it has all of them.
    #[test]
    fn a_listing_keeps_only_the_selected_children_and_stops_once_it_has_them() {
        let wanted: HashSet<String> = ["smb://nas/share/b", "smb://nas/share/d"]
            .map(str::to_owned)
            .into();
        let mut children = ["a", "b", "c", "d", "e", "f"]
            .map(|name| (format!("smb://nas/share/{name}"), Some(false)))
            .into_iter();
        let mut read = 0;

        let answers = selected_answers(&wanted, &Cancellation::new(), || {
            read += 1;
            children.next()
        })
        .unwrap();

        assert_eq!(answers.keys().cloned().collect::<HashSet<_>>(), wanted);
        assert_eq!(read, 4);
    }

    /// A listing that never reaches a selected child stops after a bounded
    /// number of children, so a huge folder is never read to its end.
    #[test]
    fn a_listing_stops_after_a_bounded_number_of_children() {
        let wanted: HashSet<String> = ["smb://nas/share/gone"].map(str::to_owned).into();
        let mut read = 0;

        let answers = selected_answers(&wanted, &Cancellation::new(), || {
            read += 1;
            Some((format!("smb://nas/share/{read}"), Some(false)))
        })
        .unwrap();

        assert!(answers.is_empty());
        assert_eq!(read, CHILDREN_READ_PER_ITEM);
    }

    /// GIO's listing answers for the selected children it finds, by the
    /// same URIs the selection names, and for no other child.
    #[test]
    fn the_listing_answers_only_for_the_selected_children() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        for name in ["a", "b", "c"] {
            std::fs::write(temp.path().join(name), b"x").expect("a test file");
        }
        let uri = |name: &str| gio::File::for_path(temp.path().join(name)).uri().to_string();
        let folder = gio::File::for_path(temp.path()).uri().to_string();
        let wanted: HashSet<String> = [uri("a"), uri("c"), uri("gone")].into();

        let answers = children_have_trash(&folder, &wanted, &Cancellation::new()).unwrap();

        let answered: HashSet<String> = answers.keys().cloned().collect();
        assert_eq!(answered, [uri("a"), uri("c")].into());
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

        let plan = plan_with(
            &selection,
            &Cancellation::new(),
            |uri| {
                asked.push(uri.to_owned());
                Ok(true)
            },
            |folder, _| panic!("{folder} was listed"),
        )
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
