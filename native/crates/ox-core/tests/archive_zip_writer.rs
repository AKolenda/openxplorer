// SPDX-License-Identifier: AGPL-3.0-only
//! Writes ZIP archives byte by byte for the `archive_*` integration tests;
//! `archive_support.rs` includes it and re-exports what tests use. Cargo
//! also builds this file as a test executable of its own, which has no
//! tests.
//!
//! The Python tests build archives with `zipfile` and then change fields
//! in memory. [`TestMember`] writes those fields directly instead, so a
//! test can store what `zipfile` refuses to write, such as a NUL in a name
//! or a local header that names another file.

use std::fs;
use std::io::Write;
use std::path::Path;

/// `S_IFREG`, `S_IFDIR` and the other file types of a Unix mode.
pub mod file_type {
    /// A regular file.
    pub const REGULAR: u32 = 0o100_000;
    /// A folder.
    pub const DIRECTORY: u32 = 0o040_000;
    /// A symbolic link.
    pub const SYMLINK: u32 = 0o120_000;
    /// A FIFO.
    pub const FIFO: u32 = 0o010_000;
    /// A character device.
    pub const CHARACTER_DEVICE: u32 = 0o020_000;
    /// A block device.
    pub const BLOCK_DEVICE: u32 = 0o060_000;
    /// A socket.
    pub const SOCKET: u32 = 0o140_000;
}

/// General purpose flag bit 0: the member is encrypted.
pub const ENCRYPTED_FLAG: u16 = 1;
/// General purpose flag bit 11: the name is UTF-8.
pub const UTF8_NAME_FLAG: u16 = 1 << 11;
/// `PK\x03\x04`, `PK\x01\x02`, `PK\x05\x06`, `PK\x06\x06` and `PK\x06\x07`.
const LOCAL_HEADER_SIGNATURE: u32 = 0x0403_4b50;
const CENTRAL_HEADER_SIGNATURE: u32 = 0x0201_4b50;
const END_OF_DIRECTORY_SIGNATURE: u32 = 0x0605_4b50;
const ZIP64_END_SIGNATURE: u32 = 0x0606_4b50;
const ZIP64_LOCATOR_SIGNATURE: u32 = 0x0706_4b50;
/// The ZIP version needed for ZIP64 records, times ten: 4.5.
const ZIP64_VERSION_NEEDED: u16 = 45;
/// Every member's modification date: 2026-09-28.
const DOS_DATE: u16 = dos_date(2026, 9, 28);
/// Every member's modification time: 12:00:00.
const DOS_TIME: u16 = dos_time(12, 0);
/// The ZIP version needed to extract, times ten: 2.0.
const VERSION_NEEDED: u16 = 20;
/// The system that made the archive: Unix.
const MADE_ON_UNIX: u16 = 3;
/// "Made by": Unix, ZIP 2.0.
const VERSION_MADE_BY: u16 = (MADE_ON_UNIX << 8) | VERSION_NEEDED;

/// A date in MS-DOS format: years since 1980, month and day.
const fn dos_date(year: u16, month: u16, day: u16) -> u16 {
    ((year - 1980) << 9) | (month << 5) | day
}

/// A time in MS-DOS format, to the minute.
const fn dos_time(hour: u16, minute: u16) -> u16 {
    (hour << 11) | (minute << 5)
}

/// How a test member's data is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// Method 0.
    Stored,
    /// Method 8, raw deflate.
    Deflated,
    /// Method 12.
    Bzip2,
    /// Another method number, stored as is; the data is not compressed.
    Unsupported(u16),
}

impl Compression {
    /// The method number the ZIP format records.
    fn number(self) -> u16 {
        match self {
            Compression::Stored => 0,
            Compression::Deflated => 8,
            Compression::Bzip2 => 12,
            Compression::Unsupported(number) => number,
        }
    }

