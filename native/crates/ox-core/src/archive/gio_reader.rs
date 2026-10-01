// SPDX-License-Identifier: AGPL-3.0-only
//! Reading an archive in place through a seekable GIO stream, for shares
//! and devices without a local path. Ports `GioReader` in
//! `v2.0.0:desktop/native_opening.py`.
//!
//! GIO may return fewer bytes than asked for; as a [`Read`] that is simply
//! a short read, which the ZIP reader completes with further reads. Every
//! read and seek first checks the user's cancellation and passes it to GIO,
//! so a slow share can be cancelled mid-read.

use std::io::{self, Read, Seek, SeekFrom};

use gio::prelude::*;

use super::ArchiveError;
use crate::transfer::Cancellation;

/// The most bytes one GIO read asks for (64 KiB), as in the Python app.
const MAX_READ_BYTES: usize = 64 * 1024;

/// A seekable archive stream on a GIO backend.
#[derive(Debug)]
pub struct GioArchiveReader {
    stream: gio::InputStream,
    seekable: gio::Seekable,
    /// The file's size, which seeks from the end are relative to.
    size: u64,
    cancel: Cancellation,
}

impl GioArchiveReader {
    /// Opens `file` for reading and queries its size.
    ///
    /// # Errors
    ///
    /// GIO's error, which keeps "not mounted" (see
    /// [`ArchiveError::is_not_mounted`]), [`ArchiveError::NotSeekable`], or
    /// [`ArchiveError::Cancelled`].
    pub fn open(file: &gio::File, cancel: &Cancellation) -> Result<Self, ArchiveError> {
        cancel.check()?;
        let stream: gio::InputStream = file.read(Some(cancel.cancellable()))?.upcast();
        let info = file.query_info(
            gio::FILE_ATTRIBUTE_STANDARD_SIZE,
            gio::FileQueryInfoFlags::NONE,
            Some(cancel.cancellable()),
        );
        match info {
            Ok(info) => Self::new(stream, u64::try_from(info.size()).unwrap_or(0), cancel),
            Err(error) => {
                close_after_failure(&stream);
                Err(error.into())
            }
        }
    }

    /// Reads `stream`, whose total length is `size`. The stream is closed
    /// when this fails, and when the reader is dropped.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::NotSeekable`] when the stream cannot seek.
    pub fn new(stream: gio::InputStream, size: u64, cancel: &Cancellation) -> Result<Self, ArchiveError> {
        let seekable = stream
            .dynamic_cast_ref::<gio::Seekable>()
            .filter(|seekable| seekable.can_seek())
            .cloned();
        let Some(seekable) = seekable else {
            close_after_failure(&stream);
            return Err(ArchiveError::NotSeekable);
        };
        Ok(Self {
            stream,
            seekable,
            size,
            cancel: cancel.clone(),
        })
    }

    /// Stops a read or seek once the user cancelled. The error is not
    /// `Interrupted`, which `read_exact` would retry forever.
    fn check_cancelled(&self) -> io::Result<()> {
        self.cancel.check().map_err(io::Error::other)
    }
}

/// Closes `stream` right away when opening the reader failed, so a share
/// does not keep the file open until the last reference goes, as the
/// Python reader does.
fn close_after_failure(stream: &gio::InputStream) {
    // The open's own error is the one to report. If closing fails too, GIO
    // still releases the handle when the last reference is dropped.
    let _ = stream.close(gio::Cancellable::NONE);
}

impl Read for GioArchiveReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        self.check_cancelled()?;
        let count = buffer.len().min(MAX_READ_BYTES);
        self.stream
            .read(&mut buffer[..count], Some(self.cancel.cancellable()))
            .map_err(io::Error::other)
    }
}

impl Seek for GioArchiveReader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let target = match position {
            SeekFrom::Start(offset) => i128::from(offset),
            SeekFrom::Current(offset) => i128::from(self.seekable.tell()) + i128::from(offset),
            SeekFrom::End(offset) => i128::from(self.size) + i128::from(offset),
        };
        let invalid = |_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid ZIP offset.");
        // A position before the start is refused; GIO takes signed offsets.
        let position = u64::try_from(target).map_err(invalid)?;
        let offset = i64::try_from(position).map_err(invalid)?;
        self.check_cancelled()?;
        self.seekable
            .seek(offset, glib::SeekType::Set, Some(self.cancel.cancellable()))
            .map_err(io::Error::other)?;
        Ok(position)
    }
}
