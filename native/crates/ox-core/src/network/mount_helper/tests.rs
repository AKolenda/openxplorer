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

mod files;
mod removal;
mod setup;
