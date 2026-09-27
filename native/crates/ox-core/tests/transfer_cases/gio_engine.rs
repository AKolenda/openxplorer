// SPDX-License-Identifier: AGPL-3.0-only
//! The engine over the production GIO adapter with file names that are not
//! valid UTF-8, on temporary local files. The Python app handles them byte
//! for byte through `PyGObject`; the other engine cases of
//! `desktop/tests/gio_integration.py` are in `gio_integration.rs`.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use ox_core::transfer::{ConflictPolicy, TransferEngine, TransferError};

use crate::transfer_support::*;

/// A Latin-1 file name, as old ZIP files, CD rips and NAS folders leave them.
const LATIN1_NAME: &[u8] = b"caf\xe9.mp3";

/// An engine with a write guard that allows everything, as production
/// always passes one: the guard makes the engine list every tree first.
fn guarded_gio_engine() -> TransferEngine {
    gio_engine().with_write_guard(|_uri: &str| Ok::<(), TransferError>(()))
}

/// `album/` holding a Latin-1 named song and `ok.txt`, inside `folder`.
fn latin1_album(folder: &Path) -> PathBuf {
    let album = folder.join("album");
    fs::create_dir(&album).expect("create the album folder");
    fs::write(album.join(OsStr::from_bytes(LATIN1_NAME)), b"song").expect("write the song");
    write(&album.join("ok.txt"), "ok");
    album
}

/// The raw names in a folder, sorted.
fn raw_names(folder: &Path) -> Vec<OsString> {
    let mut names: Vec<OsString> = fs::read_dir(folder)
        .expect("list a folder")
        .map(|entry| entry.expect("read an entry").file_name())
        .collect();
    names.sort();
    names
}

/// Asserts `album` holds exactly the Latin-1 song and `ok.txt`, byte for
/// byte, and that no name was converted to U+FFFD.
fn assert_latin1_album(album: &Path) {
    let lossy_name = OsString::from("caf\u{fffd}.mp3");
    let expected = [
        OsStr::from_bytes(LATIN1_NAME).to_os_string(),
        OsString::from("ok.txt"),
    ];
    let names = raw_names(album);
    assert!(!names.contains(&lossy_name), "the name was converted lossily");
    assert_eq!(names, expected);
    let song = fs::read(album.join(OsStr::from_bytes(LATIN1_NAME))).expect("read the song");
    assert_eq!(song, b"song");
}

/// parity: XFER-001
#[test]
fn copy_keeps_names_that_are_not_utf8_byte_for_byte() {
    for mut engine in [gio_engine(), guarded_gio_engine()] {
        let fixture = Fixture::new();
        let album = latin1_album(&fixture.source_folder);

        let result = fixture.run(&mut engine, &[&album], Request::Copy(ConflictPolicy::Skip));

        assert!(result.errors.is_empty(), "{result:?}");
        assert_eq!(result.done, [file_uri(&album)]);
        assert_latin1_album(&fixture.destination_folder.join("album"));
        assert_latin1_album(&album);
        fixture.assert_no_staging();
    }
}

#[test]
fn move_keeps_names_that_are_not_utf8_byte_for_byte() {
    let fixture = Fixture::new();
    let album = latin1_album(&fixture.source_folder);

    let result = fixture.run(
        &mut guarded_gio_engine(),
        &[&album],
        Request::Move(ConflictPolicy::Skip),
    );

    assert!(result.errors.is_empty(), "{result:?}");
    assert!(!album.exists());
    assert_latin1_album(&fixture.destination_folder.join("album"));
}

/// Trash lists the whole tree for the write guard first, so a name that is
/// not UTF-8 anywhere inside must not stop it.
///
/// parity: XFER-015
#[test]
fn trash_and_permanent_delete_remove_folders_with_names_that_are_not_utf8() {
    // The check driver gives every test binary a disposable data folder, so
    // the Trash used here is never the user's.
    assert!(
        std::env::var_os("XDG_DATA_HOME").is_some(),
        "run through native/tools/check.py, which provides a disposable Trash"
    );
    for request in [Request::Delete, Request::Trash] {
        let fixture = Fixture::new();
        let album = latin1_album(&fixture.source_folder);

        let result = fixture.run(&mut guarded_gio_engine(), &[&album], request);

        assert!(result.errors.is_empty(), "{request:?}: {result:?}");
        assert_eq!(result.done, [file_uri(&album)]);
        assert!(!exists_without_following_links(&album));
    }
}

/// A Replace merge moves single children into the existing folder. Like the
/// Python app, whose `require_item_uri` refuses the escaped name, it stops
/// at a Latin-1 name, and the existing file under that name survives.
#[test]
fn replace_merge_stops_at_a_latin1_name_without_losing_the_existing_file() {
    let fixture = Fixture::new();
    let album = latin1_album(&fixture.source_folder);
    let existing = fixture.destination_folder.join("album");
    fs::create_dir(&existing).expect("create the existing album");
    fs::write(existing.join(OsStr::from_bytes(LATIN1_NAME)), b"old song").expect("write");
    write(&existing.join("keep.txt"), "keep");

    let result = fixture.run(
        &mut guarded_gio_engine(),
        &[&album],
        Request::Copy(ConflictPolicy::Replace),
    );

    assert!(result.done.is_empty(), "{result:?}");
    assert!(result.errors[0].contains("valid UTF-8"), "{result:?}");
    let song = fs::read(existing.join(OsStr::from_bytes(LATIN1_NAME))).expect("read");
    assert_eq!(song, b"old song");
    assert_eq!(read(&existing.join("keep.txt")), "keep");
    assert_latin1_album(&album);
    fixture.assert_no_staging();
}

/// parity: XFER-008
#[test]
fn keep_both_copies_folders_with_names_that_are_not_utf8() {
    let fixture = Fixture::new();
    let album = latin1_album(&fixture.source_folder);
    fs::create_dir(fixture.destination_folder.join("album")).expect("create the existing album");

    let result = fixture.run(
        &mut guarded_gio_engine(),
        &[&album],
        Request::Copy(ConflictPolicy::KeepBoth),
    );

    assert!(result.errors.is_empty(), "{result:?}");
    assert_latin1_album(&fixture.destination_folder.join("album (copy 2)"));
    assert!(list(&fixture.destination_folder.join("album")).is_empty());
    fixture.assert_no_staging();
}

/// A selected item whose own name is not UTF-8 is copied under exactly that
/// name. Keep both cannot make a text "(copy N)" name from it and says so
/// instead of renaming it lossily.
#[test]
fn a_selected_item_named_in_latin1_is_copied_but_never_renamed_lossily() {
    let fixture = Fixture::new();
    let song = fixture.source_folder.join(OsStr::from_bytes(LATIN1_NAME));
    fs::write(&song, b"song").expect("write the song");
    let mut engine = guarded_gio_engine();

    let copied = fixture.run(&mut engine, &[&song], Request::Copy(ConflictPolicy::Skip));

    assert!(copied.errors.is_empty(), "{copied:?}");
    assert_eq!(
        raw_names(&fixture.destination_folder),
        [OsStr::from_bytes(LATIN1_NAME)]
    );

    let kept = fixture.run(&mut engine, &[&song], Request::Copy(ConflictPolicy::KeepBoth));

    assert!(kept.done.is_empty());
    assert!(kept.errors[0].contains("not valid UTF-8"), "{kept:?}");
    assert_eq!(
        raw_names(&fixture.destination_folder),
        [OsStr::from_bytes(LATIN1_NAME)]
    );
    fixture.assert_no_staging();
}
