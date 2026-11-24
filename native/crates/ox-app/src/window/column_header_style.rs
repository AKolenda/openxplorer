// SPDX-License-Identifier: AGPL-3.0-only
//! Settings > Appearance > Layout > "Column header style" (VIEW-014): how
//! the Details view draws its sorted column's title. The app's own style,
//! the default, is the owner's: a chevron after the bright title, in the
//! accent with a line under it for any sort but Name ascending. Windows'
//! draws Windows 11 File Explorer's: only a small gray chevron centered
//! over the column, above its title, which keeps its color.
//!
//! With Windows' style the window carries [`WINDOWS_CLASS`] and
//! `resources/skin/folder-views.css` draws it; a class on the window
//! reaches every pane's titles at once, and nothing is rebuilt.

use ox_core::settings::ColumnHeaderStyle;

use super::widget_tree::toggle_class;
use super::BrowserWindow;

/// The window's class while its column titles follow Windows' style.
const WINDOWS_CLASS: &str = "windows-column-headers";

impl BrowserWindow {
    /// Draws the column titles in the style the preferences say, at start
    /// and whenever Settings or another window changes it.
    pub(super) fn follow_column_header_style_preference(&self) {
        let style = self.context().settings_data().preferences.column_header_style;
        toggle_class(self, WINDOWS_CLASS, style == ColumnHeaderStyle::Windows);
    }

    /// The style the window draws its column titles in, for tests.
    #[cfg(test)]
    pub(crate) fn column_header_style(&self) -> ColumnHeaderStyle {
        use gtk::prelude::*;

        if self.has_css_class(WINDOWS_CLASS) {
            ColumnHeaderStyle::Windows
        } else {
            ColumnHeaderStyle::OpenXplorer
        }
    }
}
