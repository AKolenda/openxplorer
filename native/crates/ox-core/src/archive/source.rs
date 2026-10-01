// SPDX-License-Identifier: AGPL-3.0-only
//! Where archive bytes come from.
//!
//! Ports the `opener` argument of `Archives` in `v2.0.0:desktop/archives.py`, its
//! production value `archive_stream` in `v2.0.0:desktop/native_opening.py`, and
//! `Archives.opened`, which every listing, preview, check and extraction
//! goes through.

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};

use rustix::fs::OFlags;

use super::gio_reader::GioArchiveReader;
use super::member_names::has_tar_name;
use super::tar::{TarArchive, TarCompression, TarMemberReader};
use super::zip::{MemberReader, ZipArchive, ZipMember};
use super::ArchiveError;
use crate::location::normalise;
use crate::network::local_path;
use crate::private_storage::KernelOpenFlags;
use crate::transfer::Cancellation;

/// ARC-005: the most members the built-in reader accepts.
const MAX_MEMBERS: usize = 100_000;
/// A plain TAR says `ustar` just before this offset.
const TAR_SIGNATURE_END: usize = 262;

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

/// ARC-007: opens archives through GIO. A file with a local path is read
/// directly, and so is an `smb://` archive inside a kernel CIFS mount or
/// the `GVfs` FUSE export of its share ([`local_path`], as
/// `archive_stream` in `v2.0.0:desktop/native_opening.py`); anything else (a share
/// or a phone without a local path) is read in place through a seekable
/// GIO stream, never copied first.
#[derive(Debug, Clone, Copy, Default)]
pub struct GioArchiveOpener;

impl ArchiveOpener for GioArchiveOpener {
    fn open(&self, uri: &str, cancel: &Cancellation) -> Result<Box<dyn ArchiveStream>, ArchiveError> {
        cancel.check()?;
        let uri = normalise(uri)?;
        let Some(path) = local_path(&uri) else {
            let file = gio::File::for_uri(&uri);
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

/// An open ZIP or TAR archive (ARC-022, ARC-024). Both are read into the
/// same member records, so every rule applies to both.
pub(super) enum OpenedArchive {
    Zip(ZipArchive<Box<dyn ArchiveStream>>),
    Tar(TarArchive),
}

impl OpenedArchive {
    /// Every member, in archive order.
    pub(super) fn members(&self) -> &[ZipMember] {
        match self {
            Self::Zip(archive) => archive.members(),
            Self::Tar(archive) => archive.members(),
        }
    }

    /// Starts reading the data of the member at `index`.
    ///
    /// # Errors
    ///
    /// Damaged data or the source's read error.
    pub(super) fn open_member(&mut self, index: usize) -> Result<MemberData<'_>, ArchiveError> {
        Ok(match self {
            Self::Zip(archive) => MemberData::Zip(archive.open_member(index)?),
            Self::Tar(archive) => MemberData::Tar(archive.open_member(index)?),
        })
    }
}

/// The data of one member being read.
pub(super) enum MemberData<'a> {
    Zip(MemberReader<'a>),
    Tar(TarMemberReader<'a>),
}

impl MemberData<'_> {
    /// Reads the next bytes into `buffer`; 0 at the member's end.
    ///
    /// # Errors
    ///
    /// Damaged data or the source's read error.
    pub(super) fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, ArchiveError> {
        match self {
            Self::Zip(reader) => reader.read_chunk(buffer),
            Self::Tar(reader) => reader.read_chunk(buffer),
        }
    }
}

/// Opens the archive at `uri` with `opener` and reads its members: a
/// ZIP's central directory (`Archives.opened`), or every header of a TAR,
/// plain or compressed.
///
/// # Errors
///
/// The opener's error, a damaged archive, a central directory over
/// 32 MiB, or [`ArchiveError::TooManyMembers`].
pub(super) fn open_archive(
    opener: &dyn ArchiveOpener,
    uri: &str,
    cancel: &Cancellation,
) -> Result<OpenedArchive, ArchiveError> {
    let mut stream = opener.open(uri, cancel)?;
    if let Some(compression) = tar_compression(&mut stream)? {
        let archive = TarArchive::open(stream, compression, MAX_MEMBERS, cancel)
            .map_err(|error| error.unless_cancelled(cancel))?;
        return Ok(OpenedArchive::Tar(archive));
    }
    // A TAR this reader does not know (old V7, compress(1)) is not read
    // as a ZIP, whose error would not say what is wrong.
    if has_tar_name(uri.rsplit('/').next().unwrap_or_default()) {
        return Err(ArchiveError::DamagedArchive);
    }
    let archive = ZipArchive::open(stream, cancel)?;
    // ARC-005: an archive with more members is left to an archive manager.
    if archive.members().len() > MAX_MEMBERS {
        return Err(ArchiveError::TooManyMembers);
    }
    Ok(OpenedArchive::Zip(archive))
}

/// The compression of a TAR in `stream`, by its first bytes; `None` for
/// anything else, which is read as a ZIP. The stream is left at its start.
fn tar_compression(stream: &mut Box<dyn ArchiveStream>) -> Result<Option<TarCompression>, ArchiveError> {
    let mut start = Vec::with_capacity(TAR_SIGNATURE_END);
    stream
        .as_mut()
        .take(TAR_SIGNATURE_END as u64)
        .read_to_end(&mut start)?;
    stream.seek(SeekFrom::Start(0))?;
    Ok(TarCompression::detect(&start))
}
