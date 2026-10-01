// SPDX-License-Identifier: AGPL-3.0-only
//! The list models behind both views: filter, sort and selection.
//!
//! Each tab owns a `gio::ListStore` of [`FileItem`]s; the folder view shows
//! the active tab's store through one filter model (hidden items and the
//! search box), one sort model (folders first, then the chosen column in
//! natural order, ties by name ascending, as `filtered()` in app.js) and one
//! multi-selection shared by the details and icon views.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::search::SearchFacets;

use crate::folder_view::details::GroupTitle;
use crate::folder_view::filter::FilterState;
use crate::folder_view::groups::{self, GroupClock};
use crate::folder_view::item::FileItem;
use crate::folder_view::sort_roles::{SortRole, SortState};
use crate::folder_view::sorting::{self, SortColumn, SortDirection};
use crate::folder_view::tree::FolderTree;

/// The item a folder model hands to its filter or sorters.
fn as_item(object: &glib::Object) -> &FileItem {
    object
        .downcast_ref::<FileItem>()
        .expect("folder models hold FileItems")
}

/// Compares two items by one column, without folders-first or tie-breaks.
/// Sizes that are not known, and folders never measured, count as 0, as
/// in app.js.
fn compare_column(column: SortColumn, a: &FileItem, b: &FileItem) -> Ordering {
    match column {
        SortColumn::Name => a.sort_name().key.natural_cmp(b.sort_name().key),
        SortColumn::Modified => a.entry().modified.cmp(&b.entry().modified),
        SortColumn::FolderPath => a.folder_path().key.natural_cmp(&b.folder_path().key),
        SortColumn::Type => a.type_sort_key().natural_cmp(b.type_sort_key()),
        SortColumn::Size => a.sort_size().cmp(&b.sort_size()),
    }
}

/// The sorter a details column uses; the column view applies the
/// direction.
pub(crate) fn column_sorter(column: SortColumn) -> gtk::CustomSorter {
    gtk::CustomSorter::new(move |a, b| {
        let order = compare_column(column, as_item(a), as_item(b));
        order.into()
    })
}

/// How the model sorts beyond the details view's column: folders first or
/// not, a further sort key (VIEW-019) and groups (VIEW-022).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SortOptions {
    folders_first: bool,
    role: Option<(SortRole, SortDirection)>,
    grouping: Option<(SortState, GroupClock)>,
}

impl Default for SortOptions {
    fn default() -> Self {
        Self {
            folders_first: true,
            role: None,
            grouping: None,
        }
    }
}

/// Shared with the sorters' callbacks, which GTK calls with no access to
/// the model.
type SharedOptions = Rc<std::cell::Cell<SortOptions>>;

/// Sorts folders before files, whichever way the column sorts, while
/// folders come first.
fn folders_first(options: &SharedOptions) -> gtk::CustomSorter {
    let options = Rc::clone(options);
    gtk::CustomSorter::new(move |a, b| {
        if !options.get().folders_first {
            return gtk::Ordering::Equal;
        }
        let a_is_folder = as_item(a).entry().is_dir;
        let b_is_folder = as_item(b).entry().is_dir;
        // Reversed, so `true` sorts first.
        b_is_folder.cmp(&a_is_folder).into()
    })
}

/// Sorts by the further key while one is chosen; the column view is then
/// unsorted.
fn role_sorter(options: &SharedOptions) -> gtk::CustomSorter {
    let options = Rc::clone(options);
    gtk::CustomSorter::new(move |a, b| {
        let Some((role, direction)) = options.get().role else {
            return gtk::Ordering::Equal;
        };
        let order = role.compare(as_item(a), as_item(b));
        directed(order, direction).into()
    })
}

/// Sorts the items into their groups, in the order the key sorts.
fn group_sorter(options: &SharedOptions) -> gtk::CustomSorter {
    let options = Rc::clone(options);
    gtk::CustomSorter::new(move |a, b| {
        let Some((state, clock)) = options.get().grouping else {
            return gtk::Ordering::Equal;
        };
        let a = groups::group_of(state.by, as_item(a), &clock);
        let b = groups::group_of(state.by, as_item(b), &clock);
        directed(a.compare(&b), state.direction).into()
    })
}

