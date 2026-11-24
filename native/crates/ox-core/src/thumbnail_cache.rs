// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop's shared thumbnail cache, as the freedesktop Thumbnail
//! Managing Standard lays it out: `~/.cache/thumbnails/{normal,large,
//! x-large,xx-large}/<MD5 of the URI>.png`, each a PNG whose `Thumb::URI`,
//! `Thumb::MTime` and, when given, `Thumb::Size` keys name the version of
//! the file it was made from, and `fail/<program>/` for the files a
//! program could not make one of.
//!
//! Windows keeps its thumbnails in `thumbcache_*.db`, so a folder of
//! photos shows them at once the second time; the shared cache is the same
//! for this desktop, and other programs (Files, Dolphin, the desktop's
//! thumbnailer service) read and write it too. This module reads an entry's
//! keys without decoding its picture, so a stale entry or a failure
//! costs no decode, and writes entries as the standard asks: to a
//! temporary file renamed into place, readable by its owner only, in
//! folders only its owner may enter. Nothing here decodes a picture: the
//! app decodes an entry's pixels in its picture decoder process.
//!
//! Old entries are left to the desktop's cleaners (the housekeeping of
//! GNOME and of KDE, or `BleachBit`), as the standard intends: the app
//! removes only the entry of a file it moved itself, once that entry moved
//! with the file.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use rustix::fs::OFlags;

use crate::entry::{thumbnail_file, ThumbnailFlavor};

/// The folder under `fail/` the app records its failures in.
pub const FAILURE_FOLDER: &str = "openxplorer";

/// The folder under `fail/` GNOME's thumbnailer factory records failures
/// in, which GIO reports as `thumbnail::failed`: the app does not try
/// again what it failed to make either.
const GNOME_FAILURE_FOLDER: &str = "gnome-thumbnail-factory";

/// The largest cache file read: an xx-large thumbnail is at most 1024 ×
/// 1024 pixels, four megabytes even uncompressed; anything larger is no
/// thumbnail.
pub const MAX_ENTRY_BYTES: u64 = 16 * 1024 * 1024;

/// What every PNG starts with.
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// The keys the app writes and replaces, with `Software`.
const KEY_NAMES: [&str; 5] = [
    "Thumb::URI",
    "Thumb::MTime",
    "Thumb::Size",
    "Thumb::Mimetype",
    "Software",
];

/// What the app calls itself in the entries it writes.
pub const SOFTWARE: &str = "OpenXplorer";

/// The version of a file a thumbnail is made from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Version {
    /// The file's URI, which names the thumbnail.
    pub uri: String,
    /// Its modification time, in whole seconds since the epoch.
    pub modified: u64,
    /// Its size in bytes, when known.
    pub size: Option<u64>,
}

impl Version {
    /// The keys an entry of this version carries, in the order written.
    pub fn keys<'a>(&'a self, mime_type: Option<&'a str>) -> Vec<(&'static str, String)> {
        let mut keys = vec![
            ("Thumb::URI", self.uri.clone()),
            ("Thumb::MTime", self.modified.to_string()),
        ];
        if let Some(size) = self.size {
            keys.push(("Thumb::Size", size.to_string()));
        }
        if let Some(mime_type) = mime_type {
            keys.push(("Thumb::Mimetype", mime_type.to_owned()));
        }
        keys.push(("Software", SOFTWARE.to_owned()));
        keys
    }
}

/// The keys of a cache entry, from its PNG text chunks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keys {
    /// `Thumb::URI`.
    pub uri: Option<String>,
    /// `Thumb::MTime`, as written.
    pub mtime: Option<String>,
    /// `Thumb::Size`, as written.
    pub size: Option<String>,
    /// `Thumb::Mimetype`.
    pub mime_type: Option<String>,
}

impl Keys {
    /// Whether the entry was made from `version` of its file: the same URI
    /// and modification time and, when the entry gives one, the same size.
    /// A time written with a fraction of a second is read to the second.
    pub fn describe(&self, version: &Version) -> bool {
        let same_uri = self.uri.as_deref() == Some(version.uri.as_str());
        let same_time = self
            .mtime
            .as_deref()
            .and_then(|mtime| mtime.split('.').next())
            .and_then(|seconds| seconds.trim().parse::<u64>().ok())
            == Some(version.modified);
        let same_size = match (self.size.as_deref(), version.size) {
            (Some(written), Some(size)) => written.trim().parse::<u64>().ok() == Some(size),
            _ => true,
        };
        same_uri && same_time && same_size
    }
}

/// A current entry found in the cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// Where it is.
    pub path: PathBuf,
    /// Its size.
    pub flavor: ThumbnailFlavor,
    /// The whole file, for the picture decoder.
    pub bytes: Vec<u8>,
}

/// One chunk of a PNG: its type, its data, and where the whole chunk
/// (length, type, data and checksum) lies in the file.
struct Chunk<'a> {
    kind: &'a [u8],
    data: &'a [u8],
    whole: std::ops::Range<usize>,
}

