// SPDX-License-Identifier: AGPL-3.0-only
//! Previews of files in the folder views (VIEW-057), from the desktop's
//! shared thumbnail cache, as Dolphin and Files show them.
//!
//! A view asks for a preview when it binds a row or tile and drops the
//! request when GTK unbinds it, so rows scrolled past cost nothing more.
//! As Windows shows a folder's thumbnails at once from its thumbcache, a
//! folder visited again shows them at once from the shared cache: a
//! preview shown lately is kept decoded ([`memory`]) and drawn on the
//! frame its tile is bound ([`kept_preview`]); any other is read from the
//! cache first ([`reader`]), in batches, its keys checked against the
//! file's time and size without decoding it ([`thumbnail_cache`]), and a
//! file a program failed on is not tried again until it changes. A
//! missing thumbnail is made by the session's thumbnailer service
//! ([`service`]) when it has one and supports the type, else, for a
//! picture gdk-pixbuf may read ([`pixbuf_may_read`]), by the app itself
//! ([`writer`]), which costs less than a program, else by a thumbnailer
//! the desktop installs for the type ([`thumbnailers`], a program the app
//! starts in a sandbox), and written to the cache for the next visit and
//! for the desktop's other programs. The tiles on the screen have theirs
//! made first ([`Slots::take_in_turn`]). The desktop's thumbnailer is asked
//! for a picture only when the app cannot read it
//! ([`writer::Failure::Unreadable`]): one the cache refuses would be
//! refused for the program too, and a dropped
//! request needs no picture at all. Only a few are decoded or made at a
//! time ([`slots`]); a type that cannot be previewed, or a failed attempt,
//! keeps its icon. Every picture the app decodes itself, and every file it
//! reads back from the cache, goes through [`picture_guard`], so a huge,
//! damaged or animated one keeps its icon instead of holding the app up.
//! [`PreviewPolicy`] says which items of a folder get previews, from the
//! Settings the user chose (VIEW-058). Files on a server or a device are
//! made on slots of their own ([`making`]), so a share that stops
//! answering never holds up the thumbnails of this computer's files.

pub(crate) mod folder_preview;
mod memory;
mod network_fs;
mod reader;
mod service;
mod slots;
mod thumbnailers;
pub(crate) mod trace;
mod writer;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gtk::gdk_pixbuf::{Pixbuf, PixbufFormat};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use ox_core::entry::{cached_thumbnail_for, CachedThumbnail, Entry, ThumbnailFlavor, THUMBNAIL_ATTRIBUTES};
use ox_core::settings::{ViewOptions, PREVIEW_SIZE_LIMIT};
use ox_core::thumbnail_cache::{self, Version};

pub(crate) use network_fs::is_on_network_filesystem;
pub(crate) use slots::Slots;
pub(crate) use thumbnailers::documents;
#[cfg(feature = "fuzzing")]
pub(crate) use thumbnailers::fuzz_thumbnailer;
pub(crate) use thumbnailers::{decoder_sandbox, pdf_pages};

use crate::picture_guard::{self, Limits};
use memory::Memory;
use reader::Cached;
use thumbnailers::Catalogue;
use writer::{Made, Source};

/// How many previews are decoded at a time.
const DECODING_AT_ONCE: usize = 4;
/// How many thumbnails are made at a time, at most: each decodes a picture
/// or runs a thumbnailer in a process of its own.
const MAKING_AT_MOST: usize = 3;

/// How many thumbnails are made at a time: one fewer than the computer's
/// processors, so the app and the desktop keep one, between two and
/// [`MAKING_AT_MOST`].
fn making_at_once(processors: usize) -> usize {
    processors.saturating_sub(1).clamp(2, MAKING_AT_MOST)
}
/// How many thumbnails of files on a server or a device are made at a
/// time, on slots of their own: a share that stops answering holds these,
/// never the ones of this computer's files.
const NETWORK_MAKING_AT_ONCE: usize = 2;
/// How much the previews kept decoded may take: 64 MB, about 2,700 photos
/// at Large icons, or 90 at Extra large icons on a screen of twice the
/// pixels, each kept at the size it is shown.
const MEMORY_BUDGET: usize = 64 * 1024 * 1024;
/// How long whether a folder is on the network is remembered.
const NETWORK_ANSWER_LIFE: Duration = Duration::from_secs(30);
/// The smallest edge, in device pixels, at which an office document shows
/// its thumbnail: Medium icons and larger. A row of Details, List or
/// Small icons keeps the document's type icon, as Windows draws it there,
/// even when the cache has its thumbnail: a page 16 pixels tall shows
/// nothing of it, and would cost a run of `LibreOffice` to make.
const SMALLEST_DOCUMENT_THUMBNAIL: i32 = 40;

/// Which items of the folder shown get previews.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is a separate option of the Settings page"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct PreviewPolicy {
    /// Previews are shown at all; off, every item keeps its icon.
    pub shown: bool,
    /// Items on a server or a device get previews too, wherever they are
    /// listed; off, they keep their icons even in a folder on this
    /// computer (search results, Recent, Quick access).
    pub remote: bool,
    /// Files larger than this keep their icon.
    pub size_limit: Option<u64>,
    /// Pictures get previews.
    pub pictures: bool,
    /// Videos get previews.
    pub videos: bool,
    /// Every other type gets previews.
    pub documents: bool,
    /// The folder shown is on the network, as a kernel CIFS mount is
    /// though its files have `file:` URIs: its files' thumbnails are made
    /// on the network's own slots without asking their file system first.
    pub folder_on_network: bool,
}

impl PreviewPolicy {
    /// The policy for a folder that is on this computer, or not
    /// (`is_remote`), under the user's `options`.
    pub(crate) fn for_folder(options: &ViewOptions, is_remote: bool) -> Self {
        Self {
            shown: options.show_previews && (options.remote_previews || !is_remote),
            remote: options.remote_previews,
            size_limit: options.skip_large_previews.then_some(PREVIEW_SIZE_LIMIT),
            pictures: options.preview_pictures,
            videos: options.preview_videos,
            documents: options.preview_documents,
            folder_on_network: is_remote,
        }
    }

    /// Whether items of `content_type` get previews.
    fn allows_type(self, content_type: &str) -> bool {
        match content_type.split_once('/').map(|(kind, _)| kind) {
            Some("image") => self.pictures,
            Some("video") => self.videos,
            _ => self.documents,
        }
    }

    /// The preview request for `entry` at `pixels` device pixels, when
    /// [`Self::allows`] it; with network previews off it asks for a file
    /// on this computer's own disks only.
    pub(crate) fn request(self, entry: &Entry, pixels: i32) -> Option<PreviewRequest> {
        let request = PreviewRequest::for_entry(entry, pixels).filter(|_| self.allows(entry))?;
        Some(PreviewRequest {
            local_only: !self.remote,
            on_network: self.folder_on_network || is_remote_item(entry),
            ..request
        })
    }

    /// Whether `entry` may show a preview: a file of a known type within
    /// the size limit.
    pub(crate) fn allows(self, entry: &Entry) -> bool {
        let within_limit = match (self.size_limit, entry.size) {
            (Some(limit), Some(size)) => size <= limit,
            _ => true,
        };
        let type_allowed = entry
            .content_type
            .as_deref()
            .is_some_and(|content_type| self.allows_type(content_type));
        let reachable = self.remote || !is_remote_item(entry);
        self.shown && !entry.is_dir && !entry.is_virtual && type_allowed && within_limit && reachable
    }
}

/// Whether `entry`, or the item it stands for, is on a server or a
/// device rather than this computer, by its URI: a search result or a
/// recent item may be listed in a folder that is not.
fn is_remote_item(entry: &Entry) -> bool {
    use ox_core::location::{location_kind, LocationKind};

    std::iter::once(entry.uri.as_str())
        .chain(entry.target_uri.as_deref())
        .chain(entry.meta.recent_target.as_deref())
        .any(|uri| {
            matches!(
                location_kind(uri),
                LocationKind::Smb | LocationKind::Remote | LocationKind::Cloud | LocationKind::Device
            )
        })
}

/// The file `entry` stands for, when it is another: a Recently used
/// files row's file (kept in its metadata, as `GVfs` names it only in the
/// row's target), or a share's or shortcut's target.
fn stands_for(entry: &Entry) -> Option<&str> {
    entry
        .meta
        .recent_target
        .as_deref()
        .or(entry.target_uri.as_deref())
}

/// A decoded preview's item, version and size.
type PreviewKey = (String, Option<u64>, i32);

/// What a preview is wanted of.
#[derive(Debug, Clone)]
pub(crate) struct PreviewRequest {
    /// The item's URI.
    pub uri: String,
    /// Its content type.
    pub content_type: String,
    /// Its modification time, which tells a current thumbnail from an old.
    pub modified: Option<u64>,
    /// Its size in bytes, which does too.
    pub size: Option<u64>,
    /// The edge of the picture wanted, in device pixels.
    pub pixels: i32,
    /// Only a file on this computer's own disks gets a preview: one on a
    /// network file system (a kernel share, sshfs, `GVfs`'s mounts) keeps its
    /// icon, as network previews are off ([`network_fs`]).
    pub local_only: bool,
    /// The file the item stands for, when it is another (a recent item's,
    /// a search result's), which [`Self::local_only`] looks at too.
    pub target_uri: Option<String>,
    /// The item is on a server or a device, by its URI or the one it
    /// stands for; [`preview`] adds a file of this computer's tree that is
    /// on a network file system. Its thumbnail is made on the network's
    /// own slots ([`making`]).
    pub on_network: bool,
    /// Whether the item's tile is on the screen now: one that is has its
    /// thumbnail made before those GTK binds around it
    /// ([`Slots::take_in_turn`]).
    pub on_screen: OnScreen,
}

