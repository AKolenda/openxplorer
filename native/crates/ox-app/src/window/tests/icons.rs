// SPDX-License-Identifier: AGPL-3.0-only
//! The bundled icons as the window paints them: glyphs in the text colour
//! of each appearance, the places' glyphs in their own colours, and those
//! in the text colour while the desktop asks for high contrast.

use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, graphene};

use super::geometry::button_for;
use crate::test_support::harness::{
    descendants, skin, wait_for, wait_for_frames, Fixture, TestWindow, ThemeGuard,
};
use crate::theme::contrast::Contrast;

/// Longer than the skin's 83 ms colour transitions (ui-spec.md M01).
const TRANSITION_TIME: Duration = Duration::from_millis(150);

/// How far a painted colour channel may stray from the CSS colour: the
/// rounding of 8-bit, premultiplied pixels.
const CHANNEL_TOLERANCE: f32 = 0.02;

/// A pixel covered at least this much shows its ink's colour exactly
/// enough to compare, once its premultiplied colour is divided back out.
const INKED_ALPHA: u8 = 128;

/// Restores normal contrast when a test that raised it ends, even when an
/// assertion fails, so later tests see the skin as designed.
struct ContrastGuard;

impl ContrastGuard {
    /// Raises the skin's contrast until the guard is dropped.
    fn raise() -> Self {
        skin().set_contrast(Contrast::High);
        Self
    }
}

impl Drop for ContrastGuard {
    fn drop(&mut self) {
        skin().set_contrast(Contrast::Normal);
    }
}

/// The glyph image of the Refresh button, which is never disabled.
fn refresh_glyph(test: &TestWindow) -> gtk::Image {
    let button = button_for(test, "win.refresh");
    button
        .child()
        .and_downcast::<gtk::Image>()
        .expect("the button shows a glyph")
}

/// The sidebar's image showing the glyph tinted with `tint_class`.
fn sidebar_glyph(test: &TestWindow, tint_class: &str) -> gtk::Image {
    descendants::<gtk::Image>(test.window.sidebar())
        .into_iter()
        .find(|image| image.has_css_class(tint_class))
        .unwrap_or_else(|| panic!("the sidebar shows a {tint_class} glyph"))
}

/// The colour `image` paints its icon in: its most opaque pixels, with
/// their premultiplied colour divided back out.
fn painted_ink(image: &gtk::Image) -> gdk::RGBA {
    let renderer = image
        .native()
        .and_then(|native| native.renderer())
        .expect("the image is on a drawn window");
    let paintable = gtk::WidgetPaintable::new(Some(image));
    let snapshot = gtk::Snapshot::new();
    let width = f64::from(paintable.intrinsic_width());
    let height = f64::from(paintable.intrinsic_height());
    paintable.snapshot(&snapshot, width, height);
    let node = snapshot.to_node().expect("the glyph paints something");
    let texture = renderer.render_texture(&node, None::<&graphene::Rect>);
    most_opaque_colour(&texture)
}

/// The average un-premultiplied colour of the pixels of `texture` covered
/// at least [`INKED_ALPHA`].
fn most_opaque_colour(texture: &gdk::Texture) -> gdk::RGBA {
    let downloader = gdk::TextureDownloader::new(texture);
    let (bytes, _stride) = downloader.download_bytes();
    let (pixels, _) = bytes.as_chunks::<4>();
    let inked: Vec<[f32; 3]> = pixels
        .iter()
        .filter(|pixel| pixel[3] >= INKED_ALPHA)
        .map(|pixel| unpremultiplied_rgb(*pixel))
        .collect();
    assert!(!inked.is_empty(), "the icon covers some pixels");
    #[expect(clippy::cast_precision_loss, reason = "an icon has few pixels")]
    let count = inked.len() as f32;
    let channel = |index: usize| inked.iter().map(|rgb| rgb[index]).sum::<f32>() / count;
    gdk::RGBA::new(channel(0), channel(1), channel(2), 1.0)
}

/// The red, green and blue of a pixel in GDK's default memory format
/// (blue, green, red and alpha, premultiplied), with the alpha divided
/// back out.
fn unpremultiplied_rgb(pixel: [u8; 4]) -> [f32; 3] {
    let [blue, green, red, alpha] = pixel.map(f32::from);
    [red / alpha, green / alpha, blue / alpha]
}

/// Asserts that `painted` is `expected`, channel by channel.
fn assert_same_colour(painted: gdk::RGBA, expected: gdk::RGBA, what: &str) {
    let channels = [
        (painted.red(), expected.red()),
        (painted.green(), expected.green()),
        (painted.blue(), expected.blue()),
    ];
    let is_same = channels
        .iter()
        .all(|(painted, expected)| (painted - expected).abs() <= CHANNEL_TOLERANCE);
    assert!(is_same, "{what}: painted {painted}, expected {expected}");
}

/// The relative lightness of `colour`, from 0 (black) to 1 (white).
fn lightness(colour: gdk::RGBA) -> f32 {
    0.2126 * colour.red() + 0.7152 * colour.green() + 0.0722 * colour.blue()
}

/// A symbolic Fluent glyph is painted in its image's CSS colour: dark ink
/// in the light appearance and light ink in the dark one.
///
/// parity: LOOK-015
#[gtk::test]
fn glyphs_are_painted_in_the_text_colour_of_each_appearance() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    for theme in ["light", "dark"] {
        test.activate("theme", Some(theme));
        wait_for(TRANSITION_TIME);
        wait_for_frames(&test.window, 2);
        let glyph = refresh_glyph(&test);
        let ink = painted_ink(&glyph);
        assert_same_colour(ink, glyph.color(), theme);
        let is_dark_ink = lightness(ink) < 0.3;
        assert_eq!(is_dark_ink, theme == "light", "{theme} ink {ink}");
    }
}

/// Home and the standard folders keep the current app's glyph colours
/// (`im.style.color` in `renderSidebar`), the same in both appearances.
///
/// parity: LOOK-015
#[gtk::test]
fn the_sidebar_glyphs_keep_their_places_colours_in_both_appearances() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let home_blue = gdk::RGBA::parse("#0078d4").expect("a CSS colour");
    for theme in ["light", "dark"] {
        test.activate("theme", Some(theme));
        wait_for(TRANSITION_TIME);
        wait_for_frames(&test.window, 2);
        let home = sidebar_glyph(&test, "tint-home");
        assert_same_colour(home.color(), home_blue, theme);
        assert_same_colour(painted_ink(&home), home_blue, theme);
    }
}

/// With high contrast, a place's glyph takes the text colour, which always
/// stands out from the sidebar.
#[gtk::test]
fn high_contrast_paints_the_place_glyphs_in_the_text_colour() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let home = sidebar_glyph(&test, "tint-home");
    let tinted = home.color();
    let _contrast = ContrastGuard::raise();
    wait_for(TRANSITION_TIME);
    let row_text = home
        .ancestor(gtk::ListBoxRow::static_type())
        .and_then(|row| descendants::<gtk::Label>(&row).into_iter().next())
        .expect("the Home row has a name");
    assert_same_colour(home.color(), row_text.color(), "high contrast");
    assert_ne!(home.color(), tinted, "the tint gives way");
}
