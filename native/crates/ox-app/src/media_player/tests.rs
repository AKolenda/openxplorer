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
