// SPDX-License-Identifier: AGPL-3.0-only
//! Folder-size metadata of local folders, read with `lstat`.
//!
//! Ports `LocalSizeProvider` in `desktop/folder_sizes.py`. Local scans
//! read the file system directly rather than through GIO, because only
//! `lstat` gives the device and inode that let hard links count once, and
//! the mount table shows mount points that share their parent's device.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{self, FileType, Metadata};
use std::io;
use std::ops::ControlFlow;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;

use super::mounts::{read_mount_points, MOUNT_TABLE};
use super::{FileIdentity, SizeEntry, SizeEntryKind, SizeProvider};
use crate::entry::EntryError;
use crate::location::{file_uri, split_location, LocationKind};
use crate::transfer::Cancellation;

/// The [`SizeProvider`] for `file://` locations. Never follows a link: every
/// item is read with `lstat`.
///
/// There is no `Default`: a provider that knew no mount points would enter
/// every filesystem mounted inside the scanned folder.
#[derive(Debug, Clone)]
pub struct LocalSizeProvider {
    /// Folders with another filesystem mounted on them.
    mount_points: HashSet<PathBuf>,
}

impl LocalSizeProvider {
    /// A provider that knows the mount points of this process, read from
    /// its mount table now.
    ///
    /// # Errors
    ///
    /// Why the mount table (`/proc/self/mountinfo`) could not be read.
    pub fn new() -> Result<Self, EntryError> {
        let mount_points = read_mount_points().map_err(|error| read_error(Path::new(MOUNT_TABLE), &error))?;
        Ok(Self { mount_points })
    }

    /// A provider that treats exactly `mount_points` as mount points, for a
    /// caller that has read the mount table already.
    pub fn with_mount_points(mount_points: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            mount_points: mount_points.into_iter().collect(),
        }
    }

    /// The scan's view of the item at `path` with the `lstat` result
    /// `metadata`.
    fn size_entry(&self, path: &Path, metadata: &Metadata) -> SizeEntry {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        SizeEntry {
            uri: file_uri(path),
            name: name.into_owned(),
            kind: kind_of(metadata.file_type()),
            size: Some(metadata.len()),
            filesystem: Some(metadata.dev().to_string()),
            identity: Some(FileIdentity {
                device: metadata.dev(),
                inode: metadata.ino(),
            }),
            is_mount_point: self.mount_points.contains(path),
        }
    }
}

impl SizeProvider for LocalSizeProvider {
    fn inspect(&self, uri: &str, cancel: &Cancellation) -> Result<SizeEntry, EntryError> {
        if cancel.is_cancelled() {
            return Err(EntryError::Cancelled);
        }
        let path = local_path(uri)?;
        let metadata = fs::symlink_metadata(&path).map_err(|error| read_error(&path, &error))?;
        Ok(self.size_entry(&path, &metadata))
    }

    fn visit_children(
        &self,
        folder_uri: &str,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(SizeEntry) -> ControlFlow<()>,
    ) -> Result<(), EntryError> {
        let folder = local_path(folder_uri)?;
        let children = fs::read_dir(&folder).map_err(|error| read_error(&folder, &error))?;
        for child in children {
            if cancel.is_cancelled() {
                return Err(EntryError::Cancelled);
            }
            let child = child.map_err(|error| read_error(&folder, &error))?;
            // Safety rule PROP-028: `DirEntry::metadata` is `lstat`, so a
            // link is never followed.
            let entry = match child.metadata() {
                Ok(metadata) => self.size_entry(&child.path(), &metadata),
                Err(_) => SizeEntry::unreadable(child.file_name().to_string_lossy()),
            };
            if visit(entry).is_break() {
                break;
            }
        }
        Ok(())
    }
}

/// What an `lstat` file type is to the scan.
fn kind_of(file_type: FileType) -> SizeEntryKind {
    if file_type.is_symlink() {
        SizeEntryKind::Symlink
    } else if file_type.is_dir() {
        SizeEntryKind::Folder
    } else if file_type.is_file() {
        SizeEntryKind::File
    } else {
        SizeEntryKind::Other
    }
}

/// The local path of a `file://` URI, decoded to bytes so that names that
/// are not UTF-8 are found too.
///
/// Another scheme is refused rather than read as a local path: the path
/// of `smb://nas/share` is not `/share` on this computer.
fn local_path(uri: &str) -> Result<PathBuf, EntryError> {
    let parts = split_location(uri)?;
    if parts.kind() != LocationKind::Local {
        return Err(EntryError::NotSupported(format!(
            "{uri} is not on this computer."
        )));
    }
    let bytes: Vec<u8> = percent_decode_str(&parts.path).collect();
    Ok(PathBuf::from(OsString::from_vec(bytes)))
}

/// An I/O error on `path`, sorted like the GIO errors of other locations
/// and naming the path, as Python's `OSError` does.
fn read_error(path: &Path, error: &io::Error) -> EntryError {
    let message = format!("{error}: {}", path.display());
    match error.kind() {
        io::ErrorKind::NotFound => EntryError::NotFound(message),
        io::ErrorKind::PermissionDenied => EntryError::PermissionDenied(message),
        io::ErrorKind::NotADirectory => EntryError::NotDirectory(message),
        _ => EntryError::Other(message),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::ffi::OsStrExt;

    use super::*;

    /// Unlike the Python provider, which decoded the path as UTF-8, a
    /// folder whose name is not UTF-8 is found.
    #[test]
    fn a_path_that_is_not_utf8_is_decoded_to_its_bytes() {
        let path = local_path("file:///srv/caf%E9/a%20b").expect("a local URI");

        assert_eq!(path.as_os_str().as_bytes(), b"/srv/caf\xe9/a b");
    }

    #[test]
    fn a_location_that_is_not_local_is_not_read_as_a_path() {
        let refusal = local_path("smb://nas/share").unwrap_err();

        assert_eq!(refusal.code(), "not-supported");
    }

    #[test]
    fn io_errors_name_the_path_and_keep_their_kind() {
        let path = Path::new("/srv/missing");
        let error = io::Error::from(io::ErrorKind::NotFound);

        let read = read_error(path, &error);

        assert_eq!(read.code(), "not-found");
        assert!(read.to_string().ends_with(": /srv/missing"), "{read}");
    }
}
