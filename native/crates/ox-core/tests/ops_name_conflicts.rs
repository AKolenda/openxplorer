// SPDX-License-Identifier: AGPL-3.0-only
//! Name conflicts of a copy or move, as Windows Explorer finds and
//! resolves them (OPS-026, OPS-027, OPS-028): the check looks inside a
//! folder whose name is taken by a folder, since Windows merges those
//! without asking and asks only about the files inside, and the answers
//! (Replace, Skip, or both kept as "name (2).ext") run with every safety
//! rule of the transfer engine. A replaced file goes to the test run's
//! private Recycle Bin, so Undo can put it back. All names are fictional.

#[path = "ops_support/folders.rs"]
mod folders;
mod ops_support;
#[path = "ops_support/private_trash.rs"]
mod private_trash;
#[path = "ops_support/snapshots.rs"]
mod snapshots;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use ox_core::ops::{
    changed_copies, reverse, run_answered_items, run_free_items, run_resolved_transfer, scan_name_conflicts,
    ConflictScan, OperationContext, OpsError, PlannedKind, Resolution, UndoRecord,
};
use ox_core::transfer::{Cancellation, TransferMode, MAX_ITEMS};

use folders::Folders;
use ops_support::{block_on, file_uri};
use private_trash::require_private_trash;
use snapshots::{snapshot_protection, READ_ONLY};

/// The conflicts of moving or copying `names` of `from` into `into`.
fn scan(mode: TransferMode, from: &Path, names: &[&str], into: &Path) -> ConflictScan {
    let uris: Vec<String> = names.iter().map(|name| file_uri(&from.join(name))).collect();
    block_on(scan_name_conflicts(
        mode,
        &uris,
        &file_uri(into),
        &Cancellation::new(),
    ))
    .expect("a scan")
}

/// The URIs of each conflict's incoming item, relative to `from`.
fn conflict_names(scan: &ConflictScan, from: &Path) -> Vec<String> {
    let base = file_uri(from);
    scan.conflicts
        .iter()
        .map(|conflict| {
            conflict
                .incoming_uri
                .trim_start_matches(&format!("{base}/"))
                .to_owned()
        })
        .collect()
}

/// Writes `contents` at `path`, making its folders.
fn write(path: &Path, contents: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// Runs `scan` with `answers`, one per conflict.
fn run(scan: &ConflictScan, answers: &[Resolution]) -> ox_core::ops::TransferOutcome {
    block_on(run_resolved_transfer(
        scan,
        answers,
        &OperationContext::default(),
        |_| {},
    ))
    .expect("an accepted transfer")
}

/// A folder whose name is taken by a folder is merged: only the files
/// inside with taken names are conflicts, as Windows asks ("The
/// destination has 3 files with the same names"), and files only one
/// side has never are.
///
/// parity: OPS-026, OPS-027
#[test]
fn the_check_looks_inside_a_folder_that_merges_into_its_namesake() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    write(&src.join("report.png"), b"incoming");
    write(&src.join("Projects/plan.txt"), b"incoming");
    write(&src.join("Projects/new.txt"), b"incoming");
    write(&src.join("Projects/Drafts/outline.txt"), b"incoming");
    write(&dst.join("report.png"), b"existing");
    write(&dst.join("Projects/plan.txt"), b"existing");
    write(&dst.join("Projects/only here.txt"), b"existing");
    write(&dst.join("Projects/Drafts/outline.txt"), b"existing");

    let scan = scan(TransferMode::Copy, &src, &["report.png", "Projects"], &dst);

    assert_eq!(
        conflict_names(&scan, &src),
        ["report.png", "Projects/Drafts/outline.txt", "Projects/plan.txt"],
        "in the order of the items, each folder's contents by name"
    );
    assert_eq!(
        scan.conflicts[2].existing_uri,
        file_uri(&dst.join("Projects/plan.txt"))
    );
    let free: Vec<(String, String)> = scan
        .items
        .iter()
        .filter(|item| item.kind == PlannedKind::Free)
        .map(|item| (item.uri.clone(), item.destination_folder.clone()))
        .collect();
    assert_eq!(
        free,
        [(
            file_uri(&src.join("Projects/new.txt")),
            file_uri(&dst.join("Projects"))
        )]
    );
    assert_eq!(
        scan.merged_folders,
        [
            file_uri(&src.join("Projects/Drafts")),
            file_uri(&src.join("Projects"))
        ],
        "the deepest merged folder first"
    );
}

