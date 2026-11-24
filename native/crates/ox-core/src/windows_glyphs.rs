// SPDX-License-Identifier: AGPL-3.0-only
//! Glyphs read at run time from the Windows symbol font the user chooses
//! (LOOK-031).
//!
//! Windows 11 File Explorer draws its command bar, menus and navigation
//! buttons with the Segoe Fluent Icons font (`SegoeIcons.ttf`; Segoe MDL2
//! Assets, `segmdl2.ttf`, before it). `OpenXplorer` ships no Microsoft font.
//! When the user turns on Windows icon files and chooses a font they
//! already have, this module reads the outlines of the glyphs the app
//! uses ([`truetype`]) and writes each as a symbolic SVG icon into the
//! user's cache folder ([`cache`]), from where GTK draws it in the
//! widget's colour, as it draws the app's own glyphs. The font is read,
//! never installed: nothing reaches the desktop's font configuration.
//!
//! The font is untrusted input: every offset is checked against its
//! table and every count is capped, so a damaged font gives no glyph and
//! the app keeps its own.

mod cache;
pub mod synthetic;
mod truetype;

#[cfg(test)]
mod tests;

pub use cache::{glyph_cache_folder, is_glyph_icon_name, load_glyphs, GlyphRequest, LoadedGlyphs};
pub use truetype::{Font, FontError};
