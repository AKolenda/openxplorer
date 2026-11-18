// SPDX-License-Identifier: AGPL-3.0-only
//! 7z archives, read into the same member records as a ZIP or a TAR so
//! that browsing, opening and extracting apply exactly the same rules
//! (ARC-022, ARC-024), as Windows 11 reads 7z in File Explorer.
//!
//! The headers are read and checked here (`header`), and a listing comes
//! from them alone: nothing is decompressed to browse. They are kept for
//! the archives browsed last (`cache`), so going from folder to folder
//! reads only the 32 bytes that say whether the archive changed.
//!
//! The data is decoded by the `sevenz-rust2` crate, on a thread of its own
//! that hands the files of one block over in order (`decode`). A solid
//! archive packs many files into one block, which can only be decoded
//! from its start: reading files in archive order, as extracting does,
//! decodes each block once; going back to an earlier file starts its
//! block again, as 7-Zip does.
//!
//! The reader decodes LZMA, LZMA2, Copy, Delta, the branch filters 7-Zip
//! pairs with them (BCJ2 included) and `BZip2`. Encrypted archives and other
//! methods (`PPMd`, Deflate) are listed when their names can be read, and
//! say plainly that they aren't supported when a file is opened or
//! extracted.

mod cache;
mod decode;
mod header;

use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

use super::source::{ArchiveOpener, ArchiveStream};
use super::zip::{CompressionMethod, DosDateTime, SevenZipCoding, ZipMember};
use super::{ArchiveError, ExtractionLimits};
use crate::transfer::Cancellation;
use decode::{BlockStream, MemberChunks};
use header::{BlockProblem, Headers, K_ENCODED_HEADER, SIGNATURE_HEADER_BYTES};

pub(crate) use header::SIGNATURE;

/// The flags of an encrypted ZIP member that the reader cannot decrypt.
const ENCRYPTED_FLAGS: u16 = 1 | 1 << 6;
/// The attribute 7-Zip and p7zip set when the high 16 bits hold the
/// item's Unix mode.
const UNIX_EXTENSION: u32 = 0x8000;
/// Windows' attribute of a link or other reparse point.
const REPARSE_POINT: u32 = 0x400;
/// Unix file types, as a ZIP records them in its external attributes.
const REGULAR_FILE: u32 = 0o100_644;
const DIRECTORY: u32 = 0o040_755;
const SYMBOLIC_LINK: u32 = 0o120_777;
/// Hundreds of nanoseconds from 1601-01-01 to 1970-01-01.
const NT_UNIX_EPOCH: u64 = 116_444_736_000_000_000;
const NT_TICKS_PER_SECOND: u64 = 10_000_000;
/// The property ids of an empty substreams record.
const SUB_STREAMS_INFO: u8 = 0x08;
const END: u8 = 0x00;

/// The checked headers of one archive, shared by its listings and the
/// threads decoding it.
#[derive(Debug)]
pub(super) struct Parsed {
    headers: Headers,
    /// The archive's length in bytes.
    length: u64,
    /// Its first 32 bytes, which say where its last header is and hold
    /// that header's checksum: the decoder checks the archive it opens
    /// again against them.
    start: [u8; 32],
    /// The header checked here, unpacked when the archive packs it: the
    /// only header the decoder hands the `sevenz-rust2` reader.
    checked_header: Arc<[u8]>,
}

/// An open 7z archive.
pub(crate) struct SevenZipArchive {
    parsed: Arc<Parsed>,
    members: Vec<ZipMember>,
    /// The file each member is: anti-items are not members.
    member_files: Vec<usize>,
    opener: Arc<dyn ArchiveOpener>,
    uri: String,
    cancel: Cancellation,
    /// The block being decoded, if any.
    stream: Option<BlockStream>,
}

impl std::fmt::Debug for SevenZipArchive {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SevenZipArchive")
            .field("members", &self.members.len())
            .finish_non_exhaustive()
    }
}

impl SevenZipArchive {
    /// Reads and checks the headers of the 7z in `stream`, the archive
    /// at `uri`, which `opener` opens again to decode data.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::DamagedSevenZip`] for a damaged or cut archive,
    /// [`ArchiveError::TooManyMembers`],
    /// [`ArchiveError::SevenZipEncryption`] when the names are encrypted,
    /// [`ArchiveError::SevenZipMethod`] for a header packed with a method
    /// the reader lacks, or the source's read error.
    pub(crate) fn open(
        mut stream: Box<dyn ArchiveStream>,
        opener: Arc<dyn ArchiveOpener>,
        uri: &str,
        cancel: &Cancellation,
    ) -> Result<Self, ArchiveError> {
        let length = stream.seek(SeekFrom::End(0))?;
        stream.seek(SeekFrom::Start(0))?;
        let mut start = [0u8; 32];
        stream.read_exact(&mut start).map_err(damaged)?;
        let key = cache::Key::new(uri, length, start);
        let parsed = if let Some(parsed) = cache::find(&key) {
            parsed
        } else {
            let parsed = Arc::new(read_headers(stream.as_mut(), &start, length, cancel)?);
            cache::keep(key, Arc::clone(&parsed));
            parsed
        };
        let (members, member_files) = members(&parsed.headers);
        Ok(Self {
            parsed,
            members,
            member_files,
            opener,
            uri: uri.to_owned(),
            cancel: cancel.clone(),
            stream: None,
        })
    }

