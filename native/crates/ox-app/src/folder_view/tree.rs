// SPDX-License-Identifier: AGPL-3.0-only
//! Folders that expand in place in the details view (VIEW-035).
//!
//! Ports Dolphin's expandable folders (`ExpandableFolders`, on by default):
//! a folder's arrow lists its contents beneath it, indented, filtered and
//! sorted as the folder's own items are. A [`FolderTree`] puts GTK's tree
//! list model between the sorted items and the selection; it passes the
//! items through, so every position the views and the window use names a
//! [`FileItem`], whether it is a folder's own item or one of a sub-folder.
//! A folder is listed when it is expanded and forgotten when it is
//! collapsed, together with the listings beneath it.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use gtk::{gio, glib};

use crate::folder_view::item::FileItem;
use crate::folder_view::loader::{self, Listing};

/// The tree of expanded folders over the sorted items.
#[derive(Debug, Clone)]
pub(crate) struct FolderTree(Rc<TreeState>);

/// What a [`FolderTree`] keeps; its folders' listings hold it weakly.
#[derive(Debug)]
struct TreeState {
    /// The sorted items with the contents of expanded folders beneath them.
    tree: gtk::TreeListModel,
    /// Whether folders may expand now, which the tree's callback reads.
    expandable: Rc<Cell<bool>>,
    /// The running listings of expanded folders, by location.
    listings: RefCell<HashMap<String, Listing>>,
    /// Folders to expand again once they are listed, by location (Back and
    /// Forward return to them expanded).
    pending: RefCell<HashSet<String>>,
    /// The rows expanded so far; collapsed ones and those inside collapsed
    /// folders are skipped.
    expanded: RefCell<Vec<glib::WeakRef<gtk::TreeListRow>>>,
}

/// The URI of the item `row` shows.
fn row_uri(row: &gtk::TreeListRow) -> Option<String> {
    let item = row.item().and_downcast::<FileItem>()?;
    Some(item.entry().uri.clone())
}

impl FolderTree {
    /// A tree over `sorted`, whose sub-folders' contents are filtered by
    /// `filter` and sorted as `sorted` sorts.
    pub(crate) fn new(sorted: &gtk::SortListModel, filter: &gtk::CustomFilter) -> Self {
        let expandable = Rc::new(Cell::new(false));
        let create = {
            let sorted = sorted.clone();
            let filter = filter.clone();
            move |object: &glib::Object| -> Option<gio::ListModel> {
                let item = object.downcast_ref::<FileItem>()?;
                // GTK caches a None answer permanently. Whether the UI
                // currently allows expansion must not change this answer.
                if !item.entry().is_dir || item.entry().is_virtual {
                    return None;
                }
                Some(child_items(&sorted, &filter).upcast())
            }
        };
        let tree = gtk::TreeListModel::new(sorted.clone(), true, false, create);
        Self(Rc::new(TreeState {
            tree,
            expandable,
            listings: RefCell::default(),
            pending: RefCell::default(),
            expanded: RefCell::default(),
        }))
    }

    /// The model the selection shows.
    pub(crate) fn model(&self) -> &gtk::TreeListModel {
        &self.0.tree
    }

    /// Lets folders expand, or collapses them all and stops them.
    pub(crate) fn set_expandable(&self, expandable: bool) {
        if !expandable {
            self.collapse_all();
        }
        self.0.expandable.set(expandable);
    }

    /// Whether folders may expand now.
    pub(crate) fn is_expandable(&self) -> bool {
        self.0.expandable.get()
    }

    /// The tree row at `position`.
    pub(crate) fn row(&self, position: u32) -> Option<gtk::TreeListRow> {
        self.0.tree.row(position)
    }

    /// Expands or collapses `row`, listing a folder that expands.
    pub(crate) fn set_expanded(&self, row: &gtk::TreeListRow, expanded: bool) {
        if !self.is_expandable() || !row.is_expandable() || row.is_expanded() == expanded {
            return;
        }
        let Some(uri) = row_uri(row) else { return };
        if !expanded {
            self.collapse_branch(row);
            return;
        }
        row.set_expanded(true);
        self.0.expanded.borrow_mut().push(row.downgrade());
        if let Some(children) = row.children() {
            self.list_children(&uri, &children);
        }
    }

    /// Cancels every listing below `row`, deepest first. Cached partial
    /// contents must not make the next expansion look fully listed.
    fn collapse_branch(&self, row: &gtk::TreeListRow) {
        let mut descendants: Vec<gtk::TreeListRow> = self
            .expanded_rows()
            .into_iter()
            .filter(|candidate| {
                candidate == row
                    || std::iter::successors(candidate.parent(), gtk::TreeListRow::parent)
                        .any(|parent| parent == *row)
            })
            .collect();
        descendants.sort_by_key(|child| std::cmp::Reverse(child.depth()));
        for child in descendants {
            if let Some(uri) = row_uri(&child) {
                self.0.listings.borrow_mut().remove(&uri);
            }
            if let Some(store) = child.children().as_ref().and_then(child_store) {
                store.remove_all();
            }
            child.set_expanded(false);
        }
    }

    /// The rows expanded now, outermost first.
    fn expanded_rows(&self) -> Vec<gtk::TreeListRow> {
        let mut expanded = self.0.expanded.borrow_mut();
        expanded.retain(|row| row.upgrade().is_some_and(|row| row.is_expanded()));
        let mut rows: Vec<gtk::TreeListRow> = expanded.iter().filter_map(glib::WeakRef::upgrade).collect();
        rows.sort_by_key(gtk::TreeListRow::depth);
        rows
    }

