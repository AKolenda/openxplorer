// SPDX-License-Identifier: AGPL-3.0-only
//! Per-tab navigation history.
//!
//! Ports the history handling of `navigate` and `goHistory` in
//! `v2.0.0:desktop/ui/app.js`: navigating somewhere new drops the forward entries,
//! navigating to the current location does not add a duplicate, and Back and
//! Forward move within the list without changing it.
//!
//! [`HistoryViews`] adds what Dolphin keeps and app.js did not: where the
//! view was in each location the tab left, so Back and Forward return to
//! the same scroll position and current item (NAV-008).
//!
//! A search is a step too, as Windows 11 File Explorer's "Search Results
//! in Documents" is ([`SearchStep`]): Back from the results returns to the
//! folder, Forward to the results, and Back from a folder opened from them
//! returns to them.

use std::collections::BTreeMap;

use ox_core::location::same_location;
use ox_core::search::{SearchFacets, SearchIn};

use crate::search::SearchScope;

/// Where the view was in a location a tab left.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LeftView {
    /// The vertical scroll position.
    pub scroll: f64,
    /// The URI of the current item: the first selected one, if any.
    pub current: Option<String>,
    /// The folders expanded in place, outermost first (VIEW-035).
    pub expanded: Vec<String>,
}

/// The [`LeftView`] of each history position a tab left.
#[derive(Debug, Clone, Default)]
pub(crate) struct HistoryViews(BTreeMap<usize, LeftView>);

impl HistoryViews {
    /// Remembers `view` for history position `position`.
    pub(crate) fn remember(&mut self, position: usize, view: LeftView) {
        self.0.insert(position, view);
    }

    /// Forgets the views from `position` on, whose entries a new location
    /// replaced.
    pub(crate) fn forget_from(&mut self, position: usize) {
        self.0.split_off(&position);
    }

    /// Takes the view remembered for `position`.
    pub(crate) fn take(&mut self, position: usize) -> Option<LeftView> {
        self.0.remove(&position)
    }
}

/// A search shown as a step of a tab's history, as Windows 11 File
/// Explorer shows "Search Results in Documents": what was searched for
/// and how, so Back and Forward show the results again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchStep {
    /// The search box's text.
    pub query: String,
    /// The folder's scope, or every cached folder.
    pub scope: SearchScope,
    /// Whether names or names and contents are searched.
    pub search_in: SearchIn,
    /// The search options that narrowed what was shown.
    pub facets: SearchFacets,
}

/// One entry of a tab's history: a location, and the search shown there,
/// if any.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Step {
    uri: String,
    search: Option<SearchStep>,
}

impl Step {
    fn location(uri: &str) -> Self {
        Self {
            uri: uri.to_owned(),
            search: None,
        }
    }
}

/// The locations a tab has visited, with the current position.
///
/// `steps` is never empty and `position` always points into it, so
/// [`History::current`] cannot fail. A search of a folder is a step of
/// its own after the folder, as in Windows 11 File Explorer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct History {
    steps: Vec<Step>,
    position: usize,
    /// How many times the current location changed, so work started
    /// before a navigation can tell its result is stale.
    moves: u64,
}

impl History {
    /// A history holding only `uri`.
    pub(crate) fn new(uri: &str) -> Self {
        Self {
            steps: vec![Step::location(uri)],
            position: 0,
            moves: 0,
        }
    }

    /// A history of `entries` at `position`, as a saved session kept it;
    /// `None` when there are no entries or `position` is past them.
    pub(crate) fn restored(entries: Vec<String>, position: usize) -> Option<Self> {
        (position < entries.len()).then(|| Self {
            steps: entries
                .into_iter()
                .map(|uri| Step { uri, search: None })
                .collect(),
            position,
            moves: 0,
        })
    }

    /// The location currently shown.
    pub(crate) fn current(&self) -> &str {
        &self.steps[self.position].uri
    }

    /// The search shown at the current step, if it is one.
    pub(crate) fn current_search(&self) -> Option<&SearchStep> {
        self.steps[self.position].search.as_ref()
    }

    /// The index of the current location among the entries.
    pub(crate) fn position(&self) -> usize {
        self.position
    }

