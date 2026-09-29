// SPDX-License-Identifier: AGPL-3.0-only
//! TAR archives, plain or compressed with gzip, bzip2, XZ or Zstandard
//! (ARC-022, ARC-024), read into the same member records as a ZIP so that
//! browsing, previewing and extracting apply exactly the ZIP rules: unsafe
//! names, links, special files and the size limits are refused the same
//! way.
//!
//! A TAR has no directory, so opening one reads every header once, which
//! decompresses the whole archive. Reading a member then decompresses up
//! to it; going back starts again from the beginning. Extraction reads the
//! members in order, so it decompresses the archive once more.

mod header;

use std::cell::RefCell;
use std::io::{self, Read, Seek, SeekFrom};
use std::rc::Rc;

use super::source::ArchiveStream;
use super::zip::{CompressionMethod, DosDateTime, ZipMember};
use super::ArchiveError;
use crate::transfer::Cancellation;
use header::{is_end, parse, pax_path_and_size, HeaderKind, BLOCK};

/// The most bytes of a GNU long name or pax record set.
const MAX_METADATA_BYTES: u64 = 1024 * 1024;
/// The Unix file type bits of a regular file and a folder, as a ZIP
/// records them.
const REGULAR_FILE: u32 = 0o100_000;
const DIRECTORY: u32 = 0o040_000;
const SYMBOLIC_LINK: u32 = 0o120_000;

/// How the TAR stream is compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TarCompression {
    None,
    Gzip,
    Bzip2,
    Xz,
    Zstd,
}

impl TarCompression {
    /// The compression the first bytes of a file announce, or `None` when
    /// they are not a TAR this reader knows.
    pub(crate) fn detect(start: &[u8]) -> Option<Self> {
        if start.starts_with(&[0x1f, 0x8b]) {
            Some(Self::Gzip)
        } else if start.starts_with(b"BZh") {
            Some(Self::Bzip2)
        } else if start.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0]) {
            Some(Self::Xz)
        } else if start.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
            Some(Self::Zstd)
        } else if start.get(257..262) == Some(b"ustar".as_slice()) {
            Some(Self::None)
        } else {
            None
        }
    }
}

/// The archive's bytes, shared by the decoders read from it in turn.
#[derive(Clone)]
struct SharedStream(Rc<RefCell<Box<dyn ArchiveStream>>>);

impl Read for SharedStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.borrow_mut().read(buffer)
    }
}

/// An open TAR archive.
pub(crate) struct TarArchive {
    source: SharedStream,
    compression: TarCompression,
    members: Vec<ZipMember>,
    /// Where each member's data starts in the decompressed stream.
    data_offsets: Vec<u64>,
    /// The decompressed stream and how far it has been read.
    reader: Box<dyn Read>,
    position: u64,
    cancel: Cancellation,
}

impl std::fmt::Debug for TarArchive {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TarArchive")
            .field("compression", &self.compression)
            .field("members", &self.members.len())
            .finish_non_exhaustive()
    }
}

impl TarArchive {
    /// Reads every header of the archive in `source`.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::DamagedArchive`] for a stream that is not a TAR,
    /// [`ArchiveError::TooManyMembers`], the source's read error or
    /// [`ArchiveError::Cancelled`].
    pub(crate) fn open(
        source: Box<dyn ArchiveStream>,
        compression: TarCompression,
        max_members: usize,
        cancel: &Cancellation,
    ) -> Result<Self, ArchiveError> {
        let source = SharedStream(Rc::new(RefCell::new(source)));
        let reader = decoder(&source, compression)?;
        let mut archive = Self {
            source,
            compression,
            members: Vec::new(),
            data_offsets: Vec::new(),
            reader,
            position: 0,
            cancel: cancel.clone(),
        };
        archive.read_headers(max_members)?;
        Ok(archive)
    }

    /// Every member, in archive order.
    pub(crate) fn members(&self) -> &[ZipMember] {
        &self.members
    }

