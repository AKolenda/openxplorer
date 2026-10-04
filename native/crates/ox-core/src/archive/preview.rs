// SPDX-License-Identifier: AGPL-3.0-only
//! Opening one archive member as a private, read-only copy. Ports
//! `Archives.preview_member` in `v2.0.0:desktop/archives.py`.
//!
//! Safety and privacy rules (ARC-006):
//! - Only the one member the user chose is decompressed; nothing else of
//!   the archive reaches the disk, and archive paths are never used.
//! - Only documents, images and media are opened, never scripts,
//!   executables or templates. Encrypted members, links, special files and
//!   members with altered names are refused.
//! - A member over 256 MiB, or compressed more than 1,000 times, is
//!   refused before anything is written, and decompression stops at
//!   256 MiB whatever the member declares.
//! - The copy is written to a new folder only the user can open (`0700`)
//!   inside the private preview root, then made read-only (`0400`). It is
//!   never written back into the archive.

use std::fs::{self, DirBuilder, File, OpenOptions, Permissions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use gio::prelude::*;

use super::browse::ArchiveBrowser;
use super::member_names::{is_previewable, is_safe_member};
use super::source::{open_archive, OpenedArchive};
use super::worker::on_worker;
use super::zip::{MemberFileType, ZipMember};
use super::ArchiveError;
use crate::private_storage::private_directory;
use crate::random::{random_hex, NAME_BYTES};
use crate::transfer::{Cancellation, TransferError, PRIVATE_DIRECTORY_MODE};

/// The largest member that can be opened (256 MiB).
pub(crate) const MAX_PREVIEW_BYTES: u64 = 256 * 1024 * 1024;
/// The highest compression ratio of a member that can be opened.
const MAX_PREVIEW_RATIO: u64 = 1000;
/// The chunk size for decompressing.
const CHUNK_BYTES: usize = 64 * 1024;
/// The copy's mode while it is written: owner only.
const WRITABLE_COPY_MODE: u32 = 0o600;
/// The copy's final mode: read-only, owner only.
const READ_ONLY_COPY_MODE: u32 = 0o400;
/// The start of each copy's private folder name.
const PREVIEW_FOLDER_PREFIX: &str = "winspace-zip-";

/// How long a copy opened from an archive is kept. The preview root is in
/// the runtime folder, which lives in memory until logout, so copies are
/// removed this long after they were made: by then the application that
/// opened one has read it, and on Linux removing a file an application
/// has open does not disturb it.
pub const PREVIEW_LIFETIME: Duration = Duration::from_secs(10 * 60);

/// The notice to show after opening a copy.
pub const PREVIEW_NOTICE: &str =
    "Opened a read-only temporary copy. Changes are NOT saved back into the ZIP.";

/// A member copied out of an archive to open with its default application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewCopy {
    /// The read-only copy.
    pub path: PathBuf,
    /// The member it was copied from.
    pub member: String,
}

impl PreviewCopy {
    /// The copy's `file://` URI.
    pub fn uri(&self) -> String {
        gio::File::for_path(&self.path).uri().to_string()
    }
}

impl ArchiveBrowser {
    /// Copies the member `member` of the archive at `uri` into a new
    /// private folder, read-only, and returns the copy. Blocking; see
    /// [`Self::preview_member_in_background`].
    ///
    /// # Errors
    ///
    /// A refusal from the rules in the module documentation, the errors of
    /// reading the archive or writing the copy, or
    /// [`ArchiveError::Cancelled`]. Nothing is left behind on failure.
    pub fn preview_member(
        &self,
        uri: &str,
        member: &str,
        cancel: &Cancellation,
    ) -> Result<PreviewCopy, ArchiveError> {
        if !is_safe_member(member) || member.ends_with('/') {
            return Err(ArchiveError::NotARegularMember);
        }
        let file_name = member.rsplit_once('/').map_or(member, |(_folder, name)| name);
        if !is_previewable(file_name) {
            return Err(ArchiveError::UnsafePreviewType);
        }
        let mut archive = open_archive(self.opener.as_ref(), uri, cancel)?;
        let index = unique_member(archive.members(), member)?;
        check_previewable(&archive.members()[index])?;
        remove_old_previews(&self.preview_root, PREVIEW_LIFETIME);
        let folder = create_preview_folder(&self.preview_root)?;
        let copy_path = folder.join(file_name);
        match write_copy(&mut archive, index, &copy_path, cancel) {
            Ok(()) => Ok(PreviewCopy {
                path: copy_path,
                member: member.to_owned(),
            }),
            Err(error) => {
                remove_preview_folder(&folder, &copy_path);
                Err(error)
            }
        }
    }

