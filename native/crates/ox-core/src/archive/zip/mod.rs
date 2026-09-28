// SPDX-License-Identifier: AGPL-3.0-only
//! A read-only ZIP reader with the behaviour of Python's `zipfile`, which
//! `desktop/archives.py` and `desktop/zip_extraction.py` rely on.
//!
//! The archive services refuse members by the exact rules of the Python
//! app, so the reader reports what `zipfile` reports: every member of the
//! central directory in order (duplicates included, which general-purpose
//! ZIP crates merge), the name before and after `zipfile`'s clean-up, and
//! the raw Unix mode, flags and method. Decompression uses the `flate2`,
//! `bzip2` and `lzma-rust2` crates.
//!
//! | Module | Responsibility |
//! |---|---|
//! | `directory` | Finding and reading the central directory (`_RealGetContents`) |
//! | `extra` | ZIP64 sizes and Info-ZIP Unicode names (`_decodeExtra`) |
//! | `member` | One member's metadata (`ZipInfo`) |
//! | `names` | Name decoding: UTF-8 or code page 437, cut at NUL |
//! | `python_repr` | Names in messages, quoted as Python's `repr` quotes them |
//! | `reader` | Local header checks, decompression and CRC (`ZipExtFile`) |
//! | `records` | The fixed-size ZIP records and their fields |
//! | `error` | [`ZipFormatError`]: damaged or unsupported archives |

mod directory;
mod error;
mod extra;
mod member;
mod names;
mod python_repr;
mod reader;
mod records;

use std::io::{Read, Seek};

pub use error::{Zip64Field, ZipFormatError};
pub(crate) use member::{MemberFileType, ZipMember};
pub(crate) use reader::MemberReader;

use crate::archive::ArchiveError;
use crate::transfer::Cancellation;
use directory::read_directory;

/// An open ZIP archive: its members, and the source to read their data
/// from.
#[derive(Debug)]
pub(crate) struct ZipArchive<S> {
    source: S,
    members: Vec<ZipMember>,
    /// Added to a recorded offset, gives the file position.
    shift: i128,
    cancel: Cancellation,
}

impl<S: Read + Seek> ZipArchive<S> {
    /// Reads the central directory of the archive in `source`. A failure
    /// after the user cancelled is reported as the cancellation: GIO aborts
    /// a cancelled read with an error of its own.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::DirectoryTooLarge`], a [`ZipFormatError`], the
    /// source's read error or [`ArchiveError::Cancelled`].
    pub(crate) fn open(mut source: S, cancel: &Cancellation) -> Result<Self, ArchiveError> {
        let directory = read_directory(&mut source).map_err(|error| error.unless_cancelled(cancel))?;
        Ok(Self {
            source,
            members: directory.members,
            shift: directory.shift,
            cancel: cancel.clone(),
        })
    }

    /// Every member, in central directory order.
    pub(crate) fn members(&self) -> &[ZipMember] {
        &self.members
    }

    /// Starts reading the data of the member at `index` in
    /// [`Self::members`].
    ///
    /// # Errors
    ///
    /// See [`MemberReader`]; a local header before the start of the file is
    /// [`ZipFormatError::BadHeaderSignature`].
    ///
    /// # Panics
    ///
    /// When `index` is not an index of [`Self::members`].
    pub(crate) fn open_member(&mut self, index: usize) -> Result<MemberReader<'_>, ArchiveError> {
        let member = &self.members[index];
        let position = i128::from(member.header_offset) + self.shift;
        let header_position = u64::try_from(position).map_err(|_| ZipFormatError::BadHeaderSignature)?;
        MemberReader::open(&mut self.source, member, header_position, &self.cancel)
    }
}
