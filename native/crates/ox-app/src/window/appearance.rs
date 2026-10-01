// SPDX-License-Identifier: AGPL-3.0-only
//! How the window follows the skin: light or dark, and the text size.
//!
//! Ports `applyTheme` and the `textSizeChanged` handling in
//! `v2.0.0:desktop/ui/app.js`. Nothing is redrawn for a new appearance or screen
//! scale: the icons are bundled SVG files shown by name, which GTK renders
//! again at the new scale by itself, and the glyphs take the CSS colour of
//! the new palette, as the web app's `currentColor` glyphs do.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::window_action::WindowAction;
use super::BrowserWindow;
use crate::theme;

impl BrowserWindow {
    /// Applies the skin's current appearance and text size, and follows
    /// them from now on.
    pub(super) fn follow_skin(&self) {
        let appearance_handler = self.skin().connect_appearance_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.show_appearance_choice()
        ));
        let text_size_handler = self.skin().connect_text_size_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.show_text_size()
        ));
        self.imp().handlers.borrow_mut().skin = vec![appearance_handler, text_size_handler];
        self.show_appearance_choice();
        self.show_text_size();
    }

    /// Draws both folder panes' items at the skin's text size.
    fn show_text_size(&self) {
        let size = self.skin().drawn_text_size();
        for pane in self.folder_panes() {
            pane.set_text_size(size);
        }
    }

    /// Shows the chosen and drawn appearance on the Appearance button and
    /// in the Appearance menu.
    fn show_appearance_choice(&self) {
        let theme = self.skin().theme();
        let appearance = self.skin().appearance();
        self.command_bar()
            .show_appearance(appearance, &theme::tooltip(theme, appearance));
        self.set_action_state(WindowAction::Theme, &theme.as_str().to_variant());
    }
}
