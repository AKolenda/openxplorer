// SPDX-License-Identifier: AGPL-3.0-only
//! Ctrl+wheel over the folder pane changes the layout, one step per wheel
//! notch: Details, then Small, Medium, Large and Extra large icons, as
//! Windows Explorer does and as Dolphin zooms its icons (VIEW-011).
//! Smooth-scrolling touchpads send fractions of a notch, or pixels on
//! Wayland, which add up until they make a whole one.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};

use super::folder_pane::FolderPane;
use super::preferences::Preference;
use super::{BrowserWindow, WindowAction};

/// Touchpad movement, in pixels, that makes one step: about a wheel
/// notch's worth of scrolling.
const PIXELS_PER_NOTCH: f64 = 50.0;

impl BrowserWindow {
    /// Lets Ctrl+wheel over a folder pane step through the layouts.
    pub(super) fn install_view_zoom(&self) {
        for pane in self.folder_panes() {
            self.zoom_with_wheel(pane);
        }
    }

    /// Lets Ctrl+wheel over `pane` step through its layouts; a split tab's
    /// other pane becomes active first.
    fn zoom_with_wheel(&self, pane: &FolderPane) {
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
                let (steps, rest) = zoom_steps(pending.get(), delta_y, wheel.unit());
                pending.set(rest);
                if let Some(side) = wheel.widget().and_then(|pane| window.side_holding(&pane)) {
                    window.activate_pane(side);
                }
                window.zoom_view(steps);
                glib::Propagation::Stop
            }
        ));
        pane.add_controller(wheel);
    }

    /// Shows the layout `steps` steps bigger (smaller when negative) and
    /// saves it. Unlike choosing a view in the menu, zooming keeps the
    /// selected item in sight instead of going back to the top.
    pub(super) fn zoom_view(&self, steps: i32) {
        let pane = self.folder_pane();
        let view = pane.view();
        let zoomed = view.zoomed(steps);
        if zoomed == view {
            return;
        }
        let kept = pane.model().first_selected();
        self.set_action_state(WindowAction::View, &zoomed.as_str().to_variant());
        self.show_view(zoomed);
        self.save_preference(Preference::View(zoomed));
        if let Some(position) = kept {
            glib::idle_add_local_once(glib::clone!(
                #[weak]
                pane,
                move || pane.reveal(position)
            ));
        }
    }
}

/// The whole steps that `delta_y` of a scroll in `unit` adds to the
/// `pending` part of a step, and the part left over. Scrolling up (a
/// negative delta) zooms in.
pub(super) fn zoom_steps(pending: f64, delta_y: f64, unit: gdk::ScrollUnit) -> (i32, f64) {
    let notches = match unit {
        gdk::ScrollUnit::Surface => delta_y / PIXELS_PER_NOTCH,
        _ => delta_y,
    };
    let total = pending - notches;
    let steps = total.trunc();
    (steps_of(steps), total - steps)
}

/// Whole wheel steps as a step count; a wheel never turns billions of
/// notches in one event.
#[expect(clippy::cast_possible_truncation, reason = "`steps` is a small whole number")]
fn steps_of(steps: f64) -> i32 {
    steps as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wheel notches step one view each; touchpad pixels add up to a
    /// step instead of jumping several views in one swipe.
    ///
    /// parity: VIEW-011
    #[test]
    fn wheel_and_touchpad_deltas_add_up_to_whole_steps() {
        assert_eq!(zoom_steps(0.0, -1.0, gdk::ScrollUnit::Wheel), (1, 0.0));
        assert_eq!(zoom_steps(0.0, 2.0, gdk::ScrollUnit::Wheel), (-2, 0.0));
        let (steps, rest) = zoom_steps(0.0, -0.4, gdk::ScrollUnit::Wheel);
        assert_eq!(steps, 0);
        assert_eq!(zoom_steps(rest, -0.6, gdk::ScrollUnit::Wheel).0, 1);

        let (steps, rest) = zoom_steps(0.0, -30.0, gdk::ScrollUnit::Surface);
        assert_eq!(steps, 0, "one touchpad event of 30 px is less than a step");
        let (steps, rest) = zoom_steps(rest, -30.0, gdk::ScrollUnit::Surface);
        assert_eq!(steps, 1);
        assert!((rest - 0.2).abs() < 1e-9);
    }
}
