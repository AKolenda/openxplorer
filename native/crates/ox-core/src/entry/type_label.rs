// SPDX-License-Identifier: AGPL-3.0-only
//! Text for the Type column.
//!
//! Ports the type description of `entry_from_info` in
//! `desktop/gio_backend.py`: the classifier's folder wording ("File
//! folder", "Network share", "Network location") for navigable items,
//! otherwise `Gio.content_type_get_description`, otherwise "File".
//!
//! Common Office, text, ZIP and video files show the interface's own names
//! instead, the ones of the preview listing in `desktop/ui/app.js`
//! (`demo`), such as "Word document", "Excel worksheet", "Text document",
//! "Compressed folder" and "MP4 video".
//! GIO's description of these types depends on which packages installed
//! MIME definitions: shared-mime-info alone says "Word 2007 document" and
//! "Plain text document", the `LibreOffice` definitions say "Microsoft Word
//! Document", so the column would read differently from one computer to
//! the next. Every other type keeps GIO's description, which already reads
//! "PDF document", "PNG image" or "Markdown document".

use super::classify::FolderType;
use crate::integration::MimeType;

/// Label for an item whose content type is unknown or has no description.
const UNKNOWN_TYPE: &str = "File";

/// The Type column's name for ZIP archives, as Explorer and the preview
/// listing call them, and as the ZIP browser's title reads
/// (`"<name> — Compressed folder"` in `desktop/ui/app.js`).
const ZIP_LABEL: &str = "Compressed folder";

/// Type column text: the label of `folder_type` for navigable items, then
/// the interface's name for a common type, then GIO's description of
/// `content_type`, then "File".
pub(super) fn type_label(folder_type: Option<FolderType>, content_type: Option<&str>) -> String {
    if let Some(folder_type) = folder_type {
        return folder_type.label().to_owned();
    }
    let Some(content_type) = content_type.filter(|mime| !mime.is_empty()) else {
        return UNKNOWN_TYPE.to_owned();
    };
    if let Some(label) = interface_label(content_type) {
        return label.to_owned();
    }
    let description = gio::content_type_get_description(content_type);
    if description.is_empty() {
        UNKNOWN_TYPE.to_owned()
    } else {
        description.into()
    }
}

/// The interface's own name for `content_type`, for the types whose GIO
/// description varies with the installed MIME definitions; `None` for
/// every other type.
fn interface_label(content_type: &str) -> Option<&'static str> {
    if MimeType::from_name(content_type).is_some_and(MimeType::is_zip) {
        return Some(ZIP_LABEL);
    }
    let label =
        match content_type {
            "text/plain" => "Text document",
            "application/msword"
            | "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "Word document",
            "application/vnd.ms-excel"
            | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "Excel worksheet",
            "application/vnd.ms-powerpoint"
            | "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
                "PowerPoint presentation"
            }
            "video/mp4" => "MP4 video",
            _ => return None,
        };
    Some(label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::test_support::FOLDER_MIME_TYPE;

    /// A content type and the Type column text it must show.
    struct LabelCase {
        content_type: &'static str,
        label: &'static str,
    }

    /// parity: VIEW-002
    #[test]
    fn folder_wording_wins() {
        assert_eq!(
            type_label(Some(FolderType::NetworkShare), Some(FOLDER_MIME_TYPE)),
            "Network share"
        );
        assert_eq!(type_label(Some(FolderType::FileFolder), None), "File folder");
    }

    /// Regression: a review replaced these names with GIO's descriptions,
    /// so the Documents folder read "Word 2007 document", "Excel 2007
    /// spreadsheet" and "Plain text document" wherever the `LibreOffice` MIME
    /// definitions were not installed.
    ///
    /// parity: VIEW-002
    #[test]
    fn office_text_zip_and_video_files_use_the_interface_names() {
        let cases = [
            LabelCase {
                content_type: "text/plain",
                label: "Text document",
            },
            LabelCase {
                content_type: "application/msword",
                label: "Word document",
            },
            LabelCase {
                content_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                label: "Word document",
            },
            LabelCase {
                content_type: "application/vnd.ms-excel",
                label: "Excel worksheet",
            },
            LabelCase {
                content_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                label: "Excel worksheet",
            },
            LabelCase {
                content_type: "application/vnd.ms-powerpoint",
                label: "PowerPoint presentation",
            },
            LabelCase {
                content_type: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
                label: "PowerPoint presentation",
            },
            LabelCase {
                content_type: "application/zip",
                label: "Compressed folder",
            },
            LabelCase {
                content_type: "application/x-zip",
                label: "Compressed folder",
            },
            LabelCase {
                content_type: "application/x-zip-compressed",
                label: "Compressed folder",
            },
            LabelCase {
                content_type: "video/mp4",
                label: "MP4 video",
            },
        ];
        for case in cases {
            assert_eq!(
                type_label(None, Some(case.content_type)),
                case.label,
                "{}",
                case.content_type
            );
        }
    }

    /// parity: VIEW-002
    #[test]
    fn other_types_keep_the_gio_description() {
        let content_types = [
            "application/pdf",
            "image/png",
            "image/jpeg",
            "text/markdown",
            "text/csv",
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
