// SPDX-License-Identifier: AGPL-3.0-only
//! Helpers the window tests share: driving tabs as their widgets do, and
//! finding the menus of the command bar and the title bar.

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::test_support::harness::{descendants, TestWindow};
use crate::window::menu_popover::MenuPopover;
use crate::window::session::TabId;
use crate::window::window_action::WindowAction;

impl TestWindow {
    /// The id of the tab in front, if any.
    pub(super) fn active_tab(&self) -> Option<TabId> {
        self.window.imp().session.borrow().active_id()
    }

    /// Shows tab `id`, as clicking it does.
    pub(super) fn activate_tab(&self, id: TabId) {
        self.run_tab_action(WindowAction::SelectTab, id);
    }

    /// Closes tab `id`, as its close button does.
    pub(super) fn activate_tab_close(&self, id: TabId) {
        self.run_tab_action(WindowAction::CloseTabById, id);
    }

    fn run_tab_action(&self, action: WindowAction, id: TabId) {
        let target = id.to_variant();
        WidgetExt::activate_action(&self.window, &action.detailed_name(), Some(&target))
            .expect("the window has the tab actions");
    }
}

/// The menu button in `test`'s window that has the CSS class `class`.
pub(super) fn menu_button_with_class(test: &TestWindow, class: &str) -> gtk::MenuButton {
    descendants::<gtk::MenuButton>(&test.window)
        .into_iter()
        .find(|button| button.has_css_class(class))
        .unwrap_or_else(|| panic!("the window has a .{class} menu button"))
}

/// The app menu `button` opens.
pub(super) fn app_menu(button: &gtk::MenuButton) -> MenuPopover {
    button
        .popover()
        .and_downcast::<MenuPopover>()
        .expect("the button opens an app menu")
}
