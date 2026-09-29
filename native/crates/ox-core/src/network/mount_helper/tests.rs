// SPDX-License-Identifier: AGPL-3.0-only
//! The helper's file rules and its whole run, inside a temporary folder
//! that stands for `/`, with systemd and the terminal recorded. Nothing is
//! mounted and no real systemd unit is touched.

use std::collections::VecDeque;
use std::fs;
use std::io;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::time::Duration;

use super::arguments::{parse, Arguments, Request, UsageError};
use super::host::{parse_passwd_entry, Account, Host, Systemd};
use super::setup::{run, Outcome};
use super::terminal::Terminal;
use super::*;
use crate::network::{mount_plan, DesktopUser};

/// The unit name of `//nas/Downloads` for [`account`].
const UNIT: &str = "mnt-winspace-u1000sd14e42e53b";

fn account() -> Account {
    Account {
        name: "sam".into(),
        uid: 1000,
        gid: 1000,
    }
}

fn arguments(remove: bool, plan_only: bool) -> Arguments {
    Arguments {
        share: "//nas/Downloads".into(),
        remove,
        plan_only,
    }
}

/// A terminal that answers from a script and records what was printed.
#[derive(Default)]
struct ScriptedTerminal {
    answers: VecDeque<&'static str>,
    printed: String,
    warnings: String,
    not_a_terminal: bool,
}

impl ScriptedTerminal {
    fn answering(answers: &[&'static str]) -> Self {
        Self {
            answers: answers.iter().copied().collect(),
            ..Self::default()
        }
    }
}

impl Terminal for ScriptedTerminal {
    fn is_interactive(&self) -> bool {
        !self.not_a_terminal
    }

    fn say(&mut self, text: &str) {
        self.printed.push_str(text);
        self.printed.push('\n');
    }

    fn warn(&mut self, text: &str) {
        self.warnings.push_str(text);
        self.warnings.push('\n');
    }

    fn ask(&mut self, prompt: &str) -> io::Result<String> {
        self.printed.push_str(prompt);
        Ok(self.answers.pop_front().unwrap_or_default().to_owned())
    }

    fn ask_secret(&mut self, prompt: &str) -> io::Result<String> {
        self.ask(prompt)
    }
}

/// Records each `systemctl` command and fails the one starting with
/// `failing`.
#[derive(Default)]
struct RecordedSystemd {
    commands: Vec<String>,
    failing: Option<&'static str>,
}

impl Systemd for RecordedSystemd {
    fn systemctl(&mut self, arguments: &[&str], _limit: Duration) -> Result<(), MountHelperError> {
        let command = arguments.join(" ");
        self.commands.push(command.clone());
        match self.failing {
            Some(failing) if command.starts_with(failing) => Err(MountHelperError::Systemctl {
                command,
                reason: "failed with exit status: 1".into(),
            }),
            _ => Ok(()),
        }
    }
}

/// A temporary folder standing for `/`, owned by the test's user.
struct FakeRoot {
    folder: tempfile::TempDir,
}

impl FakeRoot {
    fn new() -> Self {
        let folder = tempfile::tempdir().expect("temporary folder");
        // Only its owner may write in it, whatever the umask.
        fs::set_permissions(folder.path(), fs::Permissions::from_mode(0o700)).expect("private folder");
        Self { folder }
    }

    fn tree(&self) -> AdministrativeTree {
        AdministrativeTree {
            root: self.folder.path().to_path_buf(),
            owner: rustix::process::getuid().as_raw(),
        }
    }

    fn host<'a>(&self, systemd: &'a mut RecordedSystemd) -> Host<'a> {
        Host {
            tree: self.tree(),
            is_administrator: true,
            has_cifs_utils: true,
            systemd,
        }
    }

    fn path(&self, system_path: &str) -> PathBuf {
        self.tree().path(Path::new(system_path))
    }

    fn mount_unit(&self) -> PathBuf {
        self.path(&format!("/etc/systemd/system/{UNIT}.mount"))
    }

    fn automount_unit(&self) -> PathBuf {
        self.path(&format!("/etc/systemd/system/{UNIT}.automount"))
    }

    fn credentials(&self) -> PathBuf {
        self.path("/etc/winspace/mount-credentials/u1000sd14e42e53b")
    }

    fn mountpoint(&self) -> PathBuf {
        self.path("/mnt/winspace/u1000sd14e42e53b")
    }

    /// Sets up `//nas/Downloads` as `OFFICE\sam`.
    fn set_up(&self) -> (Outcome, RecordedSystemd) {
        let mut systemd = RecordedSystemd::default();
        let mut terminal = ScriptedTerminal::answering(&["SETUP", "OFFICE\\sam", "secret"]);
        let outcome = run(
            &arguments(false, false),
            &account(),
            &mut self.host(&mut systemd),
            &mut terminal,
        );
        (outcome.expect("set up"), systemd)
    }
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).expect("exists").permissions().mode() & 0o777
}

/// Ported from `desktop/tests/test_terminal_security.py::AdditionalSecurityTests::test_admin_helper_checks_existing_parent_chain`
///
/// The temporary folder's parents (such as the world-writable `/tmp`,
/// or a home owned by the user) are unsafe even though the directory
/// itself looks private.
///
/// parity: NET-028, SAFE-021
#[test]
fn an_unsafe_parent_chain_is_refused() {
    let root = tempfile::tempdir().expect("temporary folder");
    let owned = root.path().join("owned");
    DirBuilder::new()
        .mode(0o700)
        .create(&owned)
        .expect("private folder");

    let refused = AdministrativeTree::system().secure_directory(&owned, 0o700);

    assert!(
        matches!(refused, Err(MountHelperError::UnsafeDirectory(_))),
        "{refused:?}"
    );
}

