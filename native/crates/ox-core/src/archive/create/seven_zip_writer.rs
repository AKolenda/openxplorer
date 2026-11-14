// SPDX-License-Identifier: AGPL-3.0-only
//! The `.7z` format, as Windows 11's Compress to › 7z File writes it: each
//! file an LZMA2 stream of its own, then the 7z headers, written by the
//! `sevenz-rust2` crate.
//!
//! A 7z archive starts with a header that says where its last header is,
//! so the writer goes back to the start once every entry is written: the
//! output must be seekable. A local file is, and so is a file on a share
//! that GVfs writes; an output that is not is refused before anything is
//! compressed.

use std::io::{self, Read, Seek, SeekFrom, Write};

use gio::prelude::*;
use sevenz_rust2::encoder_options::Lzma2Options;
use sevenz_rust2::{ArchiveEntry, ArchiveWriter, NtTime};

use super::super::ArchiveError;
use crate::transfer::Cancellation;

/// The level a 7z gets unless one is asked for: 7-Zip's "Normal".
const DEFAULT_LEVEL: u32 = 5;

/// Windows' attribute of a folder.
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
/// The attribute 7-Zip and p7zip set when the high 16 bits hold the item's
/// Unix mode.
const FILE_ATTRIBUTE_UNIX_EXTENSION: u32 = 0x8000;
/// Hundreds of nanoseconds in a second, the unit of a 7z time.
const NT_TICKS_PER_SECOND: u64 = 10_000_000;

/// A new 7z archive being written into a GIO output stream.
pub(super) struct SevenZipWriter {
    archive: ArchiveWriter<SeekableOutput>,
}

impl SevenZipWriter {
    /// A writer into `output`, compressing at `level` (1 to 9).
    ///
    /// # Errors
    ///
    /// [`ArchiveError::SevenZipNotSeekable`] when `output` cannot seek back to the
    /// start, or the output's error.
    pub(super) fn new(output: &gio::FileOutputStream, level: Option<u32>) -> Result<Self, ArchiveError> {
        if !output.can_seek() {
            return Err(ArchiveError::SevenZipNotSeekable);
        }
        let mut archive = ArchiveWriter::new(SeekableOutput(output.clone())).map_err(seven_zip_error)?;
        let options = Lzma2Options::from_level(level.unwrap_or(DEFAULT_LEVEL));
        archive.set_content_methods(vec![options.into()]);
        // No password, so nothing to encrypt: the header stays readable.
        archive.set_encrypt_header(false);
        Ok(Self { archive })
    }

    /// Adds the folder `name` (`Photos/`), modified at `modified` seconds
    /// since the Unix epoch, with the `rwx` bits `mode`.
    pub(super) fn add_folder(
        &mut self,
        name: &str,
        (modified, mode): (u64, u32),
    ) -> Result<(), ArchiveError> {
        let mut entry = ArchiveEntry::new_directory(name.trim_end_matches('/'));
        describe(
            &mut entry,
            modified,
            FILE_ATTRIBUTE_DIRECTORY | unix_mode(0o040_000 | mode),
        );
        self.archive
            .push_archive_entry::<&[u8]>(entry, None)
            .map_err(seven_zip_error)?;
        Ok(())
    }

    /// Adds the file `name` with `content`, reporting how much of it is
    /// read to `progress`; stops when `cancel` is.
    pub(super) fn add_file(
        &mut self,
        name: &str,
        (modified, mode): (u64, u32),
        content: &mut dyn Read,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), ArchiveError> {
        cancel.check()?;
        let mut entry = ArchiveEntry::new_file(name);
        describe(&mut entry, modified, unix_mode(0o100_000 | mode));
        let reader = WatchedReader {
            content,
            cancel,
            progress,
            read: 0,
        };
        let pushed = self.archive.push_archive_entry(entry, Some(reader));
        // A cancellation surfaces as the reader's error; report it as one.
        cancel.check()?;
        pushed.map_err(seven_zip_error)?;
        Ok(())
    }

    /// Writes the headers and goes back to write the start header.
    pub(super) fn finish(self) -> Result<(), ArchiveError> {
        let mut output = self.archive.finish()?;
        output.flush()?;
        Ok(())
    }
}

/// Gives `entry` its time and attributes.
fn describe(entry: &mut ArchiveEntry, modified: u64, attributes: u32) {
    let ticks = modified.saturating_mul(NT_TICKS_PER_SECOND);
    entry.last_modified_date = NtTime::new(u64::from(NtTime::UNIX_EPOCH).saturating_add(ticks));
    entry.has_last_modified_date = true;
    entry.windows_attributes = attributes;
    entry.has_windows_attributes = true;
}

/// The attribute bits that carry the Unix `mode` (type and permissions).
fn unix_mode(mode: u32) -> u32 {
    FILE_ATTRIBUTE_UNIX_EXTENSION | ((mode & 0xffff) << 16)
}

/// A crate error as the archive error it stands for.
fn seven_zip_error(error: sevenz_rust2::Error) -> ArchiveError {
    match error {
        sevenz_rust2::Error::Io(error, _) => error.into(),
        other => io::Error::other(other.to_string()).into(),
    }
}

/// A file's content as the 7z writer reads it: each read checks for a
/// cancellation and reports how much is read.
struct WatchedReader<'a> {
    content: &'a mut dyn Read,
    cancel: &'a Cancellation,
    progress: &'a mut dyn FnMut(u64),
    read: u64,
}

impl Read for WatchedReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.cancel.check().map_err(io::Error::other)?;
        let count = self.content.read(buffer)?;
        self.read += count as u64;
        (self.progress)(self.read);
        Ok(count)
    }
}

/// A GIO output stream that can seek, as [`ArchiveWriter`] needs.
struct SeekableOutput(gio::FileOutputStream);

impl Write for SeekableOutput {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self
            .0
            .write(buffer, gio::Cancellable::NONE)
            .map_err(io::Error::other)?;
        usize::try_from(written).map_err(io::Error::other)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush(gio::Cancellable::NONE).map_err(io::Error::other)
    }
}

impl Seek for SeekableOutput {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let (offset, from) = match position {
            SeekFrom::Start(offset) => (
                i64::try_from(offset).map_err(io::Error::other)?,
                glib::SeekType::Set,
            ),
            SeekFrom::Current(offset) => (offset, glib::SeekType::Cur),
            SeekFrom::End(offset) => (offset, glib::SeekType::End),
        };
        self.0
            .seek(offset, from, gio::Cancellable::NONE)
            .map_err(io::Error::other)?;
        u64::try_from(self.0.tell()).map_err(io::Error::other)
    }
}
