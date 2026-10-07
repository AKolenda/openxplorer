// SPDX-License-Identifier: AGPL-3.0-only
//! File checksums for the Checksums tab of Properties (PROP-014), as
//! Dolphin's Checksums tab computes them: MD5, SHA1, SHA256 and SHA512 on
//! demand, and an expected checksum pasted by the user checked against the
//! algorithm its length names.
//!
//! The file is read in 64 KiB blocks, through GIO for a file on a share,
//! so it is read in place, and a cancelled read stops between two blocks.
//!
//! Only an ordinary file is read. Opening a named pipe (FIFO) waits in the
//! kernel until a program writes to it, which Cancel cannot interrupt, and
//! a device or a socket has no contents to sum, so those are refused at
//! once: GIO is asked what the item is first, and a local file is opened
//! without waiting (`O_NONBLOCK`) and checked again once it is open, so a
//! file replaced by a pipe in between is refused too.

use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;

use gio::prelude::*;

use crate::entry::EntryError;
use crate::location::normalise;
use crate::transfer::Cancellation;

/// How much is read at a time.
const BLOCK_BYTES: usize = 64 * 1024;

/// A checksum algorithm the tab offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChecksumKind {
    /// MD5, 32 hexadecimal digits.
    Md5,
    /// SHA-1, 40 hexadecimal digits.
    Sha1,
    /// SHA-256, 64 hexadecimal digits.
    Sha256,
    /// SHA-512, 128 hexadecimal digits.
    Sha512,
}

impl ChecksumKind {
    /// Every algorithm, in the order the tab lists them.
    pub const ALL: [Self; 4] = [Self::Md5, Self::Sha1, Self::Sha256, Self::Sha512];

    /// The name the tab shows.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Md5 => "MD5",
            Self::Sha1 => "SHA1",
            Self::Sha256 => "SHA256",
            Self::Sha512 => "SHA512",
        }
    }

    const fn glib_type(self) -> glib::ChecksumType {
        match self {
            Self::Md5 => glib::ChecksumType::Md5,
            Self::Sha1 => glib::ChecksumType::Sha1,
            Self::Sha256 => glib::ChecksumType::Sha256,
            Self::Sha512 => glib::ChecksumType::Sha512,
        }
    }

    const fn hex_digits(self) -> usize {
        match self {
            Self::Md5 => 32,
            Self::Sha1 => 40,
            Self::Sha256 => 64,
            Self::Sha512 => 128,
        }
    }

    /// The algorithm of the expected checksum `text`, told by its number
    /// of hexadecimal digits; `None` for anything else.
    pub fn of_expected(text: &str) -> Option<Self> {
        let text = text.trim();
        if !text.chars().all(|character| character.is_ascii_hexdigit()) {
            return None;
        }
        Self::ALL.into_iter().find(|kind| kind.hex_digits() == text.len())
    }
}

/// True when `expected`, as pasted, is the checksum `computed`: spaces
/// around it and letter case do not matter.
pub fn matches(expected: &str, computed: &str) -> bool {
    expected.trim().eq_ignore_ascii_case(computed)
}

/// The `kind` checksum of the file at `uri`, in lowercase hexadecimal.
/// Blocking; see [`compute_in_background`].
///
/// # Errors
///
/// Why the file could not be read, or [`EntryError::Cancelled`].
pub fn compute(uri: &str, kind: ChecksumKind, cancel: &Cancellation) -> Result<String, EntryError> {
    let file = gio::File::for_uri(&normalise(uri)?);
    let info = file.query_info(
        gio::FILE_ATTRIBUTE_STANDARD_TYPE,
        gio::FileQueryInfoFlags::NONE,
        Some(cancel.cancellable()),
    )?;
    if info.file_type() != gio::FileType::Regular {
        return Err(not_a_file());
    }
    let Some(mut checksum) = glib::Checksum::new(kind.glib_type()) else {
        return Err(EntryError::Failed(format!("{} is not available.", kind.label())));
    };
    let mut block = vec![0; BLOCK_BYTES];
    if let Some(path) = file.path().filter(|_| file.is_native()) {
        let mut local = open_local_file(&path)?;
        sum_blocks(&mut checksum, &mut block, cancel, |block| {
            local
                .read(block)
                .map_err(|error| EntryError::Failed(error.to_string()))
        })?;
    } else {
        let stream = file.read(Some(cancel.cancellable()))?;
        sum_blocks(&mut checksum, &mut block, cancel, |block| {
            Ok(stream.read(block, Some(cancel.cancellable()))?)
        })?;
    }
    Ok(checksum.string().unwrap_or_default())
}

/// Why an item that is not an ordinary file has no checksum.
fn not_a_file() -> EntryError {
    EntryError::NotSupported(crate::i18n::gettext(
        "Checksums can only be calculated for ordinary files, not for pipes, devices or sockets.",
    ))
}

