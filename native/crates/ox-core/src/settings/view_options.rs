// SPDX-License-Identifier: AGPL-3.0-only
//! The folder views' own options: previews (VIEW-057, VIEW-058), item
//! counts in a folder's Size (VIEW-037) and the details columns shown and
//! their order (VIEW-033, VIEW-034), as Dolphin's View settings and its
//! Previews page keep them.

use serde::Serialize;
use serde_json::Value;

/// Files larger than this, 50 MB, get no preview while
/// [`ViewOptions::skip_large_previews`] is on: decoding them costs more
/// than a preview is worth, as Files' `thumbnail-limit` decides. Settings
/// names the limit in its text.
pub const PREVIEW_SIZE_LIMIT: u64 = 50 * 1024 * 1024;

/// The details columns after Name that a new installation shows, in their
/// order.
pub const DEFAULT_DETAILS_COLUMNS: [&str; 3] = ["modified", "type", "size"];

/// The most details columns that may be stored, and the longest key.
const MAX_COLUMNS: usize = 16;
const MAX_COLUMN_KEY: usize = 24;

/// How the folder views show items. Stored as `viewOptions`, which the
/// Python app does not read.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is a separate on/off option of the Settings page"
)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewOptions {
    /// Items show a preview of their contents in place of their icon
    /// (Dolphin's Show Previews).
    pub show_previews: bool,
    /// Items in network folders get previews too; off, as Dolphin's "Skip
    /// previews for remote files" starts.
    pub remote_previews: bool,
    /// Files over [`PREVIEW_SIZE_LIMIT`] get no preview.
    pub skip_large_previews: bool,
    /// Pictures get previews; this and the next two choose the types, as
    /// Dolphin's list of preview plugins does.
    pub preview_pictures: bool,
    /// Videos get previews.
    pub preview_videos: bool,
    /// Documents and every other type a thumbnailer supports get previews.
    pub preview_documents: bool,
    /// A folder's Size in the Details view says how many items it holds,
    /// counted for folders on this computer only.
    pub count_folder_items: bool,
    /// The keys of the details columns shown after Name, in their order.
    pub details_columns: Vec<String>,
}

impl Default for ViewOptions {
    fn default() -> Self {
        Self {
            show_previews: true,
            remote_previews: false,
            skip_large_previews: true,
            preview_pictures: true,
            preview_videos: true,
            preview_documents: true,
            count_folder_items: true,
            details_columns: DEFAULT_DETAILS_COLUMNS.map(str::to_owned).to_vec(),
        }
    }
}

impl ViewOptions {
    /// Whether these are the options of a new installation.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The options stored in `value`; a missing or mistyped option keeps
    /// its default, and `None` when `value` is not an object. A column
    /// list that is too long, repeats a key or holds odd keys is ignored.
    pub fn from_json(value: &Value) -> Option<Self> {
        let values = value.as_object()?;
        let defaults = Self::default();
        let flag = |key: &str, default: bool| values.get(key).and_then(Value::as_bool).unwrap_or(default);
        let details_columns = values
            .get("detailsColumns")
            .and_then(read_column_keys)
            .unwrap_or(defaults.details_columns);
        Some(Self {
            show_previews: flag("showPreviews", defaults.show_previews),
            remote_previews: flag("remotePreviews", defaults.remote_previews),
            skip_large_previews: flag("skipLargePreviews", defaults.skip_large_previews),
            preview_pictures: flag("previewPictures", defaults.preview_pictures),
            preview_videos: flag("previewVideos", defaults.preview_videos),
            preview_documents: flag("previewDocuments", defaults.preview_documents),
            count_folder_items: flag("countFolderItems", defaults.count_folder_items),
            details_columns,
        })
    }
}

/// A list of distinct, short, lower-camel-case column keys.
pub(super) fn read_column_keys(value: &Value) -> Option<Vec<String>> {
    let keys = value.as_array()?;
    if keys.len() > MAX_COLUMNS {
        return None;
    }
    let mut columns: Vec<String> = Vec::with_capacity(keys.len());
    for key in keys {
        let key = key.as_str()?;
        let is_key =
            !key.is_empty() && key.len() <= MAX_COLUMN_KEY && key.chars().all(|c| c.is_ascii_alphabetic());
        if !is_key || columns.iter().any(|column| column == key) {
            return None;
        }
        columns.push(key.to_owned());
    }
    Some(columns)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// parity: VIEW-058
    #[test]
    fn view_options_round_trip_and_ignore_odd_values() {
        let options = ViewOptions {
            show_previews: false,
            remote_previews: true,
            skip_large_previews: false,
            preview_pictures: true,
            preview_videos: false,
            preview_documents: true,
            count_folder_items: false,
            details_columns: vec!["size".to_owned(), "created".to_owned()],
        };
        let stored = serde_json::to_value(&options).expect("serialises");
        assert_eq!(ViewOptions::from_json(&stored), Some(options));

        let odd = ViewOptions::from_json(&json!({"showPreviews": "no", "detailsColumns": ["size", "size"]}))
            .expect("an object");
        assert_eq!(odd, ViewOptions::default());
        assert_eq!(ViewOptions::from_json(&json!([])), None);
    }
}
