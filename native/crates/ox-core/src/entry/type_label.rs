// SPDX-License-Identifier: AGPL-3.0-only
//! Text for the Type column.
//!
//! `entry_from_info` in `desktop/gio_backend.py` uses the classifier's
//! folder wording ("File folder", "Network share", "Network location") for
//! navigable items and `Gio.content_type_get_description` for everything
//! else, falling back to "File". GIO's own wording already matches Explorer
//! for most types ("PDF document", "PNG image", "Zip archive", "MPEG-4
//! video", "Markdown document"). A few differ from the names the interface
//! uses in its sample data (`desktop/ui/app.js`, the preview listing):
//! shared-mime-info and LibreOffice register "Plain text document" and
//! "Microsoft Word Document", where OpenXplorer shows "Text document" and
//! "Word document". Those are overridden here; every other type keeps its
//! GIO description.

/// Content types whose GIO description is replaced, with the replacement.
const OVERRIDES: [(&str, &str); 7] = [
    ("text/plain", "Text document"),
    ("application/msword", "Word document"),
    (
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "Word document",
    ),
    ("application/vnd.ms-excel", "Excel worksheet"),
    (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "Excel worksheet",
    ),
    ("application/vnd.ms-powerpoint", "PowerPoint presentation"),
    (
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "PowerPoint presentation",
    ),
];

/// Label for an item whose type is unknown.
pub const GENERIC_FILE: &str = "File";

/// The OpenXplorer wording for `content_type`, when it differs from GIO's.
pub fn override_for(content_type: &str) -> Option<&'static str> {
    OVERRIDES
        .iter()
        .find(|(mime, _)| *mime == content_type)
        .map(|(_, label)| *label)
}

/// Type column text: the classifier's folder wording, then an override,
/// then GIO's description of the content type, then "File".
pub fn type_label(folder_type: Option<&str>, content_type: Option<&str>) -> String {
    if let Some(folder_type) = folder_type {
        return folder_type.to_string();
    }
    let Some(content_type) = content_type.filter(|c| !c.is_empty()) else {
        return GENERIC_FILE.to_string();
    };
    if let Some(label) = override_for(content_type) {
        return label.to_string();
    }
    let description = gio::content_type_get_description(content_type);
    if description.is_empty() {
        GENERIC_FILE.to_string()
    } else {
        description.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_wording_wins() {
        assert_eq!(
            type_label(Some("Network share"), Some("inode/directory")),
            "Network share"
        );
        assert_eq!(type_label(Some("File folder"), None), "File folder");
    }

    #[test]
    fn office_and_text_types_use_the_interface_wording() {
        let docx = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
        let xlsx = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
        assert_eq!(type_label(None, Some(docx)), "Word document");
        assert_eq!(type_label(None, Some("application/msword")), "Word document");
        assert_eq!(type_label(None, Some(xlsx)), "Excel worksheet");
        assert_eq!(type_label(None, Some("text/plain")), "Text document");
    }

    #[test]
    fn other_types_keep_the_gio_description() {
        for mime in [
            "application/pdf",
            "image/png",
            "application/zip",
            "video/mp4",
            "text/markdown",
        ] {
            let expected = gio::content_type_get_description(mime).to_string();
            assert_eq!(type_label(None, Some(mime)), expected, "{mime}");
        }
    }

    #[test]
    fn missing_content_type_is_a_file() {
        assert_eq!(type_label(None, None), "File");
        assert_eq!(type_label(None, Some("")), "File");
    }
}
