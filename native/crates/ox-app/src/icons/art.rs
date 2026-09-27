// SPDX-License-Identifier: AGPL-3.0-only
//! Colour icon art: folders, ZIP folders, documents and network locations.
//!
//! Reproduces `appendFolderArt`, `folderIcon`, `zipFolderIcon`, `fileIcon`
//! and `networkIcon` in `desktop/ui/app.js` as SVG (48-unit viewBox), which
//! GDK rasterises through the gdk-pixbuf SVG loader. Colours that depend on
//! the theme (document paper, the network badge, glyphs on the network pipe)
//! are baked per [`Appearance`], and textures are cached per size.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write as _;

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

/// `.zip` names and ZIP content types, as `isZipEntry` in app.js.
pub fn is_zip(name: &str, content_type: Option<&str>) -> bool {
    let zip_name = name.to_lowercase().ends_with(".zip");
    let zip_type = matches!(
        content_type,
        Some("application/zip" | "application/x-zip" | "application/x-zip-compressed")
    );
    zip_name || zip_type
}

/// The art for a listed item.
pub fn kind_for_entry(entry: &Entry) -> ArtKind {
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
    ArtKind::Document(DocumentStyle::for_extension(&extension(&entry.name)))
}

/// The lower-cased text after the last dot (the whole name when there is
/// none, as `split('.').pop()` does in app.js).
pub fn extension(name: &str) -> String {
    name.rsplit('.').next().unwrap_or_default().to_lowercase()
}

