// SPDX-License-Identifier: AGPL-3.0-only
//! Which view lists a folder's items: the details view, the compact list,
//! icons of one size, or Windows 11's Tiles and Content.
//!
//! Ports `state.view` of `v2.0.0:desktop/ui/app.js` (`details` or `grid`) and
//! the Python app's saved `view` preference, which the native app extends
//! with Explorer's List layout (Dolphin's Compact view), the icon zoom
//! levels from 16 to 256 pixels, and Explorer's Tiles and Content layouts
//! (VIEW-071).

use ox_core::settings::View;

use crate::folder_view::grid::{GridLayout, IconSize};

/// Which view lists the items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FolderView {
    /// Rows with Name, Date modified, Type and Size columns.
    Details,
    /// Small icons with names beside them, in columns (VIEW-008).
    Compact,
    /// Icon tiles of one size.
    Icons(IconSize),
    /// Explorer's Tiles: a 48-pixel icon with its name, type and size
    /// beside it, in columns that fill the width (VIEW-071).
    Tiles,
    /// Explorer's Content: one item per row, its name and type beside its
    /// icon and its date modified and size at the right (VIEW-071).
    Content,
}

/// The `mode` a saved view style keeps for each view.
const DETAILS_MODE: &str = "details";
const COMPACT_MODE: &str = "compact";
const ICONS_MODE: &str = "icons";
const TILES_MODE: &str = "tiles";
const CONTENT_MODE: &str = "content";

