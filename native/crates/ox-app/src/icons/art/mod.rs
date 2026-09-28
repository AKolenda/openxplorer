// SPDX-License-Identifier: AGPL-3.0-only
//! Colour icon art: folders, ZIP folders, documents and network locations.
//!
//! Reproduces `appendFolderArt`, `folderIcon`, `zipFolderIcon`, `fileIcon`
//! and `networkIcon` in `desktop/ui/app.js`. This module decides which art
//! an item gets ([`kind_for_entry`]) and keeps the rasterised textures
//! ([`texture`]); the [`svg`] module draws the art as SVG, which GDK
//! rasterises through the gdk-pixbuf SVG loader.

mod svg;

use std::cell::RefCell;
use std::collections::HashMap;

use gtk::{gdk, glib};
use ox_core::entry::Entry;

use crate::icons::Glyph;
use crate::theme::Appearance;

/// Which piece of art to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArtKind {
    /// The yellow folder.
    Folder,
    /// A folder with a zipper, for ZIP archives.
    ZipFolder,
    /// A network share in a server listing: a folder with a network badge.
    SharedFolder,
    /// A folder on the green network pipe (a saved network location).
    NetworkFolder,
    /// A stroke glyph (for example [`Glyph::Server`]) on the green network
    /// pipe.
    NetworkGlyph(Glyph),
    /// A document in the colour and badge of its type.
    Document(DocumentStyle),
}

impl ArtKind {
    /// True when the art uses theme colours (document paper, the network
    /// badge, a glyph on the pipe), so it is drawn and cached per
    /// [`Appearance`].
    fn depends_on_theme(self) -> bool {
        matches!(
            self,
            ArtKind::Document(_) | ArtKind::SharedFolder | ArtKind::NetworkGlyph(_)
        )
    }
}

/// The document looks of `fileIcon` in app.js, one per entry of its `map`.
///
/// Items are drawn by style rather than by raw extension, so every file
/// type without a look of its own shares one cached texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocumentStyle {
    /// `pdf`.
    Pdf,
    /// `docx`, `doc`.
    Word,
    /// `xlsx`, `csv`.
    Spreadsheet,
    /// `pptx`.
    Presentation,
    /// `zip` (an item named `.zip` is drawn as a ZIP folder instead).
    Zip,
    /// `png`, `jpg`, `jpeg`, `webp`.
    Image,
    /// `mp4`.
    Video,
    /// `md`.
    Markdown,
    /// `txt`.
    Text,
    /// `py`.
    Python,
    /// `js`.
    JavaScript,
    /// Every other extension.
    Generic,
}

impl DocumentStyle {
    /// The style for a lower-cased extension from [`extension`].
    fn for_extension(extension: &str) -> Self {
        match extension {
            "pdf" => Self::Pdf,
            "docx" | "doc" => Self::Word,
            "xlsx" | "csv" => Self::Spreadsheet,
            "pptx" => Self::Presentation,
            "zip" => Self::Zip,
            "png" | "jpg" | "jpeg" | "webp" => Self::Image,
            "mp4" => Self::Video,
            "md" => Self::Markdown,
            "txt" => Self::Text,
            "py" => Self::Python,
            "js" => Self::JavaScript,
            _ => Self::Generic,
        }
    }
}

/// The art for a listed item: a folder (a network folder for a share in a
/// server listing), a ZIP folder for a ZIP file, else a document.
pub(crate) fn kind_for_entry(entry: &Entry) -> ArtKind {
    if entry.is_dir {
        return if entry.is_virtual {
            ArtKind::SharedFolder
        } else {
            ArtKind::Folder
        };
    }
    if is_zip(&entry.name, entry.content_type.as_deref()) {
        return ArtKind::ZipFolder;
    }
    let style = DocumentStyle::for_extension(&extension(&entry.name));
    ArtKind::Document(style)
}

/// `.zip` names and ZIP content types, as `isZipEntry` in app.js.
fn is_zip(name: &str, content_type: Option<&str>) -> bool {
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

/// Identifies one rasterised texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CacheKey {
    kind: ArtKind,
    /// The edge in device pixels.
    pixels: i32,
    /// The appearance, for art that [depends on the
    /// theme](ArtKind::depends_on_theme); `None` for the rest, which looks
    /// the same in both.
    appearance: Option<Appearance>,
}

