// SPDX-License-Identifier: AGPL-3.0-only
//! How a folder is displayed, saved for all folders or for each folder.
//!
//! Ports Dolphin's `ViewProperties` (`src/views/viewproperties.cpp`) and its
//! `GlobalViewProps` setting: by default one style is shared by every folder
//! (`viewDefaults`); with "Remember display style for each folder" on, each
//! folder the user restyles keeps its own (`folderViews`), and a style saved
//! for a folder "and its sub-folders" applies below it too, as Dolphin's
//! `ApplyViewPropsJob` does. The Python app reads neither key.

use serde::Serialize;
use serde_json::Value;

use super::preferences::{read_column_widths, ColumnWidths};
use super::view_options::read_column_keys;

use crate::location::{parent_location, same_location};

/// The most folders whose own style is kept; the oldest go first.
pub const MAX_FOLDER_VIEWS: usize = 500;

/// The longest location a folder style is kept for.
const MAX_FOLDER_LOCATION: usize = 4096;

/// The longest sort key kept, such as `modified`.
const MAX_SORT_KEY: usize = 32;

/// The icon sizes a style may hold, in pixels.
const ICON_SIZES: std::ops::RangeInclusive<u32> = 16..=256;

/// How one folder is displayed: Dolphin's view properties.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is one of Dolphin's view properties"
)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewProperties {
    /// `details`, `compact` or `icons`.
    pub mode: String,
    /// The icon edge of the icon view, in pixels (16 to 256).
    pub icon_size: u32,
    /// The sort key, such as `name` or `modified`.
    pub sort: String,
    /// Sorted descending.
    pub descending: bool,
    /// Items shown in groups by the sort key.
    pub groups: bool,
    /// Folders listed before files.
    pub folders_first: bool,
    /// Hidden items shown.
    pub show_hidden: bool,
    /// Hidden items sort after visible ones within the folder/file groups.
    pub hidden_last: bool,
    /// Previews, inheriting the shared option for older saved styles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_previews: Option<bool>,
    /// Visible details columns after Name, in their display order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details_columns: Option<Vec<String>>,
    /// Saved widths, inheriting the shared layout until customized.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column_widths: Option<ColumnWidths>,
}

impl Default for ViewProperties {
    /// Details, by name ascending, folders first: a new installation.
    fn default() -> Self {
        Self {
            mode: "details".to_owned(),
            icon_size: 56,
            sort: "name".to_owned(),
            descending: false,
            groups: false,
            folders_first: true,
            show_hidden: false,
            hidden_last: false,
            show_previews: None,
            details_columns: None,
            column_widths: None,
        }
    }
}

impl ViewProperties {
    /// The style stored in `value`: a missing or mistyped property keeps
    /// its default; `None` when `value` is not an object.
    pub fn from_json(value: &Value) -> Option<Self> {
        let values = value.as_object()?;
        let defaults = Self::default();
        let flag = |key: &str, default: bool| values.get(key).and_then(Value::as_bool).unwrap_or(default);
        let key = |name: &str| {
            values
                .get(name)
                .and_then(Value::as_str)
                .filter(|key| is_short_key(key))
                .map(str::to_owned)
        };
        let icon_size = values
            .get("iconSize")
            .and_then(Value::as_u64)
            .and_then(|size| u32::try_from(size).ok())
            .filter(|size| ICON_SIZES.contains(size));
        Some(Self {
            mode: key("mode").unwrap_or(defaults.mode),
            icon_size: icon_size.unwrap_or(defaults.icon_size),
            sort: key("sort").unwrap_or(defaults.sort),
            descending: flag("descending", defaults.descending),
            groups: flag("groups", defaults.groups),
            folders_first: flag("foldersFirst", defaults.folders_first),
            show_hidden: flag("showHidden", defaults.show_hidden),
            hidden_last: flag("hiddenLast", defaults.hidden_last),
            show_previews: values.get("showPreviews").and_then(Value::as_bool),
            details_columns: values.get("detailsColumns").and_then(read_column_keys),
            column_widths: values
                .get("columnWidths")
                .and_then(read_column_widths)
                .map(|widths| ColumnWidths::from_values(&widths)),
        })
    }
}

/// A key the app gives a mode or a sort order: short ASCII letters.
fn is_short_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= MAX_SORT_KEY && key.chars().all(|c| c.is_ascii_alphanumeric())
}

/// One folder's own style.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderView {
    /// The folder.
    pub uri: String,
    /// Its style.
    #[serde(flatten)]
    pub properties: ViewProperties,
    /// The style applies to the folder's sub-folders too, unless they have
    /// their own.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub subfolders: bool,
}

impl FolderView {
    /// The folder style stored in `value`, if it names a folder that may
    /// be kept.
    fn from_json(value: &Value) -> Option<Self> {
        let uri = value.get("uri")?.as_str()?;
        if !may_remember(uri) {
            return None;
        }
        Some(Self {
            uri: uri.to_owned(),
            properties: ViewProperties::from_json(value)?,
            subfolders: value.get("subfolders").and_then(Value::as_bool).unwrap_or(false),
        })
    }
}

