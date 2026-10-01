// SPDX-License-Identifier: AGPL-3.0-only
//! Reading one folder for the index.
//!
//! Ports `index_directory` in `v2.0.0:desktop/gio_backend.py`, which the Python
//! service receives as its `list_directory` function. The service takes
//! any [`FolderReader`]; the app passes [`GioFolderReader`].

use std::mem;

use gio::prelude::*;

use super::error::{check_cancelled, SearchError};
use super::root::HiddenItems;
use super::scan::ListedItem;
use crate::entry::{entry_from_info, EntryError, ATTRIBUTES};
use crate::location::normalise;

/// Items handed to the index at once.
const BATCH_SIZE: usize = 256;

/// Reads the items of one folder, in batches.
pub trait FolderReader: Send + Sync {
    /// Calls `receive` with each batch of the items in `folder`, leaving
    /// out hidden items unless `hidden_items` includes them.
    ///
    /// # Errors
    ///
    /// Why the folder could not be read, [`SearchError::Cancelled`] once
    /// `cancellable` is cancelled, or the first error `receive` returns,
    /// which stops the reading.
    fn read_folder(
        &self,
        folder: &str,
        hidden_items: HiddenItems,
        cancellable: &gio::Cancellable,
        receive: &mut dyn FnMut(Vec<ListedItem>) -> Result<(), SearchError>,
    ) -> Result<(), SearchError>;
}

/// Reads folders through GIO, for local folders and `GVfs` shares alike.
///
/// Safety rules of `index_directory`: metadata only (no file is opened),
/// symlinks are neither followed nor listed, virtual items are left out,
/// and nothing is ever mounted. An unmounted share fails with
/// [`EntryError::NotMounted`], which ends the scan of that root.
#[derive(Debug, Clone, Copy, Default)]
pub struct GioFolderReader;

impl FolderReader for GioFolderReader {
    fn read_folder(
        &self,
        folder: &str,
        hidden_items: HiddenItems,
        cancellable: &gio::Cancellable,
        receive: &mut dyn FnMut(Vec<ListedItem>) -> Result<(), SearchError>,
    ) -> Result<(), SearchError> {
        let file = gio::File::for_uri(&normalise(folder)?);
        let enumerator = file
            .enumerate_children(
                ATTRIBUTES,
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                Some(cancellable),
            )
            .map_err(EntryError::from)?;
        let result = read_batches(&enumerator, hidden_items, cancellable, receive);
        // Closing only releases the enumerator; the listing's own result is
        // what the scan reports.
        let _ = enumerator.close(gio::Cancellable::NONE);
        result
    }
}

/// Reads every item of `enumerator` and hands them over in batches.
fn read_batches(
    enumerator: &gio::FileEnumerator,
    hidden_items: HiddenItems,
    cancellable: &gio::Cancellable,
    receive: &mut dyn FnMut(Vec<ListedItem>) -> Result<(), SearchError>,
) -> Result<(), SearchError> {
    let mut batch = Vec::with_capacity(BATCH_SIZE);
    loop {
        check_cancelled(cancellable)?;
        let Some(info) = enumerator
            .next_file(Some(cancellable))
            .map_err(EntryError::from)?
        else {
            break;
        };
        if hidden_items == HiddenItems::Skip && info.is_hidden() {
            continue;
        }
        let item = ListedItem::from(entry_from_info(&enumerator.child(&info), &info));
        if item.is_indexable() {
            batch.push(item);
        }
        if batch.len() >= BATCH_SIZE {
            receive(mem::take(&mut batch))?;
        }
    }
    if !batch.is_empty() {
        receive(batch)?;
    }
    Ok(())
}