    /// `data`, compressed with this method.
    fn compress(self, data: &[u8]) -> Vec<u8> {
        match self {
            Compression::Stored | Compression::Unsupported(_) => data.to_vec(),
            Compression::Deflated => {
                let mut encoder =
                    flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                encoder.write_all(data).expect("deflate into memory");
                encoder.finish().expect("deflate into memory")
            }
            Compression::Bzip2 => {
                let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
                encoder.write_all(data).expect("bzip2 into memory");
                encoder.finish().expect("bzip2 into memory")
            }
        }
    }
}

/// One member of a test archive, as [`zip_bytes`] writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestMember {
    /// The name exactly as the central directory stores it.
    pub raw_name: Vec<u8>,
    /// The name the local header stores, when it differs from `raw_name`.
    pub local_name: Option<Vec<u8>>,
    /// The uncompressed data.
    pub data: Vec<u8>,
    /// How the data is stored.
    pub compression: Compression,
    /// The Unix mode in the upper half of the external attributes.
    pub unix_mode: u32,
    /// The general purpose flags. The UTF-8 flag is added for a name that
    /// is UTF-8 but not ASCII.
    pub flags: u16,
    /// The uncompressed size the archive declares, when it is not the
    /// data's length.
    pub declared_size: Option<u64>,
    /// The compressed size the archive declares, when it is not the
    /// compressed data's length.
    pub declared_compressed_size: Option<u64>,
    /// The ZIP version needed to extract, times ten.
    pub version_needed: u16,
}

impl TestMember {
    /// A file as `ZipFile.writestr` stores it: deflated, mode `0600`.
    pub fn file(name: &str, data: &[u8]) -> Self {
        Self {
            raw_name: name.as_bytes().to_vec(),
            local_name: None,
            data: data.to_vec(),
            compression: Compression::Deflated,
            unix_mode: file_type::REGULAR | 0o600,
            flags: 0,
            declared_size: None,
            declared_compressed_size: None,
            version_needed: VERSION_NEEDED,
        }
    }

    /// A folder entry as `ZipFile.writestr('name/', b'')` stores it.
    pub fn folder(name: &str) -> Self {
        Self {
            compression: Compression::Stored,
            unix_mode: file_type::DIRECTORY | 0o775,
            ..Self::file(name, b"")
        }
    }

    /// A member of a `ZipInfo` with only its mode set: stored, and typed
    /// by `unix_mode` alone.
    pub fn with_unix_mode(name: &str, unix_mode: u32, data: &[u8]) -> Self {
        Self {
            compression: Compression::Stored,
            unix_mode,
            ..Self::file(name, data)
        }
    }

    /// This member, stored with `compression`.
    #[must_use]
    pub fn compressed_with(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    /// This member with the general purpose `flags`.
    #[must_use]
    pub fn with_flags(mut self, flags: u16) -> Self {
        self.flags = flags;
        self
    }

    /// This member, declaring `size` uncompressed bytes.
    #[must_use]
    pub fn declaring_size(mut self, size: u64) -> Self {
        self.declared_size = Some(size);
        self
    }

    /// This member, declaring `size` compressed bytes.
    #[must_use]
    pub fn declaring_compressed_size(mut self, size: u64) -> Self {
        self.declared_compressed_size = Some(size);
        self
    }

    /// This member, needing ZIP version `version` (times ten).
    #[must_use]
    pub fn needing_version(mut self, version: u16) -> Self {
        self.version_needed = version;
        self
    }

    /// This member with the raw name `raw_name` in both headers.
    #[must_use]
    pub fn named_raw(mut self, raw_name: &[u8]) -> Self {
        self.raw_name = raw_name.to_vec();
        self
    }

    /// This member with `local_name` in its local header only.
    #[must_use]
    pub fn with_local_name(mut self, local_name: &str) -> Self {
        self.local_name = Some(local_name.as_bytes().to_vec());
        self
    }

    /// The flags as stored, with the UTF-8 flag for a UTF-8 name that is
    /// not ASCII.
    fn stored_flags(&self) -> u16 {
        let is_unicode_text = !self.raw_name.is_ascii() && std::str::from_utf8(&self.raw_name).is_ok();
        if is_unicode_text {
            self.flags | UTF8_NAME_FLAG
        } else {
            self.flags
        }
    }
}

/// A member's records, ready to write.
struct MemberRecord<'a> {
    member: &'a TestMember,
    compressed: Vec<u8>,
    crc32: u32,
    header_offset: u32,
}

