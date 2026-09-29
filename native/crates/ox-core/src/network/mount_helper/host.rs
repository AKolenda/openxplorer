// SPDX-License-Identifier: AGPL-3.0-only
//! The computer the helper changes: the folders it writes in, systemd,
//! whether `mount.cifs` is installed, and the desktop account the mount
//! belongs to.

use std::env;
use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use super::{AdministrativeTree, MountHelperError, ROOT_UID};

/// The `systemctl` the helper runs, by absolute path, as `mount_share.py`
/// did.
const SYSTEMCTL: &str = "/usr/bin/systemctl";
/// The user database lookup, which also asks LDAP and other NSS sources,
/// as Python's `pwd.getpwuid` did.
const GETENT: &str = "/usr/bin/getent";
/// How often a running `systemctl` is checked for its exit.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// The desktop account that ran `sudo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Account {
    /// The login name.
    pub(super) name: String,
    /// The user ID; never root.
    pub(super) uid: u32,
    /// The primary group ID.
    pub(super) gid: u32,
}

/// Runs `systemctl` commands.
pub(super) trait Systemd {
    /// Runs `systemctl arguments…` and waits at most `limit` for it.
    ///
    /// # Errors
    ///
    /// [`MountHelperError::Systemctl`] when it cannot start, fails or
    /// takes longer than `limit`.
    fn systemctl(&mut self, arguments: &[&str], limit: Duration) -> Result<(), MountHelperError>;
}

/// The computer the helper runs on.
pub(super) struct Host<'a> {
    /// Where the helper's folders and files are, and who must own them.
    pub(super) tree: AdministrativeTree,
    /// Whether the helper runs as root, through `sudo`.
    pub(super) is_administrator: bool,
    /// Whether `mount.cifs` (cifs-utils) is installed.
    pub(super) has_cifs_utils: bool,
    /// Where systemd is told about the units.
    pub(super) systemd: &'a mut dyn Systemd,
}

impl<'a> Host<'a> {
    /// This computer, as the helper finds it.
    pub(super) fn system(systemd: &'a mut dyn Systemd) -> Self {
        Self {
            tree: AdministrativeTree::system(),
            is_administrator: rustix::process::geteuid().is_root(),
            has_cifs_utils: is_on_path("mount.cifs"),
            systemd,
        }
    }
}

/// The real `systemctl`.
pub(super) struct SystemSystemd;

impl Systemd for SystemSystemd {
    fn systemctl(&mut self, arguments: &[&str], limit: Duration) -> Result<(), MountHelperError> {
        let failure = |reason: String| MountHelperError::Systemctl {
            command: arguments.join(" "),
            reason,
        };
        let mut child = Command::new(SYSTEMCTL)
            .args(arguments)
            .spawn()
            .map_err(|error| failure(format!("could not start: {error}")))?;
        let deadline = Instant::now() + limit;
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => return Err(failure(format!("failed with {status}"))),
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(failure(format!("timed out after {} seconds", limit.as_secs())));
                }
                Ok(None) => thread::sleep(POLL_INTERVAL),
                Err(error) => return Err(failure(format!("could not be waited for: {error}"))),
            }
        }
    }
}

/// The account that ran `sudo` (`SUDO_UID`), or the one running the
/// helper.
///
/// # Errors
///
/// [`MountHelperError::RootLogin`] for root, and
/// [`MountHelperError::UnknownAccount`] for a user ID without an account.
pub(super) fn invoking_account() -> Result<Account, MountHelperError> {
    let uid = match env::var("SUDO_UID") {
        Ok(value) => value
            .trim()
            .parse::<u32>()
            .map_err(|_| MountHelperError::UnknownAccount(value.clone()))?,
        Err(_) => rustix::process::getuid().as_raw(),
    };
    if uid == ROOT_UID {
        return Err(MountHelperError::RootLogin);
    }
    let output = Command::new(GETENT)
        .args(["passwd", &uid.to_string()])
        .output()
        .map_err(|_| MountHelperError::UnknownAccount(uid.to_string()))?;
    let entry = String::from_utf8_lossy(&output.stdout);
    let account = output
        .status
        .success()
        .then(|| parse_passwd_entry(&entry))
        .flatten();
    account.ok_or_else(|| MountHelperError::UnknownAccount(uid.to_string()))
}

/// Reads the account from a user database line such as
/// `sam:x:1000:1000:Sam,,,:/home/sam:/bin/bash`.
pub(super) fn parse_passwd_entry(line: &str) -> Option<Account> {
    let fields: Vec<&str> = line.trim_end_matches('\n').split(':').collect();
    let [name, _, uid, gid, ..] = fields.as_slice() else {
        return None;
    };
    Some(Account {
        name: (*name).to_owned(),
        uid: uid.parse().ok()?,
        gid: gid.parse().ok()?,
    })
}

/// Whether an executable file `program` is in one of the `PATH` folders,
/// as Python's `shutil.which` looked.
fn is_on_path(program: &str) -> bool {
    let path = env::var_os("PATH").unwrap_or_default();
    env::split_paths(&path).any(|folder| is_executable(&folder.join(OsStr::new(program))))
}

fn is_executable(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}
