// SPDX-License-Identifier: AGPL-3.0-only
//! Reading the shared thumbnail cache in batches, so a folder visited
//! again shows its thumbnails at once.
//!
//! The tiles GTK binds in one layout each ask for their thumbnail; the
//! requests are gathered until the main loop is idle and then read a batch
//! at a time on a few workers: each entry's keys are read from its PNG
//! without decoding it ([`thumbnail_cache`]), so a stale entry or a
//! recorded failure costs no decode, and the current entries of a batch go
//! to one picture decoder process in a single request
//! ([`picture_guard::decode_batch_to_fit`]), as every picture the app
//! shows is decoded out of the app. Each answer is handed back as soon as
//! it is decoded, the first tiles bound (the top of the view) first; a
//! request dropped while it waits, as a tile scrolled away is, is skipped.
//!
//! The names of the entries of each size are read once, in one listing
//! ([`probably_cached`]), so a view knows which tiles are about to show a
//! thumbnail without touching the disk, and draws them without their type
//! icon rather than an icon that is replaced a moment later.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use gtk::{gdk, glib};
use ox_core::entry::ThumbnailFlavor;
use ox_core::thumbnail_cache::{self, Version};

use super::Slots;
use crate::picture_guard::{self, Limits};

/// How many entries go to a decoder in one request.
const BATCH: usize = 12;

/// How many batches are read at a time.
pub(super) const READERS: usize = 4;

/// What the cache holds for a file.
#[derive(Debug, Clone)]
pub(super) enum Cached {
    /// A current thumbnail, decoded to the size asked for.
    Hit(gdk::Texture),
    /// A program failed to make one of this version of the file: it is
    /// not tried again until the file changes.
    Failed,
    /// None of this version, or one that could not be decoded.
    Missing,
}

/// A read waiting for its batch.
struct Read {
    version: Version,
    /// GNOME's recorded failures count too, besides the app's own.
    gnome_failures_count: bool,
    flavor: ThumbnailFlavor,
    pixels: i32,
    answer: async_channel::Sender<Cached>,
}

/// What is known of the entries of one size.
#[derive(Debug)]
enum Names {
    /// Being listed, with the entries written meanwhile.
    Listing(HashSet<thumbnail_cache::EntryKey>),
    /// Listed: the entries' names.
    Listed(HashSet<thumbnail_cache::EntryKey>),
}

thread_local! {
    static WAITING: RefCell<Vec<Read>> = const { RefCell::new(Vec::new()) };
    static SCHEDULED: Cell<bool> = const { Cell::new(false) };
    static READING: Rc<Slots> = Rc::new(Slots::new(READERS));
    static NAMES: RefCell<HashMap<&'static str, Names>> = RefCell::new(HashMap::new());
}

/// What the cache under `cache_dir` holds for `version` of a file in
/// `flavor` or a larger size, decoded to fit `pixels` square.
pub(super) async fn read(
    cache_dir: PathBuf,
    version: Version,
    gnome_failures_count: bool,
    flavor: ThumbnailFlavor,
    pixels: i32,
) -> Cached {
    let (answer, answered) = async_channel::bounded(1);
    WAITING.with_borrow_mut(|waiting| {
        waiting.push(Read {
            version,
            gnome_failures_count,
            flavor,
            pixels,
            answer,
        });
    });
    if !SCHEDULED.replace(true) {
        // Once every tile of this layout has asked.
        glib::idle_add_local_full(glib::Priority::HIGH_IDLE, move || {
            SCHEDULED.set(false);
            send_batches(&cache_dir);
            glib::ControlFlow::Break
        });
    }
    answered.recv().await.unwrap_or(Cached::Missing)
}

/// Hands the reads waiting to the workers, a batch at a time.
fn send_batches(cache_dir: &std::path::Path) {
    let waiting = WAITING.take();
    // A tile scrolled away before its read began asks for nothing.
    let mut waiting: Vec<Read> = waiting
        .into_iter()
        .filter(|read| !read.answer.is_closed())
        .collect();
    while !waiting.is_empty() {
        // A batch decodes to one size: the first read's, and the reads of
        // that size after it.
        let pixels = waiting[0].pixels;
        let (mut batch, rest): (Vec<Read>, Vec<Read>) =
            waiting.into_iter().partition(|read| read.pixels == pixels);
        waiting = rest;
        let later = batch.split_off(batch.len().min(BATCH));
        waiting.splice(0..0, later);
        let cache_dir = cache_dir.to_path_buf();
        let reading = READING.with(Rc::clone);
        glib::spawn_future_local(async move {
            reading.blocking(move |_| read_batch(&cache_dir, batch)).await;
        });
    }
}

