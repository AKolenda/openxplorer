// SPDX-License-Identifier: AGPL-3.0-only
//! Per-window tabs. Stable IDs and load generations reject stale callbacks.
//!
//! Ports the tab state of `desktop/ui/app.js` (`addTab`, `closeTab`,
//! `switchTab` and each tab's `history`, `scroll`, `loaded` and `busy`).
//! [`Session`] keeps its tabs and the active one private, so the active
//! id always names an open tab.

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use ox_core::entry::EntryError;

use crate::folder_view::item::FileItem;
use crate::folder_view::loader::Listing;
use crate::folder_view::watch::Watch;
use crate::history::History;

use super::listing_state::{ListingEnd, ListingState};

/// Identifies a tab for the lifetime of its window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TabId(u64);

impl TabId {
    /// The id as a window action's target (`win.select-tab`).
    pub(super) fn to_variant(self) -> glib::Variant {
        self.0.to_variant()
    }

    /// The id in a window action's target.
    pub(super) fn from_variant(variant: &glib::Variant) -> Option<Self> {
        variant.get::<u64>().map(TabId)
    }

    /// The id as the number inside a compound action target, such as the
    /// tab and window of `win.move-tab-into-window`.
    pub(super) const fn to_raw(self) -> u64 {
        self.0
    }

    /// The id a compound action target names by [`Self::to_raw`]. A number
    /// that names no open tab changes nothing where it is used.
    pub(super) const fn from_raw(raw: u64) -> Self {
        TabId(raw)
    }
}

/// Whether a new tab becomes the active one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TabPlacement {
    /// Show the new tab now.
    Foreground,
    /// Keep the current tab in front. The new tab is listed only when it is
    /// first shown, so an SMB sign-in never appears over the current tab
    /// (`addTab(uri, {background: true})` in app.js).
    Background,
}

/// Which way a step goes, through a tab's history or along the tab strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Direction {
    /// Back in history (Alt+Left), or to the previous tab (Ctrl+Shift+Tab).
    Backward,
    /// Forward in history (Alt+Right), or to the next tab (Ctrl+Tab).
    Forward,
}

impl Direction {
    /// The step as an offset in a list: -1 or 1.
    pub(super) const fn offset(self) -> isize {
        match self {
            Direction::Backward => -1,
            Direction::Forward => 1,
        }
    }
}

/// One tab: its history, its items and the state of its listing.
#[derive(Debug)]
pub(super) struct Tab {
    /// The tab's identity for the window's lifetime.
    pub id: TabId,
    /// The locations visited in this tab.
    pub history: History,
    /// The tab's items, unfiltered and unsorted.
    pub store: gio::ListStore,
    /// Advanced only by [`Tab::begin_load`]; [`Session::accepts`] rejects
    /// the results of an older load, so a stale listing never refills the
    /// tab (parity NAV-016).
    generation: u64,
    /// Whether the location is listed, being listed or never was.
    pub listing_state: ListingState,
    /// Why the last listing failed.
    pub error: Option<EntryError>,
    /// URIs of the selected items, restored after a reload or tab switch.
    pub selected: Vec<String>,
    /// The next listing scrolls to the first selected item, as a
    /// `FileManager1` `ShowItems` request asks.
    pub reveals_selection: bool,
    /// The vertical scroll position, restored when the tab is shown again.
    pub scroll: f64,
    /// A scroll position to restore once the listing finishes: a tab moved
    /// from another window keeps its place in its folder (TAB-039).
    pub scroll_after_listing: Option<f64>,
    /// The running listing; dropping it cancels it.
    pub listing: Option<Listing>,
    /// The folder watch, kept while the tab shows the same folder.
    pub watch: Option<Watch>,
    /// An item to scroll into view once the folder is listed, as "Open
    /// file location" asks.
    pub revealed_item: Option<String>,
}

impl Tab {
    fn new(id: TabId, uri: &str) -> Self {
        Self {
            id,
            history: History::new(uri),
            store: gio::ListStore::new::<FileItem>(),
            generation: 0,
            listing_state: ListingState::NotListed,
            error: None,
            selected: Vec::new(),
            reveals_selection: false,
            scroll: 0.0,
            scroll_after_listing: None,
            listing: None,
            watch: None,
            revealed_item: None,
        }
    }

