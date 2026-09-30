// SPDX-License-Identifier: AGPL-3.0-only
//! The bundled icons as the window paints them: glyphs in the text colour
//! of each appearance, the places' glyphs in their own colours, and those
//! in the text colour while the desktop asks for high contrast; the green
//! network bar and the red cross in theirs.

use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, graphene};
use ox_core::places::NetworkKind;

use super::geometry::button_for;
use super::support::art_image_showing;
use crate::icons::{Art, Connection};
use crate::test_support::harness::{
    descendants, skin, wait_for, wait_for_frames, wait_until, Fixture, TestWindow, ThemeGuard,
};
use crate::theme::contrast::Contrast;

/// Longer than the skin's 83 ms colour transitions (ui-spec.md M01).
pub(super) const TRANSITION_TIME: Duration = Duration::from_millis(150);

/// How far a painted colour channel may stray from the CSS colour: the
/// rounding of 8-bit, premultiplied pixels.
const CHANNEL_TOLERANCE: f32 = 0.02;

/// A pixel covered at least this much shows its ink's colour exactly
/// enough to compare, once its premultiplied colour is divided back out.
const INKED_ALPHA: u8 = 128;

/// The green network bar, the current app's pipe, in both appearances
/// (`@ox_network_bar`).
const NETWORK_BAR_GREEN: &str = "#35a854";

/// The red cross of a disconnected share, in both appearances
/// (`@ox_critical_fill`).
const CROSS_RED: &str = "#c42b1c";

/// A pixel in GDK's default memory format: blue, green, red and alpha,
/// premultiplied.
pub(super) type Pixel = [u8; 4];

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

/// What `widget` looks like on its window, rendered by the window's own
/// renderer.
pub(super) fn painted(widget: &impl IsA<gtk::Widget>) -> gdk::Texture {
    let renderer = widget
        .native()
        .and_then(|native| native.renderer())
        .expect("the widget is on a drawn window");
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let snapshot = gtk::Snapshot::new();
    let width = f64::from(paintable.intrinsic_width());
    let height = f64::from(paintable.intrinsic_height());
    paintable.snapshot(&snapshot, width, height);
    let node = snapshot.to_node().expect("the widget paints something");
    renderer.render_texture(&node, None::<&graphene::Rect>)
}

/// The pixels of `texture`, row by row from the top.
pub(super) fn pixel_rows(texture: &gdk::Texture) -> Vec<Vec<Pixel>> {
    let downloader = gdk::TextureDownloader::new(texture);
    let (bytes, stride) = downloader.download_bytes();
    let width = usize::try_from(texture.width()).expect("a texture's width is positive");
    let rows = bytes.chunks(stride).map(|row| &row[..width * 4]);
    rows.map(|row| row.as_chunks::<4>().0.to_vec()).collect()
}

/// The colour `image` paints its icon in: the average of its inked
/// pixels.
fn painted_ink(image: &gtk::Image) -> gdk::RGBA {
    let pixels = pixel_rows(&painted(image)).concat();
    average_inked_colour(&pixels)
}

/// The average un-premultiplied colour of the `pixels` covered at least
/// [`INKED_ALPHA`].
fn average_inked_colour(pixels: &[Pixel]) -> gdk::RGBA {
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

/// The red, green and blue of `pixel`, with the alpha divided back out.
pub(super) fn unpremultiplied_rgb(pixel: Pixel) -> [f32; 3] {
    let [blue, green, red, alpha] = pixel.map(f32::from);
    [red / alpha, green / alpha, blue / alpha]
}

/// Whether `painted` is `expected`, channel by channel, within
/// [`CHANNEL_TOLERANCE`].
pub(super) fn is_same_colour(painted: gdk::RGBA, expected: gdk::RGBA) -> bool {
    let channels = [
        (painted.red(), expected.red()),
        (painted.green(), expected.green()),
        (painted.blue(), expected.blue()),
    ];
    channels
        .iter()
        .all(|(painted, expected)| (painted - expected).abs() <= CHANNEL_TOLERANCE)
}

/// Asserts that `painted` is `expected`, channel by channel.
pub(super) fn assert_same_colour(painted: gdk::RGBA, expected: gdk::RGBA, what: &str) {
    let is_same = is_same_colour(painted, expected);
    assert!(is_same, "{what}: painted {painted}, expected {expected}");
}

/// Whether some fully covered pixel of `pixels` is `colour`.
fn shows_colour(pixels: &[Pixel], colour: gdk::RGBA) -> bool {
    let opaque = pixels.iter().filter(|pixel| pixel[3] == u8::MAX);
    let colours = opaque.map(|pixel| {
        let [red, green, blue] = unpremultiplied_rgb(*pixel);
        gdk::RGBA::new(red, green, blue, 1.0)
    });
    colours.into_iter().any(|painted| is_same_colour(painted, colour))
}

/// A colour of the skin, in CSS notation.
pub(super) fn css_colour(css: &str) -> gdk::RGBA {
    gdk::RGBA::parse(css).expect("a CSS colour")
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
    let home_blue = css_colour("#0078d4");
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

/// The network bar and the red cross are boxes and a glyph the skin
/// colours: painted, the bottom of a disconnected mapped drive's icon is
/// the current app's pipe green, and its cross shows the critical red.
///
/// parity: LOOK-016
#[gtk::test]
fn the_network_bar_and_the_red_cross_are_painted_in_their_colours() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.save_share("smb://nas/media", "Media (M:)");
    let crossed_out_drive =
        Art::for_network_location(NetworkKind::Share, "Media (M:)", Connection::Disconnected);
    let sidebar = test.window.sidebar();
    wait_until("the saved share's row", || {
        art_image_showing(sidebar, crossed_out_drive).is_some()
    });
    wait_for_frames(&test.window, 2);
    let icon = art_image_showing(sidebar, crossed_out_drive).expect("the row shows its icon");
    let rows = pixel_rows(&painted(&icon));
    // At the sidebar's 19 pixels the bar is the bottom 2 rows
    // (icons::composition).
    let bar = rows[rows.len() - 2..].concat();
    assert_same_colour(
        average_inked_colour(&bar),
        css_colour(NETWORK_BAR_GREEN),
        "the bar",
    );
    assert!(
        shows_colour(&rows.concat(), css_colour(CROSS_RED)),
        "the red cross"
    );
}
