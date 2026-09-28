// SPDX-License-Identifier: AGPL-3.0-only
//! How the window follows the skin: light or dark, the text size and the
//! screen's scale factor.
//!
//! Ports `applyTheme` and the `textSizeChanged` handling in
//! `desktop/ui/app.js`. The web app draws the folder and drive art of
//! `fileIcon` and `folderIcon` as SVG, which the browser scales for the
//! screen; GTK images are pixel textures, so every part of the frame that
//! shows art (tabs, the address bar, the sidebar, the landing pages and the
//! details pane) needs both the appearance and the scale factor, which
//! [`ArtStyle`] carries together, and is redrawn when either changes.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::icons::{self, ArtKind};
use crate::theme::{Appearance, SkinChange};

use super::window_action::WindowAction;
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

    /// Applies the skin's current appearance and text size, and follows
    /// them and the screen's scale factor from now on.
    pub(super) fn follow_skin(&self) {
        let listener = self.skin().connect_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |change| window.skin_changed(change)
        ));
        self.imp().handlers.borrow_mut().skin = Some(listener);
        self.show_appearance_choice();
        self.content().set_text_size(self.skin().text_size());
        self.connect_scale_factor_notify(|window| {
            window.content().icons.redraw();
            window.render_places();
            window.update_details_pane();
        });
    }

    fn skin_changed(&self, change: SkinChange) {
        match change {
            SkinChange::Appearance(appearance) => self.appearance_changed(appearance),
            SkinChange::TextSize(percent) => self.content().set_text_size(percent),
        }
    }

    fn appearance_changed(&self, appearance: Appearance) {
        self.content().icons.set_appearance(appearance);
        self.render_places();
        // Redraws the tabs' and the address bar's colour art too.
        self.render_location();
        self.update_details_pane();
        self.show_appearance_choice();
    }

    /// Shows the chosen and drawn appearance on the Appearance button and
    /// in the Appearance menu.
    fn show_appearance_choice(&self) {
        let preference = self.skin().preference();
        let appearance = self.skin().appearance();
        self.chrome()
            .commands
            .show_appearance(appearance, &preference.tooltip(appearance));
        self.set_action_state(WindowAction::Theme, &preference.key().to_variant());
    }
}
