// SPDX-License-Identifier: AGPL-3.0-only
//! The glyphs of the user's chosen symbol font, written once as symbolic
//! SVG icons into the user's cache folder and found there again later.
//!
//! The font gets a folder named after a hash of its path, size,
//! modification time and inode, of this reader's version and of the
//! icons asked for. It holds one `<icon name>.svg` for each icon whose
//! glyph the font has, named as the app asks (`ox-win-add-20-symbolic`),
//! which the app adds to its icon theme's search path, and an `index` of
//! them, written last, so a folder without one is never used. Folders of fonts no longer chosen are removed. Nothing is
//! installed: the font is read, not loaded into the desktop's font
//! configuration.

use std::collections::{BTreeSet, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

use super::truetype::{Font, MAX_FONT_SIZE};

/// Changes whenever the reader would write a glyph differently.
const READER_VERSION: u32 = 3;

/// The name of a cache folder's list of glyphs.
const INDEX: &str = "index";

/// Where in a cache folder the icons go: the scalable actions of the
/// hicolor theme, every icon theme's fallback. GTK draws only a themed
/// `-symbolic` icon in its widget's colour, not a loose file.
const ICON_DIRECTORY: &str = "hicolor/scalable/actions";

/// The hicolor theme's description, for a desktop that has none of its
/// own; GTK reads the first it finds on its search path.
const INDEX_THEME: &str = "[Icon Theme]\nName=Hicolor\nComment=Fallback icon theme\nHidden=true\n\
Directories=scalable/actions\n\n[scalable/actions]\nSize=16\nMinSize=1\nMaxSize=512\n\
Context=Actions\nType=Scalable\n";

/// Whether `name` may name an icon in the cache: lower-case letters,
/// digits and dashes, ending in `-symbolic`, so GTK draws it in its
/// widget's colour, and short enough for a file name.
pub fn is_glyph_icon_name(name: &str) -> bool {
    name.len() <= 96
        && name.ends_with("-symbolic")
        && name
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
}

/// One icon to make from a glyph: its name, the glyph's codepoint, and
/// the em the glyph is drawn at in a square of `size` pixels.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GlyphRequest {
    /// The icon's name, as [`is_glyph_icon_name`] allows.
    pub name: String,
    /// The glyph's codepoint.
    pub codepoint: u32,
    /// The em, in pixels: the font size the glyph is drawn at.
    pub em: u32,
    /// The square's edge, in pixels, at least the em.
    pub size: u32,
}

/// The glyphs of a chosen font, as written into the cache.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadedGlyphs {
    /// The folder holding the icons; `None` when the font gave none.
    folder: Option<PathBuf>,
    /// The icons the folder has.
    names: BTreeSet<String>,
}

impl LoadedGlyphs {
    /// The folder to add to the icon theme's search path: it holds the
    /// icons as a `hicolor` theme directory.
    pub fn folder(&self) -> Option<&Path> {
        self.folder.as_deref()
    }

    /// Whether the folder has the icon `name`.
    pub fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    /// How many of the icons asked for the font drew.
    pub fn found(&self) -> usize {
        self.names.len()
    }
}

/// The folder the glyph icons are kept in: `winspace/windows-glyphs` in
/// the user's cache folder.
pub fn glyph_cache_folder() -> PathBuf {
    glib::user_cache_dir().join("winspace").join("windows-glyphs")
}

