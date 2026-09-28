// SPDX-License-Identifier: AGPL-3.0-only
//! Reading one member's data. Ports `ZipFile.open` and `ZipExtFile` of
//! Python's `zipfile`.
//!
//! Before any data is read, the local header must carry the name the
//! central directory records, and the data must end before the next
//! member starts (the overlapping-members ZIP bomb). The data is then
//! decompressed, never yielding more than the member's declared size, and
//! its CRC-32 is checked when the member ends: a corrupt member fails
//! instead of passing damaged bytes on.

use std::io::{self, Read, Seek, SeekFrom, Take};

use super::error::ZipFormatError;
use super::member::{CompressionMethod, ZipMember};
use super::names::decode_name;
use super::records::LocalHeader;
use crate::archive::ArchiveError;
use crate::transfer::Cancellation;

/// General purpose flag bit 5: compressed patched data.
const COMPRESSED_PATCHED_DATA_FLAG: u16 = 1 << 5;
/// General purpose flag bit 6: strong encryption.
const STRONG_ENCRYPTION_FLAG: u16 = 1 << 6;
/// The LZMA properties in front of ZIP LZMA data: the lc/lp/pb byte and the
/// dictionary size.
const LZMA_PROPERTIES_SIZE: usize = 5;

/// A member's decompressed data, read in chunks.
pub(crate) struct MemberReader<'a> {
    decompressed: Box<dyn Read + 'a>,
    name: String,
    /// Decompressed bytes the member may still yield.
    remaining: u64,
    checksum: crc32fast::Hasher,
    expected_checksum: u32,
    finished: bool,
    cancel: Cancellation,
}

impl<'a> MemberReader<'a> {
    /// Checks `member`'s local header at file position `header_position`
    /// and starts decompressing its data. A read that fails after `cancel`
    /// was cancelled reports the cancellation.
    ///
    /// # Errors
    ///
    /// A [`ZipFormatError`] for a damaged header, a name that differs from
    /// the central directory's, overlapping data, flags or a compression
    /// method `zipfile` does not support, or an encrypted member; the
    /// source's read error or [`ArchiveError::Cancelled`].
    pub(super) fn open<S: Read + Seek>(
        source: &'a mut S,
        member: &ZipMember,
        header_position: u64,
        cancel: &Cancellation,
    ) -> Result<Self, ArchiveError> {
        Self::start(source, member, header_position, cancel).map_err(|error| error.unless_cancelled(cancel))
    }

    /// [`Self::open`] without the cancellation check of its errors.
    fn start<S: Read + Seek>(
        source: &'a mut S,
        member: &ZipMember,
        header_position: u64,
        cancel: &Cancellation,
    ) -> Result<Self, ArchiveError> {
        source.seek(SeekFrom::Start(header_position))?;
        let header = read_local_header(source)?;
        let mut raw_name = Vec::new();
        source
            .by_ref()
            .take(u64::from(header.name_length))
            .read_to_end(&mut raw_name)?;
        source.seek(SeekFrom::Current(i64::from(header.extra_length)))?;
        check_flags(member)?;
        check_local_name(member, &raw_name, header.flags)?;
        check_no_overlap(member, &header)?;
        if member.is_encrypted() {
            return Err(ZipFormatError::PasswordRequired(member.name.clone()).into());
        }
        let compressed = source.take(member.compressed_size);
        Ok(Self {
            decompressed: decompressor(member, compressed)?,
            name: member.name.clone(),
            remaining: member.size,
            checksum: crc32fast::Hasher::new(),
            expected_checksum: member.crc32,
            finished: false,
            cancel: cancel.clone(),
        })
    }

    /// Reads the next decompressed bytes into `buffer` and returns how many
    /// were read; 0 once the member has ended.
    ///
    /// # Errors
    ///
    /// [`ZipFormatError::BadCrc`] when the member ends with data that does
    /// not match its checksum, [`ZipFormatError::CorruptData`] for data
    /// that cannot be decompressed, the source's read error, or
    /// [`ArchiveError::Cancelled`] for a read that failed after the user
    /// cancelled.
    pub(crate) fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, ArchiveError> {
        if self.finished {
            return Ok(0);
        }
        let allowed = usize::try_from(self.remaining).unwrap_or(usize::MAX);
        let window = buffer.len().min(allowed);
        let count = if window == 0 {
            0
        } else {
            self.decompressed
                .read(&mut buffer[..window])
                .map_err(|error| self.read_failure(error))?
        };
        self.checksum.update(&buffer[..count]);
        self.remaining -= count as u64;
        if count == 0 || self.remaining == 0 {
            self.finish()?;
        }
        Ok(count)
    }

    /// Ends the member and checks what it yielded against its CRC-32.
    fn finish(&mut self) -> Result<(), ArchiveError> {
        self.finished = true;
        let checksum = std::mem::take(&mut self.checksum).finalize();
        if checksum == self.expected_checksum {
            Ok(())
        } else {
            Err(ZipFormatError::BadCrc(self.name.clone()).into())
        }
    }