    /// Every file and folder, in archive order.
    pub(crate) fn members(&self) -> &[ZipMember] {
        &self.members
    }

    /// Starts reading the data of the member at `index`, decoding its
    /// block from where the last member read left it, or from its start.
    ///
    /// # Errors
    ///
    /// The block's problem (encryption, a method or dictionary the reader
    /// does not take), damaged data, or the source's error.
    ///
    /// # Panics
    ///
    /// When `index` is not an index of [`Self::members`].
    pub(crate) fn open_member(&mut self, index: usize) -> Result<SevenZipMemberReader<'_>, ArchiveError> {
        let index = self.member_files[index];
        let file = &self.parsed.headers.files[index];
        if !file.has_stream {
            return Ok(SevenZipMemberReader { chunks: None });
        }
        let block_index = file.block.ok_or(ArchiveError::DamagedSevenZip)?;
        if let Some(problem) = self.parsed.headers.blocks[block_index].problem {
            return Err(problem.error());
        }
        let reusable = self
            .stream
            .as_ref()
            .filter(|stream| stream.can_reach(block_index, index));
        let is_reusable = reusable.is_some();
        // ARC-006, ARC-017: every file of the block before this one is
        // decoded too, though only this one is wanted and checked.
        let from = reusable.map_or(0, BlockStream::next_file);
        let skipped = self.parsed.headers.files[from..index]
            .iter()
            .filter(|earlier| earlier.has_stream && earlier.block == Some(block_index))
            .fold(0u64, |sum, earlier| sum.saturating_add(earlier.size));
        check_skipped(skipped, &self.parsed.headers.blocks[block_index])?;
        if !is_reusable {
            self.stream = Some(BlockStream::start(
                Arc::clone(&self.opener),
                self.uri.clone(),
                Arc::clone(&self.parsed),
                block_index,
                self.cancel.clone(),
            ));
        }
        let stream = self.stream.as_mut().ok_or(ArchiveError::DamagedSevenZip)?;
        let chunks = stream.member(index)?;
        Ok(SevenZipMemberReader { chunks: Some(chunks) })
    }
}

/// Bytes before a wanted file that are decoded without any check: small
/// enough that a solid block of any ratio stays cheap.
const UNCHECKED_SKIP_BYTES: u64 = 1024 * 1024;

/// ARC-006, ARC-017: refuses to decode `skipped` bytes of `block` before
/// the file wanted, as a member that large would be refused: over the
/// per-member size limit, or from a block compressed beyond the ratio
/// limit. Without this, a huge, highly compressed stream placed before a
/// small file would be decompressed past every limit to open or copy it.
fn check_skipped(skipped: u64, block: &header::Block) -> Result<(), ArchiveError> {
    let limits = ExtractionLimits::default();
    let is_over_ratio = block.unpack_size > block.packed_size.max(1).saturating_mul(limits.max_ratio);
    if skipped > UNCHECKED_SKIP_BYTES && (skipped > limits.max_member_bytes || is_over_ratio) {
        return Err(ArchiveError::MemberTooLarge);
    }
    Ok(())
}

/// The data of one 7z member being read.
pub(crate) struct SevenZipMemberReader<'a> {
    /// `None` for a member without data.
    chunks: Option<MemberChunks<'a>>,
}

impl SevenZipMemberReader<'_> {
    /// Reads the next bytes into `buffer`; 0 at the member's end.
    ///
    /// # Errors
    ///
    /// Damaged data, the source's read error or a cancellation.
    pub(crate) fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, ArchiveError> {
        match &mut self.chunks {
            Some(chunks) => chunks.read(buffer),
            None => Ok(0),
        }
    }
}

