// SPDX-License-Identifier: AGPL-3.0-only
//! Text for the Type column, as Windows 11 File Explorer names types.
//!
//! Navigable items keep the classifier's folder wording ("File folder",
//! "Network share", "Network location"), as `entry_from_info` in
//! `v2.0.0:desktop/gio_backend.py` gave it. Files take the name Windows
//! shows for their type: its registered name for common types, such as
//! "Text Document", "PDF Document", "Microsoft Word Document",
//! "`OpenDocument` Spreadsheet", "Compressed (zipped) Folder", "Application"
//! and "Shortcut"; for pictures, recordings and Markdown, which a fresh
//! Windows registers no name for, "<EXT> File" with the name's extension
//! ("PNG File", "JPG File", "MP4 File"). Every other type takes GIO's
//! description in Title Case, as Windows writes type names ("ODS
//! spreadsheet" from shared-mime-info reads "ODS Spreadsheet"), and a type
//! GIO cannot describe is "<EXT> File", or "File" without an extension.
//!
//! The names of common types are fixed because GIO's description of them
//! depends on which packages installed MIME definitions: shared-mime-info
//! alone says "Word 2007 document" and "Plain text document", the
//! `LibreOffice` definitions say "Microsoft Word Document", so the column
//! would read differently from one computer to the next.
//!
//! Only an English desktop gets the casing and the "<EXT> File" names: in
//! another language GIO's descriptions are already translated, so they are
//! kept, and a fixed name is shown only when the interface's catalogue
//! translates it.

use super::classify::FolderType;
use crate::i18n::message_id;
use crate::integration::MimeType;

/// Label for an item whose type is unknown and whose name has no
/// extension, as Windows calls one.
const UNKNOWN_TYPE: &str = message_id("File");

/// The Type column's name for ZIP archives, as Windows 11 calls them.
const ZIP_LABEL: &str = message_id("Compressed (zipped) Folder");

/// Windows' name for a PDF while an app opens PDFs; without one it is
/// "PDF File", as for any type nothing registered.
const PDF_LABEL: &str = message_id("PDF Document");

/// GIO's content type of every empty file.
const EMPTY_FILE_TYPE: &str = "application/x-zerosize";

/// What decides a type's name besides the item itself.
#[derive(Debug, Clone, Copy)]
struct Desktop {
    /// The desktop's language is English, so GIO's descriptions are too.
    english: bool,
    /// An app opens PDFs.
    has_pdf_reader: bool,
}

impl Desktop {
    /// This computer's.
    fn current() -> Self {
        Self {
            english: crate::i18n::desktop_language_is_english(),
            has_pdf_reader: has_pdf_reader(),
        }
    }
}

/// Type column text of the item `name`: the label of `folder_type` for
/// navigable items, else Windows' name for `content_type`.
pub(super) fn type_label(folder_type: Option<FolderType>, content_type: Option<&str>, name: &str) -> String {
    if let Some(folder_type) = folder_type {
        return folder_type.label().to_owned();
    }
    file_type_label(content_type, name, Desktop::current())
}

fn file_type_label(content_type: Option<&str>, name: &str, desktop: Desktop) -> String {
    let content_type = content_type.filter(|mime| !mime.is_empty());
    // GIO calls every empty file an "empty document"; Windows names it by
    // its extension, as any other file ("Text Document" for an empty .txt).
    let guessed =
        (content_type == Some(EMPTY_FILE_TYPE)).then(|| gio::content_type_guess(Some(name), None).0);
    let content_type = guessed.as_deref().or(content_type);
    if let Some(content_type) = content_type {
        if let Some(label) = windows_name(content_type, desktop) {
            return label;
        }
        if desktop.english && names_by_extension(content_type) {
            if let Some(label) = extension_label(name) {
                return label;
            }
        }
        let description = gio::content_type_get_description(content_type);
        if !is_generic(content_type, &description) {
            return if desktop.english {
                title_case(&description)
            } else {
                description.into()
            };
        }
    }
    let by_extension = desktop.english.then(|| extension_label(name)).flatten();
    by_extension.unwrap_or_else(|| crate::i18n::gettext(UNKNOWN_TYPE))
}

