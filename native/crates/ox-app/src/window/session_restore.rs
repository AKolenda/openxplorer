// SPDX-License-Identifier: AGPL-3.0-only
//! The window's tabs saved for the next start, and reopened then
//! (TAB-053).
//!
//! Ports Dolphin's `RememberOpenedTabs` (`DolphinTabWidget::saveState`
//! and `restoreState`) as Explorer's "Restore previous folder windows at
//! logon" offers it: off unless the settings turn it on. The last window
//! that closes saves its tabs, the panes of split tabs, their histories,
//! selections and scroll positions; a start without locations opens them
//! again. The Settings tab is not saved.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::session::{
    SavedSession, SavedTab, SortDirection, SortField, TabSnapshot, MAX_HISTORY_ENTRIES, MAX_SAVED_TABS,
    MAX_SCROLL, MAX_SELECTED_ITEMS,
};
use ox_core::settings::View;

use crate::history::History;
use crate::locations::Page;

use super::session::{PaneSide, Tab, TabId};
use super::BrowserWindow;

/// The entries of `history` a saved session keeps: at most
/// [`MAX_HISTORY_ENTRIES`] around the current one, and its position
/// among them.
fn bounded_history(history: &History) -> (Vec<String>, usize) {
    let entries = history.entries();
    let position = history.position();
    let start = (position + 1).saturating_sub(MAX_HISTORY_ENTRIES);
    let end = (start + MAX_HISTORY_ENTRIES).min(entries.len());
    (entries[start..end].to_vec(), position - start)
}

/// The saved form of the pane `tab`.
fn snapshot(tab: &Tab) -> TabSnapshot {
    let (history, index) = bounded_history(&tab.history);
    let mut selection = tab.selected.clone();
    selection.truncate(MAX_SELECTED_ITEMS);
    TabSnapshot {
        uri: tab.uri().to_owned(),
        history,
        index,
        scroll: tab.scroll.clamp(0.0, MAX_SCROLL),
        selection,
        view: View::default(),
        sort: SortField::default(),
        direction: SortDirection::default(),
        settings_section: None,
    }
}

/// The saved form of the tab whose active pane is `tab`: its panes left
/// first, and which of them is active.
fn saved_tab(tab: &Tab) -> SavedTab {
    let mut panes = vec![(tab.side, snapshot(tab))];
    if let Some(beside) = tab.beside() {
        panes.push((beside.side, snapshot(beside)));
    }
    panes.sort_by_key(|(side, _)| *side == PaneSide::End);
    let active_pane = panes.iter().position(|(side, _)| *side == tab.side).unwrap_or(0);
    SavedTab {
        panes: panes.into_iter().map(|(_, pane)| pane).collect(),
        active_pane,
    }
}

impl BrowserWindow {
    /// The window's tabs as a session to reopen; `None` when it shows only
    /// Settings.
    pub(crate) fn saved_session(&self) -> Option<SavedSession> {
        self.save_tab_view();
        self.save_beside_view();
        let session = self.imp().session.borrow();
        let kept = session
            .tabs()
            .iter()
            .filter(|tab| Page::from_uri(tab.uri()) != Some(Page::Settings))
            .take(MAX_SAVED_TABS);
        let mut active_tab = 0;
        let mut tabs = Vec::new();
        for tab in kept {
            if session.is_active(tab.id) {
                active_tab = tabs.len();
            }
            tabs.push(saved_tab(tab));
        }
        (!tabs.is_empty()).then_some(SavedSession { tabs, active_tab })
    }

    /// Saves the window's tabs for the next start, when it is the last
    /// window open and the settings ask for that.
    pub(super) fn save_session_if_last(&self) {
        let preferences = self.context().settings_data().preferences;
        if !(preferences.restore_session && self.imp().remembers_session.get()) {
            return;
        }
        let others_open = self.application().is_some_and(|app| {
            app.windows()
                .iter()
                .any(|window| window != self.upcast_ref::<gtk::Window>() && window.is::<BrowserWindow>())
        });
        if others_open {
            return;
        }
        if let Some(saved) = self.saved_session() {
            if let Err(error) = saved.save(&self.context().settings_directory()) {
                glib::g_warning!(ox_core::LOG_DOMAIN, "The open tabs could not be saved: {error}");
            }
        }
    }

    /// Opens the tabs of `saved` in this window, which has none yet, and
    /// shows the one that was in front.
    pub(crate) fn restore_session(&self, saved: &SavedSession) {
        let mut front = None;
        for (index, tab) in saved.tabs.iter().enumerate() {
            let id = self.restore_tab(tab);
            if index == saved.active_tab {
                front = id;
            }
        }
        let shown = front.or_else(|| self.imp().session.borrow().active_id());
        if let Some(id) = shown {
            self.imp().session.borrow_mut().activate(id);
            self.show_tab(id);
        }
    }

    /// Adds the tab `saved` at the end, split when it was, without listing
    /// it; the id it has in the strip.
    fn restore_tab(&self, saved: &SavedTab) -> Option<TabId> {
        let mut session = self.imp().session.borrow_mut();
        let mut panes = saved.panes.iter();
        let left = panes.next()?;
        let history = History::restored(left.history.clone(), left.index)?;
        let id = session.insert_moved(history, None);
        if let Some(tab) = session.tab_mut(id) {
            restore_view(tab, left);
        }
        let right = panes.next().and_then(|right| {
            let history = History::restored(right.history.clone(), right.index)?;
            let id = session.split_active(&right.uri)?;
            Some((right, history, id))
        });
        if let Some((right, history, id)) = right {
            if let Some(tab) = session.tab_mut(id) {
                tab.history = history;
                restore_view(tab, right);
            }
            if saved.active_pane == 0 {
                session.activate_beside();
            }
        }
        session.active_id()
    }
}

/// Gives the restored pane `tab` the selection and scroll position of
/// `saved`, for its first listing.
fn restore_view(tab: &mut Tab, saved: &TabSnapshot) {
    tab.selected.clone_from(&saved.selection);
    tab.scroll_after_listing = Some(saved.scroll);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::{application, Fixture, TestWindow};

    /// The window's tabs, a split one included, come back in a new window
    /// in their order, with the tab and the pane that were in front.
    ///
    /// parity: TAB-053
    #[gtk::test]
    fn a_saved_window_reopens_its_tabs_and_split_panes() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        test.window
            .add_tab(&fixture.uri_of("Documents"))
            .expect("a folder");
        test.wait_for_listing("the second tab");
        test.window.split_tab(Some(&fixture.uri())).expect("a folder");
        let saved = test.window.saved_session().expect("tabs to save");

        let restored = BrowserWindow::new(&application(), test.window.context());
        restored.restore_session(&saved);

        let session = restored.imp().session.borrow();
        let uris: Vec<&str> = session.tabs().iter().map(Tab::uri).collect();
        assert_eq!(uris, [fixture.uri(), fixture.uri()]);
        let front = session.active().expect("a tab in front");
        assert_eq!(front.side, PaneSide::End);
        let beside = front.beside().expect("the second tab is split");
        assert_eq!(beside.uri(), fixture.uri_of("Documents"));
        drop(session);
        restored.destroy();
    }
}
