// SPDX-License-Identifier: AGPL-3.0-only
//! The archive readers as the archive browser, Properties and Extract All
//! use them, on an archive held in memory.
//!
//! The first byte of the input picks the name the archive is opened under
//! (the readers look at the extension) and which options are used; the
//! rest is the archive.

use std::fs;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};

use gio::prelude::FileExt;

use crate::archive::{
    lift_single_folder, remove_preview_copy, ArchiveBrowser, ArchiveEntryKind, ArchiveError, ArchiveOpener,
    ArchivePassword, ArchiveStream, ExtractionLimits, ExtractionRequest, GioExtractionOutput, ZipExtractor,
};
use crate::gio_node::GioNode;
use crate::transfer::{Cancellation, Node, NodeFactory};

/// The names an archive is opened under, picked by the low three bits of
/// the first byte.
const NAMES: [&str; 8] = [
    "Fuzz.zip",
    "Fuzz.7z",
    "Fuzz.tar",
    "Fuzz.tar.gz",
    "Fuzz.tar.bz2",
    "Fuzz.tar.xz",
    "Fuzz.tar.zst",
    "Fuzz.bin",
];

/// The folders listed below the top, and the members previewed, at most.
const MAX_VISITS: usize = 6;

/// The options an input asks for, each a bit of its first byte.
#[allow(clippy::struct_excessive_bools)]
struct Input<'a> {
    uri: String,
    archive: &'a [u8],
    /// Decrypt with a password.
    password: bool,
    /// Extract only the members the browser lists.
    selection: bool,
    /// Extract as the private step of an extraction into an existing folder.
    into_existing: bool,
    /// Lift a lone folder out of the new one, as Extract here does.
    lift: bool,
}

fn input(data: &[u8]) -> Option<Input<'_>> {
    let (&options, archive) = data.split_first()?;
    // Each input is a file of its own: the 7z reader keeps the headers of
    // the archives it read last by their location, length and start.
    let file = crc32fast::hash(archive);
    Some(Input {
        uri: format!(
            "file:///home/alex/Downloads/{file:08x}/{}",
            NAMES[usize::from(options & 7)]
        ),
        archive,
        password: options & 0x08 != 0,
        selection: options & 0x10 != 0,
        into_existing: options & 0x20 != 0,
        lift: options & 0x40 != 0,
    })
}

/// Opens every URI as `archive`.
fn memory_opener(archive: &[u8]) -> Arc<dyn ArchiveOpener> {
    let archive = archive.to_vec();
    let opener = move |_uri: &str, cancel: &Cancellation| -> Result<Box<dyn ArchiveStream>, ArchiveError> {
        cancel.check()?;
        Ok(Box::new(Cursor::new(archive.clone())))
    };
    Arc::new(opener)
}

fn gio_factory() -> NodeFactory {
    Arc::new(|uri: &str| Ok(Box::new(GioNode::new(uri)) as Box<dyn Node>))
}

/// A private folder for this process's copies and extractions, removed
/// with the process's temporary folder.
fn scratch() -> &'static Path {
    static SCRATCH: OnceLock<tempfile::TempDir> = OnceLock::new();
    SCRATCH
        .get_or_init(|| {
            tempfile::Builder::new()
                .prefix("ox-fuzz-")
                .tempdir()
                .expect("a temporary folder")
        })
        .path()
}

/// Lower limits than the app's, so a fuzzed archive that is within every
/// limit still extracts in well under a second.
fn fuzz_limits() -> ExtractionLimits {
    ExtractionLimits {
        max_entries: 2000,
        max_paths: 4000,
        max_depth: 64,
        max_total_bytes: 16 * 1024 * 1024,
        max_member_bytes: 8 * 1024 * 1024,
        max_ratio: 1000,
    }
}

fn browser(input: &Input<'_>) -> ArchiveBrowser {
    let browser = ArchiveBrowser::new(memory_opener(input.archive), scratch().join("previews"));
    if input.password {
        browser.with_password(ArchivePassword::from("lighthouse"))
    } else {
        browser
    }
}