/// The icons of `wanted`, each a glyph of `font` at its em in its
/// square, written into `cache` or found there. A font that is
/// missing, unreadable, too large or damaged gives none, as does a
/// codepoint the font lacks; a name [`is_glyph_icon_name`] refuses is
/// left out.
pub fn load_glyphs(font: &Path, wanted: &[GlyphRequest], cache: &Path) -> LoadedGlyphs {
    let mut wanted: Vec<GlyphRequest> = wanted
        .iter()
        .filter(|request| is_glyph_icon_name(&request.name) && (1..=512).contains(&request.em))
        .filter(|request| (request.em..=512).contains(&request.size))
        .cloned()
        .collect();
    wanted.sort_unstable();
    wanted.dedup_by(|a, b| a.name == b.name);
    let Ok(metadata) = fs::metadata(font) else {
        return LoadedGlyphs::default();
    };
    if !metadata.is_file()
        || fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(cache)
            .is_err()
    {
        return LoadedGlyphs::default();
    }
    let key = cache_key(font, &metadata, &wanted);
    let folder = cache.join(&key);
    let loaded = read_index(&folder).unwrap_or_else(|| {
        let _ = fs::remove_dir_all(&folder);
        let glyphs = draw_glyphs(font, metadata.len(), &wanted);
        if write_folder(&folder, &glyphs).is_err() {
            return LoadedGlyphs::default();
        }
        read_index(&folder).unwrap_or_default()
    });
    remove_unused(cache, &key);
    loaded
}

/// The SVG of each of `wanted` whose glyph the font at `path` has.
fn draw_glyphs(path: &Path, size: u64, wanted: &[GlyphRequest]) -> Vec<(String, String)> {
    if size > MAX_FONT_SIZE as u64 {
        return Vec::new();
    }
    let Ok(bytes) = fs::read(path) else {
        return Vec::new();
    };
    let Ok(font) = Font::parse(bytes) else {
        return Vec::new();
    };
    wanted
        .iter()
        .filter_map(|request| {
            let svg = font
                .glyph_svg(request.codepoint, request.em, request.size)
                .ok()??;
            Some((request.name.clone(), svg))
        })
        .collect()
}

/// The name of the cache folder: a 64-bit FNV-1a hash of the reader's
/// version, the icons and codepoints and the font's path, size,
/// modification time and inode, in hexadecimal.
fn cache_key(path: &Path, metadata: &fs::Metadata, wanted: &[GlyphRequest]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut add = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    add(&READER_VERSION.to_le_bytes());
    for request in wanted {
        add(request.name.as_bytes());
        add(&request.codepoint.to_le_bytes());
        add(&request.em.to_le_bytes());
        add(&request.size.to_le_bytes());
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

/// The glyphs `folder`'s index lists, `None` without an index.
fn read_index(folder: &Path) -> Option<LoadedGlyphs> {
    let index = fs::read_to_string(folder.join(INDEX)).ok()?;
    let names: BTreeSet<String> = index
        .lines()
        .take(4096)
        .filter(|name| is_glyph_icon_name(name))
        .filter(|name| folder.join(ICON_DIRECTORY).join(format!("{name}.svg")).is_file())
        .map(str::to_owned)
        .collect();
    Some(LoadedGlyphs {
        folder: (!names.is_empty()).then(|| folder.to_path_buf()),
        names,
    })
}

/// Writes `glyphs` and their index into a new folder beside `folder`,
/// then moves it into place, so a folder with an index is always whole.
fn write_folder(folder: &Path, glyphs: &[(String, String)]) -> std::io::Result<()> {
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
        let icons = partial.join(ICON_DIRECTORY);
        fs::DirBuilder::new().recursive(true).mode(0o700).create(&icons)?;
        fs::write(partial.join("hicolor").join("index.theme"), INDEX_THEME)?;
        let mut index = String::new();
        for (name, svg) in glyphs {
            fs::write(icons.join(format!("{name}.svg")), svg)?;
            let _ = writeln!(index, "{name}");
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

/// Removes the cache folders in `cache` other than `kept`.
fn remove_unused(cache: &Path, kept: &str) {
    let Ok(entries) = fs::read_dir(cache) else {
        return;
    };
    let kept: HashSet<&str> = HashSet::from([kept]);
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let is_key = name.len() == 16 && name.bytes().all(|byte| byte.is_ascii_hexdigit());
        let is_directory = entry.file_type().is_ok_and(|kind| kind.is_dir());
        if is_directory && is_key && !kept.contains(name) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}
