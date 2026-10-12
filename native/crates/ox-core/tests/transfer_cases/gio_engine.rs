// SPDX-License-Identifier: AGPL-3.0-only
//! The engine over the production GIO adapter with file names that are not
//! valid UTF-8, on temporary local files. The Python app handles them byte
//! for byte through `PyGObject`; the other engine cases of
//! `v2.0.0:desktop/tests/gio_integration.py` are in `gio_integration.rs`.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{symlink, MetadataExt};
use std::path::{Path, PathBuf};

use ox_core::transfer::{Cancellation, ConflictPolicy, Operation, TransferEngine, TransferError};

use crate::transfer_support::*;

/// A Latin-1 file name, as old ZIP files, CD rips and NAS folders leave them.
const LATIN1_NAME: &[u8] = b"caf\xe9.mp3";

/// An engine with a write guard that allows everything, as production
/// always passes one: the guard makes the engine list every tree first.
fn guarded_gio_engine() -> TransferEngine {
    gio_engine().with_write_guard(|_uri: &str| Ok::<(), TransferError>(()))
}

/// Writes a song named [`LATIN1_NAME`] holding `content` into `folder`;
/// returns its path.
fn latin1_song(folder: &Path, content: &str) -> PathBuf {
    let song = folder.join(OsStr::from_bytes(LATIN1_NAME));
    write(&song, content);
    song
}

/// `album/` holding a Latin-1 named song and `ok.txt`, inside `folder`.
fn latin1_album(folder: &Path) -> PathBuf {
    let album = folder.join("album");
    fs::create_dir(&album).expect("create the album folder");
    latin1_song(&album, "song");
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
    latin1_song(&existing, "old song");
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
    assert_latin1_album(&fixture.destination_folder.join("album - Copy"));
    assert!(list(&fixture.destination_folder.join("album")).is_empty());
    fixture.assert_no_staging();
}

/// A selected item whose own name is not UTF-8 is copied under exactly that
/// name.
#[test]
fn a_selected_item_named_in_latin1_is_copied_under_exactly_that_name() {
    let fixture = Fixture::new();
    let song = latin1_song(&fixture.source_folder, "song");

    let copied = fixture.run(
        &mut guarded_gio_engine(),
        &[&song],
        Request::Copy(ConflictPolicy::Skip),
    );

    assert!(copied.errors.is_empty(), "{copied:?}");
    assert_eq!(
        raw_names(&fixture.destination_folder),
        [OsStr::from_bytes(LATIN1_NAME)]
    );
    fixture.assert_no_staging();
}

/// Keep both cannot make a text "- Copy" name from a name that is not
/// UTF-8, so it says so instead of renaming the item lossily, and the item
/// that holds the name is untouched.
#[test]
fn keep_both_refuses_a_latin1_name_instead_of_renaming_it_lossily() {
    let fixture = Fixture::new();
    let song = latin1_song(&fixture.source_folder, "song");
    let existing = latin1_song(&fixture.destination_folder, "existing song");

    let kept = fixture.run(
        &mut guarded_gio_engine(),
        &[&song],
        Request::Copy(ConflictPolicy::KeepBoth),
    );

    assert!(kept.done.is_empty());
    assert!(kept.errors[0].contains("not valid UTF-8"), "{kept:?}");
    assert_eq!(
        raw_names(&fixture.destination_folder),
        [OsStr::from_bytes(LATIN1_NAME)]
    );
    assert_eq!(read(&existing), "existing song");
    fixture.assert_no_staging();
}

