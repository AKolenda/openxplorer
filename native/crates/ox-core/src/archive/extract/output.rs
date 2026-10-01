// SPDX-License-Identifier: AGPL-3.0-only
//! Where extracted files are written. Ports the `writer` of `ZipExtractor`
//! in `v2.0.0:desktop/zip_extraction.py` and its production value,
//! `exclusive_output` in `v2.0.0:desktop/gio_backend.py`.

use gio::prelude::*;

use crate::transfer::{Cancellation, Node, TransferError};

/// Creates the files of an extraction. Production uses
/// [`GioExtractionOutput`]; tests use local files and failing doubles.
pub trait ExtractionOutput: Send + Sync {
    /// Creates `file` as a new regular file that only the user can access
    /// and opens it for writing.
    ///
    /// ARC-012 and ARC-018: it never replaces an existing item, and never
    /// applies the archive's permissions, owner or executable bits.
    ///
    /// # Errors
    ///
    /// The backend's error; [`TransferError::Exists`] when the name is
    /// taken.
    fn create(&self, file: &dyn Node, cancel: &Cancellation) -> Result<Box<dyn OutputFile>, TransferError>;
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

    /// Flushes and closes the file after its last block. A file dropped
    /// without closing is abandoned; the extraction then removes it with
    /// its staging folder.
    ///
    /// # Errors
    ///
    /// The backend's error, for example when a share cannot store the
    /// last buffered bytes.
    fn close(self: Box<Self>) -> Result<(), TransferError>;
}

/// Writes extracted files through GIO, on local disks, shares and devices
/// alike.
#[derive(Debug, Clone, Copy, Default)]
pub struct GioExtractionOutput;

impl ExtractionOutput for GioExtractionOutput {
    fn create(&self, file: &dyn Node, cancel: &Cancellation) -> Result<Box<dyn OutputFile>, TransferError> {
        // `create` fails on an existing name, links included, and `PRIVATE`
        // makes a local file 0600 whatever mode the archive records.
        let stream = gio::File::for_uri(&file.uri())
            .create(gio::FileCreateFlags::PRIVATE, Some(cancel.cancellable()))?;
        Ok(Box::new(GioOutputFile { stream }))
    }
}

/// A file being written through GIO. Dropping the stream closes it.
struct GioOutputFile {
    stream: gio::FileOutputStream,
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

    fn close(self: Box<Self>) -> Result<(), TransferError> {
        // Closed without the cancellable, as in the Python app, so the
        // handle is flushed and released however the extraction ends.
        self.stream.close(gio::Cancellable::NONE)?;
        Ok(())
    }
}
