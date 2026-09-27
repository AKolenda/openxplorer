// SPDX-License-Identifier: AGPL-3.0-only
//! Mouse gestures: middle-click to open in a tab or close a tab, and the
//! mouse's Back and Forward buttons.
//!
//! Ports `bindMiddleClick` and `bindMiddleOpen` in `desktop/ui/app.js` and
//! the mouse-button history keys in `desktop/winspace.py`. A middle-click
//! acts on release, only when the press started on the same widget (GTK
//! cancels the gesture when the pointer drags away), never changes the
//! selection and never launches a file. It opens a folder in a background
//! tab; with Shift held, the new tab comes to the front.

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;

/// The mouse's Back side button (X11 button 8).
const BACK_BUTTON: u32 = 8;
/// The mouse's Forward side button (X11 button 9).
const FORWARD_BUTTON: u32 = 9;

/// The history step a mouse button takes, if it is a side button.
pub(super) fn history_step(button: u32) -> Option<isize> {
    match button {
        BACK_BUTTON => Some(-1),
        FORWARD_BUTTON => Some(1),
        _ => None,
    }
}

/// The action that opens a folder from a middle-click, and whether the tab
/// comes to the front: only with Shift.
pub(super) fn open_action(modifiers: gdk::ModifierType) -> &'static str {
    if modifiers.contains(gdk::ModifierType::SHIFT_MASK) {
        "win.open-tab"
    } else {
        "win.open-tab-background"
    }
}

/// A middle-button gesture that claims its press, so neither primary-paste
/// nor the title bar's middle-click action sees it, and calls `on_click`
/// with the release position and modifiers.
pub(super) fn middle_click(on_click: impl Fn(&gtk::GestureClick, f64, f64) + 'static) -> gtk::GestureClick {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(gdk::BUTTON_MIDDLE);
    gesture.connect_pressed(|gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    gesture.connect_released(move |gesture, _, x, y| on_click(gesture, x, y));
    gesture
}

/// Opens `uri` in a new tab when `widget` is middle-clicked.
pub(super) fn open_folder_on_middle_click(widget: &impl IsA<gtk::Widget>, uri: &str) {
    let uri = uri.to_owned();
    let gesture = middle_click(move |gesture, _, _| {
        let Some(widget) = gesture.widget() else {
            return;
        };
        let action = open_action(gesture.current_event_state());
        // The action exists on every browser window; a widget outside one
        // has nothing to open, so a failure is ignored.
        let _ = widget.activate_action(action, Some(&uri.to_variant()));
    });
    widget.add_controller(gesture);
}

/// Calls `step_history` with -1 or 1 when a mouse side button is pressed
/// anywhere in `window`. Other buttons pass through untouched.
pub(super) fn connect_history_buttons(
    window: &impl IsA<gtk::Widget>,
    step_history: impl Fn(isize) + 'static,
) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(0);
    gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
    gesture.connect_pressed(move |gesture, _, _, _| {
        let Some(step) = history_step(gesture.current_button()) else {
            gesture.set_state(gtk::EventSequenceState::Denied);
            return;
        };
        gesture.set_state(gtk::EventSequenceState::Claimed);
        step_history(step);
    });
    window.add_controller(gesture);
}

/// Scrolls `scroller` sideways with a plain mouse wheel, as the
/// breadcrumbs do in app.js; GTK scrolls sideways only with Shift.
pub(super) fn scroll_sideways_with_wheel(scroller: &gtk::ScrolledWindow) {
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    let target = scroller.downgrade();
    wheel.connect_scroll(move |_, _, dy| {
        let Some(scroller) = target.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let adjustment = scroller.hadjustment();
        if adjustment.upper() <= adjustment.page_size() {
            return glib::Propagation::Proceed;
        }
        let step = adjustment.step_increment().max(40.0);
        adjustment.set_value(adjustment.value() + dy * step);
        glib::Propagation::Stop
    });
    scroller.add_controller(wheel);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_buttons_step_through_history() {
        assert_eq!(history_step(BACK_BUTTON), Some(-1));
        assert_eq!(history_step(FORWARD_BUTTON), Some(1));
        assert_eq!(history_step(gdk::BUTTON_PRIMARY), None);
        assert_eq!(history_step(gdk::BUTTON_SECONDARY), None);
    }

    #[test]
    fn shift_brings_a_middle_clicked_tab_to_the_front() {
        assert_eq!(open_action(gdk::ModifierType::empty()), "win.open-tab-background");
        assert_eq!(open_action(gdk::ModifierType::SHIFT_MASK), "win.open-tab");
    }
}
