// SPDX-License-Identifier: AGPL-3.0-only
//! The details pane's content preview beyond pictures: inline players
//! and the cached thumbnails of documents.

use std::fs;
use std::path::PathBuf;

use gtk::prelude::*;
use gtk::{gdk_pixbuf, gio, glib};

use crate::test_support::harness::{descendants, wait_until, Fixture, TestWindow};

/// Opens a window on `fixture` with the details pane shown.
fn window_with_pane(fixture: &Fixture) -> TestWindow {
    let test = TestWindow::open(&fixture.uri());
    if !test.window.details_pane().is_visible() {
        test.activate("details-pane", None);
    }
    test
}

/// Whether `stream` plays, or cannot, having no media backend here.
fn plays_or_cannot(stream: &gtk::MediaStream) -> bool {
    stream.is_playing() || stream.error().is_some()
}

/// Audio and video get an inline player that starts only when the user
/// turned auto-play on.
///
/// parity: PROP-011
#[gtk::test]
fn recordings_get_a_player_that_starts_only_with_auto_play() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Clip.mp4"), b"not really a video").expect("a video");
    fs::write(fixture.path("Song.mp3"), b"not really a song").expect("a song");
    let test = window_with_pane(&fixture);
    let pane = test.window.details_pane();

    test.select_named("Clip.mp4");
    // The player opens once its file's path is checked off the main thread.
    wait_until("a video player", || !descendants::<gtk::Video>(pane).is_empty());
    let video = descendants::<gtk::Video>(pane).pop().expect("a video player");
    let stream = video.media_stream().expect("the video's stream");
    assert!(!stream.is_playing(), "a video never starts on its own");
    test.select_named("Song.mp3");
    wait_until("audio controls", || {
        !descendants::<gtk::MediaControls>(pane).is_empty()
    });
    let controls = descendants::<gtk::MediaControls>(pane)
        .pop()
        .expect("audio controls");
    assert!(descendants::<gtk::Video>(pane).is_empty());
    assert!(!controls.media_stream().expect("the song's stream").is_playing());

    let mut options = pane.options();
    options.auto_play = true;
    pane.set_options(options);
    test.select_named("Clip.mp4");
    wait_until("the video player again", || {
        !descendants::<gtk::Video>(pane).is_empty()
    });
    let video = descendants::<gtk::Video>(pane).pop().expect("a video player");
    assert!(plays_or_cannot(
        &video.media_stream().expect("the video's stream")
    ));
}

/// Another document shows its valid cached freedesktop thumbnail.
///
/// parity: PROP-011
#[gtk::test]
fn a_document_shows_its_cached_thumbnail() {
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from);
    assert!(
        cache
            .as_ref()
            .is_some_and(|cache| cache.starts_with(std::env::temp_dir())),
        "thumbnail tests only run with a private XDG_CACHE_HOME; found {cache:?}"
    );
    let fixture = Fixture::standard();
    let report = fixture.path("Report.pdf");
    fs::write(&report, b"%PDF-1.4 not really").expect("a document");
    write_thumbnail(&fixture.uri_of("Report.pdf"), &report);
    let test = window_with_pane(&fixture);
    let pane = test.window.details_pane();

    test.select_named("Report.pdf");

    wait_until("the thumbnail", || !descendants::<gtk::Picture>(pane).is_empty());
}

/// Writes a valid normal-size thumbnail of the file at `path`, `uri`, to
/// the thumbnail cache.
fn write_thumbnail(uri: &str, path: &std::path::Path) {
    let folder = glib::user_cache_dir().join("thumbnails").join("normal");
    fs::create_dir_all(&folder).expect("the thumbnail cache");
    let hash = glib::compute_checksum_for_string(glib::ChecksumType::Md5, uri).expect("a hash");
    let info = gio::File::for_path(path)
        .query_info(
            "time::modified",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        )
        .expect("the document's time");
    let modified = info.attribute_uint64("time::modified").to_string();
    let pixbuf = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 16, 16).expect("a picture");
    pixbuf.fill(0x4080_c0ff);
    pixbuf
        .savev(
            folder.join(format!("{hash}.png")),
            "png",
            &[("tEXt::Thumb::URI", uri), ("tEXt::Thumb::MTime", &modified)],
        )
        .expect("the thumbnail");
}

/// Writes a `width` by `height` PNG called `name` into `fixture`.
fn write_png(fixture: &Fixture, name: &str, width: i32, height: i32) {
    let picture =
        gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, width, height).expect("a picture");
    picture.fill(0x3366_99ff);
    picture.savev(fixture.path(name), "png", &[]).expect("png");
}

/// Arrowing through a folder of photos reads only the photo the
/// selection stops on: a photo passed on the way is never decoded, as
/// the Preview pane's pages wait for the selection to rest, so a folder
/// of large photos stays as quick with the pane open as without it.
///
/// parity: PROP-011
#[gtk::test]
fn arrowing_through_photos_reads_only_the_one_the_selection_stops_on() {
    let fixture = Fixture::standard();
    let names: Vec<String> = (1..=6).map(|number| format!("Photo {number}.png")).collect();
    for name in &names {
        write_png(&fixture, name, 64, 48);
    }
    let test = window_with_pane(&fixture);
    let pane = test.window.details_pane();
    let before = pane.media_reads();

    for name in &names {
        test.select_named(name);
    }

    wait_until("the last photo's picture", || {
        !descendants::<gtk::Picture>(pane).is_empty()
    });
    assert_eq!(
        pane.media_reads() - before,
        1,
        "only the photo the selection stopped on is read"
    );
    let last = fixture.uri_of(names.last().expect("photos"));
    assert!(pane.shows_media_of(&last), "the last photo is shown");
}
