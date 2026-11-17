// SPDX-License-Identifier: AGPL-3.0-only
//! 7z archives for the tests: real ones written by the `sevenz-rust2`
//! writer the app uses for Compress to › 7z File, and hand-made ones whose
//! headers say exactly what a test needs (an encrypted coder, a forged
//! count), built from the records of 7-Zip's `7zFormat.txt`.

use std::io::Cursor;

use sevenz_rust2::encoder_options::Lzma2Options;
use sevenz_rust2::{ArchiveEntry, ArchiveWriter, NtTime, SourceReader};

/// The attribute 7-Zip and p7zip set when the high 16 bits hold the
/// item's Unix mode.
const UNIX_EXTENSION: u32 = 0x8000;
/// 2026-03-04 05:06:08 UTC, in seconds since the Unix epoch.
pub const SEVEN_ZIP_MODIFIED: u64 = 1_772_600_768;

/// One item of a test 7z.
#[derive(Debug, Clone)]
pub struct SevenEntry {
    /// The name as stored.
    pub name: String,
    /// The data of a file; `None` for a folder.
    pub data: Option<Vec<u8>>,
    /// The Unix mode recorded beside the Windows attributes, if any.
    pub unix_mode: Option<u32>,
}

impl SevenEntry {
    /// A file `name` holding `data`.
    pub fn file(name: &str, data: &[u8]) -> Self {
        Self {
            name: name.to_owned(),
            data: Some(data.to_vec()),
            unix_mode: None,
        }
    }

    /// A folder `name`.
    pub fn folder(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            data: None,
            unix_mode: None,
        }
    }

    /// A symbolic link `name` to `target`, as p7zip stores one: a file
    /// whose data is the target and whose Unix mode says it is a link.
    pub fn link(name: &str, target: &str) -> Self {
        Self {
            name: name.to_owned(),
            data: Some(target.as_bytes().to_vec()),
            unix_mode: Some(0o120_777),
        }
    }

    fn archive_entry(&self) -> ArchiveEntry {
        let mut entry = if self.data.is_some() {
            ArchiveEntry::new_file(&self.name)
        } else {
            ArchiveEntry::new_directory(&self.name)
        };
        entry.has_last_modified_date = true;
        entry.last_modified_date = NtTime::from(SEVEN_ZIP_MODIFIED * 10_000_000 + 116_444_736_000_000_000);
        if let Some(mode) = self.unix_mode {
            entry.has_windows_attributes = true;
            entry.windows_attributes = (mode << 16) | UNIX_EXTENSION;
        }
        entry
    }
}

/// A 7z of `entries`, each file compressed with LZMA2 on its own, or with
/// `solid` every file in one LZMA2 stream, as 7-Zip writes by default.
///
/// # Panics
///
/// When the writer fails.
pub fn seven_zip_bytes(entries: &[SevenEntry], solid: bool) -> Vec<u8> {
    let mut writer = ArchiveWriter::new(Cursor::new(Vec::new())).expect("a 7z writer");
    writer.set_content_methods(vec![Lzma2Options::from_level(1).into()]);
    writer.set_encrypt_header(false);
    if solid {
        let (folders, files): (Vec<&SevenEntry>, Vec<&SevenEntry>) =
            entries.iter().partition(|entry| entry.data.is_none());
        for folder in folders {
            writer
                .push_archive_entry::<&[u8]>(folder.archive_entry(), None)
                .expect("a folder");
        }
        let archive_entries = files.iter().map(|entry| entry.archive_entry()).collect();
        let readers = files
            .iter()
            .map(|entry| SourceReader::new(Cursor::new(entry.data.clone().unwrap_or_default())))
            .collect();
        writer
            .push_archive_entries(archive_entries, readers)
            .expect("the files");
    } else {
        for entry in entries {
            let data = entry.data.as_deref();
            writer
                .push_archive_entry(entry.archive_entry(), data)
                .expect("an entry");
        }
    }
    writer.finish().expect("finished").into_inner()
}

/// A 7z number (`REAL_UINT64` of `7zFormat.txt`).
pub fn number(value: u64) -> Vec<u8> {
    if value < 0x80 {
        return vec![u8::try_from(value).expect("small")];
    }
    let mut bytes = vec![0xff];
    bytes.extend_from_slice(&value.to_le_bytes());
    bytes
}

/// A 7z file around `packed` data and the header `header` (starting with
/// `kHeader` or `kEncodedHeader`), with a correct start header.
pub fn raw_seven_zip(packed: &[u8], header: &[u8]) -> Vec<u8> {
    let mut start = Vec::new();
    start.extend_from_slice(&(packed.len() as u64).to_le_bytes());
    start.extend_from_slice(&(header.len() as u64).to_le_bytes());
    start.extend_from_slice(&crc32fast::hash(header).to_le_bytes());
    let mut bytes = b"7z\xBC\xAF\x27\x1C\x00\x04".to_vec();
    bytes.extend_from_slice(&crc32fast::hash(&start).to_le_bytes());
    bytes.extend_from_slice(&start);
    bytes.extend_from_slice(packed);
    bytes.extend_from_slice(header);
    bytes
}

/// The `kName` property for `names`.
fn names_property(names: &[&str]) -> Vec<u8> {
    let mut text = vec![0u8];
    for name in names {
        for unit in name.encode_utf16().chain([0]) {
            text.extend_from_slice(&unit.to_le_bytes());
        }
    }
    let mut property = vec![0x11];
    property.extend(number(text.len() as u64));
    property.extend(text);
    property
}

