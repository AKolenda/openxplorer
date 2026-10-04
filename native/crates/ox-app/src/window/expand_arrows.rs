// SPDX-License-Identifier: AGPL-3.0-only
//! Settings > Appearance > Layout > "Hide expand arrows" (SIDE-032), as
//! Windows Explorer draws the navigation pane: the chevrons of This PC and
//! Network and the folder tree's arrows show only while the pointer is
//! over the pane or keyboard focus is in it. Off by default. The file
//! list's folder arrows are the "Expandable folders" switch's.
//!
//! On, the window carries the [`HIDE_CLASS`] class and
//! `resources/skin/sidebar.css` draws those arrows transparent, keeping
//! their room so nothing moves, and fades them in. A class on the window
//! reaches every arrow at once, whichever rows are shown or built later,
//! and nothing is reloaded.

use super::widget_tree::toggle_class;
use super::BrowserWindow;

/// The window's class while the navigation pane's arrows are hidden.
const HIDE_CLASS: &str = "hide-expand-arrows";

impl BrowserWindow {
    /// Hides or shows the navigation pane's arrows as the preferences say,
    /// at start and whenever Settings or another window changes them.
    pub(super) fn follow_expand_arrows_preference(&self) {
        let hidden = self.context().settings_data().preferences.hide_expand_arrows;
        toggle_class(self, HIDE_CLASS, hidden);
    }

    /// Whether the navigation pane's arrows are hidden, for tests.
    #[cfg(test)]
    pub(crate) fn hides_expand_arrows(&self) -> bool {
        use gtk::prelude::*;

        self.has_css_class(HIDE_CLASS)
    }
}
