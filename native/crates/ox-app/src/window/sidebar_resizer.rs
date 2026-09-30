// SPDX-License-Identifier: AGPL-3.0-only
//! The sidebar resizer as a keyboard and screen-reader control.
//!
//! Ports `#sidebar-resizer` and the keys of `setupSidebarResize` in
//! `desktop/ui/app.js`: the handle of the workspace's `GtkPaned` is a
//! focusable separator named "Resize sidebar", titled "Drag to resize
//! sidebar · double-click to reset", that announces the width with its
//! limits. Left and Right change the width by 10 pixels (40 with Shift)
//! and Home returns it to 210; every change is saved. Dragging and the
//! double-click reset are [`super::preferences`]'.

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;

use super::preferences::{sidebar_widths, Preference, DEFAULT_SIDEBAR_WIDTH};
use super::BrowserWindow;

/// The resizer's hover text (`handle.title` in app.js).
const TOOLTIP: &str = "Drag to resize sidebar · double-click to reset";

/// How far an arrow key moves the resizer, and with Shift held.
const KEY_STEP: i32 = 10;
const SHIFT_KEY_STEP: i32 = 40;

/// The handle `GtkPaned` draws between its children.
fn paned_handle(paned: &gtk::Paned) -> Option<gtk::Widget> {
    let start = paned.start_child();
    let end = paned.end_child();
    std::iter::successors(paned.first_child(), WidgetExt::next_sibling)
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
    /// Makes the pane handle a named, focusable separator that the arrow
    /// keys and Home move.
    pub(super) fn install_sidebar_resizer(&self) {
        let workspace = self.workspace();
        let Some(handle) = paned_handle(workspace) else {
            return;
        };
        handle.set_focusable(true);
        handle.set_tooltip_text(Some(TOOLTIP));
        handle.add_css_class("sidebar-resizer");
        handle.update_property(&[gtk::accessible::Property::Label("Resize sidebar")]);
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
        handle.add_controller(keys);
        workspace.connect_position_notify(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.announce_sidebar_width()
        ));
        self.announce_sidebar_width();
    }

    /// Moves the sidebar for `key`, within its limits, and saves the new
    /// width; false for a key the resizer does not take.
    pub(super) fn resize_sidebar_by_key(&self, key: gdk::Key, shift: bool) -> bool {
        let workspace = self.workspace();
        let Some(wanted) = width_for_key(key, shift, workspace.position()) else {
            return false;
        };
        let widest = self.sidebar_limit().unwrap_or(*sidebar_widths().end());
        let width = wanted.clamp(*sidebar_widths().start(), widest);
        workspace.set_position(width);
        self.save_preference(Preference::SidebarWidth(width));
        true
    }

    /// Tells screen readers the sidebar width and its limits
    /// (`aria-valuemin`, `aria-valuemax`, `aria-valuenow`).
    fn announce_sidebar_width(&self) {
        let workspace = self.workspace();
        let Some(handle) = paned_handle(workspace) else {
            return;
        };
        let widest = self.sidebar_limit().unwrap_or(*sidebar_widths().end());
        handle.update_property(&[
            gtk::accessible::Property::ValueMin(f64::from(*sidebar_widths().start())),
            gtk::accessible::Property::ValueMax(f64::from(widest)),
            gtk::accessible::Property::ValueNow(f64::from(workspace.position())),
        ]);
    }

    /// The pane handle, for tests.
    #[cfg(test)]
    pub(super) fn sidebar_resizer(&self) -> gtk::Widget {
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