/// Hidden items, dangling links and a file meeting a folder's name are
/// conflicts too; their names are taken. A file never merges.
///
/// parity: OPS-027
#[test]
fn hidden_items_dangling_links_and_a_file_meeting_a_folder_count() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    for name in ["free.txt", "taken.txt", ".hidden", "link", "Music"] {
        write(&src.join(name), b"incoming");
    }
    write(&dst.join("taken.txt"), b"existing");
    write(&dst.join(".hidden"), b"existing");
    symlink("missing-target", dst.join("link")).unwrap();
    write(&dst.join("Music/song.ogg"), b"existing");

    let scan = scan(
        TransferMode::Copy,
        &src,
        &["free.txt", "taken.txt", ".hidden", "link", "Music"],
        &dst,
    );

    assert_eq!(
        conflict_names(&scan, &src),
        ["taken.txt", ".hidden", "link", "Music"]
    );
    assert!(scan.conflicts[3].existing.is_folder && !scan.conflicts[3].incoming.is_folder);
    assert!(scan.merged_folders.is_empty());
}

/// A copy into the item's own folder meets only its own name: it becomes
/// "name - Copy" without asking, as in Windows Explorer.
///
/// parity: OPS-026
#[test]
fn a_copy_into_its_own_folder_is_no_conflict() {
    let folders = Folders::new();
    let src = folders.source();
    write(&src.join("Budget.ods"), b"sheet");

    let scan = scan(TransferMode::Copy, &src, &["Budget.ods"], &src);

    assert!(scan.conflicts.is_empty());
    assert_eq!(scan.items[0].kind, PlannedKind::OwnFolderCopy);
}

/// The check needs a folder and between one and the maximum items, and a
/// cancelled check never reads as "no conflicts".
///
/// parity: OPS-027, OPS-035
#[test]
fn a_check_needs_a_folder_and_reports_a_cancellation() {
    let folders = Folders::new();
    let file = folders.source().join("a.txt");
    write(&file, b"a");
    let cancel = Cancellation::new();
    let one = [file_uri(&file)];
    let destination = file_uri(&folders.destination());
    let check = |uris: &[String], folder: &str, cancel: &Cancellation| {
        block_on(scan_name_conflicts(TransferMode::Copy, uris, folder, cancel)).map(|_| ())
    };
    let cancelled = Cancellation::new();
    cancelled.cancel();

    let open_folder = "Open a destination folder before pasting.";
    assert_eq!(
        check(&one, &file_uri(&file), &cancel),
        Err(OpsError::Failed(open_folder.into()))
    );
    let count = OpsError::Failed("Select between 1 and 100,000 items.".into());
    assert_eq!(check(&[], &destination, &cancel), Err(count.clone()));
    assert_eq!(
        check(&vec![one[0].clone(); MAX_ITEMS + 1], &destination, &cancel),
        Err(count)
    );
    assert!(matches!(
        check(&["smb://nas/share".into()], "file:///tmp", &cancel),
        Err(OpsError::Failed(message)) if message.starts_with("Open the network share first")
    ));
    assert_eq!(check(&one, &destination, &cancelled), Err(OpsError::Cancelled));
}