/// Opens the local file at `path` without waiting on a pipe, and refuses
/// it unless it is an ordinary file. An ordinary file reads as usual
/// with `O_NONBLOCK`.
fn open_local_file(path: &std::path::Path) -> Result<std::fs::File, EntryError> {
    let failed = |error: std::io::Error| EntryError::Failed(error.to_string());
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NONBLOCK.bits().cast_signed())
        .open(path)
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => EntryError::NotFound(error.to_string()),
            std::io::ErrorKind::PermissionDenied => EntryError::PermissionDenied(error.to_string()),
            _ => failed(error),
        })?;
    if !file.metadata().map_err(failed)?.is_file() {
        return Err(not_a_file());
    }
    Ok(file)
}

/// Adds every block `read` gives to `checksum`, stopping between two
/// blocks when `cancel` is cancelled.
fn sum_blocks(
    checksum: &mut glib::Checksum,
    block: &mut [u8],
    cancel: &Cancellation,
    mut read: impl FnMut(&mut [u8]) -> Result<usize, EntryError>,
) -> Result<(), EntryError> {
    loop {
        if cancel.is_cancelled() {
            return Err(EntryError::Cancelled);
        }
        let count = read(block)?;
        if count == 0 {
            return Ok(());
        }
        checksum.update(&block[..count]);
    }
}

/// [`compute`] on a GIO worker thread, for the main loop to await.
///
/// # Errors
///
/// As [`compute`].
pub async fn compute_in_background(
    uri: String,
    kind: ChecksumKind,
    cancel: Cancellation,
) -> Result<String, EntryError> {
    match gio::spawn_blocking(move || compute(&uri, kind, &cancel)).await {
        Ok(result) => result,
        // A panicking read is a bug; it surfaces where it is awaited.
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::file_uri;

    /// parity: PROP-014
    #[test]
    fn each_algorithm_gives_the_known_digest_and_a_pasted_one_is_recognised() {
        let folder = tempfile::tempdir().expect("a folder");
        let path = folder.path().join("abc.txt");
        std::fs::write(&path, b"abc").expect("a file");
        let uri = file_uri(&path);
        let digest = |kind| compute(&uri, kind, &Cancellation::new()).expect("reads");

        assert_eq!(digest(ChecksumKind::Md5), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            digest(ChecksumKind::Sha1),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        let sha256 = digest(ChecksumKind::Sha256);
        assert_eq!(
            sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(digest(ChecksumKind::Sha512).len(), 128);
        let pasted = format!("  {}\n", sha256.to_uppercase());
        assert_eq!(ChecksumKind::of_expected(&pasted), Some(ChecksumKind::Sha256));
        assert!(matches(&pasted, &sha256));
        assert_eq!(ChecksumKind::of_expected("not a checksum"), None);
        assert_eq!(ChecksumKind::of_expected("abc"), None);
    }

    /// parity: PROP-014
    #[test]
    fn a_cancelled_read_stops() {
        let folder = tempfile::tempdir().expect("a folder");
        let path = folder.path().join("big.bin");
        std::fs::write(&path, vec![0u8; 3 * BLOCK_BYTES]).expect("a file");
        let cancel = Cancellation::new();
        cancel.cancel();

        let result = compute(&file_uri(&path), ChecksumKind::Sha256, &cancel);

        assert!(result.is_err());
    }

    /// A named pipe, which would wait for a writer forever, and a device
    /// are refused at once instead of being read.
    ///
    /// parity: PROP-014
    #[test]
    fn a_pipe_or_a_device_is_refused_at_once() {
        let folder = tempfile::tempdir().expect("a folder");
        let pipe = folder.path().join("pipe");
        crate::test_support::make_fifo(&pipe);
        let (sender, receiver) = std::sync::mpsc::channel();
        let uri = file_uri(&pipe);
        std::thread::spawn(move || {
            let _ = sender.send(compute(&uri, ChecksumKind::Sha256, &Cancellation::new()));
        });

        let result = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("a pipe never blocks the checksum");
        assert!(matches!(result, Err(EntryError::NotSupported(_))), "{result:?}");
        let device = compute("file:///dev/null", ChecksumKind::Md5, &Cancellation::new());
        assert!(matches!(device, Err(EntryError::NotSupported(_))), "{device:?}");
    }

    /// A local file opened without waiting is refused when it turns out
    /// not to be an ordinary file, as a file replaced by a pipe after it
    /// was checked would be.
    ///
    /// parity: PROP-014
    #[test]
    fn a_file_replaced_by_a_pipe_is_refused_once_open() {
        let folder = tempfile::tempdir().expect("a folder");
        let pipe = folder.path().join("pipe");
        crate::test_support::make_fifo(&pipe);

        let opened = open_local_file(&pipe);

        assert!(matches!(opened, Err(EntryError::NotSupported(_))), "{opened:?}");
        let plain = folder.path().join("plain.txt");
        std::fs::write(&plain, b"abc").expect("a file");
        assert!(open_local_file(&plain).is_ok());
    }
}