/// A cut and paste onto another filesystem, once the user agreed, copies
/// the item through staging and removes the source only after the copy was
/// published; links stay links. `/tmp` and `/dev/shm` are separate
/// filesystems on most Linux systems; where they are not (package build
/// sandboxes), the check is skipped.
///
/// parity: XFER-013
#[test]
fn a_move_to_another_filesystem_copies_then_removes_the_source() {
    let device_of = |path: &Path| fs::metadata(path).expect("the folder exists").dev();
    let source_root = tempfile::tempdir().expect("the test may create folders in TMPDIR");
    let target_root = match tempfile::tempdir_in("/dev/shm") {
        Ok(root) if device_of(root.path()) != device_of(source_root.path()) => root,
        _ => {
            eprintln!("skipped: /dev/shm is missing, read-only or on the same file system as TMPDIR");
            return;
        }
    };
    let folder = source_root.path().join("folder");
    fs::create_dir(&folder).expect("create the source folder");
    write(&folder.join("notes.txt"), "notes");
    symlink("notes.txt", folder.join("link")).expect("create a link");
    let target_uri = file_uri(target_root.path());
    let operation = Operation::Move {
        destination_folder: &target_uri,
        policy: ConflictPolicy::Skip,
    };

    let result = guarded_gio_engine()
        .with_move_by_copying_question(|_| true)
        .run(operation, &[file_uri(&folder)], &Cancellation::new())
        .expect("the run is accepted");

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(result.done, [file_uri(&folder)]);
    assert!(!folder.exists(), "the source is removed after the copy");
    let moved = target_root.path().join("folder");
    assert_eq!(read(&moved.join("notes.txt")), "notes");
    assert_eq!(
        fs::read_link(moved.join("link")).expect("a link"),
        Path::new("notes.txt")
    );
    assert_eq!(raw_names(target_root.path()), [OsString::from("folder")]);
}

/// An unreadable item asks what to do: Retry runs it again once it can be
/// read, "Skip all" leaves it and every later failure out without asking,
/// and what failed is still reported.
///
/// parity: OPS-047
#[test]
fn a_failed_item_is_retried_or_skipped_as_answered() {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc, Mutex};

    use ox_core::transfer::{FailedItem, FailureAnswer};

    let fixture = Fixture::new();
    let names = ["a.txt", "b.txt", "c.txt", "d.txt"];
    let paths: Vec<PathBuf> = names
        .iter()
        .map(|name| fixture.source_folder.join(name))
        .collect();
    for (path, name) in paths.iter().zip(names) {
        write(path, name);
    }
    for unreadable in [&paths[0], &paths[1], &paths[3]] {
        fs::set_permissions(unreadable, fs::Permissions::from_mode(0o000)).expect("chmod");
    }
    if fs::read(&paths[0]).is_ok() {
        return; // Running as root, which reads anything.
    }
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&asked);
    let first = paths[0].clone();
    let mut engine = gio_engine().with_failure_question(move |item: &FailedItem| {
        let mut asked = log.lock().expect("question log");
        asked.push(item.name.clone());
        if asked.len() == 1 {
            fs::set_permissions(&first, fs::Permissions::from_mode(0o644)).expect("chmod");
            FailureAnswer::Retry
        } else {
            FailureAnswer::SkipAll
        }
    });
    let sources: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();

    let result = fixture.run(&mut engine, &sources, Request::Copy(ConflictPolicy::Skip));

    assert_eq!(*asked.lock().expect("question log"), ["a.txt", "b.txt"]);
    assert_eq!(result.done, [file_uri(&paths[0]), file_uri(&paths[2])]);
    assert_eq!(result.errors.len(), 2, "{result:?}");
    assert!(!result.cancelled);
    assert_eq!(list(&fixture.destination_folder), ["a.txt", "c.txt"]);
    fixture.assert_no_staging();
}

