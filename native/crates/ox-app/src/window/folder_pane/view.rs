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

    /// The name of the view's page in the pane's view stack; every icon
    /// size shares one icon view.
    pub(super) const fn stack_name(self) -> &'static str {
        match self {
            FolderView::Details => "details",
            FolderView::Icons(_) => "grid",
        }
    }
}
