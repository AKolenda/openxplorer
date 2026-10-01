// SPDX-License-Identifier: AGPL-3.0-only
//! Validating the destination of a standard folder.
//!
//! Ports `FolderLocations.validate` in `v2.0.0:desktop/folder_locations.py`. The
//! checks run in the Python order, so the same destination fails with the
//! same message in both apps.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use gio::prelude::*;
use rustix::fs::Access;

use super::RelocationError;
use crate::location::{file_uri, is_smb_location, normalise_location};
use crate::network::{mount_for_path, resolve_smb_path, MountEntry};
use crate::places::KnownFolder;

/// Folders that are cleared at boot or belong to one login.
pub(super) const TEMPORARY_ROOTS: [&str; 3] = ["/run", "/tmp", "/var/tmp"];

/// What a destination is checked against.
#[derive(Debug, Clone, Copy)]
pub(super) struct Surroundings<'a> {
    /// The user's home folder, which `~` means and which is refused.
    pub home: &'a Path,
    /// The mount table.
    pub mounts: &'a [MountEntry],
    /// The folders a destination must not lie in: [`TEMPORARY_ROOTS`],
    /// except in tests, whose folders are all temporary.
    pub temporary_roots: &'a [PathBuf],
}

/// A destination that passed every check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedLocation {
    /// The folder to move.
    pub folder: KnownFolder,
    /// The destination, with every symlink resolved.
    pub path: PathBuf,
    /// Its `file:` URI.
    pub uri: String,
    /// Whether it lies on a kernel SMB mount.
    pub is_network: bool,
    /// What is mounted there, such as `//nas/share`, when it lies on a
    /// mount.
    pub source: Option<String>,
    /// Where the folder is now.
    pub previous: PathBuf,
}

impl CheckedLocation {
    /// The checked `path` of `folder`, described with the mount it lies on.
    pub(super) fn new(folder: KnownFolder, path: PathBuf, mounts: &[MountEntry], previous: PathBuf) -> Self {
        let mount = mount_for_path(&path.to_string_lossy(), mounts);
        Self {
            folder,
            uri: file_uri(&path),
            is_network: mount.is_some_and(MountEntry::is_smb),
            source: mount.map(|mount| mount.source.clone()),
            path,
            previous,
        }
    }
}

/// The real path of destination `value`, if it can hold a standard folder.
///
/// # Errors
///
/// The [`RelocationError`] of the first rule it breaks.
pub(super) fn destination(value: &str, surroundings: Surroundings<'_>) -> Result<PathBuf, RelocationError> {
    let Surroundings {
        home,
        mounts,
        temporary_roots,
    } = surroundings;
    let uri = normalise_location(value, None, home)?;
    let requested = local_path(&uri, mounts)?;
    // Safety rule "only persistent destinations": the real path is
    // checked, so a symlink into a temporary GVfs session is caught
    // (`target.resolve(strict=True)`).
    let real = fs::canonicalize(&requested).map_err(unresolvable)?;
    if temporary_roots.iter().any(|root| real.starts_with(root)) {
        return Err(RelocationError::Temporary);
    }
    let mount = mount_for_path(&real.to_string_lossy(), mounts);
    if mount.is_some_and(|mount| mount.filesystem.contains("gvfs")) {
        return Err(RelocationError::GvfsSession);
    }
    if !real.is_dir() {
        return Err(RelocationError::NotAFolder);
    }
    if is_home_or_root(&real, home) {
        return Err(RelocationError::WholeHomeOrRoot);
    }
    if rustix::fs::access(&real, Access::WRITE_OK | Access::EXEC_OK).is_err() {
        return Err(RelocationError::NotWritable);
    }
    Ok(real)
}

/// The local path `uri` names: a `file:` path, or the path of an `smb://`
/// folder inside a kernel mount of its share.
///
/// Python took the path part of any other URL as a local path, so
/// `sftp://host/srv` checked the local `/srv`; here other schemes are not
/// a folder.
fn local_path(uri: &str, mounts: &[MountEntry]) -> Result<PathBuf, RelocationError> {
    if is_smb_location(uri) {
        // Safety rule "a network folder needs a stable kernel mount": a
        // GVfs share or a bookmark disappears at logout.
        return resolve_smb_path(uri, mounts)?.ok_or(RelocationError::NotMounted);
    }
    gio::File::for_uri(uri).path().ok_or(RelocationError::NotAFolder)
}

/// A missing destination is not a folder; anything else could not be read.
fn unresolvable(error: io::Error) -> RelocationError {
    if error.kind() == io::ErrorKind::NotFound {
        RelocationError::NotAFolder
    } else {
        RelocationError::Unreadable(error)
    }
}

/// Whether `path` is the home folder or `/`.
fn is_home_or_root(path: &Path, home: &Path) -> bool {
    let real_home = fs::canonicalize(home).unwrap_or_else(|_| home.to_owned());
    path == real_home || path == Path::new("/")
}