/// A folder opened through a symbolic link is a folder for a paste: the
/// items land in it, the scan reads the link's own listings, and the
/// destination that stays planned is the link's address, the one the
/// interface shows.
///
/// parity: OPS-027
#[test]
fn a_destination_through_a_symbolic_link_is_a_folder() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    let alias = dst.with_file_name("alias");
    symlink(&dst, &alias).unwrap();
    write(&src.join("free.txt"), b"incoming");
    write(&src.join("taken.txt"), b"incoming too");
    write(&dst.join("taken.txt"), b"existing");

    let scan = scan(TransferMode::Copy, &src, &["free.txt", "taken.txt"], &alias);

    assert_eq!(scan.destination_folder, file_uri(&alias));
    assert_eq!(conflict_names(&scan, &src), ["taken.txt"]);
    run(&scan, &[Resolution::Skip]);
    assert_eq!(fs::read(dst.join("free.txt")).unwrap(), b"incoming");
    assert_eq!(fs::read(dst.join("taken.txt")).unwrap(), b"existing");
}

/// The folder's own address reached through a symbolic link is the folder:
/// a cut item pasted into it stays where it is, without asking and without
/// its folder being removed as a merge that emptied it, and a copy is kept
/// as "name - Copy" instead of meeting itself as a name conflict. The two
/// addresses are the same folder, whatever the URI says.
///
/// parity: OPS-027
#[test]
fn a_link_to_the_item_s_own_folder_is_its_own_folder() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    write(&src.join("a.txt"), b"only copy");
    write(&src.join("Folder/inside.txt"), b"inside");
    fs::create_dir(src.join("Empty")).unwrap();
    let alias = dst.with_file_name("alias");
    symlink(&src, &alias).unwrap();

    let moved = scan(TransferMode::Move, &src, &["a.txt", "Folder", "Empty"], &alias);

    assert!(moved.conflicts.is_empty());
    assert!(moved.items.is_empty());
    assert!(moved.merged_folders.is_empty());
    run(&moved, &[]);
    assert_eq!(fs::read(src.join("a.txt")).unwrap(), b"only copy");
    assert_eq!(fs::read(src.join("Folder/inside.txt")).unwrap(), b"inside");
    assert!(src.join("Empty").is_dir(), "the folder stays");

    let copied = scan(TransferMode::Copy, &src, &["Folder"], &alias);

    assert!(copied.conflicts.is_empty());
    assert_eq!(copied.items[0].kind, PlannedKind::OwnFolderCopy);
    run(&copied, &[]);
    assert_eq!(fs::read(src.join("Folder - Copy/inside.txt")).unwrap(), b"inside");
}

/// "Skip N files with the same date and size" counts the conflicts whose
/// two files have the same size and modification time.
///
/// parity: OPS-028
#[test]
fn a_conflict_knows_when_both_files_have_the_same_date_and_size() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    write(&src.join("same.txt"), b"1234");
    write(&dst.join("same.txt"), b"abcd");
    write(&src.join("other.txt"), b"12345");
    write(&dst.join("other.txt"), b"abcd");
    let time = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    for path in [src.join("same.txt"), dst.join("same.txt")] {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }

    let scan = scan(TransferMode::Copy, &src, &["same.txt", "other.txt"], &dst);

    assert_eq!(scan.conflicts[0].incoming.size, Some(4));
    assert_eq!(scan.conflicts[0].incoming.modified, Some(1_700_000_000));
    assert!(scan.conflicts[0].same_date_and_size());
    assert!(!scan.conflicts[1].same_date_and_size());
}

