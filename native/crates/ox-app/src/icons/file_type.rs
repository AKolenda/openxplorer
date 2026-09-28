// SPDX-License-Identifier: AGPL-3.0-only
//! Which colour art a file gets: its type, from its extension or, for a name
//! without a known one, its content type.
//!
//! Replaces the `map` of `fileIcon` and `isZipEntry` in `desktop/ui/app.js`.
//! Files are grouped by the Fluent colour icon that shows them. The Word,
//! Excel and PDF letter badges of app.js are gone: an open icon set cannot
//! use Office logos (owner, 2026-09-27), so those files show the document.

use crate::icons::Icon;

/// The kind of file a colour icon shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FileType {
    /// Word documents, PDFs, slides and every file of no other type.
    Document,
    /// Plain text and Markdown.
    Text,
    /// Spreadsheets and CSV files.
    Spreadsheet,
    /// Pictures.
    Image,
    /// Videos.
    Video,
    /// Music and other audio.
    Audio,
    /// Source code, HTML and JSON.
    Code,
}

/// Which of Fluent's designs of a colour icon is drawn. Each is made for
/// one size, so a file is drawn with the design nearest the size it is
/// shown at ("the 16/20/32/48 variant nearest the drawn size" in the
/// approved icon mapping; no picture is drawn below 19 pixels).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Design {
    /// The 20-pixel design, up to 26 pixels.
    Small,
    /// The 32-pixel design, from 27 to 40 pixels.
    Medium,
    /// The 48-pixel design, above 40 pixels.
    Large,
}

impl Design {
    /// The design nearest `size` pixels.
    const fn nearest(size: i32) -> Self {
        if size <= 26 {
            Design::Small
        } else if size <= 40 {
            Design::Medium
        } else {
            Design::Large
        }
    }
}

impl FileType {
    /// The type of a file called `name` whose content type GIO reported as
    /// `content_type`. The extension decides when it is a known one, as in
    /// app.js; otherwise the content type does, and any other file is a
    /// [`FileType::Document`].
    pub(crate) fn of(name: &str, content_type: Option<&str>) -> Self {
        let by_extension = Self::for_extension(&extension(name));
        let by_content = content_type.and_then(Self::for_content_type);
        by_extension.or(by_content).unwrap_or(FileType::Document)
    }

    /// The type of a lower-cased extension, `None` for an unknown one. An
    /// exhaustive table, hence its length.
    fn for_extension(extension: &str) -> Option<Self> {
        let file_type = match extension {
            "txt" | "md" | "markdown" | "log" | "rst" => FileType::Text,
            "xlsx" | "xls" | "ods" | "csv" | "tsv" => FileType::Spreadsheet,
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "svg" | "tif" | "tiff" | "heic" => {
                FileType::Image
            }
            "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" => FileType::Video,
            "mp3" | "flac" | "ogg" | "oga" | "opus" | "wav" | "m4a" | "aac" => FileType::Audio,
            "py" | "js" | "ts" | "rs" | "c" | "h" | "cpp" | "java" | "sh" | "css" | "html" | "htm"
            | "json" | "xml" | "toml" | "yaml" | "yml" => FileType::Code,
            _ => return None,
        };
        Some(file_type)
    }

    /// The type of a GIO content type, `None` when it says nothing the
    /// icons show.
    fn for_content_type(content_type: &str) -> Option<Self> {
        let (family, subtype) = content_type.split_once('/')?;
        match (family, subtype) {
            ("image", _) => Some(FileType::Image),
            ("video", _) => Some(FileType::Video),
            ("audio", _) => Some(FileType::Audio),
            ("text", "csv") => Some(FileType::Spreadsheet),
            ("text", "html") | ("application", "json" | "javascript" | "xml") => Some(FileType::Code),
            ("text", _) => Some(FileType::Text),
            _ => None,
        }
    }

    /// The colour icon of this type, in the design for `size` pixels. An
    /// exhaustive table, hence its length. Fluent draws code no larger than
    /// 24 pixels, so that design serves the larger sizes.
    pub(crate) const fn icon(self, size: i32) -> Icon {
        match (self, Design::nearest(size)) {
            (FileType::Document, Design::Small) => Icon::DocumentColor20,
            (FileType::Document, Design::Medium) => Icon::DocumentColor32,
            (FileType::Document, Design::Large) => Icon::DocumentColor48,
            (FileType::Text, Design::Small) => Icon::DocumentTextColor20,
            (FileType::Text, Design::Medium) => Icon::DocumentTextColor32,
            (FileType::Text, Design::Large) => Icon::DocumentTextColor48,
            (FileType::Spreadsheet, Design::Small) => Icon::TableColor20,
            (FileType::Spreadsheet, Design::Medium) => Icon::TableColor32,
            (FileType::Spreadsheet, Design::Large) => Icon::TableColor48,
            (FileType::Image, Design::Small) => Icon::ImageColor20,
            (FileType::Image, Design::Medium) => Icon::ImageColor32,
            (FileType::Image, Design::Large) => Icon::ImageColor48,
            (FileType::Video, Design::Small) => Icon::VideoColor20,
            (FileType::Video, Design::Medium) => Icon::VideoColor32,
            (FileType::Video, Design::Large) => Icon::VideoColor48,
            (FileType::Audio, Design::Small) => Icon::HeadphonesColor20,
            (FileType::Audio, Design::Medium) => Icon::HeadphonesColor32,
            (FileType::Audio, Design::Large) => Icon::HeadphonesColor48,
            (FileType::Code, Design::Small) => Icon::CodeColor20,
            (FileType::Code, Design::Medium | Design::Large) => Icon::CodeColor24,
        }
    }
}

