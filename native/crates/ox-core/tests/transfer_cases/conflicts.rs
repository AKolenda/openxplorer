// SPDX-License-Identifier: AGPL-3.0-only
//! Name conflicts on copy and move: Skip, Keep both and moves into the
//! item's own folder. Ports the conflict cases of `TransferTests` in
//! `desktop/tests/test_operations.py`.

use ox_core::transfer::{ConflictPolicy, TransferMode};

use crate::transfer_support::{local, *};

/// Port of `test_keep_both`: a taken `(copy 2)` name is skipped too, and
/// both existing files stay untouched.
///
/// parity: XFER-008
#[test]
fn keep_both_skips_every_taken_copy_name() {
    let fixture = Fixture::new();
    let source = fixture.src.join("file.txt");
    write(&source, "new");
    write(&fixture.dst.join("file.txt"), "old");
    write(&fixture.dst.join("file (copy 2).txt"), "also old");

    let result = fixture.copy(local::local(), &[&source], ConflictPolicy::KeepBoth);

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(result.done, [uri(&source)]);
    assert_eq!(read(&fixture.dst.join("file (copy 3).txt")), "new");
    assert_eq!(read(&fixture.dst.join("file (copy 2).txt")), "also old");
    assert_eq!(read(&fixture.dst.join("file.txt")), "old");
    fixture.no_stage();
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
        let source = fixture.src.join("a");
        write(&source, "a");
        let mut engine = fixture.engine(local::local());

        let result = fixture.run(
            &mut engine,
            &[&source],
            TransferMode::Move,
            policy,
            Some(&fixture.src),
        );

        assert!(result.errors.is_empty(), "{policy:?}: {result:?}");
        assert_eq!(result.skipped, [uri(&source)], "{policy:?}");
        assert_eq!(list(&fixture.src), ["a"], "{policy:?}");
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
    let source = fixture.src.join("a");
    write(&source, "new");
    write(&fixture.dst.join("a"), "old");
    let mut engine = fixture.engine(local::local());

    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Move,
        ConflictPolicy::Skip,
        None,
    );

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(result.skipped, [uri(&source)]);
    assert!(result.done.is_empty());
    assert_eq!(read(&source), "new");
    assert_eq!(read(&fixture.dst.join("a")), "old");
}
