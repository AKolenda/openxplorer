// SPDX-License-Identifier: AGPL-3.0-only
//! Icons from Windows icon files in a window (LOOK-030): every place a
//! standard folder, Network or the Recycle Bin appears takes the file's
//! picture. The files are synthetic, built in the test, so the pictures
//! are test patterns, never Windows icons.

use std::path::{Path, PathBuf};

use gtk::prelude::*;
use ox_core::location::file_uri;
use ox_core::places::{FolderLocations, KnownFolder};
use ox_core::settings::PreferencesUpdate;
use ox_core::windows_icons::synthetic::SyntheticIconFile;
use ox_core::windows_icons::{IconLibrary, IconRole};

use crate::icons::{user_icons, Art, ArtImage, Icon, Tint};
use crate::test_support::harness::{descendants, wait_until, TestWindow};

/// A home folder with the six standard folders, and the synthetic icon
/// files beside it.
struct TestHome {
    _root: tempfile::TempDir,
    home: PathBuf,
    config: PathBuf,
    files: Vec<String>,
}

impl TestHome {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("a temporary folder");
        let home = root.path().join("home");
        for folder in ["Desktop", "Downloads", "Documents", "Pictures", "Music", "Videos"] {
            std::fs::create_dir_all(home.join(folder)).expect("a standard folder");
        }
        let config = root.path().join("config");
        std::fs::create_dir_all(&config).expect("a config folder");
        let files = [
            ("imageres.dll.mun", IconLibrary::Imageres),
            ("shell32.dll.mun", IconLibrary::Shell32),
        ]
        .into_iter()
        .map(|(name, library)| write_library(root.path(), name, library))
        .collect();
        Self {
            _root: root,
            home,
            config,
            files,
        }
    }
}

/// Writes a synthetic file called `name` with every group of `library`.
fn write_library(folder: &Path, name: &str, library: IconLibrary) -> String {
    let mut groups: Vec<u16> = IconRole::ALL
        .into_iter()
        .map(IconRole::source)
        .filter(|(source, _, _)| *source == library)
        .map(|(_, group, _)| group)
        .collect();
    groups.sort_unstable();
    groups.dedup();
    let path = folder.join(name);
    std::fs::write(&path, SyntheticIconFile::with_groups(&groups).build()).expect("written");
    path.display().to_string()
}

/// The art image of `window` showing `art`, once there is one.
fn art_image(window: &impl IsA<gtk::Widget>, art: Art, what: &str) -> ArtImage {
    wait_until(what, || {
        descendants::<ArtImage>(window)
            .into_iter()
            .any(|image| image.art() == Some(art) && image.is_mapped())
    });
    descendants::<ArtImage>(window)
        .into_iter()
        .find(|image| image.art() == Some(art) && image.is_mapped())
        .expect("found above")
}

