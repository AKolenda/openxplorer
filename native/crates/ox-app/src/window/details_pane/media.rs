// SPDX-License-Identifier: AGPL-3.0-only
//! The content preview of the details pane (PROP-011), as Dolphin's
//! Information panel and Explorer's Preview pane show it: a scaled picture
//! of an image, the cached thumbnail of another document, and an inline
//! player for audio and video, which never starts on its own. An image's
//! dimensions and a recording's length join the Properties rows
//! (PROP-013).
//!
//! Everything is read off the main thread, and a preview that arrives
//! after the selection changed is dropped. Pictures are decoded through
//! [`picture_guard`]: one over its pixel limit, damaged, slow or
//! animated keeps the type icon (an animation shows its first frame), as
//! Explorer's Preview pane does, and a picture too large to preview
//! still lists its dimensions, read from its header.

use std::path::PathBuf;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};
use ox_core::entry::{thumbnail_path, THUMBNAIL_ATTRIBUTES};

use super::content::MediaPreview;
use super::DetailsPane;
use crate::folder_view::cells::CUSTOM_ICON;
use crate::icons::{Art, ArtImage};
use crate::picture_guard::{self, Limits, Refusal};

/// Images larger than this are not decoded for the pane.
const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
/// The art above the controls of an audio file.
const AUDIO_ART_SIZE: i32 = 64;
/// The largest picture or player the preview area holds: the width of
/// the widest pane inside its padding, and a height that leaves room for
/// the Details rows, as Windows' pane does.
const PREVIEW_WIDTH: i32 = 216;
const PREVIEW_HEIGHT: i32 = 180;

impl DetailsPane {
    /// Shows `media` in the preview area, or the item's art without one.
    /// The same media as before stays as it is, so a player keeps playing
    /// and a picture is not decoded again when the pane is redrawn for
    /// another reason; nothing is read while the pane is hidden.
    pub(super) fn show_media(&self, media: Option<&MediaPreview>) {
        let imp = self.imp();
        let media = media.filter(|_| self.is_visible());
        if imp.shown_media.borrow().as_ref() == media {
            // The rows it found stay with the Details rows.
            return;
        }
        imp.shown_media.replace(media.cloned());
        if !imp.media_rows.borrow().is_empty() {
            imp.media_rows.borrow_mut().clear();
            self.redraw_properties();
        }
        let generation = imp.media_generation.get().wrapping_add(1);
        imp.media_generation.set(generation);
        self.set_preview_widget(None);
        let Some(media) = media else {
            return;
        };
        match media {
            MediaPreview::Image { path, size } if *size <= MAX_IMAGE_BYTES => {
                self.load_picture(generation, Some(path.clone()), None, true);
            }
            MediaPreview::Image { .. } => {}
            MediaPreview::Document { uri } => self.load_picture(generation, None, Some(uri.clone()), false),
            MediaPreview::Folder { uri } => self.load_folder_picture(generation, uri.clone()),
            MediaPreview::Recording { path, is_video, art } => self.show_player(path, *is_video, *art),
        }
    }

    /// Puts `widget` in the preview area in place of the art, or the art
    /// back for `None`.
    pub(super) fn set_preview_widget(&self, widget: Option<&gtk::Widget>) {
        let imp = self.imp();
        let art = imp.preview.upcast_ref::<gtk::Widget>();
        let widget = widget.unwrap_or(art);
        if imp.preview_area.center_widget().as_ref() != Some(widget) {
            imp.preview_area.set_center_widget(Some(widget));
        }
        // A picture of the content takes the height its shape needs; the
        // art keeps the area's own height.
        let is_picture = widget.is::<gtk::Picture>();
        crate::window::widget_tree::toggle_class(&*imp.preview_area, "has-picture", is_picture);
    }

    /// Reads the picture of the folder at `uri` ([`folder_picture`]) and
    /// shows it, if the selection is still `generation`'s; the folder
    /// keeps its art without one.
    fn load_folder_picture(&self, generation: u64, uri: String) {
        let pane = self.downgrade();
        glib::spawn_future_local(async move {
            let picture = folder_picture(&uri).await;
            let Some(pane) = pane.upgrade() else {
                return;
            };
            let path = picture.and_then(|picture| local_picture_path(&picture));
            if pane.imp().media_generation.get() == generation && path.is_some() {
                pane.load_picture(generation, path, None, false);
            }
        });
    }

    /// Reads again the picture of the item at `uri`, if the pane shows
    /// it: its custom icon or folder picture changed. True when it did.
    pub(in crate::window) fn reread_picture_of(&self, uri: &str) -> bool {
        let imp = self.imp();
        let shown = imp.shown_media.borrow().clone();
        let Some(media) = shown.filter(|media| media.is_of(uri)) else {
            return false;
        };
        imp.shown_media.replace(None);
        self.show_media(Some(&media));
        true
    }

