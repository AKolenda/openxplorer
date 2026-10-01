// SPDX-License-Identifier: AGPL-3.0-only
//! Moves the backend cannot do natively, finished by a staged copy and the
//! removal of the copied source items (XFER-013). The Python app refused
//! these moves; Dolphin (KIO) copies and then deletes what it copied.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::sync::Arc;

use ox_core::transfer::{
    Cancellation, ConflictPolicy, FilesystemInfo, Node, TransferEngine, TransferError, UnstorableAnswer,
};

use crate::transfer_support::{
    local::{LocalNode, Provider},
    *,
};

/// What happens while the source is being copied or removed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Event {
    /// Nothing unusual.
    None,
    /// Another program saves a new file into the source folder.
    FileAppears,
    /// The user cancels.
    Cancel,
    /// Removing a source item fails.
    RemovalFails,
    /// An editor saves `album/one.jpg` over its old version, by writing a
    /// new file and renaming it over the name, after the copy was built and
    /// before the source is removed.
    SavedOver,
}

/// A destination on another file system: the user's items cannot be moved
/// there natively, only the engine's staged copy is published.
struct OtherFilesystem {
    event: Event,
    /// The fixture's source folder.
    source_folder: PathBuf,
    cancel: Cancellation,
    /// The destination is FAT, which stores no links.
    fat: bool,
}

impl OtherFilesystem {
    fn new(fixture: &Fixture, event: Event) -> Self {
        Self {
            event,
            source_folder: fixture.source_folder.clone(),
            cancel: fixture.cancel.clone(),
            fat: false,
        }
    }
}

/// True for an item inside the engine's staging, which it publishes with a
/// rename in the destination's own file system.
fn is_staged(node: &LocalNode) -> bool {
    node.local_path().ancestors().skip(1).any(is_staging_path)
}

impl Provider for OtherFilesystem {
    fn copy_file(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        let appearing = self.source_folder.join("album").join("new.jpg");
        match self.event {
            Event::FileAppears if !appearing.exists() => write(&appearing, "new photo"),
            Event::Cancel => self.cancel.cancel(),
            _ => {}
        }
        node.local_copy_file(target, cancel, progress)
    }

    fn move_native(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        if !is_staged(node) {
            return Err(TransferError::NotSupported("Native move unsupported.".into()));
        }
        if self.event == Event::SavedOver {
            let saved = self.source_folder.join("one.jpg.save");
            write(&saved, "one, edited");
            fs::rename(&saved, self.source_folder.join("album").join("one.jpg")).unwrap();
        }
        node.local_move_native(target, cancel)
    }

    fn delete(&self, node: &LocalNode) -> Result<(), TransferError> {
        if self.event == Event::RemovalFails && node.local_path().starts_with(&self.source_folder) {
            return Err(TransferError::failed("Permission denied."));
        }
        node.local_delete()
    }

    fn filesystem(&self, _node: &LocalNode) -> Option<FilesystemInfo> {
        self.fat.then(|| FilesystemInfo {
            kind: Some("msdos".into()),
            free: None,
            id: None,
        })
    }
}

/// An engine over `provider` whose user agrees to finish moves by copying.
fn consenting(fixture: &Fixture, provider: Arc<dyn Provider>) -> TransferEngine {
    fixture.engine(provider).with_move_by_copying_question(|_| true)
}

/// A folder `album` holding two photos.
fn album(fixture: &Fixture) -> PathBuf {
    let album = fixture.source_folder.join("album");
    fs::create_dir(&album).unwrap();
    write(&album.join("one.jpg"), "one");
    write(&album.join("two.jpg"), "two");
    album
}

/// A file saved into the source folder during the copy was never copied,
/// so it is kept where it is, and the user is told; everything that was
/// copied is removed from the source.
///
/// parity: XFER-013
#[test]
fn a_file_saved_into_the_source_during_the_move_is_kept() {
    let fixture = Fixture::new();
    let source = album(&fixture);
    let mut engine = consenting(
        &fixture,
        Arc::new(OtherFilesystem::new(&fixture, Event::FileAppears)),
    );

    let result = fixture.run(&mut engine, &[&source], Request::Move(ConflictPolicy::Skip));

    assert_eq!(
        list(&fixture.destination_folder.join("album")),
        ["one.jpg", "two.jpg"]
    );
    assert_eq!(list(&source), ["new.jpg"]);
    assert_eq!(result.done.len(), 1, "{result:?}");
    let notice = format!(
        "album: 1 item appeared in the source during the move and was kept at {}.",
        file_uri(&source)
    );
    assert_eq!(result.errors, [notice]);
    fixture.assert_no_staging();
}

