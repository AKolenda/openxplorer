// SPDX-License-Identifier: AGPL-3.0-only
//! The colour art as SVG markup (48-unit viewBox).
//!
//! The drawings are those of `appendFolderArt`, `zipFolderIcon`,
//! `fileIcon` and `networkIcon` in `desktop/ui/app.js`. Colours that
//! depend on the theme (document paper, the network badge, glyphs on the
//! network pipe) come from the [`Palette`] of the appearance; every other
//! colour is the web app's own.

use crate::icons::Glyph;
use crate::theme::Appearance;

use super::{ArtKind, DocumentStyle};

/// A complete SVG document for `kind` at `pixels` × `pixels`.
pub(super) fn markup(kind: ArtKind, appearance: Appearance, pixels: i32) -> String {
    let palette = Palette::of(appearance);
    let body = match kind {
        ArtKind::Folder => FOLDER.to_owned(),
        ArtKind::ZipFolder => format!("{FOLDER}{}", zipper()),
        ArtKind::SharedFolder => format!("{FOLDER}{}", share_badge(palette)),
        ArtKind::NetworkFolder => network_folder(),
        ArtKind::NetworkGlyph(glyph) => network_glyph(glyph, palette),
        ArtKind::Document(style) => document(style, palette),
    };
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48" width="{pixels}" height="{pixels}">{body}</svg>"#
    )
}

/// Theme colours used inside the art (`--doc-*`, `--bg`, `--accent` and
/// `--text` in style.css).
#[derive(Debug)]
struct Palette {
    /// A document's page (`--doc-paper`).
    paper: &'static str,
    /// A document's folded corner (`--doc-fold`).
    fold: &'static str,
    /// A document's outline (`--doc-stroke`).
    stroke: &'static str,
    /// The page background behind the share badge (`--bg`).
    background: &'static str,
    /// The share badge's glyph (`--accent`).
    accent: &'static str,
    /// A glyph on the network pipe (`--text`).
    text: &'static str,
}

impl Palette {
    const LIGHT: Palette = Palette {
        paper: "#fafcfe",
        fold: "#e5ecf5",
        stroke: "#c9d3df",
        background: "#ffffff",
        accent: "#0067c0",
        text: "#242424",
    };

    const DARK: Palette = Palette {
        paper: "#dbe3ec",
        fold: "#b7c8dc",
        stroke: "#94a6bb",
        background: "#202020",
        accent: "#74beff",
        text: "#f1f1f1",
    };

    /// The palette that draws `appearance`.
    const fn of(appearance: Appearance) -> &'static Palette {
        match appearance {
            Appearance::Light => &Self::LIGHT,
            Appearance::Dark => &Self::DARK,
        }
    }
}

/// The layered yellow folder (`appendFolderArt`).
const FOLDER: &str = concat!(
    r##"<path d="M4 12a3 3 0 0 1 3-3h12l5 5h17a3 3 0 0 1 3 3v20a3 3 0 0 1-3 3H7a3 3 0 0 1-3-3Z" fill="#d99a22"/>"##,
    r##"<path d="M5 16h36v6H5z" fill="#fff0bd"/>"##,
    r##"<path d="M4 20h17l4-4h17a3 3 0 0 1 3 3l-2 19a3 3 0 0 1-3 3H7a3 3 0 0 1-3-3Z" fill="#ffce56"/>"##,
    r##"<path d="M4 25h40l-1 13a3 3 0 0 1-3 3H7a3 3 0 0 1-3-3Z" fill="#f7bd40"/>"##,
    r##"<path d="M6 21h15l4-4h16" fill="none" stroke="#fff0a9" stroke-width="1"/>"##,
);

/// The zipper's tape, under its teeth.
const ZIPPER_TAPE: &str = r##"<rect x="28" y="15" width="7" height="25" rx="1" fill="#d59622"/>"##;

/// The zipper's pull and lock, over its teeth.
const ZIPPER_PULL: &str = concat!(
    r##"<rect x="27.5" y="32" width="8" height="8" rx="2" fill="#647789" stroke="#f8edce" stroke-width=".8"/>"##,
    r##"<rect x="29.5" y="34" width="4" height="3.5" rx=".7" fill="#ffdb70"/>"##,
);

/// How many teeth the zipper has.
const ZIPPER_TEETH: u32 = 6;

/// The zipper drawn over the folder (`zipFolderIcon`).
fn zipper() -> String {
    let teeth: String = (0..ZIPPER_TEETH).map(zipper_tooth).collect();
    format!("{ZIPPER_TAPE}{teeth}{ZIPPER_PULL}")
}

