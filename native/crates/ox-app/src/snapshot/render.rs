// SPDX-License-Identifier: AGPL-3.0-only
//! Drawing the snapshot: the window as the screen shows it, with the menus
//! open over it, saved as a PNG.
//!
//! A popover is a surface of its own, which a picture of the window leaves
//! out, so the menus are drawn over the window where their surfaces are.

use std::path::Path;

use gtk::prelude::*;
use gtk::{gdk, graphene};

#[cfg(test)]
use super::content_bounds;
use super::SnapshotError;
use crate::window::widget_tree::descendants;

/// A menu open over the window: a popover, which is a surface of its own.
#[derive(Debug, Clone)]
pub(crate) struct OpenMenu {
    /// The popover.
    pub popover: gtk::Popover,
    /// Where its widget's corner is, in the window's coordinates.
    pub corner: graphene::Point,
}

/// The menus open over `window`, with where each is. A widget in another
/// surface has no transform to the window's widgets, so the corner comes
/// from the surfaces: the popup's position on the window's surface, less
/// where each surface puts its widget.
pub(super) fn open_menus(window: &gtk::Window) -> Vec<OpenMenu> {
    let (window_x, window_y) = window.surface_transform();
    descendants::<gtk::Popover>(window)
        .into_iter()
        .filter(WidgetExt::is_mapped)
        .filter_map(|popover| {
            let popup = popover.surface()?.downcast::<gdk::Popup>().ok()?;
            let (popover_x, popover_y) = popover.surface_transform();
            let x = f64::from(popup.position_x()) + popover_x - window_x;
            let y = f64::from(popup.position_y()) + popover_y - window_y;
            #[expect(clippy::cast_possible_truncation, reason = "window measures are small")]
            let corner = graphene::Point::new(x as f32, y as f32);
            Some(OpenMenu { popover, corner })
        })
        .collect()
}

/// Saves what `window` shows now as a PNG at `path`: its title bar and
/// contents, without the frame of a window on a display without a
/// compositor.
///
/// # Errors
///
/// [`SnapshotError::NotDrawn`] before the window is shown, and
/// [`SnapshotError::Write`] when the file cannot be written.
#[cfg(test)]
pub(crate) fn save_png(window: &gtk::Window, path: &Path) -> Result<(), SnapshotError> {
    let content = content_bounds(window).ok_or(SnapshotError::NotDrawn)?;
    render_png(window, Some(content), path)
}

/// Saves what the surface `native` (a window or a popover) draws now as a
/// PNG at `path`, cropped to `crop` in logical pixels from the surface's
/// outer edge, or whole.
///
/// # Errors
///
/// [`SnapshotError::NotDrawn`] before the surface is shown, and
/// [`SnapshotError::Write`] when the file cannot be written.
#[cfg(test)]
pub(crate) fn render_png(
    native: &impl IsA<gtk::Native>,
    crop: Option<graphene::Rect>,
    path: &Path,
) -> Result<(), SnapshotError> {
    let native = native.upcast_ref::<gtk::Native>();
    let snapshot = gtk::Snapshot::new();
    let scale = scale_of(native);
    snapshot.scale(scale, scale);
    draw_surface(&snapshot, native);
    save_node(native, snapshot, crop, path)
}

/// Saves what `window` draws now as a PNG at `path`, cropped to `crop` in
/// logical pixels from its outer edge, with `menus` drawn over it where
/// they are open.
pub(super) fn render_png_with_menus(
    window: &gtk::Window,
    menus: &[OpenMenu],
    crop: graphene::Rect,
    path: &Path,
) -> Result<(), SnapshotError> {
    let native = window.upcast_ref::<gtk::Native>();
    let outer = window.compute_bounds(window).ok_or(SnapshotError::NotDrawn)?;
    let snapshot = gtk::Snapshot::new();
    let scale = scale_of(native);
    snapshot.scale(scale, scale);
    draw_surface(&snapshot, native);
    for menu in menus {
        let Some(menu_outer) = menu.popover.compute_bounds(&menu.popover) else {
            continue;
        };
        // The window is drawn from its outer edge, and so is the menu.
        let x = menu.corner.x() + menu_outer.x() - outer.x();
        let y = menu.corner.y() + menu_outer.y() - outer.y();
        snapshot.save();
        snapshot.translate(&graphene::Point::new(x, y));
        draw_surface(&snapshot, menu.popover.upcast_ref());
        snapshot.restore();
    }
    save_node(native, snapshot, Some(crop), path)
}

/// The scale of `native`'s surface: device pixels per logical pixel.
#[expect(clippy::cast_precision_loss, reason = "scale factors are small integers")]
fn scale_of(native: &gtk::Native) -> f32 {
    native.scale_factor() as f32
}

/// Draws what the surface `native` shows, from its outer edge, into
/// `snapshot`. The paintable is drawn at its own size: any other size
/// scales the picture, which blurs one-pixel lines.
fn draw_surface(snapshot: &gtk::Snapshot, native: &gtk::Native) {
    let paintable = gtk::WidgetPaintable::new(Some(native));
    let width = f64::from(paintable.intrinsic_width());
    let height = f64::from(paintable.intrinsic_height());
    paintable.snapshot(snapshot, width, height);
}

/// Renders `snapshot` with `native`'s renderer, cropped to `crop` in
/// logical pixels, and saves it as a PNG at `path`: one picture pixel per
/// device pixel, as the screen shows the surface (two per logical pixel
/// with `GDK_SCALE=2`).
fn save_node(
    native: &gtk::Native,
    snapshot: gtk::Snapshot,
    crop: Option<graphene::Rect>,
    path: &Path,
) -> Result<(), SnapshotError> {
    let renderer = native.renderer().ok_or(SnapshotError::NotDrawn)?;
    let node = snapshot.to_node().ok_or(SnapshotError::NotDrawn)?;
    let scale = scale_of(native);
    let device_crop = crop.map(|area| area.scale(scale, scale));
    let texture = renderer.render_texture(&node, device_crop.as_ref());
    texture.save_to_png(path).map_err(|source| SnapshotError::Write {
        path: path.to_owned(),
        source,
    })
}