    /// The location the tab shows.
    pub(super) fn uri(&self) -> &str {
        self.history.current()
    }

    /// Starts a load and returns its generation. The folder watch is kept:
    /// the caller replaces it only when the location changed.
    pub(super) fn begin_load(&mut self) -> u64 {
        self.listing = None;
        self.generation = self.generation.wrapping_add(1);
        self.listing_state.begin();
        self.error = None;
        self.generation
    }

    /// Forgets what belonged to the previous location: the selection and
    /// the scroll position, as `navigate()` does with `t.scroll = 0`.
    pub(super) fn forget_location_state(&mut self) {
        self.selected.clear();
        self.scroll = 0.0;
    }

    /// Stops the tab's listing and folder watch, as Sign out cancels the
    /// loads and the refreshes on its server; the rows stay.
    pub(super) fn stop_reading(&mut self) {
        self.listing = None;
        self.watch = None;
        self.generation = self.generation.wrapping_add(1);
        self.listing_state.stop();
    }

    /// Marks the tab to be listed again when next shown, as Sign out marks
    /// its server's tabs (`t.loaded=false`), and returns its items for
    /// the caller to drop with [`gio::ListStore::remove_all`] once the
    /// session is no longer borrowed: the active tab's items are on
    /// screen, and removing them runs the view's handlers, which read the
    /// session.
    #[must_use = "the caller empties the returned items"]
    pub(super) fn mark_stale(&mut self) -> gio::ListStore {
        self.stop_reading();
        self.listing_state = ListingState::NotListed;
        self.error = None;
        self.store.clone()
    }
}

/// The tabs of one window and which one is active.
#[derive(Debug, Default)]
pub(super) struct Session {
    /// The tabs, left to right.
    tabs: Vec<Tab>,
    /// The tab in front; always one of `tabs`, or `None` once none is left.
    active: Option<TabId>,
    next_id: u64,
}

impl Session {
    /// Adds a tab at the end. A background tab becomes active only when it
    /// is the first one.
    ///
    /// # Panics
    ///
    /// Only after `u64::MAX` tabs in one window.
    pub(super) fn add(&mut self, uri: &str, placement: TabPlacement) -> TabId {
        let id = self.next_tab_id();
        self.tabs.push(Tab::new(id, uri));
        if placement == TabPlacement::Foreground || self.active.is_none() {
            self.active = Some(id);
        }
        id
    }

    /// Adds a tab that moved here from another window, with its `history`,
    /// before tab `before` or at the end, and brings it to the front.
    ///
    /// # Panics
    ///
    /// Only after `u64::MAX` tabs in one window.
    pub(super) fn insert_moved(&mut self, history: History, before: Option<TabId>) -> TabId {
        let id = self.next_tab_id();
        let mut tab = Tab::new(id, history.current());
        tab.history = history;
        let index = self.index_before(before);
        self.tabs.insert(index, tab);
        self.active = Some(id);
        id
    }

    /// Moves tab `id` before tab `before`, or to the end when `before` is
    /// `None` or not open (`reorderTab` in app.js). The active tab stays
    /// the same.
    pub(super) fn move_before(&mut self, id: TabId, before: Option<TabId>) {
        if before == Some(id) {
            return;
        }
        let Some(from) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        let tab = self.tabs.remove(from);
        let index = self.index_before(before);
        self.tabs.insert(index, tab);
    }

    /// The index a tab placed before tab `before` gets: that tab's index,
    /// or the end.
    fn index_before(&self, before: Option<TabId>) -> usize {
        before
            .and_then(|before| self.tabs.iter().position(|tab| tab.id == before))
            .unwrap_or(self.tabs.len())
    }

    /// A new tab id.
    ///
    /// # Panics
    ///
    /// Only after `u64::MAX` tabs in one window.
    fn next_tab_id(&mut self) -> TabId {
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("tab IDs cannot be exhausted in one session");
        TabId(self.next_id)
    }

    /// The open tabs, left to right, to change.
    pub(super) fn tabs_mut(&mut self) -> &mut [Tab] {
        &mut self.tabs
    }