/// Reads `batch` from the cache under `cache_dir`: answers a missing,
/// stale or failed entry at once, and decodes the current ones in one
/// request. Blocks.
fn read_batch(cache_dir: &std::path::Path, batch: Vec<Read>) {
    super::trace::event("cache-batch", &batch.len().to_string());
    let mut found = Vec::new();
    for read in batch {
        if read.answer.is_closed() {
            continue;
        }
        if let Some(hit) = thumbnail_cache::look_up(cache_dir, &read.version, read.flavor) {
            found.push((read, hit.bytes));
            continue;
        }
        let failed = if read.gnome_failures_count {
            thumbnail_cache::has_failed(cache_dir, &read.version)
        } else {
            thumbnail_cache::app_has_failed(cache_dir, &read.version)
        };
        let cached = if failed {
            super::trace::event("fail-hit", &read.version.uri);
            Cached::Failed
        } else {
            Cached::Missing
        };
        let _ = read.answer.try_send(cached);
    }
    let Some(pixels) = found.first().map(|(read, _)| read.pixels) else {
        return;
    };
    let (reads, pictures): (Vec<Read>, Vec<Vec<u8>>) = found.into_iter().unzip();
    for read in &reads {
        super::trace::event("cache-read", &read.version.uri);
    }
    picture_guard::decode_batch_to_fit(
        pictures,
        pixels,
        pixels,
        Limits::THUMBNAIL_FILE,
        None,
        &mut |index, decoded| {
            let Some(read) = reads.get(index) else {
                return;
            };
            // An entry the decoder refused is made again, and replaced.
            let cached = decoded.map_or(Cached::Missing, |decoded| {
                Cached::Hit(gdk::Texture::for_pixbuf(&decoded.pixbuf))
            });
            let _ = read.answer.try_send(cached);
        },
    );
}

/// Whether the cache under `cache_dir` probably holds a thumbnail of
/// `uri` in `flavor` or a larger size, from the names of its entries read
/// once in this run: no file is opened, so a view may ask it for every
/// tile it binds. `false` until the names are read, which the first
/// call starts.
pub(super) fn probably_cached(cache_dir: &std::path::Path, uri: &str, flavor: ThumbnailFlavor) -> bool {
    let Some(name) = thumbnail_cache::entry_key(uri) else {
        return false;
    };
    flavor.and_larger().any(|flavor| {
        NAMES.with_borrow_mut(|names| match names.get(flavor.as_str()) {
            Some(Names::Listed(listed)) => listed.contains(&name),
            Some(Names::Listing(_)) => false,
            None => {
                names.insert(flavor.as_str(), Names::Listing(HashSet::new()));
                let cache_dir = cache_dir.to_path_buf();
                glib::spawn_future_local(async move {
                    let listed =
                        gtk::gio::spawn_blocking(move || thumbnail_cache::entry_names(&cache_dir, flavor))
                            .await
                            .unwrap_or_default();
                    NAMES.with_borrow_mut(|names| {
                        // Entries written while the folder was listed stay.
                        let mut listed = listed;
                        if let Some(Names::Listing(written)) = names.remove(flavor.as_str()) {
                            listed.extend(written);
                        }
                        names.insert(flavor.as_str(), Names::Listed(listed));
                    });
                });
                false
            }
        })
    })
}

/// Notes that the cache now holds a thumbnail of `uri` in `flavor`.
pub(super) fn note_written(uri: &str, flavor: ThumbnailFlavor) {
    let Some(name) = thumbnail_cache::entry_key(uri) else {
        return;
    };
    NAMES.with_borrow_mut(|names| {
        if let Some(Names::Listed(names) | Names::Listing(names)) = names.get_mut(flavor.as_str()) {
            names.insert(name);
        }
    });
}
