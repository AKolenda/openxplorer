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
    /// The vertical scroll position, restored when the tab is shown again.
    pub scroll: f64,
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
            scroll: 0.0,
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
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("tab IDs cannot be exhausted in one session");
        let id = TabId(self.next_id);
        self.tabs.push(Tab::new(id, uri));
        if placement == TabPlacement::Foreground || self.active.is_none() {
            self.active = Some(id);
        }
        id
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
