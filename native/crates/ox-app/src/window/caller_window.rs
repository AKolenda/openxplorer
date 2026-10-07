// SPDX-License-Identifier: AGPL-3.0-only
//! Ties a file dialog to the window of the application that opened it
//! (INT-032), as Windows' dialog and KDE's own do: the dialog stays above
//! that window, and the window cannot be used until the dialog answers.
//!
//! The portal names the caller's window `wayland:<handle>`, a handle the
//! caller exported with xdg-foreign, or `x11:<id>`, or nothing. On Wayland
//! the dialog imports the handle and becomes a child of that window
//! (`gdk_wayland_toplevel_set_transient_for_exported`), then modal, which
//! GTK tells the compositor with xdg-dialog (GTK 4.22 does, 4.14 does not
//! yet): `KWin` keeps a modal child above its parent and sends a click on
//! the parent to the child. GTK has no way to make an X11 window a child of another
//! process's window, and a modal window without a parent could lock the
//! other `OpenXplorer` windows instead, so such a dialog stays a window of
//! its own, as before.
//!
//! The dialog has a window group of its own either way: GTK's modality
//! blocks the other windows of the group, and the other `OpenXplorer`
//! windows stay usable while the caller waits.

use std::cell::Cell;
use std::rc::Rc;

use gdk_wayland::prelude::*;
use gtk::glib;
use gtk::prelude::*;

/// The caller's window, as the portal names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CallerWindow {
    /// A handle exported with xdg-foreign.
    Wayland(String),
    /// An X11 window ID.
    X11(u64),
    /// No window, or one this build cannot read.
    Unknown,
}

impl CallerWindow {
    /// Reads the portal's `parent_window`: `wayland:<handle>`, `x11:<id in
    /// hex>` or empty.
    pub(super) fn parse(parent: &str) -> Self {
        if let Some(handle) = parent.strip_prefix("wayland:") {
            let usable = !handle.is_empty() && handle.chars().all(|c| c.is_ascii_graphic());
            return if usable {
                Self::Wayland(handle.to_owned())
            } else {
                Self::Unknown
            };
        }
        if let Some(id) = parent.strip_prefix("x11:") {
            let id = id.trim_start_matches("0x");
            return u64::from_str_radix(id, 16).map_or(Self::Unknown, Self::X11);
        }
        Self::Unknown
    }
}

/// Gives the dialog `window` a window group of its own and, when the
/// caller named a Wayland window, makes it a modal child of that window
/// once it is realized. The flag returned turns true then.
pub(super) fn attach_to_caller(window: &gtk::Window, parent: &str) -> Rc<Cell<bool>> {
    gtk::WindowGroup::new().add_window(window);
    let attached: Rc<Cell<bool>> = Rc::default();
    let CallerWindow::Wayland(handle) = CallerWindow::parse(parent) else {
        return attached;
    };
    let done = Rc::clone(&attached);
    let attach = move |window: &gtk::Window| {
        let Some(toplevel) = window.surface().and_downcast::<gdk_wayland::WaylandToplevel>() else {
            // Not on Wayland after all (GDK_BACKEND=x11 under a Wayland
            // session): nothing to attach to.
            return;
        };
        if toplevel.set_transient_for_exported(&handle) {
            window.set_modal(true);
            done.set(true);
        } else {
            glib::g_warning!(
                ox_core::LOG_DOMAIN,
                "The compositor cannot attach the dialog to its caller's window (no xdg-foreign)"
            );
        }
    };
    if window.is_realized() {
        attach(window);
    } else {
        // Before the window is first shown, so the compositor places the
        // dialog as the caller's child from the start.
        let handler: Rc<Cell<Option<glib::SignalHandlerId>>> = Rc::default();
        let first = Rc::clone(&handler);
        let realized = window.connect_realize(move |widget| {
            if let Some(id) = first.take() {
                widget.disconnect(id);
            }
            if let Some(window) = widget.downcast_ref::<gtk::Window>() {
                attach(window);
            }
        });
        handler.set(Some(realized));
    }
    attached
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: INT-032
    #[test]
    fn the_portal_names_the_callers_window_for_wayland_x11_or_not_at_all() {
        assert_eq!(
            CallerWindow::parse("wayland:7b1a-c3"),
            CallerWindow::Wayland("7b1a-c3".to_owned())
        );
        assert_eq!(CallerWindow::parse("x11:4c0000a"), CallerWindow::X11(0x4c0_000a));
        assert_eq!(
            CallerWindow::parse("x11:0x4c0000a"),
            CallerWindow::X11(0x4c0_000a)
        );
        for unusable in ["", "wayland:", "wayland:a b", "x11:", "x11:zz", "mir:1"] {
            assert_eq!(
                CallerWindow::parse(unusable),
                CallerWindow::Unknown,
                "{unusable:?}"
            );
        }
    }
}
