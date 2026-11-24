// SPDX-License-Identifier: AGPL-3.0-only
//! The icons of the user's chosen files, decoded once into the user's
//! cache folder and found there again on later starts.
//!
//! Each chosen file `OpenXplorer` recognizes ([`IconLibrary::of_file`]) gets
//! a folder named after a hash of its path, size, modification time and
//! inode, and of this reader's version, so a changed or replaced file is
//! read afresh. The folder holds one PNG file per role and edge
//! (`folder-16.png`) and an `index` that lists them, written last, so a
//! folder without one is never used. A file that gives no icons still gets
//! a folder with an empty index, so it is not read again at every start.
//! Folders of files no longer chosen are removed.
//!
//! The cache lives in `~/.cache/winspace/windows-icons` only: nothing is
//! written next to the chosen files or anywhere else.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

use super::pe::IconFile;
use super::picture::{decode, MAX_EDGE};
use super::{IconLibrary, IconRole};

/// Changes whenever the reader would decode a file differently, so the
/// cache of an earlier version is not used.
const READER_VERSION: u32 = 2;

/// The name of a cache folder's list of images.
const INDEX: &str = "index";

/// The most chosen files read.
pub const MAX_FILES: usize = 16;

/// One cached image of a role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedImage {
    /// Its width and height in pixels.
    pub edge: u32,
    /// The PNG file.
    pub path: PathBuf,
}

/// The cached images of every role the chosen files draw.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadedIcons {
    /// Each role's images, smallest first.
    roles: BTreeMap<IconRole, Vec<CachedImage>>,
}

impl LoadedIcons {
    /// The images of `role`, smallest first; empty when no chosen file
    /// draws it.
    pub fn images(&self, role: IconRole) -> &[CachedImage] {
        self.roles.get(&role).map_or(&[], Vec::as_slice)
    }

    /// How many roles the chosen files draw.
    pub fn found(&self) -> usize {
        self.roles.len()
    }

    /// The image of `role` to draw `pixels` device pixels square: the
    /// smallest at least that large, else the largest, so an image is
    /// scaled down rather than up.
    pub fn best(&self, role: IconRole, pixels: u32) -> Option<&CachedImage> {
        let images = self.images(role);
        images
            .iter()
            .find(|image| image.edge >= pixels)
            .or_else(|| images.last())
    }
}

/// The folder the decoded icons are kept in: `winspace/windows-icons` in
/// the user's cache folder.
pub fn cache_folder() -> PathBuf {
    glib::user_cache_dir().join("winspace").join("windows-icons")
}

/// The icons of `files` for every [`IconRole`], decoded into `cache` or
/// found there. Each role takes its images from the first of `files` that
/// has its group; a file that is missing, unreadable or damaged simply
/// gives none. Only the first [`MAX_FILES`] are read.
pub fn load_icons(files: &[PathBuf], cache: &Path) -> LoadedIcons {
    let mut loaded = LoadedIcons::default();
    let mut used = HashSet::new();
    let usable = fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(cache)
        .is_ok();
    for path in files.iter().take(MAX_FILES) {
        let Some(library) = IconLibrary::of_file(path) else {
            continue;
        };
        let Ok(metadata) = fs::metadata(path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let images = if usable {
            let key = cache_key(path, &metadata);
            used.insert(key.clone());
            cached_or_decoded(path, library, &cache.join(key))
        } else {
            BTreeMap::new()
        };
        for (role, images) in images {
            loaded.roles.entry(role).or_insert(images);
        }
    }
    if usable {
        remove_unused(cache, &used);
    }
    loaded
}

/// The name of the cache folder of `path`: a 64-bit FNV-1a hash of the
/// reader's version, the role table and the file's path, size,
/// modification time and inode, in hexadecimal.
fn cache_key(path: &Path, metadata: &fs::Metadata) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut add = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    add(&READER_VERSION.to_le_bytes());
    // The role table too, so a change of a role's group reads afresh.
    for role in IconRole::ALL {
        let (_, group, _) = role.source();
        add(role.key().as_bytes());
        add(&group.to_le_bytes());
    }
    add(path.as_os_str().as_bytes());
    add(&[0]);
    add(&metadata.len().to_le_bytes());
    add(&metadata.mtime().to_le_bytes());
    add(&metadata.mtime_nsec().to_le_bytes());
    add(&metadata.ino().to_le_bytes());
    add(&metadata.dev().to_le_bytes());
    format!("{hash:016x}")
}

