// SPDX-License-Identifier: AGPL-3.0-only
//! File checksums for the Checksums tab of Properties (PROP-014), as
//! Dolphin's Checksums tab computes them: MD5, SHA1, SHA256 and SHA512 on
//! demand, and an expected checksum pasted by the user checked against the
//! algorithm its length names.
//!
//! The file is read through GIO in 64 KiB blocks, so a file on a share is
//! read in place, and a cancelled read stops between two blocks.

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
    let stream = file.read(Some(cancel.cancellable()))?;
    let Some(mut checksum) = glib::Checksum::new(kind.glib_type()) else {
        return Err(EntryError::Failed(format!("{} is not available.", kind.label())));
    };
    let mut block = vec![0; BLOCK_BYTES];
    loop {
        if cancel.is_cancelled() {
            return Err(EntryError::Cancelled);
        }
        let read = stream.read(&mut block, Some(cancel.cancellable()))?;
        if read == 0 {
            break;
        }
        checksum.update(&block[..read]);
    }
    Ok(checksum.string().unwrap_or_default())
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
}