/// Windows' registered name for `content_type`, for the types it names
/// itself; `None` for every other type, and outside English for a name the
/// interface's catalogue does not translate.
fn windows_name(content_type: &str, desktop: Desktop) -> Option<String> {
    let label = windows_message(content_type, desktop)?;
    if desktop.english {
        return Some(label.to_owned());
    }
    crate::i18n::translation(label).map(str::to_owned)
}

/// The message id of [`windows_name`].
fn windows_message(content_type: &str, desktop: Desktop) -> Option<&'static str> {
    if MimeType::from_name(content_type).is_some_and(MimeType::is_zip) {
        return Some(ZIP_LABEL);
    }
    let label = match content_type {
        "text/plain" => message_id("Text Document"),
        "application/pdf" if desktop.has_pdf_reader => PDF_LABEL,
        "application/pdf" => message_id("PDF File"),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            message_id("Microsoft Word Document")
        }
        "application/msword" => message_id("Microsoft Word 97 - 2003 Document"),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => {
            message_id("Microsoft Excel Worksheet")
        }
        "application/vnd.ms-excel" => message_id("Microsoft Excel 97-2003 Worksheet"),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
            message_id("Microsoft PowerPoint Presentation")
        }
        "application/vnd.ms-powerpoint" => message_id("Microsoft PowerPoint 97-2003 Presentation"),
        "application/vnd.oasis.opendocument.text" => message_id("OpenDocument Text"),
        "application/vnd.oasis.opendocument.spreadsheet" => message_id("OpenDocument Spreadsheet"),
        "application/vnd.oasis.opendocument.presentation" => message_id("OpenDocument Presentation"),
        "application/x-executable"
        | "application/x-pie-executable"
        | "application/x-msdownload"
        | "application/x-ms-dos-executable"
        | "application/vnd.microsoft.portable-executable" => message_id("Application"),
        "application/x-ms-shortcut" | "application/x-desktop" => message_id("Shortcut"),
        _ => return None,
    };
    Some(label)
}

/// Whether Windows names `content_type` "<EXT> File": pictures, recordings
/// and Markdown, which a fresh Windows 11 registers no name for.
fn names_by_extension(content_type: &str) -> bool {
    matches!(
        content_type,
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/tiff"
            | "image/heic"
            | "image/heif"
            | "image/avif"
            | "audio/mpeg"
            | "audio/mp4"
            | "audio/x-m4a"
            | "audio/flac"
            | "audio/x-flac"
            | "audio/wav"
            | "audio/x-wav"
            | "audio/vnd.wave"
            | "audio/ogg"
            | "video/mp4"
            | "video/quicktime"
            | "video/x-matroska"
            | "video/webm"
            | "video/x-msvideo"
            | "text/markdown"
            | "text/x-markdown"
    )
}

/// Whether GIO's `description` of `content_type` says nothing about the
/// type: unknown data, or no description at all.
fn is_generic(content_type: &str, description: &str) -> bool {
    description.is_empty() || gio::content_type_is_unknown(content_type)
}

/// "<EXT> File" for a `name` with an extension, the extension in capitals
/// as Windows writes it ("Report.ods" is an "ODS File"); `None` without
/// one.
fn extension_label(name: &str) -> Option<String> {
    let (_, extension) = name.rsplit_once('.')?;
    if extension.is_empty() {
        return None;
    }
    Some(crate::i18n::format_message(
        "{extension} File",
        &[("extension", &extension.to_uppercase())],
    ))
}