    /// Decodes the image at `path`, or the cached thumbnail of the item at
    /// `uri`, scaled to the preview area, and shows it if the selection is
    /// still `generation`'s; an image's own size, from its header, joins
    /// the rows when `names_dimensions`.
    fn load_picture(
        &self,
        generation: u64,
        path: Option<PathBuf>,
        uri: Option<String>,
        names_dimensions: bool,
    ) {
        let pane = self.downgrade();
        let is_image = names_dimensions;
        let scale = self.scale_factor().max(1);
        glib::spawn_future_local(async move {
            let decoded = gio::spawn_blocking(move || {
                let path = path.or_else(|| cached_thumbnail(uri.as_deref()?))?;
                let (width, height) = (PREVIEW_WIDTH * scale, PREVIEW_HEIGHT * scale);
                let limits = if is_image {
                    Limits::DISPLAY
                } else {
                    Limits::THUMBNAIL_FILE
                };
                let file = gio::File::for_path(&path);
                match picture_guard::decode_to_fit(&file, width, height, limits, None) {
                    Ok(picture) => Some((
                        Some(gdk::Texture::for_pixbuf(&picture.pixbuf)),
                        (picture.width, picture.height),
                    )),
                    // Too large to preview: the type icon stays, and the
                    // header's size is still listed.
                    Err(Refusal::TooLarge { width, height }) if width > 0 && height > 0 => {
                        Some((None, (width, height)))
                    }
                    Err(_) => None,
                }
            })
            .await;
            let (Some(pane), Ok(Some((texture, size)))) = (pane.upgrade(), decoded) else {
                return;
            };
            if pane.imp().media_generation.get() != generation {
                return;
            }
            if is_image {
                pane.add_media_property(
                    "Dimensions",
                    ox_core::i18n::format_message(
                        "{width} × {height} pixels",
                        &[("width", &size.0.to_string()), ("height", &size.1.to_string())],
                    ),
                );
            }
            let Some(texture) = texture else {
                return;
            };
            let picture = gtk::Picture::for_paintable(&texture);
            picture.set_content_fit(gtk::ContentFit::Contain);
            // As wide as the pane allows, which is narrower in a narrow
            // window: the picture shrinks with it, never widening it.
            picture.set_can_shrink(true);
            picture.set_hexpand(true);
            picture.set_size_request(-1, preview_height(texture.width(), texture.height()));
            picture.add_css_class("detail-picture");
            picture.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext(
                "Preview",
            ))]);
            pane.set_preview_widget(Some(picture.upcast_ref()));
        });
    }

    /// An inline player for the audio or video file at `path`; its length
    /// joins the Properties once the stream knows it.
    fn show_player(&self, path: &std::path::Path, is_video: bool, art: Art) {
        let stream = gtk::MediaFile::for_filename(path);
        let player: gtk::Widget = if is_video {
            let video = gtk::Video::builder().media_stream(&stream).build();
            video.set_hexpand(true);
            video.set_size_request(-1, PREVIEW_HEIGHT);
            video.upcast()
        } else {
            let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
            column.append(&ArtImage::new(art, AUDIO_ART_SIZE));
            column.append(&gtk::MediaControls::new(Some(&stream)));
            column.set_hexpand(true);
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
                pane.add_media_property("Length", length_text(micros));
            }
        });
    }
}

impl DetailsPane {
    /// Adds the row `name` that the media shown says, unless the reader
    /// gave it already, and keeps it for when the rows are drawn again.
    fn add_media_property(&self, name: &'static str, value: String) {
        self.imp().media_rows.borrow_mut().push((name, value));
        self.redraw_properties();
    }
}

/// The picture the pane shows for the local folder at `uri` in place of
/// its art, as Windows' Details pane shows a customized folder: its
/// custom icon (PROP-016, `metadata::custom-icon`, which the file list
/// draws too), read at low priority.
///
/// Shared hook (PROP-037): the Customize tab's folder picture, read by
/// `folder_view::cells::folder_picture_of` on the customize-tab branch,
/// comes first once both branches are merged:
/// `if let Some(picture) = folder_picture_of(uri).await { return Some(picture); }`.
/// Both are announced the same way: OK or Apply on the Customize tab
/// calls `BrowserWindow::refresh_item_icon` in every window, which has
/// the pane read its picture again ([`DetailsPane::reread_picture_of`]).
async fn folder_picture(uri: &str) -> Option<String> {
    let info = gio::File::for_uri(uri)
        .query_info_future(CUSTOM_ICON, gio::FileQueryInfoFlags::NONE, glib::Priority::LOW)
        .await
        .ok()?;
    info.attribute_string(CUSTOM_ICON)
        .filter(|picture| !picture.is_empty())
        .map(|picture| picture.to_string())
}

/// The local file a picture's location names: a `file:` URI or a path.
fn local_picture_path(picture: &str) -> Option<PathBuf> {
    if picture.starts_with('/') {
        return Some(PathBuf::from(picture));
    }
    gio::File::for_uri(picture)
        .path()
        .filter(|_| picture.starts_with("file:"))
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

/// The height of a picture `width` by `height` pixels shown as wide as
/// the preview allows, at most [`PREVIEW_HEIGHT`]: a wide photo takes no
/// more room than it needs, as in Windows' pane.
fn preview_height(width: i32, height: i32) -> i32 {
    if width <= 0 || height <= 0 {
        return PREVIEW_HEIGHT;
    }
    let scaled = i64::from(PREVIEW_WIDTH) * i64::from(height) / i64::from(width);
    i32::try_from(scaled.clamp(1, i64::from(PREVIEW_HEIGHT))).unwrap_or(PREVIEW_HEIGHT)
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
    fn a_picture_is_as_tall_as_its_shape_needs() {
        assert_eq!(preview_height(4000, 3000), 162);
        assert_eq!(preview_height(320, 200), 135);
        assert_eq!(
            preview_height(1000, 3000),
            PREVIEW_HEIGHT,
            "a tall picture is held"
        );
        assert_eq!(preview_height(0, 0), PREVIEW_HEIGHT);
    }

    #[test]
    fn a_length_reads_as_a_clock() {
        assert_eq!(length_text(125_000_000), "2:05");
        assert_eq!(length_text(3_723_000_000), "1:02:03");
    }
}
