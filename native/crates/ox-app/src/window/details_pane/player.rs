// SPDX-License-Identifier: AGPL-3.0-only
//! The inline player of the details pane (PROP-011), whose recording is
//! decoded in a player process of its own ([`crate::media_player`]), so a
//! file `GStreamer` aborts on never closes the window.

use std::path::PathBuf;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::media::{length_text, AUDIO_ART_SIZE, PREVIEW_HEIGHT, PREVIEW_WIDTH};
use super::DetailsPane;
use crate::icons::{Art, ArtImage};

/// How long the selection stays on a recording before its player starts:
/// arrowing through a folder of recordings starts none of them, and at
/// most one runs at a time.
const PLAYER_DELAY: Duration = Duration::from_millis(400);

impl DetailsPane {
    /// Shows the player of the audio or video file at `path`, which the
    /// app's file operations know as `source` (a share's own URI when
    /// `path` is its local path), once the selection has stayed on it for
    /// [`PLAYER_DELAY`]; the item keeps its art until then.
    pub(super) fn show_player(&self, path: &std::path::Path, source: String, is_video: bool, art: Art) {
        let generation = self.imp().media_generation.get();
        let path = PathBuf::from(path);
        let pane = self.downgrade();
        glib::spawn_future_local(async move {
            glib::timeout_future(PLAYER_DELAY).await;
            let Some(pane) = pane.upgrade() else {
                return;
            };
            if pane.imp().media_generation.get() == generation {
                pane.start_player(&path, &source, is_video, art);
            }
        });
    }

    /// An inline player for the audio or video file at `path`, decoded
    /// in a player process ([`crate::media_player`]); its length joins the
    /// Properties once the stream knows it. A player that fails or dies
    /// leaves the item's art, as does one the app stops before changing
    /// its file.
    fn start_player(&self, path: &std::path::Path, source: &str, is_video: bool, art: Art) {
        let imp = self.imp();
        let scale = u32::try_from(self.scale_factor().max(1)).unwrap_or(1);
        let kind = if is_video {
            crate::media_player::Kind::Video
        } else {
            crate::media_player::Kind::Audio
        };
        let size = |pixels: i32| u32::try_from(pixels).unwrap_or(1) * scale;
        let stream = crate::media_player::PlayerStream::new(
            path,
            source,
            kind,
            size(PREVIEW_WIDTH),
            size(PREVIEW_HEIGHT),
        );
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
        imp.player.replace(Some(stream.clone()));
        self.set_preview_widget(Some(&player));
        // Dolphin's "auto-play": off unless the user turned it on, and
        // never for the item whose player the app just stopped.
        let held = imp.holds_auto_play.replace(false);
        if imp.options.borrow().auto_play && !held && stream.error().is_none() {
            stream.play();
        }
        let generation = imp.media_generation.get();
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
        let pane = self.downgrade();
        let shows_art = move || {
            let pane = pane.upgrade()?;
            (pane.imp().media_generation.get() == generation).then(|| {
                pane.stop_player();
                pane.set_preview_widget(None);
                pane
            })
        };
        let on_error = shows_art.clone();
        stream.connect_error_notify(move |stream| {
            if stream.error().is_some() {
                on_error();
            }
        });
        if stream.error().is_some() {
            shows_art();
            return;
        }
        stream.connect_local("released", false, move |_| {
            if let Some(pane) = shows_art() {
                pane.imp().holds_auto_play.set(true);
            }
            None
        });
    }

    /// Stops the player shown, if any: paused, its pipeline set to NULL,
    /// then let go.
    pub(super) fn stop_player(&self) {
        if let Some(player) = self.imp().player.take() {
            player.end();
        }
    }
}
