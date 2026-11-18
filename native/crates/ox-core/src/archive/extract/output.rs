// SPDX-License-Identifier: AGPL-3.0-only
//! Where extracted files are written. Ports the `writer` of `ZipExtractor`
//! in `v2.0.0:desktop/zip_extraction.py` and its production value,
//! `exclusive_output` in `v2.0.0:desktop/gio_backend.py`.

use gio::prelude::*;

use crate::transfer::{Cancellation, Node, TransferError};

/// The mode `creat(2)` asks for a new file, before the umask.
const NEW_FILE_MODE: u32 = 0o666;

/// Creates the files of an extraction. Production uses
/// [`GioExtractionOutput`]; tests use local files and failing doubles.
pub trait ExtractionOutput: Send + Sync {
    /// Creates `file` as a new regular file that only the user can access
    /// while it is written, and opens it for writing.
    ///
    /// ARC-012 and ARC-018: it never replaces an existing item, and never
    /// applies the archive's permissions, owner or executable bits.
    ///
    /// # Errors
    ///
    /// The backend's error; [`TransferError::Exists`] when the name is
    /// taken.
    fn create(&self, file: &dyn Node, cancel: &Cancellation) -> Result<Box<dyn OutputFile>, TransferError>;

    /// Creates `link` as a new symbolic link to `target`, a relative path
    /// the extraction checked (ARC-016). It never replaces an existing
    /// item.
    ///
    /// # Errors
    ///
    /// The backend's error, for example where links cannot be stored;
    /// [`TransferError::Exists`] when the name is taken.
    fn create_link(&self, link: &dyn Node, target: &str, cancel: &Cancellation) -> Result<(), TransferError> {
        // `symlink(2)` through GIO, which fails on an existing name.
        gio::File::for_uri(&link.uri()).make_symbolic_link(target, Some(cancel.cancellable()))?;
        Ok(())
    }
}

/// A file an extraction is writing.
pub trait OutputFile {
    /// Writes `block` and returns how many bytes the destination accepted.
    /// Accepting fewer than all stops the extraction (ARC-013).
    ///
    /// # Errors
    ///
    /// The backend's error, or [`TransferError::Cancelled`].
    fn write_block(&mut self, block: &[u8], cancel: &Cancellation) -> Result<usize, TransferError>;

    /// Flushes and closes the file after its last block, then gives it
    /// what a finished file has (ARC-018): on this computer the mode any
    /// app's new file gets, 0666 less the umask, and everywhere the
    /// `modified` time the archive records, in seconds since the Unix
    /// epoch, when it records one. A file dropped without closing is
    /// abandoned; the extraction then removes it with its staging folder.
    ///
    /// # Errors
    ///
    /// The backend's error, for example when a share cannot store the
    /// last buffered bytes, or a local file's mode or time cannot be set.
    fn close(self: Box<Self>, modified: Option<u64>) -> Result<(), TransferError>;
}

/// Writes extracted files through GIO, on local disks, shares and devices
/// alike.
#[derive(Debug, Clone, Copy, Default)]
pub struct GioExtractionOutput;

impl ExtractionOutput for GioExtractionOutput {
    fn create(&self, file: &dyn Node, cancel: &Cancellation) -> Result<Box<dyn OutputFile>, TransferError> {
        // `create` fails on an existing name, links included, and `PRIVATE`
        // makes a local file 0600 whatever mode the archive records, until
        // it is complete.
        let target = gio::File::for_uri(&file.uri());
        let stream = target.create(gio::FileCreateFlags::PRIVATE, Some(cancel.cancellable()))?;
        let local = file.uri().starts_with("file:") && target.path().is_some();
        Ok(Box::new(GioOutputFile { stream, target, local }))
    }
}

/// A file being written through GIO. Dropping the stream closes it.
struct GioOutputFile {
    stream: gio::FileOutputStream,
    /// The file being written.
    target: gio::File,
    /// Whether it is a real local file, which gets Unix modes (XFER-004).
    local: bool,
}

impl OutputFile for GioOutputFile {
    fn write_block(&mut self, block: &[u8], cancel: &Cancellation) -> Result<usize, TransferError> {
        cancel.check()?;
        let (written, failure) = self.stream.write_all(block, Some(cancel.cancellable()))?;
        match failure {
            Some(error) => Err(error.into()),
            None => Ok(written),
        }
    }

    fn close(self: Box<Self>, modified: Option<u64>) -> Result<(), TransferError> {
        // Closed without the cancellable, as in the Python app, so the
        // handle is flushed and released however the extraction ends.
        self.stream.close(gio::Cancellable::NONE)?;
        // The file is still in the extraction's owner-only staging folder,
        // where no other account can swap its name for a link, and links
        // are not followed in any case.
        let flags = gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS;
        if self.local {
            let mode = NEW_FILE_MODE & !crate::permissions::process_umask();
            self.target
                .set_attribute_uint32("unix::mode", mode, flags, gio::Cancellable::NONE)?;
        }
        if let Some(seconds) = modified {
            let set = self.target.set_attribute_uint64(
                gio::FILE_ATTRIBUTE_TIME_MODIFIED,
                seconds,
                flags,
                gio::Cancellable::NONE,
            );
            // Phones and many shares cannot set times; the file is kept
            // with the time they give it.
            if self.local {
                set?;
            }
        }
        Ok(())
    }
}
