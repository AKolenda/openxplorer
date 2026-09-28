// SPDX-License-Identifier: AGPL-3.0-only
//! Writing the ZIP format: local headers, deflated data with data
//! descriptors, the central directory and its end record.
//!
//! The layout is the one of PKWARE's APPNOTE 6.3, as Python's `zipfile`
//! and every archive manager read it. File names are UTF-8 (flag bit 11),
//! files are deflated and followed by a data descriptor (flag bit 3), so
//! each file is read once, and folders are stored empty. ZIP64 is not
//! written: an archive over 4 GiB or with more than 65,535 entries is
//! refused before its end is written.

use std::io::{self, Read, Write};

use flate2::write::DeflateEncoder;
use flate2::Compression;

use crate::archive::ArchiveError;
use crate::transfer::Cancellation;

/// `PK\x03\x04`: a local file header.
const LOCAL_HEADER_SIGNATURE: u32 = 0x0403_4b50;
/// `PK\x01\x02`: a central directory header.
const CENTRAL_HEADER_SIGNATURE: u32 = 0x0201_4b50;
/// `PK\x07\x08`: a data descriptor.
const DATA_DESCRIPTOR_SIGNATURE: u32 = 0x0807_4b50;
/// `PK\x05\x06`: the end of the central directory.
const END_SIGNATURE: u32 = 0x0605_4b50;
/// Version 2.0: deflate and folders.
const VERSION_NEEDED: u16 = 20;
/// Made on Unix (3) with version 2.0 (0x14), so readers apply the Unix
/// modes below.
const VERSION_MADE_BY: u16 = (3 << 8) | 0x14;
/// Bit 3: sizes and CRC follow the data. Bit 11: the name is UTF-8.
const FILE_FLAGS: u16 = 0x0808;
/// Bit 11 only: a folder has no data and so no descriptor.
const FOLDER_FLAGS: u16 = 0x0800;
/// Compression method 0: stored.
const STORED: u16 = 0;
/// Compression method 8: deflated.
const DEFLATED: u16 = 8;
/// A folder's external attributes: `drwxr-xr-x` and the DOS folder bit.
const FOLDER_ATTRIBUTES: u32 = (0o040_755 << 16) | 0x10;
/// A file's external attributes: `-rw-r--r--`, never executable.
const FILE_ATTRIBUTES: u32 = 0o100_644 << 16;
/// The most entries an archive without ZIP64 can list.
pub(super) const MAX_ENTRIES: usize = u16::MAX as usize;
/// The chunk size for reading a file.
const CHUNK_BYTES: usize = 64 * 1024;

/// An MS-DOS date and time, as ZIP headers store them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DosTime {
    /// `(year - 1980) << 9 | month << 5 | day`.
    pub date: u16,
    /// `hour << 11 | minute << 5 | second / 2`.
    pub time: u16,
}

impl DosTime {
    /// 1 January 1980, the earliest time a ZIP can store.
    pub(super) const EARLIEST: DosTime = DosTime {
        date: (1 << 5) | 1,
        time: 0,
    };

    /// The local time `unix_seconds`, or [`Self::EARLIEST`] for a time a
    /// ZIP cannot store.
    pub(super) fn from_unix_seconds(unix_seconds: Option<u64>) -> Self {
        let local = unix_seconds
            .and_then(|seconds| i64::try_from(seconds).ok())
            .and_then(|seconds| glib::DateTime::from_unix_local(seconds).ok());
        let Some(local) = local else {
            return Self::EARLIEST;
        };
        let Some(years_since_1980) = local
            .year()
            .checked_sub(1980)
            .filter(|years| (0..128).contains(years))
        else {
            return Self::EARLIEST;
        };
        let field = |value: i32| u16::try_from(value).unwrap_or(0);
        Self {
            date: (field(years_since_1980) << 9) | (field(local.month()) << 5) | field(local.day_of_month()),
            time: (field(local.hour()) << 11) | (field(local.minute()) << 5) | (field(local.second()) / 2),
        }
    }
}

/// What the central directory records about one entry.
#[derive(Debug)]
struct CentralRecord {
    name: String,
    flags: u16,
    method: u16,
    modified: DosTime,
    crc: u32,
    compressed_size: u32,
    size: u32,
    external_attributes: u32,
    local_header_offset: u32,
}

/// Writes one ZIP archive to `W`, entry by entry.
pub(super) struct ZipWriter<W: Write> {
    output: CountingWriter<W>,
    central: Vec<CentralRecord>,
}

impl<W: Write> ZipWriter<W> {
    /// A writer at the start of an empty archive.
    pub(super) fn new(output: W) -> Self {
        Self {
            output: CountingWriter {
                inner: output,
                written: 0,
            },
            central: Vec::new(),
        }
    }

