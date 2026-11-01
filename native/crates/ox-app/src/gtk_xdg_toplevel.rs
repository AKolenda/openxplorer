// SPDX-License-Identifier: AGPL-3.0-only
//! The `xdg_toplevel` of a GTK window on Wayland, which a tab's window is
//! attached to the pointer by in xdg-toplevel-drag-v1, as Chrome drags a
//! torn-out tab's window (TAB-034).
//!
//! GTK creates and owns that Wayland object and offers no way to reach it:
//! no GTK release up to 4.22 has an API for it, and libgtk-4 exports no
//! symbol that returns it. The only way is to read it where GTK keeps it, in
//! its private `GdkWaylandToplevel` structure. Reading private memory needs
//! `unsafe`, which the workspace denies everywhere else: this file is the one
//! exception, kept small so it can be reviewed whole, and the owner decides
//! whether to keep it.
//!
//! A wrong read cannot happen, because it is gated before anything is read:
//! - only GTK releases whose structures were checked are read
//!   ([`VERIFIED`]): the offsets come from the debugging information of a
//!   GTK 4.22.4 build, the three structures are the same in the sources of
//!   GTK 4.22.0 to 4.22.4, and Ubuntu's patches to 4.22.4 do not touch them;
//! - only when `GObject` reports the instance sizes those structures have, so
//!   a build that changed them is refused;
//! - only a surface whose type is exactly `GdkWaylandToplevel`;
//! - and only when the `wl_surface` field holds what GTK's public
//!   `gdk_wayland_surface_get_wl_surface` returns, so the layout is right as
//!   far as there.
//!
//! Only then is the `xdg_toplevel` field read, and libwayland checks that
//! the object is an `xdg_toplevel` before it is used. Anything else, X11
//! among it, answers `None`, and the caller carries the tab as before.

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr::NonNull;

use gdk_wayland::prelude::*;
use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::{gdk, glib};
use wayland_backend::client::ObjectId;
use wayland_client::{Connection, Proxy};
use wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel;

/// Where a GTK release keeps a window's Wayland objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Layout {
    /// GTK's major and minor version.
    release: (u32, u32),
    /// The micro versions checked, first and last.
    micros: (u32, u32),
    /// The instance sizes of `GdkSurface`, `GdkWaylandSurface` and
    /// `GdkWaylandToplevel`.
    sizes: [usize; 3],
    /// Where `display_server.wl_surface` is in `GdkWaylandSurface`.
    wl_surface: usize,
    /// Where `display_server.xdg_toplevel` is in `GdkWaylandToplevel`.
    xdg_toplevel: usize,
}

/// The GTK releases whose structures were checked, on 64-bit x86 only: GTK
/// 4.22.0 to 4.22.4, from `ptype /o` of a 4.22.4 build's debugging
/// information (`struct _GdkWaylandToplevel` and `_GdkWaylandSurface`).
#[cfg(target_arch = "x86_64")]
const VERIFIED: &[Layout] = &[Layout {
    release: (4, 22),
    micros: (0, 4),
    sizes: [232, 448, 816],
    wl_surface: 240,
    xdg_toplevel: 456,
}];

/// No layout was checked on other processors.
#[cfg(not(target_arch = "x86_64"))]
const VERIFIED: &[Layout] = &[];

/// The checked layout of GTK `(major, minor, micro)`, if any.
fn layout_for((major, minor, micro): (u32, u32, u32)) -> Option<&'static Layout> {
    VERIFIED.iter().find(|layout| {
        layout.release == (major, minor) && (layout.micros.0..=layout.micros.1).contains(&micro)
    })
}

/// The checked layout of the GTK this process runs, when `GObject` reports the
/// instance sizes it has.
fn running_layout() -> Option<&'static Layout> {
    let layout = layout_for((gtk::major_version(), gtk::minor_version(), gtk::micro_version()))?;
    let sizes = [
        instance_size(gdk::Surface::static_type()),
        instance_size(gdk_wayland::WaylandSurface::static_type()),
        instance_size(gdk_wayland::WaylandToplevel::static_type()),
    ];
    (sizes == layout.sizes).then_some(layout)
}

/// Whether the `xdg_toplevel` of this GTK's windows can be read.
pub(crate) fn is_readable() -> bool {
    running_layout().is_some()
}

