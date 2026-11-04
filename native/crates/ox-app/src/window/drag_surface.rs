// SPDX-License-Identifier: AGPL-3.0-only
//! Keeps a finished drag and drop alive a moment longer, so GTK never
//! destroys its drag icon's Wayland surface while a frame for it is still
//! on its way.
//!
//! GTK's Wayland backend (4.14 to at least 4.18) never marks a drag
//! surface as mapped, so hiding or destroying it skips clearing the frame
//! callback it last asked for. GTK finalizes the drag, and destroys that
//! surface, as soon as the drag ends; when the compositor's answer to the
//! last frame was already in flight, it then reaches the destroyed surface
//! and GTK aborts the whole app in `frame_callback` with "assertion
//! failed: (`!GDK_SURFACE_DESTROYED (surface)`)". A tab dragged out quickly,
//! while its drag icon was still being drawn, did that on GNOME. Holding a
//! reference to the drag for a second after it ends lets that answer arrive
//! while the surface still exists; nothing draws on it by then.

use std::time::Duration;

use gtk::{gdk, glib};

/// How long a finished drag is kept: far longer than one frame.
const FINISHED_DRAG_KEPT: Duration = Duration::from_secs(1);

/// Keeps `drag`, which just ended, and so its drag icon's surface, for
/// [`FINISHED_DRAG_KEPT`].
pub(super) fn keep_finished_drag(drag: &gdk::Drag) {
    let drag = drag.clone();
    glib::timeout_add_local_once(FINISHED_DRAG_KEPT, move || drop(drag));
}