/// Skip leaves the existing file, keeping both adds Windows' number,
/// "report (2).png", then "(3)" when that is taken too, and the files
/// without a conflict are copied into the merged folder.
///
/// parity: OPS-028, XFER-008
#[test]
fn skip_and_keep_both_resolve_each_file_inside_a_merged_folder() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    write(&src.join("Photos/report.png"), b"incoming report");
    write(&src.join("Photos/beach.jpg"), b"incoming beach");
    write(&src.join("Photos/new.jpg"), b"new");
    write(&dst.join("Photos/report.png"), b"existing report");
    write(&dst.join("Photos/beach.jpg"), b"existing beach");
    write(&dst.join("Photos/beach (2).jpg"), b"an older kept copy");

    let scan = scan(TransferMode::Copy, &src, &["Photos"], &dst);
    assert_eq!(
        conflict_names(&scan, &src),
        ["Photos/beach.jpg", "Photos/report.png"]
    );
    let outcome = run(&scan, &[Resolution::KeepBoth, Resolution::Skip]);

    assert!(outcome.result.errors.is_empty(), "{:?}", outcome.result.errors);
    assert_eq!(
        fs::read(dst.join("Photos/report.png")).unwrap(),
        b"existing report"
    );
    assert_eq!(fs::read(dst.join("Photos/beach.jpg")).unwrap(), b"existing beach");
    assert_eq!(
        fs::read(dst.join("Photos/beach (3).jpg")).unwrap(),
        b"incoming beach"
    );
    assert_eq!(fs::read(dst.join("Photos/new.jpg")).unwrap(), b"new");
    assert_eq!(outcome.result.skipped, [file_uri(&src.join("Photos/report.png"))]);
    assert_eq!(
        fs::read(src.join("Photos/report.png")).unwrap(),
        b"incoming report"
    );
    let Some(UndoRecord::Copy { copies }) = &outcome.undo else {
        panic!("a copy without Replace is undone as a copy: {:?}", outcome.undo);
    };
    assert!(
        !copies.contains(&file_uri(&dst.join("Photos"))),
        "Undo never trashes the merged folder, which held files before: {copies:?}"
    );
    assert_eq!(copies.len(), 2, "{copies:?}");
}

/// Replace puts the replaced file in the Recycle Bin and the incoming one
/// in its place. Undo takes the copy away and brings the replaced file
/// back; Redo replaces it again, and a second Undo restores it again.
///
/// parity: OPS-028, OPS-029, OPS-031
#[test]
fn undo_brings_back_a_replaced_file_and_redo_replaces_it_again() {
    require_private_trash();
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    write(&src.join("report.png"), b"incoming");
    write(&dst.join("report.png"), b"existing");
    let context = OperationContext::default();

    let scan = scan(TransferMode::Copy, &src, &["report.png"], &dst);
    let outcome = run(&scan, &[Resolution::Replace]);
    assert!(outcome.result.errors.is_empty(), "{:?}", outcome.result.errors);
    assert_eq!(fs::read(dst.join("report.png")).unwrap(), b"incoming");
    let record = outcome
        .undo
        .expect("Replace is undoable while the replaced file is kept");
    let UndoRecord::Replace { replaced, .. } = &record else {
        panic!("the journal records what was replaced: {record:?}");
    };
    assert_eq!(replaced, &[dst.join("report.png")]);
    assert_eq!(record.undo_label(), "Undo Copy");

    let undone = block_on(reverse(&record, &context, |_| {})).expect("an undo");
    assert!(undone.result.errors.is_empty(), "{:?}", undone.result.errors);
    assert_eq!(fs::read(dst.join("report.png")).unwrap(), b"existing");
    let redo = undone.inverse.expect("Redo copies it again");

    let redone = block_on(reverse(&redo, &context, |_| {})).expect("a redo");
    assert!(redone.result.errors.is_empty(), "{:?}", redone.result.errors);
    assert_eq!(fs::read(dst.join("report.png")).unwrap(), b"incoming");

    let again = redone.inverse.expect("Undo again");
    let undone_again = block_on(reverse(&again, &context, |_| {})).expect("a second undo");
    assert!(
        undone_again.result.errors.is_empty(),
        "{:?}",
        undone_again.result.errors
    );
    assert_eq!(fs::read(dst.join("report.png")).unwrap(), b"existing");
    assert_eq!(fs::read(src.join("report.png")).unwrap(), b"incoming");
}

