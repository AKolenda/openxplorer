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

/// The app's open files and threads.
fn open_files_and_threads() -> (usize, usize) {
    let count = |folder: &str| fs::read_dir(folder).map_or(0, Iterator::count);
    (count("/proc/self/fd"), count("/proc/self/task"))
}

/// Selects each of `names` in turn, staying `dwell` on each.
fn walk(test: &TestWindow, names: &[String], dwell: Duration) {
    for name in names {
        select_names(test, &[name.as_str()]);
        wait_for(dwell);
    }
}

/// Arrowing through 200 recordings with the Details pane, then with the
/// Preview pane, quickly and then slowly enough for each player to start,
/// leaves the app with the open files and threads it had: no player, nor
/// anything `GStreamer` leaks, stays behind in the app. GTK 4.14's own
/// player leaked about ten files and eight threads for each recording
/// shown, until the app ran out of files and `GLib` aborted it.
///
/// parity: PROP-011
#[gtk::test]
fn arrowing_through_200_recordings_leaves_nothing_behind() {
    let fixture = Fixture::standard();
    write_clip(&fixture.path("Clip 000.ogv"), 2);
    write_song(&fixture.path("Song 000.ogg"));
    let mut names = Vec::new();
    for number in 1..=100 {
        for (stem, extension) in [("Clip", "ogv"), ("Song", "ogg")] {
            let name = format!("{stem} {number:03}.{extension}");
            fs::copy(
                fixture.path(&format!("{stem} 000.{extension}")),
                fixture.path(&name),
            )
            .expect("a copy");
            names.push(name);
        }
    }
    let test = window_with_player(&fixture, true);
    let pane = test.window.details_pane();
    wait_until("the recordings", || test.names().len() > 200);
    // What the app has open once one player ran and stopped.
    play(&test, "Clip 000.ogv");
    select_names(&test, &["Notes 2.txt"]);
    wait_until("the player to go", || !shows_player(pane));
    wait_for(Duration::from_secs(1));
    let (files, threads) = open_files_and_threads();

    for preview_pane in [false, true] {
        if preview_pane {
            test.activate("preview-pane", None);
            wait_until("the Preview pane", || pane.is_visible());
        }
        walk(&test, &names, Duration::from_millis(30));
        walk(&test, &names[..12], Duration::from_millis(700));
        wait_until("a player for the last recording", || {
            player_stream(pane).is_some()
        });
        select_names(&test, &["Notes 2.txt"]);
        wait_until("the open files and threads to come back", || {
            let (now_files, now_threads) = open_files_and_threads();
            now_files <= files + 4 && now_threads <= threads + 4
        });
    }
}

/// A paste over a recording whose player is still waiting to start, the
/// selection having just reached it, keeps the player from starting while
/// the Replace question is open, and until the file is replaced.
///
/// parity: PROP-011
#[gtk::test]
fn a_paste_over_a_recording_whose_player_waits_to_start_keeps_it_from_starting() {
    let fixture = Fixture::standard();
    write_clip(&fixture.path("Dunes.ogv"), 20);
    write_clip(&fixture.path("Documents/Dunes.ogv"), 30);
    let test = window_with_player(&fixture, true);
    let pane = test.window.details_pane();
    select_names(&test, &["Dunes.ogv"]);
    test.activate("copy", None);
    test.activate("go-to", Some(&fixture.uri_of("Documents")));
    wait_until("Documents", || {
        test.names().iter().any(|name| name == "Dunes.ogv")
    });
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));

    // The pane waits before it starts Dunes.ogv's player; the paste starts
    // at once, and its question stays open well past that wait.
    select_names(&test, &["Dunes.ogv"]);
    test.activate("paste", None);
    let question = super::name_conflicts::question_window(&test);
    wait_for(Duration::from_millis(1200));
    assert!(
        !shows_player(pane),
        "no player starts for a file the paste may replace"
    );

    let replaced = fixture.path("Documents/Dunes.ogv");
    let before = fs::metadata(&replaced).expect("the namesake").len();
    super::name_conflicts::button(&question, "Replace the file in the destination").emit_clicked();
    wait_until("the file to be replaced", || {
        fs::metadata(&replaced).is_ok_and(|meta| meta.len() != before) && !test.window.is_writing_files()
    });
}