/// How a document badge looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Badge {
    /// A coloured label with letters, such as `W` or `PDF`.
    Letter(&'static str),
    /// A small picture.
    Picture,
    /// Three text lines.
    Lines,
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
    pub fn for_extension(extension: &str) -> Self {
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

    /// Badge colour and look (the `map` in `fileIcon`).
    fn badge(self) -> (&'static str, Badge) {
        match self {
            Self::Pdf => ("#c84032", Badge::Letter("PDF")),
            Self::Word => ("#2868bd", Badge::Letter("W")),
            Self::Spreadsheet => ("#258150", Badge::Letter("X")),
            Self::Presentation => ("#ce6a35", Badge::Letter("P")),
            Self::Zip => ("#a8833d", Badge::Letter("ZIP")),
            Self::Image => ("#7d69bd", Badge::Picture),
            Self::Video => ("#975abe", Badge::Letter("▶")),
            Self::Markdown => ("#65747f", Badge::Lines),
            Self::Text => ("#748da8", Badge::Lines),
            Self::Python => ("#3d849e", Badge::Lines),
            Self::JavaScript => ("#d0a522", Badge::Lines),
            Self::Generic => ("#8092a3", Badge::Lines),
        }
    }
}

/// Theme colours used inside the art (`--doc-*`, `--bg`, `--accent` and
/// `--text` in style.css).
struct Palette {
    paper: &'static str,
    fold: &'static str,
    stroke: &'static str,
    background: &'static str,
    accent: &'static str,
    text: &'static str,
}

fn palette(appearance: Appearance) -> Palette {
    match appearance {
        Appearance::Light => Palette {
            paper: "#fafcfe",
            fold: "#e5ecf5",
            stroke: "#c9d3df",
            background: "#ffffff",
            accent: "#0067c0",
            text: "#242424",
        },
        Appearance::Dark => Palette {
            paper: "#dbe3ec",
            fold: "#b7c8dc",
            stroke: "#94a6bb",
            background: "#202020",
            accent: "#74beff",
            text: "#f1f1f1",
        },
    }
}

const FOLDER: &str = concat!(
    r##"<path d="M4 12a3 3 0 0 1 3-3h12l5 5h17a3 3 0 0 1 3 3v20a3 3 0 0 1-3 3H7a3 3 0 0 1-3-3Z" fill="#d99a22"/>"##,
    r##"<path d="M5 16h36v6H5z" fill="#fff0bd"/>"##,
    r##"<path d="M4 20h17l4-4h17a3 3 0 0 1 3 3l-2 19a3 3 0 0 1-3 3H7a3 3 0 0 1-3-3Z" fill="#ffce56"/>"##,
    r##"<path d="M4 25h40l-1 13a3 3 0 0 1-3 3H7a3 3 0 0 1-3-3Z" fill="#f7bd40"/>"##,
    r##"<path d="M6 21h15l4-4h16" fill="none" stroke="#fff0a9" stroke-width="1"/>"##,
);

/// The zipper drawn over the folder (`zipFolderIcon`).
fn zipper() -> String {
    let mut svg = String::from(r##"<rect x="28" y="15" width="7" height="25" rx="1" fill="#d59622"/>"##);
    for tooth in 0..6 {
        let x = if tooth % 2 == 1 { "31.5" } else { "28" };
        let y = 16 + tooth * 3;
        let _ = write!(
            svg,
            r##"<rect x="{x}" y="{y}" width="3.5" height="2.5" rx=".4" fill="#fff3c5"/>"##
        );
    }
    svg.push_str(r##"<rect x="27.5" y="32" width="8" height="8" rx="2" fill="#647789" stroke="#f8edce" stroke-width=".8"/>"##);
    svg.push_str(r##"<rect x="29.5" y="34" width="4" height="3.5" rx=".7" fill="#ffdb70"/>"##);
    svg
}

/// The network badge on a share in a server listing (`fileIcon`, isVirtual).
fn share_badge(colors: &Palette) -> String {
    format!(
        concat!(
            r##"<rect x="27" y="29" width="20" height="17" rx="3" fill="{bg}"/>"##,
            r##"<path d="M34 31h6v4h-6zM37 35v4m-6 0h12m-12 0v4h4v-4m4 0v4h4v-4" fill="none" "##,
            r##"stroke="{accent}" stroke-width="1.4" stroke-linejoin="round"/>"##
        ),
        bg = colors.background,
        accent = colors.accent
    )
}

/// The Windows-style green network pipe with its stem (`networkIcon`).
const PIPE: &str = concat!(
    r##"<g class="shared-bar">"##,
    r##"<rect x="21.5" y="30" width="5" height="9" fill="#23873f"/>"##,
    r##"<rect x="22.5" y="30" width="1.6" height="9" fill="#6fd989"/>"##,
    r##"<rect x="2" y="38" width="44" height="8" rx="2.5" fill="#23873f"/>"##,
    r##"<rect x="3" y="39" width="42" height="3" rx="1.5" fill="#62cf7c"/>"##,
    r##"<rect x="3" y="42" width="42" height="3" rx="1.5" fill="#35a854"/>"##,
    "</g>"
);

fn network_folder() -> String {
    format!(r#"<g transform="translate(3.84 -4.1) scale(.84)">{FOLDER}</g>{PIPE}"#)
}

fn network_glyph(glyph: Glyph, colors: &Palette) -> String {
    format!(
        concat!(
            r#"<svg x="8" y="0" width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="{color}" "#,
            r#"stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="{data}"/></svg>{pipe}"#
        ),
        color = colors.text,
        data = glyph.path_data(),
        pipe = PIPE
    )
}

fn document(style: DocumentStyle, colors: &Palette) -> String {
    let (color, badge) = style.badge();
    let mut svg = format!(
        concat!(
            r#"<path d="M11 3h19l9 9v31a2 2 0 0 1-2 2H11a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Z" "#,
            r#"fill="{paper}" stroke="{stroke}" stroke-width="1.2"/>"#,
            r#"<path d="M30 3v9h9" fill="{fold}" stroke="{stroke}" stroke-width="1.2"/>"#
        ),
        paper = colors.paper,
        fold = colors.fold,
        stroke = colors.stroke
    );
    svg.push_str(&badge_svg(badge, color));
    svg
}

fn badge_svg(badge: Badge, color: &str) -> String {
    match badge {
        Badge::Letter(letter) => {
            let font_size = if letter.chars().count() > 1 { 10 } else { 14 };
            format!(
                concat!(
                    r#"<rect x="4" y="18" width="30" height="19" rx="2" fill="{color}"/>"#,
                    r#"<text x="19" y="31" font-size="{font_size}" text-anchor="middle" fill="white" "#,
                    r#"font-family="sans-serif" font-weight="600">{letter}</text>"#
                ),
                color = color,
                font_size = font_size,
                letter = letter
            )
        }
        Badge::Picture => format!(
            concat!(
                r##"<path d="M14 36V20h20v16Z" fill="#e3dcf7"/>"##,
                r##"<path d="m14 34 7-8 5 5 4-4 4 7Z" fill="{color}"/>"##,
                r##"<circle cx="29" cy="23" r="2" fill="#f0b253"/>"##
            ),
            color = color
        ),
        Badge::Lines => {
            format!(r#"<path d="M15 22h17M15 27h17M15 32h12" stroke="{color}" stroke-width="2"/>"#)
        }
    }
}

/// A complete SVG document for `kind` at `pixels` × `pixels`.
pub fn svg(kind: ArtKind, appearance: Appearance, pixels: i32) -> String {
    let colors = palette(appearance);
    let body = match kind {
        ArtKind::Folder => FOLDER.to_string(),
        ArtKind::ZipFolder => format!("{FOLDER}{}", zipper()),
        ArtKind::SharedFolder => format!("{FOLDER}{}", share_badge(&colors)),
        ArtKind::NetworkFolder => network_folder(),
        ArtKind::NetworkGlyph(glyph) => network_glyph(glyph, &colors),
        ArtKind::Document(style) => document(style, &colors),
    };
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48" width="{pixels}" height="{pixels}">{body}</svg>"#
    )
}

/// True when the art uses theme colours and must be cached per theme.
fn depends_on_theme(kind: ArtKind) -> bool {
    matches!(
        kind,
        ArtKind::Document(_) | ArtKind::SharedFolder | ArtKind::NetworkGlyph(_)
    )
}

/// Art kind, device pixels and, for theme-dependent art, the appearance.
type CacheKey = (ArtKind, i32, Option<Appearance>);

thread_local! {
    static TEXTURES: RefCell<HashMap<CacheKey, gdk::Texture>> = RefCell::new(HashMap::new());
}

/// The art rasterised at `pixels` device pixels, cached. `None` only if the
/// SVG loader is missing.
///
/// The cache is keyed by [`ArtKind`], never by a file name or extension, so
/// it stays bounded: a few dozen kinds at the sizes and scales in use.
pub fn texture(kind: ArtKind, appearance: Appearance, pixels: i32) -> Option<gdk::Texture> {
    let theme_key = depends_on_theme(kind).then_some(appearance);
    let key = (kind, pixels, theme_key);
    if let Some(found) = TEXTURES.with(|cache| cache.borrow().get(&key).cloned()) {
        return Some(found);
    }
    let document = svg(kind, appearance, pixels);
    let bytes = glib::Bytes::from_owned(document.into_bytes());
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
fn cached_textures() -> usize {
    TEXTURES.with(|cache| cache.borrow().len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::file_entry as file;
    use gtk::gdk::prelude::TextureExt;

    #[test]
    fn zip_detection_matches_app_js() {
        assert!(is_zip("Brand kit.ZIP", None));
        assert!(is_zip("download", Some("application/x-zip-compressed")));
        assert!(!is_zip("notes.zip.txt", Some("text/plain")));
    }

    #[test]
    fn extensions_are_lower_cased() {
        assert_eq!(extension("Quarterly report.DOCX"), "docx");
        assert_eq!(extension("archive.tar.gz"), "gz");
        assert_eq!(extension("README"), "readme");
    }

    #[test]
    fn file_badges_follow_the_extension_map() {
        assert_eq!(
            DocumentStyle::for_extension("docx").badge(),
            ("#2868bd", Badge::Letter("W"))
        );
        assert_eq!(DocumentStyle::for_extension("jpeg").badge().1, Badge::Picture);
        assert_eq!(
            DocumentStyle::for_extension("unknown").badge(),
            ("#8092a3", Badge::Lines)
        );
    }

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

    #[test]
    fn documents_use_the_theme_paper() {
        let text = ArtKind::Document(DocumentStyle::Text);
        assert!(svg(text, Appearance::Dark, 24).contains("#dbe3ec"));
        assert!(svg(text, Appearance::Light, 24).contains("#fafcfe"));
    }
}