/// Whether a preview's tile is on the screen, asked when its thumbnail
/// waits to be made; without a tile, as for a dialog's picture, always.
#[derive(Clone, Default)]
pub(crate) struct OnScreen(Option<Rc<dyn Fn() -> bool>>);

impl OnScreen {
    /// Asks `on_screen` whether the tile is on the screen.
    pub(crate) fn new(on_screen: impl Fn() -> bool + 'static) -> Self {
        Self(Some(Rc::new(on_screen)))
    }

    /// Whether the tile is on the screen now.
    pub(crate) fn get(&self) -> bool {
        self.0.as_ref().is_none_or(|on_screen| on_screen())
    }
}

impl std::fmt::Debug for OnScreen {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(if self.0.is_some() {
            "OnScreen(asked)"
        } else {
            "OnScreen(always)"
        })
    }
}

impl PreviewRequest {
    /// The request for `entry` at `pixels` device pixels, if the entry
    /// has a type.
    pub(crate) fn for_entry(entry: &Entry, pixels: i32) -> Option<Self> {
        Some(Self {
            uri: entry.uri.clone(),
            content_type: entry.content_type.clone()?,
            // The thumbnail cache keeps unsigned times, as its MTime is.
            modified: entry.modified.and_then(|seconds| u64::try_from(seconds).ok()),
            size: entry.size,
            pixels,
            local_only: false,
            target_uri: stands_for(entry).map(str::to_owned),
            on_network: false,
            on_screen: OnScreen::default(),
        })
    }

    /// The version of the file a thumbnail must have been made from, when
    /// its time is known.
    fn version(&self) -> Option<Version> {
        Some(Version {
            uri: self.uri.clone(),
            modified: self.modified?,
            size: self.size,
        })
    }
}

thread_local! {
    static DECODING: Rc<Slots> = Rc::new(Slots::new(DECODING_AT_ONCE));
    static MAKING: Rc<Slots> = Rc::new(Slots::new(making_at_once(
        std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
    )));
    static NETWORK_MAKING: Rc<Slots> = Rc::new(Slots::new(NETWORK_MAKING_AT_ONCE));
    /// Office documents' thumbnails, made one at a time by `LibreOffice`
    /// ([`documents`]): the others wait here without holding a worker.
    static DOCUMENT_MAKING: Rc<Slots> = Rc::new(Slots::new(1));
    /// The previews shown lately, decoded, by item and size.
    static MEMORY: RefCell<Memory> = RefCell::new(Memory::new(MEMORY_BUDGET));
    /// Whether a folder is on a network file system, and when that was
    /// asked.
    static NETWORK_FOLDERS: RefCell<HashMap<PathBuf, (bool, Instant)>> = RefCell::new(HashMap::new());
    /// The folders whose file systems are being asked, with the requests
    /// waiting for the answer.
    static ASKING: RefCell<HashMap<Vec<PathBuf>, Vec<async_channel::Sender<bool>>>> = RefCell::new(HashMap::new());
    /// Sharp pictures decoded at a tile's size (Gallery), by item, version
    /// and edge; only the last few, as they are large.
    static SHARP_KEPT: RefCell<Vec<(PreviewKey, gdk::Texture)>> = const { RefCell::new(Vec::new()) };
    /// Item versions no thumbnail could be made of in this run.
    static FAILED: RefCell<HashSet<(String, Option<u64>)>> = RefCell::new(HashSet::new());
    /// The types gdk-pixbuf's loaders declare they read.
    static PIXBUF_TYPES: HashSet<String> = Pixbuf::formats()
        .iter()
        .flat_map(PixbufFormat::mime_types)
        .map(|mime| mime.to_string())
        .collect();
}

/// Whether gdk-pixbuf may be asked to decode the item of `content_type`,
/// given the `declared` MIME types its loaders say they read.
///
/// Every picture qualifies, not only the types a loader declares: what a
/// loader reads and what GIO names the file differ, gdk-pixbuf's dds
/// loader declaring `image/x-dds` alone for the `image/vnd.ms-dds` files
/// GIO reports, so matching the lists exactly would leave pictures
/// without thumbnails. Decoding settles it instead: [`writer`] reads
/// what a loader reads and answers [`writer::Failure::Unreadable`] for the
/// rest, which leaves the item to a thumbnailer of the desktop's, or to
/// its icon when there is none.
fn pixbuf_may_read(content_type: &str, declared: &HashSet<String>) -> bool {
    let is_picture = content_type
        .split_once('/')
        .is_some_and(|(kind, _)| kind == "image");
    is_picture || declared.contains(content_type)
}

/// The slots `request`'s thumbnail is made on: the network's own for a
/// file on a server, a device or a network file system, so one that does
/// not answer holds none of the slots of this computer's files.
fn making(request: &PreviewRequest) -> Rc<Slots> {
    if request.on_network {
        NETWORK_MAKING.with(Rc::clone)
    } else {
        MAKING.with(Rc::clone)
    }
}

/// The preview `request` asks for, or `None` when the item keeps its icon.
///
/// One shown lately comes from memory. Otherwise the shared cache is read
/// first ([`reader`]), in batches, and only a file it has no current
/// thumbnail of, nor a recorded failure, has one made ([`make_thumbnail`]),
/// which is written to the cache for the next visit, and for the other
/// programs of the desktop.
pub(crate) async fn preview(request: PreviewRequest) -> Option<gdk::Texture> {
    // An office document in a row keeps its type icon: nothing is even
    // looked up for it.
    if keeps_type_icon(&request) {
        return None;
    }
    // A file of this computer's tree may be on a share mounted there (a
    // kernel share, sshfs, GVfs's FUSE folder): it keeps its icon with
    // network previews off, and is made on the network's slots with them on.
    let on_network = request.on_network || is_on_network(&request.uri, request.target_uri.as_deref()).await;
    if request.local_only && on_network {
        return None;
    }
    note_kept_previews_used();
    if let Some(texture) = remembered(&request) {
        trace::event("kept-hit", &request.uri);
        return Some(texture);
    }
    let request = PreviewRequest {
        on_network,
        ..request
    };
    let texture = look_up_or_make(&request).await?;
    MEMORY.with_borrow_mut(|memory| {
        memory.put(
            (request.uri.clone(), request.pixels),
            texture.clone(),
            request.modified,
            request.size,
        );
    });
    note_kept_previews_used();
    Some(texture)
}

