// SPDX-License-Identifier: AGPL-3.0-only
//! Name conflicts on copy and move: Skip, Keep both, the policies side by
//! side, and moves into the item's own folder. Ports the conflict cases of
//! `TransferTests` in `desktop/tests/test_operations.py`.

use ox_core::transfer::ConflictPolicy;

use crate::transfer_support::{local, *};

/// Port of `test_keep_both`: a taken `(copy 2)` name is skipped too, and
/// both existing files stay untouched.
///
/// parity: XFER-008
#[test]
fn keep_both_skips_every_taken_copy_name() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("file.txt");
    write(&source, "new");
    write(&fixture.destination_folder.join("file.txt"), "old");
    write(&fixture.destination_folder.join("file (copy 2).txt"), "also old");

    let result = fixture.copy(local::local(), &[&source], ConflictPolicy::KeepBoth);

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(result.done, [file_uri(&source)]);
    assert_eq!(read(&fixture.destination_folder.join("file (copy 3).txt")), "new");
    assert_eq!(
        read(&fixture.destination_folder.join("file (copy 2).txt")),
        "also old"
    );
    assert_eq!(read(&fixture.destination_folder.join("file.txt")), "old");
    fixture.assert_no_staging();
}

/// Port of `test_move_same_directory_is_noop`, for every policy: moving an
/// item into the folder it is already in is skipped. With Keep both it
/// would otherwise be renamed to "(copy 2)"; with Replace it would be
/// replaced by itself.
#[test]
fn moving_an_item_into_its_own_folder_changes_nothing() {
    for policy in [
        ConflictPolicy::Skip,
        ConflictPolicy::KeepBoth,
        ConflictPolicy::Replace,
    ] {
        let fixture = Fixture::new();
        let source = fixture.source_folder.join("a");
        write(&source, "a");
        let mut engine = fixture.engine(local::local());

        let result = fixture.run(
            &mut engine,
            &[&source],
            Request::MoveInto(&fixture.source_folder, policy),
        );

        assert!(result.errors.is_empty(), "{policy:?}: {result:?}");
        assert_eq!(result.skipped, [file_uri(&source)], "{policy:?}");
        assert_eq!(list(&fixture.source_folder), ["a"], "{policy:?}");
        assert_eq!(read(&source), "a");
    }
}

/// Port of `test_move_collision_keeps_source`: Skip leaves both the source
/// and the item that holds its name alone.
///
/// parity: XFER-006
#[test]
fn a_move_onto_a_taken_name_with_skip_keeps_both_items() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("a");
    write(&source, "new");
    write(&fixture.destination_folder.join("a"), "old");
    let mut engine = fixture.engine(local::local());

    let result = fixture.run(&mut engine, &[&source], Request::Move(ConflictPolicy::Skip));

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(result.skipped, [file_uri(&source)]);
    assert!(result.done.is_empty());
    assert_eq!(read(&source), "new");
    assert_eq!(read(&fixture.destination_folder.join("a")), "old");
}

/// One conflict policy and where the existing and the incoming content end
/// up.
struct PolicyCase {
    policy: ConflictPolicy,
    /// The source is reported as skipped rather than done.
    is_skipped: bool,
    /// The content of `notes.txt` afterwards.
    notes_txt: &'static str,
    /// The content of `notes (copy 2).txt` afterwards; `None` when there
    /// must be no such file.
    notes_copy_2_txt: Option<&'static str>,
}

/// Ports `test_skip_never_overwrites`, `test_keep_both` and
/// `test_replace_file_after_staging_copy_completes`. Each run selects the
/// source twice, which must not make a second copy.
///
/// parity: XFER-006, XFER-008, XFER-009
#[test]
fn conflict_policies_never_overwrite_without_replace() {
    let cases = [
        PolicyCase {
            policy: ConflictPolicy::Skip,
            is_skipped: true,
            notes_txt: "old",
            notes_copy_2_txt: None,
        },
        PolicyCase {
            policy: ConflictPolicy::KeepBoth,
            is_skipped: false,
            notes_txt: "old",
            notes_copy_2_txt: Some("new"),
        },
        PolicyCase {
            policy: ConflictPolicy::Replace,
            is_skipped: false,
            notes_txt: "new",
            notes_copy_2_txt: None,
        },
    ];
    for case in cases {
        let fixture = Fixture::new();
        let source = fixture.source_folder.join("notes.txt");
        write(&source, "new");
        write(&fixture.destination_folder.join("notes.txt"), "old");
        let mut engine = fixture.engine(local::local());

        let result = fixture.run(&mut engine, &[&source, &source], Request::Copy(case.policy));

        assert!(result.errors.is_empty(), "{result:?}");
        assert_eq!(read(&source), "new");
        let listed = if case.is_skipped {
            &result.skipped
        } else {
            &result.done
        };
        assert_eq!(listed, &[file_uri(&source)], "{:?}", case.policy);
        let notes = fixture.destination_folder.join("notes.txt");
        assert_eq!(read(&notes), case.notes_txt, "{:?}", case.policy);
        let copy = fixture.destination_folder.join("notes (copy 2).txt");
        match case.notes_copy_2_txt {
            Some(text) => assert_eq!(read(&copy), text),
            None => assert!(!exists_without_following_links(&copy), "{:?}", case.policy),
        }
        assert!(fixture.leftovers().is_empty());
    }
}
