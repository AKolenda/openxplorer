// SPDX-License-Identifier: AGPL-3.0-only
//! Tests of icons from Windows icon files, on synthetic files the tests
//! build ([`ox_core::windows_icons::synthetic`]): their pictures are test
//! patterns, never Windows icons.

use std::path::{Path, PathBuf};

use ox_core::settings::Preferences;
use ox_core::windows_icons::synthetic::SyntheticIconFile;
use ox_core::windows_icons::{IconLibrary, IconRole};

use super::*;

/// Turns the icons off again when a test ends, also when it fails, so the
/// next test starts with the app's own icons.
struct Reset;

impl Drop for Reset {
    fn drop(&mut self) {
        follow_preferences(&Preferences::default());
        crate::icons::file_icons::follow(&ox_core::settings::FileIcons::new());
    }
}

/// Writes synthetic `imageres.dll.mun` and `shell32.dll.mun` files with
/// every group the roles use into `folder`.
fn write_both_libraries(folder: &Path) -> Vec<PathBuf> {
    [
        ("imageres.dll.mun", IconLibrary::Imageres),
        ("shell32.dll.mun", IconLibrary::Shell32),
    ]
    .into_iter()
    .map(|(name, library)| {
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
        path
    })
    .collect()
}

/// Preferences with the icons on and `files` chosen.
fn chosen(files: &[PathBuf]) -> Preferences {
    Preferences {
        use_icon_files: true,
        icon_files: files.iter().map(|path| path.display().to_string()).collect(),
        ..Preferences::default()
    }
}

/// Files that are damaged, of another name or missing give nothing, and
/// every image keeps the app's own icon; nothing is written beside them,
/// only in the user's cache folder.
///
/// parity: LOOK-030
#[gtk::test]
fn files_that_give_nothing_keep_the_app_icons() {
    let _reset = Reset;
    let folder = tempfile::tempdir().expect("a temporary folder");
    let damaged = folder.path().join("imageres.dll.mun");
    std::fs::write(&damaged, b"MZ, and nothing a PE file needs").expect("written");
    let other = folder.path().join("notepad.exe");
    std::fs::write(&other, SyntheticIconFile::with_groups(&[3]).build()).expect("written");
    let missing = folder.path().join("shell32.dll.mun");

    follow_preferences(&chosen(&[damaged, other, missing]));
    wait_until_read();
    assert_eq!(status(), Status::Ready { found: 0 });
    let image = ArtImage::new(Art::Folder, 32);
    assert!(!image.shows_user_icon());
    assert_eq!(
        image.picture().icon_name().as_deref(),
        Some(Icon::FileFolder.name())
    );

    let beside: Vec<_> = std::fs::read_dir(folder.path())
        .expect("readable")
        .map(|entry| entry.expect("an entry").file_name())
        .collect();
    assert_eq!(beside.len(), 2, "only the two files written: {beside:?}");
    let cache = ox_core::windows_icons::cache_folder();
    assert!(cache.starts_with(glib::user_cache_dir()));
}

/// The order of the icon sources: a choice made in File icons first, then
/// the Windows icon files, then the app's own. A choice of a picture the
/// files lack keeps the app's; a choice of another kind's picture is drawn
/// as the files draw that kind. Reading the files or turning them off
/// counts a new File icons generation, so folder views draw again.
///
/// parity: LOOK-030, LOOK-029
#[gtk::test]
fn a_choice_in_file_icons_comes_before_the_windows_files() {
    use crate::icons::file_icons;

    let _reset = Reset;
    let folder = tempfile::tempdir().expect("a temporary folder");
    let files = write_both_libraries(folder.path());
    let before = file_icons::generation();
    follow_preferences(&chosen(&files));
    wait_until_read();
    assert_ne!(file_icons::generation(), before, "reading counts a generation");

    let zip = Art::for_file_name("Course notes.zip");
    assert_eq!(zip, Art::ZipFolder);
    assert!(
        ArtImage::new(zip, 48).shows_user_icon(),
        "no choice: the file's ZIP"
    );

    let mut icons = ox_core::settings::FileIcons::new();
    icons.insert("zip".into(), "zip-document".into());
    icons.insert(".iso".into(), "zip-folder".into());
    file_icons::follow(&icons);
    let chosen_zip = Art::for_file_name("Course notes.zip");
    assert_eq!(chosen_zip, Art::ZipDocument);
    let image = ArtImage::new(chosen_zip, 48);
    assert!(
        !image.shows_user_icon(),
        "the chosen picture, which the files lack"
    );
    let disc = Art::for_file_name("Installer.iso");
    assert_eq!(disc, Art::ZipFolder, "the ending's choice");
    assert!(
        ArtImage::new(disc, 48).shows_user_icon(),
        "drawn as the files draw a ZIP"
    );

    let read = file_icons::generation();
    follow_preferences(&Preferences::default());
    assert_ne!(file_icons::generation(), read, "turning off counts a generation");
}
