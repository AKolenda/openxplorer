// SPDX-License-Identifier: AGPL-3.0-only
//! Text for the Type column.
//!
//! Ports the type description of `entry_from_info` in
//! `desktop/gio_backend.py`: the classifier's folder wording ("File
//! folder", "Network share", "Network location") for navigable items,
//! otherwise `Gio.content_type_get_description`, otherwise "File".
//!
//! GIO's descriptions come from shared-mime-info. They follow the user's
//! language and already use Explorer's wording for Office documents
//! ("Microsoft Word Document", "Microsoft Excel Worksheet"), so they are
//! shown unchanged, as in the Python app.

use super::classify::FolderType;

/// Label for an item whose content type is unknown or has no description.
const UNKNOWN_TYPE: &str = "File";

/// Type column text: the label of `folder_type` for navigable items,
/// otherwise GIO's description of `content_type`, otherwise "File".
pub(super) fn type_label(folder_type: Option<FolderType>, content_type: Option<&str>) -> String {
    if let Some(folder_type) = folder_type {
        return folder_type.label().to_owned();
    }
    let Some(content_type) = content_type.filter(|mime| !mime.is_empty()) else {
        return UNKNOWN_TYPE.to_owned();
    };
    let description = gio::content_type_get_description(content_type);
    if description.is_empty() {
        UNKNOWN_TYPE.to_owned()
    } else {
        description.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::test_support::FOLDER_MIME_TYPE;

    /// parity: VIEW-002
    #[test]
    fn folder_wording_wins() {
        assert_eq!(
            type_label(Some(FolderType::NetworkShare), Some(FOLDER_MIME_TYPE)),
            "Network share"
        );
        assert_eq!(type_label(Some(FolderType::FileFolder), None), "File folder");
    }

    /// Regression: the port replaced GIO's localised descriptions of text and
    /// Office files with English wording from the web interface's preview
    /// data, so the column mixed languages and lost Explorer's wording.
    ///
    /// parity: VIEW-002
    #[test]
    fn file_types_show_the_gio_description_unchanged() {
        let content_types = [
            "text/plain",
            "application/msword",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/vnd.ms-excel",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "application/vnd.ms-powerpoint",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "application/pdf",
            "image/png",
            "application/zip",
            "video/mp4",
            "text/markdown",
        ];
        for content_type in content_types {
            let description = gio::content_type_get_description(content_type);
            assert_eq!(
                type_label(None, Some(content_type)),
                description.as_str(),
                "{content_type}"
            );
        }
    }

    #[test]
    fn missing_content_type_is_a_file() {
        assert_eq!(type_label(None, None), "File");
        assert_eq!(type_label(None, Some("")), "File");
    }
}