/// Undo of a copy moves the copy to the Recycle Bin only once the player
/// of that copy has stopped.
///
/// parity: PROP-011, OPS-029
#[gtk::test]
fn undoing_a_copy_stops_the_copys_player_first() {
    super::file_ops_support::require_private_trash();
    let fixture = Fixture::standard();
    write_clip(&fixture.path("Pier.ogv"), 20);
    let test = window_with_player(&fixture, false);
    select_names(&test, &["Pier.ogv"]);
    test.activate("copy", None);
    test.activate("paste-into", Some(&fixture.uri_of("Documents")));
    let copy = fixture.path("Documents/Pier.ogv");
    wait_until("the copy", || copy.exists() && !test.window.is_writing_files());
    wait_until("Undo Copy", || is_enabled(&test, "undo"));
    test.activate("go-to", Some(&fixture.uri_of("Documents")));
    wait_until("Documents", || test.names().iter().any(|name| name == "Pier.ogv"));
    let playing = play(&test, "Pier.ogv");

    test.activate("undo", None);
    let gone = copy.clone();
    assert_stopped_before(
        &playing,
        "the copy to be undone",
        move || !gone.exists(),
        || !copy.exists() && !test.window.is_writing_files(),
    );
}

/// Renaming several selected items at once (F2 with a numbered name)
/// stops the player another window has of one of them first.
///
/// parity: PROP-011, OPS-050
#[gtk::test]
fn a_numbered_rename_stops_a_player_of_one_of_its_files_first() {
    let fixture = Fixture::standard();
    write_clip(&fixture.path("Pier.ogv"), 20);
    write_song(&fixture.path("Song.ogg"));
    let test = window_with_player(&fixture, false);
    let playing = play(&test, "Pier.ogv");
    // The second window shares the first's setting: extensions are shown.
    let other = test.open_beside(&fixture.uri());

    select_names(&other, &["Pier.ogv", "Song.ogg"]);
    other.activate("rename", None);
    let field = name_editor(&other);
    field.set_text("Walk");
    field.emit_activate();
    let pier = fixture.path("Pier.ogv");
    let gone = pier.clone();
    assert_stopped_before(
        &playing,
        "the numbered rename",
        move || !gone.exists(),
        || !pier.exists() && !fixture.path("Song.ogg").exists() && !other.window.is_writing_files(),
    );
}

/// A Recycle Bin item restored with Replace over a recording another
/// window plays stops that player before the recording is replaced.
///
/// parity: PROP-011, OPS-040
#[gtk::test]
fn restoring_over_a_playing_recording_stops_its_player_first() {
    super::file_ops_support::require_private_trash();
    let fixture = Fixture::standard();
    let path = fixture.path("Pier.ogv");
    write_clip(&path, 30);
    gtk::gio::File::for_path(&path)
        .trash(None::<&gtk::gio::Cancellable>)
        .expect("the private Trash takes the file");
    write_clip(&path, 20);
    let test = window_with_player(&fixture, false);
    let playing = play(&test, "Pier.ogv");
    let bin = test.open_beside(ox_core::location::TRASH_URI);
    wait_until("the trashed recording", || {
        bin.names().iter().any(|name| name == "Pier.ogv")
    });

    select_names(&bin, &["Pier.ogv"]);
    let identity = |path: &Path| {
        fs::metadata(path)
            .ok()
            .map(|meta| (meta.ino(), meta.ctime(), meta.ctime_nsec()))
    };
    let before = identity(&path);
    bin.activate("restore", None);
    answer_conflicts(&bin, ConflictAnswer::Replace);
    let watched = path.clone();
    assert_stopped_before(
        &playing,
        "the recording to be replaced",
        move || identity(&watched) != before,
        || identity(&path) != before && !bin.window.is_writing_files(),
    );
}

/// Copying a playing recording into its own folder makes "name - Copy"
/// and leaves the recording alone: its player plays on until the copy
/// exists, and the copy, selected then, starts with auto-play.
///
/// parity: PROP-011
#[gtk::test]
fn copying_a_playing_recording_into_its_own_folder_leaves_it_playing() {
    let fixture = Fixture::standard();
    write_clip(&fixture.path("Pier.ogv"), 20);
    let test = window_with_player(&fixture, true);
    let pane = test.window.details_pane();
    let playing = play(&test, "Pier.ogv");

    test.activate("copy", None);
    test.activate("paste", None);
    let copy = fixture.path("Pier - Copy.ogv");
    let watched = copy.clone();
    let watcher = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if watched.exists() {
                return Some(Instant::now());
            }
            thread::sleep(Duration::from_micros(500));
        }
        None
    });
    wait_until("the copy", || {
        copy.exists() && !test.window.is_writing_files() && watcher.is_finished()
    });
    let copied_at = watcher.join().expect("the watcher").expect("the copy is made");
    assert!(
        playing
            .stopped_at
            .get()
            .is_none_or(|stopped| stopped >= copied_at),
        "the recording played on while it was copied"
    );
    wait_until("the copy to play", || {
        test.selected_names() == ["Pier - Copy.ogv"]
            && player_stream(pane).is_some_and(|stream| stream.is_playing())
    });
}
