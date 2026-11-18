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
//!
//! An app ended without closing its windows (SIGTERM at logout or
//! shutdown, or killed) never closes its last window, so the window in
//! front also saves its tabs while it is open: every
//! [`SESSION_SAVE_SECONDS`] seconds when they changed, written off the
//! main thread, and at once when the session manager asks the app to end
//! (`GtkApplication`'s `query-end`, [`save_front_session`]).

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::session::{
    SavedSession, SavedTab, SortDirection, SortField, TabSnapshot, MAX_HISTORY_ENTRIES, MAX_SAVED_TABS,
    MAX_SCROLL, MAX_SELECTED_ITEMS,
};
use ox_core::settings::View;

use crate::history::History;
use crate::locations::Page;

use super::session::{PaneSide, Tab, TabId};
use super::BrowserWindow;

/// How often, in seconds, the window in front saves its tabs while they
/// change.
const SESSION_SAVE_SECONDS: u32 = 3;

/// The number the next snapshot of a window's tabs takes, in the order
/// the main thread reads them.
static NEXT_SNAPSHOT: AtomicU64 = AtomicU64::new(1);

/// The number of the newest snapshot written, locked while one is
/// written: writes never overlap, and an older snapshot never replaces a
/// newer one, whichever write finishes first.
static WRITTEN_SNAPSHOT: Mutex<u64> = Mutex::new(0);

/// Writes `saved`, snapshot `number`, to `directory`, after any write
/// still running, unless a newer snapshot is written already.
fn write_snapshot(saved: &SavedSession, number: u64, directory: &Path) -> Result<(), String> {
    let mut written = WRITTEN_SNAPSHOT.lock().unwrap_or_else(PoisonError::into_inner);
    if *written > number {
        return Ok(());
    }
    saved.save(directory).map_err(|error| error.to_string())?;
    *written = number;
    Ok(())
}

/// Writes the tabs of `window` now, after any write still running.
fn save_now(window: &BrowserWindow) {
    let Some(saved) = window.saved_session() else {
        return;
    };
    let number = NEXT_SNAPSHOT.fetch_add(1, Ordering::Relaxed);
    if let Err(error) = write_snapshot(&saved, number, &window.context().settings_directory()) {
        glib::g_warning!(ox_core::LOG_DOMAIN, "The open tabs could not be saved: {error}");
    }
}

/// The entries of `history` a saved session keeps: at most
/// [`MAX_HISTORY_ENTRIES`] locations around the current one, without its
/// searches ([`History::saved_locations`]), and its position among them.
fn bounded_history(history: &History) -> (Vec<String>, usize) {
    let (entries, position) = history.saved_locations();
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
        save_now(self);
    }

    /// Saves the window's tabs every [`SESSION_SAVE_SECONDS`] seconds
    /// while it is the browsing window in front and they changed, so an
    /// app ended by a signal keeps them (TAB-053).
    pub(super) fn keep_session_saved(&self) {
        let window = self.downgrade();
        glib::timeout_add_seconds_local(SESSION_SAVE_SECONDS, move || {
            let Some(window) = window.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if window.saves_session() && window.is_front_window() {
                window.save_session_in_background();
            }
            glib::ControlFlow::Continue
        });
    }

    /// Whether the settings ask for the tabs to be reopened and this is a
    /// browsing window of the user's that is still shown.
    fn saves_session(&self) -> bool {
        let preferences = self.context().settings_data().preferences;
        preferences.restore_session && self.imp().remembers_session.get() && self.is_visible()
    }

    /// Whether this window is the browsing window focused last, whose tabs
    /// the next start reopens.
    fn is_front_window(&self) -> bool {
        let Some(app) = self.application() else {
            return true;
        };
        let front = app
            .windows()
            .into_iter()
            .find(|window| window.downcast_ref::<BrowserWindow>().is_some());
        front.is_none_or(|front| &front == self.upcast_ref::<gtk::Window>())
    }

    /// Writes the window's tabs on a worker thread when they differ from
    /// what this window saved last and no other window saved since.
    pub(super) fn save_session_in_background(&self) {
        let Some(saved) = self.saved_session() else {
            return;
        };
        let text = saved.to_json().to_string();
        if self.imp().saved_session_text.borrow().as_deref() == Some(text.as_str()) {
            return;
        }
        // Remembered while the write runs, so the next tick does not start
        // the same write again; forgotten if it fails, so a later tick
        // tries again once the storage works (a full disk, for example).
        self.imp().saved_session_text.replace(Some(text.clone()));
        self.forget_other_saves();
        let number = NEXT_SNAPSHOT.fetch_add(1, Ordering::Relaxed);
        let directory = self.context().settings_directory();
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            let written = gio::spawn_blocking(move || write_snapshot(&saved, number, &directory)).await;
            let failure = match written {
                Ok(Ok(())) => return,
                Ok(Err(error)) => error,
                Err(_) => "the saving thread stopped".to_owned(),
            };
            glib::g_warning!(ox_core::LOG_DOMAIN, "The open tabs could not be saved: {failure}");
            if let Some(window) = window.upgrade() {
                let remembered = &window.imp().saved_session_text;
                if remembered.borrow().as_deref() == Some(text.as_str()) {
                    remembered.replace(None);
                }
            }
        });
    }

    /// Forgets what every other browsing window saved last: the session
    /// file holds this window's tabs now, so theirs are saved again once
    /// one of them is in front, even unchanged.
    fn forget_other_saves(&self) {
        let Some(app) = self.application() else {
            return;
        };
        for window in app.windows() {
            if let Some(other) = window
                .downcast_ref::<BrowserWindow>()
                .filter(|other| *other != self)
            {
                other.imp().saved_session_text.replace(None);
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

/// Saves the tabs of `app`'s browsing window in front at once, when the
/// settings ask for that: the session manager is about to end the session
/// (logout or shutdown), and the app may be ended before its windows
/// close.
pub(crate) fn save_front_session(app: &gtk::Application) {
    let front = app
        .windows()
        .into_iter()
        .filter_map(|window| window.downcast::<BrowserWindow>().ok())
        .find(BrowserWindow::saves_session);
    let Some(window) = front else {
        return;
    };
    window.forget_other_saves();
    save_now(&window);
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
