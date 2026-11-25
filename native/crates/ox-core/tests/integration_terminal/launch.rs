// SPDX-License-Identifier: AGPL-3.0-only
//! Choosing the terminal and starting it. Ports the `find_terminal`,
//! `terminal_argv` and `launch_terminal` cases of `TerminalTests`; terminal
//! lookups run in a fake system root instead of patching `shutil.which`,
//! and a recorder script stands in for the graphical terminal.

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;
use std::time::{Duration, Instant};

use ox_core::integration::{
    find_command_terminal, find_terminal, launch_terminal, terminal_arguments, ExecutableSearch,
    PreferredTerminal, PreparedDirectory, Sandbox, Terminal, TerminalError, TerminalKind, SYSTEM_PATH,
};
use ox_core::location::file_uri;
use serde_json::Value;

use super::{real_path, temporary_folder};

/// An executable shell script at `path`, creating its folder.
fn install_program(path: &Path, script: &str) {
    fs::create_dir_all(path.parent().expect("a folder")).expect("folder");
    fs::write(path, script).expect("program");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("executable");
}

/// Waits up to five seconds for `path` to appear.
fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(Instant::now() < deadline, "{} did not appear", path.display());
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_no_terminal_actionable`
/// parity: OPEN-018
#[test]
fn without_a_terminal_the_message_says_how_to_install_one() {
    let root = temporary_folder();

    let refused = find_terminal(&ExecutableSearch::under(root.path()));

    assert!(matches!(refused, Err(TerminalError::NoTerminal)), "{refused:?}");
    assert!(refused
        .expect_err("refused")
        .to_string()
        .contains("sudo apt install gnome-terminal"));
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_only_trusted_path_searched`
/// parity: OPEN-018, SAFE-015
#[test]
fn only_the_system_folders_are_searched() {
    let root = temporary_folder();
    install_program(&root.path().join("home/demo/bin/gnome-terminal"), "#!/bin/sh\n");
    install_program(&root.path().join("opt/terminal/konsole"), "#!/bin/sh\n");

    let refused = find_terminal(&ExecutableSearch::under(root.path()));

    assert_eq!(SYSTEM_PATH, ["/usr/bin", "/bin", "/usr/local/bin"]);
    assert!(matches!(refused, Err(TerminalError::NoTerminal)), "{refused:?}");
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_debian_gnome_alternative`
/// parity: OPEN-018
#[test]
fn debians_gnome_wrapper_alternative_runs_the_real_gnome_terminal() {
    let root = temporary_folder();
    let usr_bin = root.path().join("usr/bin");
    install_program(&usr_bin.join("gnome-terminal.wrapper"), "#!/bin/sh\n");
    install_program(&usr_bin.join("gnome-terminal"), "#!/bin/sh\n");
    fs::create_dir_all(root.path().join("etc/alternatives")).expect("alternatives");
    symlink(
        "/usr/bin/gnome-terminal.wrapper",
        root.path().join("etc/alternatives/x-terminal-emulator"),
    )
    .expect("alternative");
    symlink(
        "/etc/alternatives/x-terminal-emulator",
        usr_bin.join("x-terminal-emulator"),
    )
    .expect("link");

    let terminal = find_terminal(&ExecutableSearch::under(root.path())).expect("a terminal");

    let expected =
        Terminal::new("/usr/bin/gnome-terminal".into(), TerminalKind::GnomeTerminal).expect("absolute");
    assert_eq!(terminal, expected);
}

/// parity: OPEN-018
#[test]
fn the_debian_alternative_wins_over_the_preference_order() {
    let root = temporary_folder();
    let usr_bin = root.path().join("usr/bin");
    install_program(&usr_bin.join("gnome-terminal"), "#!/bin/sh\n");
    install_program(&usr_bin.join("konsole"), "#!/bin/sh\n");
    symlink("konsole", usr_bin.join("x-terminal-emulator")).expect("alternative");

    let terminal = find_terminal(&ExecutableSearch::under(root.path())).expect("a terminal");

    assert_eq!(terminal.kind(), TerminalKind::Konsole);
}

/// The desktop's configured terminal wins over the Debian alternative when
/// it is installed in the system folders, and is passed over when not.
///
/// parity: OPEN-019
#[test]
fn the_desktops_configured_terminal_wins_when_it_is_installed() {
    let root = temporary_folder();
    let usr_bin = root.path().join("usr/bin");
    install_program(&usr_bin.join("gnome-terminal"), "#!/bin/sh\n");
    install_program(&usr_bin.join("kgx"), "#!/bin/sh\n");
    symlink("gnome-terminal", usr_bin.join("x-terminal-emulator")).expect("alternative");
    let search = ExecutableSearch::under(root.path());

    let console = find_terminal(
        &search
            .clone()
            .preferring(vec![PreferredTerminal::Listed(Some(TerminalKind::Console))]),
    )
    .expect("a terminal");
    let missing = find_terminal(&search.clone().preferring(vec![
        PreferredTerminal::Listed(Some(TerminalKind::Konsole)),
        PreferredTerminal::Listed(None),
    ]))
    .expect("a terminal");
    let gnome = find_terminal(&search.preferring(vec![
        PreferredTerminal::Listed(Some(TerminalKind::Konsole)),
        PreferredTerminal::Gnome(TerminalKind::Console),
    ]))
    .expect("a terminal");

    assert_eq!(console.kind(), TerminalKind::Console);
    assert_eq!(
        missing.kind(),
        TerminalKind::GnomeTerminal,
        "without xdg-terminal-exec, terminals it would start are passed over"
    );
    assert_eq!(gnome.kind(), TerminalKind::Console);
}

/// A configured terminal the app cannot start itself, such as one it does
/// not know, is started through `xdg-terminal-exec` when that is installed
/// in the system folders; a known, installed one before it is still
/// started directly, and GNOME's setting never calls `xdg-terminal-exec`.
///
/// parity: OPEN-019, SAFE-015
#[test]
fn xdg_terminal_exec_starts_a_configured_terminal_the_app_cannot_start() {
    let root = temporary_folder();
    let usr_bin = root.path().join("usr/bin");
    install_program(&usr_bin.join("xterm"), "#!/bin/sh\n");
    install_program(&usr_bin.join("ghostty"), "#!/bin/sh\n");
    symlink("xterm", usr_bin.join("x-terminal-emulator")).expect("alternative");
    let search = ExecutableSearch::under(root.path());
    let unknown = vec![PreferredTerminal::Listed(None)];
    let not_installed = vec![PreferredTerminal::Listed(Some(TerminalKind::Alacritty))];

    // Only a program in the system folders is used.
    install_program(
        &root.path().join("home/demo/.local/bin/xdg-terminal-exec"),
        "#!/bin/sh\n",
    );
    let without = find_terminal(&search.clone().preferring(unknown.clone())).expect("a terminal");
    assert_eq!(without.kind(), TerminalKind::XTerm);

    install_program(&usr_bin.join("xdg-terminal-exec"), "#!/bin/sh\n");
    let through = find_terminal(&search.clone().preferring(unknown)).expect("a terminal");
    let not_found = find_terminal(&search.clone().preferring(not_installed)).expect("a terminal");
    let known = find_terminal(&search.clone().preferring(vec![
        PreferredTerminal::Listed(Some(TerminalKind::Ghostty)),
        PreferredTerminal::Listed(None),
    ]))
    .expect("a terminal");
    let gnome = find_terminal(
        &search
            .clone()
            .preferring(vec![PreferredTerminal::Gnome(TerminalKind::Alacritty)]),
    )
    .expect("a terminal");

    assert_eq!(
        through,
        Terminal::new("/usr/bin/xdg-terminal-exec".into(), TerminalKind::XdgTerminalExec).expect("absolute")
    );
    assert_eq!(not_found.kind(), TerminalKind::XdgTerminalExec);
    assert_eq!(known.kind(), TerminalKind::Ghostty);
    assert_eq!(gnome.kind(), TerminalKind::XTerm, "the Debian alternative");
}

/// With no known terminal installed, `xdg-terminal-exec` starts whatever
/// terminal the desktop has.
///
/// parity: OPEN-018
#[test]
fn xdg_terminal_exec_is_the_last_resort() {
    let root = temporary_folder();
    install_program(&root.path().join("usr/bin/xdg-terminal-exec"), "#!/bin/sh\n");

    let terminal = find_terminal(&ExecutableSearch::under(root.path())).expect("a terminal");

    assert_eq!(terminal.kind(), TerminalKind::XdgTerminalExec);
}

/// A program run in the terminal, such as a dropped-on script or a
/// custom command, never goes through `xdg-terminal-exec`, which hands it
/// to the terminal's own `-e` (Ghostty before 1.2 runs that as shell
/// code): a configured terminal the app cannot start itself is passed
/// over for one it can, and with no such terminal nothing runs.
///
/// parity: OPEN-019, OPEN-020, SAFE-015
#[test]
fn a_program_runs_only_in_a_terminal_the_app_starts_itself() {
    let root = temporary_folder();
    let usr_bin = root.path().join("usr/bin");
    install_program(&usr_bin.join("xdg-terminal-exec"), "#!/bin/sh\n");
    let search = ExecutableSearch::under(root.path()).preferring(vec![
        PreferredTerminal::Listed(None),
        PreferredTerminal::Listed(Some(TerminalKind::Alacritty)),
    ]);

    let alone = find_command_terminal(&search);
    install_program(&usr_bin.join("xterm"), "#!/bin/sh\n");
    let command = find_command_terminal(&search).expect("a terminal");
    install_program(&usr_bin.join("ghostty"), "#!/bin/sh\n");
    let listed = find_command_terminal(&search.clone().preferring(vec![
        PreferredTerminal::Listed(None),
        PreferredTerminal::Listed(Some(TerminalKind::Ghostty)),
    ]))
    .expect("a terminal");

    assert!(matches!(alone, Err(TerminalError::NoTerminal)), "{alone:?}");
    assert_eq!(command.kind(), TerminalKind::XTerm);
    assert_eq!(
        find_terminal(&search).expect("a terminal").kind(),
        TerminalKind::XdgTerminalExec,
        "Open in Terminal still goes through it"
    );
    assert_eq!(
        listed.kind(),
        TerminalKind::Ghostty,
        "the next configured terminal the app knows"
    );
}

/// A Debian alternative pointing to one of the added terminals is
/// followed, and Tilix's option-translating wrapper resolves to the real
/// program.
///
/// parity: OPEN-018
#[test]
fn the_debian_alternative_names_the_added_terminals_too() {
    let root = temporary_folder();
    let usr_bin = root.path().join("usr/bin");
    install_program(&usr_bin.join("gnome-terminal"), "#!/bin/sh\n");
    install_program(&usr_bin.join("tilix"), "#!/bin/sh\n");
    install_program(&usr_bin.join("tilix.wrapper"), "#!/bin/sh\n");
    symlink("tilix.wrapper", usr_bin.join("x-terminal-emulator")).expect("alternative");

    let terminal = find_terminal(&ExecutableSearch::under(root.path())).expect("a terminal");

    assert_eq!(terminal.executable(), Path::new("/usr/bin/tilix"));
    assert_eq!(terminal.kind(), TerminalKind::Tilix);
}

/// parity: OPEN-018
#[test]
fn without_an_alternative_the_first_installed_terminal_in_order_is_used() {
    let root = temporary_folder();
    install_program(&root.path().join("usr/local/bin/xterm"), "#!/bin/sh\n");
    install_program(&root.path().join("usr/bin/konsole"), "#!/bin/sh\n");
    fs::write(root.path().join("usr/bin/gnome-terminal"), "not executable").expect("plain file");

    let terminal = find_terminal(&ExecutableSearch::under(root.path())).expect("a terminal");

    assert_eq!(terminal.executable(), Path::new("/usr/bin/konsole"));
}

/// kitty is a native addition to the ported list, ahead of the `XTerm` family.
///
/// parity: OPEN-018
#[test]
fn kitty_is_preferred_over_xterm_when_nothing_else_is_installed() {
    let root = temporary_folder();
    install_program(&root.path().join("usr/bin/kitty"), "#!/bin/sh\n");
    install_program(&root.path().join("usr/bin/xterm"), "#!/bin/sh\n");

    let terminal = find_terminal(&ExecutableSearch::under(root.path())).expect("a terminal");

    assert_eq!(terminal.kind(), TerminalKind::Kitty);
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_all_cli_styles`
/// parity: OPEN-020
#[test]
fn each_terminal_gets_its_own_working_directory_option() {
    let root = temporary_folder();
    let folder = real_path(root.path());
    let joined = |option: &str| vec![format!("{option}{}", folder.display())];
    let styles: [(TerminalKind, Vec<String>); 15] = [
        (TerminalKind::Console, joined("--working-directory=")),
        (TerminalKind::GnomeTerminal, joined("--working-directory=")),
        (TerminalKind::XfceTerminal, joined("--working-directory=")),
        (TerminalKind::Konsole, joined("--workdir=")),
        (TerminalKind::Kitty, joined("--directory=")),
        (TerminalKind::Ghostty, joined("--working-directory=")),
        (TerminalKind::Alacritty, joined("--working-directory=")),
        (TerminalKind::Foot, joined("--working-directory=")),
        (TerminalKind::Tilix, joined("--working-directory=")),
        // Ptyxis ignores a folder unless asked for a window.
        (
            TerminalKind::Ptyxis,
            ["--new-window".to_owned()]
                .into_iter()
                .chain(joined("--working-directory="))
                .collect(),
        ),
        (
            TerminalKind::Terminator,
            ["--no-dbus".to_owned()]
                .into_iter()
                .chain(joined("--working-directory="))
                .collect(),
        ),
        (
            TerminalKind::WezTerm,
            vec![
                "start".to_owned(),
                "--cwd".to_owned(),
                folder.display().to_string(),
            ],
        ),
        (TerminalKind::XdgTerminalExec, joined("--dir=")),
        (TerminalKind::XTerm, Vec::new()),
        (TerminalKind::UXTerm, Vec::new()),
    ];

    for (kind, options) in styles {
        let executable = Path::new("/usr/bin").join(kind.program_name());
        let terminal = Terminal::new(executable.clone(), kind).expect("absolute");

        let arguments = terminal_arguments(&terminal, root.path()).expect("arguments");

        let mut expected = vec![executable.into_os_string()];
        expected.extend(options.into_iter().map(Into::into));
        assert_eq!(arguments, expected, "{kind:?}");
    }
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_unknown_executable_kind_rejected`
/// parity: OPEN-018, SAFE-015
#[test]
fn only_known_terminals_with_absolute_programs_are_started() {
    let refused = Terminal::new("sh".into(), TerminalKind::XTerm);

    assert_eq!(TerminalKind::from_program_name("sh"), None);
    assert!(
        matches!(refused, Err(TerminalError::UnsupportedTerminal)),
        "{refused:?}"
    );
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_real_process_preserves_literal_shell_metacharacters`
/// parity: OPEN-018, OPEN-020, SAFE-015
#[test]
fn a_folder_named_like_shell_code_stays_a_name() {
    let root = temporary_folder();
    let folder = root.path().join("$(touch PWNED); apostrophe' & spaces");
    fs::create_dir(&folder).expect("folder");
    let recorder = root.path().join("recorder");
    install_program(
        &recorder,
        "#!/usr/bin/python3\nimport json,os,sys\nopen('record.json','w').write(json.dumps(\
         {'cwd':os.getcwd(),'argv':sys.argv[1:],'pwd':os.environ.get('PWD')}))\n",
    );
    let terminal = Terminal::new(recorder, TerminalKind::GnomeTerminal).expect("absolute");
    let prepared = PreparedDirectory {
        uri: file_uri(&folder),
        path: folder.clone(),
        is_network: false,
    };

    let launched = launch_terminal(&prepared, &terminal, Sandbox::Host).expect("launched");

    wait_for_file(&folder.join("record.json"));
    let record: Value =
        serde_json::from_slice(&fs::read(folder.join("record.json")).expect("record")).expect("JSON");
    let folder_text = real_path(&folder).to_string_lossy().into_owned();
    assert_eq!(
        record["argv"],
        serde_json::json!([format!("--working-directory={folder_text}")])
    );
    assert_eq!(record["cwd"], serde_json::json!(folder_text));
    assert_eq!(record["pwd"], serde_json::json!(folder_text));
    assert_eq!(launched.terminal, "GNOME Terminal");
    assert!(!folder.join("PWNED").exists());
    assert!(!root.path().join("PWNED").exists());
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_immediate_failure_reported`
/// parity: OPEN-020
#[test]
fn a_terminal_that_fails_at_once_is_reported() {
    let root = temporary_folder();
    let failing = root.path().join("fail");
    install_program(&failing, "#!/bin/sh\nexit 7\n");
    let terminal = Terminal::new(failing, TerminalKind::XTerm).expect("absolute");
    let prepared = PreparedDirectory {
        uri: file_uri(root.path()),
        path: root.path().to_owned(),
        is_network: false,
    };

    let failed = launch_terminal(&prepared, &terminal, Sandbox::Host);

    let message = failed.expect_err("failed").to_string();
    assert!(message.contains("exit 7"), "{message}");
    assert!(message.starts_with("XTerm could not start"), "{message}");
}