/// `.zip` names and ZIP content types, as `isZipEntry` in app.js.
pub(crate) fn is_zip(name: &str, content_type: Option<&str>) -> bool {
    let zip_name = name.to_lowercase().ends_with(".zip");
    let zip_type = matches!(
        content_type,
        Some("application/zip" | "application/x-zip" | "application/x-zip-compressed")
    );
    zip_name || zip_type
}

/// The lower-cased text after the last dot (the whole name when there is
/// none, as `split('.').pop()` does in app.js).
fn extension(name: &str) -> String {
    name.rsplit('.').next().unwrap_or_default().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file and the type its icon shows.
    struct TypeCase {
        name: &'static str,
        content_type: Option<&'static str>,
        expected: FileType,
    }

    /// A case, written on one line of a table.
    const fn case(name: &'static str, content_type: Option<&'static str>, expected: FileType) -> TypeCase {
        TypeCase {
            name,
            content_type,
            expected,
        }
    }

    /// Checks that each case's file has the expected type.
    fn assert_types(cases: &[TypeCase]) {
        for case in cases {
            let file_type = FileType::of(case.name, case.content_type);
            assert_eq!(file_type, case.expected, "{}", case.name);
        }
    }

    /// parity: LOOK-015
    #[test]
    fn a_known_extension_decides_the_type() {
        let cases = [
            case("Quarterly report.docx", None, FileType::Document),
            case("Brochure.PDF", None, FileType::Document),
            case("Q3 presentation.pptx", None, FileType::Document),
            case("Read me.txt", None, FileType::Text),
            case("Research notes.md", None, FileType::Text),
            case("Budget.xlsx", None, FileType::Spreadsheet),
            case("Export.csv", None, FileType::Spreadsheet),
            case("Site mockup.png", None, FileType::Image),
            case("Clip.mp4", None, FileType::Video),
            case("Song.flac", None, FileType::Audio),
            case("index.html", None, FileType::Code),
            case("data.json", None, FileType::Code),
            case("script.py", None, FileType::Code),
            case("clip.txt", Some("video/mp4"), FileType::Text),
        ];
        assert_types(&cases);
    }

    /// parity: LOOK-015
    #[test]
    fn a_name_without_a_known_extension_follows_its_content_type() {
        let cases = [
            case("IMG_0001", Some("image/jpeg"), FileType::Image),
            case("notes", Some("text/plain"), FileType::Text),
            case("page", Some("text/html"), FileType::Code),
            case("cargo", Some("application/x-executable"), FileType::Document),
            case("README", None, FileType::Document),
        ];
        assert_types(&cases);
    }

    /// parity: LOOK-015
    #[test]
    fn extensions_are_lower_cased() {
        assert_eq!(extension("Quarterly report.DOCX"), "docx");
        assert_eq!(extension("archive.tar.gz"), "gz");
        assert_eq!(extension("README"), "readme");
    }

    /// parity: ARC-001
    #[test]
    fn zip_is_detected_by_name_in_any_case_or_by_content_type() {
        assert!(is_zip("Brand kit.ZIP", None));
        assert!(is_zip("download", Some("application/x-zip-compressed")));
        assert!(!is_zip("notes.zip.txt", Some("text/plain")));
    }

    /// Each picture is drawn with the design nearest its size: rows (21
    /// pixels) the 20-pixel one, small and medium tiles (28, 40) the
    /// 32-pixel one, large tiles and the details pane (56, 83, 96) the
    /// 48-pixel one.
    #[test]
    fn each_size_is_drawn_with_the_nearest_design() {
        assert_eq!(FileType::Document.icon(21), Icon::DocumentColor20);
        assert_eq!(FileType::Document.icon(28), Icon::DocumentColor32);
        assert_eq!(FileType::Document.icon(40), Icon::DocumentColor32);
        assert_eq!(FileType::Document.icon(56), Icon::DocumentColor48);
        assert_eq!(FileType::Image.icon(83), Icon::ImageColor48);
        assert_eq!(
            FileType::Code.icon(96),
            Icon::CodeColor24,
            "code's largest design"
        );
    }
}