    /// Starts reading the data of the member at `index`.
    ///
    /// # Errors
    ///
    /// A damaged stream, the source's read error or
    /// [`ArchiveError::Cancelled`].
    pub(crate) fn open_member(&mut self, index: usize) -> Result<TarMemberReader<'_>, ArchiveError> {
        let start = self.data_offsets[index];
        if start < self.position {
            self.restart()?;
        }
        let gap = start - self.position;
        self.skip(gap)?;
        let remaining = self.members[index].size;
        Ok(TarMemberReader {
            archive: self,
            remaining,
        })
    }

    /// Reads the stream again from its first byte.
    fn restart(&mut self) -> Result<(), ArchiveError> {
        self.source.0.borrow_mut().seek(SeekFrom::Start(0))?;
        self.reader = decoder(&self.source, self.compression)?;
        self.position = 0;
        Ok(())
    }

    /// Reads every header, recording the members and where their data is.
    fn read_headers(&mut self, max_members: usize) -> Result<(), ArchiveError> {
        let mut long_name: Option<String> = None;
        let mut pax_size: Option<u64> = None;
        loop {
            self.cancel.check()?;
            let mut block = [0u8; BLOCK];
            if !self.read_block(&mut block)? || is_end(&block) {
                break;
            }
            let header = parse(&block).map_err(|_| ArchiveError::DamagedArchive)?;
            let metadata_blocks = padded(header.size);
            match header.kind {
                HeaderKind::LongName | HeaderKind::Pax => {
                    if header.size > MAX_METADATA_BYTES {
                        return Err(ArchiveError::DamagedArchive);
                    }
                    let data = self.read_data(metadata_blocks, header.size)?;
                    if header.kind == HeaderKind::LongName {
                        long_name = Some(header::text(&data));
                    } else {
                        let (path, size) = pax_path_and_size(&data);
                        long_name = path.or(long_name);
                        pax_size = size;
                    }
                    continue;
                }
                HeaderKind::LongLink | HeaderKind::GlobalPax => {
                    self.skip(metadata_blocks)?;
                    continue;
                }
                HeaderKind::File | HeaderKind::Folder | HeaderKind::Special => {}
            }
            let size = pax_size.take().unwrap_or(header.size);
            let name = long_name.take().unwrap_or(header.name);
            if let Some(member) = member(&name, header.kind, header.mode, size, header.modified) {
                if self.members.len() >= max_members {
                    return Err(ArchiveError::TooManyMembers);
                }
                self.members.push(member);
                self.data_offsets.push(self.position);
            }
            self.skip(padded(size))?;
        }
        Ok(())
    }

    /// Reads one block; false at the end of the stream.
    fn read_block(&mut self, block: &mut [u8; BLOCK]) -> Result<bool, ArchiveError> {
        let mut filled = 0;
        while filled < BLOCK {
            let count = self.reader.read(&mut block[filled..]).map_err(damaged)?;
            if count == 0 {
                return if filled == 0 {
                    Ok(false)
                } else {
                    Err(ArchiveError::DamagedArchive)
                };
            }
            filled += count;
        }
        self.position += BLOCK as u64;
        Ok(true)
    }

    /// Reads `padded` bytes of metadata and keeps the first `size`.
    fn read_data(&mut self, padded: u64, size: u64) -> Result<Vec<u8>, ArchiveError> {
        let mut data = vec![0u8; usize::try_from(padded).map_err(|_| ArchiveError::DamagedArchive)?];
        self.reader.read_exact(&mut data).map_err(damaged)?;
        self.position += padded;
        data.truncate(usize::try_from(size).unwrap_or(data.len()));
        Ok(data)
    }

    /// Skips `count` bytes of the decompressed stream.
    fn skip(&mut self, count: u64) -> Result<(), ArchiveError> {
        let mut left = count;
        let mut buffer = vec![0u8; 64 * 1024];
        while left > 0 {
            self.cancel.check()?;
            let wanted = usize::try_from(left.min(buffer.len() as u64)).unwrap_or(buffer.len());
            let read = self.reader.read(&mut buffer[..wanted]).map_err(damaged)?;
            if read == 0 {
                return Err(ArchiveError::DamagedArchive);
            }
            left -= read as u64;
            self.position += read as u64;
        }
        Ok(())
    }
}

/// Reads one member's data.
pub(crate) struct TarMemberReader<'a> {
    archive: &'a mut TarArchive,
    remaining: u64,
}

impl TarMemberReader<'_> {
    /// Reads the next bytes of the member into `buffer`; 0 at its end.
    ///
    /// # Errors
    ///
    /// A stream that ends inside the member, or the source's read error.
    pub(crate) fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, ArchiveError> {
        if self.remaining == 0 {
            return Ok(0);
        }
        self.archive.cancel.check()?;
        let wanted = usize::try_from(self.remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        let count = self.archive.reader.read(&mut buffer[..wanted]).map_err(damaged)?;
        if count == 0 {
            return Err(ArchiveError::TruncatedMember);
        }
        self.remaining -= count as u64;
        self.archive.position += count as u64;
        Ok(count)
    }
}

