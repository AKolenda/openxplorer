// SPDX-License-Identifier: AGPL-3.0-only
//! The folder tree's model: one root folder whose subfolders a
//! `GtkTreeListModel` lists level by level.
//!
//! A folder's subfolders are a monitored `GtkDirectoryList`, filtered to
//! folders and sorted by name as the details view sorts them. The list
//! is created when GTK asks whether the row can expand, but it reads the
//! folder only once the row is expanded ([`load_children`]), so the tree
//! never lists folders nobody opened (Dolphin's Folders panel lists a
//! folder when it is expanded too).

use std::cmp::Ordering;

use gtk::prelude::*;
use gtk::{gio, glib};

use crate::folder_view::sorting::{compare_names, SortKey, SortName};

/// The attribute of a `GFileInfo` in the tree that holds its `GFile`, as
/// `GtkDirectoryList` sets it.
const FILE_ATTRIBUTE: &str = "standard::file";

/// What the tree reads of each subfolder: enough to filter and name it.
const ATTRIBUTES: &str = "standard::name,standard::display-name,standard::type,standard::is-hidden";

/// The folder a row of the tree shows.
pub(super) fn row_file(row: &gtk::TreeListRow) -> Option<gio::File> {
    let info = row.item().and_downcast::<gio::FileInfo>()?;
    info.attribute_object(FILE_ATTRIBUTE).and_downcast::<gio::File>()
}

/// The name a row of the tree shows.
pub(super) fn row_name(row: &gtk::TreeListRow) -> String {
    row.item()
        .and_downcast::<gio::FileInfo>()
        .map(|info| info.display_name().to_string())
        .unwrap_or_default()
}

/// The tree with `root`, named `title`, as its one top row; its
/// subfolders include hidden ones when `show_hidden`.
pub(super) fn tree_model(root: &gio::File, title: &str, show_hidden: bool) -> gtk::TreeListModel {
    let info = gio::FileInfo::new();
    info.set_display_name(title);
    info.set_file_type(gio::FileType::Directory);
    info.set_attribute_object(FILE_ATTRIBUTE, root);
    let top = gio::ListStore::new::<gio::FileInfo>();
    top.append(&info);
    gtk::TreeListModel::new(top, false, false, move |item| {
        item.is::<gio::FileInfo>()
            .then(|| subfolders(show_hidden).upcast())
    })
}

/// The sorted subfolders of a folder, empty until [`load_children`]
/// gives them the folder's listing.
fn subfolders(show_hidden: bool) -> gtk::SortListModel {
    let filter = gtk::CustomFilter::new(move |item| {
        item.downcast_ref::<gio::FileInfo>().is_some_and(|info| {
            info.file_type() == gio::FileType::Directory && (show_hidden || !info.is_hidden())
        })
    });
    let folders = gtk::FilterListModel::new(None::<gio::ListModel>, Some(filter));
    let sorter = gtk::CustomSorter::new(|left, right| by_name(left, right).into());
    gtk::SortListModel::new(Some(folders), Some(sorter))
}

/// Starts reading the subfolders of the expanded `row`, once, and returns
/// the listing. GTK creates a row's subfolder model just to learn that
/// the row can expand, so reading starts only here: otherwise every
/// folder on screen would be read.
pub(super) fn load_children(row: &gtk::TreeListRow) -> Option<gtk::DirectoryList> {
    let sorted = row.children().and_downcast::<gtk::SortListModel>()?;
    let filtered = sorted.model().and_downcast::<gtk::FilterListModel>()?;
    if let Some(listing) = filtered.model().and_downcast::<gtk::DirectoryList>() {
        return Some(listing);
    }
    let listing = gtk::DirectoryList::new(Some(ATTRIBUTES), Some(&row_file(row)?));
    listing.set_monitored(true);
    filtered.set_model(Some(&listing));
    Some(listing)
}

/// Waits until `listing` has read its folder.
pub(super) async fn loaded(listing: &gtk::DirectoryList) {
    if !listing.is_loading() {
        return;
    }
    let (sender, receiver) = async_channel::bounded::<()>(1);
    let handler = listing.connect_loading_notify(move |listing| {
        if !listing.is_loading() {
            let _ = sender.try_send(());
        }
    });
    let _ = receiver.recv().await;
    listing.disconnect(handler);
}

/// Natural order of two folders by their shown names.
fn by_name(left: &glib::Object, right: &glib::Object) -> Ordering {
    let name = |item: &glib::Object| {
        item.downcast_ref::<gio::FileInfo>()
            .map(|info| info.display_name().to_string())
            .unwrap_or_default()
    };
    let (left, right) = (name(left), name(right));
    let (left_key, right_key) = (SortKey::new(&left), SortKey::new(&right));
    compare_names(
        SortName {
            key: &left_key,
            name: &left,
        },
        SortName {
            key: &right_key,
            name: &right,
        },
    )
}