/// `order` for an ascending sort, reversed for a descending one.
fn directed(order: Ordering, direction: SortDirection) -> Ordering {
    match direction {
        SortDirection::Ascending => order,
        SortDirection::Descending => order.reverse(),
    }
}

/// Breaks ties by name, always ascending.
fn names_ascending() -> gtk::CustomSorter {
    gtk::CustomSorter::new(|a, b| {
        let order = sorting::compare_names(as_item(a).sort_name(), as_item(b).sort_name());
        order.into()
    })
}

/// Counts for the status bar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SelectionSummary {
    /// Selected items.
    pub count: u32,
    /// Total size of the selected files (folders count as 0).
    pub bytes: u64,
    /// At least one selected item is a file with a known size.
    pub has_files: bool,
}

/// Filter, sort and selection over the active tab's store.
#[derive(Debug)]
pub(crate) struct FolderModel {
    /// Shared with the filter's callback, which GTK calls with no access
    /// to the model.
    filter_state: Rc<RefCell<FilterState>>,
    filter: gtk::CustomFilter,
    filter_model: gtk::FilterListModel,
    sort_model: gtk::SortListModel,
    /// The sorted items with expanded folders' contents (VIEW-035).
    tree: FolderTree,
    selection: gtk::MultiSelection,
    sort_options: SharedOptions,
    folders_first: gtk::CustomSorter,
    role_sorter: gtk::CustomSorter,
    group_sorter: gtk::CustomSorter,
}

impl FolderModel {
    /// Empty models; [`FolderModel::set_store`] shows a tab's items.
    pub(crate) fn new() -> Self {
        let filter_state = Rc::new(RefCell::new(FilterState::default()));
        let state = Rc::clone(&filter_state);
        let filter = gtk::CustomFilter::new(move |object| {
            let item = as_item(object);
            let state = state.borrow();
            state.accepts(item.lowercase_name(), item.visibility()) && state.passes_facets(item.entry())
        });
        let filter_model = gtk::FilterListModel::new(None::<gio::ListStore>, Some(filter.clone()));
        let sort_model = gtk::SortListModel::new(Some(filter_model.clone()), None::<gtk::Sorter>);
        let tree = FolderTree::new(&sort_model, &filter);
        let selection = gtk::MultiSelection::new(Some(tree.model().clone()));
        let sort_options = SharedOptions::default();
        Self {
            filter_state,
            filter,
            filter_model,
            sort_model,
            tree,
            selection,
            folders_first: folders_first(&sort_options),
            role_sorter: role_sorter(&sort_options),
            group_sorter: group_sorter(&sort_options),
            sort_options,
        }
    }

    /// Completes the sorter once the details view exists: folders first,
    /// then a further sort key while one is chosen, then `column_sorter`
    /// (the column view's sorter, which applies the chosen direction), then
    /// names ascending.
    pub(crate) fn attach_column_sorter(&self, column_sorter: &gtk::Sorter) {
        let sorter = gtk::MultiSorter::new();
        sorter.append(self.folders_first.clone());
        sorter.append(self.role_sorter.clone());
        sorter.append(column_sorter.clone());
        sorter.append(names_ascending());
        self.sort_model.set_sorter(Some(&sorter));
    }

    /// Lists folders before files when `first`, else among them.
    pub(crate) fn set_folders_first(&self, first: bool) {
        self.change_sort_options(&self.folders_first, |options| options.folders_first = first);
    }

    /// Whether folders are listed before files.
    pub(crate) fn folders_first(&self) -> bool {
        self.sort_options.get().folders_first
    }

    /// Sorts by a further key and direction, or by the details view's
    /// column again with `None`.
    pub(crate) fn set_sort_role(&self, role: Option<(SortRole, SortDirection)>) {
        self.change_sort_options(&self.role_sorter, |options| options.role = role);
    }

    /// The further key the model sorts by, if one is chosen.
    pub(crate) fn sort_role(&self) -> Option<(SortRole, SortDirection)> {
        self.sort_options.get().role
    }