/// Tooth `index` of the zipper: teeth alternate between the tape's left
/// and right halves, 3 units apart.
fn zipper_tooth(index: u32) -> String {
    let x = if index % 2 == 1 { "31.5" } else { "28" };
    let y = 16 + index * 3;
    format!(r##"<rect x="{x}" y="{y}" width="3.5" height="2.5" rx=".4" fill="#fff3c5"/>"##)
}

/// The network badge on a share in a server listing (`fileIcon`, isVirtual).
fn share_badge(palette: &Palette) -> String {
    format!(
        concat!(
            r##"<rect x="27" y="29" width="20" height="17" rx="3" fill="{background}"/>"##,
            r##"<path d="M34 31h6v4h-6zM37 35v4m-6 0h12m-12 0v4h4v-4m4 0v4h4v-4" fill="none" "##,
            r##"stroke="{accent}" stroke-width="1.4" stroke-linejoin="round"/>"##
        ),
        background = palette.background,
        accent = palette.accent
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

/// A smaller folder standing on the network pipe.
fn network_folder() -> String {
    format!(r#"<g transform="translate(3.84 -4.1) scale(.84)">{FOLDER}</g>{PIPE}"#)
}

/// `glyph`, drawn in the text colour, standing on the network pipe.
fn network_glyph(glyph: Glyph, palette: &Palette) -> String {
    format!(
        concat!(
            r#"<svg x="8" y="0" width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="{color}" "#,
            r#"stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="{data}"/></svg>{pipe}"#
        ),
        color = palette.text,
        data = glyph.path_data(),
        pipe = PIPE
    )
}

/// A page with a folded corner and the badge of `style` (`fileIcon`).
fn document(style: DocumentStyle, palette: &Palette) -> String {
    let page = format!(
        concat!(
            r#"<path d="M11 3h19l9 9v31a2 2 0 0 1-2 2H11a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Z" "#,
            r#"fill="{paper}" stroke="{stroke}" stroke-width="1.2"/>"#,
            r#"<path d="M30 3v9h9" fill="{fold}" stroke="{stroke}" stroke-width="1.2"/>"#
        ),
        paper = palette.paper,
        fold = palette.fold,
        stroke = palette.stroke
    );
    let badge = badge_markup(badge(style));
    format!("{page}{badge}")
}

/// A document badge: its colour and what it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Badge {
    /// The badge's colour, as in app.js.
    color: &'static str,
    /// What the badge shows.
    mark: BadgeMark,
}

/// What a document badge shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BadgeMark {
    /// A coloured label with letters, such as `W` or `PDF`.
    Letters(&'static str),
    /// A small picture.
    Picture,
    /// Three text lines.
    Lines,
}

/// The badge of each document style (the `map` in `fileIcon`).
const fn badge(style: DocumentStyle) -> Badge {
    let (color, mark) = match style {
        DocumentStyle::Pdf => ("#c84032", BadgeMark::Letters("PDF")),
        DocumentStyle::Word => ("#2868bd", BadgeMark::Letters("W")),
        DocumentStyle::Spreadsheet => ("#258150", BadgeMark::Letters("X")),
        DocumentStyle::Presentation => ("#ce6a35", BadgeMark::Letters("P")),
        DocumentStyle::Zip => ("#a8833d", BadgeMark::Letters("ZIP")),
        DocumentStyle::Image => ("#7d69bd", BadgeMark::Picture),
        DocumentStyle::Video => ("#975abe", BadgeMark::Letters("▶")),
        DocumentStyle::Markdown => ("#65747f", BadgeMark::Lines),
        DocumentStyle::Text => ("#748da8", BadgeMark::Lines),
        DocumentStyle::Python => ("#3d849e", BadgeMark::Lines),
        DocumentStyle::JavaScript => ("#d0a522", BadgeMark::Lines),
        DocumentStyle::Generic => ("#8092a3", BadgeMark::Lines),
    };
    Badge { color, mark }
}

/// The markup of `badge`, drawn over the page.
fn badge_markup(badge: Badge) -> String {
    match badge.mark {
        BadgeMark::Letters(letters) => letters_badge(badge.color, letters),
        BadgeMark::Picture => picture_badge(badge.color),
        BadgeMark::Lines => lines_badge(badge.color),
    }
}

/// A `color` label with white `letters`, smaller when there are several.
fn letters_badge(color: &str, letters: &str) -> String {
    let font_size = if letters.chars().count() > 1 { 10 } else { 14 };
    format!(
        concat!(
            r#"<rect x="4" y="18" width="30" height="19" rx="2" fill="{color}"/>"#,
            r#"<text x="19" y="31" font-size="{font_size}" text-anchor="middle" fill="white" "#,
            r#"font-family="sans-serif" font-weight="600">{letters}</text>"#
        ),
        color = color,
        font_size = font_size,
        letters = letters
    )
}

/// A small landscape picture with `color` hills.
fn picture_badge(color: &str) -> String {
    format!(
        concat!(
            r##"<path d="M14 36V20h20v16Z" fill="#e3dcf7"/>"##,
            r##"<path d="m14 34 7-8 5 5 4-4 4 7Z" fill="{color}"/>"##,
            r##"<circle cx="29" cy="23" r="2" fill="#f0b253"/>"##
        ),
        color = color
    )
}

/// Three text lines in `color`.
fn lines_badge(color: &str) -> String {
    format!(r#"<path d="M15 22h17M15 27h17M15 32h12" stroke="{color}" stroke-width="2"/>"#)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: LOOK-015
    #[test]
    fn file_badges_follow_the_extension_map() {
        assert_eq!(
            badge(DocumentStyle::for_extension("docx")),
            Badge {
                color: "#2868bd",
                mark: BadgeMark::Letters("W")
            }
        );
        assert_eq!(
            badge(DocumentStyle::for_extension("jpeg")).mark,
            BadgeMark::Picture
        );
        assert_eq!(
            badge(DocumentStyle::for_extension("unknown")),
            Badge {
                color: "#8092a3",
                mark: BadgeMark::Lines
            }
        );
    }

    /// parity: LOOK-015
    #[test]
    fn documents_use_the_theme_paper() {
        let text = ArtKind::Document(DocumentStyle::Text);
        assert!(markup(text, Appearance::Dark, 24).contains("#dbe3ec"));
        assert!(markup(text, Appearance::Light, 24).contains("#fafcfe"));
    }

    /// parity: ARC-001
    #[test]
    fn the_zipper_has_six_alternating_teeth() {
        let zipper = zipper();
        assert_eq!(zipper.matches(r##"fill="#fff3c5""##).count(), 6);
        assert!(zipper.contains(r#"<rect x="28" y="16" "#));
        assert!(zipper.contains(r#"<rect x="31.5" y="31" "#));
    }
}