/// The chunks of the PNG in `bytes`, up to its end or the first chunk cut
/// short; `None` for anything that is not a PNG.
fn chunks(bytes: &[u8]) -> Option<Vec<Chunk<'_>>> {
    if bytes.get(..PNG_SIGNATURE.len())? != PNG_SIGNATURE {
        return None;
    }
    let mut found = Vec::new();
    let mut at = PNG_SIGNATURE.len();
    while let Some(head) = bytes.get(at..at + 8) {
        let length = usize::try_from(u32::from_be_bytes(head[..4].try_into().ok()?)).ok()?;
        // The length, the kind, the data and the checksum; a chunk cut
        // short ends the reading.
        let end = at.checked_add(12)?.checked_add(length)?;
        let Some(data) = bytes.get(at + 8..end - 4) else {
            break;
        };
        if end > bytes.len() {
            break;
        }
        let kind = &head[4..8];
        found.push(Chunk {
            kind,
            data,
            whole: at..end,
        });
        at = end;
        if kind == b"IEND" {
            break;
        }
    }
    Some(found)
}

/// The longest key value read, once inflated: a URI of a few kilobytes at
/// most, so a compressed value made to inflate without end is not.
pub const MAX_KEY_BYTES: usize = 64 * 1024;

/// The keys of the PNG in `bytes`, or `None` for anything that is not a
/// PNG. Keys are read from plain (`tEXt`), compressed (`zTXt`) and
/// international (`iTXt`) text chunks, as gdk-pixbuf reads them: the
/// standard writes plain ones, and Qt, which Dolphin's thumbnails are
/// written with, compresses a value of 40 characters or more. A chunk cut
/// short ends the reading.
pub fn png_keys(bytes: &[u8]) -> Option<Keys> {
    let mut keys = Keys::default();
    for chunk in chunks(bytes)? {
        let Some((name, value)) = text_of(chunk.kind, chunk.data) else {
            continue;
        };
        let slot = match name {
            b"Thumb::URI" => &mut keys.uri,
            b"Thumb::MTime" => &mut keys.mtime,
            b"Thumb::Size" => &mut keys.size,
            b"Thumb::Mimetype" => &mut keys.mime_type,
            _ => continue,
        };
        slot.get_or_insert(value);
    }
    Some(keys)
}

/// The key and value of a text chunk of `kind`, or `None` for another
/// chunk or one that cannot be read.
fn text_of<'a>(kind: &[u8], data: &'a [u8]) -> Option<(&'a [u8], String)> {
    let split = data.iter().position(|byte| *byte == 0)?;
    let (name, rest) = (data.get(..split)?, data.get(split + 1..)?);
    // tEXt and zTXt are Latin-1; the keys the app looks at are written in
    // ASCII, and a URI's other characters are escaped.
    let latin1 = |bytes: &[u8]| bytes.iter().map(|byte| char::from(*byte)).collect::<String>();
    match kind {
        b"tEXt" => Some((name, latin1(rest))),
        // The compression method, 0 (zlib), then the compressed text.
        b"zTXt" => {
            let (&method, compressed) = rest.split_first()?;
            (method == 0).then_some(())?;
            Some((name, latin1(&inflate(compressed)?)))
        }
        // Whether it is compressed, the method, the language and the
        // translated key, each ended by a zero byte, then UTF-8 text.
        b"iTXt" => {
            let (&compressed, rest) = rest.split_first()?;
            let (&method, rest) = rest.split_first()?;
            let language_end = rest.iter().position(|byte| *byte == 0)?;
            let rest = rest.get(language_end + 1..)?;
            let translated_end = rest.iter().position(|byte| *byte == 0)?;
            let text = rest.get(translated_end + 1..)?;
            let text = match (compressed, method) {
                (0, _) => text.to_vec(),
                (1, 0) => inflate(text)?,
                _ => return None,
            };
            (text.len() <= MAX_KEY_BYTES).then_some(())?;
            Some((name, String::from_utf8(text).ok()?))
        }
        _ => None,
    }
}

/// The zlib stream `compressed`, inflated, if it is no longer than
/// [`MAX_KEY_BYTES`].
fn inflate(compressed: &[u8]) -> Option<Vec<u8>> {
    let mut text = Vec::new();
    flate2::read::ZlibDecoder::new(compressed)
        .take(MAX_KEY_BYTES as u64 + 1)
        .read_to_end(&mut text)
        .ok()?;
    (text.len() <= MAX_KEY_BYTES).then_some(text)
}

/// The current entry of `version` in `flavor` or a larger size under the
/// cache folder `cache_dir` (the user's `~/.cache`), smallest first.
/// Blocks.
pub fn look_up(cache_dir: &Path, version: &Version, flavor: ThumbnailFlavor) -> Option<Hit> {
    flavor.and_larger().find_map(|flavor| {
        let path = thumbnail_file(cache_dir, &version.uri, flavor);
        let bytes = read_entry(&path)?;
        png_keys(&bytes)?
            .describe(version)
            .then_some(Hit { path, flavor, bytes })
    })
}

