// SPDX-License-Identifier: AGPL-3.0-only
//! Writing the ZIP format: local headers, deflated data with data
//! descriptors, the central directory and its end record.
//!
//! The layout is the one of PKWARE's APPNOTE 6.3, as Python's `zipfile`
//! and every archive manager read it. File names are UTF-8 (flag bit 11),
//! files are deflated and followed by a data descriptor (flag bit 3), so
//! each file is read once, and folders are stored empty. ZIP64 records
//! (APPNOTE 4.3.14 to 4.3.16 and 4.5.3) are written only where a value
//! needs them, as Python's `zipfile` writes them: a file of 4 GiB or more
//! gets the ZIP64 extra field in its local header and 8-byte sizes in its
//! data descriptor, a size or offset past 4 GiB goes into the central
//! directory's ZIP64 extra field, and an archive with 65,535 entries or
//! more, or a directory past 4 GiB, ends with the ZIP64 end records.

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
/// `PK\x06\x06`: the ZIP64 end of the central directory.
const ZIP64_END_SIGNATURE: u32 = 0x0606_4b50;
/// `PK\x06\x07`: the locator of the ZIP64 end record.
const ZIP64_LOCATOR_SIGNATURE: u32 = 0x0706_4b50;
/// The header id of the ZIP64 extended information extra field.
const ZIP64_EXTRA_ID: u16 = 0x0001;
/// Version 2.0: deflate and folders.
const VERSION_NEEDED: u16 = 20;
/// Version 4.5: ZIP64.
const ZIP64_VERSION_NEEDED: u16 = 45;
/// A 32-bit field holding this value is found in the ZIP64 extra field.
const MARKER_32: u32 = u32::MAX;
/// An entry count holding this value is found in the ZIP64 end record.
const MARKER_16: u16 = u16::MAX;
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
    compressed_size: u64,
    size: u64,
    external_attributes: u32,
    local_header_offset: u64,
}

