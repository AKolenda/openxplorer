// SPDX-License-Identifier: AGPL-3.0-only
//! Name conflicts on copy and move: Skip, Keep both, the policies side by
//! side, and moves into the item's own folder. Ports the conflict cases of
//! `TransferTests` in `v2.0.0:desktop/tests/test_operations.py`.

use ox_core::transfer::{ConflictPolicy, TransferMode};

use crate::transfer_support::{local, *};

/// Ported from `v2.0.0:desktop/tests/test_operations.py::TransferTests::test_keep_both`:
/// a taken `(copy 2)` name is skipped too, and
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

/// Ported from `v2.0.0:desktop/tests/test_operations.py::TransferTests::test_move_same_directory_is_noop`, for every policy: moving an
/// item into the folder it is already in is skipped (XFER-012). With Keep
/// both it would otherwise be renamed to "(copy 2)"; with Replace it would
/// be replaced by itself.
///
/// parity: XFER-012
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

/// The conflict dialog's Rename copies an item under the typed name and
/// refuses a name taken meanwhile; Replace never overwrites an item with
/// itself (Dolphin's "cannot copy file onto itself").
///
/// parity: OPS-028
#[test]
fn rename_uses_the_typed_name_and_nothing_replaces_itself() {
    let fixture = Fixture::new();
    let source = fixture.source_folder.join("a.txt");
    write(&source, "new");
    write(&fixture.destination_folder.join("a.txt"), "old");
    write(&fixture.destination_folder.join("taken.txt"), "taken");
    let destination = file_uri(&fixture.destination_folder);
    let cancel = ox_core::transfer::Cancellation::new();
    let mut engine = fixture.engine(local::local());

    let renamed = engine
        .run_renamed(
            TransferMode::Copy,
            &destination,
            &file_uri(&source),
            "b.txt".as_ref(),
            &cancel,
        )
        .expect("the run is accepted");
    let taken = engine
        .run_renamed(
            TransferMode::Copy,
            &destination,
            &file_uri(&source),
            "taken.txt".as_ref(),
            &cancel,
        )
        .expect("the run is accepted");
    let itself = fixture.run(
        &mut engine,
        &[&source],
        Request::CopyInto(&fixture.source_folder, ConflictPolicy::Replace),
    );

    assert_eq!(renamed.done, [file_uri(&source)], "{renamed:?}");
    assert_eq!(read(&fixture.destination_folder.join("b.txt")), "new");
    assert_eq!(read(&fixture.destination_folder.join("a.txt")), "old");
    assert_eq!(taken.errors.len(), 1, "{taken:?}");
    assert_eq!(read(&fixture.destination_folder.join("taken.txt")), "taken");
    assert!(
        itself.errors[0].ends_with("An item cannot replace itself."),
        "{itself:?}"
    );
    assert_eq!(read(&source), "new");
    fixture.assert_no_staging();
}

/// Ported from `v2.0.0:desktop/tests/test_operations.py::TransferTests::test_move_collision_keeps_source`: Skip leaves both the source
/// and the item that holds its name alone (XFER-006, XFER-012).
///
/// parity: XFER-006, XFER-012
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