/// The streams info of one block of `packed_size` bytes unpacking to
/// `unpack_size` through one coder `method` with `properties`.
fn one_block_streams(method: &[u8], properties: &[u8], packed_size: u64, unpack_size: u64) -> Vec<u8> {
    let mut info = vec![0x06]; // kPackInfo
    info.extend(number(0));
    info.extend(number(1));
    info.push(0x09); // kSize
    info.extend(number(packed_size));
    info.push(0x00);
    info.extend([0x07, 0x0b]); // kUnpackInfo, kFolder
    info.extend(number(1));
    info.push(0x00); // not external
    info.extend(number(1)); // one coder
    let flags = u8::try_from(method.len()).expect("short id") | if properties.is_empty() { 0 } else { 0x20 };
    info.push(flags);
    info.extend_from_slice(method);
    if !properties.is_empty() {
        info.extend(number(properties.len() as u64));
        info.extend_from_slice(properties);
    }
    info.push(0x0c); // kCodersUnpackSize
    info.extend(number(unpack_size));
    info.push(0x00);
    info.extend([0x08, 0x00]); // kSubStreamsInfo: one stream
    info.push(0x00);
    info
}

/// A 7z holding one file `name` of `data.len()` bytes stored in one block
/// coded with `method` (such as AES `06F10701`) and `properties`; the
/// data is `data` as it is, so only the headers are real.
pub fn one_file_seven_zip(name: &str, method: &[u8], properties: &[u8], data: &[u8]) -> Vec<u8> {
    let size = data.len() as u64;
    let mut header = vec![0x01, 0x04]; // kHeader, kMainStreamsInfo
    header.extend(one_block_streams(method, properties, size, size));
    header.push(0x05); // kFilesInfo
    header.extend(number(1));
    header.extend(names_property(&[name]));
    header.extend([0x00, 0x00]);
    raw_seven_zip(data, &header)
}

/// A 7z whose header is "encoded": packed with `method` (AES here, so the
/// names cannot be read without the password).
pub fn encoded_header_seven_zip(method: &[u8], properties: &[u8]) -> Vec<u8> {
    let packed = vec![0x5a; 32];
    let mut header = vec![0x17]; // kEncodedHeader
    header.extend(one_block_streams(method, properties, 32, 32));
    raw_seven_zip(&packed, &header)
}

/// A 7z whose files record claims `count` files with no names.
pub fn seven_zip_claiming_files(count: u64) -> Vec<u8> {
    let mut header = vec![0x01, 0x05];
    header.extend(number(count));
    header.extend([0x00, 0x00]);
    raw_seven_zip(&[], &header)
}

/// One coder of a hand-made block: its method id, how many streams it
/// reads and its properties.
pub struct TestCoder<'a> {
    pub method: &'a [u8],
    pub inputs: u64,
    pub properties: &'a [u8],
}

/// A 7z holding one file `name` of `unpack_size` bytes, whose one block
/// has `coders` linked by `bind_pairs` (input stream, coder output) and
/// reads `packed` input streams from the `streams` given, in order.
pub fn coded_seven_zip(
    name: &str,
    coders: &[TestCoder<'_>],
    bind_pairs: &[(u64, u64)],
    packed: &[u64],
    streams: &[&[u8]],
    unpack_sizes: &[u64],
) -> Vec<u8> {
    let mut header = vec![0x01, 0x04, 0x06];
    header.extend(number(0));
    header.extend(number(streams.len() as u64));
    header.push(0x09);
    for stream in streams {
        header.extend(number(stream.len() as u64));
    }
    header.push(0x00);
    header.extend([0x07, 0x0b]);
    header.extend(number(1));
    header.push(0x00);
    header.extend(number(coders.len() as u64));
    for coder in coders {
        let mut flags = u8::try_from(coder.method.len()).expect("short id");
        if coder.inputs != 1 {
            flags |= 0x10;
        }
        if !coder.properties.is_empty() {
            flags |= 0x20;
        }
        header.push(flags);
        header.extend_from_slice(coder.method);
        if coder.inputs != 1 {
            header.extend(number(coder.inputs));
            header.extend(number(1));
        }
        if !coder.properties.is_empty() {
            header.extend(number(coder.properties.len() as u64));
            header.extend_from_slice(coder.properties);
        }
    }
    for (input, output) in bind_pairs {
        header.extend(number(*input));
        header.extend(number(*output));
    }
    if packed.len() > 1 {
        for input in packed {
            header.extend(number(*input));
        }
    }
    header.push(0x0c);
    for size in unpack_sizes {
        header.extend(number(*size));
    }
    header.push(0x00);
    header.extend([0x08, 0x00, 0x00]);
    header.push(0x05);
    header.extend(number(1));
    header.extend(names_property(&[name]));
    header.extend([0x00, 0x00]);
    let data: Vec<u8> = streams.concat();
    raw_seven_zip(&data, &header)
}

/// The BCJ2 method id.
pub const BCJ2: [u8; 4] = [0x03, 0x03, 0x01, 0x1b];
/// The `BZip2` method id.
pub const BZIP2: [u8; 3] = [0x04, 0x02, 0x02];
/// The Deflate method id.
pub const DEFLATE: [u8; 3] = [0x04, 0x01, 0x08];
/// The Copy method id.
pub const COPY: [u8; 1] = [0x00];

/// The LZMA2 method id and a dictionary property of `property` (40 is
/// 4 GiB).
pub const LZMA2: [u8; 1] = [0x21];
/// The AES-256 + SHA-256 method id.
pub const AES: [u8; 4] = [0x06, 0xf1, 0x07, 0x01];
/// The `PPMd` method id.
pub const PPMD: [u8; 3] = [0x03, 0x04, 0x01];