/// An entry inside a folder that cannot be copied is asked about by
/// itself, by its path, as in Windows Explorer: Retry copies it again once
/// it can be read, Skip leaves only it out, and the rest of the folder is
/// still copied and published; what was left out is reported.
///
/// parity: OPS-047
#[test]
fn an_entry_inside_a_folder_is_asked_about_by_itself() {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc, Mutex};

    use ox_core::transfer::{FailedItem, FailureAnswer};

    let fixture = Fixture::new();
    let tree = fixture.source_folder.join("tree");
    fs::create_dir_all(tree.join("inner")).expect("folders");
    write(&tree.join("a.txt"), "a");
    write(&tree.join("locked.txt"), "locked");
    write(&tree.join("inner").join("c.txt"), "c");
    make_fifo(&tree.join("inner").join("pipe"));
    let locked = tree.join("locked.txt");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");
    // As root, which reads anything, only the pipe is asked about.
    let as_root = fs::read(&locked).is_ok();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&asked);
    let unlock = locked.clone();
    let mut engine = gio_engine().with_failure_question(move |item: &FailedItem| {
        log.lock().expect("question log").push(item.name.clone());
        if item.name.ends_with("locked.txt") {
            fs::set_permissions(&unlock, fs::Permissions::from_mode(0o644)).expect("chmod");
            FailureAnswer::Retry
        } else {
            FailureAnswer::Skip
        }
    });

    let result = fixture.run(&mut engine, &[&tree], Request::Copy(ConflictPolicy::Skip));

    let mut asked = asked.lock().expect("question log").clone();
    asked.sort();
    let expected: &[&str] = if as_root {
        &["tree/inner/pipe"]
    } else {
        &["tree/inner/pipe", "tree/locked.txt"]
    };
    assert_eq!(asked, expected);
    assert_eq!(result.done, [file_uri(&tree)], "{result:?}");
    assert!(!result.cancelled);
    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert!(result.errors[0].starts_with("tree/inner/pipe: "), "{result:?}");
    let copy = fixture.destination_folder.join("tree");
    assert_eq!(list(&copy), ["a.txt", "inner", "locked.txt"]);
    assert_eq!(read(&copy.join("locked.txt")), "locked");
    assert_eq!(list(&copy.join("inner")), ["c.txt"]);
    fixture.assert_no_staging();
}

/// "Skip all" about an entry inside a folder leaves out every later entry
/// that cannot be copied without asking again, and "Cancel" stops the run:
/// nothing of the cancelled folder is published and its staging is gone.
///
/// parity: OPS-047
#[test]
fn skip_all_and_cancel_about_an_entry_inside_a_folder() {
    use std::sync::{Arc, Mutex};

    use ox_core::transfer::{FailedItem, FailureAnswer};

    let fixture = Fixture::new();
    let tree = fixture.source_folder.join("tree");
    fs::create_dir(&tree).expect("folder");
    write(&tree.join("a.txt"), "a");
    for pipe in ["one", "two", "three"] {
        make_fifo(&tree.join(pipe));
    }
    let asked = Arc::new(Mutex::new(0));
    let count = Arc::clone(&asked);
    let mut engine = gio_engine().with_failure_question(move |_: &FailedItem| {
        *count.lock().expect("count") += 1;
        FailureAnswer::SkipAll
    });
    let result = fixture.run(&mut engine, &[&tree], Request::Copy(ConflictPolicy::Skip));
    assert_eq!(*asked.lock().expect("count"), 1, "asked once");
    assert_eq!(result.errors.len(), 3, "{result:?}");
    assert_eq!(list(&fixture.destination_folder.join("tree")), ["a.txt"]);
    fixture.assert_no_staging();

    let cancelled = Fixture::new();
    let tree = cancelled.source_folder.join("tree");
    fs::create_dir(&tree).expect("folder");
    write(&tree.join("a.txt"), "a");
    make_fifo(&tree.join("pipe"));
    let later = cancelled.source_folder.join("later.txt");
    write(&later, "later");
    let mut engine = gio_engine().with_failure_question(|_: &FailedItem| FailureAnswer::Cancel);
    let result = cancelled.run(&mut engine, &[&tree, &later], Request::Copy(ConflictPolicy::Skip));
    assert!(result.cancelled, "{result:?}");
    assert!(result.done.is_empty(), "{result:?}");
    assert!(
        list(&cancelled.destination_folder).is_empty(),
        "nothing published, not even later.txt"
    );
    cancelled.assert_no_staging();
}
