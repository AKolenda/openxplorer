// SPDX-License-Identifier: AGPL-3.0-only
//! Dragging a tab live along the strip, as Windows Explorer and Chrome do
//! (TAB-032, TAB-033): the tab itself follows the pointer and the others
//! slide aside, with no drag icon or drop mark. A strip that a tab from
//! another window hovers opens a gap where it would land.
//!
//! The strip only shows the drag; [`TabLayout`]'s live layout places the
//! tabs, and the window decides what a release does. While a drag lasts
//! the strip keeps its tab widgets: new tabs it is given wait until the
//! drag is over.

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::TabStrip;
use crate::window::session::TabId;
use crate::window::tab_layout::{TabLayout, TAB_GAP};

/// Where a released tab went: the tab, and the tab it now comes before, or
/// `None` at the end.
pub(in crate::window) type LiveMove = (TabId, Option<TabId>);

impl TabStrip {
    fn layout(&self) -> TabLayout {
        self.imp().layout.get()
    }

    /// `x` of the strip in the row of tabs, which scrolls inside it.
    pub(super) fn in_row(&self, x: f64) -> f64 {
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let point = gtk::graphene::Point::new(x as f32, 0.0);
        self.compute_point(&*self.imp().tab_list, &point)
            .map_or(x, |point| f64::from(point.x()))
    }

    /// The widget of tab `id`.
    fn widget_of(&self, id: TabId) -> Option<gtk::Widget> {
        let shown = self.imp().shown.borrow();
        shown
            .iter()
            .find(|(tab, _)| tab.id == id)
            .map(|(_, widget)| widget.clone().upcast())
    }

    /// The widget of tab `id`, for tests.
    #[cfg(test)]
    pub(in crate::window) fn tab_widget_of(&self, id: TabId) -> Option<gtk::Widget> {
        self.widget_of(id)
    }

    /// Draws tab `id` as the active tab and the others as not, while a
    /// drag keeps the tab widgets.
    pub(in crate::window) fn mark_active(&self, id: TabId) {
        for (tab, widget) in self.imp().shown.borrow().iter() {
            crate::window::widget_tree::toggle_class(widget, "active", tab.id == id);
        }
    }

    /// The tab whose widget is `widget`.
    fn id_of(&self, widget: &gtk::Widget) -> Option<TabId> {
        let shown = self.imp().shown.borrow();
        shown
            .iter()
            .find(|(_, tab)| tab.upcast_ref::<gtk::Widget>() == widget)
            .map(|(tab, _)| tab.id)
    }

    /// The tab widgets in the session's order.
    fn widgets(&self) -> Vec<gtk::Widget> {
        let shown = self.imp().shown.borrow();
        shown.iter().map(|(_, widget)| widget.clone().upcast()).collect()
    }

    /// Each tab, its left edge and width in the row, in the session's
    /// order, wherever a drag draws them.
    pub(in crate::window) fn tab_slots(&self) -> Vec<(TabId, f64, f64)> {
        let shown = self.imp().shown.borrow();
        let layout = self.layout();
        let mut left = 0.0;
        shown
            .iter()
            .map(|(tab, widget)| {
                let width = layout.width_of(widget.upcast_ref());
                let slot = (tab.id, left, width);
                left += width + f64::from(TAB_GAP);
                slot
            })
            .collect()
    }

    /// Remembers where tab `id` was grabbed: `x` of the strip, from the
    /// tab's left edge.
    pub(in crate::window) fn grab_tab(&self, id: TabId, x: f64) {
        let row_x = self.in_row(x);
        let left = self
            .tab_slots()
            .into_iter()
            .find_map(|(tab, left, _)| (tab == id).then_some(left));
        if let Some(left) = left {
            self.imp().grab.set(Some((id, row_x - left)));
        }
    }

    /// Where along itself tab `id` was grabbed, if it was the last one.
    pub(in crate::window) fn grab_offset(&self, id: TabId) -> Option<f64> {
        let (grabbed, offset) = self.imp().grab.get()?;
        (grabbed == id).then_some(offset)
    }

