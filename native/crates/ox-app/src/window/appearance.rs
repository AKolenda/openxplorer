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

pub(super) use crate::icons::ArtStyle;
use crate::theme::Appearance;

use super::window_action::WindowAction;
use super::BrowserWindow;

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
        let appearance_handler = self.skin().connect_appearance_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.appearance_changed(window.skin().appearance())
        ));
        let text_size_handler = self.skin().connect_text_size_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.folder_pane().set_text_size(window.skin().text_size())
        ));
        self.imp().handlers.borrow_mut().skin = vec![appearance_handler, text_size_handler];
        let appearance = self.skin().appearance();
        // The template builds the panes light; nothing is drawn in them yet.
        self.folder_pane().icons().set_appearance(appearance);
        self.details_pane().show_placeholder(appearance);
        self.show_appearance_choice();
        self.folder_pane().set_text_size(self.skin().text_size());
        self.connect_scale_factor_notify(|window| {
            window.folder_pane().icons().redraw();
            window.render_places();
            window.update_details_pane();
        });
    }

    fn appearance_changed(&self, appearance: Appearance) {
        self.folder_pane().icons().set_appearance(appearance);
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
        self.command_bar()
            .show_appearance(appearance, &preference.tooltip(appearance));
        self.set_action_state(WindowAction::Theme, &preference.key().to_variant());
    }
}