/// Whether a style may be kept for `uri`: a bounded location without
/// control characters or a user name, so no account ends up in the file.
pub fn may_remember(uri: &str) -> bool {
    let authority = uri
        .split_once("://")
        .map_or("", |(_, rest)| rest.split('/').next().unwrap_or(""));
    !uri.is_empty()
        && uri.len() <= MAX_FOLDER_LOCATION
        && !uri.contains(char::is_control)
        && !authority.contains('@')
}

/// The folder styles stored in `value`, at most [`MAX_FOLDER_VIEWS`] of the
/// newest; `None` when `value` is not a list.
pub(super) fn read_folder_views(value: &Value) -> Option<Vec<FolderView>> {
    let stored: Vec<FolderView> = value
        .as_array()?
        .iter()
        .filter_map(FolderView::from_json)
        .collect();
    let skip = stored.len().saturating_sub(MAX_FOLDER_VIEWS);
    Some(stored.into_iter().skip(skip).collect())
}

/// How far a saved style reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewScope {
    /// The folder alone.
    Folder,
    /// The folder and its sub-folders: their own styles are dropped.
    FolderAndSubfolders,
    /// Every folder: the style becomes the default and every folder's own
    /// style is dropped.
    AllFolders,
}

/// The style `uri` is shown in: its own, else that of the nearest folder
/// above it saved for its sub-folders, else `defaults`.
pub fn style_for(folder_views: &[FolderView], defaults: &ViewProperties, uri: &str) -> ViewProperties {
    if let Some(own) = folder_views.iter().find(|view| same_location(&view.uri, uri)) {
        return own.properties.clone();
    }
    let mut above = parent_location(uri);
    while let Some(folder) = above {
        let inherited = folder_views
            .iter()
            .find(|view| view.subfolders && same_location(&view.uri, &folder));
        if let Some(inherited) = inherited {
            return inherited.properties.clone();
        }
        above = parent_location(&folder);
    }
    defaults.clone()
}

/// Saves `properties` for `uri` with `scope` into `folder_views` and
/// `defaults`. A location that may not be kept changes nothing but the
/// default of [`ViewScope::AllFolders`].
pub(super) fn remember(
    folder_views: &mut Vec<FolderView>,
    defaults: &mut Option<ViewProperties>,
    uri: &str,
    properties: ViewProperties,
    scope: ViewScope,
) {
    if scope == ViewScope::AllFolders {
        folder_views.clear();
        *defaults = Some(properties);
        return;
    }
    if !may_remember(uri) {
        return;
    }
    let subfolders = scope == ViewScope::FolderAndSubfolders;
    let replaced =
        |view: &FolderView| same_location(&view.uri, uri) || (subfolders && is_below(&view.uri, uri));
    folder_views.retain(|view| !replaced(view));
    folder_views.push(FolderView {
        uri: uri.to_owned(),
        properties,
        subfolders,
    });
    let excess = folder_views.len().saturating_sub(MAX_FOLDER_VIEWS);
    folder_views.drain(..excess);
}

/// Whether `uri` is inside `folder`, at any depth.
fn is_below(uri: &str, folder: &str) -> bool {
    let mut above = parent_location(uri);
    while let Some(parent) = above {
        if same_location(&parent, folder) {
            return true;
        }
        above = parent_location(&parent);
    }
    false
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn sorted_by(key: &str) -> ViewProperties {
        ViewProperties {
            sort: key.to_owned(),
            ..ViewProperties::default()
        }
    }

    /// A folder's own style wins, a style saved for sub-folders reaches
    /// below its folder, and every other folder shows the default; saving
    /// for all folders drops the folders' own styles.
    ///
    /// parity: VIEW-020
    #[test]
    fn folders_keep_their_own_style_and_the_rest_share_the_default() {
        let mut views = Vec::new();
        let mut defaults = None;
        remember(
            &mut views,
            &mut defaults,
            "file:///a",
            sorted_by("size"),
            ViewScope::Folder,
        );
        remember(
            &mut views,
            &mut defaults,
            "file:///b",
            sorted_by("type"),
            ViewScope::FolderAndSubfolders,
        );
        remember(
            &mut views,
            &mut defaults,
            "sftp://me@host/x",
            sorted_by("type"),
            ViewScope::Folder,
        );
        let shared = ViewProperties::default();
        assert_eq!(style_for(&views, &shared, "file:///a/").sort, "size");
        assert_eq!(style_for(&views, &shared, "file:///a/inner").sort, "name");
        assert_eq!(style_for(&views, &shared, "file:///b/c/d").sort, "type");
        assert_eq!(views.len(), 2, "no user name is kept");

        let stored = serde_json::to_value(&views).expect("serialises");
        assert_eq!(read_folder_views(&stored), Some(views.clone()));
        assert_eq!(stored[1]["subfolders"], json!(true));

        remember(
            &mut views,
            &mut defaults,
            "file:///a",
            sorted_by("size"),
            ViewScope::AllFolders,
        );
        assert!(views.is_empty());
        assert_eq!(defaults.map(|style| style.sort).as_deref(), Some("size"));
    }
}
