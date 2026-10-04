// SPDX-License-Identifier: AGPL-3.0-only
//! Settings > Appearance > Layout > "Hide expand arrows" (SIDE-032), as
//! Windows Explorer draws them: the chevrons of This PC and Network and
//! the folder tree's arrows show only while the pointer is over the
//! sidebar, and the file list draws no folder arrows. Off by default.
//!
//! On, the window carries the [`HIDE_CLASS`] class and
//! `resources/skin/sidebar.css` draws those arrows transparent, keeping
//! their room so nothing moves, and fades the sidebar's in on hover. A
//! class on the window reaches every arrow at once, whichever rows are
//! shown or built later, and nothing is reloaded. Folders still open and
//! close in place with Right and Left.

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