    /// Sorts a decompressor failure: damaged data, or a failing source.
    fn read_failure(&self, error: io::Error) -> ArchiveError {
        let failure: ArchiveError = match error.kind() {
            io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput | io::ErrorKind::UnexpectedEof => {
                ZipFormatError::CorruptData {
                    name: self.name.clone(),
                    detail: error.to_string(),
                }
                .into()
            }
            _ => error.into(),
        };
        failure.unless_cancelled(&self.cancel)
    }
}

/// Reads and checks the fixed part of a local header.
fn read_local_header(source: &mut impl Read) -> Result<LocalHeader, ArchiveError> {
    let mut bytes = [0u8; LocalHeader::SIZE];
    source
        .read_exact(&mut bytes)
        .map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => ZipFormatError::TruncatedHeader.into(),
            _ => ArchiveError::from(error),
        })?;
    let header = LocalHeader::parse(&bytes);
    if header.signature != LocalHeader::SIGNATURE {
        return Err(ZipFormatError::BadHeaderSignature.into());
    }
    Ok(header)
}

/// Refuses the flags `zipfile` cannot read.
fn check_flags(member: &ZipMember) -> Result<(), ZipFormatError> {
    if member.flags & COMPRESSED_PATCHED_DATA_FLAG != 0 {
        return Err(ZipFormatError::CompressedPatchedData);
    }
    if member.flags & STRONG_ENCRYPTION_FLAG != 0 {
        return Err(ZipFormatError::StrongEncryption);
    }
    Ok(())
}

/// Refuses a local header that names another file than the directory: the
/// two names are how one archive can show one file and extract another.
fn check_local_name(member: &ZipMember, raw_name: &[u8], flags: u16) -> Result<(), ZipFormatError> {
    let local_name = decode_name(raw_name, flags)?;
    if local_name == member.original_name {
        return Ok(());
    }
    Err(ZipFormatError::NameMismatch {
        directory: member.original_name.clone(),
        header: raw_name.to_vec(),
    })
}

/// Refuses data that would run into the next member or the directory.
fn check_no_overlap(member: &ZipMember, header: &LocalHeader) -> Result<(), ZipFormatError> {
    let header_length =
        LocalHeader::SIZE as u64 + u64::from(header.name_length) + u64::from(header.extra_length);
    let data_end = member
        .header_offset
        .saturating_add(header_length)
        .saturating_add(member.compressed_size);
    if data_end > member.data_limit {
        return Err(ZipFormatError::OverlappedEntries(member.original_name.clone()));
    }
    Ok(())
}

/// The decompressor for `member`'s method over its compressed bytes.
fn decompressor<'a, S: Read>(
    member: &ZipMember,
    compressed: Take<&'a mut S>,
) -> Result<Box<dyn Read + 'a>, ArchiveError> {
    match member.method {
        CompressionMethod::Stored => Ok(Box::new(compressed)),
        CompressionMethod::Deflated => Ok(Box::new(flate2::read::DeflateDecoder::new(compressed))),
        CompressionMethod::Bzip2 => Ok(Box::new(bzip2::read::BzDecoder::new(compressed))),
        CompressionMethod::Lzma => lzma_decompressor(member, compressed),
        CompressionMethod::Other(_) => Err(ZipFormatError::UnsupportedMethod.into()),
    }
}

/// ZIP's LZMA data starts with a version (2 bytes), the properties' size
/// (2 bytes) and the properties, followed by a raw LZMA stream. The stream
/// is read to the member's declared size, which also bounds the dictionary
/// the decoder allocates.
fn lzma_decompressor<'a, S: Read>(
    member: &ZipMember,
    mut compressed: Take<&'a mut S>,
) -> Result<Box<dyn Read + 'a>, ArchiveError> {
    let corrupt = |detail: String| ZipFormatError::CorruptData {
        name: member.name.clone(),
        detail,
    };
    let mut header = [0u8; 4];
    compressed
        .read_exact(&mut header)
        .map_err(|error| corrupt(error.to_string()))?;
    let properties_size = u16::from_le_bytes([header[2], header[3]]);
    if usize::from(properties_size) != LZMA_PROPERTIES_SIZE {
        return Err(corrupt("Invalid or unsupported options".to_owned()).into());
    }
    let mut properties = [0u8; LZMA_PROPERTIES_SIZE];
    compressed
        .read_exact(&mut properties)
        .map_err(|error| corrupt(error.to_string()))?;
    let [literal_properties, dictionary @ ..] = properties;
    let dictionary_size = u32::from_le_bytes(dictionary);
    let reader = lzma_rust2::LzmaReader::new_with_props(
        compressed,
        member.size,
        literal_properties,
        dictionary_size,
        None,
    )
    .map_err(|error| corrupt(error.to_string()))?;
    Ok(Box::new(reader))
}
