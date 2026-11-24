// SPDX-License-Identifier: AGPL-3.0-only
//! Tests of glyphs from a Windows symbol font, on a synthetic font built
//! in the tests ([`ox_core::windows_glyphs::synthetic`]): never a
//! Microsoft font.

use gtk::prelude::*;
use ox_core::settings::Preferences;
use ox_core::windows_glyphs::synthetic::SyntheticFont;

use super::*;

/// Turns the glyphs off again when a test ends, also when it fails.
struct Reset;

impl Drop for Reset {
    fn drop(&mut self) {
        follow_preferences(&Preferences::default());
    }
}

/// Preferences with Windows icon files on and `font` chosen.
fn with_font(font: &std::path::Path) -> Preferences {
    Preferences {
        use_icon_files: true,
        symbol_font: Some(font.display().to_string()),
        ..Preferences::default()
    }
}

/// The rows and columns of `texture` any ink covers: its inked bounding
/// box, `None` when nothing is drawn.
fn inked_box(texture: &gtk::gdk::Texture) -> Option<(i32, i32, i32, i32)> {
    let downloader = gtk::gdk::TextureDownloader::new(texture);
    let (bytes, stride) = downloader.download_bytes();
    let (width, height) = (texture.width(), texture.height());
    let mut found: Option<(i32, i32, i32, i32)> = None;
    for y in 0..height {
        for x in 0..width {
            let alpha = bytes
                [usize::try_from(y).expect("a row") * stride + usize::try_from(x).expect("a column") * 4 + 3];
            if alpha > 127 {
                found = Some(match found {
                    None => (x, y, x, y),
                    Some((left, top, right, bottom)) => {
                        (left.min(x), top.min(y), right.max(x), bottom.max(y))
                    }
                });
            }
        }
    }
    found
}

/// The inked box of the icon `name` as the icon theme of `image`'s
/// display paints it `size` pixels square.
fn painted_box(image: &gtk::Image, name: &str, size: i32) -> Option<(i32, i32, i32, i32)> {
    let theme = gtk::IconTheme::for_display(&image.display());
    let icon = theme.lookup_icon(
        name,
        &[],
        size,
        1,
        gtk::TextDirection::Ltr,
        gtk::IconLookupFlags::empty(),
    );
    let renderer = gtk::gsk::CairoRenderer::new();
    renderer
        .realize_for_display(&image.display())
        .expect("the Cairo renderer needs no GPU");
    let snapshot = gtk::Snapshot::new();
    let edge = f64::from(size);
    icon.snapshot(&snapshot, edge, edge);
    let node = snapshot.to_node()?;
    #[expect(clippy::cast_precision_loss, reason = "a small icon")]
    let bounds = gtk::graphene::Rect::new(0.0, 0.0, size as f32, size as f32);
    let texture = renderer.render_texture(node, Some(&bounds));
    renderer.unrealize();
    inked_box(&texture)
}

/// Chevrons are drawn at the ems that give Windows 11's ink at 1x with the
/// real font: 13 in a 16-pixel square (a submenu's arrow, 5 by 10), 11 in
/// a 12- or 13-pixel one (a breadcrumb separator, 5 by 8; Windows' sort
/// chevron) and 8 elsewhere; a drop-down button's caret in
/// the command bar's 17-pixel square inks 8 whole pixels, not the 16 of
/// the glyphs beside it, and not 9 blurred across a half pixel.
///
/// parity: LOOK-031
#[gtk::test]
fn chevrons_are_drawn_at_windows_ems() {
    use ox_core::windows_glyphs::synthetic::SyntheticGlyph;

    for (size, em) in [(12, 11), (13, 11), (16, 13), (17, 8), (20, 8), (10, 8), (8, 8)] {
        assert_eq!(em_of(Icon::ChevronRight16, size), em, "{size}");
    }
    for size in [12, 17] {
        assert_eq!(em_of(Icon::ChevronDown, size), 8, "the caret in {size}");
    }
    assert_eq!(em_of(Icon::Cut, 17), 16);
    let _reset = Reset;
    let folder = tempfile::tempdir().expect("a temporary folder");
    let font_path = folder.path().join("SegoeIcons.ttf");
    let font = SyntheticFont {
        glyphs: vec![SyntheticGlyph::Simple(vec![vec![
            (0, 0, true),
            (1024, 0, true),
            (1024, 1024, true),
            (0, 1024, true),
        ]])],
        mapping: vec![(0xE70D, 1)],
        format_12: false,
    };
    std::fs::write(&font_path, font.build()).expect("written");
    follow_preferences(&with_font(&font_path));
    wait_until_read();
    let image = crate::icons::image(Icon::ChevronDown, 17);
    let name = image.icon_name().expect("a name").to_string();
    assert_eq!(name, "ox-win-chevron-down-20-box17-symbolic");
    assert_eq!(
        painted_box(&image, &name, 17),
        Some((4, 4, 11, 11)),
        "an 8-pixel em on whole pixels in 17"
    );
}
