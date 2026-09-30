// SPDX-License-Identifier: AGPL-3.0-only
//! Which view lists a folder's items: the details view, or icons of one
//! size.
//!
//! Ports `state.view` of `desktop/ui/app.js` (`details` or `grid`) and
//! the Python app's saved `view` preference, which the native app extends
//! with the four icon sizes of Windows 11 File Explorer.

use ox_core::settings::View;

use crate::folder_view::grid::IconSize;

/// Which view lists the items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FolderView {
    /// Rows with Name, Date modified, Type and Size columns.
    Details,
    /// Icon tiles of one size.
    Icons(IconSize),
}

impl FolderView {
    /// The `win.view` action state: `details` or an icon size key.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            FolderView::Details => "details",
            FolderView::Icons(size) => size.as_str(),
        }
    }

    /// The view for a `win.view` action state.
    pub(crate) fn from_key(key: &str) -> Option<FolderView> {
        if key == "details" {
            return Some(FolderView::Details);
        }
        IconSize::from_key(key).map(FolderView::Icons)
    }

    /// The view saved in settings. The Python app knows one icon view,
    /// "grid", which is Large icons.
    pub(crate) fn from_setting(view: View) -> FolderView {
        match view {
            View::Details => FolderView::Details,
            View::Grid => FolderView::Icons(IconSize::Large),
        }
    }

    /// The value settings store: only `details` and `grid` are valid for
    /// the Python app, so every icon size is saved as `grid`.
    pub(crate) fn setting(self) -> View {
        match self {
            FolderView::Details => View::Details,
            FolderView::Icons(_) => View::Grid,
        }
    }

    /// The view `steps` wheel notches bigger (smaller when negative) than
    /// this one, as Ctrl+wheel steps Explorer's layouts: Details, then
    /// Small to Extra large icons. It stops at either end.
    pub(crate) fn zoomed(self, steps: i32) -> FolderView {
        let order = Self::ZOOM_ORDER;
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
    pub(crate) const fn shortcut(self) -> (&'static str, &'static str) {
        match self {
            FolderView::Icons(IconSize::ExtraLarge) => ("<Primary><Shift>1", "Ctrl+Shift+1"),
            FolderView::Icons(IconSize::Large) => ("<Primary><Shift>2", "Ctrl+Shift+2"),
            FolderView::Icons(IconSize::Medium) => ("<Primary><Shift>3", "Ctrl+Shift+3"),
            FolderView::Icons(IconSize::Small) => ("<Primary><Shift>4", "Ctrl+Shift+4"),
            FolderView::Details => ("<Primary><Shift>6", "Ctrl+Shift+6"),
        }
    }

    /// The views from smallest to largest items, for [`Self::zoomed`].
    pub(crate) const ZOOM_ORDER: [FolderView; 5] = [
        FolderView::Details,
        FolderView::Icons(IconSize::Small),
        FolderView::Icons(IconSize::Medium),
        FolderView::Icons(IconSize::Large),
        FolderView::Icons(IconSize::ExtraLarge),
    ];

    /// The name of the view's page in the pane's view stack; every icon
    /// size shares one icon view.
    pub(super) const fn stack_name(self) -> &'static str {
        match self {
            FolderView::Details => "details",
            FolderView::Icons(_) => "grid",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ctrl+wheel steps through the layouts one notch at a time and stops
    /// at Details and at Extra large icons.
    ///
    /// parity: VIEW-011
    #[test]
    fn the_wheel_steps_through_the_views_and_stops_at_the_ends() {
        let large = FolderView::Icons(IconSize::Large);
        assert_eq!(FolderView::Details.zoomed(1), FolderView::Icons(IconSize::Small));
        assert_eq!(large.zoomed(-1), FolderView::Icons(IconSize::Medium));
        assert_eq!(large.zoomed(5), FolderView::Icons(IconSize::ExtraLarge));
        assert_eq!(large.zoomed(-9), FolderView::Details);
        assert_eq!(large.zoomed(0), large);
    }
}