/// True for the name of a cache folder [`cache_key`] makes.
fn is_cache_key(name: &str) -> bool {
    name.len() == 16 && name.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// The images of `path` from its cache `folder`, decoding them into it
/// first when it has no index yet.
fn cached_or_decoded(
    path: &Path,
    library: IconLibrary,
    folder: &Path,
) -> BTreeMap<IconRole, Vec<CachedImage>> {
    if let Some(images) = read_index(folder) {
        return images;
    }
    // A folder without an index was left half written; it is made again.
    let _ = fs::remove_dir_all(folder);
    let decoded = decode_file(path, library);
    if write_folder(folder, &decoded).is_err() {
        return BTreeMap::new();
    }
    read_index(folder).unwrap_or_default()
}

/// The images `folder`'s index lists, `None` without an index. A line
/// that does not name a known role and an edge of 1 to 256 is left out.
fn read_index(folder: &Path) -> Option<BTreeMap<IconRole, Vec<CachedImage>>> {
    let index = fs::read_to_string(folder.join(INDEX)).ok()?;
    let mut images: BTreeMap<IconRole, Vec<CachedImage>> = BTreeMap::new();
    for line in index.lines().take(IconRole::ALL.len() * 64) {
        let Some((key, edge)) = line.split_once(' ') else {
            continue;
        };
        let (Some(role), Ok(edge)) = (IconRole::from_key(key), edge.parse::<u32>()) else {
            continue;
        };
        if !(1..=MAX_EDGE).contains(&edge) {
            continue;
        }
        let path = folder.join(image_name(role, edge));
        images.entry(role).or_default().push(CachedImage { edge, path });
    }
    for list in images.values_mut() {
        list.sort_by_key(|image| image.edge);
        list.dedup_by_key(|image| image.edge);
    }
    Some(images)
}

/// The file name of `role`'s image `edge` pixels square.
fn image_name(role: IconRole, edge: u32) -> String {
    format!("{}-{edge}.png", role.key())
}

/// The PNG files of `path`'s icons for each role of `library`, by role and
/// edge. Only square images are kept, and of several of one edge the
/// deepest that decodes. A file that cannot be read gives none.
fn decode_file(path: &Path, library: IconLibrary) -> BTreeMap<(IconRole, u32), Vec<u8>> {
    let mut pictures = BTreeMap::new();
    let Ok(file) = IconFile::open(path) else {
        return pictures;
    };
    for role in IconRole::ALL {
        let (source, group, _) = role.source();
        if source != library {
            continue;
        }
        let Ok(mut images) = file.group(group) else {
            continue;
        };
        // Deepest first, so the first that decodes at an edge is kept.
        images.sort_by_key(|image| std::cmp::Reverse(image.bit_count));
        for image in images {
            let Ok(decoded) = decode(&image.data) else {
                continue;
            };
            let (width, height) = decoded.size();
            if width != height {
                continue;
            }
            pictures.entry((role, width)).or_insert_with(|| decoded.to_png());
        }
    }
    pictures
}

/// Writes `pictures` and their index into a new folder beside `folder`,
/// then moves it into place, so a folder with an index is always whole.
/// Another `OpenXplorer` that wrote it first wins.
fn write_folder(folder: &Path, pictures: &BTreeMap<(IconRole, u32), Vec<u8>>) -> std::io::Result<()> {
    let parent = folder.parent().ok_or(std::io::ErrorKind::NotFound)?;
    let name = folder.file_name().ok_or(std::io::ErrorKind::NotFound)?;
    let partial = parent.join(format!(
        ".partial-{}-{}",
        name.to_string_lossy(),
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&partial);
    fs::DirBuilder::new().mode(0o700).create(&partial)?;
    let written = (|| {
        let mut index = String::new();
        for ((role, edge), png) in pictures {
            fs::write(partial.join(image_name(*role, *edge)), png)?;
            let _ = writeln!(index, "{} {edge}", role.key());
        }
        let mut file = fs::File::create(partial.join(INDEX))?;
        file.write_all(index.as_bytes())?;
        file.sync_all()
    })();
    let moved = written.and_then(|()| fs::rename(&partial, folder));
    if moved.is_err() {
        let _ = fs::remove_dir_all(&partial);
        if folder.join(INDEX).is_file() {
            return Ok(());
        }
    }
    moved
}

/// Removes the cache folders in `cache` that no chosen file uses.
fn remove_unused(cache: &Path, used: &HashSet<String>) {
    let Ok(entries) = fs::read_dir(cache) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let is_directory = entry.file_type().is_ok_and(|kind| kind.is_dir());
        if is_directory && is_cache_key(name) && !used.contains(name) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}
