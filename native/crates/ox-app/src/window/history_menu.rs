// SPDX-License-Identifier: AGPL-3.0-only
//! The Back and Forward menus, and where Back, Forward and Up lead.
//!
//! app.js had neither; this brings Dolphin's and Explorer's: a right-click
//! or a long press on Back or Forward lists up to twelve locations of the
//! tab's history, nearest first, each named as its tab would be
//! ("Documents"), and choosing one jumps straight there (NAV-006). A
//! middle-click on Back, Forward or Up, or on an entry of their menus,
//! opens where it leads in a new tab and leaves the current tab alone
//! (NAV-007).

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use ox_core::location::parent_location;

use crate::history::History;

use super::gestures;
use super::location_view::address_icon;
use super::menu_popover::{MenuEntry, MenuItem, MenuPopover};
use super::session::Direction;
use super::window_action::WindowAction;
use super::BrowserWindow;

/// The most locations a Back or Forward menu lists (Dolphin's limit).
const MENU_LENGTH: usize = 12;

/// The steps a history menu offers from `history`'s current location in
/// `direction`, nearest first, each with the location it leads to.
fn history_steps(history: &History, direction: Direction) -> Vec<(i32, String)> {
    let position = history.position();
    let entries = history.entries();
    let indices: Vec<usize> = match direction {
        Direction::Backward => (0..position).rev().take(MENU_LENGTH).collect(),
        Direction::Forward => (position + 1..entries.len()).take(MENU_LENGTH).collect(),
    };
    indices
        .into_iter()
        .filter_map(|index| {
            let steps = i32::try_from(index).ok()? - i32::try_from(position).ok()?;
            Some((steps, entries[index].clone()))
        })
        .collect()
}

impl BrowserWindow {
    /// The Back or Forward menu of the active tab: one item per location,
    /// with the icon the address bar would show there.
    #[cfg(test)]
    pub(super) fn history_menu_entries(&self, direction: Direction) -> Vec<MenuEntry> {
        self.history_menu(direction).0
    }

    /// The Back or Forward menu's entries, and the location of each.
    fn history_menu(&self, direction: Direction) -> (Vec<MenuEntry>, Vec<String>) {
        let steps = {
            let session = self.imp().session.borrow();
            let Some(tab) = session.active() else {
                return (Vec::new(), Vec::new());
            };
            history_steps(&tab.history, direction)
        };
        let locations = self.imp().locations.borrow();
        let home = locations.home_uri();
        steps
            .into_iter()
            .map(|(steps, uri)| {
                // As Windows Explorer's menus do, and as the tab is titled.
                let label = locations.title_for(&uri);
                let glyph = address_icon(&uri, &home);
                let item = MenuItem::with_target(&label, glyph, WindowAction::GoHistory, steps.to_variant());
                (item.into(), uri)
            })
            .unzip()
    }

    /// Where Back, Forward, Up or Refresh leads from the active tab, for
    /// a middle-click and the tooltips; `None` where it leads nowhere.
    pub(super) fn navigation_target(&self, action: WindowAction) -> Option<String> {
        let session = self.imp().session.borrow();
        let history = &session.active()?.history;
        let position = history.position();
        match action {
            WindowAction::Back => history.entries().get(position.checked_sub(1)?).cloned(),
            WindowAction::Forward => history.entries().get(position + 1).cloned(),
            WindowAction::Up => parent_location(history.current()),
            WindowAction::Refresh => Some(history.current().to_owned()),
            _ => None,
        }
    }

    /// Gives the Back, Forward or Up `button` its middle-click, and Back
    /// and Forward their menu on a right-click or a long press.
    pub(super) fn add_history_gestures(&self, button: &gtk::Button, action: WindowAction) {
        button.add_controller(gestures::middle_click(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |gesture, _, _| {
                let Some(target) = window.navigation_target(action) else {
                    return;
                };
                let open = gestures::open_action(gesture.current_event_state());
                open.activate_from(&window, Some(&target.to_variant()));
            }
        )));
        let direction = match action {
            WindowAction::Back => Direction::Backward,
            WindowAction::Forward => Direction::Forward,
            _ => return,
        };
        let right_click = gtk::GestureClick::new();
        right_click.set_button(gdk::BUTTON_SECONDARY);
        right_click.connect_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |gesture, _, _, _| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                if let Some(button) = gesture.widget() {
                    window.popup_history_menu(&button, direction);
                }
            }
        ));
        button.add_controller(right_click);
        let long_press = gtk::GestureLongPress::new();
        long_press.connect_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |gesture, _, _| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                if let Some(button) = gesture.widget() {
                    window.popup_history_menu(&button, direction);
                }
            }
        ));
        button.add_controller(long_press);
    }

    /// Opens the Back or Forward menu under `button`; with no history
    /// that way, nothing opens.
    pub(super) fn popup_history_menu(
        &self,
        button: &gtk::Widget,
        direction: Direction,
    ) -> Option<MenuPopover> {
        let (entries, targets) = self.history_menu(direction);
        if entries.is_empty() {
            return None;
        }
        let popover = MenuPopover::new(entries);
        popover.connect_row_middle_click(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |index, modifiers| {
                if let Some(uri) = targets.get(index) {
                    gestures::open_action(modifiers).activate_from(&window, Some(&uri.to_variant()));
                }
            }
        ));
        popover.set_parent(button);
        popover.connect_closed(|popover| {
            let closed = popover.clone();
            glib::idle_add_local_once(move || closed.unparent());
        });
        popover.popup();
        Some(popover)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(entries: &[&str], position: usize) -> History {
        let mut history = History::new(entries[0]);
        for entry in &entries[1..] {
            history.push(entry);
        }
        let back = isize::try_from(entries.len() - 1 - position).expect("a short history");
        history.go(-back);
        history
    }

    /// parity: NAV-006
    #[test]
    fn a_history_menu_lists_twelve_steps_nearest_first() {
        let uris: Vec<String> = (0..20).map(|number| format!("file:///tmp/{number}")).collect();
        let entries: Vec<&str> = uris.iter().map(String::as_str).collect();
        let middle = history(&entries, 15);

        let back = history_steps(&middle, Direction::Backward);
        let forward = history_steps(&middle, Direction::Forward);

        assert_eq!(back.len(), 12);
        assert_eq!(back[0], (-1, "file:///tmp/14".to_owned()));
        assert_eq!(back[11], (-12, "file:///tmp/3".to_owned()));
        assert_eq!(
            forward,
            (16..20)
                .map(|number| (number - 15, format!("file:///tmp/{number}")))
                .collect::<Vec<_>>()
        );
    }
}
