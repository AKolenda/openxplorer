// SPDX-License-Identifier: AGPL-3.0-only
//! What the strip shows while tabs and files are dragged: which tab a
//! press would drag, where a dropped tab would go, the dragged tab itself
//! and the tab a file drop would go into (TAB-018, TAB-032, TAB-033).
//!
//! Ports `showTabDropHint` of `v2.0.0:desktop/ui/app.js` and the `.tab-drop-active`,
//! `.tab-insert-before` and `.tab-drag-source` rules of
//! `v2.0.0:desktop/ui/style.css`. The strip answers for points in its own
//! coordinates, so its sideways scrolling needs no arithmetic.

use gtk::graphene;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::{tab_icon, TabStrip};
use crate::window::menu_popover::MenuEntry;
use crate::window::session::TabId;

/// The strip's class while a tab drag is over it (`.tabs.tab-drop-active`).
const STRIP_DROP_CLASS: &str = "tab-drop-active";

/// The class of the tab a dropped tab would go before
/// (`.tab-insert-before`).
const INSERT_BEFORE_CLASS: &str = "tab-insert-before";

/// The class of the last tab when a dropped tab would go after it.
const INSERT_AFTER_CLASS: &str = "tab-insert-after";

/// The class of the tab being dragged (`.tab-drag-source`).
const DRAGGED_CLASS: &str = "tab-drag-source";

/// The class of the tab whose folder a file drop would go into (TAB-018).
const FILE_DROP_CLASS: &str = "file-drop-active";

/// The class of a tab's close button, which never starts a drag.
const CLOSE_BUTTON_CLASS: &str = "tab-close";

/// Where a dragged tab would go in the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::window) enum TabInsertion {
    /// No tab drag is over the strip.
    Hidden,
    /// Before this tab.
    Before(TabId),
    /// After the last tab.
    AtEnd,
}

impl TabInsertion {
    /// Before `tab`, or at the end without one.
    pub(in crate::window) fn before(tab: Option<TabId>) -> Self {
        tab.map_or(TabInsertion::AtEnd, TabInsertion::Before)
    }
}

/// Adds `class` to `widget` when `shown`, and removes it otherwise.
fn toggle_class(widget: &impl IsA<gtk::Widget>, class: &str, shown: bool) {
    if shown {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

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
    pub(in crate::window) fn tab_after(&self, x: f64) -> Option<TabId> {
        let shown = self.imp().shown.borrow();
        shown.iter().find_map(|(tab, widget)| {
            let bounds = widget.compute_bounds(self)?;
            let middle = f64::from(bounds.x() + bounds.width() / 2.0);
            (x < middle).then_some(tab.id)
        })
    }

    /// Shows where a dragged tab would go: the strip highlighted and a
    /// line at the insertion point, or nothing.
    pub(in crate::window) fn show_tab_insertion(&self, insertion: TabInsertion) {
        toggle_class(self, STRIP_DROP_CLASS, insertion != TabInsertion::Hidden);
        let shown = self.imp().shown.borrow();
        let last = shown.last().map(|(tab, _)| tab.id);
        for (tab, widget) in shown.iter() {
            let is_before = insertion == TabInsertion::Before(tab.id);
            let is_after = insertion == TabInsertion::AtEnd && Some(tab.id) == last;
            toggle_class(widget, INSERT_BEFORE_CLASS, is_before);
            toggle_class(widget, INSERT_AFTER_CLASS, is_after);
        }
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

    /// The likeness of tab `id` that follows the pointer while it is
    /// dragged: its icon and title.
    pub(in crate::window) fn drag_icon(&self, id: TabId) -> Option<gtk::Widget> {
        let shown = self.imp().shown.borrow();
        let (tab, _) = shown.iter().find(|(tab, _)| tab.id == id)?;
        let likeness = gtk::Box::builder()
            .spacing(super::ICON_TO_TITLE)
            .css_classes(["tab-drag-icon"])
            .build();
        likeness.append(&tab_icon(tab.icon));
        likeness.append(&gtk::Label::new(Some(&tab.title)));
        Some(likeness.upcast())
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

    /// Each tab and its style classes, left to right, for tests.
    #[cfg(test)]
    pub(in crate::window) fn tab_classes(&self) -> Vec<(TabId, Vec<String>)> {
        let shown = self.imp().shown.borrow();
        shown
            .iter()
            .map(|(tab, widget)| {
                let classes = widget.css_classes().iter().map(ToString::to_string).collect();
                (tab.id, classes)
            })
            .collect()
    }
}
