// SPDX-License-Identifier: AGPL-3.0-only
//! Ctrl+wheel over the folder pane changes the layout, one step per wheel
//! notch: Details, then Small, Medium, Large and Extra large icons, as
//! Windows Explorer does and as Dolphin zooms its icons (VIEW-011).
//! Smooth-scrolling touchpads send fractions of a notch, which add up
//! until they make a whole one.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};

use super::BrowserWindow;

/// Wheel movement that makes one step: a whole notch.
const NOTCH: f64 = 1.0;

impl BrowserWindow {
    /// Lets Ctrl+wheel over the folder pane step through the layouts.
    pub(super) fn install_view_zoom(&self) {
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
        let pending = Rc::new(Cell::new(0.0_f64));
        wheel.connect_scroll(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |wheel, _, delta_y| {
                if !wheel
                    .current_event_state()
                    .contains(gdk::ModifierType::CONTROL_MASK)
                {
                    pending.set(0.0);
                    return glib::Propagation::Proceed;
                }
                let total = pending.get() - delta_y;
                let steps = (total / NOTCH).trunc();
                pending.set(total - steps * NOTCH);
                window.zoom_view(steps_of(steps));
                glib::Propagation::Stop
            }
        ));
        self.folder_pane().add_controller(wheel);
    }

    /// Shows the layout `steps` steps bigger (smaller when negative) and
    /// saves it, as choosing it in the View menu does.
    pub(super) fn zoom_view(&self, steps: i32) {
        let view = self.folder_pane().view();
        let zoomed = view.zoomed(steps);
        if zoomed != view {
            super::WindowAction::View.activate_from(self, Some(&zoomed.as_str().to_variant()));
        }
    }
}

/// Whole wheel steps as a step count; a wheel never turns billions of
/// notches in one event.
#[expect(clippy::cast_possible_truncation, reason = "`steps` is a small whole number")]
fn steps_of(steps: f64) -> i32 {
    steps as i32
}