/// Lists the top of the archive and a few of its folders, previews a few
/// members, and checks the whole archive as Extract All does first.
pub fn list_inspect_preview(data: &[u8]) {
    let Some(input) = input(data) else {
        return;
    };
    let cancel = Cancellation::new();
    let browser = browser(&input);
    let mut folders = vec![String::new()];
    let mut visited = 0;
    let mut previewed = 0;
    while let Some(prefix) = folders.pop() {
        visited += 1;
        let Ok(listing) = browser.list(&input.uri, &prefix, &cancel) else {
            continue;
        };
        for entry in &listing.entries {
            match entry.kind {
                ArchiveEntryKind::Folder if visited + folders.len() < MAX_VISITS => {
                    folders.push(entry.member.clone());
                }
                ArchiveEntryKind::File { size, .. } if entry.can_open && previewed < MAX_VISITS => {
                    previewed += 1;
                    if let Ok(copy) = browser.preview_member(&input.uri, &entry.member, &cancel) {
                        check_preview(&copy.path, size);
                        remove_preview_copy(&copy.path);
                    }
                }
                _ => {}
            }
        }
    }
    let extractor = ZipExtractor::new(
        memory_opener(input.archive),
        gio_factory(),
        Arc::new(GioExtractionOutput),
    )
    .with_limits(fuzz_limits());
    let _ = extractor.inspect(&input.uri, &cancel);
}

/// Checks that a member's copy is a plain file in a folder of its own in
/// the preview folder, as long as the listing said.
fn check_preview(path: &Path, size: u64) {
    let previews = scratch().join("previews");
    let folder = path.parent().expect("a copy's folder");
    assert_eq!(
        folder.parent(),
        Some(previews.as_path()),
        "a copy at {}",
        path.display()
    );
    let metadata = fs::symlink_metadata(path).expect("the copy");
    assert!(
        metadata.is_file(),
        "a copy that is no plain file: {}",
        path.display()
    );
    assert_eq!(
        metadata.len(),
        size,
        "the copy of {} is not as long as listed",
        path.display()
    );
}

/// Extracts the archive into a new folder in an empty folder, as Extract
/// All does, then checks that nothing was written outside the new folder
/// and that every link it holds stays inside it.
///
/// # Panics
///
/// When the extraction wrote outside its new folder or extracted a link
/// leading out of it.
pub fn extract(data: &[u8]) {
    let Some(input) = input(data) else {
        return;
    };
    let cancel = Cancellation::new();
    let root = tempfile::Builder::new()
        .prefix("extract-")
        .tempdir_in(scratch())
        .expect("a temporary folder");
    let destination = root.path().join("Destination");
    fs::create_dir(&destination).expect("the destination folder");
    let mut extractor = ZipExtractor::new(
        memory_opener(input.archive),
        gio_factory(),
        Arc::new(GioExtractionOutput),
    )
    .with_limits(fuzz_limits());
    if input.password {
        extractor = extractor.with_password(ArchivePassword::from("lighthouse"));
    }
    if input.selection {
        let listed = browser(&input)
            .list(&input.uri, "", &cancel)
            .map(|listing| {
                listing
                    .entries
                    .into_iter()
                    .map(|entry| entry.member)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        extractor = extractor.with_selection(&listed);
    }
    if input.into_existing {
        extractor = extractor.into_existing_folder();
    }
    let request = ExtractionRequest {
        archive_uri: input.uri.clone(),
        destination_uri: gio::File::for_path(&destination).uri().to_string(),
        folder_name: "Extracted".to_owned(),
    };
    let result = extractor.extract(&request, &cancel);
    let new_folder = destination.join("Extracted");
    check_only_child(root.path(), "Destination");
    match result {
        Ok(extracted) => {
            check_only_child(&destination, "Extracted");
            check_contained(&new_folder, &new_folder);
            // Extract here then lifts a lone folder out of the new one.
            if input.lift {
                let lifted = lift_single_folder(extracted);
                let output = gio::File::for_uri(&lifted.uri).path().expect("a local output");
                assert_eq!(
                    output.parent(),
                    Some(destination.as_path()),
                    "lifted to {}",
                    output.display()
                );
                let left = names(&destination);
                assert_eq!(left.len(), 1, "lifting left {left:?}");
                check_contained(&output, &output);
            }
        }
        Err(ArchiveError::StagingLeftBehind { .. }) => {}
        Err(_) => {
            let left = names(&destination);
            assert!(left.is_empty(), "a failed extraction left {left:?} behind");
        }
    }
}

/// Reads the bytes of a generator input in turn; zeros once they run out.
struct Choices<'a>(&'a [u8]);

impl Choices<'_> {
    fn byte(&mut self) -> u8 {
        let (&first, rest) = self.0.split_first().unwrap_or((&0, &[]));
        self.0 = rest;
        first
    }

    fn bytes(&mut self, most: u8) -> Vec<u8> {
        let count = usize::from(self.byte() % most.max(1));
        let (taken, rest) = self.0.split_at(count.min(self.0.len()));
        self.0 = rest;
        taken.to_vec()
    }
}

