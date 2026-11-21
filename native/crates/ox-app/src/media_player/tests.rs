// SPDX-License-Identifier: AGPL-3.0-only
//! The player process as the tests start it.

/// Not a test of its own: the body of the player process the tests start
/// (see `player_command`), which runs only this test with its command
/// line in the environment, and exits when it is done. Run as a test, it
/// does nothing.
#[test]
fn player_process() {
    let Ok(joined) = std::env::var(super::TEST_ARGUMENTS_VARIABLE) else {
        return;
    };
    let arguments: Vec<String> = std::iter::once("openxplorer".to_owned())
        .chain(joined.split('\n').map(str::to_owned))
        .collect();
    let status = super::run_as_player(&arguments).expect("a player's command line");
    std::process::exit(i32::from(status != gtk::glib::ExitCode::SUCCESS));
}

#[test]
fn only_the_player_flag_starts_a_player() {
    assert!(super::run_as_player(&["openxplorer".into(), "/home".into()]).is_none());
    assert_eq!(
        super::run_as_player(&["openxplorer".into(), super::PLAYER_FLAG.into(), "film".into()]),
        Some(gtk::glib::ExitCode::FAILURE),
        "an unknown kind"
    );
}

/// Starts a player of a short fictional song in `folder`, which the app
/// knows as `source` when given (a share's own URI), and waits until it is
/// ready.
fn ready_player(folder: &std::path::Path, source: Option<&str>) -> super::PlayerStream {
    let song = folder.join("Song.ogg");
    crate::media_details::tests::write_song(&song);
    let local = gio::File::for_path(&song).uri().to_string();
    let source = source.unwrap_or(&local);
    let stream = super::PlayerStream::new(&song, source, super::Kind::Audio, 216, 180);
    let context = glib::MainContext::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !stream.is_prepared() {
        assert!(stream.error().is_none(), "the player plays: {:?}", stream.error());
        assert!(
            std::time::Instant::now() < deadline,
            "the player is ready in time"
        );
        context.iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    stream
}

/// A player that does not quit when asked, as a hung decoder, is killed,
/// and is gone before a release of its file returns: nothing that comes
/// after the release meets it still holding the file.
#[gtk::test]
fn a_player_that_will_not_quit_is_gone_before_its_release_returns() {
    let folder = tempfile::tempdir().expect("a folder");
    std::env::set_var(super::TEST_STUBBORN_VARIABLE, "1");
    let stream = ready_player(folder.path(), None);
    std::env::remove_var(super::TEST_STUBBORN_VARIABLE);
    let pid = stream
        .process()
        .and_then(|process| process.identifier())
        .expect("the player's process");
    let uri = gio::File::for_path(folder.path().join("Song.ogg"))
        .uri()
        .to_string();

    glib::MainContext::default().block_on(async {
        let _hold = super::hold_players_within(&[uri]).await;
    });

    assert!(
        !std::path::Path::new(&format!("/proc/{pid}")).exists(),
        "the player has exited, and been reaped, when the release returns"
    );
}

use gtk::prelude::*;
use gtk::{gio, glib};

/// A recording of a share plays from the share's local path, but the app
/// changes it by the share's own URI: a hold of that URI stops its player.
#[gtk::test]
fn a_player_of_a_share_is_stopped_by_the_shares_own_uri() {
    const SHARE_URI: &str = "smb://nas.example/Music/Song.ogg";
    let folder = tempfile::tempdir().expect("a folder");
    let stream = ready_player(folder.path(), Some(SHARE_URI));
    let released = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = std::rc::Rc::clone(&released);
    stream.connect_local("released", false, move |_| {
        seen.set(true);
        None
    });

    glib::MainContext::default().block_on(async {
        let _hold = super::hold_players_within(&[SHARE_URI.to_owned()]).await;
    });

    assert!(released.get(), "the player is stopped by the share's URI");
    assert!(stream.process().is_none(), "its process is let go");
}

/// While the app holds a file, no player of it starts: a player the pane
/// was about to start shows the file's icon instead.
#[gtk::test]
fn no_player_starts_for_a_held_file() {
    let folder = tempfile::tempdir().expect("a folder");
    let song = folder.path().join("Song.ogg");
    crate::media_details::tests::write_song(&song);
    let uri = gio::File::for_path(&song).uri().to_string();
    let context = glib::MainContext::default();
    let hold = context.block_on(super::hold_players_within(std::slice::from_ref(&uri)));

    let held = super::PlayerStream::new(&song, &uri, super::Kind::Audio, 216, 180);
    assert!(
        held.error().is_some() && held.process().is_none(),
        "no player starts"
    );
    drop(hold);
    let free = super::PlayerStream::new(&song, &uri, super::Kind::Audio, 216, 180);
    assert!(free.process().is_some(), "a player starts once the hold ends");
    free.end();
}