    /// The open tabs, left to right.
    pub(super) fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    /// The tab `id`, if it is open.
    pub(super) fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    /// The tab `id`, to change it.
    pub(super) fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    /// The id of the tab in front.
    pub(super) fn active_id(&self) -> Option<TabId> {
        self.active
    }

    /// The tab in front.
    pub(super) fn active(&self) -> Option<&Tab> {
        self.active.and_then(|id| self.tab(id))
    }

    /// The tab in front, to change it.
    pub(super) fn active_mut(&mut self) -> Option<&mut Tab> {
        let id = self.active?;
        self.tab_mut(id)
    }

    /// True when tab `id` is in front.
    pub(super) fn is_active(&self, id: TabId) -> bool {
        self.active == Some(id)
    }

    /// True when tab `id` is open and not already in front.
    pub(super) fn can_activate(&self, id: TabId) -> bool {
        !self.is_active(id) && self.tab(id).is_some()
    }

    /// Brings tab `id` to the front. An id that is not open changes
    /// nothing, so the active id always names an open tab.
    pub(super) fn activate(&mut self, id: TabId) {
        if self.tab(id).is_some() {
            self.active = Some(id);
        }
    }

    /// True while `generation` is the latest load of tab `id`.
    pub(super) fn accepts(&self, id: TabId, generation: u64) -> bool {
        self.tab(id).is_some_and(|tab| tab.generation == generation)
    }

    /// Ends tab `id`'s listing: the tab is listed, and is listed again
    /// when its folder changed meanwhile (see [`ListingEnd`]).
    pub(super) fn end_listing(&mut self, id: TabId) -> ListingEnd {
        let Some(tab) = self.tab_mut(id) else {
            return ListingEnd::TabClosed;
        };
        tab.listing_state.finish()
    }

