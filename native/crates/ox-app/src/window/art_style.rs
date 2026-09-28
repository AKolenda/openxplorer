// SPDX-License-Identifier: AGPL-3.0-only
//! How the chrome draws its colour art: light or dark, at the screen's scale.
//!
//! The web app draws the folder and drive art of `fileIcon` and `folderIcon`
//! in `desktop/ui/app.js` as SVG, which the browser scales for the screen.
//! GTK images are pixel textures, so every part of the frame that shows art
//! (tabs, the address bar, the sidebar, the landing pages and the details
//! pane) needs both the appearance and the scale factor; [`ArtStyle`] carries
//! the two together.

use gtk::prelude::*;

use crate::icons::{self, ArtKind};
use crate::theme::Appearance;

use super::BrowserWindow;

/// The appearance and scale factor colour art is drawn for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ArtStyle {
    /// Light or dark art.
    pub appearance: Appearance,
    /// The screen's scale factor.
    pub scale: i32,
}

impl ArtStyle {
    /// A new image of `kind` art, `size` pixels square.
    pub fn image(self, kind: ArtKind, size: i32) -> gtk::Image {
        icons::art_image(kind, size, self.appearance, self.scale)
    }

    /// Makes `image` show `kind` art, `size` pixels square.
    pub fn draw_into(self, image: &gtk::Image, kind: ArtKind, size: i32) {
        icons::set_art(image, kind, size, self.appearance, self.scale);
    }
}

impl BrowserWindow {
    /// The style the window's art is drawn in now.
    pub(super) fn art_style(&self) -> ArtStyle {
        ArtStyle {
            appearance: self.skin().appearance(),
            scale: self.scale_factor(),
        }
    }
}
