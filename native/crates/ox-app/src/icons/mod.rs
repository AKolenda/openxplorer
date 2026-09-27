// SPDX-License-Identifier: AGPL-3.0-only
//! Icons: stroke glyphs drawn natively and colour art drawn from SVG.
//!
//! Glyphs ([`glyph`]) follow the widget's CSS colour; art ([`art_image`],
//! [`set_art`]) is rasterised at the widget's scale factor so it stays sharp
//! on HiDPI screens.

pub mod art;
pub mod glyphs;

use gtk::prelude::*;

pub use art::ArtKind;
pub use glyphs::GlyphPaintable;

use crate::theme::Appearance;

/// A centred image of the glyph `name` at `size` logical pixels, painted in
/// the CSS colour of the image (so hover and disabled states apply).
pub fn glyph(name: &str, size: i32) -> gtk::Image {
    glyph_image(GlyphPaintable::new(name, size, None), size)
}

/// Like [`glyph`], with extra CSS classes.
pub fn glyph_with_classes(name: &str, size: i32, classes: &[&str]) -> gtk::Image {
    let image = glyph(name, size);
    for class in classes {
        image.add_css_class(class);
    }
    image
}

/// A glyph in a fixed colour (CSS hex), for the coloured Quick access and
/// sidebar glyphs.
pub fn colored_glyph(name: &str, size: i32, color: &str) -> gtk::Image {
    glyph_image(GlyphPaintable::new(name, size, Some(color)), size)
}

fn glyph_image(paintable: GlyphPaintable, size: i32) -> gtk::Image {
    let image = gtk::Image::from_paintable(Some(&paintable));
    image.set_pixel_size(size);
    image.set_halign(gtk::Align::Center);
    image.set_valign(gtk::Align::Center);
    image.add_css_class("glyph");
    image
}

/// Replaces an image's content with the glyph `name`.
pub fn set_glyph(image: &gtk::Image, name: &str, size: i32) {
    image.set_paintable(Some(&GlyphPaintable::new(name, size, None)));
    image.set_pixel_size(size);
}

/// An image showing colour art at `size` logical pixels.
pub fn art_image(kind: &ArtKind, size: i32, appearance: Appearance, scale: i32) -> gtk::Image {
    let image = gtk::Image::new();
    set_art(&image, kind, size, appearance, scale);
    image
}

/// Replaces an image's content with colour art, rasterised for `scale`.
pub fn set_art(image: &gtk::Image, kind: &ArtKind, size: i32, appearance: Appearance, scale: i32) {
    let pixels = size * scale.max(1);
    match art::texture(kind, appearance, pixels) {
        Some(texture) => image.set_paintable(Some(&texture)),
        None => image.set_paintable(None::<&gtk::gdk::Paintable>),
    }
    image.set_pixel_size(size);
}
