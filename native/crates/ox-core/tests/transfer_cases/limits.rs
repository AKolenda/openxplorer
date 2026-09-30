// SPDX-License-Identifier: AGPL-3.0-only
//! What the destination's file system can hold (XFER-028): free space, the
//! FAT file size limit, and names and links FAT cannot store. Beyond the
//! Python app, which showed the raw GIO error of the failing item.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ox_core::transfer::{
    ConflictPolicy, FilesystemInfo, UnstorableAnswer, UnstorableReason, FAT_MAX_FILE_SIZE,
};

use crate::transfer_support::{
    local::{LocalNode, Provider},
    *,
};

/// A USB stick formatted with FAT, with `free` bytes left.
struct FatStick {
    free: Option<u64>,
}

impl Provider for FatStick {
    fn filesystem(&self, _node: &LocalNode) -> Option<FilesystemInfo> {
        Some(FilesystemInfo {
            kind: Some("msdos".into()),
            free: self.free,
            id: None,
        })
    }
}

/// parity: XFER-028
#[test]
fn a_copy_that_does_not_fit_is_refused_before_anything_is_written() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("video.mp4");
    fs::write(&source, random_bytes(2048)).unwrap();
    let mut engine = fixture.engine(Arc::new(FatStick { free: Some(1024) }));

    let refused = fixture.try_run(&mut engine, &[&source], Request::Copy(ConflictPolicy::Skip));

    assert_eq!(
        refused.unwrap_err().to_string(),
        "Not enough free space on destination: 2.0 KB needed, 1.0 KB free."
    );
    assert!(list(&fixture.destination_folder).is_empty());
}

/// parity: XFER-028
#[test]
fn a_file_over_4_gib_is_refused_on_fat_with_a_message_that_says_why() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("disk.img");
    // Sparse: no 4 GiB are written.
    fs::File::create(&source)
        .unwrap()
        .set_len(FAT_MAX_FILE_SIZE + 1)
        .unwrap();
    let mut engine = fixture.engine(Arc::new(FatStick { free: None }));

    let result = fixture.run(&mut engine, &[&source], Request::Copy(ConflictPolicy::Skip));

    assert_eq!(
        result.errors,
        ["disk.img: disk.img is too large for the destination file system, which only supports files up to 4 GiB."]
    );
    assert!(list(&fixture.destination_folder).is_empty());
}

/// "Replace all" gives every forbidden name `_` instead; "Skip" leaves a
/// link out, since FAT cannot store links.
///
/// parity: XFER-028
#[test]
fn names_and_links_fat_cannot_store_are_renamed_or_left_out_as_answered() {
    let fixture = Fixture::new();
    let folder = fixture.source_folder.join("notes");
    fs::create_dir(&folder).unwrap();
    write(&folder.join("a:b.txt"), "a");
    write(&folder.join("why?.txt"), "why");
    symlink("a:b.txt", folder.join("link")).unwrap();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let questions = Arc::clone(&asked);
    let mut engine = fixture
        .engine(Arc::new(FatStick { free: None }))
        .with_unstorable_question(move |item| {
            questions.lock().unwrap().push((item.name.clone(), item.reason));
            match item.reason {
                UnstorableReason::InvalidCharacters => UnstorableAnswer::ReplaceAll,
                UnstorableReason::SymbolicLink => UnstorableAnswer::Skip,
            }
        });

    let result = fixture.run(&mut engine, &[&folder], Request::Copy(ConflictPolicy::Skip));

    assert!(result.errors.is_empty(), "{result:?}");
    let copied = fixture.destination_folder.join("notes");
    assert_eq!(list(&copied), ["a_b.txt", "why_.txt"]);
    assert_eq!(read(&copied.join("why_.txt")), "why");
    // One question per kind of problem: "Replace all" answers the second
    // name too.
    let asked = asked.lock().unwrap().clone();
    let count = |reason| asked.iter().filter(|(_, asked)| *asked == reason).count();
    assert_eq!(count(UnstorableReason::InvalidCharacters), 1, "{asked:?}");
    assert!(asked.contains(&("link".to_owned(), UnstorableReason::SymbolicLink)));
    fixture.assert_no_staging();
}

/// A renamed top-level item is reported where it landed, so Undo and the
/// selection after a paste find `a_b.txt`, not `a:b.txt`.
///
/// parity: XFER-028
#[test]
fn a_renamed_item_is_reported_under_its_new_name() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("a:b.txt");
    write(&source, "a");
    let mut engine = fixture
        .engine(Arc::new(FatStick { free: None }))
        .with_unstorable_question(|_| UnstorableAnswer::ReplaceAll);

    let result = fixture.run(&mut engine, &[&source], Request::Copy(ConflictPolicy::Skip));

    let landed = &result.landed[0];
    assert_eq!(landed.source, file_uri(&source));
    assert_eq!(
        landed.destination,
        file_uri(&fixture.destination_folder.join("a_b.txt"))
    );
}

/// Two file systems, told apart by `id::filesystem`: the destination
/// folder's and the source's.
struct TwoFilesystems {
    destination_folder: PathBuf,
    /// The source's id; the destination's makes a move a rename.
    source_id: &'static str,
}

impl Provider for TwoFilesystems {
    fn filesystem(&self, node: &LocalNode) -> Option<FilesystemInfo> {
        let is_destination = node.local_path().starts_with(&self.destination_folder);
        let id = if is_destination {
            "destination"
        } else {
            self.source_id
        };
        Some(FilesystemInfo {
            kind: Some("ext4".into()),
            free: Some(1024),
            id: Some(id.into()),
        })
    }
}

/// A move needs free space only when it crosses file systems; within one
/// it is a rename.
///
/// parity: XFER-028
#[test]
fn a_move_needs_free_space_only_across_file_systems() {
    for (source_id, fits) in [("destination", true), ("other", false)] {
        let fixture = Fixture::new();
        let source = fixture.source_folder.join("video.mp4");
        fs::write(&source, random_bytes(2048)).unwrap();
        let provider = TwoFilesystems {
            destination_folder: fixture.destination_folder.clone(),
            source_id,
        };
        let mut engine = fixture.engine(Arc::new(provider));

        let run = fixture.try_run(&mut engine, &[&source], Request::Move(ConflictPolicy::Skip));

        assert_eq!(run.is_ok(), fits, "{source_id}: {run:?}");
    }
}
