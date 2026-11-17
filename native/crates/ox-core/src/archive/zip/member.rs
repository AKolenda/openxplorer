// SPDX-License-Identifier: AGPL-3.0-only
//! One member of a ZIP archive, as its central directory describes it: the
//! fields of Python's `zipfile.ZipInfo` that the archive services use.

/// General purpose flag bit 0: the member is encrypted.
const ENCRYPTED_FLAG: u16 = 1;
/// General purpose flag bit 3: the sizes and CRC-32 follow the data.
const DATA_DESCRIPTOR_FLAG: u16 = 1 << 3;
/// General purpose flag bit 6: PKWARE strong encryption.
const STRONG_ENCRYPTION_FLAG: u16 = 1 << 6;
/// The compression method number of `WinZip`'s AES encryption.
const AES_METHOD: u16 = 99;
/// The file type bits of a Unix mode (`S_IFMT`).
const FILE_TYPE_MASK: u32 = 0o170_000;
/// A regular file (`S_IFREG`).
const REGULAR_FILE: u32 = 0o100_000;
/// A directory (`S_IFDIR`).
const DIRECTORY: u32 = 0o040_000;

/// How a member's data is compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompressionMethod {
    /// Method 0: not compressed.
    Stored,
    /// Method 8: raw deflate.
    Deflated,
    /// Method 12: bzip2.
    Bzip2,
    /// Method 14: LZMA with the ZIP-specific properties header.
    Lzma,
    /// Any other method, which only an archive manager can read.
    Other(u16),
    /// A 7z member, and whether the built-in reader decodes its block.
    SevenZip(SevenZipCoding),
}

/// How the block of a 7z member is coded, as far as the built-in reader
/// is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SevenZipCoding {
    /// LZMA, LZMA2, Copy, Delta, a branch filter or `BZip2`: decoded.
    Readable,
    /// Encrypted with AES, which the reader does not decrypt.
    Encrypted,
    /// Another method (`PPMd`, Deflate and others).
    Unsupported,
    /// A dictionary over the reader's memory limit.
    TooMuchMemory,
}

impl CompressionMethod {
    /// The method with the number `method` of the ZIP specification.
    pub(super) fn from_number(method: u16) -> Self {
        match method {
            0 => CompressionMethod::Stored,
            8 => CompressionMethod::Deflated,
            12 => CompressionMethod::Bzip2,
            14 => CompressionMethod::Lzma,
            other => CompressionMethod::Other(other),
        }
    }

    /// True for the four methods Python's `zipfile` can read.
    pub(crate) fn is_supported(self) -> bool {
        !matches!(
            self,
            CompressionMethod::Other(_)
                | CompressionMethod::SevenZip(SevenZipCoding::Unsupported | SevenZipCoding::TooMuchMemory)
        )
    }
}

/// How a member is encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Encryption {
    /// Not encrypted.
    None,
    /// The traditional PKWARE encryption (`ZipCrypto`), which the reader
    /// decrypts with a password.
    Traditional,
    /// AES or PKWARE strong encryption, left to an archive manager.
    Unsupported,
}

/// The Unix file type an archive records for a member, in the upper half
/// of its external attributes (`S_IFMT(external_attr >> 16)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemberFileType {
    /// No Unix type recorded, as in archives made on Windows.
    Unrecorded,
    /// A regular file.
    Regular,
    /// A directory.
    Directory,
    /// A symbolic link, FIFO, device or socket: never extracted or opened.
    LinkOrSpecial,
}

/// A member's modification time in MS-DOS format, as stored in the archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DosDateTime {
    /// Year since 1980, month and day.
    pub(super) date: u16,
    /// Hour, minute and seconds divided by two.
    pub(super) time: u16,
}

impl DosDateTime {
    /// No time at all: day 0 of month 0, which no calendar has, so
    /// [`Self::to_unix_seconds`] reads it as `None`.
    pub(crate) const UNKNOWN: Self = Self { date: 0, time: 0 };