/// Reads the headers after the start header `start` of an archive of
/// `length` bytes, unpacking a packed header.
fn read_headers(
    stream: &mut dyn ArchiveStream,
    start: &[u8; 32],
    length: u64,
    cancel: &Cancellation,
) -> Result<Parsed, ArchiveError> {
    let next = header::next_header(start, length)?;
    cancel.check()?;
    let bytes = read_at(stream, SIGNATURE_HEADER_BYTES + next.offset, next.size)?;
    if crc32fast::hash(&bytes) != next.crc {
        return Err(ArchiveError::DamagedSevenZip);
    }
    let plain = if bytes.first() == Some(&K_ENCODED_HEADER) {
        let encoded = header::encoded_header(&bytes[1..], length)?;
        cancel.check()?;
        decode::unpack_header(stream, &encoded.block)?
    } else {
        bytes
    };
    let headers = header::plain_header(&plain, length)?;
    let checked_header = with_sub_streams_record(plain, headers.missing_sub_streams_at);
    Ok(Parsed {
        headers,
        length,
        start: *start,
        checked_header: checked_header.into(),
    })
}

/// `header` with an empty substreams record (`kSubStreamsInfo`, `kEnd`)
/// put `at` the place the archive left it out: it means one file per
/// block either way, and the `sevenz-rust2` reader needs the record.
fn with_sub_streams_record(mut header: Vec<u8>, at: Option<usize>) -> Vec<u8> {
    if let Some(at) = at {
        header.splice(at..at, [SUB_STREAMS_INFO, END]);
    }
    header
}

/// `size` bytes at `offset` of `stream`.
fn read_at(stream: &mut dyn ArchiveStream, offset: u64, size: u64) -> Result<Vec<u8>, ArchiveError> {
    stream.seek(SeekFrom::Start(offset))?;
    let size = usize::try_from(size).map_err(|_| ArchiveError::DamagedSevenZip)?;
    let mut bytes = vec![0u8; size];
    stream.read_exact(&mut bytes).map_err(damaged)?;
    Ok(bytes)
}

/// The member records of every file and folder in `headers`, and the
/// index of the file each one is. An item that deletes another when an
/// archive is updated ("anti-item") is left out, as 7-Zip leaves it out
/// when it extracts.
fn members(headers: &Headers) -> (Vec<ZipMember>, Vec<usize>) {
    headers
        .files
        .iter()
        .enumerate()
        .filter(|(_, file)| !file.is_anti)
        .map(|(index, file)| {
            // An empty file between two files of a solid block is given
            // that block, but nothing of it is decoded to open it.
            let block = file
                .block
                .filter(|_| file.has_stream)
                .map(|index| &headers.blocks[index]);
            let coding = match block.and_then(|block| block.problem) {
                None => SevenZipCoding::Readable,
                Some(BlockProblem::Encrypted) => SevenZipCoding::Encrypted,
                Some(BlockProblem::Method) => SevenZipCoding::Unsupported,
                Some(BlockProblem::Memory) => SevenZipCoding::TooMuchMemory,
            };
            // A Windows folder separator, as 7-Zip reads it.
            let mut name = file.name.replace('\\', "/");
            if file.is_directory && !name.ends_with('/') {
                name.push('/');
            }
            let compressed_size = block.map_or(0, |block| {
                let share = (u128::from(file.size) * u128::from(block.packed_size))
                    .div_ceil(u128::from(block.unpack_size.max(1)));
                u64::try_from(share).unwrap_or(u64::MAX)
            });
            let member = ZipMember {
                original_name: name.clone(),
                name,
                flags: if coding == SevenZipCoding::Encrypted {
                    ENCRYPTED_FLAGS
                } else {
                    0
                },
                method: CompressionMethod::SevenZip(coding),
                crc32: 0,
                compressed_size,
                size: file.size,
                header_offset: 0,
                data_limit: 0,
                external_attributes: file_type(file) << 16,
                modified: file
                    .modified
                    .and_then(|time| time.checked_sub(NT_UNIX_EPOCH))
                    .map_or(DosDateTime::UNKNOWN, |time| {
                        DosDateTime::from_unix_seconds(time / NT_TICKS_PER_SECOND)
                    }),
            };
            (member, index)
        })
        .unzip()
}

/// The Unix type and mode of `file`, as a ZIP records them: the mode 7-Zip
/// and p7zip store beside the attributes, or one from the attributes.
fn file_type(file: &header::FileRecord) -> u32 {
    let attributes = file.attributes.unwrap_or(0);
    if attributes & UNIX_EXTENSION != 0 && attributes >> 16 != 0 {
        return attributes >> 16;
    }
    if attributes & REPARSE_POINT != 0 {
        SYMBOLIC_LINK
    } else if file.is_directory {
        DIRECTORY
    } else {
        REGULAR_FILE
    }
}

/// A read that ran out of bytes means the archive was cut short.
fn damaged(error: std::io::Error) -> ArchiveError {
    match error.kind() {
        std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::InvalidData => ArchiveError::DamagedSevenZip,
        _ => error.into(),
    }
}
