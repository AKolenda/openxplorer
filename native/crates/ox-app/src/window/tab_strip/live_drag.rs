// SPDX-License-Identifier: AGPL-3.0-only
//! Dragging a tab live along the strip, as Windows Explorer and Chrome do
//! (TAB-032, TAB-033): the tab itself follows the pointer and the others
//! slide aside, with no drag icon or drop mark. A strip that a tab from
//! another window hovers takes a stand-in for it, which follows the pointer
//! the same way, where the tab lands on release.
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

/// The class of the tab that follows the pointer, which lifts it.
const DRAGGING_CLASS: &str = "tab-dragging";

/// Where a released tab went: the tab, and the tab it now comes before, or
/// `None` at the end.
pub(in crate::window) type LiveMove = (TabId, Option<TabId>);

/// Where a tab of another window dropped on the strip lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::window) struct Landing {
    /// The tab it comes before, or `None` at the end.
    pub(in crate::window) before: Option<TabId>,
}

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

    /// Remembers where tab `id` was grabbed: (`x`, `y`) of the strip, from
    /// the tab's top left corner as it is drawn. A tab let go a moment ago
    /// may still slide into a place the session's order does not have yet,
    /// so where it is drawn is where the press met it.
    pub(in crate::window) fn grab_tab(&self, id: TabId, (x, y): (f64, f64)) {
        let Some(widget) = self.widget_of(id) else {
            return;
        };
        let row = self.imp().tab_list.get();
        let (Some(in_row), Some(in_strip)) = (widget.compute_bounds(&row), widget.compute_bounds(self))
        else {
            return;
        };
        let along = self.in_row(x) - f64::from(in_row.x());
        self.imp()
            .grab
            .set(Some((id, (along, y - f64::from(in_strip.y())))));
    }

    /// Where on itself tab `id` was grabbed, from its top left corner, if
    /// it was the last one.
    pub(in crate::window) fn grab_offset(&self, id: TabId) -> Option<(f64, f64)> {
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
        let left = self.in_row(x) - offset.0;
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
        widget.add_css_class(DRAGGING_CLASS);
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
    /// `done` then gets where it went, if the order changed: before the
    /// first tab after it still open, as one may close while it slides.
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
                let after: Vec<TabId> = order
                    .iter()
                    .skip(at + 1)
                    .filter_map(|next| self.id_of(next))
                    .collect();
                Some((id, after))
            })
            .flatten();
        let strip = self.downgrade();
        layout.settle(order, move || {
            let moved = strip.upgrade().and_then(|strip| {
                // The tabs as the session has them now.
                strip.stop_live();
                let (id, after) = moved?;
                Some((
                    id,
                    after.into_iter().find(|next| strip.widget_of(*next).is_some()),
                ))
            });
            done(moved);
        });
    }

    /// The dragged tab `id` leaves the strip: it vanishes there and the
    /// others close up.
    pub(in crate::window) fn detach_tab(&self, id: TabId) {
        self.layout().detach();
        self.show_dragged_tab(Some(id));
    }

    /// Ends a drag along the strip at once: the tabs are drawn as the
    /// session has them.
    pub(in crate::window) fn end_drag_now(&self) {
        self.layout().finish_now();
        self.stop_live();
    }

    /// A tab `view` of another window, grabbed `grab_x` along itself, is
    /// over this strip at `x`: a stand-in for it follows the pointer here as
    /// this strip's own tabs do, and the others make room, as Chrome takes a
    /// tab into the strip it is dragged over.
    pub(in crate::window) fn host_guest(&self, view: &super::TabView, x: f64, grab_x: f64) {
        let layout = self.layout();
        let left = self.in_row(x) - grab_x;
        let guest = self.imp().guest.borrow().clone();
        if let Some(guest) = guest {
            if layout.dragged().as_ref() == Some(&guest) {
                layout.follow(left);
                return;
            }
        }
        // A tab released or let go a moment ago takes its place first.
        layout.finish_now();
        self.stop_live();
        let guest = super::carried_tab::carried_tab(view, None, self.strip_title_font().as_ref(), 0);
        let list = self.imp().tab_list.get();
        // Drawn last, so it slides over the others.
        list.append(&guest);
        let mut order = self.widgets();
        let at = self
            .tab_after(x)
            .and_then(|before| self.tab_slots().iter().position(|(tab, _, _)| *tab == before))
            .unwrap_or(order.len());
        order.insert(at, guest.clone());
        for tab in &order {
            tab.set_has_tooltip(false);
        }
        // The tab dragged in is the one shown in front, as in Chrome.
        self.show_own_front(false);
        self.imp().guest.replace(Some(guest.clone()));
        layout.begin_drag(order, &guest, left);
    }

    /// Shows this strip's tab in front as in front, or with `shown` false
    /// as any other while a stand-in is.
    pub(super) fn show_own_front(&self, shown: bool) {
        for (view, widget) in self.imp().shown.borrow().iter() {
            crate::window::widget_tree::toggle_class(widget, "active", view.active && shown);
        }
    }

    /// The tab of another window left this strip: its stand-in goes and the
    /// tabs close up.
    pub(in crate::window) fn dismiss_guest(&self) {
        let Some(guest) = self.imp().guest.take() else {
            return;
        };
        self.imp().tab_list.remove(&guest);
        self.show_own_front(true);
        let layout = self.layout();
        if layout.dragged().as_ref() != Some(&guest) {
            return;
        }
        let strip = self.downgrade();
        layout.settle(self.widgets(), move || {
            if let Some(strip) = strip.upgrade() {
                strip.stop_live();
            }
        });
    }

    /// The tab of another window is dropped here: its stand-in goes, and
    /// where it was is where the tab lands, before the first tab after it
    /// still open, as one may close during the drag; `None` without a
    /// stand-in.
    pub(in crate::window) fn land_guest(&self) -> Option<Landing> {
        let guest = self.imp().guest.take()?;
        let layout = self.layout();
        let order = layout.live_order().unwrap_or_default();
        let after = order.iter().position(|tab| *tab == guest).map(|at| {
            order
                .iter()
                .skip(at + 1)
                .filter_map(|next| self.id_of(next))
                .collect::<Vec<_>>()
        });
        self.imp().tab_list.remove(&guest);
        self.show_own_front(true);
        layout.end();
        // The tabs as the session has them now.
        self.stop_live();
        after.map(|after| Landing {
            before: after.into_iter().find(|next| self.widget_of(*next).is_some()),
        })
    }

    /// The border box of tab `id`, in `target`'s coordinates.
    pub(in crate::window) fn tab_bounds_in(
        &self,
        id: TabId,
        target: &impl IsA<gtk::Widget>,
    ) -> Option<gtk::graphene::Rect> {
        self.widget_of(id)?.compute_bounds(target)
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
            widget.remove_css_class(DRAGGING_CLASS);
            list.reorder_child_after(&widget, previous.as_ref());
            previous = Some(widget);
        }
        if let Some(tabs) = self.imp().waiting_tabs.take() {
            self.set_tabs(&tabs);
        }
    }
}
