// SPDX-License-Identifier: AGPL-3.0-only
//! Finding and reading the central directory, the archive's table of
//! contents. Ports `_EndRecData`, `_EndRecData64` and `_RealGetContents` of
//! Python's `zipfile`, including its tolerance for archives with bytes in
//! front of them (self-extracting archives) and its ZIP-bomb check for
//! members whose data overlaps the next member.
//!
//! Safety rule (ARC-005, `BoundedReader` in `v2.0.0:desktop/archives.py`): the
//! directory is read into memory only when it is at most
//! [`MAX_DIRECTORY_BYTES`] long, so a forged size cannot make the reader
//! allocate gigabytes before anything else is checked.

use std::io::{self, Read, Seek, SeekFrom};

use super::error::ZipFormatError;
use super::extra::apply_extra_fields;
use super::member::{CompressionMethod, DosDateTime, ZipMember};
use super::names::{cut_at_nul, decode_name};
use super::records::{CentralHeader, EndOfDirectory, Zip64EndOfDirectory, Zip64Locator};
use crate::archive::ArchiveError;

/// The largest central directory the built-in viewer reads (32 MiB).
pub(crate) const MAX_DIRECTORY_BYTES: u64 = 32 * 1024 * 1024;
/// An archive comment is at most 64 KiB, so the end record is within
/// this many bytes of the end.
const MAX_COMMENT_BYTES: u64 = 64 * 1024;
/// The newest ZIP version `zipfile` extracts (6.3).
const MAX_VERSION_NEEDED: u8 = 63;

/// The central directory: the members, and where the archive starts within
/// its file.
#[derive(Debug)]
pub(super) struct CentralDirectory {
    pub(super) members: Vec<ZipMember>,
    /// How far the archive is shifted within its file: positive when bytes
    /// were put in front of it, negative for a corrupt recorded offset.
    /// Recorded offsets plus this value are file positions.
    pub(super) shift: i128,
}

/// Where the central directory is, from the end records.
#[derive(Debug, Clone, Copy)]
struct DirectoryLocation {
    /// The directory's size in bytes.
    size: u64,
    /// The directory's offset as the archive records it.
    recorded_offset: u64,
    /// The file position right after the directory: where the (ZIP64) end
    /// record starts.
    end: u64,
}

/// Reads the central directory of the archive in `source`.
///
/// # Errors
///
/// [`ArchiveError::DirectoryTooLarge`], a [`ZipFormatError`] for a
/// damaged or unsupported archive, or the source's read error.
pub(super) fn read_directory(source: &mut (impl Read + Seek)) -> Result<CentralDirectory, ArchiveError> {
    let location = locate_directory(source)?;
    let start = location
        .end
        .checked_sub(location.size)
        .ok_or(ZipFormatError::BadDirectoryOffset)?;
    // ARC-005: refuse a huge directory before reading it.
    if location.size > MAX_DIRECTORY_BYTES {
        return Err(ArchiveError::DirectoryTooLarge);
    }
    source.seek(SeekFrom::Start(start))?;
    let mut directory = Vec::new();
    source.take(location.size).read_to_end(&mut directory)?;
    let mut members = parse_members(&directory, location.size)?;
    assign_data_limits(&mut members, location.recorded_offset);
    Ok(CentralDirectory {
        members,
        shift: i128::from(start) - i128::from(location.recorded_offset),
    })
}

/// Finds the end records and returns where the directory is.
fn locate_directory(source: &mut (impl Read + Seek)) -> Result<DirectoryLocation, ArchiveError> {
    let (end_record, position) = find_end_record(source)?.ok_or(ZipFormatError::NotAZip)?;
    let location = DirectoryLocation {
        size: u64::from(end_record.size),
        recorded_offset: u64::from(end_record.offset),
        end: position,
    };
    apply_zip64_record(source, location)
}

/// The end of central directory record and its position. It is either the
/// last 22 bytes, or followed by a comment of at most 64 KiB, in which case
/// the last signature in that range counts (`_EndRecData`).
fn find_end_record(source: &mut (impl Read + Seek)) -> io::Result<Option<(EndOfDirectory, u64)>> {
    let file_size = source.seek(SeekFrom::End(0))?;
    let record_size = EndOfDirectory::SIZE as u64;
    let Some(last_record_position) = file_size.checked_sub(record_size) else {
        return Ok(None);
    };
    source.seek(SeekFrom::Start(last_record_position))?;
    let mut last_record = [0u8; EndOfDirectory::SIZE];
    source.read_exact(&mut last_record)?;
    let has_no_comment = last_record.ends_with(&[0, 0]);
    let record = EndOfDirectory::parse(&last_record);
    if record.signature == EndOfDirectory::SIGNATURE && has_no_comment {
        return Ok(Some((record, last_record_position)));
    }
    let search_start = file_size.saturating_sub(MAX_COMMENT_BYTES + record_size);
    source.seek(SeekFrom::Start(search_start))?;
    let mut tail = Vec::new();
    source.read_to_end(&mut tail)?;
    Ok(last_end_record_in(&tail).map(|(record, index)| (record, search_start + index as u64)))
}

/// The last complete end record in `tail` and its index there.
fn last_end_record_in(tail: &[u8]) -> Option<(EndOfDirectory, usize)> {
    let signature = EndOfDirectory::SIGNATURE.to_le_bytes();
    let index = tail
        .windows(signature.len())
        .rposition(|window| window == signature)?;
    let record = tail.get(index..)?.first_chunk::<{ EndOfDirectory::SIZE }>()?;
    Some((EndOfDirectory::parse(record), index))
}

