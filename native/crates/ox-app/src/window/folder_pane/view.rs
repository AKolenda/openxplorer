// SPDX-License-Identifier: AGPL-3.0-only
//! Which view lists a folder's items: the details view, the compact list,
//! or icons of one size.
//!
//! Ports `state.view` of `v2.0.0:desktop/ui/app.js` (`details` or `grid`) and
//! the Python app's saved `view` preference, which the native app extends
//! with Explorer's List layout (Dolphin's Compact view) and the icon zoom
//! levels from 16 to 256 pixels.

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
}

/// The `mode` a saved view style keeps for each view.
const DETAILS_MODE: &str = "details";
const COMPACT_MODE: &str = "compact";
const ICONS_MODE: &str = "icons";

impl FolderView {
    /// The `win.view` action state: `details`, `compact` or an icon size
    /// key.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            FolderView::Details => DETAILS_MODE,
            FolderView::Compact => COMPACT_MODE,
            FolderView::Icons(size) => size.as_str(),
        }
    }

    /// The view for a `win.view` action state.
    pub(crate) fn from_key(key: &str) -> Option<FolderView> {
        match key {
            DETAILS_MODE => Some(FolderView::Details),
            COMPACT_MODE => Some(FolderView::Compact),
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
    /// the Python app, so every other view is saved as the closer of them.
    pub(crate) fn setting(self) -> View {
        match self {
            FolderView::Details | FolderView::Compact => View::Details,
            FolderView::Icons(_) => View::Grid,
        }
    }

    /// The view a saved style's `mode` and `icon_size` describe; Details
    /// for a mode this build does not know.
    pub(crate) fn from_style(mode: &str, icon_size: u32) -> FolderView {
        match mode {
            COMPACT_MODE => FolderView::Compact,
            ICONS_MODE => FolderView::Icons(IconSize::nearest(icon_size)),
            _ => FolderView::Details,
        }
    }

    /// The `mode` a saved style keeps for this view.
    pub(crate) const fn style_mode(self) -> &'static str {
        match self {
            FolderView::Details => DETAILS_MODE,
            FolderView::Compact => COMPACT_MODE,
            FolderView::Icons(_) => ICONS_MODE,
        }
    }

    /// The icon view's layout for this view; `None` for Details.
    pub(crate) const fn grid_layout(self) -> Option<GridLayout> {
        match self {
            FolderView::Details => None,
            FolderView::Compact => Some(GridLayout::Compact),
            FolderView::Icons(size) => Some(GridLayout::Icons(size)),
        }
    }

    /// The view `steps` wheel notches bigger (smaller when negative) than
    /// this one, as Ctrl+wheel zooms Explorer's layouts: Details, then the
    /// compact list, then every icon size from 16 to 256 pixels. It stops
    /// at either end.
    pub(crate) fn zoomed(self, steps: i32) -> FolderView {
        let order: Vec<FolderView> = [FolderView::Details, FolderView::Compact]
            .into_iter()
            .chain(IconSize::levels().map(FolderView::Icons))
            .collect();
        let at = order.iter().position(|view| *view == self).unwrap_or_default();
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
            FolderView::Icons(_) => return None,
        };
        Some(shortcut)
    }

    /// The views with a shortcut of their own, as the Layout menu lists
    /// them.
    pub(crate) const NAMED: [FolderView; 6] = [
        FolderView::Icons(IconSize::EXTRA_LARGE),
        FolderView::Icons(IconSize::LARGE),
        FolderView::Icons(IconSize::MEDIUM),
        FolderView::Icons(IconSize::SMALL),
        FolderView::Compact,
        FolderView::Details,
    ];

    /// The name of the view's page in the pane's view stack; the compact
    /// list and every icon size share one icon view.
    pub(super) const fn stack_name(self) -> &'static str {
        match self {
            FolderView::Details => "details",
            FolderView::Compact | FolderView::Icons(_) => "grid",
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
        assert_eq!(FolderView::Compact.zoomed(1), FolderView::Icons(IconSize::at_index(0)));
        assert_eq!(large.zoomed(-2), FolderView::Icons(IconSize::MEDIUM));
        assert_eq!(large.zoomed(50), FolderView::Icons(IconSize::LARGEST));
        assert_eq!(large.zoomed(-90), FolderView::Details);
        assert_eq!(large.zoomed(0), large);
    }

    #[test]
    fn saved_styles_name_the_view_they_were_saved_from() {
        for view in FolderView::NAMED {
            assert_eq!(FolderView::from_style(view.style_mode(), 96), match view {
                FolderView::Icons(_) => FolderView::Icons(IconSize::EXTRA_LARGE),
                other => other,
            });
            assert_eq!(FolderView::from_key(view.as_str()), Some(view));
        }
    }
}
