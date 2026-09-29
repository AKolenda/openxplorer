// SPDX-License-Identifier: AGPL-3.0-only
//! The handle between the sidebar and the folder pane: its tooltip, the
//! width it announces, and resizing the sidebar with the keyboard.
//!
//! Ports the keyboard and ARIA half of the sidebar resizer in
//! `desktop/ui/app.js` (`#sidebar-resizer`, `role=separator`): Left and
//! Right narrow or widen the sidebar by 10 pixels, by 40 with Shift, and
//! Home returns it to 210; each change is saved. The workspace split is a
//! `GtkPaned`, whose handle takes keyboard focus with F8, GNOME's key for
//! pane handles, where the web page put the resizer in the Tab order. The
//! pointer half (dragging, double-click to reset, the widest the sidebar
//! may be) is [`super::preferences`]'.

use gtk::prelude::*;
use gtk::{gdk, glib};

use super::preferences::{clamp_sidebar_width, sidebar_widths, Preference, DEFAULT_SIDEBAR_WIDTH};
use super::BrowserWindow;

/// The handle's tooltip (`title` of `#sidebar-resizer`).
const HANDLE_TOOLTIP: &str = "Drag to resize sidebar · double-click to reset";

/// Pixels an arrow key moves the handle.
const KEY_STEP: i32 = 10;

/// Pixels an arrow key moves the handle with Shift held.
const SHIFT_KEY_STEP: i32 = 40;

/// The sidebar width `key` asks for, from `current`; `None` for a key
/// the resizer does not take.
fn keyed_sidebar_width(current: i32, key: gdk::Key, modifiers: gdk::ModifierType) -> Option<i32> {
    let step = if modifiers.contains(gdk::ModifierType::SHIFT_MASK) {
        SHIFT_KEY_STEP
    } else {
        KEY_STEP
    };
    let width = match key {
        gdk::Key::Left | gdk::Key::KP_Left => current - step,
        gdk::Key::Right | gdk::Key::KP_Right => current + step,
        gdk::Key::Home | gdk::Key::KP_Home => DEFAULT_SIDEBAR_WIDTH,
        _ => return None,
    };
    Some(clamp_sidebar_width(width))
}

/// The handle widget of `paned`: its one child that is neither pane.
fn paned_handle(paned: &gtk::Paned) -> Option<gtk::Widget> {
    let start = paned.start_child();
    let end = paned.end_child();
    super::widget_tree::children(paned)
        .find(|child| Some(child) != start.as_ref() && Some(child) != end.as_ref())
}

impl BrowserWindow {
    /// Gives the workspace handle its tooltip, announces the sidebar width
    /// as the separator's value, and lets the keyboard resize it.
    pub(super) fn install_sidebar_resizer(&self) {
        let workspace = self.workspace();
        if let Some(handle) = paned_handle(workspace) {
            handle.set_tooltip_text(Some(HANDLE_TOOLTIP));
        }
        announce_sidebar_width(workspace);
        workspace.connect_position_notify(announce_sidebar_width);
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| window.resize_sidebar_with_key(key, modifiers)
        ));
        workspace.add_controller(keys);
    }

    /// Moves the focused handle for `key` and saves the new width; keys
    /// pressed anywhere else in the workspace go on.
    fn resize_sidebar_with_key(&self, key: gdk::Key, modifiers: gdk::ModifierType) -> glib::Propagation {
        let workspace = self.workspace();
        if !workspace.is_focus() {
            return glib::Propagation::Proceed;
        }
        let Some(width) = keyed_sidebar_width(workspace.position(), key, modifiers) else {
            return glib::Propagation::Proceed;
        };
        workspace.set_position(width);
        // The workspace may have held the handle back to keep the folder
        // pane's room; the width shown is the one saved.
        self.save_preference(Preference::SidebarWidth(workspace.position()));
        glib::Propagation::Stop
    }
}

/// Announces the sidebar's width and its limits as the handle's value
/// (`aria-valuenow`, `aria-valuemin`, `aria-valuemax`).
fn announce_sidebar_width(workspace: &gtk::Paned) {
    let widths = sidebar_widths();
    workspace.update_property(&[
        gtk::accessible::Property::ValueMin(f64::from(*widths.start())),
        gtk::accessible::Property::ValueMax(f64::from(*widths.end())),
        gtk::accessible::Property::ValueNow(f64::from(workspace.position())),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One key press on the handle and the width it asks for.
    struct KeyCase {
        key: gdk::Key,
        modifiers: gdk::ModifierType,
        from: i32,
        width: Option<i32>,
    }

    /// parity: SIDE-023
    #[test]
    fn arrows_move_the_handle_10_pixels_40_with_shift_and_home_resets_it() {
        let none = gdk::ModifierType::empty();
        let shift = gdk::ModifierType::SHIFT_MASK;
        let cases = [
            KeyCase {
                key: gdk::Key::Left,
                modifiers: none,
                from: 250,
                width: Some(240),
            },
            KeyCase {
                key: gdk::Key::Right,
                modifiers: none,
                from: 250,
                width: Some(260),
            },
            KeyCase {
                key: gdk::Key::Left,
                modifiers: shift,
                from: 250,
                width: Some(210),
            },
            KeyCase {
                key: gdk::Key::Right,
                modifiers: shift,
                from: 250,
                width: Some(290),
            },
            KeyCase {
                key: gdk::Key::Home,
                modifiers: none,
                from: 400,
                width: Some(210),
            },
            KeyCase {
                key: gdk::Key::Left,
                modifiers: shift,
                from: 150,
                width: Some(140),
            },
            KeyCase {
                key: gdk::Key::Right,
                modifiers: shift,
                from: 550,
                width: Some(560),
            },
            KeyCase {
                key: gdk::Key::Up,
                modifiers: none,
                from: 250,
                width: None,
            },
        ];
        for case in cases {
            let width = keyed_sidebar_width(case.from, case.key, case.modifiers);
            assert_eq!(
                width, case.width,
                "{:?} {:?} from {}",
                case.key, case.modifiers, case.from
            );
        }
    }
}
