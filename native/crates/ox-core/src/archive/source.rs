// SPDX-License-Identifier: AGPL-3.0-only
//! Where archive bytes come from.
//!
//! Ports the `opener` argument of `Archives` in `desktop/archives.py`, its
//! production value `archive_stream` in `desktop/native_opening.py`, and
//! `Archives.opened`, which every listing, preview, check and extraction
//! goes through.

use std::fs::OpenOptions;
use std::io::{Read, Seek};

use gio::prelude::*;
use rustix::fs::OFlags;

use super::gio_reader::GioArchiveReader;
use super::zip::ZipArchive;
use super::ArchiveError;
use crate::location::normalise;
use crate::private_storage::KernelOpenFlags;
use crate::transfer::Cancellation;

/// ARC-005: the most members the built-in reader accepts.
const MAX_MEMBERS: usize = 100_000;

/// A seekable stream of archive bytes. Reading a ZIP starts at its end, so
/// every source must be able to seek.
pub trait ArchiveStream: Read + Seek {}

impl<T: Read + Seek> ArchiveStream for T {}

/// Opens archives for reading.
///
/// Production uses [`GioArchiveOpener`]. Any
/// `Fn(&str, &Cancellation) -> Result<Box<dyn ArchiveStream>, ArchiveError>`
/// is an opener too, which tests use for archives in memory.
pub trait ArchiveOpener: Send + Sync {
    /// Opens the archive at `uri`.
    ///
    /// # Errors
    ///
    /// The backend's error, or [`ArchiveError::NotSeekable`] for a stream
    /// that cannot seek.
    fn open(&self, uri: &str, cancel: &Cancellation) -> Result<Box<dyn ArchiveStream>, ArchiveError>;
}

impl<F> ArchiveOpener for F
where
    F: Fn(&str, &Cancellation) -> Result<Box<dyn ArchiveStream>, ArchiveError> + Send + Sync,
{
    fn open(&self, uri: &str, cancel: &Cancellation) -> Result<Box<dyn ArchiveStream>, ArchiveError> {
        self(uri, cancel)
    }
}

/// ARC-007: opens archives through GIO. A file with a local path,
/// including the `GVfs` FUSE path of a mounted share, is read directly;
/// anything else (an SMB share or a phone without a FUSE path) is read in
/// place through a seekable GIO stream, never copied first.
///
/// `local_path` in `desktop/native_opening.py` also read an `smb://`
/// archive through a kernel CIFS mount of the same share. Here such an
/// archive is read through `GVfs`, which may have to mount the share
/// first; the CIFS shortcut returns when the mount-table reader of the
/// search service is part of `ox-core`.
#[derive(Debug, Clone, Copy, Default)]
pub struct GioArchiveOpener;

impl ArchiveOpener for GioArchiveOpener {
    fn open(&self, uri: &str, cancel: &Cancellation) -> Result<Box<dyn ArchiveStream>, ArchiveError> {
        cancel.check()?;
        let file = gio::File::for_uri(&normalise(uri)?);
        let Some(path) = file.path() else {
            return Ok(Box::new(GioArchiveReader::open(&file, cancel)?));
        };
        // `O_NONBLOCK`: a FIFO named like an archive fails to read instead
        // of blocking the worker forever. Regular files ignore the flag.
        let local = OpenOptions::new()
            .read(true)
            .kernel_flags(OFlags::NONBLOCK)
            .open(path)?;
        Ok(Box::new(local))
    }
}

/// Opens the archive at `uri` with `opener` and reads its central
/// directory (`Archives.opened`).
///
/// # Errors
///
/// The opener's error, a damaged archive, a central directory over
/// 32 MiB, or [`ArchiveError::TooManyMembers`].
pub(super) fn open_archive(
    opener: &dyn ArchiveOpener,
    uri: &str,
    cancel: &Cancellation,
) -> Result<ZipArchive<Box<dyn ArchiveStream>>, ArchiveError> {
    let stream = opener.open(uri, cancel)?;
    let archive = ZipArchive::open(stream, cancel)?;
    // ARC-005: an archive with more members is left to an archive manager.
    if archive.members().len() > MAX_MEMBERS {
        return Err(ArchiveError::TooManyMembers);
    }
    Ok(archive)
}
