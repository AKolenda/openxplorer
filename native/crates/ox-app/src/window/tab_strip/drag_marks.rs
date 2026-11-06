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

use super::{TabStrip, TabView};
use crate::window::menu_popover::MenuEntry;
use crate::window::session::TabId;
use crate::window::widget_tree::toggle_class;

/// The class of the tab being dragged (`.tab-drag-source`).
pub(super) const DRAGGED_CLASS: &str = "tab-drag-source";

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

    /// Tab `id` as it follows the pointer out of the strip: the tab in
    /// front, as wide as it is here, its title in the font drawn here.
    pub(in crate::window) fn carried_tab(&self, id: TabId) -> Option<gtk::Widget> {
        let width = self.tab_width(id)?;
        let shown = self.imp().shown.borrow();
        let (view, widget) = shown.iter().find(|(tab, _)| tab.id == id)?;
        let font = title_font(widget);
        #[expect(clippy::cast_possible_truncation, reason = "a tab's width")]
        let width = width.round() as i32;
        Some(super::carried_tab::carried_tab(
            view,
            Some(width),
            font.as_ref(),
            super::CARRIED_MARGIN,
        ))
    }

    /// What the strip shows of tab `id`.
    pub(in crate::window) fn tab_view(&self, id: TabId) -> Option<TabView> {
        let shown = self.imp().shown.borrow();
        shown
            .iter()
            .find(|(tab, _)| tab.id == id)
            .map(|(tab, _)| tab.clone())
    }

    /// The font this strip draws tab titles in.
    pub(super) fn strip_title_font(&self) -> Option<gtk::pango::FontDescription> {
        let shown = self.imp().shown.borrow();
        shown.first().and_then(|(_, widget)| title_font(widget))
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

/// The font the title of the tab `widget` is drawn in.
fn title_font(widget: &gtk::Box) -> Option<gtk::pango::FontDescription> {
    crate::window::widget_tree::descendants::<gtk::Label>(widget)
        .into_iter()
        .find(|label| label.has_css_class("tab-title"))
        .and_then(|label| label.pango_context().font_description())
}
