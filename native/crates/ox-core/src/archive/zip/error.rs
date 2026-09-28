// SPDX-License-Identifier: AGPL-3.0-only
//! Damaged or unsupported ZIP structures.
//!
//! These are the refusals of Python's `zipfile` (`BadZipFile`,
//! `NotImplementedError`) that `desktop/archives.py` and
//! `desktop/zip_extraction.py` pass on to the user. Where `zipfile` has a
//! message it is kept word for word, with names quoted as its `%r` quotes
//! them (see `python_repr`); where Python only raised a generic error (a
//! `UnicodeDecodeError`, a `zlib.error`), the message says what is wrong
//! with the archive instead.

use std::fmt;

use super::python_repr::{PythonBytesRepr, PythonRepr};

/// A ZIP archive that is damaged, or uses a feature the built-in reader
/// does not support. `Display` is the user-facing message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ZipFormatError {
    /// No end of central directory record was found.
    #[error("File is not a zip file")]
    NotAZip,
    /// The central directory would start before the file does.
    #[error("Bad offset for central directory")]
    BadDirectoryOffset,
    /// The central directory ends in the middle of a record.
    #[error("Truncated central directory")]
    TruncatedDirectory,
    /// A central directory record has the wrong signature.
    #[error("Bad magic number for central directory")]
    BadDirectorySignature,
    /// The archive is split over several files.
    #[error("zipfiles that span multiple disks are not supported")]
    MultipleDisks,
    /// The ZIP64 locator points past its own position.
    #[error("Corrupt zip64 end of central directory locator")]
    CorruptZip64Locator,
    /// A ZIP64 locator exists, but the record it points to does not.
    #[error("Zip64 end of central directory record not found")]
    MissingZip64Record,
    /// The ZIP64 end record contradicts the locator.
    #[error("Corrupt zip64 end of central directory record")]
    CorruptZip64Record,
    /// An extra field is longer than the space left for it.
    #[error("Corrupt extra field {field_id:04x} (size={size})")]
    CorruptExtraField {
        /// The field's header id.
        field_id: u16,
        /// The size the field claims.
        size: u16,
    },
    /// A ZIP64 extra field lacks a value its member needs.
    #[error("Corrupt zip64 extra field. {0} not found.")]
    CorruptZip64Field(Zip64Field),
    /// An Info-ZIP Unicode path field is too short.
    #[error("Corrupt unicode path extra field (0x7075)")]
    CorruptUnicodePathField,
    /// An Info-ZIP Unicode path field is not UTF-8.
    #[error("Corrupt unicode path extra field (0x7075): invalid utf-8 bytes")]
    InvalidUnicodePath,
    /// A name flagged as UTF-8 is not valid UTF-8.
    #[error("A file name in this ZIP is marked as UTF-8 but is not valid UTF-8.")]
    NameNotUtf8,
    /// A member needs a newer ZIP version than 6.3 to be read.
    #[error("zip file version {}.{}", .0 / 10, .0 % 10)]
    UnsupportedVersion(u8),
    /// A member's local header ends early.
    #[error("Truncated file header")]
    TruncatedHeader,
    /// A member's local header has the wrong signature.
    #[error("Bad magic number for file header")]
    BadHeaderSignature,
    /// The local header names another file than the central directory.
    #[error(
        "File name in directory {} and header {} differ.",
        PythonRepr(.directory),
        PythonBytesRepr(.header)
    )]
    NameMismatch {
        /// The name in the central directory.
        directory: String,
        /// The name in the member's local header, as it is stored.
        header: Vec<u8>,
    },
    /// A member's data runs into the next member: a ZIP bomb technique.
    #[error("Overlapped entries: {} (possible zip bomb)", PythonRepr(.0))]
    OverlappedEntries(String),
    /// Compressed patched data (general purpose flag bit 5).
    #[error("compressed patched data (flag bit 5)")]
    CompressedPatchedData,
    /// Strong encryption (general purpose flag bit 6).
    #[error("strong encryption (flag bit 6)")]
    StrongEncryption,
    /// The member is encrypted and no password is ever supplied. The
    /// archive services refuse encrypted members before they open one, so
    /// this only guards the reader itself.
    #[error("File {} is encrypted, password required for extraction", PythonRepr(.0))]
    PasswordRequired(String),
    /// The compression method is not stored, deflate, bzip2 or LZMA.
    #[error("That compression method is not supported")]
    UnsupportedMethod,
    /// The decompressed data does not match the member's CRC-32.
    #[error("Bad CRC-32 for file {}", PythonRepr(.0))]
    BadCrc(String),
    /// The compressed data cannot be decompressed. The member's name is
    /// quoted like the names in `zipfile`'s messages.
    #[error("Error while decompressing {}: {detail}", PythonRepr(.name))]
    CorruptData {
        /// The member's name.
        name: String,
        /// The decompressor's description of the problem.
        detail: String,
    },
}

/// A value a ZIP64 extra field can carry, named as `zipfile` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zip64Field {
    /// The uncompressed size.
    FileSize,
    /// The compressed size.
    CompressedSize,
    /// The position of the member's local header.
    HeaderOffset,
}

impl fmt::Display for Zip64Field {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Zip64Field::FileSize => "File size",
            Zip64Field::CompressedSize => "Compress size",
            Zip64Field::HeaderOffset => "Header offset",
        };
        formatter.write_str(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_keep_the_wording_of_python_zipfile() {
        assert_eq!(ZipFormatError::NotAZip.to_string(), "File is not a zip file");
        assert_eq!(
            ZipFormatError::CorruptExtraField {
                field_id: 1,
                size: 16
            }
            .to_string(),
            "Corrupt extra field 0001 (size=16)"
        );
        assert_eq!(
            ZipFormatError::CorruptZip64Field(Zip64Field::CompressedSize).to_string(),
            "Corrupt zip64 extra field. Compress size not found."
        );
        assert_eq!(
            ZipFormatError::UnsupportedVersion(64).to_string(),
            "zip file version 6.4"
        );
        assert_eq!(
            ZipFormatError::BadCrc("doc.txt".into()).to_string(),
            "Bad CRC-32 for file 'doc.txt'"
        );
        assert_eq!(
            ZipFormatError::OverlappedEntries("a.txt".into()).to_string(),
            "Overlapped entries: 'a.txt' (possible zip bomb)"
        );
        assert_eq!(
            ZipFormatError::NameMismatch {
                directory: "report.txt".into(),
                header: b"report.exe".to_vec()
            }
            .to_string(),
            "File name in directory 'report.txt' and header b'report.exe' differ."
        );
    }
}