    /// Removes a tab. When it was active, the tab to its right becomes
    /// active, or the new last tab (`Math.min(i, tabs.length - 1)` in
    /// app.js, as in Windows Explorer and browsers).
    pub(super) fn remove(&mut self, id: TabId) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        self.tabs.remove(index);
        if self.active == Some(id) {
            let next = self.tabs.get(index).or_else(|| self.tabs.last());
            self.active = next.map(|tab| tab.id);
        }
    }

    /// The tab next to the active one in `direction`, wrapping around at
    /// either end.
    pub(super) fn adjacent(&self, direction: Direction) -> Option<TabId> {
        let current = self.tabs.iter().position(|tab| Some(tab.id) == self.active)?;
        let count = self.tabs.len();
        let target = match direction {
            Direction::Forward => (current + 1) % count,
            Direction::Backward => (current + count - 1) % count,
        };
        Some(self.tabs[target].id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn three_tabs() -> (Session, [TabId; 3]) {
        let mut session = Session::default();
        let first = session.add("file:///one", TabPlacement::Foreground);
        let second = session.add("file:///two", TabPlacement::Foreground);
        let third = session.add("file:///three", TabPlacement::Foreground);
        (session, [first, second, third])
    }

    #[test]
    fn closing_a_background_tab_keeps_the_active_tab() {
        let mut session = Session::default();
        let first = session.add("file:///one", TabPlacement::Foreground);
        let second = session.add("file:///two", TabPlacement::Foreground);
        session.remove(first);
        assert_eq!(session.active_id(), Some(second));
        session.remove(second);
        assert_eq!(session.active_id(), None);
    }

    #[test]
    fn closing_the_active_tab_chooses_a_neighbor() {
        let mut session = Session::default();
        let first = session.add("file:///one", TabPlacement::Foreground);
        let second = session.add("file:///two", TabPlacement::Foreground);
        session.remove(second);
        assert_eq!(session.active_id(), Some(first));
        assert_eq!(session.adjacent(Direction::Forward), Some(first));
    }

    /// parity: TAB-002
    #[test]
    fn closing_the_active_middle_tab_activates_the_tab_to_its_right() {
        let (mut session, [_, second, third]) = three_tabs();
        session.activate(second);
        session.remove(second);
        assert_eq!(session.active_id(), Some(third));
    }

    #[test]
    fn closing_the_active_last_tab_activates_the_new_last_tab() {
        let (mut session, [_, second, third]) = three_tabs();
        session.remove(third);
        assert_eq!(session.active_id(), Some(second));
    }

    #[test]
    fn a_background_tab_leaves_the_active_tab_in_front() {
        let mut session = Session::default();
        let first = session.add("file:///one", TabPlacement::Foreground);
        let background = session.add("file:///two", TabPlacement::Background);
        assert_eq!(session.active_id(), Some(first));
        let background = session.tab(background).expect("added tab");
        assert!(background.listing_state.needs_listing());
    }

    #[test]
    fn the_first_tab_is_active_even_when_opened_in_the_background() {
        let mut session = Session::default();
        let only = session.add("file:///one", TabPlacement::Background);
        assert_eq!(session.active_id(), Some(only));
    }

    /// parity: NAV-016
    #[test]
    fn old_results_cannot_repopulate_a_navigated_or_closed_tab() {
        let mut session = Session::default();
        let id = session.add("file:///one", TabPlacement::Foreground);
        let first = session.tab_mut(id).expect("added tab").begin_load();
        assert!(session.accepts(id, first));
        let second = session.tab_mut(id).expect("added tab").begin_load();
        assert!(!session.accepts(id, first));
        assert!(session.accepts(id, second));
        session.remove(id);
        assert!(!session.accepts(id, second));
    }

    /// parity: TAB-005
    #[test]
    fn tab_cycling_wraps_in_both_directions() {
        let mut session = Session::default();
        let first = session.add("file:///one", TabPlacement::Foreground);
        let second = session.add("file:///two", TabPlacement::Foreground);
        assert_eq!(session.adjacent(Direction::Forward), Some(first));
        session.activate(first);
        assert_eq!(session.adjacent(Direction::Backward), Some(second));
    }

    #[test]
    fn a_tab_that_is_not_open_cannot_be_brought_to_the_front() {
        let (mut session, [first, second, _]) = three_tabs();
        session.remove(first);
        assert!(!session.can_activate(first));
        session.activate(first);
        assert_ne!(session.active_id(), Some(first), "a closed tab stays closed");
        assert!(session.can_activate(second));
    }

    fn tab_order(session: &Session) -> Vec<TabId> {
        session.tabs().iter().map(|tab| tab.id).collect()
    }

    /// parity: TAB-032
    #[test]
    fn a_reordered_tab_goes_before_the_named_tab_or_to_the_end() {
        let (mut session, [first, second, third]) = three_tabs();

        session.move_before(third, Some(first));
        let before_first = tab_order(&session);
        session.move_before(third, None);
        let at_the_end = tab_order(&session);
        session.move_before(second, Some(second));

        assert_eq!(before_first, [third, first, second]);
        assert_eq!(at_the_end, [first, second, third]);
        assert_eq!(
            tab_order(&session),
            [first, second, third],
            "a tab dropped on itself stays"
        );
        assert_eq!(
            session.active_id(),
            Some(third),
            "reordering keeps the active tab"
        );
    }

    /// parity: TAB-033, TAB-039
    #[test]
    fn a_moved_tab_keeps_its_history_and_goes_in_front_where_it_was_dropped() {
        let (mut session, [first, second, _]) = three_tabs();
        let mut history = History::new("file:///moved/a");
        history.push("file:///moved/b");
        history.go(-1);

        let moved = session.insert_moved(history.clone(), Some(second));

        assert_eq!(tab_order(&session)[..3], [first, moved, second]);
        assert_eq!(session.active_id(), Some(moved));
        let tab = session.tab(moved).expect("the moved tab is open");
        assert_eq!(tab.history, history);
        assert_eq!(tab.uri(), "file:///moved/a");
    }

    /// parity: NAV-016
    #[test]
    fn ending_the_listing_of_a_closed_tab_says_so() {
        let mut session = Session::default();
        let id = session.add("file:///one", TabPlacement::Foreground);
        session.tab_mut(id).expect("added tab").begin_load();
        assert_eq!(session.end_listing(id), ListingEnd::Done);
        session.remove(id);
        assert_eq!(session.end_listing(id), ListingEnd::TabClosed);
    }
}
