// SPDX-License-Identifier: AGPL-3.0-only
//! Reading snapshot collections and the items inside snapshots.
//!
//! Ports `SnapshotProvider` in `v2.0.0:desktop/file_services.py`. The lookup only
//! reads metadata through this trait, so it can be tested without GIO or a
//! NAS; [`GioSnapshotProvider`] is the one the app uses.

use gio::prelude::*;

use crate::entry::{entry_from_info, Entry, EntryError, ATTRIBUTES};
use crate::location::normalise;
use crate::transfer::Cancellation;

/// The first entries of a snapshot collection folder.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CollectionListing {
    /// The entries read, at most the limit asked for.
    pub snapshots: Vec<Entry>,
    /// True when the collection holds more entries than were read.
    pub has_more: bool,
}

/// Reads snapshot collections and the items inside snapshots, without
/// following links and without reading file contents.
pub trait SnapshotProvider {
    /// Lists at most `limit` entries of the collection folder `uri`.
    ///
    /// # Errors
    ///
    /// Why the folder could not be listed, including
    /// [`EntryError::Cancelled`] once `cancel` is cancelled.
    fn list_collection(
        &self,
        uri: &str,
        limit: usize,
        cancel: &Cancellation,
    ) -> Result<CollectionListing, EntryError>;

    /// Queries the item at `uri` itself, not the target of a link.
    ///
    /// # Errors
    ///
    /// [`EntryError::NotFound`] when there is no item at `uri`, which the
    /// lookup treats as "not in this snapshot"; any other failure is shown
    /// as a warning.
    fn inspect(&self, uri: &str, cancel: &Cancellation) -> Result<Entry, EntryError>;
}

/// The [`SnapshotProvider`] of the app: GIO's synchronous API with
/// `NOFOLLOW_SYMLINKS`, so local folders and network shares (`smb://`)
/// behave alike. It blocks, so call it on a worker thread.
#[derive(Debug, Clone, Copy, Default)]
pub struct GioSnapshotProvider;

impl SnapshotProvider for GioSnapshotProvider {
    fn list_collection(
        &self,
        uri: &str,
        limit: usize,
        cancel: &Cancellation,
    ) -> Result<CollectionListing, EntryError> {
        let folder = gio::File::for_uri(&normalise(uri)?);
        let enumerator = folder.enumerate_children(
            ATTRIBUTES,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            Some(cancel.cancellable()),
        )?;
        let listing = read_listing(&enumerator, limit, cancel);
        // Closing only releases the enumerator; it cannot change what was
        // read.
        let _ = enumerator.close(gio::Cancellable::NONE);
        listing
    }

    fn inspect(&self, uri: &str, cancel: &Cancellation) -> Result<Entry, EntryError> {
        let file = gio::File::for_uri(&normalise(uri)?);
        let info = file.query_info(
            ATTRIBUTES,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            Some(cancel.cancellable()),
        )?;
        Ok(entry_from_info(&file, &info))
    }
}

/// Reads up to `limit` entries, plus one more to learn whether the folder
/// holds more than that.
fn read_listing(
    enumerator: &gio::FileEnumerator,
    limit: usize,
    cancel: &Cancellation,
) -> Result<CollectionListing, EntryError> {
    let mut snapshots = Vec::new();
    while snapshots.len() <= limit {
        if cancel.is_cancelled() {
            return Err(EntryError::Cancelled);
        }
        let Some(info) = enumerator.next_file(Some(cancel.cancellable()))? else {
            break;
        };
        snapshots.push(entry_from_info(&enumerator.child(&info), &info));
    }
    let has_more = snapshots.len() > limit;
    snapshots.truncate(limit);
    Ok(CollectionListing { snapshots, has_more })
}