/// Gets the previews of the folder about to be shown ready: lists the
/// names of the shared cache's entries ([`reader::probably_cached`]) and
/// starts the picture decoders that read them, both in the background,
/// once per run. A view calls it when it is about to show previews, so
/// with "Always show icons, never thumbnails" nothing is started.
pub(crate) fn prepare() {
    thread_local! {
        static PREPARED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if cfg!(test) || PREPARED.replace(true) {
        return;
    }
    let cache_dir = glib::user_cache_dir();
    for flavor in ThumbnailFlavor::ALL {
        reader::probably_cached(&cache_dir, "", flavor);
    }
    picture_guard::warm_up_decoders(reader::READERS);
}

/// The preview of `request` shown lately, if it is kept in memory and
/// still of the current version of its file: what a view shows on the
/// frame it binds a tile, without waiting for anything. With network
/// previews off, only for a file known to be on this computer's disks.
pub(crate) fn kept_preview(request: &PreviewRequest) -> Option<gdk::Texture> {
    if request.local_only && !known_local(request) {
        return None;
    }
    let texture = remembered(request)?;
    note_kept_previews_used();
    Some(texture)
}

fn remembered(request: &PreviewRequest) -> Option<gdk::Texture> {
    MEMORY.with_borrow_mut(|memory| {
        memory.get(
            &(request.uri.clone(), request.pixels),
            request.modified,
            request.size,
        )
    })
}

/// Whether the shared cache probably holds a thumbnail for `request`,
/// known without touching the disk ([`reader::probably_cached`]): a view
/// draws such a tile without its type icon for the moment the thumbnail
/// takes to read, rather than an icon replaced a moment later.
pub(crate) fn probably_cached(request: &PreviewRequest) -> bool {
    request.modified.is_some()
        && reader::probably_cached(
            &glib::user_cache_dir(),
            &request.uri,
            ThumbnailFlavor::for_pixels(request.pixels),
        )
}

/// The most files of a moved folder whose thumbnails follow it.
const MOST_FOLLOWED: usize = 5_000;

/// Moves the thumbnails of the items the app moved or renamed, each from
/// its old place to its new one in `moved` (and those of the files inside
/// a moved folder), in the shared cache: the files keep their thumbnails,
/// as Windows' cache, which knows a file by its identity, keeps them.
/// Only local files are followed. Runs off the main thread.
pub(crate) fn follow_moves(moved: Vec<(String, String)>) {
    let moved: Vec<(PathBuf, PathBuf)> = moved
        .into_iter()
        .filter_map(|(old, new)| Some((gio::File::for_uri(&old).path()?, gio::File::for_uri(&new).path()?)))
        .collect();
    if moved.is_empty() {
        return;
    }
    let cache_dir = glib::user_cache_dir();
    // Test safety: tests write only into a private cache.
    #[cfg(test)]
    if !cache_dir.starts_with(std::env::temp_dir()) {
        return;
    }
    glib::spawn_future_local(async move {
        let followed = gio::spawn_blocking(move || follow_moves_of(&cache_dir, &moved))
            .await
            .unwrap_or_default();
        for (old_uri, new_uri) in followed {
            MEMORY.with_borrow_mut(|memory| memory.forget(&old_uri));
            for flavor in ThumbnailFlavor::ALL {
                reader::note_written(&new_uri, flavor);
            }
        }
    });
}

/// Moves the thumbnails of the items moved, each from its old path to its
/// new one in `moved`, files or folders of files, in the cache under
/// `cache_dir`, all at once ([`thumbnail_cache::follow_moves`]): the old
/// and new URIs of each file whose thumbnails moved. An item now on a
/// network file system is passed over: its folder is not listed over the
/// network, and its thumbnails, if any, are named by another URI. Blocks.
fn follow_moves_of(cache_dir: &Path, moved: &[(PathBuf, PathBuf)]) -> Vec<(String, String)> {
    let mut seen = 0_usize;
    let mut files = Vec::new();
    for (old, new) in moved {
        if network_fs::is_on_network_filesystem(new) {
            continue;
        }
        files.extend(files_moved(old, new, &mut seen));
    }
    let moved_entries = thumbnail_cache::follow_moves(cache_dir, &files);
    files
        .into_iter()
        .zip(moved_entries)
        .filter(|(_, entries)| *entries > 0)
        .map(|((old_uri, version), _)| {
            trace::event("moved", &version.uri);
            (old_uri, version.uri)
        })
        .collect()
}

/// The files moved from `old` to `new`, the item itself or, for a folder,
/// the files in it, each its old URI and its version now; at most
/// [`MOST_FOLLOWED`] items are looked at, counted in `seen`. Links are not
/// followed. Blocks.
fn files_moved(old: &Path, new: &Path, seen: &mut usize) -> Vec<(String, Version)> {
    let mut files = Vec::new();
    let mut folders = vec![(old.to_path_buf(), new.to_path_buf())];
    while let Some((old, new)) = folders.pop() {
        let Ok(metadata) = std::fs::symlink_metadata(&new) else {
            continue;
        };
        if metadata.is_dir() {
            let Ok(listing) = std::fs::read_dir(&new) else {
                continue;
            };
            for child in listing.filter_map(Result::ok) {
                *seen += 1;
                if *seen > MOST_FOLLOWED {
                    return files;
                }
                folders.push((old.join(child.file_name()), child.path()));
            }
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        let (Ok(old_uri), Ok(new_uri)) = (
            glib::filename_to_uri(&old, None),
            glib::filename_to_uri(&new, None),
        ) else {
            continue;
        };
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|time| time.as_secs());
        let Some(modified) = modified else {
            continue;
        };
        files.push((
            old_uri.to_string(),
            Version {
                uri: new_uri.to_string(),
                modified,
                size: Some(metadata.len()),
            },
        ));
    }
    files
}

thread_local! {
    /// When a view last asked for a preview.
    static LAST_PREVIEW_ASKED: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
    /// Set while a check for unused previews is due.
    static FORGETTING_PREVIEWS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Notes that a view asked for a preview now. The previews kept for rows
/// and tiles scrolled back into view are dropped once no view asked for
/// one for [`picture_guard::unused_limit`]: the rows shown hold their own,
/// and the photos of a folder left long ago, up to [`MEMORY_BUDGET`] of
/// previews and 48 tile-sized pictures, are otherwise held for as long as
/// the app runs. A folder visited again after that reads them back from
/// the shared cache, in batches ([`reader`]).
fn note_kept_previews_used() {
    LAST_PREVIEW_ASKED.set(Some(std::time::Instant::now()));
    if !FORGETTING_PREVIEWS.replace(true) {
        forget_kept_previews_after(picture_guard::unused_limit());
    }
}

/// Drops the kept previews after `delay`, unless a view asked for one
/// since, which puts it off.
fn forget_kept_previews_after(delay: std::time::Duration) {
    glib::timeout_add_local_once(delay, || {
        let limit = picture_guard::unused_limit();
        let unused = LAST_PREVIEW_ASKED.get().map_or(limit, |asked| asked.elapsed());
        if unused < limit {
            forget_kept_previews_after(limit.saturating_sub(unused));
            return;
        }
        FORGETTING_PREVIEWS.set(false);
        forget_kept_previews();
    });
}

/// Lets the next preview start a check for unused previews with the
/// limit as it is now, for tests that shorten it.
#[cfg(test)]
pub(crate) fn restart_unused_check() {
    FORGETTING_PREVIEWS.set(false);
}

/// Lets go of every kept preview and sharp picture.
fn forget_kept_previews() {
    MEMORY.with_borrow_mut(Memory::clear);
    SHARP_KEPT.with_borrow_mut(Vec::clear);
}

/// How many previews and sharp pictures are kept now, for tests.
#[cfg(test)]
pub(crate) fn kept_preview_count() -> usize {
    MEMORY.with_borrow(Memory::len) + SHARP_KEPT.with_borrow(Vec::len)
}

/// How many sharp pictures are kept for a tile scrolled back into view.
const SHARP_KEPT_COUNT: usize = 48;

/// The photo at `request`'s item, a file on this computer's own disks,
/// turned upright and shrunk to cover exactly `width` by `height` device
/// pixels, cut to them from the middle: Gallery's sharp picture of a tile
/// on screen, drawn pixel for pixel (HOME-033). The 128- and 256-pixel
/// thumbnails of the cache, and gdk-pixbuf's own shrinking while it
/// decodes (one bilinear step, which smears small print when it shrinks
/// a screenshot five times), would draw it blurred; so a picture is
/// decoded at full size, or a JPEG at no less than twice the size, by
/// its decoder's own exact halving, and shrunk once with gdk-pixbuf's
/// best filter. `request.pixels` is the edge of the square the photo must
/// cover. `None` for an item on a network or not a local file, or one the
/// picture limits refuse; it keeps its thumbnail. Dropping the future
/// cancels the decode.
pub(crate) async fn sharp_picture(request: PreviewRequest, width: i32, height: i32) -> Option<gdk::Texture> {
    let key = (
        request.uri.clone(),
        request.modified,
        (width << 16) | (height & 0xffff),
    );
    let kept = SHARP_KEPT.with_borrow(|kept| {
        kept.iter()
            .find(|(seen, _)| *seen == key)
            .map(|(_, texture)| texture.clone())
    });
    note_kept_previews_used();
    if let Some(texture) = kept {
        return Some(texture);
    }
    gio::File::for_uri(&request.uri).path()?;
    if is_on_network(&request.uri, None).await {
        return None;
    }
    let uri = request.uri.clone();
    // A JPEG's decoder halves it exactly and cheaply; its last step to the
    // size asked is a short bilinear one, so it is asked for twice the size.
    let decode_edge = if request.content_type == "image/jpeg" {
        request.pixels.saturating_mul(2)
    } else {
        SHARP_FULL_SIZE
    };
    let texture = DECODING
        .with(Rc::clone)
        .blocking(move |cancel| {
            let file = gio::File::for_uri(&uri);
            let decoded =
                picture_guard::decode_to_fit(&file, decode_edge, decode_edge, Limits::DISPLAY, Some(cancel))
                    .ok()?;
            let shrunk = shrink_to_cover(&decoded.pixbuf, width, height)?;
            Some(gdk::Texture::for_pixbuf(&shrunk))
        })
        .await
        .flatten()?;
    SHARP_KEPT.with_borrow_mut(|kept| {
        if kept.len() >= SHARP_KEPT_COUNT {
            kept.remove(0);
        }
        kept.push((key, texture.clone()));
    });
    note_kept_previews_used();
    Some(texture)
}

/// The edge a picture other than a JPEG is decoded within for Gallery's
/// sharp picture: its own size, under the picture limits.
const SHARP_FULL_SIZE: i32 = 16_384;

/// `picture` shrunk with gdk-pixbuf's best filter until it just covers
/// `width` by `height`, then cut to that from the middle; a picture smaller
/// than that is kept at its own size, cut to the tile's shape.
pub(crate) fn shrink_to_cover(picture: &Pixbuf, width: i32, height: i32) -> Option<Pixbuf> {
    let (own_width, own_height) = (f64::from(picture.width()), f64::from(picture.height()));
    let factor = (f64::from(width) / own_width).max(f64::from(height) / own_height);
    let shrunk = if factor < 1.0 {
        #[expect(clippy::cast_possible_truncation, reason = "picture sizes are small")]
        let (cover_width, cover_height) = (
            ((own_width * factor).round() as i32).max(width).max(1),
            ((own_height * factor).round() as i32).max(height).max(1),
        );
        picture.scale_simple(cover_width, cover_height, gtk::gdk_pixbuf::InterpType::Hyper)?
    } else {
        picture.clone()
    };
    let (cut_width, cut_height) = (width.min(shrunk.width()), height.min(shrunk.height()));
    let (left, top) = (
        (shrunk.width() - cut_width) / 2,
        (shrunk.height() - cut_height) / 2,
    );
    shrunk.new_subpixbuf(left, top, cut_width, cut_height).copy()
}

/// Whether the item at `uri`, or the file `target` it stands for, is on a
/// network file system, asked off the main thread, which a server that
/// does not answer would hold up. A URI without a local path is left to
/// [`PreviewPolicy::allows`], which tells servers and devices by their
/// URIs.
async fn is_on_network(uri: &str, target: Option<&str>) -> bool {
    let folders = local_folders(uri, target);
    if folders.is_empty() {
        return false;
    }
    // The tiles of a folder bound in one layout ask at once: one asks the
    // file system, the others wait for its answer.
    loop {
        if let Some(known) = known_answer(&folders, Some(NETWORK_ANSWER_LIFE)) {
            return known;
        }
        let waiting = ASKING.with_borrow_mut(|asking| {
            let waiters = asking.get_mut(&folders)?;
            let (sender, receiver) = async_channel::bounded(1);
            waiters.push(sender);
            Some(receiver)
        });
        let Some(receiver) = waiting else {
            let asking = Asking::start(folders.clone());
            let answer = ask_file_systems(folders).await;
            asking.answer(answer);
            return answer;
        };
        // A request that asked and was dropped before the answer came
        // hands the asking on.
        if let Ok(answer) = receiver.recv().await {
            return answer;
        }
    }
}

/// A request asking the file systems of its folders, which other
/// requests for the same folders wait for. Dropped before it answered, as
/// when its tile is bound to another item, it lets them ask themselves.
struct Asking(Option<Vec<PathBuf>>);

impl Asking {
    fn start(folders: Vec<PathBuf>) -> Self {
        ASKING.with_borrow_mut(|asking| asking.insert(folders.clone(), Vec::new()));
        Self(Some(folders))
    }

    /// Hands `answer` to the requests waiting.
    fn answer(mut self, answer: bool) {
        let Some(folders) = self.0.take() else {
            return;
        };
        for waiter in ASKING
            .with_borrow_mut(|asking| asking.remove(&folders))
            .unwrap_or_default()
        {
            let _ = waiter.try_send(answer);
        }
    }
}

impl Drop for Asking {
    fn drop(&mut self) {
        if let Some(folders) = self.0.take() {
            // The waiters' senders are dropped with the entry.
            ASKING.with_borrow_mut(|asking| asking.remove(&folders));
        }
    }
}

/// Asks the file systems of `folders` whether one of them is on the
/// network, off the main thread, and remembers each answer.
async fn ask_file_systems(folders: Vec<PathBuf>) -> bool {
    let asked = folders.clone();
    let answers = gio::spawn_blocking(move || {
        asked
            .iter()
            .map(|folder| network_fs::is_on_network_filesystem(folder))
            .collect::<Vec<_>>()
    })
    .await;
    let Ok(answers) = answers else {
        return true;
    };
    NETWORK_FOLDERS.with_borrow_mut(|known| {
        if known.len() > 256 {
            known.clear();
        }
        let now = Instant::now();
        for (folder, answer) in folders.into_iter().zip(&answers) {
            known.insert(folder, (*answer, now));
        }
    });
    answers.into_iter().any(|answer| answer)
}

/// The folders of the local files `uri` and `target` name: each file's
/// own folder, whose file system is the file's, so one answer serves
/// every tile of a folder.
fn local_folders(uri: &str, target: Option<&str>) -> Vec<PathBuf> {
    std::iter::once(uri)
        .chain(target)
        .filter_map(|uri| gio::File::for_uri(uri).path())
        .map(|path| path.parent().map_or_else(|| path.clone(), Path::to_path_buf))
        .collect()
}

/// Whether one of `folders` is on a network file system, as asked within
/// `life` (or at any time without it); `None` when any of them was not.
fn known_answer(folders: &[PathBuf], life: Option<Duration>) -> Option<bool> {
    NETWORK_FOLDERS.with_borrow(|known| {
        let mut on_network = false;
        for folder in folders {
            let (answer, asked) = known.get(folder)?;
            if life.is_some_and(|life| asked.elapsed() > life) {
                return None;
            }
            on_network |= *answer;
        }
        Some(on_network)
    })
}

/// Whether `request`'s item was found on this computer's own disks, however
/// long ago: a preview kept in memory was shown under the same rule, so it
/// may be shown again at once.
fn known_local(request: &PreviewRequest) -> bool {
    let folders = local_folders(&request.uri, request.target_uri.as_deref());
    folders.is_empty() || known_answer(&folders, None) == Some(false)
}

/// The thumbnail of `request`'s item: read from the shared cache, else
/// made and written there.
async fn look_up_or_make(request: &PreviewRequest) -> Option<gdk::Texture> {
    let cache_dir = glib::user_cache_dir();
    // The cache's own pictures keep their icons: a thumbnail of a
    // thumbnail would grow the cache from itself, so nothing is read, made
    // or asked of the service and the desktop's programs for them.
    if writer::inside_cache(&cache_dir, &request.uri) {
        return None;
    }
    let Some(version) = request.version() else {
        // Without the file's time no thumbnail can be told current: GIO
        // looks one up by the file's own.
        return thumbnail_known_to_gio(request).await;
    };
    let failed_key = (request.uri.clone(), request.modified);
    if FAILED.with_borrow(|failed| failed.contains(&failed_key)) {
        return None;
    }
    let flavor = ThumbnailFlavor::for_pixels(request.pixels);
    // GNOME's thumbnailer marks a document it could not draw, as an Office
    // file without a picture of its own; LibreOffice may draw it all the
    // same, so only the app's own failures count for one.
    let gnome_failures_count = !is_document(request);
    match reader::read(
        cache_dir.clone(),
        version.clone(),
        gnome_failures_count,
        flavor,
        request.pixels,
    )
    .await
    {
        Cached::Hit(texture) => return Some(texture),
        Cached::Failed => {
            FAILED.with_borrow_mut(|failed| failed.insert(failed_key));
            return None;
        }
        Cached::Missing => {}
    }
    match make_thumbnail(request, &cache_dir, flavor, &version).await {
        Ok(Some(made)) => {
            reader::note_written(&request.uri, flavor);
            Some(made.picture)
        }
        // The thumbnailer service wrote it to the cache.
        Ok(None) => {
            match reader::read(cache_dir, version, gnome_failures_count, flavor, request.pixels).await {
                Cached::Hit(texture) => Some(texture),
                Cached::Failed | Cached::Missing => None,
            }
        }
        Err(missing) => {
            if matches!(missing, Missing::Failed | Missing::Broken) {
                FAILED.with_borrow_mut(|failed| failed.insert(failed_key));
            }
            if missing == Missing::Broken {
                record_failure(cache_dir, version);
            }
            None
        }
    }
}

/// Records in the shared cache that no thumbnail can be made of `version`
/// of its file, so neither this app nor a later run tries again until the
/// file changes. Runs off the main thread.
fn record_failure(cache_dir: PathBuf, version: Version) {
    // Test safety: tests write only into a private cache.
    #[cfg(test)]
    if !cache_dir.starts_with(std::env::temp_dir()) {
        return;
    }
    trace::event("fail-written", &version.uri);
    gio::spawn_blocking(move || {
        let _ = thumbnail_cache::record_failure(&cache_dir, &version);
    });
}

/// The cached thumbnail of an item whose time is not known, as GIO finds
/// it by the file's own time, decoded.
async fn thumbnail_known_to_gio(request: &PreviewRequest) -> Option<gdk::Texture> {
    trace::event("gio-query", &request.uri);
    let file = gio::File::for_uri(&request.uri);
    let info = file
        .query_info_future(
            THUMBNAIL_ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::LOW,
        )
        .await
        .ok()?;
    // A thumbnail smaller than the one asked for is not shown stretched:
    // GIO reports whichever size it found.
    let wanted = ThumbnailFlavor::for_pixels(request.pixels);
    let CachedThumbnail::Valid(path) = cached_thumbnail_for(&info, wanted) else {
        return None;
    };
    let pixels = request.pixels;
    DECODING
        .with(Rc::clone)
        .blocking(move |_| decode(&path, pixels))
        .await
        .flatten()
}

/// Why no thumbnail was made of an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Missing {
    /// None could be made, and this version of the item is not tried
    /// again in this run.
    Failed,
    /// None could be made because the file is one no thumbnailer reads
    /// (a damaged picture, a document its program gave up on): this version
    /// is not tried again in this run, nor, as the shared cache records the
    /// failure, in another.
    Broken,
    /// The user turned the desktop's thumbnailers off for its type, which
    /// they may turn back on at any time.
    TurnedOff,
    /// Decoding the picture took longer than its time limit; it is tried
    /// again the next time it is shown.
    Later,
}

/// Makes the missing thumbnail of the item: by the thumbnailer service
/// when it supports the type (`Ok(None)`: it wrote the thumbnail to the
/// cache), else by gdk-pixbuf for a picture, else by the desktop's own
/// thumbnailer for the type when it installs one.
async fn make_thumbnail(
    request: &PreviewRequest,
    cache_dir: &Path,
    flavor: ThumbnailFlavor,
    version: &Version,
) -> Result<Option<Made>, Missing> {
    if let Some(service) = service::service().await {
        if service.supports(&request.uri, &request.content_type) && service.makes(flavor) {
            let _slot = making(request).take_in_turn(&|| request.on_screen.get()).await;
            let made = service.make(&request.uri, &request.content_type, flavor).await;
            if made {
                return Ok(None);
            }
            // The service draws few documents (an OpenDocument's own
            // thumbnail at best): one it could not draw is LibreOffice's.
            if !is_document(request) {
                return Err(Missing::Failed);
            }
            if keeps_type_icon(request) {
                return Err(Missing::Later);
            }
        }
    }
    // Test safety: tests write thumbnails only into a private cache.
    #[cfg(test)]
    if !cache_dir.starts_with(std::env::temp_dir()) {
        return Err(Missing::Failed);
    }
    make_thumbnail_with(request, cache_dir, None, flavor, version.modified)
        .await
        .map(Some)
}

/// The thumbnail `request` asks for: the app decodes a picture itself
/// ([`pixbuf_may_read`]), which costs less than a program of the
/// desktop's, and a program of the desktop's makes it when the type is not
/// a picture or gdk-pixbuf cannot read it (a type without its loader). `catalogue` is the desktop's own
/// list unless a test hands its own.
async fn make_thumbnail_with(
    request: &PreviewRequest,
    cache_dir: &Path,
    catalogue: Option<Arc<Catalogue>>,
    flavor: ThumbnailFlavor,
    modified: u64,
) -> Result<Made, Missing> {
    if PIXBUF_TYPES.with(|declared| pixbuf_may_read(&request.content_type, declared)) {
        match by_decoding(request, cache_dir, modified, flavor).await {
            Ok(made) => return Ok(made),
            // Only a picture the app cannot read leaves room for another
            // thumbnailer, and one larger than the app decodes itself for
            // a thumbnailer whose memory is limited, as a scan of many
            // megapixels still deserves its preview. One it read and the
            // cache refused is refused for every thumbnailer, one damaged
            // or slow would be read the same way by the program, and a
            // dropped request has nobody left to show a picture to:
            // forking a program for any of them would only cost one.
            Err(writer::Failure::Unreadable) => {}
            Err(writer::Failure::TooLarge) => {
                return by_thumbnailer(request, cache_dir, modified, flavor, catalogue, true).await;
            }
            // Asked again when it is shown again.
            Err(writer::Failure::TimedOut | writer::Failure::Cancelled) => return Err(Missing::Later),
            // A damaged picture is recorded as such in the cache.
            Err(writer::Failure::Refused) => return Err(Missing::Broken),
            Err(writer::Failure::Unwritable) => return Err(Missing::Failed),
        }
    }
    match by_thumbnailer(request, cache_dir, modified, flavor, catalogue, false).await {
        // No program of the desktop's drew the document, or one ran and
        // gave up on it, and it is on this computer: LibreOffice may draw
        // it.
        Err(Missing::Failed | Missing::Broken) if is_document(request) && !request.on_network => {
            if keeps_type_icon(request) {
                // Asked again at a size it is drawn for.
                return Err(Missing::Later);
            }
            by_document(request, cache_dir, modified, flavor).await
        }
        outcome => outcome,
    }
}

/// Whether `request`'s item keeps its type icon at its size: an office
/// document in a row smaller than [`SMALLEST_DOCUMENT_THUMBNAIL`].
fn keeps_type_icon(request: &PreviewRequest) -> bool {
    request.pixels < SMALLEST_DOCUMENT_THUMBNAIL && is_document(request)
}

/// Whether `request` is for an office document ([`documents::is_document`]).
fn is_document(request: &PreviewRequest) -> bool {
    let name = request.uri.rsplit('/').next().unwrap_or_default();
    let name = glib::Uri::unescape_string(name, None::<&str>).map_or_else(|| name.to_owned(), Into::into);
    documents::is_document(&request.content_type, &name)
}

/// The thumbnail of the office document `request` asks for, its first
/// page drawn from `LibreOffice`'s PDF ([`documents::write_thumbnail`]),
/// one document at a time; a request dropped, or turned away while too
/// many wait, is asked again the next time the item is shown.
async fn by_document(
    request: &PreviewRequest,
    cache_dir: &Path,
    modified: u64,
    flavor: ThumbnailFlavor,
) -> Result<Made, Missing> {
    let job = thumbnailers::Job {
        cache_dir: cache_dir.to_path_buf(),
        uri: request.uri.clone(),
        content_type: request.content_type.clone(),
        modified,
        flavor,
        size: request.size,
        shown: request.pixels,
    };
    let made = DOCUMENT_MAKING
        .with(Rc::clone)
        .blocking_in_turn(&|| request.on_screen.get(), move |cancel| {
            documents::write_thumbnail(&job, cancel)
        })
        .await;
    match made {
        Some(Ok(made)) => Ok(made),
        None | Some(Err(documents::Failure::Cancelled)) => Err(Missing::Later),
        Some(Err(
            documents::Failure::Unavailable
            | documents::Failure::Refused
            | documents::Failure::Failed
            | documents::Failure::TimedOut,
        )) => Err(Missing::Failed),
    }
}

/// The thumbnail of `request` decoded in the app, or why there is none:
/// [`writer::Failure::Unreadable`] leaves the type to another thumbnailer,
/// the others do not.
async fn by_decoding(
    request: &PreviewRequest,
    cache_dir: &Path,
    modified: u64,
    flavor: ThumbnailFlavor,
) -> Result<Made, writer::Failure> {
    let source = Source {
        uri: request.uri.clone(),
        modified,
        size: request.size,
        mime_type: Some(request.content_type.clone()),
        shown: request.pixels,
    };
    let cache_dir = cache_dir.to_path_buf();
    let decoded = making(request)
        .blocking_in_turn(&|| request.on_screen.get(), move |cancel| {
            let made = writer::write_thumbnail(&cache_dir, &source, flavor, cancel);
            // A refusal is also what a decoder that died says, as one that
            // used up its processor time over many pictures: the picture
            // is tried once more, in another decoder, before it is taken
            // as damaged and recorded as such for good.
            if made.as_ref().err() == Some(&writer::Failure::Refused) {
                return writer::write_thumbnail(&cache_dir, &source, flavor, cancel);
            }
            made
        })
        .await;
    // A worker that never ran belongs to a request already dropped.
    decoded.unwrap_or(Err(writer::Failure::Cancelled))
}

/// The thumbnail of `request` made by the desktop's thumbnailer for its
/// type, or [`Missing::TurnedOff`] when the user turned them off for it
/// and [`Missing::Failed`] when none makes it or it failed. With
/// `oversized`, a picture too large for the app to decode, only a
/// thumbnailer whose memory is limited runs.
async fn by_thumbnailer(
    request: &PreviewRequest,
    cache_dir: &Path,
    modified: u64,
    flavor: ThumbnailFlavor,
    catalogue: Option<Arc<Catalogue>>,
    oversized: bool,
) -> Result<Made, Missing> {
    let job = thumbnailers::Job {
        cache_dir: cache_dir.to_path_buf(),
        uri: request.uri.clone(),
        content_type: request.content_type.clone(),
        modified,
        flavor,
        size: request.size,
        shown: request.pixels,
    };
    making(request)
        .blocking_in_turn(&|| request.on_screen.get(), move |cancel| {
            // The desktop's entries are files, read in the worker rather
            // than on the thread that asked, and once per run; what the
            // user turned off is read for each request, so a change takes
            // effect at once, and with every program off the entries are
            // not even read. A test hands its own, with nothing off.
            let installed = if let Some(catalogue) = catalogue {
                catalogue
            } else {
                let disabled = thumbnailers::Disabled::of_session();
                if disabled.covers(&job.content_type) {
                    return Err(Missing::TurnedOff);
                }
                Arc::new(thumbnailers::installed().with_disabled(disabled))
            };
            if oversized && !installed.limits_memory() {
                return Err(Missing::Failed);
            }
            thumbnailers::make_thumbnail(&job, &installed, cancel).map_err(|not_made| match not_made {
                thumbnailers::NotMade::Failed => Missing::Broken,
                thumbnailers::NotMade::NoProgram => Missing::Failed,
                thumbnailers::NotMade::Interrupted => Missing::Later,
            })
        })
        .await
        .unwrap_or(Err(Missing::Later))
}

/// The thumbnail at `path`, scaled to fit `pixels` square; `None` for a
/// file in the cache that is not a sensible thumbnail (more than the
/// largest standard size, damaged, or slow to read), so the item keeps
/// its icon.
fn decode(path: &std::path::Path, pixels: i32) -> Option<gdk::Texture> {
    trace::event("cache-read", &path.display().to_string());
    let file = gio::File::for_path(path);
    let decoded = picture_guard::decode_to_fit(&file, pixels, pixels, Limits::THUMBNAIL_FILE, None).ok()?;
    Some(gdk::Texture::for_pixbuf(&decoded.pixbuf))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use gtk::gdk_pixbuf::Colorspace;
    use ox_core::entry::thumbnail_file;

    use super::*;

    /// Scrolling through thousands of photos at Extra large icons on a
    /// 200 % screen kept up to 512 previews of up to 512 × 512 pixels,
    /// a megabyte each; the kept previews now stay within a budget in
    /// bytes as well as in number.
    ///
    /// parity: VIEW-057
    #[gtk::test]
    fn the_kept_previews_stay_within_their_budget() {
        let picture = |edge: i32| {
            let stride = usize::try_from(edge * 4).expect("an edge");
            let bytes =
                glib::Bytes::from_owned(vec![0x80_u8; stride * usize::try_from(edge).expect("an edge")]);
            gdk::MemoryTexture::new(edge, edge, gdk::MemoryFormat::R8g8b8a8, &bytes, stride)
                .upcast::<gdk::Texture>()
        };
        forget_kept_previews();
        for number in 0..120 {
            MEMORY.with_borrow_mut(|memory| {
                memory.put(
                    (format!("file:///Photos/Harbor%20{number}.jpg"), 512),
                    picture(512),
                    Some(1),
                    None,
                );
            });
            let bytes = MEMORY.with_borrow(Memory::bytes);
            assert!(
                bytes <= MEMORY_BUDGET,
                "{bytes} bytes kept after {number} previews"
            );
        }
        assert!(MEMORY.with_borrow(Memory::len) > 0, "previews are still kept");
        forget_kept_previews();
        assert_eq!(kept_preview_count(), 0);
    }

    /// A picture whose type no loader declares is still decoded, and a
    /// file that is no picture never reaches gdk-pixbuf.
    ///
    /// parity: VIEW-057
    #[test]
    fn pictures_a_loader_does_not_declare_are_tried() {
        // gdk-pixbuf's dds loader declares `image/x-dds` alone, while GIO
        // names the same files `image/vnd.ms-dds`.
        let declared: HashSet<String> = ["image/jpeg", "image/x-dds"]
            .into_iter()
            .map(str::to_owned)
            .collect();

        assert!(pixbuf_may_read("image/vnd.ms-dds", &declared));
        assert!(pixbuf_may_read("image/jpeg", &declared));
        assert!(!pixbuf_may_read("application/pdf", &declared));
        assert!(!pixbuf_may_read("video/mp4", &declared));
        assert!(!pixbuf_may_read("text/plain", &declared));
        // A loader's own declaration counts too, whatever its kind.
        let font = HashSet::from(["application/x-font-ttf".to_owned()]);
        assert!(pixbuf_may_read("application/x-font-ttf", &font));
    }

    /// Previews follow Settings: off, only in folders on this computer
    /// unless network previews are on (they are off by default, as
    /// Windows 11 File Explorer shows only icons in network folders), and
    /// not for files over the limit.
    ///
    /// parity: VIEW-058
    #[test]
    fn previews_follow_the_settings() {
        let mut picture = crate::test_support::file_entry("photo.jpg");
        picture.content_type = Some("image/jpeg".to_owned());
        picture.size = Some(PREVIEW_SIZE_LIMIT + 1);
        let options = ViewOptions::default();

        assert!(
            !PreviewPolicy::for_folder(&options, false).allows(&picture),
            "too large"
        );
        picture.size = Some(4096);
        assert!(PreviewPolicy::for_folder(&options, false).allows(&picture));
        assert!(
            !PreviewPolicy::for_folder(&options, true).allows(&picture),
            "a network folder"
        );
        let off = ViewOptions {
            show_previews: false,
            ..ViewOptions::default()
        };
        assert!(!PreviewPolicy::for_folder(&off, false).allows(&picture));
        let no_pictures = ViewOptions {
            preview_pictures: false,
            ..ViewOptions::default()
        };
        assert!(!PreviewPolicy::for_folder(&no_pictures, false).allows(&picture));

        // A file of a server or a phone listed in a folder of this
        // computer, as a recent item or a search result is, keeps its icon
        // unless network previews are on.
        let local = PreviewPolicy::for_folder(&options, false);
        let mut shared = picture.clone();
        shared.uri = "smb://fileserver.example/team/photo.jpg".to_owned();
        assert!(!local.allows(&shared), "a share");
        // A Recently used files row keeps its file in its metadata, not in
        // its target, which stays empty for a file.
        let mut recent = picture.clone();
        recent.uri = "recent:///3a1b2c".to_owned();
        recent.target_uri = None;
        recent.meta.recent_target = Some("mtp://Example_Phone/DCIM/photo.jpg".to_owned());
        assert!(!local.allows(&recent), "a recent item on a phone");
        let mut recent_share = recent.clone();
        recent_share.meta.recent_target = Some("smb://fileserver.example/team/photo.jpg".to_owned());
        assert!(!local.allows(&recent_share), "a recent item on a share");
        // One on this computer is previewed, and its file is what the
        // network file system check looks at.
        let mut recent_local = recent.clone();
        recent_local.meta.recent_target = Some("file:///home/demo/Pictures/photo.jpg".to_owned());
        let request = local.request(&recent_local, 128).expect("a recent local picture");
        assert!(request.local_only);
        assert_eq!(
            request.target_uri.as_deref(),
            Some("file:///home/demo/Pictures/photo.jpg")
        );
        let everywhere = ViewOptions {
            remote_previews: true,
            ..ViewOptions::default()
        };
        assert!(PreviewPolicy::for_folder(&everywhere, true).allows(&picture));
        assert!(PreviewPolicy::for_folder(&everywhere, false).allows(&shared));
        assert!(PreviewPolicy::for_folder(&everywhere, false).allows(&recent));
        // With them on, an item of a server or a device asks for its
        // thumbnail on the network's own slots; one of this computer does
        // not, until its file system says otherwise.
        let on_network = PreviewPolicy::for_folder(&everywhere, true)
            .request(&shared, 128)
            .expect("a picture on a share");
        assert!(on_network.on_network && !on_network.local_only);
        let at_home = PreviewPolicy::for_folder(&everywhere, false)
            .request(&picture, 128)
            .expect("a picture of this computer");
        assert!(!at_home.on_network);
    }

    /// A file of a folder known to be on the network, such as a kernel
    /// CIFS mount whose files have `file:` URIs, asks for its thumbnail on
    /// the network's own slots from the start, with no file system probe
    /// of its own that a mount that stopped answering would hold up.
    ///
    /// parity: VIEW-057, VIEW-058
    #[test]
    fn a_file_of_a_mounted_share_asks_for_the_networks_slots() {
        let mut picture = crate::test_support::file_entry("photo.jpg");
        picture.content_type = Some("image/jpeg".to_owned());
        picture.uri = "file:///mnt/nas/Photos/photo.jpg".to_owned();
        let everywhere = ViewOptions {
            remote_previews: true,
            ..ViewOptions::default()
        };

        let request = PreviewPolicy::for_folder(&everywhere, true)
            .request(&picture, 128)
            .expect("a picture on the mounted share");

        assert!(request.on_network);
        assert!(!request.local_only);
    }

    /// A thumbnailer of the test's own, and the file it leaves when it is
    /// started: it hands back `handed.png`, so a test tells its picture
    /// from the app's, and its start from no program run at all.
    fn fake_thumbnailer(folder: &Path) -> (Catalogue, PathBuf) {
        let started = folder.join("started");
        let handed = folder.join("handed.png");
        let picture = Pixbuf::new(Colorspace::Rgb, false, 8, 64, 64).expect("a picture");
        picture.fill(0xcc_3333ff);
        picture.savev(&handed, "png", &[]).expect("the picture");
        let script = folder.join("ox-test-thumbnailer");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\ntouch '{}'\ncp '{}' \"$2\"\n",
                started.display(),
                handed.display()
            ),
        )
        .expect("the script");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("the mode");
        let command = [script.to_str().expect("a path"), "%i", "%o", "%s"];
        let thumbnailer = thumbnailers::Thumbnailer::for_test(&command, &["image/png"]);
        (Catalogue::for_test(vec![thumbnailer]), started)
    }

    /// The request for the file at `path`, of `content_type` at the normal
    /// size.
    fn request_of(path: &Path, content_type: &str) -> PreviewRequest {
        PreviewRequest {
            uri: gio::File::for_path(path).uri().to_string(),
            content_type: content_type.to_owned(),
            modified: Some(1_700_000_000),
            size: None,
            pixels: 128,
            local_only: false,
            target_uri: None,
            on_network: false,
            on_screen: OnScreen::default(),
        }
    }

    /// A picture shown in Details first gets a 128-pixel thumbnail; shown
    /// at Extra large icons next (256 pixels), or at Large icons on a
    /// screen at 200 %, it gets one made at that size, never the small one
    /// stretched and blurred, which GIO reports as the item's thumbnail.
    ///
    /// parity: VIEW-057
    #[gtk::test]
    fn a_larger_view_does_not_stretch_a_smaller_thumbnail() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let item = folder.path().join("photo.png");
        let picture = Pixbuf::new(Colorspace::Rgb, false, 8, 1200, 900).expect("a picture");
        picture.fill(0x3366_99ff);
        picture.savev(&item, "png", &[]).expect("the picture");
        let modified = fs::metadata(&item)
            .and_then(|metadata| metadata.modified())
            .expect("its time")
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after 1970")
            .as_secs();
        let request = |pixels| PreviewRequest {
            modified: Some(modified),
            pixels,
            ..request_of(&item, "image/png")
        };
        let context = glib::MainContext::default();
        let cache = glib::user_cache_dir();
        context
            .block_on(preview(request(16)))
            .expect("a thumbnail for Details");
        assert!(
            thumbnail_file(&cache, &request(16).uri, ThumbnailFlavor::Normal).exists(),
            "a normal thumbnail for Details"
        );
        for (pixels, flavor) in [(256, ThumbnailFlavor::Large), (512, ThumbnailFlavor::XLarge)] {
            let texture = context
                .block_on(preview(request(pixels)))
                .expect("the larger preview");
            assert!(
                thumbnail_file(&cache, &request(pixels).uri, flavor).exists(),
                "a {} thumbnail made for {pixels} pixels",
                flavor.as_str()
            );
            assert_eq!(texture.width(), pixels, "drawn from a {pixels}-pixel thumbnail");
        }
    }

    /// A share that does not answer holds only the network's own slots:
    /// with every one of them taken, as by reads of a server that never
    /// return, a picture of this computer still gets its thumbnail, and a
    /// file of the share waits for a network slot.
    ///
    /// parity: VIEW-057, VIEW-058
    #[gtk::test]
    fn a_share_that_does_not_answer_leaves_local_thumbnails_their_slots() {
        use std::future::Future;
        use std::task::{Context, Waker};

        let folder = tempfile::tempdir().expect("a temporary folder");
        let (thumbnailers, started) = fake_thumbnailer(folder.path());
        let thumbnailers = Arc::new(thumbnailers);
        let cache = folder.path().join("cache");
        let picture = Pixbuf::new(Colorspace::Rgb, false, 8, 400, 200).expect("a picture");
        picture.fill(0x3366_99ff);
        let item = folder.path().join("photo.png");
        picture.savev(&item, "png", &[]).expect("the picture");
        let shared = folder.path().join("shared.png");
        picture.savev(&shared, "png", &[]).expect("the picture");
        let on_share = PreviewRequest {
            on_network: true,
            ..request_of(&shared, "image/png")
        };
        let context = glib::MainContext::default();
        let network = making(&on_share);
        let hung: Vec<_> = (0..NETWORK_MAKING_AT_ONCE)
            .map(|_| context.block_on(network.take()))
            .collect();

        let local = request_of(&item, "image/png");
        let written = context
            .block_on(make_thumbnail_with(
                &local,
                &cache,
                Some(Arc::clone(&thumbnailers)),
                ThumbnailFlavor::Normal,
                1_700_000_000,
            ))
            .expect("a local thumbnail while the share hangs")
            .path;
        assert_eq!(
            written,
            thumbnail_file(&cache, &local.uri, ThumbnailFlavor::Normal)
        );

        let mut waiting = Box::pin(make_thumbnail_with(
            &on_share,
            &cache,
            Some(thumbnailers),
            ThumbnailFlavor::Normal,
            1_700_000_000,
        ));
        assert!(
            waiting
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending(),
            "the share's file waits for a network slot"
        );
        drop(hung);
        let written = context
            .block_on(waiting)
            .expect("its thumbnail once a slot is free")
            .path;
        assert_eq!(
            written,
            thumbnail_file(&cache, &on_share.uri, ThumbnailFlavor::Normal)
        );
        assert!(!started.exists(), "the app decoded both pictures itself");
    }

    /// A picture the app cannot read — here a PNG that is no picture, as a
    /// type whose gdk-pixbuf loader is missing looks from the app — is
    /// made by the desktop's thumbnailer for its type, and the app writes
    /// it into the cache as it writes its own.
    ///
    /// parity: VIEW-057
    #[gtk::test]
    fn a_picture_the_app_cannot_read_falls_back_to_the_desktop_thumbnailer() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let (thumbnailers, started) = fake_thumbnailer(folder.path());
        let cache = folder.path().join("cache");
        let item = folder.path().join("photo.png");
        fs::write(&item, b"not a picture").expect("the item");
        let request = request_of(&item, "image/png");
        // Reading it is what fails, which is the one failure that leaves
        // room for another thumbnailer.
        assert_eq!(
            writer::write_thumbnail(
                &cache,
                &writer::Source {
                    uri: request.uri.clone(),
                    modified: 1_700_000_000,
                    size: None,
                    mime_type: None,
                    shown: 128,
                },
                ThumbnailFlavor::Normal,
                &gio::Cancellable::new(),
            )
            .map(|made| made.path),
            Err(writer::Failure::Unreadable)
        );

        let written = glib::MainContext::default()
            .block_on(make_thumbnail_with(
                &request,
                &cache,
                Some(Arc::new(thumbnailers)),
                ThumbnailFlavor::Normal,
                1_700_000_000,
            ))
            .expect("a thumbnail")
            .path;

        assert!(started.exists(), "the desktop's thumbnailer made it");
        assert_eq!(
            written,
            thumbnail_file(&cache, &request.uri, ThumbnailFlavor::Normal)
        );
        let thumbnail = Pixbuf::from_file(&written).expect("a PNG");
        assert_eq!((thumbnail.width(), thumbnail.height()), (64, 64));
        assert_eq!(
            thumbnail.option("tEXt::Thumb::URI").as_deref(),
            Some(request.uri.as_str())
        );
    }

    /// A picture larger than the app decodes itself, as a scan of many
    /// megapixels is, is made by the desktop's thumbnailer for its type,
    /// whose memory is limited, rather than keeping its icon.
    ///
    /// parity: VIEW-057
    #[gtk::test]
    fn a_picture_too_large_to_decode_goes_to_a_limited_thumbnailer() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let (thumbnailers, started) = fake_thumbnailer(folder.path());
        let cache = folder.path().join("cache");
        let item = crate::picture_guard::huge_png(&folder.path().join("Scan.png"));
        let request = request_of(&item, "image/png");

        let written = glib::MainContext::default()
            .block_on(make_thumbnail_with(
                &request,
                &cache,
                Some(Arc::new(thumbnailers)),
                ThumbnailFlavor::Normal,
                1_700_000_000,
            ))
            .expect("a thumbnail")
            .path;

        assert!(started.exists(), "the desktop's thumbnailer made it");
        assert_eq!(
            written,
            thumbnail_file(&cache, &request.uri, ThumbnailFlavor::Normal)
        );
    }

    /// A picture the app reads itself is never handed to a program of the
    /// desktop's, which costs more than decoding it.
    ///
    /// parity: VIEW-057
    #[gtk::test]
    fn a_picture_the_app_reads_starts_no_thumbnailer() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let (thumbnailers, started) = fake_thumbnailer(folder.path());
        let cache = folder.path().join("cache");
        let item = folder.path().join("photo.png");
        let picture = Pixbuf::new(Colorspace::Rgb, false, 8, 400, 200).expect("a picture");
        picture.fill(0x3366_99ff);
        picture.savev(&item, "png", &[]).expect("the picture");
        let request = request_of(&item, "image/png");

        let written = glib::MainContext::default()
            .block_on(make_thumbnail_with(
                &request,
                &cache,
                Some(Arc::new(thumbnailers)),
                ThumbnailFlavor::Normal,
                1_700_000_000,
            ))
            .expect("a thumbnail")
            .path;

        assert!(!started.exists(), "the app decoded the picture itself");
        assert_eq!(
            written,
            thumbnail_file(&cache, &request.uri, ThumbnailFlavor::Normal)
        );
        let thumbnail = Pixbuf::from_file(&written).expect("a PNG");
        assert_eq!((thumbnail.width(), thumbnail.height()), (128, 64));
    }

    /// A picture of the cache's own folder is decoded but never written
    /// there, and no program of the desktop's is started to try what the
    /// cache refuses whatever makes it: browsing `~/.cache/thumbnails`
    /// itself forks nothing for its pictures.
    ///
    /// parity: VIEW-057
    #[gtk::test]
    fn a_picture_of_the_cache_itself_starts_no_thumbnailer() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let (thumbnailers, started) = fake_thumbnailer(folder.path());
        let cache = folder.path().join("cache");
        let inside = cache.join("thumbnails").join(ThumbnailFlavor::Normal.as_str());
        fs::create_dir_all(&inside).expect("the cache folder");
        let item = inside.join("photo.png");
        let picture = Pixbuf::new(Colorspace::Rgb, false, 8, 400, 200).expect("a picture");
        picture.fill(0x3366_99ff);
        picture.savev(&item, "png", &[]).expect("the picture");
        let request = request_of(&item, "image/png");
        assert_eq!(
            writer::write_thumbnail(
                &cache,
                &writer::Source {
                    uri: request.uri.clone(),
                    modified: 1_700_000_000,
                    size: None,
                    mime_type: None,
                    shown: 128,
                },
                ThumbnailFlavor::Normal,
                &gio::Cancellable::new(),
            )
            .map(|made| made.path),
            Err(writer::Failure::Unwritable),
            "the picture reads, but the cache refuses its thumbnail"
        );

        let made = glib::MainContext::default().block_on(make_thumbnail_with(
            &request,
            &cache,
            Some(Arc::new(thumbnailers)),
            ThumbnailFlavor::Normal,
            1_700_000_000,
        ));

        assert_eq!(
            made.map(|made| made.path),
            Err(Missing::Failed),
            "the cache holds no pictures of itself"
        );
        assert!(!started.exists(), "and no program is started for one");
    }

    /// The color of a texture's first pixel, as red, green and blue.
    fn first_pixel(texture: &gdk::Texture) -> (u8, u8, u8) {
        let mut bytes = vec![0_u8; usize::try_from(texture.width() * texture.height() * 4).expect("small")];
        texture.download(&mut bytes, usize::try_from(texture.width() * 4).expect("small"));
        // GDK's default layout: blue, green, red, alpha.
        (bytes[2], bytes[1], bytes[0])
    }

    /// A picture of 400 × 200 pixels of `color`, at `name` in `folder`.
    fn picture_at(folder: &Path, name: &str, color: u32) -> PathBuf {
        let path = folder.join(name);
        let pixels = Pixbuf::new(Colorspace::Rgb, false, 8, 400, 200).expect("a picture");
        pixels.fill(color);
        pixels.savev(&path, "png", &[]).expect("the picture");
        path
    }

    /// The private cache of the test run (check.py gives each test
    /// program its own `XDG_CACHE_HOME`), or `None` where the cache is not
    /// a temporary folder, so a test never writes into a real one.
    fn private_cache() -> Option<PathBuf> {
        let cache = glib::user_cache_dir();
        cache.starts_with(std::env::temp_dir()).then_some(cache)
    }

    /// A thumbnail another program left in the shared cache for the
    /// current version of a file is shown, and nothing is made; once the
    /// file changes, its thumbnail is made again and the entry replaced
    /// with the new time and size, which the next look finds.
    ///
    /// parity: VIEW-057
    #[gtk::test]
    fn a_cached_thumbnail_is_shown_until_its_file_changes() {
        let Some(cache) = private_cache() else {
            return;
        };
        let folder = tempfile::tempdir().expect("a temporary folder");
        // The picture is red; the thumbnail in the cache is green, so the
        // test tells a cache hit from a thumbnail made again.
        let item = picture_at(folder.path(), "Lake.png", 0xff00_00ff);
        let request = PreviewRequest {
            size: Some(4096),
            ..request_of(&item, "image/png")
        };
        let version = request.version().expect("a version");
        let cached = Pixbuf::new(Colorspace::Rgb, false, 8, 128, 64).expect("a thumbnail");
        cached.fill(0x00ff_00ff);
        let entry = thumbnail_file(&cache, &request.uri, ThumbnailFlavor::Normal);
        fs::create_dir_all(entry.parent().expect("a folder")).expect("the folder");
        cached
            .savev(
                &entry,
                "png",
                &[
                    ("tEXt::Thumb::URI", request.uri.as_str()),
                    ("tEXt::Thumb::MTime", "1700000000"),
                    ("tEXt::Thumb::Size", "4096"),
                ],
            )
            .expect("the entry");
        let context = glib::MainContext::default();

        let shown = context
            .block_on(preview(request.clone()))
            .expect("the cached thumbnail");

        assert_eq!(first_pixel(&shown), (0, 255, 0), "read from the cache, not made");
        // Shown once, it is kept for the frame a tile is bound on, also
        // with network previews off once its folder was found on this
        // computer, however long ago that was.
        let kept = kept_preview(&request).expect("kept");
        assert_eq!(first_pixel(&kept), (0, 255, 0));
        let local_only = PreviewRequest {
            local_only: true,
            ..request.clone()
        };
        let folders = local_folders(&request.uri, None);
        NETWORK_FOLDERS.with_borrow_mut(|known| {
            for folder in &folders {
                if let Some((_, asked)) = known.get_mut(folder) {
                    *asked = Instant::now()
                        .checked_sub(10 * NETWORK_ANSWER_LIFE)
                        .unwrap_or(*asked);
                }
            }
        });
        assert!(
            kept_preview(&local_only).is_some(),
            "shown at once after a while too"
        );

        // The file changed: the kept preview and the entry are both stale.
        let changed = PreviewRequest {
            modified: Some(1_700_000_100),
            size: Some(5000),
            ..request
        };
        assert!(kept_preview(&changed).is_none(), "another version is not shown");
        let made = context.block_on(preview(changed.clone())).expect("made again");
        assert_eq!(first_pixel(&made), (255, 0, 0), "made from the file itself");
        let keys = thumbnail_cache::png_keys(&fs::read(&entry).expect("read")).expect("keys");
        assert_eq!(keys.mtime.as_deref(), Some("1700000100"));
        assert_eq!(keys.size.as_deref(), Some("5000"));
        assert!(!keys.describe(&version), "the old version's entry is gone");
        assert!(thumbnail_cache::look_up(
            &cache,
            &changed.version().expect("a version"),
            ThumbnailFlavor::Normal
        )
        .is_some());
    }

    /// A damaged picture is recorded in the cache's `fail/` folder, and a
    /// recorded failure keeps a file from being tried again, here or in a
    /// later run, until the file changes.
    ///
    /// parity: VIEW-057
    #[gtk::test]
    fn a_failed_file_is_not_tried_again_until_it_changes() {
        let Some(cache) = private_cache() else {
            return;
        };
        let folder = tempfile::tempdir().expect("a temporary folder");
        let context = glib::MainContext::default();

        // A PNG cut short after its header: the decoder refuses it as
        // damaged, which no other thumbnailer would read better.
        let whole = fs::read(picture_at(folder.path(), "Whole.png", 0x3366_99ff)).expect("read");
        let damaged = folder.path().join("Damaged.png");
        fs::write(&damaged, &whole[..whole.len() / 3]).expect("written");
        let request = request_of(&damaged, "image/png");
        assert!(context.block_on(preview(request.clone())).is_none());
        let failure = thumbnail_cache::failure_file(&cache, &request.uri);
        // The failure is written off the main thread.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !failure.exists() && Instant::now() < deadline {
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(failure.exists(), "recorded in fail/openxplorer");
        assert!(thumbnail_cache::has_failed(
            &cache,
            &request.version().expect("a version")
        ));

        // A failure an earlier run recorded for a good picture keeps it
        // from being made, until the picture changes.
        let item = picture_at(folder.path(), "Recorded.png", 0x3366_99ff);
        let recorded = request_of(&item, "image/png");
        thumbnail_cache::record_failure(&cache, &recorded.version().expect("a version")).expect("recorded");
        assert!(
            context.block_on(preview(recorded.clone())).is_none(),
            "not tried again"
        );
        assert!(!thumbnail_file(&cache, &recorded.uri, ThumbnailFlavor::Normal).exists());
        let changed = PreviewRequest {
            modified: Some(1_700_000_500),
            ..recorded
        };
        assert!(
            context.block_on(preview(changed)).is_some(),
            "tried once it changed"
        );
    }

    /// A file renamed, or inside a folder renamed, keeps its thumbnail:
    /// the entry moves to the new URI with the new URI in its keys.
    ///
    /// parity: VIEW-057
    #[gtk::test]
    fn a_renamed_folder_keeps_its_files_thumbnails() {
        let Some(cache) = private_cache() else {
            return;
        };
        let folder = tempfile::tempdir().expect("a temporary folder");
        let before = folder.path().join("Trip");
        fs::create_dir(&before).expect("the folder");
        let item = picture_at(&before, "Beach.png", 0x3366_99ff);
        let metadata = fs::metadata(&item).expect("its metadata");
        let modified = metadata
            .modified()
            .expect("a time")
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after 1970")
            .as_secs();
        let request = PreviewRequest {
            modified: Some(modified),
            size: Some(metadata.len()),
            ..request_of(&item, "image/png")
        };
        let context = glib::MainContext::default();
        context.block_on(preview(request.clone())).expect("made");
        let old_entry = thumbnail_file(&cache, &request.uri, ThumbnailFlavor::Normal);
        assert!(old_entry.exists());

        let after = folder.path().join("Trip 2026");
        fs::rename(&before, &after).expect("renamed");
        let followed = follow_moves_of(&cache, &[(before.clone(), after.clone())]);

        let new_uri = gio::File::for_path(after.join("Beach.png")).uri().to_string();
        assert_eq!(followed, [(request.uri.clone(), new_uri.clone())]);
        assert!(!old_entry.exists(), "the old entry moved");
        let moved = Version {
            uri: new_uri,
            modified,
            size: Some(metadata.len()),
        };
        assert!(thumbnail_cache::look_up(&cache, &moved, ThumbnailFlavor::Normal).is_some());
    }

    /// The tiles of one folder share one question to its file system; the
    /// tile that asked being bound to another item meanwhile, as GTK
    /// rebinds tiles while it lays a view out, leaves the others to ask
    /// rather than waiting for ever.
    ///
    /// parity: VIEW-057, VIEW-058
    #[gtk::test]
    fn a_dropped_network_question_leaves_the_others_their_answer() {
        use std::future::Future;
        use std::task::{Context, Waker};

        let folder = tempfile::tempdir().expect("a temporary folder");
        let uri = |name: &str| gio::File::for_path(folder.path().join(name)).uri().to_string();
        let (first_uri, second_uri) = (uri("a.jpg"), uri("b.jpg"));
        let mut first = Box::pin(is_on_network(&first_uri, None));
        let mut second = Box::pin(is_on_network(&second_uri, None));
        let mut context = Context::from_waker(Waker::noop());
        assert!(first.as_mut().poll(&mut context).is_pending(), "the first asks");
        assert!(
            second.as_mut().poll(&mut context).is_pending(),
            "the second waits for it"
        );

        drop(first);

        let answer = glib::MainContext::default()
            .block_on(async { glib::future_with_timeout(Duration::from_secs(10), second).await });
        assert_eq!(
            answer,
            Ok(false),
            "answered, and a temporary folder is on this computer"
        );
    }

    /// Thumbnails are made one fewer at a time than the computer has
    /// processors, two at least and three at most, so a cold folder fills
    /// in faster where there is room for it, and a computer of two
    /// processors keeps two.
    ///
    /// parity: VIEW-057
    #[test]
    fn thumbnails_are_made_on_all_but_one_processor() {
        assert_eq!(making_at_once(1), 2);
        assert_eq!(making_at_once(2), 2);
        assert_eq!(making_at_once(4), 3);
        assert_eq!(making_at_once(64), 3);
    }
}
