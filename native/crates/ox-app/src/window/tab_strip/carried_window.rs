// SPDX-License-Identifier: AGPL-3.0-only
//! What follows the pointer while GTK's drag and drop carries a tab out of
//! its strip, as Chrome's drag does where the desktop cannot move a real
//! window with it (TAB-034): a picture of the whole window, title bar and
//! all, [`HEIGHT`] pixels tall, Chrome's being 200 (`kTargetHeightDip`).
//! Over its own strip it goes away, and over another window's strip it hangs
//! below the pointer, by the drag's hotspot alone.
//!
//! The picture is drawn once, into a texture, when the drag starts: the
//! window shows the dragged tab then, and a texture draws the same on any
//! renderer, where a live picture of a widget in another window did not.

use gtk::prelude::*;

/// How tall the picture of the window is.
const HEIGHT: f64 = 240.0;

/// The picture of `window` that follows a tab carried out of its strip:
/// its title bar over its contents, drawn now. (A picture of the toplevel
/// itself draws nothing, and one of a widget only shows its last drawing,
/// which a change throws away.)
pub(super) fn carried_window(window: &gtk::Widget) -> gtk::Widget {
    let Some(window) = window.downcast_ref::<gtk::Window>() else {
        return gtk::Box::new(gtk::Orientation::Vertical, 0).upcast();
    };
    let parts: Vec<gtk::Widget> = [window.titlebar(), window.child()]
        .into_iter()
        .flatten()
        .collect();
    let width = parts.iter().map(WidgetExt::width).max().unwrap_or(1).max(1);
    let height = parts.iter().map(WidgetExt::height).sum::<i32>().max(1);
    let scale = (HEIGHT / f64::from(height)).min(1.0);
    let snapshot = gtk::Snapshot::new();
    #[expect(clippy::cast_possible_truncation, reason = "a scale near 1")]
    snapshot.scale(scale as f32, scale as f32);
    // Drawn afresh: a widget's last drawing is gone once it changed, as
    // the contents just did for the tab that came to the front.
    for part in &parts {
        window.snapshot_child(part, &snapshot);
    }
    let texture = snapshot.to_node().and_then(|node| {
        let renderer = window.renderer()?;
        Some(renderer.render_texture(&node, None))
    });
    #[expect(clippy::cast_possible_truncation, reason = "a picture's size")]
    let size = (
        (f64::from(width) * scale).round() as i32,
        (f64::from(height) * scale).round() as i32,
    );
    let frame = gtk::Box::builder()
        .width_request(size.0)
        .height_request(size.1)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["tab-drag-window"])
        .build();
    if let Some(texture) = texture {
        let picture = gtk::Picture::for_paintable(&texture);
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        frame.append(&picture);
    }
    frame.upcast()
}

#[cfg(test)]
mod tests {
    use gtk::gdk;
    use gtk::prelude::*;

    use crate::test_support::harness::{wait_for_frames, Fixture, TestWindow};

    /// The picture carried out of the strip is the window as drawn now,
    /// title bar and contents, about 240 pixels tall, not an empty box:
    /// a picture of the toplevel, or of a widget that just changed, drew
    /// nothing.
    ///
    /// parity: TAB-034
    #[gtk::test]
    fn the_carried_picture_shows_the_whole_window() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        wait_for_frames(&test.window, 3);

        let frame = super::carried_window(test.window.upcast_ref());
        let texture = frame
            .first_child()
            .and_downcast::<gtk::Picture>()
            .and_then(|picture| picture.paintable())
            .and_downcast::<gdk::Texture>()
            .expect("the picture is drawn into a texture");
        let (width, height) = (texture.width(), texture.height());
        let mut pixels = vec![0_u8; usize::try_from(width * height * 4).expect("a small picture")];
        texture.download(&mut pixels, usize::try_from(width * 4).expect("a small picture"));
        let colours: std::collections::BTreeSet<[u8; 3]> = pixels
            .chunks(4)
            .step_by(7)
            .map(|pixel| [pixel[0], pixel[1], pixel[2]])
            .collect();

        assert!((235..=245).contains(&height), "about 240 pixels tall: {height}");
        assert!(width > height, "window-shaped: {width}x{height}");
        assert!(
            colours.len() > 40,
            "it shows the window, not a blank box: {} colours",
            colours.len()
        );
    }
}