    /// Tab `id`, grabbed by [`Self::grab_tab`], follows the pointer at `x`
    /// of the strip; the other tabs make room.
    pub(in crate::window) fn follow_pointer(&self, id: TabId, x: f64) {
        let Some((grabbed, offset)) = self.imp().grab.get() else {
            return;
        };
        let Some(widget) = self.widget_of(id) else {
            return;
        };
        if grabbed != id {
            return;
        }
        let left = self.in_row(x) - offset;
        let layout = self.layout();
        if layout.dragged().as_ref() == Some(&widget) {
            layout.follow(left);
            return;
        }
        // A tab released a moment ago takes its place first.
        layout.finish_now();
        let order = self.widgets();
        // No tooltip pops up over a tab that moves.
        for tab in &order {
            tab.set_has_tooltip(false);
        }
        // Drawn last, so it slides over the others.
        let list = self.imp().tab_list.get();
        list.reorder_child_after(&widget, list.last_child().as_ref());
        layout.begin_drag(order, &widget, left);
    }

    /// The tab following the pointer, or sliding into place after it was
    /// released.
    pub(in crate::window) fn live_tab(&self) -> Option<TabId> {
        self.layout().dragged().and_then(|widget| self.id_of(&widget))
    }

    /// Whether a tab of this strip follows the pointer.
    pub(in crate::window) fn is_following(&self) -> bool {
        self.layout().following().is_some()
    }

    /// The dragged tab is let go: it slides into the place shown, or with
    /// `keep_order` false back to where it was, and the others with it.
    /// `done` then gets where it went, if the order changed.
    pub(in crate::window) fn let_go(&self, keep_order: bool, done: impl FnOnce(Option<LiveMove>) + 'static) {
        let layout = self.layout();
        let Some(dragged) = layout.dragged() else {
            done(None);
            return;
        };
        let original = self.widgets();
        let order = if keep_order {
            layout.live_order().unwrap_or_else(|| original.clone())
        } else {
            original.clone()
        };
        let moved = (order != original)
            .then(|| {
                let id = self.id_of(&dragged)?;
                let at = order.iter().position(|tab| *tab == dragged)?;
                let before = order.get(at + 1).and_then(|next| self.id_of(next));
                Some((id, before))
            })
            .flatten();
        let strip = self.downgrade();
        layout.settle(order, move || {
            if let Some(strip) = strip.upgrade() {
                strip.stop_live();
            }
            done(moved);
        });
    }

    /// Opens a gap `width` wide where a tab dropped at `x` of the strip
    /// would go, for a tab from another window.
    pub(in crate::window) fn open_gap(&self, x: f64, width: f64) {
        let layout = self.layout();
        if layout.dragged().is_some() {
            return;
        }
        layout.begin_gap(self.widgets());
        let before = self.tab_after(x);
        let shown = self.tab_slots();
        let index = before
            .and_then(|before| shown.iter().position(|(tab, _, _)| *tab == before))
            .unwrap_or(shown.len());
        layout.open_gap(index, width);
    }

    /// Closes the gap of [`Self::open_gap`]: the tabs slide back.
    pub(in crate::window) fn close_gap(&self) {
        let layout = self.layout();
        if !layout.is_gap() {
            return;
        }
        let strip = self.downgrade();
        layout.settle(self.widgets(), move || {
            if let Some(strip) = strip.upgrade() {
                strip.stop_live();
            }
        });
    }

    /// Ends a gap at once, as a tab lands in it.
    pub(in crate::window) fn end_gap(&self) {
        if self.layout().is_gap() {
            self.stop_live();
        }
    }

    /// The width of tab `id`.
    pub(in crate::window) fn tab_width(&self, id: TabId) -> Option<f64> {
        self.widget_of(id).map(|widget| self.layout().width_of(&widget))
    }

    /// Stops the live layout: the tabs go back to the session's order and
    /// tabs given during the drag are shown.
    fn stop_live(&self) {
        self.layout().end();
        let list = self.imp().tab_list.get();
        let mut previous: Option<gtk::Widget> = None;
        for widget in self.widgets() {
            widget.set_has_tooltip(true);
            list.reorder_child_after(&widget, previous.as_ref());
            previous = Some(widget);
        }
        if let Some(tabs) = self.imp().waiting_tabs.take() {
            self.set_tabs(&tabs);
        }
    }
}