/// Whether the app, or GNOME's thumbnailer factory, failed to make a
/// thumbnail of `version` of its file. Blocks.
pub fn has_failed(cache_dir: &Path, version: &Version) -> bool {
    failed_for(cache_dir, version, &[FAILURE_FOLDER, GNOME_FAILURE_FOLDER])
}

/// Whether the app itself failed to make a thumbnail of `version` of its
/// file: for a type GNOME's thumbnailer factory gives up on that the app
/// makes another way, as an office document. Blocks.
pub fn app_has_failed(cache_dir: &Path, version: &Version) -> bool {
    failed_for(cache_dir, version, &[FAILURE_FOLDER])
}

/// Whether one of `programs` recorded a failure for `version` of its file.
fn failed_for(cache_dir: &Path, version: &Version, programs: &[&str]) -> bool {
    programs.iter().any(|program| {
        let path = failure_folder(cache_dir, program).join(format!("{}.png", entry_name(&version.uri)));
        read_entry(&path)
            .and_then(|bytes| png_keys(&bytes))
            .is_some_and(|keys| keys.describe(version))
    })
}

fn failure_folder(cache_dir: &Path, program: &str) -> PathBuf {
    cache_dir.join("thumbnails").join("fail").join(program)
}

/// Records that no thumbnail could be made of `version` of its file, so
/// it is not tried again until the file changes. Blocks.
///
/// # Errors
///
/// The folder or the file could not be written.
pub fn record_failure(cache_dir: &Path, version: &Version) -> io::Result<PathBuf> {
    let path = failure_file(cache_dir, &version.uri);
    let keys = version.keys(None);
    let keys: Vec<(&str, &str)> = keys.iter().map(|(name, value)| (*name, value.as_str())).collect();
    let png = insert_text(&plain_png(), &keys).ok_or_else(|| io::Error::other("no PNG"))?;
    write_entry(&path, &png)?;
    Ok(path)
}

/// Where the app records a failure for `uri`.
pub fn failure_file(cache_dir: &Path, uri: &str) -> PathBuf {
    failure_folder(cache_dir, FAILURE_FOLDER).join(format!("{}.png", entry_name(uri)))
}

/// Writes `bytes` to `target` as the standard asks: into a temporary
/// file in the same folder, readable by its owner only, renamed into
/// place, in folders only the owner may enter. Blocks.
///
/// # Errors
///
/// The folder or the file could not be written.
pub fn write_entry(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let folder = target.parent().ok_or_else(|| io::Error::other("no folder"))?;
    fs::DirBuilder::new().recursive(true).mode(0o700).create(folder)?;
    let name = crate::random::random_hex(8)?;
    let temporary = folder.join(format!(".{name}.openxplorer.tmp"));
    let written = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags((OFlags::NOFOLLOW | OFlags::CLOEXEC).bits().cast_signed())
            .open(&temporary)?;
        // The mode given at creation is narrowed by the umask only.
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(bytes)?;
        file.flush()?;
        drop(file);
        fs::rename(&temporary, target)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// The PNG `png` with its keys replaced by those of `version` (and
/// `mime_type`): the thumbnail of a file moved or renamed, whose picture
/// is still right. `None` when `png` is no PNG.
pub fn with_keys(png: &[u8], version: &Version, mime_type: Option<&str>) -> Option<Vec<u8>> {
    let keys = version.keys(mime_type);
    let keys: Vec<(&str, &str)> = keys.iter().map(|(name, value)| (*name, value.as_str())).collect();
    insert_text(&strip_keys(png)?, &keys)
}

/// Moves the entries of the file that was at `old_uri` to its new place,
/// `moved`: each current entry is written again under the new URI's name
/// with the new URI in its keys, and the old one removed, so the moved
/// file shows its thumbnail without making it again, as Windows' cache,
/// which follows a file by its identity, does. An entry made from
/// another version (the move changed the file's time, as a copy between
/// disks may) is left alone. How many entries moved. Blocks.
pub fn follow_move(cache_dir: &Path, old_uri: &str, moved: &Version) -> usize {
    follow_moves(cache_dir, &[(old_uri.to_owned(), moved.clone())])
        .iter()
        .sum()
}

/// Moves the entries of several files at once, as [`follow_move`] moves
/// one's, each from its old URI to its new place: every entry is read
/// before any is removed or written, so files that swapped names in one
/// batch keep their own thumbnails. How many entries of each file moved.
/// Blocks.
pub fn follow_moves(cache_dir: &Path, moves: &[(String, Version)]) -> Vec<usize> {
    let mut staged = Vec::new();
    for (index, (old_uri, moved)) in moves.iter().enumerate() {
        let before = Version {
            uri: old_uri.clone(),
            ..moved.clone()
        };
        for flavor in ThumbnailFlavor::ALL {
            let old = thumbnail_file(cache_dir, old_uri, flavor);
            let Some(bytes) = read_entry(&old) else {
                continue;
            };
            let Some(keys) = png_keys(&bytes).filter(|keys| keys.describe(&before)) else {
                continue;
            };
            let Some(renamed) = with_keys(&bytes, moved, keys.mime_type.as_deref()) else {
                continue;
            };
            staged.push((index, old, thumbnail_file(cache_dir, &moved.uri, flavor), renamed));
        }
    }
    for (_, old, new, _) in &staged {
        if old != new {
            let _ = fs::remove_file(old);
        }
    }
    let mut moved = vec![0; moves.len()];
    for (index, _, new, renamed) in &staged {
        if write_entry(new, renamed).is_ok() {
            moved[*index] += 1;
        }
    }
    moved
}

/// An entry's name as a number: the MD5 of its URI, which takes 16 bytes
/// where its hexadecimal text takes 32 and an allocation, so the names of
/// a cache of a few hundred thousand entries fit in a few megabytes.
pub type EntryKey = u128;

/// The names of the entries in `flavor`'s folder under `cache_dir`, read
/// in one listing: which files probably have a thumbnail, known without
/// opening any of them. Blocks.
pub fn entry_names(cache_dir: &Path, flavor: ThumbnailFlavor) -> HashSet<EntryKey> {
    let folder = cache_dir.join("thumbnails").join(flavor.as_str());
    let Ok(listing) = fs::read_dir(folder) else {
        return HashSet::new();
    };
    listing
        .filter_map(Result::ok)
        .filter_map(|entry| key_of_name(entry.file_name().to_str()?.strip_suffix(".png")?))
        .collect()
}

/// The name of `uri`'s entries in every flavor's folder: the MD5 of the
/// URI, in hexadecimal.
pub fn entry_name(uri: &str) -> String {
    glib::compute_checksum_for_string(glib::ChecksumType::Md5, uri)
        .map(|hash| hash.to_string())
        .unwrap_or_default()
}

/// The name of `uri`'s entries as an [`EntryKey`].
pub fn entry_key(uri: &str) -> Option<EntryKey> {
    key_of_name(&entry_name(uri))
}

/// The key a name of 32 hexadecimal digits stands for.
fn key_of_name(name: &str) -> Option<EntryKey> {
    (name.len() == 32 && name.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| EntryKey::from_str_radix(name, 16).ok())
        .flatten()
}

/// Reads the regular file at `path`, never following a link, if it is no
/// larger than [`MAX_ENTRY_BYTES`]. Blocks.
fn read_entry(path: &Path) -> Option<Vec<u8>> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(
            (OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK)
                .bits()
                .cast_signed(),
        )
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_ENTRY_BYTES {
        return None;
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).ok()?);
    file.take(MAX_ENTRY_BYTES + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= MAX_ENTRY_BYTES).then_some(bytes)
}

