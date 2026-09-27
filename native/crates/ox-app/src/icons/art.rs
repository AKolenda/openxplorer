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

use crate::icons::glyphs;
use crate::theme::Appearance;

/// Which piece of art to draw.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ArtKind {
    /// The yellow folder.
    Folder,
    /// A folder with a zipper, for ZIP archives.
    ZipFolder,
    /// A network share in a server listing: a folder with a network badge.
    SharedFolder,
    /// A folder on the green network pipe (a saved network location).
    NetworkFolder,
    /// A stroke glyph (for example `server`) on the green network pipe.
    NetworkGlyph(&'static str),
    /// A document; the lower-cased extension picks colour and badge.
    File(String),
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
    ArtKind::File(extension(&entry.name))
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

/// Badge colour and style for an extension (the `map` in `fileIcon`).
fn file_style(ext: &str) -> (&'static str, Badge) {
    match ext {
        "pdf" => ("#c84032", Badge::Letter("PDF")),
        "docx" | "doc" => ("#2868bd", Badge::Letter("W")),
        "xlsx" | "csv" => ("#258150", Badge::Letter("X")),
        "pptx" => ("#ce6a35", Badge::Letter("P")),
        "zip" => ("#a8833d", Badge::Letter("ZIP")),
        "png" | "jpg" | "jpeg" | "webp" => ("#7d69bd", Badge::Picture),
        "mp4" => ("#975abe", Badge::Letter("▶")),
        "md" => ("#65747f", Badge::Lines),
        "txt" => ("#748da8", Badge::Lines),
        "py" => ("#3d849e", Badge::Lines),
        "js" => ("#d0a522", Badge::Lines),
        _ => ("#8092a3", Badge::Lines),
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

fn network_glyph(glyph: &str, colors: &Palette) -> String {
    format!(
        concat!(
            r#"<svg x="8" y="0" width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="{color}" "#,
            r#"stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="{data}"/></svg>{pipe}"#
        ),
        color = colors.text,
        data = glyphs::path_data(glyph),
        pipe = PIPE
    )
}

fn document(ext: &str, colors: &Palette) -> String {
    let (color, badge) = file_style(ext);
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
    match badge {
        Badge::Letter(letter) => {
            let font_size = if letter.chars().count() > 1 { 10 } else { 14 };
            let _ = write!(
                svg,
                concat!(
                    r#"<rect x="4" y="18" width="30" height="19" rx="2" fill="{color}"/>"#,
                    r#"<text x="19" y="31" font-size="{size}" text-anchor="middle" fill="white" "#,
                    r#"font-family="sans-serif" font-weight="600">{letter}</text>"#
                ),
                color = color,
                size = font_size,
                letter = letter
            );
        }
        Badge::Picture => {
            let _ = write!(
                svg,
                concat!(
                    r##"<path d="M14 36V20h20v16Z" fill="#e3dcf7"/>"##,
                    r##"<path d="m14 34 7-8 5 5 4-4 4 7Z" fill="{color}"/>"##,
                    r##"<circle cx="29" cy="23" r="2" fill="#f0b253"/>"##
                ),
                color = color
            );
        }
        Badge::Lines => {
            let _ = write!(
                svg,
                r#"<path d="M15 22h17M15 27h17M15 32h12" stroke="{color}" stroke-width="2"/>"#
            );
        }
    }
    svg
}

/// A complete SVG document for `kind` at `pixels` × `pixels`.
pub fn svg(kind: &ArtKind, appearance: Appearance, pixels: i32) -> String {
    let colors = palette(appearance);
    let body = match kind {
        ArtKind::Folder => FOLDER.to_string(),
        ArtKind::ZipFolder => format!("{FOLDER}{}", zipper()),
        ArtKind::SharedFolder => format!("{FOLDER}{}", share_badge(&colors)),
        ArtKind::NetworkFolder => network_folder(),
        ArtKind::NetworkGlyph(glyph) => network_glyph(glyph, &colors),
        ArtKind::File(ext) => document(ext, &colors),
    };
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48" width="{pixels}" height="{pixels}">{body}</svg>"#
    )
}

/// True when the art uses theme colours and must be cached per theme.
fn depends_on_theme(kind: &ArtKind) -> bool {
    matches!(
        kind,
        ArtKind::File(_) | ArtKind::SharedFolder | ArtKind::NetworkGlyph(_)
    )
}

type CacheKey = (ArtKind, i32, Option<Appearance>);

thread_local! {
    static TEXTURES: RefCell<HashMap<CacheKey, gdk::Texture>> = RefCell::new(HashMap::new());
}

/// The art rasterised at `pixels` device pixels, cached. `None` only if the
/// SVG loader is missing.
pub fn texture(kind: &ArtKind, appearance: Appearance, pixels: i32) -> Option<gdk::Texture> {
    let theme_key = depends_on_theme(kind).then_some(appearance);
    let key = (kind.clone(), pixels, theme_key);
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
mod tests {
    use super::*;
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
        assert_eq!(file_style("docx"), ("#2868bd", Badge::Letter("W")));
        assert_eq!(file_style("jpeg").1, Badge::Picture);
        assert_eq!(file_style("unknown"), ("#8092a3", Badge::Lines));
    }

    #[test]
    fn every_kind_renders_through_the_svg_loader() {
        let kinds = [
            ArtKind::Folder,
            ArtKind::ZipFolder,
            ArtKind::SharedFolder,
            ArtKind::NetworkFolder,
            ArtKind::NetworkGlyph("server"),
            ArtKind::File("pdf".into()),
            ArtKind::File("png".into()),
            ArtKind::File("mp4".into()),
            ArtKind::File("md".into()),
        ];
        for kind in kinds {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let texture = texture(&kind, appearance, 48).expect("SVG art loads");
                assert_eq!((texture.width(), texture.height()), (48, 48), "{kind:?}");
            }
        }
    }

    #[test]
    fn documents_use_the_theme_paper() {
        assert!(svg(&ArtKind::File("txt".into()), Appearance::Dark, 24).contains("#dbe3ec"));
        assert!(svg(&ArtKind::File("txt".into()), Appearance::Light, 24).contains("#fafcfe"));
    }
}
