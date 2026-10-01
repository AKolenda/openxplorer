// SPDX-License-Identifier: AGPL-3.0-only
//! Split tabs (VIEW-059): a tab that shows two panes side by side, each
//! with its own location, history, selection and listing.
//!
//! Ports `DolphinTabPage::setSplitViewEnabled` and its active view. Each
//! pane is a [`Tab`] of its own. The active pane stands in the strip for
//! its tab, so everything that acts on the active tab (the address bar,
//! the commands, the status bar, listing and history) acts on the active
//! pane; the other pane waits beside it in [`Tab::beside`]. Making the
//! other pane active swaps the two, as Dolphin swaps its active view
//! container, and the tab takes the id of its active pane.

use super::{Session, Tab, TabId};

/// Which of the window's two folder panes shows a pane of a tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::window) enum PaneSide {
    /// The left pane, which a tab that is not split uses.
    #[default]
    Start,
    /// The right pane of a split tab.
    End,
}

impl PaneSide {
    /// The other side.
    pub(in crate::window) const fn other(self) -> Self {
        match self {
            PaneSide::Start => PaneSide::End,
            PaneSide::End => PaneSide::Start,
        }
    }
}

impl Tab {
    /// The pane beside this one, while the tab is split and this pane is
    /// the active one.
    pub(in crate::window) fn beside(&self) -> Option<&Tab> {
        self.beside.as_deref()
    }

    /// The pane beside this one, to change it.
    pub(in crate::window) fn beside_mut(&mut self) -> Option<&mut Tab> {
        self.beside.as_deref_mut()
    }

    /// Whether the tab shows two panes.
    pub(in crate::window) fn is_split(&self) -> bool {
        self.beside.is_some()
    }
}

impl Session {
    /// The index of the active tab in the strip.
    fn active_index(&self) -> Option<usize> {
        let active = self.active?;
        self.tabs.iter().position(|tab| tab.id == active)
    }

    /// Splits the active tab: a new pane at `uri` opens beside the active
    /// pane and becomes active, as Dolphin's Split does. `None` when no tab
    /// is open or the active one is split already.
    ///
    /// # Panics
    ///
    /// Only after `u64::MAX` tabs in one window.
    pub(in crate::window) fn split_active(&mut self, uri: &str) -> Option<TabId> {
        let index = self.active_index()?;
        if self.tabs[index].is_split() {
            return None;
        }
        let id = self.next_tab_id();
        let mut pane = Tab::new(id, uri);
        pane.side = self.tabs[index].side.other();
        let left = std::mem::replace(&mut self.tabs[index], pane);
        self.tabs[index].beside = Some(Box::new(left));
        self.active = Some(id);
        Some(id)
    }

    /// Makes the pane beside the active one active; its id, or `None` when
    /// the active tab is not split.
    pub(in crate::window) fn activate_beside(&mut self) -> Option<TabId> {
        let index = self.active_index()?;
        let slot = &mut self.tabs[index];
        let mut beside = slot.beside.take()?;
        std::mem::swap(slot, &mut beside);
        slot.beside = Some(beside);
        self.active = Some(slot.id);
        self.active
    }

    /// Closes the active pane of a split tab; the pane beside it stays,
    /// alone, in the start pane, as "Turning off split view closes the
    /// active pane" in Dolphin. Its id, or `None` when the active tab is
    /// not split.
    pub(in crate::window) fn close_active_pane(&mut self) -> Option<TabId> {
        let index = self.active_index()?;
        let mut remaining = *self.tabs[index].beside.take()?;
        remaining.side = PaneSide::Start;
        self.tabs[index] = remaining;
        self.active = Some(self.tabs[index].id);
        self.active
    }

    /// The id of the pane beside the active one, while the active tab is
    /// split.
    pub(in crate::window) fn beside_active(&self) -> Option<TabId> {
        self.active().and_then(Tab::beside).map(|beside| beside.id)
    }
}

#[cfg(test)]
mod tests {
    use super::super::TabPlacement;
    use super::*;

    /// Splitting opens a second pane in front with its own history; making
    /// the first one active again swaps them without changing either, and
    /// closing the split keeps the pane that was not active.
    ///
    /// parity: VIEW-059
    #[test]
    fn each_pane_of_a_split_tab_keeps_its_own_history() {
        let mut session = Session::default();
        let first = session.add("file:///one", TabPlacement::Foreground);

        let second = session.split_active("file:///one").expect("a tab is open");
        session
            .active_mut()
            .expect("the new pane")
            .history
            .push("file:///two");
        let back = session.activate_beside();

        assert_eq!(back, Some(first));
        assert_eq!(session.tabs().len(), 1, "the panes share one tab");
        assert_eq!(session.beside_active(), Some(second));
        let beside = session.tab(second).expect("the second pane is open");
        assert_eq!((beside.uri(), beside.side), ("file:///two", PaneSide::End));
        assert_eq!(
            session.split_active("file:///one"),
            None,
            "a split tab splits once"
        );

        assert_eq!(session.close_active_pane(), Some(second));
        let left = session.active().expect("one pane is left");
        assert_eq!(
            (left.uri(), left.side, left.is_split()),
            ("file:///two", PaneSide::Start, false)
        );
        assert!(session.tab(first).is_none(), "the closed pane is gone");
    }
}