impl FolderView {
    /// The `win.view` action state: `details`, `compact`, `tiles`,
    /// `content` or an icon size key.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            FolderView::Details => DETAILS_MODE,
            FolderView::Compact => COMPACT_MODE,
            FolderView::Icons(size) => size.as_str(),
            FolderView::Tiles => TILES_MODE,
            FolderView::Content => CONTENT_MODE,
        }
    }

    /// The view for a `win.view` action state.
    pub(crate) fn from_key(key: &str) -> Option<FolderView> {
        match key {
            DETAILS_MODE => Some(FolderView::Details),
            COMPACT_MODE => Some(FolderView::Compact),
            TILES_MODE => Some(FolderView::Tiles),
            CONTENT_MODE => Some(FolderView::Content),
            _ => IconSize::from_key(key).map(FolderView::Icons),
        }
    }

    /// The view saved in settings. The Python app knows one icon view,
    /// "grid", which is Large icons.
    pub(crate) fn from_setting(view: View) -> FolderView {
        match view {
            View::Details => FolderView::Details,
            View::Grid => FolderView::Icons(IconSize::LARGE),
        }
    }

    /// The value settings store: only `details` and `grid` are valid for
    /// the Python app, so every other view is saved as the closer of them:
    /// the tiles of Tiles as its grid, Content's rows as its details.
    pub(crate) fn setting(self) -> View {
        match self {
            FolderView::Details | FolderView::Compact | FolderView::Content => View::Details,
            FolderView::Icons(_) | FolderView::Tiles => View::Grid,
        }
    }

    /// The view a saved style's `mode` and `icon_size` describe; Details
    /// for a mode this build does not know, as an earlier build shows a
    /// folder saved in Tiles or Content.
    pub(crate) fn from_style(mode: &str, icon_size: u32) -> FolderView {
        match mode {
            COMPACT_MODE => FolderView::Compact,
            ICONS_MODE => FolderView::Icons(IconSize::nearest(icon_size)),
            TILES_MODE => FolderView::Tiles,
            CONTENT_MODE => FolderView::Content,
            _ => FolderView::Details,
        }
    }

    /// The `mode` a saved style keeps for this view.
    pub(crate) const fn style_mode(self) -> &'static str {
        match self {
            FolderView::Details => DETAILS_MODE,
            FolderView::Compact => COMPACT_MODE,
            FolderView::Icons(_) => ICONS_MODE,
            FolderView::Tiles => TILES_MODE,
            FolderView::Content => CONTENT_MODE,
        }
    }

    /// The icon view's layout for this view; `None` for Details.
    pub(crate) const fn grid_layout(self) -> Option<GridLayout> {
        match self {
            FolderView::Details => None,
            FolderView::Compact => Some(GridLayout::Compact),
            FolderView::Icons(size) => Some(GridLayout::Icons(size)),
            FolderView::Tiles => Some(GridLayout::Tiles),
            FolderView::Content => Some(GridLayout::Content),
        }
    }

    /// The view the icon view's `layout` shows.
    pub(crate) const fn of_grid_layout(layout: GridLayout) -> FolderView {
        match layout {
            GridLayout::Compact => FolderView::Compact,
            GridLayout::Icons(size) => FolderView::Icons(size),
            GridLayout::Tiles => FolderView::Tiles,
            GridLayout::Content => FolderView::Content,
        }
    }

    /// The view `steps` wheel notches bigger (smaller when negative) than
    /// this one, as Ctrl+wheel zooms Explorer's layouts: Details, then the
    /// compact list, then every icon size from 16 to 256 pixels. It stops
    /// at either end. Tiles and Content are not on the wheel, as in
    /// Windows: the wheel never lands on them, and from them it steps as
    /// from Details (VIEW-071).
    pub(crate) fn zoomed(self, steps: i32) -> FolderView {
        let order: Vec<FolderView> = [FolderView::Details, FolderView::Compact]
            .into_iter()
            .chain(IconSize::levels().map(FolderView::Icons))
            .collect();
        let at = order.iter().position(|view| *view == self).unwrap_or(0);
        let last = order.len() - 1;
        let moved = if steps < 0 {
            at.saturating_sub(steps.unsigned_abs() as usize)
        } else {
            at.saturating_add(steps.unsigned_abs() as usize).min(last)
        };
        order[moved]
    }

    /// Explorer's shortcut for the view: the accelerator GTK installs and
    /// its text as menus show it, kept side by side so they cannot differ.
    /// Only Explorer's named layouts have one.
    pub(crate) fn shortcut(self) -> Option<(&'static str, &'static str)> {
        let shortcut = match self {
            FolderView::Icons(IconSize::EXTRA_LARGE) => ("<Primary><Shift>1", "Ctrl+Shift+1"),
            FolderView::Icons(IconSize::LARGE) => ("<Primary><Shift>2", "Ctrl+Shift+2"),
            FolderView::Icons(IconSize::MEDIUM) => ("<Primary><Shift>3", "Ctrl+Shift+3"),
            FolderView::Icons(IconSize::SMALL) => ("<Primary><Shift>4", "Ctrl+Shift+4"),
            FolderView::Compact => ("<Primary><Shift>5", "Ctrl+Shift+5"),
            FolderView::Details => ("<Primary><Shift>6", "Ctrl+Shift+6"),
            FolderView::Tiles => ("<Primary><Shift>7", "Ctrl+Shift+7"),
            FolderView::Content => ("<Primary><Shift>8", "Ctrl+Shift+8"),
            FolderView::Icons(_) => return None,
        };
        Some(shortcut)
    }

    /// The views with a shortcut of their own, as the Layout menu lists
    /// them.
    pub(crate) const NAMED: [FolderView; 8] = [
        FolderView::Icons(IconSize::EXTRA_LARGE),
        FolderView::Icons(IconSize::LARGE),
        FolderView::Icons(IconSize::MEDIUM),
        FolderView::Icons(IconSize::SMALL),
        FolderView::Compact,
        FolderView::Details,
        FolderView::Tiles,
        FolderView::Content,
    ];

    /// The name of the view's page in the pane's view stack; the compact
    /// list, every icon size, Tiles and Content share one icon view.
    pub(super) const fn stack_name(self) -> &'static str {
        match self {
            FolderView::Details => "details",
            _ => "grid",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ctrl+wheel steps through the layouts one notch at a time and stops
    /// at Details and at the largest icons.
    ///
    /// parity: VIEW-011
    #[test]
    fn the_wheel_steps_through_the_views_and_stops_at_the_ends() {
        let large = FolderView::Icons(IconSize::LARGE);
        assert_eq!(FolderView::Details.zoomed(1), FolderView::Compact);
        assert_eq!(
            FolderView::Compact.zoomed(1),
            FolderView::Icons(IconSize::at_index(0))
        );
        assert_eq!(large.zoomed(-4), FolderView::Icons(IconSize::MEDIUM));
        assert_eq!(large.zoomed(50), FolderView::Icons(IconSize::LARGEST));
        assert_eq!(large.zoomed(-90), FolderView::Details);
        assert_eq!(large.zoomed(0), large);
    }

    /// Tiles and Content are not on the wheel, as in Windows: no notch
    /// lands on them, and from them the wheel steps as from Details. Their
    /// keys are Windows' Ctrl+Shift+7 and 8.
    ///
    /// parity: VIEW-071
    #[test]
    fn tiles_and_content_are_off_the_wheel_and_have_windows_keys() {
        for view in [FolderView::Tiles, FolderView::Content] {
            assert_eq!(view.zoomed(1), FolderView::Compact);
            assert_eq!(view.zoomed(-1), FolderView::Details);
        }
        let mut on_the_wheel = (-20..=20).map(|steps| FolderView::Details.zoomed(steps));
        assert!(on_the_wheel.all(|view| !matches!(view, FolderView::Tiles | FolderView::Content)));
        assert_eq!(
            FolderView::Tiles.shortcut().map(|keys| keys.1),
            Some("Ctrl+Shift+7")
        );
        assert_eq!(
            FolderView::Content.shortcut().map(|keys| keys.1),
            Some("Ctrl+Shift+8")
        );
        assert_eq!(FolderView::from_style("tiles", 96), FolderView::Tiles);
        assert_eq!(FolderView::from_style("content", 96), FolderView::Content);
        assert_eq!(FolderView::Tiles.setting(), View::Grid);
        assert_eq!(FolderView::Content.setting(), View::Details);
    }

    #[test]
    fn saved_styles_name_the_view_they_were_saved_from() {
        for view in FolderView::NAMED {
            assert_eq!(
                FolderView::from_style(view.style_mode(), 96),
                match view {
                    FolderView::Icons(_) => FolderView::Icons(IconSize::LARGE),
                    other => other,
                }
            );
            assert_eq!(FolderView::from_key(view.as_str()), Some(view));
        }
    }
}
