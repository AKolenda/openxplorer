// SPDX-License-Identifier: AGPL-3.0-only
//! Tab and Shift+Tab: keyboard focus through the window in Windows File
//! Explorer's order (ACC-016).
//!
//! Microsoft's tip for Windows 7 ("Navigate Windows Explorer More Quickly
//! with These Keyboard Shortcuts", archived on Microsoft Learn) gives
//! Tab's order as the address bar, the search box, the toolbar, the
//! navigation pane, the file list and its column headings; Microsoft's
//! current list of Windows 11 shortcuts says F6 cycles through the
//! elements of the window. Where Windows 11 puts its tabs and its Back,
//! Forward, Up and Refresh buttons in that order is not documented; here
//! they come just before the address bar, where they sit. GTK's own order
//! follows the widget tree instead: from the file list it went to the
//! status bar's view buttons and then to the title bar's caption buttons,
//! and every row of the navigation pane was a stop of its own.
//!
//! Here Tab visits, in order and wrapping round: the address bar (the
//! chevron or «, each crumb shown, then the history chevron; or the
//! entry), the search box, the command bar, the navigation pane (one stop,
//! the arrows move inside it), the folder tree, Map network drive, the
//! file list (one stop, on its current item; each pane of a split tab),
//! the Details pane, the status bar's buttons, the Open or Save bar, the
//! tab in front and its Close button, New tab, and the Back, Forward, Up
//! and Refresh buttons, which lead back into the address bar as they sit
//! before it.
//! The caption buttons, which Windows draws outside the window's own
//! controls, are no stops. A hidden or disabled stop is skipped.
//!
//! GTK emits `move-focus` for Tab only once the focused widget and its
//! parents let the key go, so what takes Tab itself keeps it: Tab between
//! split panes (VIEW-059), the fields of a rename in place, the message
//! and Try again of "This location is unavailable". Dialogs, menus and
//! the Settings page keep GTK's order.

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::session::PaneSide;
use super::widget_tree::descendants;
use super::BrowserWindow;

/// A stop of the Tab order, in the order Tab visits them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stop {
    Address,
    Search,
    Commands,
    Places,
    FolderTree,
    MapNetworkDrive,
    Pane(PaneSide),
    Details,
    Status,
    Picker,
    Tabs,
    NewTab,
    Navigation,
}

impl Stop {
    /// Every stop, in Tab's order.
    pub(super) const ALL: [Stop; 14] = [
        Stop::Address,
        Stop::Search,
        Stop::Commands,
        Stop::Places,
        Stop::FolderTree,
        Stop::MapNetworkDrive,
        Stop::Pane(PaneSide::Start),
        Stop::Pane(PaneSide::End),
        Stop::Details,
        Stop::Status,
        Stop::Picker,
        Stop::Tabs,
        Stop::NewTab,
        Stop::Navigation,
    ];
}

/// The stops after `current` (before it `backward`), each once, wrapping
/// round to `current` itself last; every stop from the first (the last
/// `backward`) when focus is at none.
fn stops_after(current: Option<usize>, count: usize, backward: bool) -> Vec<usize> {
    let step = |index: usize| {
        if backward {
            (index + count - 1) % count
        } else {
            (index + 1) % count
        }
    };
    let start = match current {
        Some(index) => step(index),
        None if backward => count.saturating_sub(1),
        None => 0,
    };
    std::iter::successors(Some(start), |index| Some(step(*index)))
        .take(count)
        .collect()
}

impl BrowserWindow {
    /// Makes Tab and Shift+Tab follow [`Stop::ALL`] once the focused
    /// widget let them go.
    pub(super) fn install_tab_order(&self) {
        self.connect_move_focus(|window, direction| {
            let backward = match direction {
                gtk::DirectionType::TabForward => false,
                gtk::DirectionType::TabBackward => true,
                _ => return,
            };
            if window.keeps_gtk_tab_order() {
                return;
            }
            if window.tab_to_next_stop(backward) {
                window.stop_signal_emission_by_name("move-focus");
            }
        });
        for property in ["focus-widget", "focus-visible"] {
            self.connect_notify_local(Some(property), |window, _| {
                for pane in window.folder_panes() {
                    pane.show_keyboard_on_empty_list();
                }
            });
        }
    }

    /// Whether Tab follows GTK's order: while a dialog, a menu or the
    /// Settings page has the keyboard, or a field of the file list does.
    fn keeps_gtk_tab_order(&self) -> bool {
        if self.dialog_layer().shown().is_some() || self.shows_settings() {
            return true;
        }
        let Some(focus) = gtk::prelude::GtkWindowExt::focus(self) else {
            return false;
        };
        let in_menu = focus.ancestor(gtk::Popover::static_type()).is_some();
        let in_field = focus.is::<gtk::Text>() || focus.ancestor(gtk::Text::static_type()).is_some();
        let in_files = self.folder_panes().iter().any(|pane| focus.is_ancestor(*pane));
        in_menu || (in_field && in_files)
    }

