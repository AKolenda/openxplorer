// SPDX-License-Identifier: AGPL-3.0-only
//! Previews of files in the folder views (VIEW-057), from the desktop's
//! shared thumbnail cache, as Dolphin and Files show them.
//!
//! A view asks for a preview when it binds a row or tile and drops the
//! request when GTK unbinds it, so rows scrolled past cost nothing more.
//! A request reads the cache through GIO (`thumbnail::path`,
//! `thumbnail::is-valid`, `thumbnail::failed`) at low priority. A missing
//! thumbnail is made by the session's thumbnailer service
//! ([`service`]) when it has one and supports the type, else, for the
//! picture types gdk-pixbuf reads, by the app itself ([`writer`]). Only a
//! few are decoded or made at a time ([`slots`]); a type that cannot be
//! previewed, or a failed attempt, keeps its icon. [`PreviewPolicy`]
//! says which items of a folder get previews, from the Settings the user
//! chose (VIEW-058).

mod service;
mod slots;
mod writer;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use gtk::gdk_pixbuf::{Pixbuf, PixbufFormat};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use ox_core::entry::{cached_thumbnail, CachedThumbnail, Entry, ThumbnailFlavor, THUMBNAIL_ATTRIBUTES};
use ox_core::settings::{ViewOptions, PREVIEW_SIZE_LIMIT};

pub(crate) use slots::Slots;
use writer::Source;

/// How many previews are decoded at a time.
const DECODING_AT_ONCE: usize = 4;
/// How many thumbnails are made at a time.
const MAKING_AT_ONCE: usize = 2;
/// How many decoded previews are kept.
const MAX_KEPT: usize = 512;

/// Which items of the folder shown get previews.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is a separate option of the Settings page"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct PreviewPolicy {
    /// Previews are shown at all; off, every item keeps its icon.
    pub shown: bool,
    /// Files larger than this keep their icon.
    pub size_limit: Option<u64>,
    /// Pictures get previews.
    pub pictures: bool,
    /// Videos get previews.
    pub videos: bool,
    /// Every other type gets previews.
    pub documents: bool,
}

impl PreviewPolicy {
    /// The policy for a folder that is on this computer, or not
    /// (`is_remote`), under the user's `options`.
    pub(crate) fn for_folder(options: &ViewOptions, is_remote: bool) -> Self {
        Self {
            shown: options.show_previews && (options.remote_previews || !is_remote),
            size_limit: options.skip_large_previews.then_some(PREVIEW_SIZE_LIMIT),
            pictures: options.preview_pictures,
            videos: options.preview_videos,
            documents: options.preview_documents,
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
        self.shown && !entry.is_dir && !entry.is_virtual && type_allowed && within_limit
    }
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
    /// The edge of the picture wanted, in device pixels.
    pub pixels: i32,
}

impl PreviewRequest {
    /// The request for `entry` at `pixels` device pixels, if the entry
    /// has a type.
    pub(crate) fn for_entry(entry: &Entry, pixels: i32) -> Option<Self> {
        Some(Self {
            uri: entry.uri.clone(),
            content_type: entry.content_type.clone()?,
            modified: entry.modified,
            pixels,
        })
    }