/// A ZIP written from `data`, every offset, length and checksum right but
/// what the choices ask to be wrong: names that leave the folder, links,
/// special files, Unicode path fields whose checksum matches the recorded
/// name, ZIP64 fields, data descriptors, encryption flags, a local name
/// that differs and members claiming methods their data is not. It is then
/// listed, previewed and extracted as [`list_inspect_preview`] and
/// [`extract`] do.
pub fn zip_generated(data: &[u8]) {
    let archive = generated_zip(&mut Choices(data));
    let mut input = vec![archive.0 & !7];
    input.extend_from_slice(&archive.1);
    list_inspect_preview(&input);
    extract(&input);
}

/// The options byte and the ZIP the choices describe.
// Each member's records are written in the order they lie in the file;
// split up, the layout would be harder to follow.
#[allow(clippy::too_many_lines)]
fn generated_zip(choices: &mut Choices<'_>) -> (u8, Vec<u8>) {
    use std::io::Write;
    const NAMES: [&str; 8] = [
        "Docs/",
        "Docs/a.txt",
        "../up.txt",
        "/root.txt",
        "b\\c.txt",
        "link",
        "Docs/..",
        "x/./y",
    ];
    let options = choices.byte();
    let count = choices.byte() % 8 + 1;
    let mut zip = Vec::new();
    let mut directory = Vec::new();
    for _ in 0..count {
        let flags = choices.byte();
        let more = choices.byte();
        let name = if flags & 1 == 0 {
            NAMES[usize::from(more % 8)].as_bytes().to_vec()
        } else {
            choices.bytes(48)
        };
        let content = choices.bytes(96);
        let (method, packed) = match more >> 3 & 3 {
            0 => (0_u16, content.clone()),
            1 => {
                let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
                let _ = encoder.write_all(&content);
                (8, encoder.finish().unwrap_or_default())
            }
            // A method claimed for data that is not in it.
            2 => (8, content.clone()),
            _ => (u16::from(choices.byte() % 100), content.clone()),
        };
        let crc = if flags & 2 == 0 {
            crc32fast::hash(&content)
        } else {
            0x1234_5678
        };
        let mut general = 0_u16;
        if flags & 4 != 0 {
            general |= 1; // encrypted
        }
        if flags & 8 != 0 {
            general |= 8; // data descriptor
        }
        if more & 0x20 != 0 {
            general |= 0x800; // UTF-8 names
        }
        let (made_by, attributes) = match more >> 6 {
            0 => (0x0314_u16, 0o100_644_u32 << 16),
            1 => (0x0314, 0o120_777 << 16),
            2 => (0x0314, (0o010_644 << 16) | u32::from(choices.byte())),
            _ => (0x0014, 0x10),
        };
        let mut extra = Vec::new();
        let zip64 = flags & 0x10 != 0;
        if zip64 {
            let mut field = Vec::new();
            for value in [content.len() as u64, packed.len() as u64, zip.len() as u64] {
                field.extend_from_slice(&value.to_le_bytes());
            }
            field.truncate(usize::from(choices.byte() % 25));
            extra.extend_from_slice(&0x0001_u16.to_le_bytes());
            extra.extend_from_slice(&u16::try_from(field.len()).unwrap_or(0).to_le_bytes());
            extra.extend_from_slice(&field);
        }
        if flags & 0x20 != 0 {
            let unicode = if flags & 0x40 == 0 {
                NAMES[usize::from(choices.byte() % 8)].as_bytes().to_vec()
            } else {
                choices.bytes(48)
            };
            let checksum = if choices.byte() & 1 == 0 {
                crc32fast::hash(&name)
            } else {
                7
            };
            let mut field = vec![1];
            field.extend_from_slice(&checksum.to_le_bytes());
            field.extend_from_slice(&unicode);
            extra.extend_from_slice(&0x7075_u16.to_le_bytes());
            extra.extend_from_slice(&u16::try_from(field.len()).unwrap_or(0).to_le_bytes());
            extra.extend_from_slice(&field);
        }
        let local_name = if flags & 0x80 != 0 {
            b"other.txt".to_vec()
        } else {
            name.clone()
        };
        let small = |value: usize| {
            if zip64 {
                u32::MAX
            } else {
                u32::try_from(value).unwrap_or(u32::MAX)
            }
        };
        let offset = zip.len();
        let word = |bytes: &mut Vec<u8>, value: u32| bytes.extend_from_slice(&value.to_le_bytes());
        let half = |bytes: &mut Vec<u8>, value: u16| bytes.extend_from_slice(&value.to_le_bytes());
        word(&mut zip, 0x0403_4b50);
        for value in [20, general, method, 0, 0x5821] {
            half(&mut zip, value);
        }
        for value in [crc, small(packed.len()), small(content.len())] {
            word(&mut zip, value);
        }
        half(&mut zip, u16::try_from(local_name.len()).unwrap_or(0));
        half(&mut zip, 0);
        zip.extend_from_slice(&local_name);
        zip.extend_from_slice(&packed);
        if general & 8 != 0 {
            for value in [0x0807_4b50, crc, small(packed.len()), small(content.len())] {
                word(&mut zip, value);
            }
        }
        word(&mut directory, 0x0201_4b50);
        for value in [made_by, 20, general, method, 0, 0x5821] {
            half(&mut directory, value);
        }
        for value in [crc, small(packed.len()), small(content.len())] {
            word(&mut directory, value);
        }
        for value in [
            u16::try_from(name.len()).unwrap_or(0),
            u16::try_from(extra.len()).unwrap_or(0),
            0,
            0,
            0,
        ] {
            half(&mut directory, value);
        }
        word(&mut directory, attributes);
        word(&mut directory, small(offset));
        directory.extend_from_slice(&name);
        directory.extend_from_slice(&extra);
    }
    let start = u32::try_from(zip.len()).unwrap_or(u32::MAX);
    let length = u32::try_from(directory.len()).unwrap_or(u32::MAX);
    zip.extend_from_slice(&directory);
    zip.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    for value in [0_u16, 0, u16::from(count), u16::from(count)] {
        zip.extend_from_slice(&value.to_le_bytes());
    }
    zip.extend_from_slice(&length.to_le_bytes());
    zip.extend_from_slice(&start.to_le_bytes());
    let comment = choices.bytes(16);
    zip.extend_from_slice(&u16::try_from(comment.len()).unwrap_or(0).to_le_bytes());
    zip.extend_from_slice(&comment);
    (options, zip)
}