/// `description` with each word's first letter in capitals, as Windows
/// writes type names: "ODS spreadsheet" is "ODS Spreadsheet". Short joining
/// words after the first stay small ("Rich Text Format" but "Tape Archive
/// of Files"), and letters already in capitals stay so ("PostScript").
fn title_case(description: &str) -> String {
    const SMALL_WORDS: [&str; 10] = ["a", "an", "and", "as", "for", "in", "of", "or", "the", "with"];
    let mut words = Vec::new();
    for (index, word) in description.split(' ').enumerate() {
        if index > 0 && SMALL_WORDS.contains(&word) {
            words.push(word.to_owned());
            continue;
        }
        let mut letters = word.chars();
        let capitalised = match letters.next() {
            Some(first) => first.to_uppercase().chain(letters).collect(),
            None => String::new(),
        };
        words.push(capitalised);
    }
    words.join(" ")
}

/// Whether an app opens PDFs, as Windows then calls them "PDF Document";
/// asked once per run.
fn has_pdf_reader() -> bool {
    static HAS_READER: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *HAS_READER.get_or_init(|| gio::AppInfo::default_for_type("application/pdf", false).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::test_support::FOLDER_MIME_TYPE;

    const ENGLISH: Desktop = Desktop {
        english: true,
        has_pdf_reader: true,
    };

    /// A file and the Type column text it must show.
    struct LabelCase {
        name: &'static str,
        content_type: &'static str,
        label: &'static str,
    }

    const fn case(name: &'static str, content_type: &'static str, label: &'static str) -> LabelCase {
        LabelCase {
            name,
            content_type,
            label,
        }
    }

    /// Every type Windows names itself, and the name.
    const WINDOWS_NAMES: [LabelCase; 19] = [
        case("Notes.txt", "text/plain", "Text Document"),
        case("README", "text/plain", "Text Document"),
        case("Manual.pdf", "application/pdf", "PDF Document"),
        case(
            "Letter.docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "Microsoft Word Document",
        ),
        case(
            "Old letter.doc",
            "application/msword",
            "Microsoft Word 97 - 2003 Document",
        ),
        case(
            "Budget.xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "Microsoft Excel Worksheet",
        ),
        case(
            "Old budget.xls",
            "application/vnd.ms-excel",
            "Microsoft Excel 97-2003 Worksheet",
        ),
        case(
            "Talk.pptx",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "Microsoft PowerPoint Presentation",
        ),
        case(
            "Old talk.ppt",
            "application/vnd.ms-powerpoint",
            "Microsoft PowerPoint 97-2003 Presentation",
        ),
        case(
            "Essay.odt",
            "application/vnd.oasis.opendocument.text",
            "OpenDocument Text",
        ),
        case(
            "Sales.ods",
            "application/vnd.oasis.opendocument.spreadsheet",
            "OpenDocument Spreadsheet",
        ),
        case(
            "Slides.odp",
            "application/vnd.oasis.opendocument.presentation",
            "OpenDocument Presentation",
        ),
        case("Bundle.zip", "application/zip", "Compressed (zipped) Folder"),
        case(
            "Bundle.zip",
            "application/x-zip-compressed",
            "Compressed (zipped) Folder",
        ),
        case("tool", "application/x-executable", "Application"),
        case("tool", "application/x-pie-executable", "Application"),
        case("Setup.exe", "application/x-msdownload", "Application"),
        case("Editor.desktop", "application/x-desktop", "Shortcut"),
        case("Old link.lnk", "application/x-ms-shortcut", "Shortcut"),
    ];

    /// parity: VIEW-002
    #[test]
    fn folder_wording_wins() {
        assert_eq!(
            type_label(Some(FolderType::NetworkShare), Some(FOLDER_MIME_TYPE), "Share"),
            "Network share"
        );
        assert_eq!(
            type_label(Some(FolderType::FileFolder), None, "Docs"),
            "File folder"
        );
    }

    /// Regression: GIO's descriptions of these types read "Word 2007
    /// document", "Excel 2007 spreadsheet" and "Plain text document"
    /// wherever the `LibreOffice` MIME definitions were not installed.
    ///
    /// parity: VIEW-002
    #[test]
    fn common_types_take_the_names_windows_gives_them() {
        for case in WINDOWS_NAMES {
            assert_eq!(
                file_type_label(Some(case.content_type), case.name, ENGLISH),
                case.label,
                "{}",
                case.content_type
            );
        }
    }

    /// parity: VIEW-002
    #[test]
    fn a_pdf_is_a_pdf_file_while_no_app_opens_pdfs() {
        let no_reader = Desktop {
            has_pdf_reader: false,
            ..ENGLISH
        };
        assert_eq!(
            file_type_label(Some("application/pdf"), "Manual.pdf", no_reader),
            "PDF File"
        );
    }

    /// parity: VIEW-002
    #[test]
    fn pictures_recordings_and_markdown_are_named_by_their_extension() {
        let cases = [
            case("Beach.png", "image/png", "PNG File"),
            case("Beach.jpg", "image/jpeg", "JPG File"),
            case("Beach.JPEG", "image/jpeg", "JPEG File"),
            case("Song.mp3", "audio/mpeg", "MP3 File"),
            case("Clip.mp4", "video/mp4", "MP4 File"),
            case("README.md", "text/markdown", "MD File"),
        ];
        for case in cases {
            assert_eq!(
                file_type_label(Some(case.content_type), case.name, ENGLISH),
                case.label,
                "{}",
                case.name
            );
        }
        let without_extension = file_type_label(Some("image/png"), "Beach", ENGLISH);
        assert_eq!(without_extension, "PNG Image", "GIO's description, in Title Case");
    }

    /// parity: VIEW-002
    #[test]
    fn other_types_take_the_gio_description_in_title_case() {
        for content_type in [
            "application/vnd.oasis.opendocument.graphics",
            "text/csv",
            "text/html",
        ] {
            let description = gio::content_type_get_description(content_type);
            let label = file_type_label(Some(content_type), "Item.dat", ENGLISH);
            assert_eq!(label, title_case(&description), "{content_type}");
            assert!(
                label.split(' ').all(|word| !word.starts_with(char::is_lowercase)),
                "{label}"
            );
        }
        assert_eq!(title_case("ODS spreadsheet"), "ODS Spreadsheet");
        assert_eq!(title_case("tape archive of files"), "Tape Archive of Files");
        assert_eq!(title_case("PostScript document"), "PostScript Document");
    }

    /// parity: VIEW-002
    #[test]
    fn an_unknown_type_is_named_by_its_extension_or_is_a_file() {
        let unknown = "application/octet-stream";
        assert_eq!(file_type_label(Some(unknown), "Data.xyz", ENGLISH), "XYZ File");
        assert_eq!(file_type_label(Some(unknown), "Data", ENGLISH), "File");
        assert_eq!(file_type_label(None, "Data.bin", ENGLISH), "BIN File");
        assert_eq!(file_type_label(None, "Data", ENGLISH), "File");
        assert_eq!(file_type_label(Some(""), "Data.", ENGLISH), "File");
    }

    /// An empty file is named by its extension, not "Empty Document".
    ///
    /// parity: VIEW-002
    #[test]
    fn an_empty_file_is_named_by_its_extension() {
        let empty = EMPTY_FILE_TYPE;
        assert_eq!(
            file_type_label(Some(empty), "Notes.txt", ENGLISH),
            "Text Document"
        );
        assert_eq!(file_type_label(Some(empty), "Data.xyz", ENGLISH), "XYZ File");
        assert_eq!(file_type_label(Some(empty), "Data", ENGLISH), "File");
    }

    /// Another language keeps GIO's translated descriptions and its plain
    /// "File"; the fixed names need the interface's translation, which
    /// tests have none of.
    ///
    /// parity: VIEW-002
    #[test]
    fn another_language_keeps_the_gio_description() {
        let other = Desktop {
            english: false,
            ..ENGLISH
        };
        for content_type in ["text/plain", "image/png", "text/markdown"] {
            let description = gio::content_type_get_description(content_type);
            assert_eq!(
                file_type_label(Some(content_type), "Item.dat", other),
                description.as_str(),
                "{content_type}"
            );
        }
        assert_eq!(file_type_label(None, "Data.bin", other), "File");
    }
}