/// A decoder over the archive from its current position.
fn decoder(source: &SharedStream, compression: TarCompression) -> Result<Box<dyn Read>, ArchiveError> {
    let source = source.clone();
    Ok(match compression {
        TarCompression::None => Box::new(source),
        TarCompression::Gzip => Box::new(flate2::read::MultiGzDecoder::new(source)),
        TarCompression::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(source)),
        TarCompression::Xz => Box::new(lzma_rust2::XzReader::new(source, true)),
        TarCompression::Zstd => Box::new(
            ruzstd::decoding::StreamingDecoder::new(source).map_err(|_| ArchiveError::DamagedArchive)?,
        ),
    })
}

/// The member record of a TAR entry, or `None` for the archive's root
/// (`./`). A leading `./` is dropped, as `tar` writes it for `tar -C dir .`.
fn member(name: &str, kind: HeaderKind, mode: u32, size: u64, modified: u64) -> Option<ZipMember> {
    let mut name = name;
    while let Some(rest) = name.strip_prefix("./") {
        name = rest;
    }
    if name.is_empty() || name == "." {
        return None;
    }
    let (name, type_bits, size) = match kind {
        HeaderKind::Folder => {
            let folder = if name.ends_with('/') {
                name.to_owned()
            } else {
                format!("{name}/")
            };
            (folder, DIRECTORY, 0)
        }
        HeaderKind::File => (name.to_owned(), REGULAR_FILE, size),
        _ => (name.to_owned(), SYMBOLIC_LINK, 0),
    };
    Some(ZipMember {
        original_name: name.clone(),
        name,
        flags: 0,
        method: CompressionMethod::Stored,
        crc32: 0,
        compressed_size: size,
        size,
        header_offset: 0,
        data_limit: 0,
        external_attributes: (type_bits | mode) << 16,
        modified: DosDateTime::from_unix_seconds(modified),
    })
}

/// `size` rounded up to whole blocks.
fn padded(size: u64) -> u64 {
    size.div_ceil(BLOCK as u64) * BLOCK as u64
}

/// A decompression failure is damaged data.
fn damaged(error: io::Error) -> ArchiveError {
    if error.kind() == io::ErrorKind::UnexpectedEof || error.kind() == io::ErrorKind::InvalidData {
        ArchiveError::DamagedArchive
    } else {
        error.into()
    }
}

#[cfg(test)]
pub(super) mod tests {
    use std::io::{Cursor, Write};

    use super::header::ustar;
    use super::*;

    /// A TAR of `entries` (name, kind, data), with the end blocks.
    pub(crate) fn tar_of(entries: &[(&str, u8, &[u8])]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for (name, kind, data) in entries {
            let size = data.len() as u64;
            bytes.extend_from_slice(&ustar(name, *kind, 0o644, size));
            bytes.extend_from_slice(data);
            bytes.resize(bytes.len() + usize::try_from(padded(size) - size).unwrap(), 0);
        }
        bytes.extend_from_slice(&[0u8; 2 * BLOCK]);
        bytes
    }

    /// `tar` compressed with gzip.
    pub(crate) fn gzipped(tar: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(tar).unwrap();
        encoder.finish().unwrap()
    }

    fn open(bytes: Vec<u8>) -> TarArchive {
        let compression = TarCompression::detect(&bytes).expect("a TAR");
        TarArchive::open(
            Box::new(Cursor::new(bytes)),
            compression,
            100,
            &Cancellation::new(),
        )
        .expect("opens")
    }

    fn read_all(archive: &mut TarArchive, index: usize) -> Vec<u8> {
        let mut reader = archive.open_member(index).expect("a member");
        let mut data = Vec::new();
        let mut block = [0u8; 7];
        loop {
            let count = reader.read_chunk(&mut block).expect("reads");
            if count == 0 {
                return data;
            }
            data.extend_from_slice(&block[..count]);
        }
    }

    /// parity: ARC-024
    #[test]
    fn a_gzipped_tar_lists_its_members_and_reads_them_in_any_order() {
        let tar = tar_of(&[
            ("./", b'5', b""),
            ("./docs", b'5', b""),
            ("./docs/plan.txt", b'0', b"the plan"),
            ("./link", b'2', b""),
            ("./notes.txt", b'0', b"notes"),
        ]);
        let mut archive = open(gzipped(&tar));

        let names: Vec<&str> = archive
            .members()
            .iter()
            .map(|member| member.name.as_str())
            .collect();
        assert_eq!(names, ["docs/", "docs/plan.txt", "link", "notes.txt"]);
        assert!(archive.members()[0].is_directory());
        assert_eq!(
            archive.members()[2].file_type(),
            super::super::zip::MemberFileType::LinkOrSpecial
        );
        assert_eq!(read_all(&mut archive, 3), b"notes");
        assert_eq!(read_all(&mut archive, 1), b"the plan", "going back starts again");
    }
}