impl MemberRecord<'_> {
    /// The fields from "flags" to "name length", in the order both
    /// records store them, for a record naming `name`.
    fn common_fields(&self, name: &[u8], bytes: &mut Vec<u8>) {
        let declared_size = self.member.declared_size.unwrap_or(self.member.data.len() as u64);
        let compressed_size = self
            .member
            .declared_compressed_size
            .unwrap_or(self.compressed.len() as u64);
        bytes.extend(self.member.stored_flags().to_le_bytes());
        bytes.extend(self.member.compression.number().to_le_bytes());
        bytes.extend(DOS_TIME.to_le_bytes());
        bytes.extend(DOS_DATE.to_le_bytes());
        bytes.extend(self.crc32.to_le_bytes());
        bytes.extend(small(compressed_size).to_le_bytes());
        bytes.extend(small(declared_size).to_le_bytes());
        bytes.extend(short(name.len()).to_le_bytes());
    }

    /// The local header, name and data.
    fn write_local(&self, archive: &mut Vec<u8>) {
        let name = self.member.local_name.as_deref().unwrap_or(&self.member.raw_name);
        archive.extend(LOCAL_HEADER_SIGNATURE.to_le_bytes());
        archive.extend(self.member.version_needed.to_le_bytes());
        self.common_fields(name, archive);
        archive.extend(0u16.to_le_bytes());
        archive.extend(name);
        archive.extend(&self.compressed);
    }

    /// The central directory record and name.
    fn write_central(&self, directory: &mut Vec<u8>) {
        directory.extend(CENTRAL_HEADER_SIGNATURE.to_le_bytes());
        directory.extend(VERSION_MADE_BY.to_le_bytes());
        directory.extend(self.member.version_needed.to_le_bytes());
        self.common_fields(&self.member.raw_name, directory);
        // Extra field and comment lengths, disk number, internal attributes.
        directory.extend([0u8; 8]);
        directory.extend((self.member.unix_mode << 16).to_le_bytes());
        directory.extend(self.header_offset.to_le_bytes());
        directory.extend(&self.member.raw_name);
    }
}

/// A 32-bit ZIP field; test archives are small.
fn small(value: u64) -> u32 {
    u32::try_from(value).expect("test archives stay below 4 GiB")
}

/// A 16-bit ZIP field.
fn short(value: usize) -> u16 {
    u16::try_from(value).expect("test names and comments fit 16 bits")
}

/// The records that locate the central directory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EndRecords {
    /// The end of central directory record alone.
    #[default]
    Classic,
    /// ZIP64 end records, with the classic record holding only markers.
    Zip64,
}

/// How [`zip_bytes_with`] lays out an archive around its members.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchiveLayout {
    /// Bytes in front of the archive, as in a self-extracting archive. The
    /// recorded offsets do not count them.
    pub prefix: Vec<u8>,
    /// The archive comment after the end record.
    pub comment: Vec<u8>,
    /// The records that locate the central directory.
    pub end_records: EndRecords,
}

/// Where the central directory is, as the end records declare it.
#[derive(Debug, Clone, Copy)]
struct DirectoryPlace {
    member_count: usize,
    size: u64,
    offset: u64,
}

/// The bytes of a ZIP archive holding `members` in order, without ZIP64
/// records or comments.
///
/// # Panics
///
/// When the archive does not fit the 32-bit ZIP fields.
pub fn zip_bytes(members: &[TestMember]) -> Vec<u8> {
    zip_bytes_with(members, &ArchiveLayout::default())
}

