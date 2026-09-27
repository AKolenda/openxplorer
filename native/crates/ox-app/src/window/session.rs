// SPDX-License-Identifier: AGPL-3.0-only
//! Per-window tabs. Stable IDs and load generations reject stale callbacks.

use gtk::gio;

use crate::folder_view::item::FileItem;
use crate::folder_view::loader::{Listing, Watch};
use crate::history::History;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TabId(u64);

pub(super) struct Tab {
    pub id: TabId,
    pub history: History,
    pub store: gio::ListStore,
    pub generation: u64,
    pub loading: bool,
    pub loaded: bool,
    pub error: Option<String>,
    pub selected: Vec<String>,
    pub listing: Option<Listing>,
    pub watch: Option<Watch>,
}

impl Tab {
    fn new(id: TabId, uri: &str) -> Self {
        Self {
            id,
            history: History::new(uri),
            store: gio::ListStore::new::<FileItem>(),
            generation: 0,
            loading: false,
            loaded: false,
            error: None,
            selected: Vec::new(),
            listing: None,
            watch: None,
        }
    }

    pub fn begin_load(&mut self) -> u64 {
        self.listing = None;
        self.watch = None;
        self.generation = self.generation.wrapping_add(1);
        self.loading = true;
        self.loaded = false;
        self.error = None;
        self.generation
    }
}

#[derive(Default)]
pub(super) struct Session {
    pub tabs: Vec<Tab>,
    pub active: Option<TabId>,
    next_id: u64,
}

impl Session {
    pub fn add(&mut self, uri: &str) -> TabId {
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("tab IDs cannot be exhausted in one session");
        let id = TabId(self.next_id);
        self.tabs.push(Tab::new(id, uri));
        self.active = Some(id);
        id
    }

    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    pub fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    pub fn active(&self) -> Option<&Tab> {
        self.active.and_then(|id| self.tab(id))
    }

    pub fn accepts(&self, id: TabId, generation: u64) -> bool {
        self.tab(id).is_some_and(|tab| tab.generation == generation)
    }

    pub fn remove(&mut self, id: TabId) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        self.tabs.remove(index);
        if self.active == Some(id) {
            self.active = self.tabs.get(index.saturating_sub(1)).map(|tab| tab.id);
        }
    }

    pub fn adjacent(&self, delta: isize) -> Option<TabId> {
        let current = self.tabs.iter().position(|tab| Some(tab.id) == self.active)?;
        let target = (current as isize + delta).rem_euclid(self.tabs.len() as isize) as usize;
        Some(self.tabs[target].id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_a_background_tab_keeps_the_active_tab() {
        let mut session = Session::default();
        let first = session.add("file:///one");
        let second = session.add("file:///two");
        session.remove(first);
        assert_eq!(session.active, Some(second));
        session.remove(second);
        assert_eq!(session.active, None);
    }

    #[test]
    fn closing_the_active_tab_chooses_a_neighbor() {
        let mut session = Session::default();
        let first = session.add("file:///one");
        let second = session.add("file:///two");
        session.remove(second);
        assert_eq!(session.active, Some(first));
        assert_eq!(session.adjacent(1), Some(first));
    }

    #[test]
    fn old_results_cannot_repopulate_a_navigated_or_closed_tab() {
        let mut session = Session::default();
        let id = session.add("file:///one");
        let first = session.tab_mut(id).expect("added tab").begin_load();
        assert!(session.accepts(id, first));
        let second = session.tab_mut(id).expect("added tab").begin_load();
        assert!(!session.accepts(id, first));
        assert!(session.accepts(id, second));
        session.remove(id);
        assert!(!session.accepts(id, second));
    }

    #[test]
    fn tab_cycling_wraps_in_both_directions() {
        let mut session = Session::default();
        let first = session.add("file:///one");
        let second = session.add("file:///two");
        assert_eq!(session.adjacent(1), Some(first));
        session.active = Some(first);
        assert_eq!(session.adjacent(-1), Some(second));
    }
}