    /// How many steps there are.
    pub(crate) fn len(&self) -> usize {
        self.steps.len()
    }

    /// The location of step `index`, and whether it shows a search.
    pub(crate) fn step(&self, index: usize) -> Option<(&str, bool)> {
        let step = self.steps.get(index)?;
        Some((&step.uri, step.search.is_some()))
    }

    /// The search step `index` shows, if it shows one.
    pub(crate) fn search_at(&self, index: usize) -> Option<&SearchStep> {
        self.steps.get(index)?.search.as_ref()
    }

    /// The locations a saved session keeps, oldest first, and the
    /// current one's index among them. Searches are not saved, as Windows
    /// restores folders only: a search step becomes its folder, and is
    /// left out after the same folder.
    pub(crate) fn saved_locations(&self) -> (Vec<String>, usize) {
        let mut locations: Vec<String> = Vec::with_capacity(self.steps.len());
        let mut position = 0;
        for (index, step) in self.steps.iter().enumerate() {
            let repeats = step.search.is_some() && locations.last() == Some(&step.uri);
            if !repeats {
                locations.push(step.uri.clone());
            }
            if index == self.position {
                position = locations.len() - 1;
            }
        }
        (locations, position)
    }

    /// Records a navigation to `uri`. Returns false (and changes nothing)
    /// when `uri` is already the current location and no search is shown
    /// there.
    pub(crate) fn push(&mut self, uri: &str) -> bool {
        let current = &self.steps[self.position];
        if current.uri == uri && current.search.is_none() {
            return false;
        }
        self.add(Step::location(uri));
        self.moves += 1;
        true
    }

    /// Forgets the current location, the last entry, and goes back to the
    /// one before it, as if the navigation to it had never happened.
    /// Returns false (and changes nothing) when the current location is
    /// not the last entry or is the only one.
    pub(crate) fn drop_last(&mut self) -> bool {
        if self.position == 0 || self.position + 1 != self.steps.len() {
            return false;
        }
        self.steps.pop();
        self.position -= 1;
        self.moves += 1;
        true
    }

    /// Records `search` of the current location as a step of its own,
    /// after the location, dropping the forward steps. The location stays
    /// the same, so work started there is not stale.
    pub(crate) fn push_search(&mut self, search: SearchStep) {
        let uri = self.current().to_owned();
        self.add(Step {
            uri,
            search: Some(search),
        });
    }

    fn add(&mut self, step: Step) {
        self.steps.truncate(self.position + 1);
        self.steps.push(step);
        self.position = self.steps.len() - 1;
    }

    /// Sets the search shown at the current step without adding a step:
    /// a search edited where it is shown, or ended there. Returns whether
    /// the step became or stopped being a search.
    pub(crate) fn set_current_search(&mut self, search: Option<SearchStep>) -> bool {
        let step = &mut self.steps[self.position];
        let changes_kind = step.search.is_some() != search.is_some();
        step.search = search;
        changes_kind
    }

    /// Whether the step before the current one is its folder without a
    /// search, where clearing the current step's search goes back to.
    pub(crate) fn follows_its_folder(&self) -> bool {
        let Some(before) = self.position.checked_sub(1).map(|index| &self.steps[index]) else {
            return false;
        };
        before.search.is_none() && same_location(&before.uri, self.current())
    }

    /// Replaces the current location without adding an entry, for example
    /// when a location resolves to a different canonical URI.
    pub(crate) fn replace_current(&mut self, uri: &str) {
        uri.clone_into(&mut self.steps[self.position].uri);
    }

    /// How many times the tab moved to another location: a navigation,
    /// Back or Forward.
    pub(crate) fn moves(&self) -> u64 {
        self.moves
    }

    /// True when Back has somewhere to go.
    pub(crate) fn can_go_back(&self) -> bool {
        self.position > 0
    }

    /// True when Forward has somewhere to go.
    pub(crate) fn can_go_forward(&self) -> bool {
        self.position + 1 < self.steps.len()
    }

