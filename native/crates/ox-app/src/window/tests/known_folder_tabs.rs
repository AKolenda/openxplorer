// SPDX-License-Identifier: AGPL-3.0-only
//! The tab of a standard folder (Desktop, Downloads, Documents, Pictures,
//! Music, Videos) shows the folder's navigation pane glyph, as Windows 11
//! File Explorer's tabs do (TAB-010); the home folder shows Home's, and a
//! plain folder the yellow folder. The home folder and its standard
//! folders lie in a temporary directory.

use std::fs;
use std::path::PathBuf;

use ox_core::location::file_uri;
use ox_core::places::{FolderLocations, KnownFolder};

use super::support::arts_in;
use crate::icons::Art;
use crate::locations::Page;
use crate::test_support::harness::{wait_until, TestWindow};

/// The standard folders whose tabs show their glyph.
const STANDARD: [(&str, KnownFolder); 6] = [
    ("Desktop", KnownFolder::Desktop),
    ("Downloads", KnownFolder::Downloads),
    ("Documents", KnownFolder::Documents),
    ("Pictures", KnownFolder::Pictures),
    ("Music", KnownFolder::Music),
    ("Videos", KnownFolder::Videos),
];

/// What the strip of `test` shows on its one tab, once it shows `uri`.
fn tab_art(test: &TestWindow, uri: &str) -> Vec<Art> {
    test.window.navigate(uri).expect("a valid folder");
    test.wait_for_listing("the folder");
    arts_in(&test.window.tab_strip().tab_list())
}

/// parity: TAB-010
#[gtk::test]
fn a_standard_folders_tab_shows_its_navigation_pane_glyph() {
    let root = tempfile::tempdir().expect("the test home has room for folders");
    let base = fs::canonicalize(root.path()).expect("the folder resolves");
    let home: PathBuf = base.join("home");
    let config = base.join("config");
    for (name, _) in STANDARD {
        fs::create_dir_all(home.join(name)).expect("a standard folder");
    }
    fs::create_dir_all(home.join("Work")).expect("a plain folder");
    fs::create_dir_all(&config).expect("the config folder");
    let locations = FolderLocations::new(home.clone(), &config);
    let test = TestWindow::open_with_standard_folders(&file_uri(&home.join("Work")), locations, |_| {});
    wait_until("the standard folders", || {
        test.context
            .known_folders()
            .iter()
            .any(|place| place.known_folder == Some(KnownFolder::Videos))
    });

    let mut shown = Vec::new();
    let mut expected = Vec::new();
    for (name, folder) in STANDARD {
        shown.push((name, tab_art(&test, &file_uri(&home.join(name)))));
        expected.push((name, vec![Art::for_known_folder(folder)]));
    }
    shown.push(("Work", tab_art(&test, &file_uri(&home.join("Work")))));
    expected.push(("Work", vec![Art::Folder]));
    // The window's own home folder, which Home opens while Quick access
    // is off, as it is by default.
    let home_folder = test.window.home_location();
    shown.push(("Home", tab_art(&test, &home_folder)));
    expected.push(("Home", vec![Art::Glyph(Page::Home.icon())]));

    assert_eq!(shown, expected);
}
