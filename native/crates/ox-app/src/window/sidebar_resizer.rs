// SPDX-License-Identifier: AGPL-3.0-only
//! The sidebar resizer as a keyboard and screen-reader control.
//!
//! Ports `#sidebar-resizer` and the keys of `setupSidebarResize` in
//! `desktop/ui/app.js`. The pointer drags the handle of the workspace's
//! `GtkPaned`, titled "Drag to resize sidebar · double-click to reset";
//! dragging and the double-click reset are [`super::preferences`]'.
//! GTK gives that handle a generic role it cannot change, so a separator
//! of no width beside it (`sidebar_resizer` in `window.ui`) stands for it:
//! it is the Tab stop between the sidebar and the files, named "Resize
//! sidebar" with the separator role and the width and its limits as its
//! value, and lights the handle while it has focus. Left and Right change
//! the width by 10 pixels (40 with Shift) and Home returns it to 210;
//! every change is saved.

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::preferences::{sidebar_widths, Preference, DEFAULT_SIDEBAR_WIDTH};
use super::BrowserWindow;
use crate::resizer_control::ResizerControl;

/// The resizer's hover text (`handle.title` in app.js).
const TOOLTIP: &str = "Drag to resize sidebar · double-click to reset";

/// How far an arrow key moves the resizer, and with Shift held.
const KEY_STEP: i32 = 10;
const SHIFT_KEY_STEP: i32 = 40;

/// The handle `GtkPaned` draws between its children.
fn paned_handle(paned: &gtk::Paned) -> Option<gtk::Widget> {
    let start = paned.start_child();
    let end = paned.end_child();
    super::widget_tree::children(paned)
        .find(|child| Some(child) != start.as_ref() && Some(child) != end.as_ref())
}

/// The sidebar width a key asks for from `width`, before the limits;
/// `None` for a key the resizer does not take.
fn width_for_key(key: gdk::Key, shift: bool, width: i32) -> Option<i32> {
    let step = if shift { SHIFT_KEY_STEP } else { KEY_STEP };
    match key {
        gdk::Key::Left | gdk::Key::KP_Left => Some(width - step),
        gdk::Key::Right | gdk::Key::KP_Right => Some(width + step),
        gdk::Key::Home | gdk::Key::KP_Home => Some(DEFAULT_SIDEBAR_WIDTH),
        _ => None,
    }
}

impl BrowserWindow {
    /// Titles the pane handle and makes the separator beside it the named,
    /// focusable resizer that the arrow keys and Home move.
    pub(super) fn install_sidebar_resizer(&self) {
        let workspace = self.workspace();
        let Some(handle) = paned_handle(workspace) else {
            return;
        };
        handle.set_tooltip_text(Some(TOOLTIP));
        handle.add_css_class("sidebar-resizer");
        // The separator speaks for the handle, so screen readers meet the
        // resizer once.
        handle.update_state(&[gtk::accessible::State::Hidden(true)]);
        let resizer = self.sidebar_resizer();
        resizer.set_tooltip_text(Some(TOOLTIP));
        resizer.set_label("Resize sidebar");
        resizer.connect_value_requested(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |width| {
                #[expect(clippy::cast_possible_truncation, reason = "sidebar widths are small")]
                window.resize_sidebar_to(width.round() as i32);
            }
        ));
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| {
                let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
                if window.resize_sidebar_by_key(key, shift) {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        ));
        resizer.add_controller(keys);
        let focus = gtk::EventControllerFocus::new();
        focus.connect_enter(glib::clone!(
            #[weak]
            handle,
            move |_| handle.add_css_class("keyboard-focus")
        ));
        focus.connect_leave(glib::clone!(
            #[weak]
            handle,
            move |_| handle.remove_css_class("keyboard-focus")
        ));
        resizer.add_controller(focus);
        workspace.connect_position_notify(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.announce_sidebar_width()
        ));
        self.announce_sidebar_width();
    }

    /// The resizer's keyboard and screen-reader side.
    pub(super) fn sidebar_resizer(&self) -> &ResizerControl {
        &self.imp().sidebar_resizer
    }

    /// Moves the sidebar for `key`, within its limits, and saves the new
    /// width; false for a key the resizer does not take.
    pub(super) fn resize_sidebar_by_key(&self, key: gdk::Key, shift: bool) -> bool {
        let Some(wanted) = width_for_key(key, shift, self.workspace().position()) else {
            return false;
        };
        self.resize_sidebar_to(wanted);
        true
    }

    /// Makes the sidebar `wanted` pixels wide, within its limits, and saves
    /// the width.
    fn resize_sidebar_to(&self, wanted: i32) {
        let widest = self.sidebar_limit().unwrap_or(*sidebar_widths().end());
        let width = wanted.clamp(*sidebar_widths().start(), widest);
        self.workspace().set_position(width);
        self.save_preference(Preference::SidebarWidth(width));
    }

    /// Tells screen readers the sidebar width and its limits
    /// (`aria-valuemin`, `aria-valuemax`, `aria-valuenow`).
    fn announce_sidebar_width(&self) {
        let widest = self.sidebar_limit().unwrap_or(*sidebar_widths().end());
        let width = self.workspace().position();
        self.sidebar_resizer()
            .set_values(*sidebar_widths().start(), widest, width);
    }

    /// The pane handle, for tests.
    #[cfg(test)]
    pub(super) fn sidebar_handle(&self) -> gtk::Widget {
        paned_handle(self.workspace()).expect("the workspace has a handle")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: SIDE-023, ACC-006
    #[test]
    fn arrows_move_the_resizer_10_pixels_shift_40_and_home_resets_it() {
        assert_eq!(width_for_key(gdk::Key::Left, false, 250), Some(240));
        assert_eq!(width_for_key(gdk::Key::Right, false, 250), Some(260));
        assert_eq!(width_for_key(gdk::Key::Left, true, 250), Some(210));
        assert_eq!(width_for_key(gdk::Key::Right, true, 250), Some(290));
        assert_eq!(width_for_key(gdk::Key::Home, false, 400), Some(210));
        assert_eq!(width_for_key(gdk::Key::Up, false, 250), None);
    }
}
