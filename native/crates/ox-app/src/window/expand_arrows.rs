// SPDX-License-Identifier: AGPL-3.0-only
//! Settings > Appearance > Layout > "Hide expand arrows" (SIDE-032), as
//! Windows Explorer draws no expand arrows in its file list: no arrow is
//! drawn beside This PC and Network in the sidebar, in the folder tree or
//! beside folders in the file list. Off by default.
//!
//! On, the window carries the [`HIDE_CLASS`] class and
//! `resources/skin/sidebar.css` draws the three kinds of arrow fully
//! transparent, keeping their room so nothing moves. A class on the window
//! reaches every arrow at once, whichever rows are shown or built later,
//! and nothing is reloaded. Folders still open and close in place with
//! Right and Left. This PC and Network do not fold, so their chevrons
//! were only drawn.

use gtk::prelude::*;

use super::BrowserWindow;

/// The window's class while the expand arrows are hidden.
pub(crate) const HIDE_CLASS: &str = "hide-expand-arrows";

impl BrowserWindow {
    /// Hides or shows the expand arrows as the preferences say, at start
    /// and whenever Settings or another window changes them.
    pub(super) fn follow_expand_arrows_preference(&self) {
        let hidden = self.context().settings_data().preferences.hide_expand_arrows;
        if hidden {
            self.add_css_class(HIDE_CLASS);
        } else {
            self.remove_css_class(HIDE_CLASS);
        }
    }

    /// Whether the expand arrows are hidden, for tests.
    #[cfg(test)]
    pub(crate) fn hides_expand_arrows(&self) -> bool {
        self.has_css_class(HIDE_CLASS)
    }
}