    /// The MS-DOS form of `seconds` since the Unix epoch in local time,
    /// for archives that record Unix times; times before 1980 or after 2107
    /// are clamped to the range the format can hold.
    pub(crate) fn from_unix_seconds(seconds: u64) -> Self {
        let local = i64::try_from(seconds)
            .ok()
            .and_then(|seconds| glib::DateTime::from_unix_local(seconds).ok());
        let Some(local) = local else {
            return Self { date: 0x21, time: 0 };
        };
        let year = local.year().clamp(1980, 2107);
        let field = |value: i32| u16::try_from(value).unwrap_or(0);
        Self {
            date: field(year - 1980) << 9 | field(local.month()) << 5 | field(local.day_of_month()),
            time: field(local.hour()) << 11 | field(local.minute()) << 5 | field(local.second() / 2),
        }
    }

    /// Seconds since the Unix epoch, reading the time as local time like
    /// `datetime(*date_time).timestamp()` in `v2.0.0:desktop/archives.py`; `None`
    /// for an impossible date or time, where Python reported 0.
    pub(crate) fn to_unix_seconds(self) -> Option<u64> {
        let year = 1980 + i32::from(self.date >> 9);
        let month = i32::from((self.date >> 5) & 0xf);
        let day = i32::from(self.date & 0x1f);
        let hour = i32::from(self.time >> 11);
        let minute = i32::from((self.time >> 5) & 0x3f);
        let second = f64::from((self.time & 0x1f) * 2);
        let local = glib::DateTime::from_local(year, month, day, hour, minute, second).ok()?;
        u64::try_from(local.to_unix()).ok()
    }
}

/// One member of the central directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ZipMember {
    /// The name after `zipfile`'s clean-up (`ZipInfo.filename`): cut at the
    /// first NUL, or taken from a matching Info-ZIP Unicode path field.
    pub(crate) name: String,
    /// The name exactly as the central directory spells it
    /// (`ZipInfo.orig_filename`).
    pub(crate) original_name: String,
    /// The general purpose flags.
    pub(crate) flags: u16,
    /// How the data is compressed.
    pub(crate) method: CompressionMethod,
    /// The CRC-32 of the uncompressed data.
    pub(crate) crc32: u32,
    /// The size of the compressed data.
    pub(crate) compressed_size: u64,
    /// The size of the uncompressed data.
    pub(crate) size: u64,
    /// Where the local header is, as recorded (before any shift of the
    /// whole archive within its file).
    pub(crate) header_offset: u64,
    /// Where the next member's local header or the central directory
    /// starts, as recorded: the member's data must end before it.
    pub(crate) data_limit: u64,
    /// Unix mode and MS-DOS attributes.
    pub(crate) external_attributes: u32,
    /// The modification time.
    pub(crate) modified: DosDateTime,
}

impl ZipMember {
    /// True for a folder entry: its name ends with a slash
    /// (`ZipInfo.is_dir`).
    pub(crate) fn is_directory(&self) -> bool {
        self.name.ends_with('/')
    }

    /// True when the member is encrypted.
    pub(crate) fn is_encrypted(&self) -> bool {
        self.flags & ENCRYPTED_FLAG != 0
    }

    /// How the member is encrypted.
    pub(crate) fn encryption(&self) -> Encryption {
        if !self.is_encrypted() {
            Encryption::None
        } else if self.flags & STRONG_ENCRYPTION_FLAG != 0
            || self.method == CompressionMethod::Other(AES_METHOD)
        {
            Encryption::Unsupported
        } else {
            Encryption::Traditional
        }
    }

