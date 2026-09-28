// SPDX-License-Identifier: AGPL-3.0-only
//! Icons: stroke glyphs drawn natively and colour art drawn from SVG.
//!
//! Ports the icon helpers of `desktop/ui/app.js` (`icon()` and the art
//! functions). Glyphs ([`glyph`]) follow the widget's CSS colour; art
//! ([`ArtStyle::image`], [`ArtStyle::draw_into`]) is rasterised at the
//! window's scale factor so it stays sharp on high-resolution screens.

pub(crate) mod art;
mod glyphs;

use gtk::gdk;
use gtk::prelude::*;

pub(crate) use art::ArtKind;
pub(crate) use glyphs::Glyph;

use crate::theme::Appearance;
use glyphs::GlyphPaintable;

/// A centred image of `glyph` at `size` logical pixels, painted in the CSS
/// colour of the image (so hover and disabled states apply).
pub(crate) fn glyph(glyph: Glyph, size: i32) -> gtk::Image {
    centred_glyph_image(&GlyphPaintable::new(glyph, size, None), size)
}

/// A glyph in a fixed colour, for the coloured Quick access and sidebar
/// glyphs.
pub(crate) fn colored_glyph(glyph: Glyph, size: i32, color: gdk::RGBA) -> gtk::Image {
    centred_glyph_image(&GlyphPaintable::new(glyph, size, Some(color)), size)
}

/// An image of `paintable`, centred in its allocation and styled as a
/// glyph (the `glyph` CSS class).
fn centred_glyph_image(paintable: &GlyphPaintable, size: i32) -> gtk::Image {
    let image = gtk::Image::from_paintable(Some(paintable));
    image.set_pixel_size(size);
    image.set_halign(gtk::Align::Center);
    image.set_valign(gtk::Align::Center);
    image.add_css_class("glyph");
    image
}

/// Replaces an image's content with `glyph`.
pub(crate) fn set_glyph(image: &gtk::Image, glyph: Glyph, size: i32) {
    image.set_paintable(Some(&GlyphPaintable::new(glyph, size, None)));
    image.set_pixel_size(size);
}

/// How colour art is drawn in one window: light or dark, at the window's
/// display scale. The web app's SVG art scales with the screen by itself;
/// GTK images are pixel textures, so every piece of art needs both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArtStyle {
    /// Light or dark art.
    pub appearance: Appearance,
    /// Device pixels per logical pixel: the window's scale factor.
    pub scale: i32,
}

impl ArtStyle {
    /// A new image of `kind` art at `size` logical pixels. It is centred in
    /// its allocation, because a `GtkImage` stretches its picture to fill a
    /// larger one, which blurs the art.
    pub(crate) fn image(self, kind: ArtKind, size: i32) -> gtk::Image {
        let image = gtk::Image::builder()
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        self.draw_into(&image, kind, size);
        image
    }

    /// Replaces `image`'s content with `kind` art at `size` logical
    /// pixels, rasterised for the display scale.
    pub(crate) fn draw_into(self, image: &gtk::Image, kind: ArtKind, size: i32) {
        let device_pixels = size * self.scale.max(1);
        let texture = art::texture(kind, self.appearance, device_pixels);
        image.set_paintable(texture.as_ref());
        image.set_pixel_size(size);
    }
}