    /// Groups the items by the key `state` sorts by, or stops grouping
    /// them with `None`. Dates are grouped by the periods as of now.
    pub(crate) fn set_grouping(&self, state: Option<SortState>) {
        let grouping = state.and_then(|state| Some((state, GroupClock::now()?)));
        let was_grouped = self.sort_options.get().grouping.is_some();
        self.change_sort_options(&self.group_sorter, |options| options.grouping = grouping);
        if grouping.is_some() != was_grouped {
            let sections = grouping.map(|_| self.group_sorter.clone());
            self.sort_model.set_section_sorter(sections.as_ref());
            // GTK 4.14's tree list passes no groups through, so grouped
            // items are shown without it, and folders do not expand.
            if grouping.is_some() {
                self.tree.set_expandable(false);
                self.selection.set_model(Some(&self.sort_model));
            } else {
                self.selection.set_model(Some(self.tree.model()));
            }
        }
    }

    /// The key the items are grouped by, if they are grouped.
    pub(crate) fn grouping(&self) -> Option<SortState> {
        self.sort_options.get().grouping.map(|(state, _)| state)
    }

    /// Names the group an item is in, while the items are grouped.
    pub(crate) fn group_titles(&self) -> GroupTitle {
        let options = Rc::clone(&self.sort_options);
        Rc::new(move |item| {
            let (state, clock) = options.get().grouping?;
            Some(groups::group_of(state.by, item, &clock).title)
        })
    }

    /// Applies `change` to the sort options and sorts again with `sorter`
    /// when they changed.
    fn change_sort_options(&self, sorter: &gtk::CustomSorter, change: impl FnOnce(&mut SortOptions)) {
        let mut options = self.sort_options.get();
        change(&mut options);
        if options == self.sort_options.get() {
            return;
        }
        self.sort_options.set(options);
        sorter.changed(gtk::SorterChange::Different);
    }

    /// The selection model both views display.
    pub(crate) fn selection(&self) -> &gtk::MultiSelection {
        &self.selection
    }

    /// The sorted, filtered items in display order, without the contents
    /// of expanded folders.
    #[cfg(test)]
    pub(crate) fn sorted(&self) -> &gtk::SortListModel {
        &self.sort_model
    }

    /// The folders that expand in place in the details view.
    pub(crate) fn tree(&self) -> &FolderTree {
        &self.tree
    }

    /// Shows another tab's items, with no folder expanded.
    pub(crate) fn set_store(&self, store: Option<&gio::ListStore>) {
        self.tree.collapse_all();
        self.filter_model.set_model(store);
    }

    /// Items shown (after filtering), expanded folders' contents included.
    pub(crate) fn n_items(&self) -> u32 {
        self.selection.n_items()
    }

    /// How many of `store`'s items the folder lists, searched or not: the
    /// hidden ones count only while hidden files are shown, as the Python
    /// backend lists them (`enumerate_folder(uri, showHidden)`).
    pub(crate) fn listed_count(&self, store: &gio::ListStore) -> u32 {
        let filter = self.filter_state.borrow();
        let listed = store
            .iter::<FileItem>()
            .filter_map(Result::ok)
            .filter(|item| filter.lists(item.visibility()))
            .count();
        u32::try_from(listed).unwrap_or(u32::MAX)
    }

    /// The item at a display position.
    pub(crate) fn item(&self, position: u32) -> Option<FileItem> {
        self.selection.item(position).and_downcast::<FileItem>()
    }

    /// The display position of the item at `uri`, if it is shown.
    pub(crate) fn position_of_uri(&self, uri: &str) -> Option<u32> {
        (0..self.n_items()).find(|&position| self.item(position).is_some_and(|item| item.entry().uri == uri))
    }

    /// The display name at a position, or `None` past the end.
    pub(crate) fn name_at(&self, position: u32) -> Option<String> {
        let item = self.item(position)?;
        Some(item.entry().name.clone())
    }

    /// Sets the search text; returns true when the shown items changed.
    pub(crate) fn set_query(&self, query: &str) -> bool {
        self.update_filter(|state| state.set_query(query))
    }

    /// Sets the search options (SRCH-037); returns true when they changed.
    pub(crate) fn set_facets(&self, facets: SearchFacets) -> bool {
        self.update_filter(|state| state.set_facets(facets))
    }

    /// Whether hidden items are shown.
    pub(crate) fn shows_hidden(&self) -> bool {
        self.filter_state.borrow().shows_hidden()
    }

