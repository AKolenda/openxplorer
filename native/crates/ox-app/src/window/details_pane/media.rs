// SPDX-License-Identifier: AGPL-3.0-only
//! The content preview of the details pane (PROP-011), as Dolphin's
//! Information panel and Explorer's Preview pane show it: a scaled picture
//! of an image, the cached thumbnail of another document, and an inline
//! player for audio and video, which never starts on its own. An image's
//! dimensions and a recording's length join the Properties rows
//! (PROP-013).
//!
//! Everything is read off the main thread, and a preview that arrives
//! after the selection changed is dropped.

use std::path::PathBuf;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};
use ox_core::entry::{thumbnail_path, THUMBNAIL_ATTRIBUTES};

use super::content::MediaPreview;
use super::DetailsPane;
use crate::icons::{Art, ArtImage};

/// Images larger than this are not decoded for the pane.
const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
/// The art above the controls of an audio file.
const AUDIO_ART_SIZE: i32 = 64;
/// The largest picture or player the preview area holds.
const PREVIEW_WIDTH: i32 = 220;
const PREVIEW_HEIGHT: i32 = 146;

impl DetailsPane {
    /// Shows `media` in the preview area, or the item's art without one.
    pub(super) fn show_media(&self, media: Option<&MediaPreview>) {
        let imp = self.imp();
        let generation = imp.media_generation.get().wrapping_add(1);
        imp.media_generation.set(generation);
        self.set_preview_widget(None);
        let Some(media) = media else {
            return;
        };
        match media {
            MediaPreview::Image { path, size } if *size <= MAX_IMAGE_BYTES => {
                self.load_picture(generation, Some(path.clone()), None);
            }
            MediaPreview::Image { .. } => {}
            MediaPreview::Document { uri } => self.load_picture(generation, None, Some(uri.clone())),
            MediaPreview::Recording { path, is_video, art } => self.show_player(path, *is_video, *art),
        }
    }

    /// Puts `widget` in the preview area in place of the art, or the art
    /// back for `None`.
    fn set_preview_widget(&self, widget: Option<&gtk::Widget>) {
        let imp = self.imp();
        let art = imp.preview.upcast_ref::<gtk::Widget>();
        let widget = widget.unwrap_or(art);
        if imp.preview_area.center_widget().as_ref() != Some(widget) {
            imp.preview_area.set_center_widget(Some(widget));
        }
    }

    /// Decodes the image at `path`, or the cached thumbnail of the item at
    /// `uri`, and shows it if the selection is still `generation`'s.
    fn load_picture(&self, generation: u64, path: Option<PathBuf>, uri: Option<String>) {
        let pane = self.downgrade();
        let is_image = path.is_some();
        glib::spawn_future_local(async move {
            let decoded = gio::spawn_blocking(move || {
                let path = path.or_else(|| cached_thumbnail(uri.as_deref()?))?;
                gdk::Texture::from_filename(path).ok()
            })
            .await;
            let (Some(pane), Ok(Some(texture))) = (pane.upgrade(), decoded) else {
                return;
            };
            if pane.imp().media_generation.get() != generation {
                return;
            }
            let picture = gtk::Picture::for_paintable(&texture);
            picture.set_content_fit(gtk::ContentFit::Contain);
            picture.set_size_request(PREVIEW_WIDTH, PREVIEW_HEIGHT);
            picture.add_css_class("detail-picture");
            picture.update_property(&[gtk::accessible::Property::Label("Preview")]);
            pane.set_preview_widget(Some(picture.upcast_ref()));
            if is_image {
                let dimensions = format!("{} × {} pixels", texture.width(), texture.height());
                pane.add_property("Dimensions", &dimensions);
            }
        });
    }

    /// An inline player for the audio or video file at `path`; its length
    /// joins the Properties once the stream knows it.
    fn show_player(&self, path: &std::path::Path, is_video: bool, art: Art) {
        let stream = gtk::MediaFile::for_filename(path);
        let player: gtk::Widget = if is_video {
            let video = gtk::Video::builder().media_stream(&stream).build();
            video.set_size_request(PREVIEW_WIDTH, PREVIEW_HEIGHT);
            video.upcast()
        } else {
            let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
            column.append(&ArtImage::new(art, AUDIO_ART_SIZE));
            column.append(&gtk::MediaControls::new(Some(&stream)));
            column.set_size_request(PREVIEW_WIDTH, -1);
            column.upcast()
        };
        self.set_preview_widget(Some(&player));
        // Dolphin's "auto-play": off unless the user turned it on.
        if self.imp().options.borrow().auto_play {
            stream.play();
        }
        let generation = self.imp().media_generation.get();
        let pane = self.downgrade();
        stream.connect_prepared_notify(move |stream| {
            let Some(pane) = pane.upgrade() else {
                return;
            };
            let micros = stream.duration();
            if pane.imp().media_generation.get() == generation && stream.is_prepared() && micros > 0 {
                pane.add_property("Length", &length_text(micros));
            }
        });
    }
}

/// The valid cached thumbnail of the item at `uri`.
fn cached_thumbnail(uri: &str) -> Option<PathBuf> {
    let info = gio::File::for_uri(uri)
        .query_info(
            THUMBNAIL_ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        )
        .ok()?;
    thumbnail_path(&info)
}

/// A length in microseconds as `1:02:03` or `2:03`.
fn length_text(micros: i64) -> String {
    let seconds = micros / 1_000_000;
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_length_reads_as_a_clock() {
        assert_eq!(length_text(125_000_000), "2:05");
        assert_eq!(length_text(3_723_000_000), "1:02:03");
    }
}