/// A move merges its folder: the answered files move or stay, the folder
/// the move emptied goes, one holding a skipped file stays, and Undo
/// moves everything back, the replaced file included.
///
/// parity: OPS-026, OPS-028, OPS-029
#[test]
fn a_move_merges_folders_and_undo_puts_everything_back() {
    require_private_trash();
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    write(&src.join("Music/Live/track.ogg"), b"incoming track");
    write(&src.join("Music/cover.jpg"), b"incoming cover");
    write(&src.join("Music/notes.txt"), b"incoming notes");
    write(&dst.join("Music/Live/track.ogg"), b"existing track");
    write(&dst.join("Music/cover.jpg"), b"existing cover");

    let scan = scan(TransferMode::Move, &src, &["Music"], &dst);
    assert_eq!(
        conflict_names(&scan, &src),
        ["Music/cover.jpg", "Music/Live/track.ogg"]
    );
    let outcome = run(&scan, &[Resolution::Skip, Resolution::Replace]);

    assert!(outcome.result.errors.is_empty(), "{:?}", outcome.result.errors);
    assert_eq!(
        fs::read(dst.join("Music/Live/track.ogg")).unwrap(),
        b"incoming track"
    );
    assert_eq!(fs::read(dst.join("Music/notes.txt")).unwrap(), b"incoming notes");
    assert_eq!(fs::read(dst.join("Music/cover.jpg")).unwrap(), b"existing cover");
    assert!(!src.join("Music/Live").exists(), "the emptied folder goes");
    assert_eq!(
        fs::read(src.join("Music/cover.jpg")).unwrap(),
        b"incoming cover",
        "the skipped file stays where it was"
    );
    assert!(
        !outcome.result.done.contains(&scan.selection[0]),
        "the folder holding the skipped file stays on the clipboard"
    );

    let record = outcome.undo.expect("an undoable move");
    assert_eq!(record.undo_label(), "Undo Move");
    let undone = block_on(reverse(&record, &OperationContext::default(), |_| {})).expect("an undo");
    assert!(undone.result.errors.is_empty(), "{:?}", undone.result.errors);
    assert_eq!(
        fs::read(src.join("Music/Live/track.ogg")).unwrap(),
        b"incoming track"
    );
    assert_eq!(fs::read(src.join("Music/notes.txt")).unwrap(), b"incoming notes");
    assert_eq!(
        fs::read(dst.join("Music/Live/track.ogg")).unwrap(),
        b"existing track"
    );
    assert!(!dst.join("Music/notes.txt").exists());
}

/// The items without a conflict are copied before the user answers, as
/// Windows queues the conflicts to the end of the job; the answers then
/// finish it, and Undo covers both parts.
///
/// parity: OPS-026, OPS-027
#[test]
fn the_items_without_a_conflict_go_before_the_answers() {
    require_private_trash();
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    write(&src.join("free.txt"), b"free");
    write(&src.join("taken.txt"), b"incoming");
    write(&dst.join("taken.txt"), b"existing");
    let context = OperationContext::default();
    let scan = scan(TransferMode::Copy, &src, &["free.txt", "taken.txt"], &dst);

    let first = block_on(run_free_items(&scan, &context, |_| {})).expect("the free items");
    assert!(dst.join("free.txt").is_file(), "copied before the answers");
    assert_eq!(fs::read(dst.join("taken.txt")).unwrap(), b"existing");
    assert!(!first.cancelled());
    let outcome = block_on(run_answered_items(
        &scan,
        &[Resolution::Replace],
        first,
        &context,
        |_| {},
    ))
    .expect("the answers");

    assert_eq!(fs::read(dst.join("taken.txt")).unwrap(), b"incoming");
    assert_eq!(
        outcome.created,
        [file_uri(&dst.join("free.txt")), file_uri(&dst.join("taken.txt"))]
    );
    let Some(UndoRecord::Replace { steps, .. }) = &outcome.undo else {
        panic!("{:?}", outcome.undo);
    };
    assert!(matches!(steps.as_slice(), [UndoRecord::Copy { copies }] if copies.len() == 2));
}