/// The CRC-32 of a chunk's type and data, as PNG writes it.
fn crc32(parts: &[&[u8]]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in parts.iter().flat_map(|part| part.iter()) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

/// A whole chunk of `kind` holding `data`.
fn chunk(kind: [u8; 4], data: &[u8]) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(data.len() + 12);
    bytes.extend_from_slice(&u32::try_from(data.len()).ok()?.to_be_bytes());
    bytes.extend_from_slice(&kind);
    bytes.extend_from_slice(data);
    bytes.extend_from_slice(&crc32(&[&kind, data]).to_be_bytes());
    Some(bytes)
}

/// The Adler-32 of `bytes`, which ends a zlib stream.
fn adler32(bytes: &[u8]) -> u32 {
    let (mut low, mut high) = (1_u32, 0_u32);
    for byte in bytes {
        low = (low + u32::from(*byte)) % 65_521;
        high = (high + low) % 65_521;
    }
    (high << 16) | low
}

/// A PNG of one transparent pixel without keys: what a failure is
/// recorded as. Its pixels are a zlib stream of one stored block, so no
/// compressor is needed.
fn plain_png() -> Vec<u8> {
    // Width 1, height 1, 8 bits, RGBA, deflate, no filter, no interlace.
    let header = [0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0];
    // One row: the filter byte, then four bytes of a transparent pixel.
    let row = [0_u8; 5];
    let mut pixels = vec![0x78, 0x01, 0x01];
    pixels.extend_from_slice(&5_u16.to_le_bytes());
    pixels.extend_from_slice(&(!5_u16).to_le_bytes());
    pixels.extend_from_slice(&row);
    pixels.extend_from_slice(&adler32(&row).to_be_bytes());
    let mut png = PNG_SIGNATURE.to_vec();
    for (kind, data) in [(b"IHDR", &header[..]), (b"IDAT", &pixels[..]), (b"IEND", &[][..])] {
        png.extend(chunk(*kind, data).unwrap_or_default());
    }
    png
}

/// `png` with `keys` written as text chunks after its header.
fn insert_text(png: &[u8], keys: &[(&str, &str)]) -> Option<Vec<u8>> {
    let mut chunks = Vec::new();
    for (name, value) in keys {
        // A key's value is Latin-1 text; anything else is not written.
        let latin1: Option<Vec<u8>> = value.chars().map(|c| u8::try_from(u32::from(c)).ok()).collect();
        let mut data = name.as_bytes().to_vec();
        data.push(0);
        data.extend(latin1?);
        chunks.push((*b"tEXt", data));
    }
    insert_chunks(png, &chunks)
}