/// The second byte of a fix-up input: which compression wraps the TAR.
const COMPRESSIONS: [&str; 5] = [
    "Fuzz.tar",
    "Fuzz.tar.gz",
    "Fuzz.tar.bz2",
    "Fuzz.tar.xz",
    "Fuzz.tar.zst",
];

/// [`list_inspect_preview`] and [`extract`] on an archive whose checksums
/// are made right first, so mutations reach the parsers behind them: the
/// first byte is the options byte, the second picks a compression for a
/// TAR, and the rest is a TAR (each header's checksum is set, walking the
/// headers as a reader does) or a 7z (its start and next header CRCs are
/// set), compressed here as the second byte says.
pub fn fixed_up(data: &[u8]) {
    let [options, compression, archive @ ..] = data else {
        return;
    };
    let mut archive = archive.to_vec();
    let (name_index, packed) = if archive.starts_with(b"7z\xBC\xAF\x27\x1C") {
        fix_seven_zip(&mut archive);
        (1, archive)
    } else {
        fix_tar(&mut archive);
        let kind = usize::from(*compression) % COMPRESSIONS.len();
        (2 + kind, compress(kind, &archive))
    };
    let mut input = vec![(options & !7) | u8::try_from(name_index).unwrap_or(7)];
    input.extend_from_slice(&packed);
    list_inspect_preview(&input);
    extract(&input);
}