    /// Shows or hides hidden items; returns true when that changed.
    pub(crate) fn set_show_hidden(&self, show: bool) -> bool {
        self.update_filter(|state| state.set_show_hidden(show))
    }

    /// Applies `update` to the filter state and, when it reports a change,
    /// filters the items again. Returns what `update` returned.
    fn update_filter(&self, update: impl FnOnce(&mut FilterState) -> bool) -> bool {
        // The borrow ends with this statement: filtering again calls the
        // filter's callback, which borrows the state too.
        let changed = update(&mut self.filter_state.borrow_mut());
        if changed {
            self.filter.changed(gtk::FilterChange::Different);
        }
        changed
    }

    /// Display positions of the selected items, ascending.
    pub(crate) fn selected_positions(&self) -> Vec<u32> {
        let bitset = self.selection.selection();
        // A list model has at most u32::MAX positions.
        let count = u32::try_from(bitset.size()).unwrap_or(u32::MAX);
        (0..count).map(|nth| bitset.nth(nth)).collect()
    }

    /// The selected items in display order.
    pub(crate) fn selected_items(&self) -> Vec<FileItem> {
        self.selected_positions()
            .into_iter()
            .filter_map(|position| self.item(position))
            .collect()
    }

    /// The first selected position, if any.
    pub(crate) fn first_selected(&self) -> Option<u32> {
        let bitset = self.selection.selection();
        if bitset.is_empty() {
            return None;
        }
        Some(bitset.minimum())
    }

    /// Count and size of the selection.
    pub(crate) fn summary(&self) -> SelectionSummary {
        let mut summary = SelectionSummary::default();
        for item in self.selected_items() {
            summary.count += 1;
            if let Some(size) = item.file_size() {
                summary.bytes += size;
                summary.has_files = true;
            }
        }
        summary
    }

    /// Selects only `position`.
    pub(crate) fn select_only(&self, position: u32) {
        self.selection.select_item(position, true);
    }

    /// Selects exactly the items at `positions`.
    pub(crate) fn select_positions(&self, positions: &[u32]) {
        let selected = gtk::Bitset::new_empty();
        for &position in positions {
            selected.add(position);
        }
        let everything = gtk::Bitset::new_range(0, self.n_items());
        self.selection.set_selection(&selected, &everything);
    }

    /// Selects every shown item.
    pub(crate) fn select_all(&self) {
        self.selection.select_all();
    }

    /// Clears the selection.
    pub(crate) fn select_none(&self) {
        self.selection.unselect_all();
    }

    /// Selects exactly the items that were not selected.
    pub(crate) fn invert_selection(&self) {
        let count = self.n_items();
        let everything = gtk::Bitset::new_range(0, count);
        let inverted = gtk::Bitset::new_range(0, count);
        inverted.subtract(&self.selection.selection());
        self.selection.set_selection(&inverted, &everything);
    }

    /// URIs of the selected items, in display order.
    pub(crate) fn selected_uris(&self) -> Vec<String> {
        self.selected_items()
            .iter()
            .map(|item| item.entry().uri.clone())
            .collect()
    }

    /// Selects exactly the items whose URIs are in `uris` (after a tab
    /// switch or a reload). URIs that are not listed are ignored.
    ///
    /// One pass over the rows with a set lookup: a whole selected folder of
    /// 20,000 photos is restored in milliseconds, not seconds.
    pub(crate) fn select_uris(&self, uris: &[String]) {
        if uris.is_empty() {
            self.select_none();
            return;
        }
        let wanted: HashSet<&str> = uris.iter().map(String::as_str).collect();
        let count = self.n_items();
        let selected = gtk::Bitset::new_empty();
        for position in 0..count {
            let is_wanted = self
                .item(position)
                .is_some_and(|item| wanted.contains(item.entry().uri.as_str()));
            if is_wanted {
                selected.add(position);
            }
        }
        let everything = gtk::Bitset::new_range(0, count);
        self.selection.set_selection(&selected, &everything);
    }
}

