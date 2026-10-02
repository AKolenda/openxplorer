// SPDX-License-Identifier: AGPL-3.0-only
//! Stopping a slow listing (VIEW-049). While the loading line shows, the
//! Refresh button before the address bar turns into Stop, as Windows
//! Explorer's does, and `win.stop` ends the listing, keeping the items
//! that arrived, as Dolphin's View › Stop does. No "Loading" text comes
//! back: the owner decided a slow listing shows only the line (VIEW-047);
//! a stop says so once in the message line.

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::actions::plain_action;
use super::window_action::WindowAction;
use super::BrowserWindow;
use crate::icons::{self, Icon};

/// The glyph of the button before the address bar.
const BUTTON_GLYPH: i32 = 16;

/// What the message line says after a stop.
const STOPPED: &str = "Stopped. Some items may be missing; press F5 to list the folder again.";

impl BrowserWindow {
    /// Adds `win.stop`.
    pub(super) fn install_stop_action(&self) {
        self.add_action_entries([plain_action(WindowAction::Stop, BrowserWindow::stop_listing)]);
    }

    /// Shows Stop in place of Refresh while the loading line shows, and
    /// Refresh again once it is gone.
    pub(super) fn show_stop_or_refresh(&self) {
        let loading = self.folder_pane().shows_loading_line();
        let row = &*self.imp().navigation_buttons;
        let buttons = std::iter::successors(row.first_child(), WidgetExt::next_sibling);
        let wanted = [WindowAction::Refresh, WindowAction::Stop].map(WindowAction::detailed_name);
        let Some(button) = buttons
            .filter_map(|child| child.downcast::<gtk::Button>().ok())
            .find(|button| {
                button
                    .action_name()
                    .is_some_and(|name| wanted.contains(&name.to_string()))
            })
        else {
            return;
        };
        let (glyph, name, tooltip, action) = if loading {
            (Icon::Dismiss, "Stop", "Stop", WindowAction::Stop)
        } else {
            (
                Icon::ArrowClockwise,
                "Refresh",
                "Refresh (F5)",
                WindowAction::Refresh,
            )
        };
        if button.action_name().as_deref() == Some(action.detailed_name().as_str()) {
            return;
        }
        button.set_child(Some(&icons::image(glyph, BUTTON_GLYPH)));
        button.set_tooltip_text(Some(tooltip));
        button.update_property(&[gtk::accessible::Property::Label(name)]);
        action.assign_to(&button);
    }

    /// Ends the active tab's running listing, keeping the items that
    /// arrived; does nothing when no listing runs.
    pub(super) fn stop_listing(&self) {
        let stopped = {
            let mut session = self.imp().session.borrow_mut();
            let Some(tab) = session.active_mut() else { return };
            if !tab.listing_state.is_listing() {
                return;
            }
            // Dropping the listing cancels it and everything it would
            // still deliver.
            tab.listing = None;
            tab.reloading = false;
            tab.listing_state.finish();
            true
        };
        if stopped {
            self.update_content();
            self.show_message(STOPPED);
        }
    }
}