/// parity: SAFE-021
#[test]
fn relative_and_traversing_paths_are_refused() {
    for path in ["etc/winspace", "/etc/../tmp"] {
        let refused = AdministrativeTree::system().secure_directory(Path::new(path), 0o755);
        assert!(
            matches!(refused, Err(MountHelperError::NotAbsolute)),
            "{path}: {refused:?}"
        );
    }
}

/// parity: NET-028, SAFE-021
#[test]
fn a_new_file_gets_its_mode_and_never_replaces_anything() {
    let root = tempfile::tempdir().expect("temporary folder");
    let credential = root.path().join("credential");

    write_new_file(&credential, "username=sam\n", 0o600).expect("a new file");

    assert_eq!(mode(&credential), 0o600);
    assert!(write_new_file(&credential, "other", 0o600).is_err());
    assert_eq!(
        fs::read_to_string(&credential).expect("readable"),
        "username=sam\n"
    );
}

/// parity: SAFE-021
#[test]
fn a_symlink_is_never_followed() {
    let root = tempfile::tempdir().expect("temporary folder");
    let target = root.path().join("target");
    fs::write(&target, "unchanged").expect("target file");
    let link = root.path().join("link");
    symlink(&target, &link).expect("symlink");

    assert!(write_new_file(&link, "password=x\n", 0o600).is_err());
    assert_eq!(fs::read_to_string(&target).expect("readable"), "unchanged");
}

/// parity: NET-028
#[test]
fn the_credential_file_names_user_password_and_optional_domain() {
    assert_eq!(
        credential_file_text(" OFFICE\\sam ", "secret").expect("valid"),
        "username=sam\npassword=secret\ndomain=OFFICE\n"
    );
    assert_eq!(
        credential_file_text("sam", "secret").expect("valid"),
        "username=sam\npassword=secret\n"
    );
}

/// parity: NET-028
#[test]
fn credentials_that_would_break_the_file_are_refused() {
    let invalid = [("", "secret"), ("sam", "line\nbreak"), ("sam\0", "secret")];
    for (username, password) in invalid {
        let refused = credential_file_text(username, password);
        assert!(
            matches!(refused, Err(MountHelperError::InvalidCredentials)),
            "{username:?}"
        );
    }
    let without_user = credential_file_text("OFFICE\\", "secret");
    assert!(matches!(without_user, Err(MountHelperError::MissingUsername)));
}

/// The command line of `mount_share.py`: `--share` is required, `--plan`
/// and `--remove` are flags, and `-h` answers first.
///
/// parity: NET-028
#[test]
fn the_command_line_matches_the_python_helper() {
    let parsed = |words: &[&str]| parse(words.iter().map(|word| (*word).to_owned()));

    assert_eq!(
        parsed(&["--share", "//nas/Downloads", "--remove"]),
        Ok(Request::Run(arguments(true, false)))
    );
    assert_eq!(
        parsed(&["--plan", "--share=//nas/Downloads"]),
        Ok(Request::Run(arguments(false, true)))
    );
    assert_eq!(parsed(&["--share", "x", "-h"]), Ok(Request::Help));
    assert_eq!(parsed(&["--plan"]), Err(UsageError::MissingShare));
    assert_eq!(parsed(&["--share"]), Err(UsageError::MissingValue));
    assert_eq!(
        parsed(&["--share", "x", "--force"]),
        Err(UsageError::Unrecognized("--force".into()))
    );
}

#[test]
fn the_desktop_account_is_read_from_the_user_database() {
    assert_eq!(
        parse_passwd_entry("sam:x:1000:1000:Sam,,,:/home/sam:/bin/bash\n"),
        Some(account())
    );
    assert_eq!(parse_passwd_entry("broken"), None);
}

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

/// Removal needs REMOVE and units exactly as written; it deletes the
/// configuration and an empty mount point, never anything else.
///
/// parity: NET-028
#[test]
fn removal_needs_unchanged_units_and_deletes_only_the_configuration() {
    let root = FakeRoot::new();
    root.set_up();
    let mut systemd = RecordedSystemd::default();
    let mut terminal = ScriptedTerminal::answering(&["remove"]);
    let kept = run(
        &arguments(true, false),
        &account(),
        &mut root.host(&mut systemd),
        &mut terminal,
    );
    assert_eq!(kept.expect("cancelled"), Outcome::Cancelled);
    assert!(root.credentials().exists());

    let mut terminal = ScriptedTerminal::answering(&["REMOVE"]);
    let removed = run(
        &arguments(true, false),
        &account(),
        &mut root.host(&mut systemd),
        &mut terminal,
    );

    assert_eq!(removed.expect("removed"), Outcome::Done);
    for path in [
        root.credentials(),
        root.mount_unit(),
        root.automount_unit(),
        root.mountpoint(),
    ] {
        assert!(!path.exists(), "{} was removed", path.display());
    }
    assert_eq!(
        systemd.commands,
        [
            format!("stop {UNIT}.mount {UNIT}.automount"),
            format!("disable {UNIT}.automount"),
            "daemon-reload".to_owned(),
        ]
    );

    root.set_up();
    fs::write(root.automount_unit(), "edited").expect("edited unit");
    let mut terminal = ScriptedTerminal::answering(&["REMOVE"]);
    let refused = run(
        &arguments(true, false),
        &account(),
        &mut root.host(&mut systemd),
        &mut terminal,
    );
    assert!(
        matches!(refused, Err(MountHelperError::EditedUnits)),
        "{refused:?}"
    );
    assert!(root.credentials().exists());
}
