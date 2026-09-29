// SPDX-License-Identifier: AGPL-3.0-only
//! What the destination's file system can hold (XFER-028): free space, the
//! FAT file size limit, and names and links FAT cannot store. Beyond the
//! Python app, which showed the raw GIO error of the failing item.

use std::fs;
use std::os::unix::fs::symlink;
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
