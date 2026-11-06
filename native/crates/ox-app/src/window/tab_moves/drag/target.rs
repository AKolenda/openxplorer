// SPDX-License-Identifier: AGPL-3.0-only
//! The receiving window's side of a tab drag: where a dropped tab goes,
//! what the window shows while the tab is over it, and taking the drop.
//!
//! Nothing is written over the window: over its own strip the tab there
//! follows the pointer again, over another window's strip a stand-in for
//! it does, as Chrome takes the tab into that strip, and anywhere else the
//! carried tab does.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene};

use super::{dragged_tab, CarriedLook, DraggedTab, TabDropSpot};
use crate::window::session::TabId;
use crate::window::tab_strip::Landing;
use crate::window::BrowserWindow;

impl BrowserWindow {
    pub(super) fn attach_tab_drop_target(&self) {
        let target = gtk::DropTarget::new(DraggedTab::static_type(), gdk::DragAction::MOVE);
        target.connect_motion(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            gdk::DragAction::empty(),
            move |target, x, y| window.hover_tab_drop(target, x, y)
        ));
        target.connect_leave(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.leave_tab_drop()
        ));
        target.connect_drop(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            false,
            move |_, value, x, y| window.take_tab_drop(value, x, y)
        ));
        self.add_controller(target);
    }

    /// A tab drag moves over this window: shows where a drop would go and
    /// tells the source whether this window takes the tab.
    fn hover_tab_drop(&self, target: &gtk::DropTarget, x: f64, y: f64) -> gdk::DragAction {
        let Some(dragged) = target.current_drop().as_ref().and_then(dragged_tab) else {
            return gdk::DragAction::empty();
        };
        let Some(source) = dragged.source.upgrade() else {
            return gdk::DragAction::empty();
        };
        let spot = self.tab_drop_spot(&source, x, y);
        source.note_tab_hover(Some(spot.is_some()));
        self.show_tab_drop_spot(&source, dragged.tab, spot, x);
        if spot.is_some() {
            gdk::DragAction::MOVE
        } else {
            gdk::DragAction::empty()
        }
    }

    /// The tab drag left this window: its strip closes any gap, and the
    /// source learns the pointer is outside this window.
    fn leave_tab_drop(&self) {
        let windows = self.application().map(|app| app.windows()).unwrap_or_default();
        let dragging = windows
            .into_iter()
            .filter_map(|window| window.downcast::<BrowserWindow>().ok())
            .find(|window| window.imp().outgoing_tab.borrow().is_some());
        if let Some(source) = dragging {
            source.note_tab_hover(None);
            self.show_tab_away(&source);
        } else {
            self.tab_strip().dismiss_guest();
        }
    }

    /// Takes the tab `value` carries at (`x`, `y`); true when it moved or
    /// will tear out.
    fn take_tab_drop(&self, value: &glib::Value, x: f64, y: f64) -> bool {
        let Ok(dragged) = value.get::<DraggedTab>() else {
            return false;
        };
        let Some(source) = dragged.source.upgrade() else {
            return false;
        };
        let Some(spot) = self.tab_drop_spot(&source, x, y) else {
            return false;
        };
        let is_in_own_strip = &source == self && matches!(spot, TabDropSpot::Strip { .. });
        if is_in_own_strip && self.tab_strip().is_following() {
            return self.settle_live_drop(dragged.tab);
        }
        // A tab from another window lands where its stand-in was.
        let spot = match (spot, self.tab_strip().land_guest()) {
            (TabDropSpot::Strip { .. }, Some(Landing { before })) => TabDropSpot::Strip { before },
            (spot, _) => spot,
        };
        source.settle_tab_drag(dragged.tab, self, spot)
    }

    /// Where a tab from `source` dropped at (`x`, `y`) of this window
    /// goes: into the strip where the pointer is over the tab row, else
    /// into a new window, as in Chrome; `None` while this window is busy
    /// (TAB-035, TAB-037).
    pub(super) fn tab_drop_spot(&self, _source: &BrowserWindow, x: f64, y: f64) -> Option<TabDropSpot> {
        if self.is_busy_for_tab_moves() {
            return None;
        }
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let point = graphene::Point::new(x as f32, y as f32);
        if !self.is_in_tab_row(point) {
            return Some(TabDropSpot::TearOut);
        }
        let in_strip = self.compute_point(self.tab_strip(), &point)?;
        let before = self.tab_strip().tab_after(f64::from(in_strip.x()));
        Some(TabDropSpot::Strip { before })
    }

    /// True for a point of the title bar from the first tab to the end of
    /// the empty drag area, where the caption buttons start: the tabs, "+"
    /// and the drag area.
    pub(super) fn is_in_tab_row(&self, point: graphene::Point) -> bool {
        let strip = self.tab_strip().compute_bounds(self);
        let drag_area = self.imp().title_drag.compute_bounds(self);
        let (Some(strip), Some(drag_area), Some(bottom)) = (strip, drag_area, self.title_bar_bottom()) else {
            return false;
        };
        let is_across = point.x() >= strip.x() && point.x() < drag_area.x() + drag_area.width();
        let is_down = point.y() >= 0.0 && point.y() < bottom;
        is_across && is_down
    }

    /// Where the title bar ends, in the window's coordinates.
    pub(super) fn title_bar_bottom(&self) -> Option<f32> {
        let bar = self.titlebar()?.compute_bounds(self)?;
        Some(bar.y() + bar.height())
    }

    /// Shows tab `tab` of `source` at `spot`, `x` of this window: in its
    /// own strip it follows the pointer there again, and another window's
    /// strip takes a stand-in for it that follows the pointer, while the
    /// carried tab hides; anywhere else the strips are as they were.
    pub(super) fn show_tab_drop_spot(
        &self,
        source: &BrowserWindow,
        tab: TabId,
        spot: Option<TabDropSpot>,
        x: f64,
    ) {
        let strip = self.tab_strip();
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let in_strip = self
            .compute_point(strip, &graphene::Point::new(x as f32, 0.0))
            .map_or(0.0, |point| f64::from(point.x()));
        let is_own = source == self;
        match spot {
            Some(TabDropSpot::Strip { .. }) if is_own => {
                source.note_over_own_strip(true);
                strip.show_dragged_tab(None);
                strip.follow_pointer(tab, in_strip);
                source.bring_carried_tab_forward(tab);
                source.show_carried(CarriedLook::Hidden);
            }
            Some(TabDropSpot::Strip { .. }) => {
                let share = source.tab_strip().grab_share(tab);
                if let Some(view) = source.tab_strip().tab_view(tab) {
                    strip.host_guest(&view, in_strip, share);
                }
                source.show_carried(CarriedLook::Hidden);
            }
            Some(TabDropSpot::TearOut) | None => self.show_tab_away(source),
        }
    }

    /// Shows the tab `source` drags away from this window's strip: the
    /// strip as it was, and the carried tab under the pointer.
    fn show_tab_away(&self, source: &BrowserWindow) {
        let strip = self.tab_strip();
        if source == self {
            source.note_over_own_strip(false);
            // Out of its strip again: the tab leaves it, which closes up.
            if let Some(tab) = strip.live_tab().filter(|_| strip.is_following()) {
                source.take_tab_off_strip(tab);
            }
        }
        strip.dismiss_guest();
        source.show_carried(CarriedLook::Shown);
    }
}
