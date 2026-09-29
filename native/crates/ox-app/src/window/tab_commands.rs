// SPDX-License-Identifier: AGPL-3.0-only
//! Tab commands Dolphin has beyond opening, closing and switching:
//! Alt+1…9 and Alt+0 (TAB-007), Close other tabs (TAB-015) and reopening
//! closed tabs (TAB-016).
//!
//! A closed tab is remembered with its history, selection and scroll
//! position, most recent first, as Dolphin's "Recently Closed Tabs" does.
//! Ctrl+Shift+T reopens the most recent one where it was; the open-windows
//! menu lists them all ([`super::title_bar`]). A tab that moved to another
//! window was not closed and is not remembered.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};

use crate::history::History;

use super::actions::{plain_action, tab_action};
use super::session::TabId;
use super::window_action::WindowAction;
use super::BrowserWindow;

/// How many closed tabs a window remembers.
const MAX_CLOSED_TABS: usize = 10;

/// A closed tab, as it was when it closed.
#[derive(Debug, Clone)]
pub(super) struct ClosedTab {
    /// Where it had been and where it was.
    history: History,
    /// The URIs of the items it had selected.
    selected: Vec<String>,
    /// Its vertical scroll position.
    scroll: f64,
    /// Its place in the strip, counted from the left.
    index: usize,
}

impl ClosedTab {
    /// The location the tab showed.
    pub(super) fn uri(&self) -> &str {
        self.history.current()
    }
}

impl BrowserWindow {
    /// Adds `win.show-tab-number`, `win.close-other-tabs`,
    /// `win.reopen-closed-tab` and `win.restore-closed-tab`.
    pub(super) fn install_tab_commands(&self) {
        let show_number = gio::ActionEntry::builder(WindowAction::ShowTabNumber.name())
            .parameter_type(Some(glib::VariantTy::UINT32))
            .activate(|window: &BrowserWindow, _, target| {
                if let Some(number) = target.and_then(glib::Variant::get::<u32>) {
                    window.show_tab_number(number);
                }
            })
            .build();
        let restore = gio::ActionEntry::builder(WindowAction::RestoreClosedTab.name())
            .parameter_type(Some(glib::VariantTy::UINT32))
            .activate(|window: &BrowserWindow, _, target| {
                let index = target.and_then(glib::Variant::get::<u32>);
                if let Some(index) = index.and_then(|index| usize::try_from(index).ok()) {
                    window.reopen_closed_tab(index);
                }
            })
            .build();
        self.add_action_entries([
            show_number,
            restore,
            tab_action(WindowAction::CloseOtherTabs, BrowserWindow::close_other_tabs),
            plain_action(WindowAction::ReopenClosedTab, |window| {
                window.reopen_closed_tab(0);
            }),
        ]);
    }

    /// Shows tab `number`, counted from 1; 0 shows the last tab
    /// (Alt+1…Alt+9, Alt+0). A number past the last tab does nothing.
    fn show_tab_number(&self, number: u32) {
        let id = {
            let session = self.imp().session.borrow();
            let tabs = session.tabs();
            let index = match number {
                0 => tabs.len().checked_sub(1),
                _ => usize::try_from(number - 1).ok(),
            };
            index.and_then(|index| tabs.get(index)).map(|tab| tab.id)
        };
        if let Some(id) = id {
            self.switch_tab(id);
        }
    }

    /// Closes every tab but `keep`, which comes to the front.
    fn close_other_tabs(&self, keep: TabId) {
        let others: Vec<TabId> = {
            let session = self.imp().session.borrow();
            if session.tab(keep).is_none() {
                return;
            }
            session
                .tabs()
                .iter()
                .map(|tab| tab.id)
                .filter(|id| *id != keep)
                .collect()
        };
        self.switch_tab(keep);
        for id in others {
            self.close_tab(id);
        }
    }

    /// Remembers tab `id` as it is now, before it closes.
    pub(super) fn remember_closed_tab(&self, id: TabId) {
        self.save_tab_view();
        let closed = {
            let session = self.imp().session.borrow();
            let Some(index) = session.tabs().iter().position(|tab| tab.id == id) else {
                return;
            };
            let tab = &session.tabs()[index];
            ClosedTab {
                history: tab.history.clone(),
                selected: tab.selected.clone(),
                scroll: tab.scroll,
                index,
            }
        };
        let mut closed_tabs = self.imp().closed_tabs.borrow_mut();
        closed_tabs.insert(0, closed);
        closed_tabs.truncate(MAX_CLOSED_TABS);
    }

    /// The closed tabs, most recent first.
    pub(super) fn closed_tabs(&self) -> Vec<ClosedTab> {
        self.imp().closed_tabs.borrow().clone()
    }

    /// Opens closed tab `index` (0 is the most recent) again where it was,
    /// in front, with its history, selection and scroll position.
    fn reopen_closed_tab(&self, index: usize) {
        let closed = {
            let mut closed_tabs = self.imp().closed_tabs.borrow_mut();
            if index >= closed_tabs.len() {
                return;
            }
            closed_tabs.remove(index)
        };
        self.save_tab_view();
        let before = self
            .imp()
            .session
            .borrow()
            .tabs()
            .get(closed.index)
            .map(|tab| tab.id);
        let id = self
            .imp()
            .session
            .borrow_mut()
            .insert_moved(closed.history, before);
        if let Some(tab) = self.imp().session.borrow_mut().tab_mut(id) {
            tab.selected = closed.selected;
            tab.scroll_after_listing = Some(closed.scroll);
        }
        self.show_tab(id);
    }
}
