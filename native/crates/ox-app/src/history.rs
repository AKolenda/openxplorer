// SPDX-License-Identifier: AGPL-3.0-only
//! Per-tab navigation history.
//!
//! Ports the history handling of `navigate` and `goHistory` in
//! `desktop/ui/app.js`: navigating somewhere new drops the forward entries,
//! navigating to the current location does not add a duplicate, and Back and
//! Forward move within the list without changing it.
//!
//! [`HistoryViews`] adds what Dolphin keeps and app.js did not: where the
//! view was in each location the tab left, so Back and Forward return to
//! the same scroll position and current item (NAV-008).

use std::collections::BTreeMap;

/// Where the view was in a location a tab left.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LeftView {
    /// The vertical scroll position.
    pub scroll: f64,
    /// The URI of the current item: the first selected one, if any.
    pub current: Option<String>,
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

/// The locations a tab has visited, with the current position.
///
/// `entries` is never empty and `position` always points into it, so
/// [`History::current`] cannot fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct History {
    entries: Vec<String>,
    position: usize,
}

impl History {
    /// A history holding only `uri`.
    pub(crate) fn new(uri: &str) -> Self {
        Self {
            entries: vec![uri.to_string()],
            position: 0,
        }
    }

    /// The location currently shown.
    pub(crate) fn current(&self) -> &str {
        &self.entries[self.position]
    }

    /// The index of the current location among the entries.
    pub(crate) fn position(&self) -> usize {
        self.position
    }

    /// Every location, oldest first.
    pub(crate) fn entries(&self) -> &[String] {
        &self.entries
    }

    /// Records a navigation to `uri`. Returns false (and changes nothing)
    /// when `uri` is already the current location.
    pub(crate) fn push(&mut self, uri: &str) -> bool {
        if self.current() == uri {
            return false;
        }
        self.entries.truncate(self.position + 1);
        self.entries.push(uri.to_string());
        self.position = self.entries.len() - 1;
        true
    }

    /// Replaces the current location without adding an entry, for example
    /// when a location resolves to a different canonical URI.
    pub(crate) fn replace_current(&mut self, uri: &str) {
        self.entries[self.position] = uri.to_string();
    }

    /// True when Back has somewhere to go.
    pub(crate) fn can_go_back(&self) -> bool {
        self.position > 0
    }

    /// True when Forward has somewhere to go.
    pub(crate) fn can_go_forward(&self) -> bool {
        self.position + 1 < self.entries.len()
    }

    /// Moves `delta` steps (negative for Back) and returns the new current
    /// location, or `None` (without moving) when that is out of range.
    pub(crate) fn go(&mut self, delta: isize) -> Option<&str> {
        let target = self.position.checked_add_signed(delta)?;
        if target >= self.entries.len() {
            return None;
        }
        self.position = target;
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

    #[test]
    fn replacing_keeps_the_position() {
        let mut history = History::new("file:///a");
        history.push("smb://nas/share");
        history.replace_current("smb://nas/share/");
        assert_eq!(history.current(), "smb://nas/share/");
        assert_eq!(history.go(-1), Some("file:///a"));
    }
}