/// Stopping at the question keeps what was copied before it, undoably,
/// and changes none of the files it asked about.
///
/// parity: OPS-026
#[test]
fn stopping_at_the_question_keeps_the_items_copied_before_it() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    write(&src.join("free.txt"), b"free");
    write(&src.join("taken.txt"), b"incoming");
    write(&dst.join("taken.txt"), b"existing");
    let context = OperationContext::default();
    let scan = scan(TransferMode::Copy, &src, &["free.txt", "taken.txt"], &dst);

    let first = block_on(run_free_items(&scan, &context, |_| {})).expect("the free items");
    let outcome = first.into_outcome(&scan, &context);

    assert!(outcome.result.cancelled, "nothing to report");
    assert_eq!(fs::read(dst.join("taken.txt")).unwrap(), b"existing");
    assert_eq!(
        outcome.undo,
        Some(UndoRecord::Copy {
            copies: vec![file_uri(&dst.join("free.txt"))]
        })
    );
}

/// XFER-020 keeps a merging move whole: a selected folder holding a
/// protected snapshot in a merged subfolder is refused as one tree before
/// anything in it moves, as the engine refuses a selected folder before
/// it changes it.
///
/// parity: XFER-020, OPS-026
#[test]
fn a_merging_move_with_a_protected_folder_inside_moves_nothing() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    write(&src.join("project/a.txt"), b"plain");
    write(&src.join("project/sub/.snapshot/version.txt"), b"kept");
    fs::create_dir_all(dst.join("project/sub")).unwrap();
    let context = OperationContext::new(snapshot_protection());
    let scan = scan(TransferMode::Move, &src, &["project"], &dst);

    let free = block_on(run_free_items(&scan, &context, |_| {}));
    let whole = block_on(run_resolved_transfer(&scan, &[], &context, |_| {}));

    assert_eq!(free.map(|_| ()), Err(OpsError::Failed(READ_ONLY.into())));
    assert_eq!(whole.map(|_| ()), Err(OpsError::Failed(READ_ONLY.into())));
    assert_eq!(
        fs::read(src.join("project/a.txt")).unwrap(),
        b"plain",
        "nothing moved"
    );
    assert!(!dst.join("project/a.txt").exists());
}

/// A conflict whose answer cannot be carried out after the free items
/// moved is reported in a finished outcome, which still lists and undoes
/// what was done: here the extension leaves no room for " (2)".
///
/// parity: OPS-028, OPS-029
#[test]
fn a_failed_answer_after_the_free_items_still_finishes_the_outcome() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    let crowded = format!("a.{}", "x".repeat(252));
    write(&src.join("free.txt"), b"free");
    write(&src.join(&crowded), b"incoming");
    write(&dst.join(&crowded), b"existing");
    let context = OperationContext::default();
    let scan = scan(TransferMode::Copy, &src, &["free.txt", &crowded], &dst);

    let first = block_on(run_free_items(&scan, &context, |_| {})).expect("the free items");
    let outcome = block_on(run_answered_items(
        &scan,
        &[Resolution::KeepBoth],
        first,
        &context,
        |_| {},
    ))
    .expect("a finished outcome, not a bare error");

    assert_eq!(outcome.result.errors.len(), 1, "{:?}", outcome.result.errors);
    assert_eq!(outcome.created, [file_uri(&dst.join("free.txt"))]);
    assert_eq!(
        outcome.undo,
        Some(UndoRecord::Copy {
            copies: vec![file_uri(&dst.join("free.txt"))]
        })
    );
    assert_eq!(fs::read(dst.join(&crowded)).unwrap(), b"existing");
}