    /// The locations of the expanded folders, outermost first.
    pub(crate) fn expanded_uris(&self) -> Vec<String> {
        self.expanded_rows().iter().filter_map(row_uri).collect()
    }

    /// Expands the folders at `uris` once they are listed, the ones inside
    /// others once those are expanded and listed.
    pub(crate) fn expand_when_listed(&self, uris: Vec<String>) {
        self.0.pending.replace(uris.into_iter().collect());
        self.expand_pending();
    }

    /// Expands the listed folders that wait to be expanded again.
    fn expand_pending(&self) {
        let mut position = 0;
        while !self.0.pending.borrow().is_empty() {
            let Some(row) = self.row(position) else { return };
            let waits = row_uri(&row).is_some_and(|uri| self.0.pending.borrow_mut().remove(&uri));
            if waits {
                self.set_expanded(&row, true);
            }
            position += 1;
        }
    }

    /// Collapses every expanded folder and stops their listings.
    pub(crate) fn collapse_all(&self) {
        for row in self.expanded_rows().into_iter().filter(|row| row.depth() == 0) {
            self.collapse_branch(&row);
        }
        self.0.expanded.borrow_mut().clear();
        self.0.listings.borrow_mut().clear();
        self.0.pending.borrow_mut().clear();
    }

    /// Lists the folder at `uri` into `children`, the sorted list the tree
    /// made for it, unless it is listed already.
    fn list_children(&self, uri: &str, children: &gio::ListModel) {
        let Some(store) = child_store(children) else {
            return;
        };
        if store.n_items() > 0 || self.0.listings.borrow().contains_key(uri) {
            return;
        }
        let weak_store = store.downgrade();
        let tree: Weak<TreeState> = Rc::downgrade(&self.0);
        let listing = loader::list_folder(
            uri,
            move |entries| {
                if let Some(store) = weak_store.upgrade() {
                    let items: Vec<FileItem> = entries.into_iter().map(FileItem::new).collect();
                    store.extend_from_slice(&items);
                }
                if let Some(state) = tree.upgrade() {
                    FolderTree(state).expand_pending();
                }
            },
            |_| {},
        );
        self.0.listings.borrow_mut().insert(uri.to_owned(), listing);
    }
}

/// An empty, filtered and sorted list for a folder's contents; the folder
/// is listed into it when it expands.
fn child_items(sorted: &gtk::SortListModel, filter: &gtk::CustomFilter) -> gtk::SortListModel {
    let store = gio::ListStore::new::<FileItem>();
    let filtered = gtk::FilterListModel::new(Some(store), Some(filter.clone()));
    gtk::SortListModel::new(Some(filtered), sorted.sorter())
}

/// The store under a list [`child_items`] made.
fn child_store(children: &gio::ListModel) -> Option<gio::ListStore> {
    let sorted = children.downcast_ref::<gtk::SortListModel>()?;
    let filtered = sorted.model().and_downcast::<gtk::FilterListModel>()?;
    filtered.model().and_downcast::<gio::ListStore>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::{wait_until, Fixture};

    /// Collapsing a branch stops its descendants and reopening lists fresh
    /// contents, including files created while it was closed.
    ///
    /// parity: VIEW-035
    #[gtk::test]
    fn collapsing_a_branch_discards_all_its_child_listings() {
        let fixture = Fixture::standard();
        let nested = fixture.path("Documents").join("Nested");
        std::fs::create_dir(&nested).expect("a nested folder");
        std::fs::write(nested.join("inside.txt"), b"Synthetic data\n").expect("an inner file");
        let mut entry = crate::test_support::folder_entry("Documents");
        entry.uri = fixture.uri_of("Documents");
        let store = gio::ListStore::new::<FileItem>();
        store.append(&FileItem::new(entry));
        let sorted = gtk::SortListModel::new(Some(store), None::<gtk::Sorter>);
        let filter = gtk::CustomFilter::new(|_| true);
        let tree = FolderTree::new(&sorted, &filter);
        let documents = tree.row(0).expect("Documents");
        assert!(
            documents.is_expandable(),
            "GTK's cached capability survives the setting being off"
        );
        tree.set_expandable(true);
        tree.set_expanded(&documents, true);
        wait_until("Nested to be listed", || tree.model().n_items() == 2);
        let child = tree.row(1).expect("Nested");
        tree.set_expanded(&child, true);
        wait_until("the nested file", || tree.model().n_items() == 3);
        assert_eq!(tree.0.listings.borrow().len(), 2);
        tree.set_expanded(&documents, false);
        assert_eq!(tree.model().n_items(), 1);
        assert!(
            tree.0.listings.borrow().is_empty(),
            "descendant listings are cancelled too"
        );
        std::fs::write(fixture.path("Documents").join("new.txt"), b"Synthetic data\n").expect("another file");
        tree.set_expanded(&documents, true);
        wait_until("fresh contents on reopen", || tree.model().n_items() == 3);
        assert!(tree
            .model()
            .iter::<FileItem>()
            .filter_map(Result::ok)
            .any(|item| item.entry().name == "new.txt"));
        tree.collapse_all();
    }
}