/// `png` with `chunks`, each its kind and data, written after its header.
fn insert_chunks(png: &[u8], chunks_to_add: &[([u8; 4], Vec<u8>)]) -> Option<Vec<u8>> {
    let found = chunks(png)?;
    let header = found.first().filter(|first| first.kind == b"IHDR")?;
    let mut out = png.get(..header.whole.end)?.to_vec();
    for (kind, data) in chunks_to_add {
        out.extend(chunk(*kind, data)?);
    }
    out.extend_from_slice(png.get(header.whole.end..)?);
    Some(out)
}

/// `png` without its `Thumb::` keys and `Software`, plain, compressed or
/// international.
fn strip_keys(png: &[u8]) -> Option<Vec<u8>> {
    let found = chunks(png)?;
    let mut out = PNG_SIGNATURE.to_vec();
    for chunk in &found {
        let is_key = [&b"tEXt"[..], b"zTXt", b"iTXt"].contains(&chunk.kind)
            && KEY_NAMES.iter().any(|name| {
                chunk
                    .data
                    .strip_prefix(name.as_bytes())
                    .is_some_and(|rest| rest.first() == Some(&0))
            });
        if !is_key {
            out.extend_from_slice(png.get(chunk.whole.clone())?);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(uri: &str) -> Version {
        Version {
            uri: uri.to_owned(),
            modified: 1_700_000_000,
            size: Some(4096),
        }
    }

    /// A thumbnail as another program writes it: a PNG of one pixel with
    /// `keys`, in the order given.
    fn entry_png(keys: &[(&str, &str)]) -> Vec<u8> {
        insert_text(&plain_png(), keys).expect("a PNG")
    }

    fn cache() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary cache")
    }

    fn put(cache: &Path, uri: &str, flavor: ThumbnailFlavor, png: &[u8]) -> PathBuf {
        let path = thumbnail_file(cache, uri, flavor);
        fs::create_dir_all(path.parent().expect("a folder")).expect("the folder");
        fs::write(&path, png).expect("the entry");
        path
    }

    /// An entry Files, Dolphin or the thumbnailer service wrote for the
    /// current version of a file is found and read whole, so the app shows
    /// it without making the thumbnail again.
    ///
    /// parity: VIEW-057
    #[test]
    fn a_current_entry_of_another_program_is_a_hit() {
        let cache = cache();
        let item = version("file:///home/demo/Pictures/Lake.jpg");
        let png = entry_png(&[
            ("Thumb::URI", &item.uri),
            ("Thumb::MTime", "1700000000"),
            ("Software", "GNOME::ThumbnailFactory"),
        ]);
        let path = put(cache.path(), &item.uri, ThumbnailFlavor::Normal, &png);

        let hit = look_up(cache.path(), &item, ThumbnailFlavor::Normal).expect("a hit");

        assert_eq!(hit.path, path);
        assert_eq!(hit.flavor, ThumbnailFlavor::Normal);
        assert_eq!(hit.bytes, png);
        let keys = png_keys(&hit.bytes).expect("keys");
        assert_eq!(keys.uri.as_deref(), Some(item.uri.as_str()));
    }

    /// An entry made from another version of the file is no hit: another
    /// modification time, another size when the entry gives one, or
    /// another URI (two URIs whose hashes meet) all make it stale.
    ///
    /// parity: VIEW-057
    #[test]
    fn a_changed_file_misses_its_old_entry() {
        let cache = cache();
        let item = version("file:///home/demo/Pictures/Lake.jpg");
        let write =
            |keys: &[(&str, &str)]| put(cache.path(), &item.uri, ThumbnailFlavor::Normal, &entry_png(keys));

        write(&[("Thumb::URI", &item.uri), ("Thumb::MTime", "1699999999")]);
        assert_eq!(
            look_up(cache.path(), &item, ThumbnailFlavor::Normal),
            None,
            "older"
        );

        write(&[
            ("Thumb::URI", &item.uri),
            ("Thumb::MTime", "1700000000"),
            ("Thumb::Size", "17"),
        ]);
        assert_eq!(
            look_up(cache.path(), &item, ThumbnailFlavor::Normal),
            None,
            "resized"
        );

        write(&[
            ("Thumb::URI", "file:///elsewhere.jpg"),
            ("Thumb::MTime", "1700000000"),
        ]);
        assert_eq!(
            look_up(cache.path(), &item, ThumbnailFlavor::Normal),
            None,
            "another file"
        );

        write(&[("Thumb::MTime", "1700000000")]);
        assert_eq!(
            look_up(cache.path(), &item, ThumbnailFlavor::Normal),
            None,
            "no URI"
        );

        // The size is checked only where the entry gives one.
        write(&[("Thumb::URI", &item.uri), ("Thumb::MTime", "1700000000")]);
        assert!(look_up(cache.path(), &item, ThumbnailFlavor::Normal).is_some());
        write(&[
            ("Thumb::URI", &item.uri),
            ("Thumb::MTime", "1700000000"),
            ("Thumb::Size", "4096"),
        ]);
        assert!(look_up(cache.path(), &item, ThumbnailFlavor::Normal).is_some());
        // A time written with its fraction is the same second.
        write(&[("Thumb::URI", &item.uri), ("Thumb::MTime", "1700000000.250000")]);
        assert!(look_up(cache.path(), &item, ThumbnailFlavor::Normal).is_some());
    }

    /// A larger size stands in for a missing smaller one, shrunk sharply;
    /// a smaller one never stands in for a larger, which would blur.
    ///
    /// parity: VIEW-057
    #[test]
    fn a_larger_entry_serves_a_smaller_size() {
        let cache = cache();
        let item = version("file:///home/demo/Pictures/Lake.jpg");
        let png = entry_png(&[("Thumb::URI", &item.uri), ("Thumb::MTime", "1700000000")]);
        put(cache.path(), &item.uri, ThumbnailFlavor::XLarge, &png);

        let hit = look_up(cache.path(), &item, ThumbnailFlavor::Normal).expect("the larger entry");
        assert_eq!(hit.flavor, ThumbnailFlavor::XLarge);
        assert_eq!(look_up(cache.path(), &item, ThumbnailFlavor::XXLarge), None);
    }

    /// Damaged files, links and files too large to be thumbnails are no
    /// hits, and reading their keys never panics.
    ///
    /// parity: VIEW-057
    #[test]
    fn broken_entries_are_misses() {
        let cache = cache();
        let item = version("file:///home/demo/Pictures/Lake.jpg");
        let good = entry_png(&[("Thumb::URI", &item.uri), ("Thumb::MTime", "1700000000")]);
        for cut in 0..good.len() {
            let _ = png_keys(&good[..cut]);
        }
        assert_eq!(png_keys(b"GIF89a"), None);
        let path = put(cache.path(), &item.uri, ThumbnailFlavor::Normal, &good[..20]);
        assert_eq!(
            look_up(cache.path(), &item, ThumbnailFlavor::Normal),
            None,
            "cut short"
        );

        // A link to a good entry elsewhere is not followed.
        fs::remove_file(&path).expect("removed");
        let elsewhere = cache.path().join("elsewhere.png");
        fs::write(&elsewhere, &good).expect("written");
        std::os::unix::fs::symlink(&elsewhere, &path).expect("a link");
        assert_eq!(
            look_up(cache.path(), &item, ThumbnailFlavor::Normal),
            None,
            "a link"
        );

        // Nor is a file far larger than any thumbnail read.
        fs::remove_file(&path).expect("removed");
        let mut huge = good.clone();
        huge.resize(usize::try_from(MAX_ENTRY_BYTES).expect("small") + 1, 0);
        fs::write(&path, &huge).expect("written");
        assert_eq!(
            look_up(cache.path(), &item, ThumbnailFlavor::Normal),
            None,
            "too large"
        );
    }

    /// A failure is recorded in the app's own `fail/` folder with the
    /// file's keys, privately, and holds until the file changes; GNOME's
    /// failures hold for the app too.
    ///
    /// parity: VIEW-057
    #[test]
    fn a_failure_holds_until_the_file_changes() {
        let cache = cache();
        let item = version("file:///home/demo/Pictures/Broken.jpg");
        assert!(!has_failed(cache.path(), &item));

        let path = record_failure(cache.path(), &item).expect("recorded");

        assert_eq!(path, failure_file(cache.path(), &item.uri));
        assert!(path.starts_with(cache.path().join("thumbnails/fail/openxplorer")));
        assert_eq!(
            fs::metadata(&path).expect("the file").permissions().mode() & 0o777,
            0o600
        );
        let folder = path.parent().expect("its folder");
        assert_eq!(
            fs::metadata(folder).expect("the folder").permissions().mode() & 0o777,
            0o700
        );
        let keys = png_keys(&fs::read(&path).expect("read")).expect("a PNG");
        assert_eq!(keys.uri.as_deref(), Some(item.uri.as_str()));
        assert_eq!(keys.mtime.as_deref(), Some("1700000000"));
        assert_eq!(keys.size.as_deref(), Some("4096"));
        assert!(has_failed(cache.path(), &item));

        let changed = Version {
            modified: item.modified + 60,
            ..item.clone()
        };
        assert!(!has_failed(cache.path(), &changed), "tried again once it changed");

        let other = version("file:///home/demo/Pictures/Other.jpg");
        let gnome = cache
            .path()
            .join("thumbnails/fail/gnome-thumbnail-factory")
            .join(format!("{}.png", entry_name(&other.uri)));
        fs::create_dir_all(gnome.parent().expect("a folder")).expect("the folder");
        fs::write(
            &gnome,
            entry_png(&[("Thumb::URI", &other.uri), ("Thumb::MTime", "1700000000")]),
        )
        .expect("written");
        assert!(has_failed(cache.path(), &other));
    }

    /// An entry is written whole or not at all: into a temporary file
    /// renamed into place, readable by its owner only, leaving nothing
    /// else in the folder; and it replaces the entry of an older version.
    ///
    /// parity: VIEW-057
    #[test]
    fn entries_are_written_atomically_and_privately() {
        let cache = cache();
        let item = version("file:///home/demo/Pictures/Lake.jpg");
        let target = thumbnail_file(cache.path(), &item.uri, ThumbnailFlavor::Large);
        put(cache.path(), &item.uri, ThumbnailFlavor::Large, b"old");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).expect("mode");
        let png = entry_png(&[("Thumb::URI", &item.uri), ("Thumb::MTime", "1700000000")]);

        write_entry(&target, &png).expect("written");

        assert_eq!(fs::read(&target).expect("read"), png);
        assert_eq!(
            fs::metadata(&target).expect("the file").permissions().mode() & 0o777,
            0o600
        );
        let names: Vec<_> = fs::read_dir(target.parent().expect("a folder"))
            .expect("listed")
            .map(|entry| entry.expect("an entry").file_name())
            .collect();
        assert_eq!(names.len(), 1, "no temporary file is left: {names:?}");
    }

    /// The keys of a PNG are replaced without touching its picture, and
    /// with the size and type.
    ///
    /// parity: VIEW-057
    #[test]
    fn keys_are_replaced_and_the_picture_kept() {
        let item = version("file:///home/demo/Pictures/New name.jpg");
        let old = entry_png(&[
            ("Thumb::URI", "file:///home/demo/Pictures/Old%20name.jpg"),
            ("Thumb::MTime", "1700000000"),
            ("Software", "Other"),
        ]);

        let renamed = with_keys(&old, &item, Some("image/jpeg")).expect("a PNG");

        let keys = png_keys(&renamed).expect("keys");
        assert_eq!(keys.uri.as_deref(), Some(item.uri.as_str()));
        assert_eq!(keys.mtime.as_deref(), Some("1700000000"));
        assert_eq!(keys.size.as_deref(), Some("4096"));
        assert_eq!(keys.mime_type.as_deref(), Some("image/jpeg"));
        assert!(keys.describe(&item));
        assert_eq!(strip_keys(&renamed), strip_keys(&old), "the same picture");
        assert_eq!(with_keys(b"not a png", &item, None), None);
    }

    /// A file moved or renamed by the app takes its thumbnails with it:
    /// each current entry is written under its new name and the old one
    /// removed; an entry of another version stays where it is.
    ///
    /// parity: VIEW-057
    #[test]
    fn a_moved_file_keeps_its_thumbnails() {
        let cache = cache();
        let old_uri = "file:///home/demo/Pictures/IMG_0001.jpg";
        let moved = version("file:///home/demo/Pictures/Holiday/Beach.jpg");
        let entry = entry_png(&[("Thumb::URI", old_uri), ("Thumb::MTime", "1700000000")]);
        let normal = put(cache.path(), old_uri, ThumbnailFlavor::Normal, &entry);
        // As Qt writes it, with the URI compressed: it moves the same way,
        // and keeps no copy of its old URI.
        let mut compressed_uri = b"Thumb::URI\0\0".to_vec();
        let mut compressed = Vec::new();
        flate2::read::ZlibEncoder::new(old_uri.as_bytes(), flate2::Compression::default())
            .read_to_end(&mut compressed)
            .expect("compressed");
        compressed_uri.extend(compressed);
        let qt_entry = insert_chunks(
            &entry_png(&[("Thumb::MTime", "1700000000")]),
            &[(*b"zTXt", compressed_uri)],
        )
        .expect("a PNG");
        let large = put(cache.path(), old_uri, ThumbnailFlavor::Large, &qt_entry);

        assert_eq!(follow_move(cache.path(), old_uri, &moved), 2);

        assert!(!normal.exists() && !large.exists(), "the old entries are gone");
        for flavor in [ThumbnailFlavor::Normal, ThumbnailFlavor::Large] {
            let hit = look_up(cache.path(), &moved, flavor).expect("moved with the file");
            assert_eq!(hit.flavor, flavor);
            assert!(chunks(&hit.bytes)
                .expect("a PNG")
                .iter()
                .all(|chunk| chunk.kind != b"zTXt"));
        }

        // A copy between disks may give the file a new time: its old entry
        // describes another version and is not carried over.
        let stale_uri = "file:///home/demo/Pictures/IMG_0002.jpg";
        let stale = put(
            cache.path(),
            stale_uri,
            ThumbnailFlavor::Normal,
            &entry_png(&[("Thumb::URI", stale_uri), ("Thumb::MTime", "1600000000")]),
        );
        let elsewhere = version("file:///media/demo/Backup/IMG_0002.jpg");
        assert_eq!(follow_move(cache.path(), stale_uri, &elsewhere), 0);
        assert!(stale.exists());
        assert_eq!(look_up(cache.path(), &elsewhere, ThumbnailFlavor::Normal), None);
    }

    /// The listing of a flavor's folder names the entries there, so the
    /// app knows which files have thumbnails without opening any.
    ///
    /// parity: VIEW-057
    #[test]
    fn the_listing_names_the_entries() {
        let cache = cache();
        assert!(entry_names(cache.path(), ThumbnailFlavor::Normal).is_empty());
        let uri = "file:///home/demo/Pictures/Lake.jpg";
        let path = put(cache.path(), uri, ThumbnailFlavor::Normal, b"x");
        // Neither a temporary file nor a name that is no MD5 counts.
        let folder = path.parent().expect("its folder");
        fs::write(folder.join(".5f3c.openxplorer.tmp"), b"x").expect("written");
        fs::write(folder.join("not-a-hash.png"), b"x").expect("written");
        fs::write(folder.join("0123456789abcdef0123456789abcdeZ.png"), b"x").expect("written");
        let names = entry_names(cache.path(), ThumbnailFlavor::Normal);
        assert_eq!(names.len(), 1);
        assert!(names.contains(&entry_key(uri).expect("a key")));
        assert_eq!(entry_name(uri).len(), 32);
        assert!(entry_names(cache.path(), ThumbnailFlavor::Large).is_empty());
    }

    /// Keys written compressed (`zTXt`), as Qt writes a value of 40
    /// characters or more, or as international text (`iTXt`), are read
    /// as plain ones are; a compressed value that inflates past the limit
    /// is not.
    ///
    /// parity: VIEW-057
    #[test]
    fn compressed_and_international_keys_are_read() {
        use std::io::Write as _;

        let item = version("file:///home/demo/Pictures/A%20rather%20long%20folder%20name/Lake.jpg");
        let zlib = |text: &[u8]| {
            let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(text).expect("compressed");
            encoder.finish().expect("compressed")
        };
        let mut compressed_uri = b"Thumb::URI\0\0".to_vec();
        compressed_uri.extend(zlib(item.uri.as_bytes()));
        let mut international_time = b"Thumb::MTime\0\0\0en\0\0".to_vec();
        international_time.extend_from_slice(b"1700000000");
        let mut compressed_size = b"Thumb::Size\0\x01\0\0\0".to_vec();
        compressed_size.extend(zlib(b"4096"));
        let png = insert_chunks(
            &plain_png(),
            &[
                (*b"zTXt", compressed_uri),
                (*b"iTXt", international_time),
                (*b"iTXt", compressed_size),
            ],
        )
        .expect("a PNG");

        let keys = png_keys(&png).expect("keys");
        assert_eq!(keys.uri.as_deref(), Some(item.uri.as_str()));
        assert_eq!(keys.mtime.as_deref(), Some("1700000000"));
        assert_eq!(keys.size.as_deref(), Some("4096"));
        assert!(keys.describe(&item));

        // A value that inflates past the limit is no key.
        let mut bomb = b"Thumb::URI\0\0".to_vec();
        bomb.extend(zlib(&vec![b'a'; MAX_KEY_BYTES + 1]));
        let png = insert_chunks(&plain_png(), &[(*b"zTXt", bomb)]).expect("a PNG");
        assert_eq!(png_keys(&png).expect("keys").uri, None);
    }

    /// Two files that swap names in one batch rename keep their own
    /// thumbnails: every entry is read before any is written.
    ///
    /// parity: VIEW-057
    #[test]
    fn files_that_swap_names_keep_their_own_thumbnails() {
        let cache = cache();
        let (a, b) = (
            "file:///home/demo/Pictures/a.jpg",
            "file:///home/demo/Pictures/b.jpg",
        );
        // Each picture told by a text chunk of its own, which moves with it.
        let entry = |uri: &str, picture: &str| {
            entry_png(&[
                ("Thumb::URI", uri),
                ("Thumb::MTime", "1700000000"),
                ("Description", picture),
            ])
        };
        put(
            cache.path(),
            a,
            ThumbnailFlavor::Normal,
            &entry(a, "picture of a"),
        );
        put(
            cache.path(),
            b,
            ThumbnailFlavor::Normal,
            &entry(b, "picture of b"),
        );
        // The same time and size, so each old entry would pass for the
        // other file.
        let now_b = version(b);
        let now_a = version(a);

        let moved = follow_moves(
            cache.path(),
            &[(a.to_owned(), now_b.clone()), (b.to_owned(), now_a.clone())],
        );

        assert_eq!(moved, [1, 1]);
        let picture = |version: &Version| {
            let hit = look_up(cache.path(), version, ThumbnailFlavor::Normal).expect("an entry");
            chunks(&hit.bytes)
                .expect("a PNG")
                .iter()
                .find(|chunk| chunk.data.starts_with(b"Description\0"))
                .map(|chunk| String::from_utf8_lossy(&chunk.data[12..]).into_owned())
        };
        assert_eq!(picture(&now_b).as_deref(), Some("picture of a"), "b is the old a");
        assert_eq!(picture(&now_a).as_deref(), Some("picture of b"), "a is the old b");
    }
}
