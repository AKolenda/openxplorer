// SPDX-License-Identifier: AGPL-3.0-only
//! The receiving window's side of a tab drag: where a dropped tab goes,
//! the marks that show it, and taking the drop.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene};

use super::{dragged_tab, DraggedTab, TabDropSpot, TEAR_OUT_GAP, TEAR_OUT_HINT};
use crate::window::tab_strip::TabInsertion;
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
            move |_| window.show_tab_drop_spot(None)
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
    /// tells the source whether it is refused here.
    fn hover_tab_drop(&self, target: &gtk::DropTarget, x: f64, y: f64) -> gdk::DragAction {
        let dragged = target.current_drop().as_ref().and_then(dragged_tab);
        let source = dragged.and_then(|dragged| dragged.source.upgrade());
        let spot = source
            .as_ref()
            .and_then(|source| self.tab_drop_spot(source, x, y));
        if let Some(source) = &source {
            source.note_tab_refusal(spot.is_none());
        }
        self.show_tab_drop_spot(spot);
        if spot.is_some() {
            gdk::DragAction::MOVE
        } else {
            gdk::DragAction::empty()
        }
    }

    /// Takes the tab `value` carries at (`x`, `y`); true when it moved or
    /// will tear out.
    fn take_tab_drop(&self, value: &glib::Value, x: f64, y: f64) -> bool {
        self.show_tab_drop_spot(None);
        let Ok(dragged) = value.get::<DraggedTab>() else {
            return false;
        };
        let Some(source) = dragged.source.upgrade() else {
            return false;
        };
        let Some(spot) = self.tab_drop_spot(&source, x, y) else {
            return false;
        };
        source.settle_tab_drag(dragged.tab, self, spot)
    }

    /// Where a tab from `source` dropped at (`x`, `y`) of this window
    /// goes; `None` where it is refused, and anywhere while this window
    /// is busy (TAB-035, TAB-037).
    pub(super) fn tab_drop_spot(&self, source: &BrowserWindow, x: f64, y: f64) -> Option<TabDropSpot> {
        if self.is_busy_for_tab_moves() {
            return None;
        }
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let point = graphene::Point::new(x as f32, y as f32);
        if self.is_in_tab_row(point) {
            let in_strip = self.compute_point(self.tab_strip(), &point)?;
            let before = self.tab_strip().tab_after(f64::from(in_strip.x()));
            return Some(TabDropSpot::Strip { before });
        }
        let is_tear_out_area = source == self && self.is_below_title_bar(y, TEAR_OUT_GAP);
        is_tear_out_area.then_some(TabDropSpot::TearOut)
    }

    /// True for a point of the title bar from the first tab to the
    /// open-windows button: the tabs, "+" and the empty drag area.
    fn is_in_tab_row(&self, point: graphene::Point) -> bool {
        let strip = self.tab_strip().compute_bounds(self);
        let windows_button = self.imp().open_windows_button.compute_bounds(self);
        let (Some(strip), Some(windows_button), Some(bottom)) =
            (strip, windows_button, self.title_bar_bottom())
        else {
            return false;
        };
        let is_across = point.x() >= strip.x() && point.x() < windows_button.x();
        let is_down = point.y() >= 0.0 && point.y() < bottom;
        is_across && is_down
    }

    /// True when `y` is at least `gap` below the title bar.
    fn is_below_title_bar(&self, y: f64, gap: f64) -> bool {
        self.title_bar_bottom()
            .is_some_and(|bottom| y >= f64::from(bottom) + gap)
    }

    /// Where the title bar ends, in the window's coordinates.
    pub(super) fn title_bar_bottom(&self) -> Option<f32> {
        let bar = self.titlebar()?.compute_bounds(self)?;
        Some(bar.y() + bar.height())
    }

    /// Shows where a dropped tab would go: the strip's insertion mark, or
    /// the tear-out note; nothing for `None`.
    pub(super) fn show_tab_drop_spot(&self, spot: Option<TabDropSpot>) {
        let insertion = match spot {
            Some(TabDropSpot::Strip { before }) => Some(before),
            Some(TabDropSpot::TearOut) | None => None,
        };
        self.tab_strip()
            .show_tab_insertion(insertion.map_or(TabInsertion::Hidden, TabInsertion::before));
        let hint = (spot == Some(TabDropSpot::TearOut)).then_some(TEAR_OUT_HINT);
        self.folder_pane().show_drag_hint(hint);
    }
}