    /// [`Self::preview_member`] on a GIO worker thread, for the main loop
    /// to await.
    ///
    /// # Errors
    ///
    /// See [`Self::preview_member`].
    pub async fn preview_member_in_background(
        &self,
        uri: String,
        member: String,
        cancel: Cancellation,
    ) -> Result<PreviewCopy, ArchiveError> {
        let browser = self.clone();
        on_worker(move || browser.preview_member(&uri, &member, &cancel)).await
    }
}

/// The index of the one member named `member`; a missing or duplicated
/// name is refused, so the copy is never taken from the wrong member.
fn unique_member(members: &[ZipMember], member: &str) -> Result<usize, ArchiveError> {
    let mut matches = members
        .iter()
        .enumerate()
        .filter(|(_, candidate)| candidate.name == member)
        .map(|(index, _)| index);
    match (matches.next(), matches.next()) {
        (Some(index), None) => Ok(index),
        _ => Err(ArchiveError::MissingOrDuplicatedMember),
    }
}

/// Refuses encrypted members, altered names, links, special files, and
/// members too large or too highly compressed to decompress safely.
fn check_previewable(member: &ZipMember) -> Result<(), ArchiveError> {
    if member.is_encrypted() {
        return Err(ArchiveError::EncryptedMember);
    }
    let is_regular = matches!(
        member.file_type(),
        MemberFileType::Unrecorded | MemberFileType::Regular
    );
    let ratio_limit = member.compressed_size.max(1).saturating_mul(MAX_PREVIEW_RATIO);
    let is_too_large = member.size > MAX_PREVIEW_BYTES || member.size > ratio_limit;
    if !member.has_unaltered_name() || !is_regular || is_too_large {
        return Err(ArchiveError::MemberNotPreviewable);
    }
    Ok(())
}

/// Creates a new owner-only folder for one copy inside `root`. The root
/// itself must be a private folder of this user, not a link.
fn create_preview_folder(root: &Path) -> Result<PathBuf, ArchiveError> {
    private_directory(root).map_err(|error| TransferError::failed(error.to_string()))?;
    let digits = random_hex(NAME_BYTES)?;
    let folder = root.join(format!("{PREVIEW_FOLDER_PREFIX}{digits}"));
    DirBuilder::new().mode(PRIVATE_DIRECTORY_MODE).create(&folder)?;
    // The umask may have removed bits; the folder must be exactly 0700.
    fs::set_permissions(&folder, Permissions::from_mode(PRIVATE_DIRECTORY_MODE))?;
    Ok(folder)
}

/// Decompresses the member at `index` into a new file at `copy_path` and
/// makes it read-only.
fn write_copy(
    archive: &mut OpenedArchive,
    index: usize,
    copy_path: &Path,
    cancel: &Cancellation,
) -> Result<(), ArchiveError> {
    let mut copy = create_copy_file(copy_path)?;
    let mut member = archive.open_member(index)?;
    let mut buffer = vec![0u8; CHUNK_BYTES];
    let mut total = 0u64;
    loop {
        cancel.check()?;
        let count = member.read_chunk(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_PREVIEW_BYTES {
            return Err(ArchiveError::PreviewLimitReached);
        }
        copy.write_all(&buffer[..count])?;
    }
    copy.set_permissions(Permissions::from_mode(READ_ONLY_COPY_MODE))?;
    Ok(())
}

/// Creates the copy as a new owner-only file; an existing name, even a
/// link, is never opened.
fn create_copy_file(copy_path: &Path) -> Result<File, ArchiveError> {
    let copy = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(WRITABLE_COPY_MODE)
        .open(copy_path)?;
    copy.set_permissions(Permissions::from_mode(WRITABLE_COPY_MODE))?;
    Ok(copy)
}

/// Removes the copies in the preview root `root` older than `lifetime`:
/// at each new copy, and when the app starts and quits. Best effort and
/// blocking; only preview folders are touched.
pub fn remove_old_previews(root: &Path, lifetime: Duration) {
    super::copies::remove_old_folders(root, PREVIEW_FOLDER_PREFIX, lifetime);
}

/// Removes the copy at `path` with its private folder, once the
/// application it was opened in has had time to read it. Anything but a
/// copy in a preview folder is left alone.
pub fn remove_preview_copy(path: &Path) {
    let Some(folder) = path.parent() else {
        return;
    };
    let is_preview_folder = folder
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(PREVIEW_FOLDER_PREFIX));
    let is_folder = folder.symlink_metadata().is_ok_and(|metadata| metadata.is_dir());
    if is_preview_folder && is_folder {
        let _ = fs::remove_dir_all(folder);
    }
}

/// Removes a failed copy and its folder. The first error is the one the
/// user needs, so a failure here is not reported; the leftover is inside
/// the private preview root.
fn remove_preview_folder(folder: &Path, copy_path: &Path) {
    let _ = fs::remove_file(copy_path);
    let _ = fs::remove_dir(folder);
}