    /// Moves `delta` steps (negative for Back) and returns the new current
    /// location, or `None` (without moving) when that is out of range.
    pub(crate) fn go(&mut self, delta: isize) -> Option<&str> {
        let target = self.position.checked_add_signed(delta)?;
        if target >= self.steps.len() {
            return None;
        }
        self.position = target;
        self.moves += 1;
        Some(self.current())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: NAV-001
    #[test]
    fn a_new_history_has_nowhere_to_go() {
        let history = History::new("file:///a");
        assert_eq!(history.current(), "file:///a");
        assert!(!history.can_go_back());
        assert!(!history.can_go_forward());
    }

    /// parity: NAV-005
    #[test]
    fn navigating_to_the_current_location_adds_nothing() {
        let mut history = History::new("file:///a");
        assert!(!history.push("file:///a"));
        assert!(!history.can_go_back());
    }

    /// A navigation that went nowhere is forgotten, and only the last one.
    ///
    /// parity: NAV-001
    #[test]
    fn the_last_navigation_can_be_forgotten() {
        let mut history = History::new("file:///a");
        assert!(!history.drop_last(), "the only location stays");
        history.push("file:///b");
        history.push("file:///c");
        assert!(history.drop_last());
        assert_eq!(history.current(), "file:///b");
        assert!(!history.can_go_forward(), "nothing to go forward to");
        history.go(-1);
        assert!(!history.drop_last(), "not the last entry");
        assert_eq!(history.current(), "file:///a");
        assert_eq!(history.go(1), Some("file:///b"));
        assert_eq!(history.go(1), None, "file:///c was forgotten");
    }

    /// parity: NAV-001, NAV-005
    #[test]
    fn back_and_forward_move_without_changing_entries() {
        let mut history = History::new("file:///a");
        history.push("file:///b");
        history.push("file:///c");
        assert_eq!(history.go(-1), Some("file:///b"));
        assert_eq!(history.go(-1), Some("file:///a"));
        assert_eq!(history.go(-1), None);
        assert_eq!(history.current(), "file:///a");
        assert_eq!(history.go(2), Some("file:///c"));
        assert_eq!(history.go(1), None);
    }

    /// parity: NAV-005
    #[test]
    fn navigating_after_back_drops_forward_entries() {
        let mut history = History::new("file:///a");
        history.push("file:///b");
        history.push("file:///c");
        history.go(-2);
        history.push("file:///d");
        assert!(!history.can_go_forward());
        assert_eq!(history.go(-1), Some("file:///a"));
        assert_eq!(history.go(1), Some("file:///d"));
    }

    fn search_for(query: &str) -> SearchStep {
        SearchStep {
            query: query.to_owned(),
            scope: SearchScope::default(),
            search_in: SearchIn::default(),
            facets: SearchFacets::default(),
        }
    }

    /// parity: SRCH-042
    #[test]
    fn a_search_is_a_step_after_its_folder() {
        let mut history = History::new("file:///a");
        let moves = history.moves();

        history.push_search(search_for("notes"));

        assert_eq!(history.current(), "file:///a");
        assert_eq!(history.current_search(), Some(&search_for("notes")));
        assert!(history.follows_its_folder());
        assert_eq!(history.moves(), moves, "the folder shown is the same");
        assert!(!history.set_current_search(Some(search_for("notes 2"))));
        assert!(history.push("file:///a"), "the folder again is a step of its own");
        assert_eq!(history.current_search(), None);
        history.push("file:///a/b");
        assert_eq!(history.go(-2), Some("file:///a"));
        assert_eq!(history.current_search(), Some(&search_for("notes 2")));
        assert_eq!(history.go(-1), Some("file:///a"));
        assert_eq!(history.current_search(), None);
    }

    /// parity: SRCH-042
    #[test]
    fn a_saved_history_keeps_folders_not_searches() {
        let mut history = History::new("file:///a");
        history.push_search(search_for("notes"));
        history.push("file:///b");
        history.push_search(search_for("report"));
        history.go(-1);

        assert_eq!(
            history.saved_locations(),
            (vec!["file:///a".to_owned(), "file:///b".to_owned()], 1)
        );
    }

    #[test]
    fn replacing_keeps_the_position() {
        let mut history = History::new("file:///a");
        history.push("smb://nas/share");
        history.replace_current("smb://nas/share/");
        assert_eq!(history.current(), "smb://nas/share/");
        assert_eq!(history.go(-1), Some("file:///a"));
    }
}
