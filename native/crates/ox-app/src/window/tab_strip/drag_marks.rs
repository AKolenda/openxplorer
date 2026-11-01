// SPDX-License-Identifier: AGPL-3.0-only
//! What the strip shows while tabs and files are dragged: which tab a
//! press would drag, where a dropped tab would go, the dragged tab itself,
//! the window that follows the pointer once a tab leaves its strip, and
//! the tab a file drop would go into (TAB-018, TAB-032 to TAB-034).
//!
//! Ports the `.tab-drag-source` rule of `v2.0.0:desktop/ui/style.css`.
//! Where `app.js` marked the insertion point, the tabs now make room for a
//! dragged tab instead ([`super::live_drag`]). The strip answers for points
//! in its own coordinates, so its sideways scrolling needs no arithmetic.

use gtk::graphene;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::TabStrip;
use crate::window::menu_popover::MenuEntry;
use crate::window::session::TabId;
use crate::window::widget_tree::toggle_class;

/// The class of the tab being dragged (`.tab-drag-source`).
const DRAGGED_CLASS: &str = "tab-drag-source";

/// The class of the tab whose folder a file drop would go into (TAB-018).
const FILE_DROP_CLASS: &str = "file-drop-active";

/// The class of a tab's close button, which never starts a drag.
const CLOSE_BUTTON_CLASS: &str = "tab-close";

impl TabStrip {
    /// The tab a press at (`x`, `y`) of the strip would drag: a tab, not
    /// its close button.
    pub(in crate::window) fn draggable_tab_at(&self, x: f64, y: f64) -> Option<TabId> {
        let picked = self.pick(x, y, gtk::PickFlags::DEFAULT)?;
        let on_close_button = std::iter::successors(Some(picked.clone()), WidgetExt::parent)
            .any(|widget| widget.has_css_class(CLOSE_BUTTON_CLASS));
        if on_close_button {
            return None;
        }
        self.tab_at(x, y).map(|tab| tab.id)
    }

    /// The first tab whose middle is right of `x` in the strip: the tab a
    /// tab dropped at `x` goes before, or `None` at the end (`before`).
    /// The tabs' places in the session count, not where a drag draws them.
    pub(in crate::window) fn tab_after(&self, x: f64) -> Option<TabId> {
        let x = self.in_row(x);
        self.tab_slots()
            .into_iter()
            .find_map(|(tab, left, width)| (x < left + width / 2.0).then_some(tab))
    }

    /// Fades tab `id` while it is dragged, or no tab.
    pub(in crate::window) fn show_dragged_tab(&self, id: Option<TabId>) {
        for (tab, widget) in self.imp().shown.borrow().iter() {
            toggle_class(widget, DRAGGED_CLASS, Some(tab.id) == id);
        }
    }

    /// Highlights tab `id` as the folder a file drop would go into, or no
    /// tab.
    pub(in crate::window) fn highlight_drop_tab(&self, id: Option<TabId>) {
        for (tab, widget) in self.imp().shown.borrow().iter() {
            toggle_class(widget, FILE_DROP_CLASS, Some(tab.id) == id);
        }
    }

    /// What follows the pointer once tab `id` leaves the strip: a picture
    /// of `window`, which shows the tab.
    pub(in crate::window) fn drag_icon(&self, id: TabId, window: &gtk::Widget) -> Option<gtk::Widget> {
        self.tab_width(id)?;
        Some(super::carried_window::carried_window(window))
    }

    /// Opens `entries` as a menu under tab `id` (`moveTabMenu` opens at
    /// the tab's bottom left).
    pub(in crate::window) fn show_menu_under_tab(&self, id: TabId, entries: Vec<MenuEntry>) {
        let bounds = {
            let shown = self.imp().shown.borrow();
            let widget = shown
                .iter()
                .find(|(tab, _)| tab.id == id)
                .map(|(_, widget)| widget.clone());
            widget.and_then(|widget| widget.compute_bounds(self))
        };
        let corner = bounds.map_or_else(
            || graphene::Point::new(0.0, 0.0),
            |bounds| graphene::Point::new(bounds.x(), bounds.y() + bounds.height()),
        );
        self.show_menu(entries, corner);
    }
}
