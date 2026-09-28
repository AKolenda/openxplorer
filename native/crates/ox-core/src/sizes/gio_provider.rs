// SPDX-License-Identifier: AGPL-3.0-only
//! Folder-size metadata of network shares and other GIO locations
//! (PROP-030).
//!
//! Ports `GioSizeProvider` in `desktop/folder_sizes.py`: only the
//! attributes below are queried, never file contents, and links are not
//! followed. GIO reports no inode, so hard links on these locations are
//! counted once per name.

use std::ops::ControlFlow;

use gio::prelude::*;

use super::{SizeEntry, SizeEntryKind, SizeProvider};
use crate::entry::EntryError;
use crate::transfer::Cancellation;

/// The metadata a scan reads; `id::filesystem` tells a subfolder on
/// another filesystem apart.
const ATTRIBUTES: &str = "standard::name,standard::type,standard::size,standard::is-symlink,id::filesystem";

/// The [`SizeProvider`] for `smb://` and other GIO locations. It blocks, so
/// call it on a worker thread.
#[derive(Debug, Clone, Copy, Default)]
pub struct GioSizeProvider;

impl SizeProvider for GioSizeProvider {
    fn inspect(&self, uri: &str, cancel: &Cancellation) -> Result<SizeEntry, EntryError> {
        let file = gio::File::for_uri(uri);
        let info = file.query_info(
            ATTRIBUTES,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            Some(cancel.cancellable()),
        )?;
        Ok(size_entry(&file, &info))
    }

    fn visit_children(
        &self,
        folder_uri: &str,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(SizeEntry) -> ControlFlow<()>,
    ) -> Result<(), EntryError> {
        let folder = gio::File::for_uri(folder_uri);
        let enumerator = folder.enumerate_children(
            ATTRIBUTES,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            Some(cancel.cancellable()),
        )?;
        let visited = visit_all(&enumerator, cancel, visit);
        // Closing only releases the enumerator; it cannot change what was
        // read.
        let _ = enumerator.close(gio::Cancellable::NONE);
        visited
    }
}

/// Passes every remaining item of `enumerator` to `visit`, until it
/// breaks.
fn visit_all(
    enumerator: &gio::FileEnumerator,
    cancel: &Cancellation,
    visit: &mut dyn FnMut(SizeEntry) -> ControlFlow<()>,
) -> Result<(), EntryError> {
    loop {
        if cancel.is_cancelled() {
            return Err(EntryError::Cancelled);
        }
        let Some(info) = enumerator.next_file(Some(cancel.cancellable()))? else {
            return Ok(());
        };
        let entry = size_entry(&enumerator.child(&info), &info);
        if visit(entry).is_break() {
            return Ok(());
        }
    }
}

/// The scan's view of `file` from its queried `info`.
fn size_entry(file: &gio::File, info: &gio::FileInfo) -> SizeEntry {
    let size = if info.has_attribute("standard::size") {
        u64::try_from(info.size()).ok()
    } else {
        None
    };
    let filesystem = info
        .attribute_string("id::filesystem")
        .filter(|filesystem| !filesystem.is_empty());
    SizeEntry {
        uri: file.uri().into(),
        name: info.name().to_string_lossy().into_owned(),
        kind: kind_of(info),
        size,
        filesystem: filesystem.map(String::from),
        identity: None,
        is_mount_point: false,
    }
}

/// What a queried item is to the scan. A link is a link whatever GIO says
/// its type is.
fn kind_of(info: &gio::FileInfo) -> SizeEntryKind {
    // The typed accessors log a critical warning for an attribute the
    // backend did not report; a missing type is unknown and a missing link
    // flag reads as false.
    let file_type = if info.has_attribute("standard::type") {
        info.file_type()
    } else {
        gio::FileType::Unknown
    };
    let is_symlink = info.boolean("standard::is-symlink");
    // Safety rule PROP-028: a link is never followed, whatever type GIO
    // reports for its target.
    if is_symlink || file_type == gio::FileType::SymbolicLink {
        return SizeEntryKind::Symlink;
    }
    match file_type {
        gio::FileType::Directory | gio::FileType::Mountable => SizeEntryKind::Folder,
        gio::FileType::Regular => SizeEntryKind::File,
        _ => SizeEntryKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Info for an item of `file_type`, filled in by hand.
    fn info(file_type: gio::FileType) -> gio::FileInfo {
        let info = gio::FileInfo::new();
        info.set_file_type(file_type);
        info.set_name("item");
        info
    }

    /// parity: PROP-030
    #[test]
    fn shares_are_folders_and_links_are_never_folders() {
        let link_to_folder = info(gio::FileType::Directory);
        link_to_folder.set_is_symlink(true);

        assert_eq!(kind_of(&info(gio::FileType::Mountable)), SizeEntryKind::Folder);
        assert_eq!(kind_of(&link_to_folder), SizeEntryKind::Symlink);
        assert_eq!(kind_of(&info(gio::FileType::Special)), SizeEntryKind::Other);
        assert_eq!(kind_of(&info(gio::FileType::Regular)), SizeEntryKind::File);
    }

    /// parity: PROP-030, PROP-028
    #[test]
    fn a_size_the_backend_does_not_report_stays_unknown() {
        let file = gio::File::for_path("/share/item");
        let reported = info(gio::FileType::Regular);
        reported.set_size(12);
        reported.set_attribute_string("id::filesystem", "smb-share:server=nas,share=work");

        let unreported = size_entry(&file, &info(gio::FileType::Regular));
        let known = size_entry(&file, &reported);

        assert_eq!(unreported.size, None);
        assert_eq!(unreported.filesystem, None);
        assert_eq!(known.size, Some(12));
        assert_eq!(
            known.filesystem.as_deref(),
            Some("smb-share:server=nas,share=work")
        );
    }
}