    fn key(&self) -> PreviewKey {
        (self.uri.clone(), self.modified, self.pixels)
    }
}

thread_local! {
    static DECODING: Rc<Slots> = Rc::new(Slots::new(DECODING_AT_ONCE));
    static MAKING: Rc<Slots> = Rc::new(Slots::new(MAKING_AT_ONCE));
    /// Decoded previews by item, version and size.
    static KEPT: RefCell<HashMap<PreviewKey, gdk::Texture>> = RefCell::new(HashMap::new());
    /// Item versions no thumbnail could be made of in this run.
    static FAILED: RefCell<HashSet<(String, Option<u64>)>> = RefCell::new(HashSet::new());
    /// The picture types gdk-pixbuf reads.
    static PIXBUF_TYPES: HashSet<String> = Pixbuf::formats()
        .iter()
        .flat_map(PixbufFormat::mime_types)
        .map(|mime| mime.to_string())
        .collect();
}

/// The preview `request` asks for, or `None` when the item keeps its icon.
pub(crate) async fn preview(request: PreviewRequest) -> Option<gdk::Texture> {
    let key = request.key();
    if let Some(texture) = KEPT.with_borrow(|kept| kept.get(&key).cloned()) {
        return Some(texture);
    }
    let path = thumbnail_of(&request).await?;
    let slot = DECODING.with(Rc::clone).take().await;
    let pixels = request.pixels;
    let texture = gio::spawn_blocking(move || decode(&path, pixels))
        .await
        .ok()
        .flatten();
    drop(slot);
    let texture = texture?;
    KEPT.with_borrow_mut(|kept| {
        if kept.len() >= MAX_KEPT {
            kept.clear();
        }
        kept.insert(key, texture.clone());
    });
    Some(texture)
}

/// The cached thumbnail of the item, made first if it is missing.
async fn thumbnail_of(request: &PreviewRequest) -> Option<PathBuf> {
    let file = gio::File::for_uri(&request.uri);
    let info = file
        .query_info_future(
            THUMBNAIL_ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::LOW,
        )
        .await
        .ok();
    match info.as_ref().map(cached_thumbnail) {
        Some(CachedThumbnail::Valid(path)) => return Some(path),
        Some(CachedThumbnail::Failed) => return None,
        Some(CachedThumbnail::Missing) | None => {}
    }
    let version = (request.uri.clone(), request.modified);
    if FAILED.with_borrow(|failed| failed.contains(&version)) {
        return None;
    }
    let made = make_thumbnail(request).await;
    if made.is_none() {
        FAILED.with_borrow_mut(|failed| failed.insert(version));
    }
    made
}

/// Makes the missing thumbnail of the item: by the thumbnailer service
/// when it supports the type, else by gdk-pixbuf for a picture.
async fn make_thumbnail(request: &PreviewRequest) -> Option<PathBuf> {
    let _slot = MAKING.with(Rc::clone).take().await;
    let flavor = ThumbnailFlavor::for_pixels(request.pixels);
    let cache_dir = glib::user_cache_dir();
    // GIO looks up local files only, and one size: a thumbnail this app
    // made earlier may be there all the same.
    let earlier = (cache_dir.clone(), request.uri.clone(), request.modified);
    let found = gio::spawn_blocking(move || {
        let (cache_dir, uri, modified) = earlier;
        writer::find_thumbnail(&cache_dir, &uri, modified?, flavor)
    });
    if let Ok(Some(path)) = found.await {
        return Some(path);
    }
    if let Some(service) = service::service().await {
        if service.supports(&request.uri, &request.content_type) {
            let made = service.make(&request.uri, &request.content_type, flavor).await;
            let path = ox_core::entry::thumbnail_file(&cache_dir, &request.uri, flavor);
            return made.then_some(path);
        }
    }
    // Test safety: tests write thumbnails only into a private cache.
    #[cfg(test)]
    if !cache_dir.starts_with(std::env::temp_dir()) {
        return None;
    }
    let is_picture = PIXBUF_TYPES.with(|types| types.contains(&request.content_type));
    let modified = request.modified?;
    if !is_picture {
        return None;
    }
    let source = Source {
        uri: request.uri.clone(),
        modified,
    };
    gio::spawn_blocking(move || writer::write_thumbnail(&cache_dir, &source, flavor))
        .await
        .ok()
        .flatten()
}

/// The thumbnail at `path`, scaled to fit `pixels` square.
fn decode(path: &std::path::Path, pixels: i32) -> Option<gdk::Texture> {
    let pixbuf = Pixbuf::from_file_at_scale(path, pixels, pixels, true).ok()?;
    Some(gdk::Texture::for_pixbuf(&pixbuf))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Previews follow Settings: off, only in folders on this computer,
    /// and not for files over the limit.
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
    }
}
