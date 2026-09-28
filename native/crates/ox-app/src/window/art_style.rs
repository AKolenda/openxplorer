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

pub(super) use crate::icons::ArtStyle;

use super::BrowserWindow;

impl BrowserWindow {
    /// The style the window's art is drawn in now.
    pub(super) fn art_style(&self) -> ArtStyle {
        ArtStyle {
            appearance: self.skin().appearance(),
            scale: self.scale_factor(),
        }
    }
}