impl Default for FolderModel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folder_view::item::store_of_files;
    use crate::test_support::file_entry;

    /// A model showing a tab's store of files called `names`, and the store.
    fn model_with(names: &[&str]) -> (FolderModel, gio::ListStore) {
        let store = store_of_files(names);
        let model = FolderModel::new();
        model.set_store(Some(&store));
        (model, store)
    }

    fn uri(name: &str) -> String {
        file_entry(name).uri
    }

    fn sorted_selection(model: &FolderModel) -> Vec<String> {
        let mut selected = model.selected_uris();
        selected.sort();
        selected
    }

    /// parity: NAV-014
    #[gtk::test]
    fn restoring_a_selection_selects_exactly_the_listed_uris() {
        let (model, _store) = model_with(&["a.txt", "b.txt", "c.txt"]);
        let saved = [uri("c.txt"), uri("a.txt"), uri("gone.txt"), uri("a.txt")];
        model.select_uris(&saved);
        assert_eq!(sorted_selection(&model), [uri("a.txt"), uri("c.txt")]);
    }

    #[gtk::test]
    fn restoring_an_empty_selection_clears_it() {
        let (model, _store) = model_with(&["a.txt", "b.txt"]);
        model.select_all();
        model.select_uris(&[]);
        assert!(model.selected_uris().is_empty());
    }

    #[gtk::test]
    fn the_folder_counts_hidden_items_only_while_they_are_shown() {
        let (model, store) = model_with(&["a.txt", "b.txt"]);
        let mut cache = file_entry(".cache");
        cache.is_hidden = true;
        store.append(&FileItem::new(cache));
        model.set_query("a.txt");
        assert_eq!(
            model.listed_count(&store),
            2,
            "a search does not change the count"
        );
        model.set_show_hidden(true);
        assert_eq!(model.listed_count(&store), 3);
    }

    /// GTK's filter and sort models keep their result: setting the same
    /// words or hidden flag again recomputes nothing, as app.js memoised
    /// `filtered()`, and only a real change filters again.
    ///
    /// parity: PERF-004
    #[gtk::test]
    fn the_shown_items_are_recomputed_only_when_their_inputs_change() {
        let (model, _store) = model_with(&["a.txt", "b.txt", "report.txt"]);
        model.set_query("report");
        let changes = Rc::new(std::cell::Cell::new(0));
        let counter = Rc::clone(&changes);
        model
            .sorted()
            .connect_items_changed(move |_, _, _, _| counter.set(counter.get() + 1));
        let first = model.item(0);

        assert!(!model.set_query("  Report "), "the same words");
        assert!(!model.set_show_hidden(false), "the same hidden flag");
        assert_eq!(changes.get(), 0);
        assert_eq!(model.item(0), first, "the kept rows are reused");

        assert!(model.set_query("a"));
        assert!(changes.get() > 0);
        assert_eq!(model.n_items(), 1);
    }

    /// Sizes sort by value; folders never measured and files of unknown
    /// size count as 0.
    ///
    /// parity: VIEW-015
    #[gtk::test]
    fn unmeasured_folders_and_unknown_sizes_sort_as_nothing() {
        let mut big = file_entry("big.bin");
        big.size = Some(10);
        let big = FileItem::new(big);
        let folder = FileItem::new(crate::test_support::folder_entry("Photos"));
        let unknown = FileItem::new(file_entry("unknown.bin"));
        assert_eq!(compare_column(SortColumn::Size, &folder, &big), Ordering::Less);
        assert_eq!(compare_column(SortColumn::Size, &unknown, &big), Ordering::Less);
        assert_eq!(
            compare_column(SortColumn::Size, &folder, &unknown),
            Ordering::Equal
        );
    }

    #[gtk::test]
    fn a_position_past_the_end_has_no_name() {
        let (model, _store) = model_with(&["a.txt", "b.txt"]);
        assert_eq!(model.name_at(1).as_deref(), Some("b.txt"));
        assert_eq!(model.name_at(2), None);
    }

    /// parity: NAV-014
    #[gtk::test]
    fn a_whole_selected_folder_is_restored() {
        let names: Vec<String> = (0..2000).map(|number| format!("photo {number}.jpg")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let (model, _store) = model_with(&names);
        let everything: Vec<String> = names.iter().map(|name| uri(name)).collect();
        model.select_uris(&everything);
        assert_eq!(model.summary().count, 2000);
    }
}