/// Replaces the location with the ZIP64 end record's, when a ZIP64
/// locator precedes the end record at `location.end` (`_EndRecData64`).
fn apply_zip64_record(
    source: &mut (impl Read + Seek),
    location: DirectoryLocation,
) -> Result<DirectoryLocation, ArchiveError> {
    let Some(locator_position) = location.end.checked_sub(Zip64Locator::SIZE as u64) else {
        return Ok(location);
    };
    source.seek(SeekFrom::Start(locator_position))?;
    let mut locator_bytes = [0u8; Zip64Locator::SIZE];
    source.read_exact(&mut locator_bytes)?;
    let locator = Zip64Locator::parse(&locator_bytes);
    if locator.signature != Zip64Locator::SIGNATURE {
        return Ok(location);
    }
    if locator.disk_with_record != 0 || locator.total_disks > 1 {
        return Err(ZipFormatError::MultipleDisks.into());
    }
    let adjacent_position = locator_position.checked_sub(Zip64EndOfDirectory::SIZE as u64);
    let adjacent_position = adjacent_position
        .filter(|adjacent| locator.record_offset <= *adjacent)
        .ok_or(ZipFormatError::CorruptZip64Locator)?;
    let (record, position) = read_zip64_record(source, locator.record_offset, adjacent_position)?;
    let extensible_data = adjacent_position - position;
    let directory_end = record.directory_offset.checked_add(record.directory_size);
    let is_consistent = directory_end == Some(locator.record_offset)
        && record
            .record_size
            .checked_add(Zip64EndOfDirectory::UNCOUNTED_BYTES)
            == Some(Zip64EndOfDirectory::SIZE as u64 + extensible_data);
    if !is_consistent {
        return Err(ZipFormatError::CorruptZip64Record.into());
    }
    Ok(DirectoryLocation {
        size: record.directory_size,
        recorded_offset: record.directory_offset,
        end: position,
    })
}

/// The ZIP64 end record and its position: at the recorded offset, or right
/// before the locator when bytes were put in front of the archive.
fn read_zip64_record(
    source: &mut (impl Read + Seek),
    recorded_position: u64,
    adjacent_position: u64,
) -> Result<(Zip64EndOfDirectory, u64), ArchiveError> {
    let mut candidates = vec![recorded_position];
    if recorded_position != adjacent_position {
        candidates.push(adjacent_position);
    }
    for position in candidates {
        source.seek(SeekFrom::Start(position))?;
        let mut bytes = [0u8; Zip64EndOfDirectory::SIZE];
        source.read_exact(&mut bytes)?;
        let record = Zip64EndOfDirectory::parse(&bytes);
        if record.signature == Zip64EndOfDirectory::SIGNATURE {
            return Ok((record, position));
        }
    }
    Err(ZipFormatError::MissingZip64Record.into())
}

/// Parses the records of a directory that declares `size` bytes. As in
/// `zipfile`, only a truncated fixed-size part is an error: names, extra
/// fields and comments cut short by the end of the data are read as far as
/// they go.
fn parse_members(directory: &[u8], size: u64) -> Result<Vec<ZipMember>, ZipFormatError> {
    let mut members = Vec::new();
    let mut rest = directory;
    let mut consumed = 0u64;
    while consumed < size {
        let (fixed, after_fixed) = rest
            .split_first_chunk::<{ CentralHeader::SIZE }>()
            .ok_or(ZipFormatError::TruncatedDirectory)?;
        let header = CentralHeader::parse(fixed);
        if header.signature != CentralHeader::SIGNATURE {
            return Err(ZipFormatError::BadDirectorySignature);
        }
        let (raw_name, after_name) = split_up_to(after_fixed, header.name_length);
        let (extra, after_extra) = split_up_to(after_name, header.extra_length);
        let (_comment, after_comment) = split_up_to(after_extra, header.comment_length);
        members.push(member_from(&header, raw_name, extra)?);
        consumed += header.total_length();
        rest = after_comment;
    }
    Ok(members)
}

/// The first `length` bytes of `bytes`, or all of them when fewer are left,
/// and the rest.
fn split_up_to(bytes: &[u8], length: u16) -> (&[u8], &[u8]) {
    bytes.split_at(bytes.len().min(usize::from(length)))
}

/// The member a central directory record describes.
fn member_from(header: &CentralHeader, raw_name: &[u8], extra: &[u8]) -> Result<ZipMember, ZipFormatError> {
    let original_name = decode_name(raw_name, header.flags)?;
    if header.version_needed > MAX_VERSION_NEEDED {
        return Err(ZipFormatError::UnsupportedVersion(header.version_needed));
    }
    let mut member = ZipMember {
        name: cut_at_nul(&original_name),
        original_name,
        flags: header.flags,
        method: CompressionMethod::from_number(header.method),
        crc32: header.crc32,
        compressed_size: u64::from(header.compressed_size),
        size: u64::from(header.size),
        header_offset: u64::from(header.header_offset),
        data_limit: 0,
        external_attributes: header.external_attributes,
        modified: DosDateTime {
            date: header.date,
            time: header.time,
        },
    };
    apply_extra_fields(&mut member, extra, crc32fast::hash(raw_name))?;
    Ok(member)
}

/// Gives each member the recorded offset its data must end before: the
/// next local header, or the directory for the last member. Members that
/// share a local header get a limit at their own header, so reading all
/// but the first of them fails as overlapping (`_end_offset`).
fn assign_data_limits(members: &mut [ZipMember], directory_offset: u64) {
    let mut by_descending_offset: Vec<usize> = (0..members.len()).collect();
    // A stable sort keeps members that share an offset in archive order.
    by_descending_offset
        .sort_by(|&first, &second| members[second].header_offset.cmp(&members[first].header_offset));
    let mut limit = directory_offset;
    for index in by_descending_offset {
        members[index].data_limit = limit;
        limit = members[index].header_offset;
    }
}