/// The size of an instance of `type_`, as `GObject` registered it.
fn instance_size(type_: glib::Type) -> usize {
    let mut query = glib::gobject_ffi::GTypeQuery {
        type_: 0,
        type_name: std::ptr::null(),
        class_size: 0,
        instance_size: 0,
    };
    // SAFETY: `g_type_query` writes into the `GTypeQuery` it is given, which
    // lives on this stack frame, and `type_` is a registered type (a static
    // type of the GDK classes).
    unsafe { glib::gobject_ffi::g_type_query(type_.into_glib(), &raw mut query) };
    usize::try_from(query.instance_size).unwrap_or(0)
}

/// The pointer `offset` bytes into `toplevel`'s instance.
///
/// The caller checks that the instance has room for a pointer there, as
/// [`running_layout`] and its offsets ensure.
fn pointer_at(toplevel: &gdk_wayland::WaylandToplevel, offset: usize) -> *mut c_void {
    let instance = toplevel.as_ptr().cast::<u8>().cast_const();
    // SAFETY: the instance is at least `offset` plus a pointer long (the
    // sizes and offsets of a checked layout, tested below), `toplevel` keeps
    // it alive, and an unaligned read has no alignment requirement. Reading a
    // pointer's bits dereferences nothing.
    unsafe { instance.add(offset).cast::<*mut c_void>().read_unaligned() }
}

/// The `xdg_toplevel` of `window`, on `connection`, GTK's connection, or
/// `None` where it cannot be read safely (see the module documentation).
///
/// Use it at once: GTK destroys the object when the window hides, and the
/// proxy returned cannot tell.
pub(crate) fn xdg_toplevel(window: &impl IsA<gtk::Native>, connection: &Connection) -> Option<XdgToplevel> {
    let toplevel = window
        .surface()?
        .downcast::<gdk_wayland::WaylandToplevel>()
        .ok()?;
    let layout = running_layout()?;
    if toplevel.type_() != gdk_wayland::WaylandToplevel::static_type() {
        return None;
    }
    let wl_surface = toplevel.wl_surface_raw()?;
    if pointer_at(&toplevel, layout.wl_surface) != wl_surface.as_ptr() {
        return None;
    }
    let xdg_toplevel = NonNull::new(pointer_at(&toplevel, layout.xdg_toplevel))?;
    // SAFETY: with the layout checked above, this field is GTK's live
    // `xdg_toplevel` proxy of `toplevel` (or null, refused above), and
    // `from_ptr` refuses it unless libwayland names its interface
    // `xdg_toplevel`.
    let id = unsafe { ObjectId::from_ptr(XdgToplevel::interface(), xdg_toplevel.as_ptr().cast()) }.ok()?;
    XdgToplevel::from_id(connection, id).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only GTK 4.22.0 to 4.22.4 are read: the tester's Ubuntu GTK
    /// 4.22.4, and nothing newer, older or unchecked.
    ///
    /// parity: TAB-034
    #[test]
    #[cfg(target_arch = "x86_64")]
    fn only_checked_gtk_releases_are_read() {
        for micro in 0..=4 {
            assert!(layout_for((4, 22, micro)).is_some(), "4.22.{micro}");
        }
        for version in [(4, 22, 5), (4, 23, 0), (4, 20, 4), (4, 14, 5), (5, 0, 0)] {
            assert!(layout_for(version).is_none(), "{version:?}");
        }
    }

    /// Every field read lies inside the structure it belongs to, past its
    /// parent's fields.
    #[test]
    fn every_field_read_lies_inside_its_structure() {
        let pointer = std::mem::size_of::<*mut c_void>();
        for layout in VERIFIED {
            let [surface, wayland_surface, toplevel] = layout.sizes;
            assert!(layout.wl_surface >= surface && layout.wl_surface + pointer <= wayland_surface);
            assert!(layout.xdg_toplevel >= wayland_surface && layout.xdg_toplevel + pointer <= toplevel);
        }
    }

    /// `GObject` reports the instance size of a type it registered.
    #[gtk::test]
    fn gobject_reports_instance_sizes() {
        assert!(
            instance_size(gdk::Surface::static_type()) >= std::mem::size_of::<glib::gobject_ffi::GObject>()
        );
        assert!(
            instance_size(gdk_wayland::WaylandToplevel::static_type())
                > instance_size(gdk::Surface::static_type())
        );
    }

    /// The GTK the tests run, GTK 4.14 on Ubuntu 24.04, is not read: its
    /// structures were not checked.
    #[gtk::test]
    fn an_unchecked_gtk_is_not_read() {
        let running = (gtk::major_version(), gtk::minor_version(), gtk::micro_version());
        if layout_for(running).is_none() {
            assert!(!is_readable(), "GTK {running:?}");
        }
    }
}