    /// Adds the empty folder `name`, which ends with `/`.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::TooLargeToCompress`] past the limits of a ZIP
    /// without ZIP64, or the output's error.
    pub(super) fn add_folder(&mut self, name: &str, modified: DosTime) -> Result<(), ArchiveError> {
        let record = CentralRecord {
            name: name.to_owned(),
            flags: FOLDER_FLAGS,
            method: STORED,
            modified,
            crc: 0,
            compressed_size: 0,
            size: 0,
            external_attributes: FOLDER_ATTRIBUTES,
            local_header_offset: self.offset()?,
        };
        self.write_local_header(&record)?;
        self.central.push(record);
        Ok(())
    }

    /// Adds the file `name`, deflating what `content` yields. `progress`
    /// hears the bytes read after each chunk.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::Cancelled`] once `cancel` is cancelled,
    /// [`ArchiveError::TooLargeToCompress`] past the limits of a ZIP
    /// without ZIP64, or the error of reading `content` or writing the
    /// output.
    pub(super) fn add_file(
        &mut self,
        name: &str,
        modified: DosTime,
        content: &mut dyn Read,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), ArchiveError> {
        let mut record = CentralRecord {
            name: name.to_owned(),
            flags: FILE_FLAGS,
            method: DEFLATED,
            modified,
            crc: 0,
            compressed_size: 0,
            size: 0,
            external_attributes: FILE_ATTRIBUTES,
            local_header_offset: self.offset()?,
        };
        self.write_local_header(&record)?;
        let data_start = self.output.written;
        let deflated = self.deflate(content, cancel, progress)?;
        record.crc = deflated.crc;
        record.size = fits_in_u32(deflated.size)?;
        record.compressed_size = fits_in_u32(self.output.written - data_start)?;
        self.write_data_descriptor(&record)?;
        self.central.push(record);
        Ok(())
    }

    /// Writes the central directory and its end record, and returns the
    /// output.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::TooLargeToCompress`] past the limits of a ZIP
    /// without ZIP64, or the output's error.
    pub(super) fn finish(mut self) -> Result<W, ArchiveError> {
        let entry_count = u16::try_from(self.central.len()).map_err(|_| ArchiveError::TooLargeToCompress)?;
        let directory_offset = self.offset()?;
        let records = std::mem::take(&mut self.central);
        for record in &records {
            self.write_central_header(record)?;
        }
        let directory_size = fits_in_u32(u64::from(self.offset()?) - u64::from(directory_offset))?;
        let mut end = Vec::with_capacity(22);
        push_u32(&mut end, END_SIGNATURE);
        push_u16(&mut end, 0);
        push_u16(&mut end, 0);
        push_u16(&mut end, entry_count);
        push_u16(&mut end, entry_count);
        push_u32(&mut end, directory_size);
        push_u32(&mut end, directory_offset);
        push_u16(&mut end, 0);
        self.output.write_all(&end)?;
        self.output.flush()?;
        Ok(self.output.inner)
    }

    /// Where the next byte goes, which must fit a 32-bit offset.
    fn offset(&self) -> Result<u32, ArchiveError> {
        fits_in_u32(self.output.written)
    }

