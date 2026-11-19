// SPDX-License-Identifier: AGPL-3.0-only
//! The details pane's inline player is never what takes the app down. A
//! recording is decoded by a player process of its own, which is stopped
//! and dropped before the app renames, moves, deletes or overwrites the
//! file it plays, and when the selection changes; one that dies leaves
//! the file's icon in the pane. Every recording is a fictional one the
//! test encodes.

use std::cell::Cell;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use gstreamer as gst;
use gstreamer::prelude::*;
use gtk::glib;
use gtk::prelude::*;

use super::file_ops_support::{answer_conflicts, is_enabled, name_editor, select_names, ConflictAnswer};
use crate::media_details::tests::{gstreamer_lock, write_song};
use crate::test_support::harness::{descendants, wait_for, wait_until, Fixture, TestWindow};
use crate::window::details_pane::DetailsPane;

/// A video of `seconds` seconds, 160 by 120 pixels at 10 frames a second,
/// with a mono Vorbis track: long enough to be still playing while a test
/// renames it.
fn write_clip(path: &Path, seconds: u32) {
    let _gstreamer = gstreamer_lock();
    gst::init().expect("GStreamer");
    let description = format!(
        "videotestsrc num-buffers={frames} pattern=ball ! video/x-raw,width=160,height=120,framerate=10/1 ! \
         videoconvert ! theoraenc ! oggmux name=mux ! filesink location=\"{path}\" \
         audiotestsrc num-buffers={blocks} samplesperbuffer=1024 ! audio/x-raw,rate=48000,channels=1 ! \
         audioconvert ! vorbisenc ! mux.",
        frames = seconds * 10,
        blocks = seconds * 47,
        path = path.display(),
    );
    let pipeline = gst::parse::launch(&description).expect("the encoder");
    pipeline.set_state(gst::State::Playing).expect("encoding");
    let bus = pipeline.bus().expect("a bus");
    let finished = bus.timed_pop_filtered(
        gst::ClockTime::from_seconds(60),
        &[gst::MessageType::Eos, gst::MessageType::Error],
    );
    pipeline.set_state(gst::State::Null).expect("stopped");
    assert!(
        finished.is_some_and(|message| message.type_() == gst::MessageType::Eos),
        "the clip is encoded"
    );
}

/// Opens a window on `fixture` with the details pane shown, and with
/// auto-play `on`.
fn window_with_player(fixture: &Fixture, auto_play: bool) -> TestWindow {
    let test = TestWindow::open(&fixture.uri());
    super::support::show_file_name_extensions(&test);
    if !test.window.details_pane().is_visible() {
        test.activate("details-pane", None);
    }
    let pane = test.window.details_pane();
    let mut options = pane.options();
    options.auto_play = auto_play;
    pane.set_options(options);
    test
}

/// The stream of the pane's player, video or audio.
fn player_stream(pane: &DetailsPane) -> Option<gtk::MediaStream> {
    descendants::<gtk::Video>(pane)
        .iter()
        .find_map(gtk::Video::media_stream)
        .or_else(|| {
            descendants::<gtk::MediaControls>(pane)
                .iter()
                .find_map(gtk::MediaControls::media_stream)
        })
}

/// Whether the pane shows a player at all.
fn shows_player(pane: &DetailsPane) -> bool {
    !descendants::<gtk::Video>(pane).is_empty() || !descendants::<gtk::MediaControls>(pane).is_empty()
}

/// A recording the pane plays, and when its stream stopped playing or was
/// released.
struct Playing {
    stream: glib::WeakRef<gtk::MediaStream>,
    stopped_at: Rc<Cell<Option<Instant>>>,
}

/// Selects `name` and waits until its recording plays and has moved on
/// from its start.
fn play(test: &TestWindow, name: &str) -> Playing {
    let pane = test.window.details_pane();
    select_names(test, &[name]);
    wait_until("the player", || player_stream(pane).is_some());
    let stream = player_stream(pane).expect("the player's stream");
    stream.play();
    wait_until("the recording to play", || {
        stream.error().is_none() && stream.is_playing() && stream.timestamp() > 0
    });
    let stopped_at = Rc::new(Cell::new(None));
    let stopped = Rc::clone(&stopped_at);
    stream.connect_playing_notify(move |stream| {
        if !stream.is_playing() && stopped.get().is_none() {
            stopped.set(Some(Instant::now()));
        }
    });
    let stopped = Rc::clone(&stopped_at);
    stream.add_weak_ref_notify_local(move || {
        if stopped.get().is_none() {
            stopped.set(Some(Instant::now()));
        }
    });
    Playing {
        stream: stream.downgrade(),
        stopped_at,
    }
}

/// Runs the main loop until `done`, while a thread watches for the moment
/// `changed` holds, and fails unless `playing` stopped before then: the
/// file changed under a stream that still played.
fn assert_stopped_before(
    playing: &Playing,
    what: &str,
    changed: impl Fn() -> bool + Send + 'static,
    done: impl Fn() -> bool,
) {
    let watching = Arc::new(AtomicBool::new(true));
    let watcher = {
        let watching = Arc::clone(&watching);
        thread::spawn(move || {
            while watching.load(Ordering::Relaxed) {
                if changed() {
                    return Some(Instant::now());
                }
                thread::sleep(Duration::from_micros(500));
            }
            None
        })
    };
    wait_until(what, || done() && watcher.is_finished());
    watching.store(false, Ordering::Relaxed);
    let changed_at = watcher.join().expect("the watcher").expect("the file changed");
    let stopped_at = playing.stopped_at.get();
    assert!(
        stopped_at.is_some_and(|stopped| stopped <= changed_at),
        "the player stopped before the file changed ({what}): stopped {:?} after the change",
        stopped_at.map(|stopped| stopped.saturating_duration_since(changed_at))
    );
}