/// A cancel during the copy keeps the whole source and leaves nothing in
/// the destination; a removal that fails after publication names the copy.
///
/// parity: XFER-013
#[test]
fn a_cancelled_copy_or_a_failed_removal_keeps_the_source() {
    let fixture = Fixture::new();
    let source = album(&fixture);
    let mut engine = consenting(&fixture, Arc::new(OtherFilesystem::new(&fixture, Event::Cancel)));

    let cancelled = fixture.run(&mut engine, &[&source], Request::Move(ConflictPolicy::Skip));

    assert!(cancelled.cancelled && cancelled.done.is_empty(), "{cancelled:?}");
    assert_eq!(list(&source), ["one.jpg", "two.jpg"]);
    assert!(list(&fixture.destination_folder).is_empty());

    let fixture = Fixture::new();
    let source = album(&fixture);
    let mut engine = consenting(
        &fixture,
        Arc::new(OtherFilesystem::new(&fixture, Event::RemovalFails)),
    );

    let failed = fixture.run(&mut engine, &[&source], Request::Move(ConflictPolicy::Skip));

    let copy = fixture.destination_folder.join("album");
    assert_eq!(list(&copy), ["one.jpg", "two.jpg"]);
    assert_eq!(list(&source), ["one.jpg", "two.jpg"]);
    assert!(failed.errors[0].contains(&file_uri(&copy)), "{failed:?}");
    assert!(failed.errors[0].contains("could not be removed"), "{failed:?}");
    fixture.assert_no_staging();
}

/// A copy that left out a link FAT cannot store is not the whole item, so
/// the source is kept and the error names the copy.
///
/// parity: XFER-013
#[test]
fn a_copy_that_left_items_out_keeps_the_source() {
    let fixture = Fixture::new();
    let source = album(&fixture);
    symlink("one.jpg", source.join("cover")).unwrap();
    let provider = OtherFilesystem {
        fat: true,
        ..OtherFilesystem::new(&fixture, Event::None)
    };
    let mut engine =
        consenting(&fixture, Arc::new(provider)).with_unstorable_question(|_| UnstorableAnswer::Skip);

    let result = fixture.run(&mut engine, &[&source], Request::Move(ConflictPolicy::Skip));

    let copy = fixture.destination_folder.join("album");
    assert_eq!(list(&copy), ["one.jpg", "two.jpg"]);
    assert_eq!(list(&source), ["cover", "one.jpg", "two.jpg"]);
    assert!(result.errors[0].contains("the original was kept"), "{result:?}");
    assert!(result.errors[0].contains(&file_uri(&copy)), "{result:?}");
    fixture.assert_no_staging();
}

/// A copied file that an editor saves over before the source is removed is
/// a different file now: it is kept with its new content, and the user is
/// told; the unchanged photo is removed.
///
/// parity: XFER-013
#[test]
fn a_file_saved_over_after_its_copy_is_kept_with_its_new_content() {
    let fixture = Fixture::new();
    let source = album(&fixture);
    let mut engine = consenting(
        &fixture,
        Arc::new(OtherFilesystem::new(&fixture, Event::SavedOver)),
    );

    let result = fixture.run(&mut engine, &[&source], Request::Move(ConflictPolicy::Skip));

    let copy = fixture.destination_folder.join("album");
    assert_eq!(list(&copy), ["one.jpg", "two.jpg"]);
    assert_eq!(read(&copy.join("one.jpg")), "one");
    assert_eq!(list(&source), ["one.jpg"]);
    assert_eq!(read(&source.join("one.jpg")), "one, edited");
    let notice = format!(
        "album: 1 item changed during the move and was kept at {}.",
        file_uri(&source)
    );
    assert_eq!(result.errors, [notice]);
    fixture.assert_no_staging();
}