/// With the files chosen, the navigation pane's Network and Recycle Bin,
/// a standard folder listed in the home folder, the address bar and
/// Home's Quick access tiles all show the files' pictures; turned off,
/// the app's own icons come back.
///
/// parity: LOOK-030
#[gtk::test]
fn every_place_of_a_window_takes_the_files_pictures() {
    let home = TestHome::new();
    let folders = FolderLocations::new(home.home.clone(), &home.config);
    let test = TestWindow::open_with_standard_folders(&file_uri(&home.home), folders, |_| {});
    test.context.update_preferences(
        PreferencesUpdate {
            use_icon_files: Some(true),
            icon_files: Some(home.files.clone()),
            // Home opens Quick access, whose tiles are checked below.
            home_quick_access: Some(true),
            ..PreferencesUpdate::default()
        },
        |result| result.expect("saved"),
    );
    wait_until("the files to be read", || {
        user_icons::status()
            == user_icons::Status::Ready {
                found: IconRole::ALL.len(),
            }
    });

    let network = art_image(
        &test.window,
        Art::TintedGlyph(Icon::Organization, Tint::Network),
        "the navigation pane's Network",
    );
    // Empty, or full when earlier tests in the process left items in it.
    let is_bin = |image: &ArtImage| {
        matches!(
            image.art(),
            Some(Art::Glyph(Icon::Delete) | Art::TintedGlyph(Icon::Delete, _))
        )
    };
    wait_until("the navigation pane's Recycle Bin", || {
        descendants::<ArtImage>(&test.window).iter().any(is_bin)
    });
    let bin = descendants::<ArtImage>(&test.window)
        .into_iter()
        .find(is_bin)
        .expect("found above");
    let documents = art_image(
        &test.window,
        Art::StandardFolder(KnownFolder::Documents),
        "Documents listed in the home folder",
    );
    for (image, what) in [
        (&network, "Network"),
        (&bin, "Recycle Bin"),
        (&documents, "Documents"),
    ] {
        wait_until(&format!("{what} to show the file's picture"), || {
            image.shows_user_icon()
        });
    }
    let address = test.window.address_bar().location_icon();
    wait_until("the address bar to show Home's picture", || {
        address.icon_name().is_none() && address.paintable().is_some()
    });

    test.window
        .add_tab(crate::locations::Page::Home.uri())
        .expect("the Home page");
    let tile = art_image(
        &test.window,
        Art::StandardFolder(KnownFolder::Music),
        "Music's Quick access tile",
    );
    wait_until("the tile to show the file's picture", || tile.shows_user_icon());
    let glyph_shows = || {
        descendants::<ArtImage>(&test.window)
            .into_iter()
            .filter(|image| image.has_css_class("home-emblem"))
            .any(|image| image.is_visible())
    };
    assert!(!glyph_shows(), "the folder's glyph steps aside for the tile");

    test.context.update_preferences(
        PreferencesUpdate {
            use_icon_files: Some(false),
            ..PreferencesUpdate::default()
        },
        |result| result.expect("saved"),
    );
    wait_until("the app's own icons to return", || {
        !network.shows_user_icon() && !tile.shows_user_icon() && glyph_shows()
    });
}

/// With Windows icon files on and a symbol font chosen, the window's
/// glyphs come from the font: the command bar's New, the Back button and
/// a tab's close button; a glyph the font lacks keeps the app's own; and
/// turning it off brings every bundled glyph back. The font is synthetic,
/// built in the test.
///
/// parity: LOOK-031
#[gtk::test]
fn the_windows_glyphs_come_from_the_chosen_symbol_font() {
    use ox_core::windows_glyphs::synthetic::SyntheticFont;

    use crate::icons::user_glyphs;

    let home = TestHome::new();
    let font = home.home.parent().expect("the root").join("SegoeIcons.ttf");
    // Add, AddTo (New's circled plus), Back and Cancel (the tab's close
    // button); Cut is left out.
    let codepoints = [0xE710, 0xECC8, 0xE72B, 0xE711];
    std::fs::write(&font, SyntheticFont::with_codepoints(&codepoints).build()).expect("written");
    let folders = FolderLocations::new(home.home.clone(), &home.config);
    let test = TestWindow::open_with_standard_folders(&file_uri(&home.home), folders, |_| {});
    test.context.update_preferences(
        PreferencesUpdate {
            use_icon_files: Some(true),
            symbol_font: Some(font.display().to_string()),
            ..PreferencesUpdate::default()
        },
        |result| result.expect("saved"),
    );
    wait_until("the font to be read", || {
        // Cancel (E711) draws both X glyphs.
        matches!(user_glyphs::status(), user_glyphs::Status::Ready { found: 4, .. })
    });
    let shown = |name: &str| {
        descendants::<gtk::Image>(&test.window)
            .iter()
            .any(|image| image.icon_name().as_deref() == Some(name))
    };
    for glyph in [
        // New's circled plus in its 18-pixel square, Back at its 12-pixel
        // em in its 16-pixel square, and the tab's close at its 12.
        "ox-win-add-20-box18-symbolic",
        "ox-win-arrow-left-20-box16-symbolic",
        "ox-win-dismiss-16-symbolic",
    ] {
        wait_until(&format!("{glyph} on screen"), || shown(glyph));
    }
    assert!(shown(Icon::Cut.name()), "Cut keeps the app's glyph");
    assert!(!shown(Icon::Add.name()), "New uses the font's");

    test.context.update_preferences(
        PreferencesUpdate {
            use_icon_files: Some(false),
            ..PreferencesUpdate::default()
        },
        |result| result.expect("saved"),
    );
    wait_until("the app's glyphs to return", || {
        shown(Icon::Add.name())
            && !descendants::<gtk::Image>(&test.window)
                .iter()
                .any(|image| image.icon_name().is_some_and(|name| name.starts_with("ox-win-")))
    });
}