/// Writes one ZIP archive to `W`, entry by entry.
pub(super) struct ZipWriter<W: Write> {
    output: CountingWriter<W>,
    central: Vec<CentralRecord>,
    /// The deflate level, 1 (faster) to 9 (smaller).
    level: Compression,
    /// Sizes and offsets from this value on are written as ZIP64 values:
    /// `0xFFFF_FFFF`, the largest 32-bit field being the ZIP64 marker.
    zip64_threshold: u64,
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
            level: Compression::default(),
            zip64_threshold: u64::from(MARKER_32),
        }
    }

    /// The same writer, writing ZIP64 values from `threshold` bytes on,
    /// so tests can check archives past 4 GiB with a few bytes.
    #[cfg(test)]
    fn with_zip64_threshold(mut self, threshold: u64) -> Self {
        self.zip64_threshold = threshold;
        self
    }

    /// True when `value` must be written as a ZIP64 value.
    fn needs_zip64(&self, value: u64) -> bool {
        value >= self.zip64_threshold
    }

    /// The same writer deflating at `level`, 1 (faster) to 9 (smaller).
    pub(super) fn with_level(mut self, level: u32) -> Self {
        self.level = Compression::new(level.clamp(1, 9));
        self
    }

    /// Adds the empty folder `name`, which ends with `/`.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::TooLargeToCompress`] for a name over 65,535 bytes,
    /// or the output's error.
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
            local_header_offset: self.output.written,
        };
        self.write_local_header(&record, false)?;
        self.central.push(record);
        Ok(())
    }

    /// Adds the file `name`, deflating what `content` yields; `expected_size`
    /// is its size when planned, which decides whether its local header
    /// announces ZIP64 sizes. `progress` hears the bytes read after each
    /// chunk.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::Cancelled`] once `cancel` is cancelled,
    /// [`ArchiveError::TooLargeToCompress`] for a name over 65,535 bytes or
    /// a file that grew past 4 GiB after it was planned smaller, or the
    /// error of reading `content` or writing the output.
    pub(super) fn add_file(
        &mut self,
        name: &str,
        (modified, expected_size): (DosTime, u64),
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
            local_header_offset: self.output.written,
        };
        // Deflate can make incompressible data a little larger.
        let is_zip64 = self.needs_zip64(expected_size.saturating_add(expected_size / 64 + 1024));
        self.write_local_header(&record, is_zip64)?;
        let data_start = self.output.written;
        let deflated = self.deflate(content, cancel, progress)?;
        record.crc = deflated.crc;
        record.size = deflated.size;
        record.compressed_size = self.output.written - data_start;
        let grew_past_4_gib = self.needs_zip64(record.size) || self.needs_zip64(record.compressed_size);
        if grew_past_4_gib && !is_zip64 {
            return Err(ArchiveError::TooLargeToCompress);
        }
        self.write_data_descriptor(&record, is_zip64)?;
        self.central.push(record);
        Ok(())
    }

    /// Writes the central directory and its end records, and returns the
    /// output.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::TooLargeToCompress`] for a name over 65,535 bytes,
    /// or the output's error.
    pub(super) fn finish(mut self) -> Result<W, ArchiveError> {
        let entry_count = self.central.len() as u64;
        let directory_offset = self.output.written;
        let records = std::mem::take(&mut self.central);
        for record in &records {
            self.write_central_header(record)?;
        }
        let directory_size = self.output.written - directory_offset;
        let needs_zip64_end = entry_count >= u64::from(MARKER_16)
            || self.needs_zip64(directory_offset)
            || self.needs_zip64(directory_size);
        if needs_zip64_end {
            self.write_zip64_end(entry_count, directory_size, directory_offset)?;
        }
        let count_field = u16::try_from(entry_count)
            .ok()
            .filter(|_| !needs_zip64_end)
            .unwrap_or(MARKER_16);
        let mut end = Vec::with_capacity(22);
        push_u32(&mut end, END_SIGNATURE);
        push_u16(&mut end, 0);
        push_u16(&mut end, 0);
        push_u16(&mut end, count_field);
        push_u16(&mut end, count_field);
        push_u32(&mut end, self.field_32(directory_size));
        push_u32(&mut end, self.field_32(directory_offset));
        push_u16(&mut end, 0);
        self.output.write_all(&end)?;
        self.output.flush()?;
        Ok(self.output.inner)
    }

    /// The ZIP64 end of central directory record and its locator.
    fn write_zip64_end(
        &mut self,
        entry_count: u64,
        directory_size: u64,
        directory_offset: u64,
    ) -> Result<(), ArchiveError> {
        let record_offset = self.output.written;
        let mut records = Vec::with_capacity(76);
        push_u32(&mut records, ZIP64_END_SIGNATURE);
        // The record's size without its signature and this field.
        push_u64(&mut records, 44);
        push_u16(&mut records, VERSION_MADE_BY);
        push_u16(&mut records, ZIP64_VERSION_NEEDED);
        push_u32(&mut records, 0);
        push_u32(&mut records, 0);
        push_u64(&mut records, entry_count);
        push_u64(&mut records, entry_count);
        push_u64(&mut records, directory_size);
        push_u64(&mut records, directory_offset);
        push_u32(&mut records, ZIP64_LOCATOR_SIGNATURE);
        push_u32(&mut records, 0);
        push_u64(&mut records, record_offset);
        push_u32(&mut records, 1);
        self.output.write_all(&records)?;
        Ok(())
    }

    /// `value` as a 32-bit field: itself, or the ZIP64 marker when the
    /// ZIP64 records hold it.
    fn field_32(&self, value: u64) -> u32 {
        if self.needs_zip64(value) {
            MARKER_32
        } else {
            u32::try_from(value).unwrap_or(MARKER_32)
        }
    }

    /// Deflates `content` into the output, returning its CRC and size.
    fn deflate(
        &mut self,
        content: &mut dyn Read,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64),
    ) -> Result<Deflated, ArchiveError> {
        let mut encoder = DeflateEncoder::new(&mut self.output, self.level);
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

    /// Writes the local header; `is_zip64` announces 8-byte sizes in the
    /// data descriptor with a ZIP64 extra field whose sizes, like the
    /// header's, follow in the descriptor.
    fn write_local_header(&mut self, record: &CentralRecord, is_zip64: bool) -> Result<(), ArchiveError> {
        let name_length = name_length(&record.name)?;
        let mut header = Vec::with_capacity(50 + record.name.len());
        push_u32(&mut header, LOCAL_HEADER_SIGNATURE);
        push_u16(
            &mut header,
            if is_zip64 {
                ZIP64_VERSION_NEEDED
            } else {
                VERSION_NEEDED
            },
        );
        push_u16(&mut header, record.flags);
        push_u16(&mut header, record.method);
        push_u16(&mut header, record.modified.time);
        push_u16(&mut header, record.modified.date);
        // With a data descriptor, the CRC and sizes here stay 0; ZIP64
        // marks the sizes as found in its field.
        let size_field = if is_zip64 { MARKER_32 } else { 0 };
        push_u32(&mut header, 0);
        push_u32(&mut header, size_field);
        push_u32(&mut header, size_field);
        push_u16(&mut header, name_length);
        push_u16(&mut header, if is_zip64 { 20 } else { 0 });
        header.extend_from_slice(record.name.as_bytes());
        if is_zip64 {
            push_u16(&mut header, ZIP64_EXTRA_ID);
            push_u16(&mut header, 16);
            push_u64(&mut header, 0);
            push_u64(&mut header, 0);
        }
        self.output.write_all(&header)?;
        Ok(())
    }

    /// Writes the CRC and sizes after the data, 8-byte sizes for ZIP64.
    fn write_data_descriptor(&mut self, record: &CentralRecord, is_zip64: bool) -> Result<(), ArchiveError> {
        let mut descriptor = Vec::with_capacity(24);
        push_u32(&mut descriptor, DATA_DESCRIPTOR_SIGNATURE);
        push_u32(&mut descriptor, record.crc);
        if is_zip64 {
            push_u64(&mut descriptor, record.compressed_size);
            push_u64(&mut descriptor, record.size);
        } else {
            push_u32(&mut descriptor, fits_in_u32(record.compressed_size)?);
            push_u32(&mut descriptor, fits_in_u32(record.size)?);
        }
        self.output.write_all(&descriptor)?;
        Ok(())
    }

    /// Writes the central directory header; a size or offset past the
    /// ZIP64 threshold goes into a ZIP64 extra field, in the order the
    /// specification gives, behind a marker in its 32-bit field.
    fn write_central_header(&mut self, record: &CentralRecord) -> Result<(), ArchiveError> {
        let name_length = name_length(&record.name)?;
        let mut extra = Vec::new();
        for value in [record.size, record.compressed_size, record.local_header_offset] {
            if self.needs_zip64(value) {
                push_u64(&mut extra, value);
            }
        }
        let mut zip64_extra = Vec::with_capacity(4 + extra.len());
        if !extra.is_empty() {
            push_u16(&mut zip64_extra, ZIP64_EXTRA_ID);
            push_u16(&mut zip64_extra, u16::try_from(extra.len()).unwrap_or(24));
            zip64_extra.extend_from_slice(&extra);
        }
        let version_needed = if zip64_extra.is_empty() {
            VERSION_NEEDED
        } else {
            ZIP64_VERSION_NEEDED
        };
        let mut header = Vec::with_capacity(46 + record.name.len() + zip64_extra.len());
        push_u32(&mut header, CENTRAL_HEADER_SIGNATURE);
        push_u16(&mut header, VERSION_MADE_BY);
        push_u16(&mut header, version_needed);
        push_u16(&mut header, record.flags);
        push_u16(&mut header, record.method);
        push_u16(&mut header, record.modified.time);
        push_u16(&mut header, record.modified.date);
        push_u32(&mut header, record.crc);
        push_u32(&mut header, self.field_32(record.compressed_size));
        push_u32(&mut header, self.field_32(record.size));
        push_u16(&mut header, name_length);
        push_u16(&mut header, u16::try_from(zip64_extra.len()).unwrap_or(28));
        // No comment, disk 0, no internal attributes.
        push_u16(&mut header, 0);
        push_u16(&mut header, 0);
        push_u16(&mut header, 0);
        push_u32(&mut header, record.external_attributes);
        push_u32(&mut header, self.field_32(record.local_header_offset));
        header.extend_from_slice(record.name.as_bytes());
        header.extend_from_slice(&zip64_extra);
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

fn push_u64(buffer: &mut Vec<u8>, value: u64) {
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
                (DosTime::EARLIEST, data.len() as u64),
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
        let mut member = archive.open_member(1, None).unwrap();
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

    /// Reads back every member of `bytes` with the app's reader and
    /// returns each name, size and data.
    fn read_back(bytes: Vec<u8>) -> Vec<(String, u64, Vec<u8>)> {
        let cancel = Cancellation::new();
        let mut archive = ZipArchive::open(Cursor::new(bytes), &cancel).unwrap();
        let mut members = Vec::new();
        for index in 0..archive.members().len() {
            let (name, size) = {
                let member = &archive.members()[index];
                (member.name.clone(), member.size)
            };
            let mut reader = archive.open_member(index, None).unwrap();
            let mut data = Vec::new();
            let mut chunk = vec![0u8; 4096];
            loop {
                let count = reader.read_chunk(&mut chunk).unwrap();
                if count == 0 {
                    break;
                }
                data.extend_from_slice(&chunk[..count]);
            }
            members.push((name, size, data));
        }
        members
    }

    /// With the ZIP64 threshold lowered to a few bytes, every size, offset
    /// and the directory are written as ZIP64 values, and the archive
    /// reads back with the app's reader and with Python's `zipfile`, as an
    /// archive over 4 GiB would.
    ///
    /// parity: ARC-023
    #[test]
    fn sizes_and_offsets_past_the_threshold_are_written_as_zip64() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new())).with_zip64_threshold(16);
        let cancel = Cancellation::new();
        let data = b"Tidewater soundings\n".repeat(50);
        writer.add_folder("Charts/", DosTime::EARLIEST).unwrap();
        for name in ["Charts/north.txt", "Charts/south.txt", "east.txt"] {
            writer
                .add_file(
                    name,
                    (DosTime::EARLIEST, data.len() as u64),
                    &mut data.as_slice(),
                    &cancel,
                    &mut |_| {},
                )
                .unwrap();
        }
        let bytes = writer.finish().unwrap().into_inner();

        assert!(
            bytes.windows(4).any(|window| window == [0x50, 0x4b, 0x06, 0x06]),
            "a ZIP64 end record"
        );
        let members = read_back(bytes.clone());
        let names: Vec<&str> = members.iter().map(|(name, _, _)| name.as_str()).collect();
        assert_eq!(
            names,
            ["Charts/", "Charts/north.txt", "Charts/south.txt", "east.txt"]
        );
        for (_, size, read) in &members[1..] {
            assert_eq!((*size, read), (data.len() as u64, &data));
        }
        let path = std::env::temp_dir().join(format!("zip64-{}.zip", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let checked = std::process::Command::new("python3")
            .args([
                "-I",
                "-c",
                "import sys, zipfile; z = zipfile.ZipFile(sys.argv[1]); assert z.testzip() is None; \
                 assert all(i.file_size == 1000 for i in z.infolist()[1:]); print(len(z.infolist()))",
            ])
            .arg(&path)
            .output()
            .unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(
            checked.status.success(),
            "{}",
            String::from_utf8_lossy(&checked.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&checked.stdout).trim(), "4");
    }

    #[test]
    fn times_a_zip_cannot_store_become_1980() {
        assert_eq!(DosTime::from_unix_seconds(Some(0)), DosTime::EARLIEST);
        assert_eq!(DosTime::from_unix_seconds(None), DosTime::EARLIEST);
    }
}