    /// Deflates `content` into the output, returning its CRC and size.
    fn deflate(
        &mut self,
        content: &mut dyn Read,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64),
    ) -> Result<Deflated, ArchiveError> {
        let mut encoder = DeflateEncoder::new(&mut self.output, Compression::default());
        let mut hasher = crc32fast::Hasher::new();
        let mut buffer = vec![0u8; CHUNK_BYTES];
        let mut size = 0u64;
        loop {
            cancel.check()?;
            let count = read_some(content, &mut buffer)?;
            if count == 0 {
                break;
            }
            let chunk = &buffer[..count];
            hasher.update(chunk);
            encoder.write_all(chunk)?;
            size += count as u64;
            progress(size);
        }
        encoder.finish()?;
        Ok(Deflated {
            crc: hasher.finalize(),
            size,
        })
    }

    fn write_local_header(&mut self, record: &CentralRecord) -> Result<(), ArchiveError> {
        let name_length = name_length(&record.name)?;
        let mut header = Vec::with_capacity(30 + record.name.len());
        push_u32(&mut header, LOCAL_HEADER_SIGNATURE);
        push_u16(&mut header, VERSION_NEEDED);
        push_u16(&mut header, record.flags);
        push_u16(&mut header, record.method);
        push_u16(&mut header, record.modified.time);
        push_u16(&mut header, record.modified.date);
        // With a data descriptor, the CRC and sizes here stay 0.
        push_u32(&mut header, 0);
        push_u32(&mut header, 0);
        push_u32(&mut header, 0);
        push_u16(&mut header, name_length);
        push_u16(&mut header, 0);
        header.extend_from_slice(record.name.as_bytes());
        self.output.write_all(&header)?;
        Ok(())
    }

    fn write_data_descriptor(&mut self, record: &CentralRecord) -> Result<(), ArchiveError> {
        let mut descriptor = Vec::with_capacity(16);
        push_u32(&mut descriptor, DATA_DESCRIPTOR_SIGNATURE);
        push_u32(&mut descriptor, record.crc);
        push_u32(&mut descriptor, record.compressed_size);
        push_u32(&mut descriptor, record.size);
        self.output.write_all(&descriptor)?;
        Ok(())
    }

    fn write_central_header(&mut self, record: &CentralRecord) -> Result<(), ArchiveError> {
        let name_length = name_length(&record.name)?;
        let mut header = Vec::with_capacity(46 + record.name.len());
        push_u32(&mut header, CENTRAL_HEADER_SIGNATURE);
        push_u16(&mut header, VERSION_MADE_BY);
        push_u16(&mut header, VERSION_NEEDED);
        push_u16(&mut header, record.flags);
        push_u16(&mut header, record.method);
        push_u16(&mut header, record.modified.time);
        push_u16(&mut header, record.modified.date);
        push_u32(&mut header, record.crc);
        push_u32(&mut header, record.compressed_size);
        push_u32(&mut header, record.size);
        push_u16(&mut header, name_length);
        // No extra field, no comment, disk 0, no internal attributes.
        push_u16(&mut header, 0);
        push_u16(&mut header, 0);
        push_u16(&mut header, 0);
        push_u16(&mut header, 0);
        push_u32(&mut header, record.external_attributes);
        push_u32(&mut header, record.local_header_offset);
        header.extend_from_slice(record.name.as_bytes());
        self.output.write_all(&header)?;
        Ok(())
    }
}

/// A deflated file's CRC-32 and uncompressed size.
#[derive(Debug, Clone, Copy)]
struct Deflated {
    crc: u32,
    size: u64,
}

/// Counts the bytes written through it, for offsets and sizes.
struct CountingWriter<W: Write> {
    inner: W,
    written: u64,
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = self.inner.write(bytes)?;
        self.written += count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Reads into `buffer`, retrying when a read is interrupted.
fn read_some(content: &mut dyn Read, buffer: &mut [u8]) -> Result<usize, ArchiveError> {
    loop {
        match content.read(buffer) {
            Ok(count) => return Ok(count),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
}

/// `value` as a 32-bit field, or [`ArchiveError::TooLargeToCompress`].
fn fits_in_u32(value: u64) -> Result<u32, ArchiveError> {
    u32::try_from(value).map_err(|_| ArchiveError::TooLargeToCompress)
}

/// The length of an entry name, which must fit its 16-bit field.
fn name_length(name: &str) -> Result<u16, ArchiveError> {
    u16::try_from(name.len()).map_err(|_| ArchiveError::TooLargeToCompress)
}

fn push_u16(buffer: &mut Vec<u8>, value: u16) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(buffer: &mut Vec<u8>, value: u32) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::archive::zip::ZipArchive;

    /// parity: ARC-023
    #[test]
    fn a_written_archive_reads_back_with_its_names_sizes_and_data() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let cancel = Cancellation::new();
        let data = b"Synthetic test data\n".repeat(100);

        writer.add_folder("Docs/", DosTime::EARLIEST).unwrap();
        writer
            .add_file(
                "Docs/notes é.txt",
                DosTime::EARLIEST,
                &mut data.as_slice(),
                &cancel,
                &mut |_| {},
            )
            .unwrap();
        let bytes = writer.finish().unwrap().into_inner();

        let mut archive = ZipArchive::open(Cursor::new(bytes), &cancel).unwrap();
        let names: Vec<&str> = archive
            .members()
            .iter()
            .map(|member| member.name.as_str())
            .collect();
        assert_eq!(names, ["Docs/", "Docs/notes é.txt"]);
        assert_eq!(archive.members()[1].size, data.len() as u64);
        let mut member = archive.open_member(1).unwrap();
        let mut read_back = Vec::new();
        let mut chunk = vec![0u8; 4096];
        loop {
            let count = member.read_chunk(&mut chunk).unwrap();
            if count == 0 {
                break;
            }
            read_back.extend_from_slice(&chunk[..count]);
        }
        assert_eq!(read_back, data);
    }

    #[test]
    fn times_a_zip_cannot_store_become_1980() {
        assert_eq!(DosTime::from_unix_seconds(Some(0)), DosTime::EARLIEST);
        assert_eq!(DosTime::from_unix_seconds(None), DosTime::EARLIEST);
    }
}
