// SPDX-License-Identifier: AGPL-3.0-only
//! Scrolling a strip whose tabs do not all fit while a tab dragged along it
//! dwells near either visible edge (TAB-032), faster closer to the edge, as
//! the file list scrolls under a file drag (DND-025), so places out of
//! sight can be reached; the tab goes on following the pointer as the row
//! moves under it.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::TabStrip;
use crate::window::file_drop::{scroll_step, SCROLL_TICK};

impl TabStrip {
    /// Follows the pointer again whenever the strip scrolls under a tab
    /// that follows it, or a stand-in for a tab of another window, by a
    /// dwell near an edge or the wheel.
    pub(super) fn follow_the_pointer_as_it_scrolls(&self) {
        let strip = self.downgrade();
        self.imp().scroller.hadjustment().connect_value_changed(move |_| {
            let Some(strip) = strip.upgrade() else {
                return;
            };
            if let Some((id, x)) = strip.imp().pointer.get() {
                if strip.follows(id) {
                    strip.follow_pointer(id, x);
                }
            }
            strip.follow_guest_again();
        });
    }

    /// The pointer a tab follows is at `x` of the strip: near either edge
    /// of a strip whose tabs do not all fit, the strip scrolls that way
    /// until the pointer leaves the edge or the tab stops following it.
    pub(super) fn scroll_near_edge(&self, x: f64) {
        let imp = self.imp();
        let scroller = imp.scroller.get();
        let scroll = scroller.hadjustment();
        let overflows = scroll.upper() - scroll.page_size() > scroll.lower() + 0.5;
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let point = gtk::graphene::Point::new(x as f32, 0.0);
        let along = self
            .compute_point(&scroller, &point)
            .map_or(x, |point| f64::from(point.x()));
        let step = if overflows {
            scroll_step(along, f64::from(scroller.width()))
        } else {
            0.0
        };
        if step.abs() < f64::EPSILON {
            self.stop_edge_scroll();
            return;
        }
        imp.edge_step.set(step);
        if imp.edge_scroll.borrow().is_some() {
            return;
        }
        let timer = glib::timeout_add_local(
            SCROLL_TICK,
            glib::clone!(
                #[weak(rename_to = strip)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || strip.scroll_edge_tick()
            ),
        );
        imp.edge_scroll.replace(Some(timer));
    }

    /// One tick of the scroll near an edge; it ends once no tab of the
    /// strip follows the pointer.
    fn scroll_edge_tick(&self) -> glib::ControlFlow {
        let imp = self.imp();
        let step = imp.edge_step.get();
        let following = imp.pointer.get().is_some_and(|(id, _)| self.follows(id));
        if !following || step.abs() < f64::EPSILON {
            // The timer ends with this tick.
            let _ended = imp.edge_scroll.take();
            return glib::ControlFlow::Break;
        }
        let scroll = imp.scroller.hadjustment();
        let end = (scroll.upper() - scroll.page_size()).max(scroll.lower());
        scroll.set_value((scroll.value() + step).clamp(scroll.lower(), end));
        glib::ControlFlow::Continue
    }

    /// Stops the scroll near an edge, if one runs, as the dragged tab
    /// leaves the strip.
    pub(in crate::window) fn stop_edge_scroll(&self) {
        let imp = self.imp();
        imp.edge_step.set(0.0);
        if let Some(timer) = imp.edge_scroll.take() {
            timer.remove();
        }
    }
}