/// F2 on a recording that plays stops and drops its player before the
/// file is renamed, and the renamed file's player waits for Play: the
/// folder's change never starts a stream again in mid-playback, even with
/// auto-play on.
///
/// parity: PROP-011
#[gtk::test]
fn renaming_a_playing_recording_stops_its_player_first() {
    let fixture = Fixture::standard();
    write_clip(&fixture.path("Harbor walk.ogv"), 20);
    let test = window_with_player(&fixture, true);
    let pane = test.window.details_pane();
    let playing = play(&test, "Harbor walk.ogv");

    test.activate("rename", None);
    let field = name_editor(&test);
    field.set_text("Pier.ogv");
    field.emit_activate();

    let renamed = fixture.path("Pier.ogv");
    assert_stopped_before(
        &playing,
        "the renamed file to be selected",
        move || renamed.exists(),
        || test.selected_names() == ["Pier.ogv"],
    );
    wait_until("the renamed recording's player", || player_stream(pane).is_some());
    wait_for(Duration::from_millis(800));
    let stream = player_stream(pane).expect("the renamed recording's player");
    assert!(
        !stream.is_playing(),
        "a renamed recording waits for Play rather than starting again"
    );
    wait_until("the old stream to be released", || {
        playing.stream.upgrade().is_none()
    });
}

/// Deleting a playing recording, moving it away and pasting another file
/// over it each stop its player before the file changes.
///
/// parity: PROP-011
#[gtk::test]
fn deleting_moving_or_replacing_a_playing_recording_stops_its_player_first() {
    super::file_ops_support::require_private_trash();
    let fixture = Fixture::standard();
    write_clip(&fixture.path("Harbor walk.ogv"), 20);
    write_clip(&fixture.path("Pier.ogv"), 20);
    write_clip(&fixture.path("Dunes.ogv"), 20);
    fs::copy(fixture.path("Pier.ogv"), fixture.path("Documents/Dunes.ogv")).expect("a namesake");
    let test = window_with_player(&fixture, false);

    let playing = play(&test, "Harbor walk.ogv");
    test.activate("trash", None);
    let deleted = fixture.path("Harbor walk.ogv");
    let gone = deleted.clone();
    assert_stopped_before(
        &playing,
        "the deleted file to go",
        move || !gone.exists(),
        || !deleted.exists() && !test.window.is_writing_files(),
    );

    // Documents' Dunes.ogv plays while the fixture's own is pasted over it.
    select_names(&test, &["Dunes.ogv"]);
    test.activate("copy", None);
    test.activate("go-to", Some(&fixture.uri_of("Documents")));
    wait_until("Documents", || {
        test.names().iter().any(|name| name == "Dunes.ogv")
    });
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));
    let playing = play(&test, "Dunes.ogv");
    let replaced = fixture.path("Documents/Dunes.ogv");
    let identity = |path: &Path| {
        fs::metadata(path)
            .ok()
            .map(|meta| (meta.ino(), meta.ctime(), meta.ctime_nsec()))
    };
    let before = identity(&replaced);
    test.activate("paste", None);
    answer_conflicts(&test, ConflictAnswer::Replace);
    let watched = replaced.clone();
    assert_stopped_before(
        &playing,
        "the file to be replaced",
        move || identity(&watched) != before,
        || identity(&replaced) != before && !test.window.is_writing_files(),
    );

    // The fixture's Pier.ogv plays while it is moved into Documents.
    test.activate("go-to", Some(&fixture.uri()));
    wait_until("the fixture's folder", || {
        test.names().iter().any(|name| name == "Pier.ogv")
    });
    let playing = play(&test, "Pier.ogv");
    test.activate("cut", None);
    test.activate("paste-into", Some(&fixture.uri_of("Documents")));
    let moved = fixture.path("Documents/Pier.ogv");
    let arrived = moved.clone();
    assert_stopped_before(
        &playing,
        "the file to be moved",
        move || arrived.exists(),
        || moved.exists() && !test.window.is_writing_files(),
    );
}

/// A player process that aborts, as `GStreamer`'s decoders do on a failed
/// assertion, takes only the preview with it: the pane shows the file's
/// icon in its place, and the next recording selected plays.
///
/// parity: PROP-011
#[gtk::test]
fn a_player_that_aborts_leaves_the_icon_and_the_app_running() {
    let fixture = Fixture::standard();
    write_clip(&fixture.path("Harbor walk.ogv"), 20);
    write_song(&fixture.path("Song.ogg"));
    let test = window_with_player(&fixture, false);
    let pane = test.window.details_pane();
    // The test's player process aborts when asked to play.
    let abort = AbortingPlayer::start();

    select_names(&test, &["Harbor walk.ogv"]);
    wait_until("the player", || player_stream(pane).is_some());
    player_stream(pane).expect("the player's stream").play();
    wait_until("the file's icon in place of the player", || !shows_player(pane));
    assert!(
        pane.shows_preview(),
        "the preview area stays, with the file's icon"
    );

    drop(abort);
    let playing = play(&test, "Song.ogg");
    assert!(playing.stream.upgrade().is_some_and(|stream| stream.is_playing()));
}

/// What the tests' player process reads (`media_player::TEST_ABORT_VARIABLE`).
const ABORT_VARIABLE: &str = "OX_TEST_PLAYER_ABORT";

/// Makes every player process the tests start abort when asked to play,
/// until dropped.
struct AbortingPlayer;

impl AbortingPlayer {
    fn start() -> Self {
        std::env::set_var(ABORT_VARIABLE, "1");
        Self
    }
}

impl Drop for AbortingPlayer {
    fn drop(&mut self) {
        std::env::remove_var(ABORT_VARIABLE);
    }
}
