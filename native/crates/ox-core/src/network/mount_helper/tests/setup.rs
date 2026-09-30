// SPDX-License-Identifier: AGPL-3.0-only
//! Printing the plan and setting the mount up, cancelled or rolled back.

use super::*;

/// `--plan` prints the share, the Linux path, the account and both units,
/// without sudo and without changing anything.
///
/// parity: NET-028
#[test]
fn the_plan_is_printed_without_any_change() {
    let root = FakeRoot::new();
    let mut systemd = RecordedSystemd::default();
    let mut host = root.host(&mut systemd);
    host.is_administrator = false;
    let mut terminal = ScriptedTerminal::default();

    let outcome = run(&arguments(false, true), &account(), &mut host, &mut terminal);

    assert_eq!(outcome.expect("printed"), Outcome::Done);
    assert!(terminal.printed.starts_with(
        "Network share: //nas/Downloads\nLinux path:    /mnt/winspace/u1000sd14e42e53b\nDesktop user:  sam\n\n"
    ));
    assert!(terminal.printed.contains("[Automount]"), "{}", terminal.printed);
    assert!(systemd.commands.is_empty());
    assert!(!root.path("/etc").exists());
}

/// A change needs sudo and a terminal to confirm it in.
///
/// parity: NET-028, SAFE-021
#[test]
fn a_change_needs_an_administrator_and_a_terminal() {
    let root = FakeRoot::new();
    let mut systemd = RecordedSystemd::default();
    let mut host = root.host(&mut systemd);
    host.is_administrator = false;
    let refused = run(
        &arguments(false, false),
        &account(),
        &mut host,
        &mut ScriptedTerminal::default(),
    );
    assert!(
        matches!(refused, Err(MountHelperError::NotAdministrator)),
        "{refused:?}"
    );

    let mut host = root.host(&mut systemd);
    let mut terminal = ScriptedTerminal {
        not_a_terminal: true,
        ..ScriptedTerminal::default()
    };
    let refused = run(&arguments(false, false), &account(), &mut host, &mut terminal);
    assert!(
        matches!(refused, Err(MountHelperError::NotATerminal)),
        "{refused:?}"
    );
}

/// After SETUP and the account, the helper writes the root-only
/// credential file and both units exclusively, creates a read-only mount
/// point, enables the automount and mounts once.
///
/// parity: NET-028, SAFE-021
#[test]
fn setup_writes_the_credential_and_units_then_starts_the_mount() {
    let root = FakeRoot::new();

    let (outcome, systemd) = root.set_up();

    assert_eq!(outcome, Outcome::Done);
    assert_eq!(
        fs::read_to_string(root.credentials()).expect("credential file"),
        "username=sam\npassword=secret\ndomain=OFFICE\n"
    );
    assert_eq!(mode(&root.credentials()), 0o600);
    assert_eq!(mode(root.credentials().parent().expect("its folder")), 0o700);
    assert_eq!(mode(&root.mount_unit()), 0o644);
    assert_eq!(mode(&root.mountpoint()), 0o555);
    let plan = mount_plan("//nas/Downloads", DesktopUser { uid: 1000, gid: 1000 }).expect("a plan");
    assert_eq!(
        fs::read_to_string(root.mount_unit()).expect("unit"),
        plan.mount_unit
    );
    assert_eq!(
        systemd.commands,
        [
            "daemon-reload".to_owned(),
            format!("enable --now {UNIT}.automount"),
            format!("start {UNIT}.mount"),
        ]
    );
}

/// Anything but SETUP changes nothing, and an existing unit, credential
/// or mount point is never overwritten.
///
/// parity: NET-028
#[test]
fn setup_is_cancelled_or_refused_without_touching_existing_files() {
    let root = FakeRoot::new();
    let mut systemd = RecordedSystemd::default();
    let mut terminal = ScriptedTerminal::answering(&["setup"]);
    let outcome = run(
        &arguments(false, false),
        &account(),
        &mut root.host(&mut systemd),
        &mut terminal,
    );
    assert_eq!(outcome.expect("cancelled"), Outcome::Cancelled);
    assert!(!root.mountpoint().exists());

    fs::write(root.mount_unit(), "someone else's unit").expect("existing unit");
    let mut terminal = ScriptedTerminal::answering(&["SETUP", "sam", "secret"]);
    let refused = run(
        &arguments(false, false),
        &account(),
        &mut root.host(&mut systemd),
        &mut terminal,
    );

    let Err(MountHelperError::AlreadyExists(removal)) = refused else {
        panic!("{refused:?}");
    };
    assert!(removal.ends_with("--share //nas/Downloads --remove"), "{removal}");
    assert_eq!(
        fs::read_to_string(root.mount_unit()).expect("unit"),
        "someone else's unit"
    );
    assert!(systemd.commands.is_empty());
}

/// When the first mount fails, the units are stopped and disabled and
/// every created file and the mount point are removed.
///
/// parity: NET-028
#[test]
fn a_failed_mount_rolls_the_setup_back() {
    let root = FakeRoot::new();
    let mut systemd = RecordedSystemd {
        failing: Some("start"),
        ..RecordedSystemd::default()
    };
    let mut terminal = ScriptedTerminal::answering(&["SETUP", "sam", "wrong"]);

    let failed = run(
        &arguments(false, false),
        &account(),
        &mut root.host(&mut systemd),
        &mut terminal,
    );

    assert!(
        matches!(failed, Err(MountHelperError::Systemctl { .. })),
        "{failed:?}"
    );
    for path in [
        root.credentials(),
        root.mount_unit(),
        root.automount_unit(),
        root.mountpoint(),
    ] {
        assert!(!path.exists(), "{} was rolled back", path.display());
    }
    assert_eq!(
        systemd.commands[3..],
        [
            format!("stop {UNIT}.mount {UNIT}.automount"),
            format!("disable {UNIT}.automount"),
            "daemon-reload".to_owned(),
        ]
    );
    assert!(
        terminal.warnings.starts_with("Setup failed."),
        "{}",
        terminal.warnings
    );
}
