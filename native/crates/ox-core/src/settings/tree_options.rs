// SPDX-License-Identifier: AGPL-3.0-only
//! The folder tree's options (SIDE-028), as Dolphin's Folders panel keeps
//! them in `dolphin_folderspanelsettings.kcfg`: whether the tree is shown,
//! whether it lists hidden folders, whether it stays inside the home
//! folder, and whether it scrolls to the folder shown.

use serde::Serialize;
use serde_json::Value;

/// How the folder tree behaves. Stored as `folderTree`, which the Python
/// app does not read.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is a separate saved on/off option, as in Dolphin's panel settings"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderTreeOptions {
    /// The tree is shown under the places (Dolphin's F7).
    pub shown: bool,
    /// Hidden folders are listed (Dolphin's `HiddenFilesShown`).
    pub show_hidden: bool,
    /// Inside the home folder, the tree starts at it rather than at `/`
    /// (Dolphin's `LimitFoldersPanelToHome`).
    pub limit_to_home: bool,
    /// The tree scrolls to the folder shown (Dolphin's `AutoScrolling`).
    pub auto_scroll: bool,
}

impl Default for FolderTreeOptions {
    fn default() -> Self {
        Self {
            shown: false,
            show_hidden: false,
            limit_to_home: true,
            auto_scroll: true,
        }
    }
}

impl FolderTreeOptions {
    /// Whether these are the options of a new installation.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The options stored in `value`; a missing or mistyped option keeps
    /// its default, and `None` when `value` is not an object.
    pub fn from_json(value: &Value) -> Option<Self> {
        let values = value.as_object()?;
        let defaults = Self::default();
        let flag = |key: &str, default: bool| values.get(key).and_then(Value::as_bool).unwrap_or(default);
        Some(Self {
            shown: flag("shown", defaults.shown),
            show_hidden: flag("showHidden", defaults.show_hidden),
            limit_to_home: flag("limitToHome", defaults.limit_to_home),
            auto_scroll: flag("autoScroll", defaults.auto_scroll),
        })
    }
}