fn compress(kind: usize, tar: &[u8]) -> Vec<u8> {
    use std::io::Write;
    match kind {
        1 => {
            let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            let _ = encoder.write_all(tar);
            encoder.finish().unwrap_or_default()
        }
        2 => {
            let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
            let _ = encoder.write_all(tar);
            encoder.finish().unwrap_or_default()
        }
        3 => {
            let Ok(mut encoder) =
                lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::with_preset(0))
            else {
                return Vec::new();
            };
            let _ = encoder.write_all(tar);
            encoder.finish().unwrap_or_default()
        }
        4 => ruzstd::encoding::compress_to_vec(tar, ruzstd::encoding::CompressionLevel::Fastest),
        _ => tar.to_vec(),
    }
}

/// Sets the checksum of each TAR header, from the first, skipping each
/// member's data by the size its header gives.
fn fix_tar(tar: &mut [u8]) {
    let mut at = 0_usize;
    while let Some(block) = tar.get_mut(at..at + 512) {
        if block.iter().all(|&byte| byte == 0) {
            at += 512;
            continue;
        }
        block[148..156].fill(b' ');
        let sum: u64 = block.iter().map(|&byte| u64::from(byte)).sum();
        let digits = format!("{sum:06o}\0 ");
        block[148..156].copy_from_slice(&digits.as_bytes()[digits.len() - 8..]);
        let size = tar_size(&block[124..136]).unwrap_or(0);
        let data_blocks = size
            .div_ceil(512)
            .min(u64::try_from(tar.len()).unwrap_or(u64::MAX));
        at = at
            .saturating_add(512)
            .saturating_add(usize::try_from(data_blocks * 512).unwrap_or(usize::MAX));
    }
}

/// A TAR size field: octal, or base-256 when its first bit is set.
fn tar_size(field: &[u8]) -> Option<u64> {
    if field.first().is_some_and(|byte| byte & 0x80 != 0) {
        return field[1..].iter().try_fold(0_u64, |value, &byte| {
            value.checked_mul(256)?.checked_add(u64::from(byte))
        });
    }
    let digits: String = field
        .iter()
        .map(|&byte| char::from(byte))
        .filter(char::is_ascii_digit)
        .collect();
    u64::from_str_radix(&digits, 8).ok()
}

/// Sets a 7z's next header CRC, when the next header is inside it, and
/// its start header CRC.
fn fix_seven_zip(archive: &mut [u8]) {
    if archive.len() < 32 {
        return;
    }
    let word = |bytes: &[u8], at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or_default());
    let offset = word(archive, 12);
    let size = word(archive, 20);
    let next = usize::try_from(offset)
        .ok()
        .and_then(|offset| offset.checked_add(32))
        .zip(usize::try_from(size).ok())
        .and_then(|(start, size)| Some(start..start.checked_add(size)?));
    if let Some(next) = next.filter(|next| next.end <= archive.len()) {
        let crc = crc32fast::hash(&archive[next]);
        archive[28..32].copy_from_slice(&crc.to_le_bytes());
    }
    let crc = crc32fast::hash(&archive[12..32]);
    archive[8..12].copy_from_slice(&crc.to_le_bytes());
}

fn names(folder: &Path) -> Vec<String> {
    fs::read_dir(folder)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn check_only_child(folder: &Path, child: &str) {
    let found = names(folder);
    assert!(
        found.iter().all(|name| name == child),
        "the extraction wrote {found:?} into {}",
        folder.display()
    );
}

/// Checks that every item below `folder` is inside `top`, and that every
/// link resolves, by its text alone, to a place inside `top`.
fn check_contained(folder: &Path, top: &Path) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(&path).expect("a link's target");
            let resolved = lexical(&folder.join(&target));
            assert!(
                resolved.starts_with(top),
                "the link {} leads to {} outside the extracted folder",
                path.display(),
                target.display()
            );
            // Through other links, a link can lead further than its text.
            if let (Ok(real), Ok(real_top)) = (fs::canonicalize(&path), fs::canonicalize(top)) {
                assert!(
                    real.starts_with(&real_top),
                    "the link {} leads to {} outside the extracted folder",
                    path.display(),
                    real.display()
                );
            }
        } else if metadata.is_dir() {
            check_contained(&path, top);
        }
    }
}

/// `path` with `.` and `..` resolved by their text.
fn lexical(path: &Path) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            other => resolved.push(other),
        }
    }
    resolved
}
