// SPDX-License-Identifier: AGPL-3.0-only
//! Icons: stroke glyphs drawn natively and colour art drawn from SVG.
//!
//! Glyphs ([`glyph`]) follow the widget's CSS colour; art ([`art_image`],
//! [`set_art`]) is rasterised at the widget's scale factor so it stays sharp
//! on high-resolution screens.

pub mod art;
pub mod glyphs;

use gtk::gdk;
use gtk::prelude::*;

pub use art::ArtKind;
pub use glyphs::{Glyph, GlyphPaintable};

use crate::theme::Appearance;

/// A centred image of `glyph` at `size` logical pixels, painted in the CSS
/// colour of the image (so hover and disabled states apply).
pub fn glyph(glyph: Glyph, size: i32) -> gtk::Image {
    glyph_image(&GlyphPaintable::new(glyph, size, None), size)
}

/// A glyph in a fixed colour, for the coloured Quick access and sidebar
/// glyphs.
pub fn colored_glyph(glyph: Glyph, size: i32, color: gdk::RGBA) -> gtk::Image {
    glyph_image(&GlyphPaintable::new(glyph, size, Some(color)), size)
}

fn glyph_image(paintable: &GlyphPaintable, size: i32) -> gtk::Image {
    let image = gtk::Image::from_paintable(Some(paintable));
    image.set_pixel_size(size);
    image.set_halign(gtk::Align::Center);
    image.set_valign(gtk::Align::Center);
    image.add_css_class("glyph");
    image
}

/// Replaces an image's content with `glyph`.
pub fn set_glyph(image: &gtk::Image, glyph: Glyph, size: i32) {
    image.set_paintable(Some(&GlyphPaintable::new(glyph, size, None)));
    image.set_pixel_size(size);
}

/// An image showing colour art at `size` logical pixels. It is centred in
/// its allocation, because a `GtkImage` stretches its picture to fill a
/// larger one, which blurs the art.
pub fn art_image(kind: ArtKind, size: i32, appearance: Appearance, scale: i32) -> gtk::Image {
    let image = gtk::Image::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    set_art(&image, kind, size, appearance, scale);
    image
}

/// Replaces an image's content with colour art, rasterised for `scale`.
pub fn set_art(image: &gtk::Image, kind: ArtKind, size: i32, appearance: Appearance, scale: i32) {
    let pixels = size * scale.max(1);
    match art::texture(kind, appearance, pixels) {
        Some(texture) => image.set_paintable(Some(&texture)),
        None => image.set_paintable(None::<&gdk::Paintable>),
    }
    image.set_pixel_size(size);
}