    /// The byte the decrypted encryption header ends with: the high byte
    /// of the modification time when the sizes follow the data, else of
    /// the CRC-32 (`_ZipDecrypter` in `zipfile`).
    /// ARC-016: the error opening or extracting this member gives when
    /// its encryption is one the reader cannot decrypt, in the words of
    /// its format.
    pub(crate) fn encryption_refusal(&self) -> Option<crate::archive::ArchiveError> {
        use crate::archive::ArchiveError;
        let refusal = match self.method {
            CompressionMethod::SevenZip(_) => ArchiveError::SevenZipEncryption,
            _ => ArchiveError::UnsupportedEncryption,
        };
        (self.encryption() == Encryption::Unsupported).then_some(refusal)
    }

    /// ARC-016: the error opening or extracting this member gives when
    /// the reader cannot decompress it, in the words of its format.
    pub(crate) fn method_refusal(&self) -> Option<crate::archive::ArchiveError> {
        use crate::archive::ArchiveError;
        match self.method {
            CompressionMethod::SevenZip(SevenZipCoding::Unsupported) => Some(ArchiveError::SevenZipMethod),
            CompressionMethod::SevenZip(SevenZipCoding::TooMuchMemory) => {
                Some(ArchiveError::DecoderMemoryLimit)
            }
            method if !method.is_supported() => Some(ArchiveError::UnsupportedCompression),
            _ => None,
        }
    }

    pub(crate) fn encryption_check_byte(&self) -> u8 {
        if self.flags & DATA_DESCRIPTOR_FLAG != 0 {
            self.modified.time.to_be_bytes()[0]
        } else {
            self.crc32.to_be_bytes()[0]
        }
    }

    /// True when the cleaned-up name is the name the archive records. A NUL
    /// inside the name, or a Unicode path field naming something else,
    /// makes a member show one name and hold another.
    pub(crate) fn has_unaltered_name(&self) -> bool {
        self.name == self.original_name
    }

    /// The Unix file type recorded for the member.
    pub(crate) fn file_type(&self) -> MemberFileType {
        match (self.external_attributes >> 16) & FILE_TYPE_MASK {
            0 => MemberFileType::Unrecorded,
            REGULAR_FILE => MemberFileType::Regular,
            DIRECTORY => MemberFileType::Directory,
            _ => MemberFileType::LinkOrSpecial,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A member with the given name and external attributes.
    fn member(name: &str, external_attributes: u32) -> ZipMember {
        ZipMember {
            name: name.to_owned(),
            original_name: name.to_owned(),
            flags: 0,
            method: CompressionMethod::Stored,
            crc32: 0,
            compressed_size: 0,
            size: 0,
            header_offset: 0,
            data_limit: 0,
            external_attributes,
            modified: DosDateTime { date: 0, time: 0 },
        }
    }

    #[test]
    fn file_types_come_from_the_upper_mode_bits() {
        assert_eq!(member("a", 0o600 << 16).file_type(), MemberFileType::Unrecorded);
        assert_eq!(member("a", 0o100_644 << 16).file_type(), MemberFileType::Regular);
        assert_eq!(
            member("a/", 0o040_755 << 16).file_type(),
            MemberFileType::Directory
        );
        for special in [0o120_777, 0o010_600, 0o020_600, 0o060_600, 0o140_600] {
            assert_eq!(
                member("a", special << 16).file_type(),
                MemberFileType::LinkOrSpecial
            );
        }
    }

    #[test]
    fn only_the_four_zipfile_methods_are_supported() {
        for number in [0, 8, 12, 14] {
            assert!(CompressionMethod::from_number(number).is_supported(), "{number}");
        }
        assert_eq!(CompressionMethod::from_number(99), CompressionMethod::Other(99));
        assert!(!CompressionMethod::Other(9).is_supported());
    }

    #[test]
    fn impossible_dos_dates_have_no_time() {
        let zero_month = DosDateTime { date: 0, time: 0 };
        let first_of_january = DosDateTime {
            date: (45 << 9) | (1 << 5) | 1,
            time: 0,
        };

        assert_eq!(zero_month.to_unix_seconds(), None);
        assert!(first_of_january.to_unix_seconds().is_some());
    }
}
