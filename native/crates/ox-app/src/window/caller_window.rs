// SPDX-License-Identifier: AGPL-3.0-only
//! Ties a file dialog to the window of the application that opened it
//! (INT-032), as Windows' dialog and KDE's own do: the dialog stays above
//! that window, and the window cannot be used until the dialog answers.
//!
//! The portal names the caller's window `wayland:<handle>`, a handle the
//! caller exported with xdg-foreign, or `x11:<id>`, or nothing. On Wayland
//! the dialog imports the handle and becomes a child of that window
//! (`gdk_wayland_toplevel_set_transient_for_exported`), then modal. GTK,
//! 4.14 included (`gdk_wayland_toplevel_set_modal_hint`), tells the
//! compositor with xdg-dialog when the compositor offers it, and otherwise
//! with gtk-shell's `gtk_surface1.set_modal`, which Mutter 46 turns into a
//! modal dialog (`META_WINDOW_MODAL_DIALOG`) of the window it is a child
//! of. `KWin` keeps a modal child above its parent and sends a click on
//! the parent to the child.
//!
//! Where Mutter's `attach-modal-dialogs` is on, as GNOME and Zorin OS
//! turn it on, Mutter glues a modal dialog to its parent: it centres the
//! dialog on the parent, and dragging the dialog moves the parent instead
//! (`meta_window_drag_begin` moves the window an attached dialog is
//! attached to). Windows' dialog moves on its own, so there the dialog
//! stays a child of the caller's window, above it, but is not modal. The
//! caller still waits for it: Chromium and GTK stop taking input in the
//! window that asked while their dialog is open. A Qt app asking through
//! the portal leaves that to the dialog, so there its window stays usable.
//!
//! GTK has no way to make an X11 window a child of another
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

/// Reads `parent` as a portal's parent window, for the fuzz targets.
#[cfg(feature = "fuzzing")]
pub(crate) fn fuzz_parent(parent: &str) {
    let _ = CallerWindow::parse(parent);
}

impl CallerWindow {
    /// Reads the portal's `parent_window`: `wayland:<handle>`, `x11:<id in
    /// hex>` or empty. A handle is printable ASCII, spaces included:
    /// Mutter draws each of its 32 characters from `' '` to `'~'`
    /// (`meta_generate_random_id`), so more than one handle in four has a
    /// space, and `KWin`'s are UUIDs.
    pub(super) fn parse(parent: &str) -> Self {
        if let Some(handle) = parent.strip_prefix("wayland:") {
            let usable = !handle.is_empty() && handle.chars().all(|c| (' '..='~').contains(&c));
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

/// Mutter's settings, and its key that glues a modal dialog to its
/// parent window.
const MUTTER_SCHEMA: &str = "org.gnome.mutter";
const ATTACH_MODAL_DIALOGS: &str = "attach-modal-dialogs";

/// Whether the compositor glues a modal dialog to its parent, so that
/// dragging the dialog would move the caller's window with it. Read from
/// Mutter's settings as the desktop sets them (`GSettings` takes the
/// desktop's own defaults from `XDG_CURRENT_DESKTOP`: Zorin OS's
/// `[org.gnome.mutter:zorin]` turns it on). The Flatpak serves no dialogs,
/// so it never needs to know.
pub(super) fn desktop_attaches_modal_dialogs() -> bool {
    crate::theme::host_desktop_key(MUTTER_SCHEMA, ATTACH_MODAL_DIALOGS)
        .is_some_and(|settings| settings.boolean(ATTACH_MODAL_DIALOGS))
}

/// Gives the dialog `window` a window group of its own and, when the
/// caller named a Wayland window, makes it a child of that window once it
/// is realized: a modal one, unless the desktop would then glue it to that
/// window ([`desktop_attaches_modal_dialogs`]). The flag returned turns
/// true then.
pub(super) fn attach_to_caller(window: &gtk::Window, parent: &str) -> Rc<Cell<bool>> {
    gtk::WindowGroup::new().add_window(window);
    let attached: Rc<Cell<bool>> = Rc::default();
    let CallerWindow::Wayland(handle) = CallerWindow::parse(parent) else {
        return attached;
    };
    let done = Rc::clone(&attached);
    let modal = !desktop_attaches_modal_dialogs();
    let attach = move |window: &gtk::Window| {
        let Some(toplevel) = window.surface().and_downcast::<gdk_wayland::WaylandToplevel>() else {
            // Not on Wayland after all (GDK_BACKEND=x11 under a Wayland
            // session): nothing to attach to.
            return;
        };
        if toplevel.set_transient_for_exported(&handle) {
            window.set_modal(modal);
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
        // Mutter's handles may hold spaces and any other printable ASCII.
        assert_eq!(
            CallerWindow::parse("wayland:7zNT Ij2Z|K; Pl\\z(4o"),
            CallerWindow::Wayland("7zNT Ij2Z|K; Pl\\z(4o".to_owned())
        );
        for unusable in [
            "",
            "wayland:",
            "wayland:a\tb",
            "wayland:a\u{e9}",
            "x11:",
            "x11:zz",
            "mir:1",
        ] {
            assert_eq!(
                CallerWindow::parse(unusable),
                CallerWindow::Unknown,
                "{unusable:?}"
            );
        }
    }
}
