// SPDX-License-Identifier: AGPL-3.0-only
//! Scrolling the file list or the sidebar while a drag hovers near its
//! top or bottom edge (DND-025).
//!
//! As in Dolphin and Windows Explorer, a file drag, from this window or
//! another application, that hovers within [`EDGE`] pixels of the top or
//! bottom of a scrolled zone scrolls it, faster closer to the edge, so
//! folders out of sight can be reached as drop targets. Leaving the edge,
//! the zone, or dropping stops it.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{glib, graphene};

use crate::window::BrowserWindow;

/// How close to an edge, in pixels, the pointer scrolls the zone.
const EDGE: f64 = 40.0;

/// The most a zone scrolls per tick, in pixels, with the pointer on the
/// edge itself.
const MAX_STEP: f64 = 24.0;

/// How often a hovering drag scrolls.
const TICK: Duration = Duration::from_millis(30);

/// The running scroll of one zone: its scrolled window, how far each
/// tick moves it, and the timer.
#[derive(Debug)]
pub(in crate::window) struct DragScroll {
    scroller: gtk::ScrolledWindow,
    step: Rc<Cell<f64>>,
    timer: glib::SourceId,
}

/// How far a zone `height` pixels tall scrolls per tick with the pointer
/// at `y`: up (negative) near the top, down near the bottom, faster
/// closer to the edge; 0 elsewhere.
pub(super) fn scroll_step(y: f64, height: f64) -> f64 {
    if height <= 2.0 * EDGE {
        return 0.0;
    }
    let closeness = |distance: f64| ((EDGE - distance.max(0.0)) / EDGE).clamp(0.0, 1.0);
    if y < EDGE {
        -MAX_STEP * closeness(y)
    } else if y > height - EDGE {
        MAX_STEP * closeness(height - y)
    } else {
        0.0
    }
}

impl BrowserWindow {
    /// A drag hovers at `y` of `widget`: scrolls the scrolled window that
    /// holds it while the pointer is near its top or bottom edge. True
    /// while it scrolls.
    pub(super) fn scroll_drag_near_edge(&self, widget: &gtk::Widget, y: f64) -> bool {
        let scroller = widget
            .downcast_ref::<gtk::ScrolledWindow>()
            .cloned()
            .or_else(|| widget.ancestor(gtk::ScrolledWindow::static_type()).and_downcast());
        let Some(scroller) = scroller else {
            return false;
        };
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let point = graphene::Point::new(0.0, y as f32);
        let Some(in_scroller) = widget.compute_point(&scroller, &point) else {
            return false;
        };
        let step = scroll_step(f64::from(in_scroller.y()), f64::from(scroller.height()));
        if step.abs() < f64::EPSILON {
            self.stop_drag_scroll();
            return false;
        }
        if let Some(running) = self.imp().drag_scroll.borrow().as_ref() {
            if running.scroller == scroller {
                running.step.set(step);
                return true;
            }
        }
        self.stop_drag_scroll();
        let shared_step = Rc::new(Cell::new(step));
        let timer = glib::timeout_add_local(
            TICK,
            glib::clone!(
                #[weak]
                scroller,
                #[strong]
                shared_step,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    let adjustment = scroller.vadjustment();
                    let highest = adjustment.upper() - adjustment.page_size();
                    let value = (adjustment.value() + shared_step.get()).clamp(adjustment.lower(), highest);
                    adjustment.set_value(value);
                    glib::ControlFlow::Continue
                }
            ),
        );
        self.imp().drag_scroll.replace(Some(DragScroll {
            scroller,
            step: shared_step,
            timer,
        }));
        true
    }

    /// Stops the scroll a hovering drag started, if any.
    pub(super) fn stop_drag_scroll(&self) {
        if let Some(running) = self.imp().drag_scroll.take() {
            running.timer.remove();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: DND-025
    #[test]
    fn a_drag_scrolls_near_the_edges_faster_closer_to_them() {
        assert!(scroll_step(0.0, 400.0) < scroll_step(30.0, 400.0));
        assert!(scroll_step(30.0, 400.0) < 0.0, "near the top scrolls up");
        assert!(scroll_step(399.0, 400.0) > scroll_step(370.0, 400.0));
        assert!(scroll_step(370.0, 400.0) > 0.0, "near the bottom scrolls down");
        assert!(
            scroll_step(200.0, 400.0).abs() < f64::EPSILON,
            "the middle does not scroll"
        );
        assert!(
            scroll_step(10.0, 60.0).abs() < f64::EPSILON,
            "a tiny zone does not scroll"
        );
    }
}