    /// Moves keyboard focus to the next control of the Tab order, or the
    /// previous one `backward`; false when nothing can take it.
    pub(super) fn tab_to_next_stop(&self, backward: bool) -> bool {
        let direction = if backward {
            gtk::DirectionType::TabBackward
        } else {
            gtk::DirectionType::TabForward
        };
        let current = self.focused_stop();
        if let Some(stop) = current {
            // Within a stop of several controls, Tab goes to the next one.
            if !self.stop_is_single(stop) && self.stop_widget(stop).child_focus(direction) {
                return true;
            }
        }
        let current = current.and_then(|stop| Stop::ALL.iter().position(|candidate| *candidate == stop));
        stops_after(current, Stop::ALL.len(), backward)
            .into_iter()
            .map(|index| Stop::ALL[index])
            .filter(|stop| self.stop_is_reachable(*stop))
            .any(|stop| self.enter_stop(stop, direction))
    }

    /// The stop holding keyboard focus, if any.
    pub(super) fn focused_stop(&self) -> Option<Stop> {
        let focus = gtk::prelude::GtkWindowExt::focus(self)?;
        Stop::ALL.into_iter().find(|stop| {
            let widget = self.stop_widget(*stop);
            focus == widget || focus.is_ancestor(&widget)
        })
    }

    /// The widget that holds `stop`.
    fn stop_widget(&self, stop: Stop) -> gtk::Widget {
        let imp = self.imp();
        match stop {
            Stop::Address => self.address_bar().clone().upcast(),
            Stop::Search => self.search_box().clone().upcast(),
            Stop::Commands => self.command_bar().clone().upcast(),
            Stop::Places => self.sidebar().list().clone().upcast(),
            Stop::FolderTree => self.sidebar().folder_tree().clone().upcast(),
            Stop::MapNetworkDrive => self.sidebar().map_network_footer(),
            Stop::Pane(side) => self.pane_on(side).clone().upcast(),
            Stop::Details => self.details_pane().clone().upcast(),
            Stop::Status => self.status_bar().clone().upcast(),
            Stop::Picker => imp.picker_bar.get().upcast(),
            Stop::Tabs => self.tab_strip().clone().upcast(),
            Stop::NewTab => imp.new_tab_button.get().upcast(),
            Stop::Navigation => imp.navigation_buttons.get().upcast(),
        }
    }

    /// Whether `stop` is one stop however many items it holds, with the
    /// arrow keys moving among them: the navigation pane, the folder tree
    /// and a list of files, as in Windows.
    fn stop_is_single(&self, stop: Stop) -> bool {
        match stop {
            Stop::Places | Stop::FolderTree => true,
            Stop::Pane(side) => self.pane_on(side).lists_items(),
            _ => false,
        }
    }

    /// Whether `stop` is shown and enabled. Each pane of a split tab is a
    /// stop; the start pane alone otherwise.
    fn stop_is_reachable(&self, stop: Stop) -> bool {
        let widget = self.stop_widget(stop);
        widget.is_mapped() && widget.is_sensitive()
    }

    /// Focuses `stop` from outside it, its first control (its last going
    /// `direction` backward) or its current item; false when it has
    /// nothing to focus.
    fn enter_stop(&self, stop: Stop, direction: gtk::DirectionType) -> bool {
        match stop {
            Stop::Places => {
                let list = self.sidebar().list();
                match list.selected_row() {
                    Some(row) => row.grab_focus(),
                    None => list.child_focus(gtk::DirectionType::TabForward),
                }
            }
            Stop::FolderTree => self
                .sidebar()
                .folder_tree()
                .child_focus(gtk::DirectionType::TabForward),
            // The tab in front, the strip's one stop (ACC-008), before its
            // Close button; GTK would go to the button first.
            Stop::Tabs if direction == gtk::DirectionType::TabForward => {
                let front = descendants::<gtk::Widget>(self.tab_strip())
                    .into_iter()
                    .find(|widget| {
                        widget.has_css_class("tab") && widget.is_focusable() && widget.is_mapped()
                    });
                match front {
                    Some(tab) => tab.grab_focus(),
                    None => self.stop_widget(stop).child_focus(direction),
                }
            }
            Stop::Pane(side) if self.stop_is_single(stop) => {
                let pane = self.pane_on(side);
                pane.focus_view();
                pane.view_has_focus()
            }
            _ => self.stop_widget(stop).child_focus(direction),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: ACC-016
    #[test]
    fn tab_tries_every_other_stop_once_and_wraps_round() {
        assert_eq!(stops_after(Some(1), 4, false), vec![2, 3, 0, 1]);
        assert_eq!(stops_after(Some(1), 4, true), vec![0, 3, 2, 1]);
        assert_eq!(stops_after(None, 3, false), vec![0, 1, 2]);
        assert_eq!(stops_after(None, 3, true), vec![2, 1, 0]);
        assert!(stops_after(None, 0, false).is_empty());
    }
}