thread_local! {
    /// Textures drawn so far, on the GTK main thread.
    static TEXTURES: RefCell<HashMap<CacheKey, gdk::Texture>> = RefCell::new(HashMap::new());
}

/// The art rasterised at `pixels` device pixels, cached. `None` only if the
/// SVG loader is missing.
///
/// The cache is keyed by [`ArtKind`], never by a file name or extension, so
/// it stays bounded: a few dozen kinds at the sizes and scales in use.
pub(crate) fn texture(kind: ArtKind, appearance: Appearance, pixels: i32) -> Option<gdk::Texture> {
    let key = CacheKey {
        kind,
        pixels,
        appearance: kind.depends_on_theme().then_some(appearance),
    };
    let cached = TEXTURES.with(|cache| cache.borrow().get(&key).cloned());
    if cached.is_some() {
        return cached;
    }
    let markup = svg::markup(kind, appearance, pixels);
    let bytes = glib::Bytes::from_owned(markup.into_bytes());
    let texture = match gdk::Texture::from_bytes(&bytes) {
        Ok(texture) => texture,
        Err(error) => {
            glib::g_warning!("openxplorer", "Could not draw icon art {kind:?}: {error}");
            return None;
        }
    };
    TEXTURES.with(|cache| cache.borrow_mut().insert(key, texture.clone()));
    Some(texture)
}

#[cfg(test)]
mod tests {
    use gtk::gdk::prelude::TextureExt;

    use super::*;
    use crate::test_support::{file_entry as file, folder_entry as folder};

    fn cached_textures() -> usize {
        TEXTURES.with(|cache| cache.borrow().len())
    }

    /// parity: ARC-001
    #[test]
    fn zip_detection_matches_app_js() {
        assert!(is_zip("Brand kit.ZIP", None));
        assert!(is_zip("download", Some("application/x-zip-compressed")));
        assert!(!is_zip("notes.zip.txt", Some("text/plain")));
    }

    /// parity: ARC-001
    #[test]
    fn a_folder_named_zip_keeps_the_plain_folder() {
        assert_eq!(kind_for_entry(&folder("Archive.zip")), ArtKind::Folder);
        assert_eq!(kind_for_entry(&file("Archive.zip")), ArtKind::ZipFolder);
    }

    #[test]
    fn extensions_are_lower_cased() {
        assert_eq!(extension("Quarterly report.DOCX"), "docx");
        assert_eq!(extension("archive.tar.gz"), "gz");
        assert_eq!(extension("README"), "readme");
    }

    /// parity: LOOK-015
    #[test]
    fn names_without_a_known_extension_share_one_kind() {
        assert_eq!(kind_for_entry(&file("ls")), kind_for_entry(&file("cargo")));
        assert_eq!(
            kind_for_entry(&file("ls")),
            ArtKind::Document(DocumentStyle::Generic)
        );
        assert_eq!(
            kind_for_entry(&file("Report.PDF")),
            ArtKind::Document(DocumentStyle::Pdf)
        );
    }

    #[test]
    fn unknown_file_types_share_one_cached_texture() {
        let first = texture(kind_for_entry(&file("ls")), Appearance::Light, 21);
        let size = cached_textures();
        let second = texture(kind_for_entry(&file("cargo")), Appearance::Light, 21);
        assert_eq!(cached_textures(), size, "a second unknown name adds no texture");
        assert_eq!(first, second);
    }

    /// parity: ARC-001, LOOK-015, LOOK-016
    #[test]
    fn every_kind_renders_through_the_svg_loader() {
        let kinds = [
            ArtKind::Folder,
            ArtKind::ZipFolder,
            ArtKind::SharedFolder,
            ArtKind::NetworkFolder,
            ArtKind::NetworkGlyph(Glyph::Server),
            ArtKind::Document(DocumentStyle::Pdf),
            ArtKind::Document(DocumentStyle::Image),
            ArtKind::Document(DocumentStyle::Video),
            ArtKind::Document(DocumentStyle::Markdown),
        ];
        for kind in kinds {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let texture = texture(kind, appearance, 48).expect("SVG art loads");
                assert_eq!((texture.width(), texture.height()), (48, 48), "{kind:?}");
            }
        }
    }
}