/// The bytes of a ZIP archive holding `members` in order, laid out as
/// `layout` says.
///
/// # Panics
///
/// When the archive does not fit the 32-bit ZIP fields.
pub fn zip_bytes_with(members: &[TestMember], layout: &ArchiveLayout) -> Vec<u8> {
    let mut archive = Vec::new();
    let mut directory = Vec::new();
    for member in members {
        let record = MemberRecord {
            member,
            compressed: member.compression.compress(&member.data),
            crc32: crc32fast::hash(&member.data),
            header_offset: small(archive.len() as u64),
        };
        record.write_local(&mut archive);
        record.write_central(&mut directory);
    }
    let place = DirectoryPlace {
        member_count: members.len(),
        size: directory.len() as u64,
        offset: archive.len() as u64,
    };
    archive.extend(directory);
    archive.extend(end_records(place, layout));
    archive.extend(&layout.comment);
    [layout.prefix.as_slice(), archive.as_slice()].concat()
}

/// The records after the central directory at `place`.
fn end_records(place: DirectoryPlace, layout: &ArchiveLayout) -> Vec<u8> {
    let comment_length = short(layout.comment.len());
    match layout.end_records {
        EndRecords::Classic => end_of_directory(place, comment_length),
        EndRecords::Zip64 => {
            let markers = DirectoryPlace {
                member_count: usize::from(u16::MAX),
                size: u64::from(u32::MAX),
                offset: u64::from(u32::MAX),
            };
            [
                zip64_end_records(place),
                end_of_directory(markers, comment_length),
            ]
            .concat()
        }
    }
}

/// The end of central directory record.
fn end_of_directory(place: DirectoryPlace, comment_length: u16) -> Vec<u8> {
    let count = u16::try_from(place.member_count).unwrap_or(u16::MAX);
    let mut record = END_OF_DIRECTORY_SIGNATURE.to_le_bytes().to_vec();
    record.extend([0u8; 4]);
    record.extend(count.to_le_bytes());
    record.extend(count.to_le_bytes());
    record.extend(small(place.size).to_le_bytes());
    record.extend(small(place.offset).to_le_bytes());
    record.extend(comment_length.to_le_bytes());
    record
}

/// The ZIP64 end of central directory record and its locator.
fn zip64_end_records(place: DirectoryPlace) -> Vec<u8> {
    let count = place.member_count as u64;
    let mut records = ZIP64_END_SIGNATURE.to_le_bytes().to_vec();
    // The record's size without its signature and this field.
    records.extend(44u64.to_le_bytes());
    records.extend(VERSION_MADE_BY.to_le_bytes());
    records.extend(ZIP64_VERSION_NEEDED.to_le_bytes());
    records.extend([0u8; 8]);
    records.extend(count.to_le_bytes());
    records.extend(count.to_le_bytes());
    records.extend(place.size.to_le_bytes());
    records.extend(place.offset.to_le_bytes());
    records.extend(ZIP64_LOCATOR_SIGNATURE.to_le_bytes());
    records.extend(0u32.to_le_bytes());
    records.extend((place.offset + place.size).to_le_bytes());
    records.extend(1u32.to_le_bytes());
    records
}

/// Where a stored member's data starts when it is the first member: after
/// the 30-byte local header and the name.
pub fn first_member_data_offset(name: &str) -> usize {
    30 + name.len()
}

/// Writes an archive whose end record declares a central directory of
/// `directory_size` bytes, as a sparse file of that size.
///
/// # Panics
///
/// When the file cannot be written.
pub fn write_archive_declaring_directory(path: &Path, directory_size: u32) {
    let file = fs::File::create(path).expect("create the archive");
    file.set_len(u64::from(directory_size))
        .expect("extend the archive");
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("reopen the archive");
    let place = DirectoryPlace {
        member_count: 0,
        size: u64::from(directory_size),
        offset: 0,
    };
    file.write_all(&end_of_directory(place, 0))
        .expect("write the end record");
}