/// A merged move answered with Keep both lands under a numbered name and
/// empties its folder, which goes; Undo makes the folder again and moves
/// the item back under its own name.
///
/// parity: OPS-028, OPS-029
#[test]
fn undoing_a_kept_both_move_out_of_a_removed_folder_makes_the_folder_again() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    write(&src.join("Music/cover.jpg"), b"incoming");
    write(&dst.join("Music/cover.jpg"), b"existing");
    let context = OperationContext::default();
    let scan = scan(TransferMode::Move, &src, &["Music"], &dst);

    let outcome = run(&scan, &[Resolution::KeepBoth]);
    assert_eq!(fs::read(dst.join("Music/cover (2).jpg")).unwrap(), b"incoming");
    assert!(!src.join("Music").exists(), "the emptied folder went");
    assert!(
        outcome.result.done.contains(&scan.selection[0]),
        "the folder moved whole, so a cut paste takes it off the clipboard"
    );
    let record = outcome.undo.expect("an undoable move");
    let undone = block_on(reverse(&record, &context, |_| {})).expect("an undo");

    assert!(undone.result.errors.is_empty(), "{:?}", undone.result.errors);
    assert_eq!(fs::read(src.join("Music/cover.jpg")).unwrap(), b"incoming");
    assert!(!dst.join("Music/cover (2).jpg").exists());
}

/// A merged move removes the empty folders it merged, even one that held
/// nothing at all; Undo makes them again and Redo removes them again.
///
/// parity: OPS-026, OPS-029, OPS-031
#[test]
fn undo_makes_again_the_empty_folders_a_merging_move_removed() {
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    fs::create_dir_all(src.join("Empty/Inner")).unwrap();
    fs::create_dir_all(dst.join("Empty/Inner")).unwrap();
    let context = OperationContext::default();
    let scan = scan(TransferMode::Move, &src, &["Empty"], &dst);

    let outcome = run(&scan, &[]);
    assert!(!src.join("Empty").exists(), "the merged folders went");
    let record = outcome.undo.expect("removed folders are undoable");
    let undone = block_on(reverse(&record, &context, |_| {})).expect("an undo");
    assert!(undone.result.errors.is_empty(), "{:?}", undone.result.errors);
    assert!(src.join("Empty/Inner").is_dir(), "made again");

    let redo = undone.inverse.expect("Redo removes them again");
    let redone = block_on(reverse(&redo, &context, |_| {})).expect("a redo");
    assert!(redone.result.errors.is_empty(), "{:?}", redone.result.errors);
    assert!(!src.join("Empty").exists());
    assert!(
        dst.join("Empty/Inner").is_dir(),
        "the destination keeps its folders"
    );
}

/// After Undo and Redo of a copy that replaced a file, editing the copy
/// still makes the next Undo ask first (OPS-030).
///
/// parity: OPS-030, OPS-031
#[test]
fn a_redone_replacing_copy_still_reports_its_edited_copy() {
    require_private_trash();
    let folders = Folders::new();
    let (src, dst) = (folders.source(), folders.destination());
    write(&src.join("report.png"), b"incoming");
    write(&dst.join("report.png"), b"existing");
    let context = OperationContext::default();
    let scan = scan(TransferMode::Copy, &src, &["report.png"], &dst);
    let record = run(&scan, &[Resolution::Replace]).undo.expect("undoable");
    let undone = block_on(reverse(&record, &context, |_| {})).expect("an undo");
    let redone = block_on(reverse(&undone.inverse.expect("a redo"), &context, |_| {})).expect("a redo");
    let again = redone.inverse.expect("Undo again");

    let now = std::time::SystemTime::now();
    let since = now.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    fs::File::options()
        .write(true)
        .open(dst.join("report.png"))
        .and_then(|file| file.set_modified(now + std::time::Duration::from_secs(60)))
        .unwrap();

    assert_eq!(again.undo_label(), "Undo Copy");
    assert_eq!(block_on(changed_copies(&again, since)), ["report.png"]);
}
