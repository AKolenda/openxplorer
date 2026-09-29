// SPDX-License-Identifier: AGPL-3.0-only
//! Setting up, removing and printing a managed mount: `main` of
//! `desktop/mount_share.py`.

use std::fs::{self, DirBuilder};
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::arguments::Arguments;
use super::host::{Account, Host};
use super::terminal::Terminal;
use super::{credential_file_text, write_new_file, MountHelperError};
use crate::network::{mount_plan, DesktopUser, MountPlan};

/// Where systemd reads administrator units.
const UNIT_FOLDER: &str = "/etc/systemd/system";
/// The folder holding the managed mount points.
const MOUNT_ROOT: &str = "/mnt/winspace";
/// How long one `systemctl` command may take.
const SYSTEMCTL_LIMIT: Duration = Duration::from_secs(55);
/// How long disabling the automount may take during a rollback.
const ROLLBACK_DISABLE_LIMIT: Duration = Duration::from_secs(20);
/// Mode of the credential file: root reads it, nobody else.
const CREDENTIAL_MODE: u32 = 0o600;
/// Mode of the unit files.
const UNIT_MODE: u32 = 0o644;
/// Mode of the credential folder.
const CREDENTIAL_FOLDER_MODE: u32 = 0o700;
/// Mode of the unit and mount-point folders.
const FOLDER_MODE: u32 = 0o755;
/// Mode of the mount point: nothing can be written there while the share
/// is not mounted.
const MOUNTPOINT_MODE: u32 = 0o555;

/// How a run that did not fail ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Outcome {
    /// The plan was printed, or the change was made.
    Done,
    /// The administrator did not type `SETUP` or `REMOVE`.
    Cancelled,
}

impl Outcome {
    /// The exit status: 0, or 1 when cancelled.
    pub(super) fn exit_status(self) -> u8 {
        match self {
            Self::Done => 0,
            Self::Cancelled => 1,
        }
    }
}

/// The files of one managed mount, inside the host's tree.
struct ManagedFiles {
    mount_unit: PathBuf,
    automount_unit: PathBuf,
    credentials: PathBuf,
    mountpoint: PathBuf,
}

impl ManagedFiles {
    fn of(plan: &MountPlan, host: &Host<'_>) -> Self {
        let units = Path::new(UNIT_FOLDER);
        Self {
            mount_unit: host.tree.path(&units.join(format!("{}.mount", plan.unit))),
            automount_unit: host.tree.path(&units.join(format!("{}.automount", plan.unit))),
            credentials: host.tree.path(&plan.credentials),
            mountpoint: host.tree.path(&plan.mountpoint),
        }
    }

    /// The unit and credential files, in the order they are written.
    fn files(&self) -> [&Path; 3] {
        [&self.credentials, &self.mount_unit, &self.automount_unit]
    }
}

/// Prints the plan for `arguments.share` and, unless only the plan was
/// asked for, sets up or removes the managed mount.
///
/// # Errors
///
/// A [`MountHelperError`] saying why nothing (or, after a failed rollback,
/// not everything) was changed.
pub(super) fn run(
    arguments: &Arguments,
    account: &Account,
    host: &mut Host<'_>,
    terminal: &mut dyn Terminal,
) -> Result<Outcome, MountHelperError> {
    let user = DesktopUser {
        uid: account.uid,
        gid: account.gid,
    };
    let plan = mount_plan(&arguments.share, user)?;
    terminal.say(&format!(
        "Network share: {}\nLinux path:    {}\nDesktop user:  {}\n",
        plan.share,
        plan.mountpoint.display(),
        account.name
    ));
    if arguments.plan_only {
        terminal.say(&format!("{}\n{}", plan.mount_unit, plan.automount_unit));
        return Ok(Outcome::Done);
    }
    if !host.is_administrator {
        return Err(MountHelperError::NotAdministrator);
    }
    if !terminal.is_interactive() {
        return Err(MountHelperError::NotATerminal);
    }
    let files = ManagedFiles::of(&plan, host);
    let credential_folder = files.credentials.parent().unwrap_or(&files.credentials);
    let unit_folder = files.mount_unit.parent().unwrap_or(&files.mount_unit);
    host.tree.secure_directory(unit_folder, FOLDER_MODE)?;
    host.tree
        .secure_directory(credential_folder, CREDENTIAL_FOLDER_MODE)?;
    host.tree
        .secure_directory(&host.tree.path(Path::new(MOUNT_ROOT)), FOLDER_MODE)?;
    if arguments.remove {
        remove(&plan, &files, host, terminal)
    } else {
        set_up(&plan, &files, host, terminal)
    }
}

/// Removes the managed mount's units and credential file, never a shared
/// or local file.
fn remove(
    plan: &MountPlan,
    files: &ManagedFiles,
    host: &mut Host<'_>,
    terminal: &mut dyn Terminal,
) -> Result<Outcome, MountHelperError> {
    terminal.say("First point Downloads/Documents elsewhere. Close files using this mount.");
    let owner = host.tree.owner();
    let is_managed = |path: &Path| {
        fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.uid() == owner)
    };
    if !files.files().into_iter().all(is_managed) {
        return Err(MountHelperError::UnsafeManagedFiles);
    }
    let unchanged = |path: &Path, text: &str| fs::read_to_string(path).is_ok_and(|read| read == text);
    if !unchanged(&files.mount_unit, &plan.mount_unit)
        || !unchanged(&files.automount_unit, &plan.automount_unit)
    {
        return Err(MountHelperError::EditedUnits);
    }
    let answer = ask(
        terminal,
        "Type REMOVE to disconnect this mount and forget its credential: ",
    )?;
    if answer != "REMOVE" {
        terminal.say("Cancelled.");
        return Ok(Outcome::Cancelled);
    }
    let (mount, automount) = unit_names(plan);
    // Fail closed: if the mount is busy, stopping fails and every file and
    // credential stays for recovery.
    host.systemd
        .systemctl(&["stop", &mount, &automount], SYSTEMCTL_LIMIT)?;
    host.systemd
        .systemctl(&["disable", &automount], SYSTEMCTL_LIMIT)?;
    for file in files.files() {
        fs::remove_file(file).map_err(|error| MountHelperError::io(file, error))?;
    }
    host.systemd.systemctl(&["daemon-reload"], SYSTEMCTL_LIMIT)?;
    // Never removes anything recursively: only an empty mount point goes.
    let _ = fs::remove_dir(&files.mountpoint);
    terminal.say("Mount configuration removed. No shared or local files deleted.");
    Ok(Outcome::Done)
}

/// Asks for the account, writes the credential file and units, and starts
/// the mount, rolling everything back if a step fails.
fn set_up(
    plan: &MountPlan,
    files: &ManagedFiles,
    host: &mut Host<'_>,
    terminal: &mut dyn Terminal,
) -> Result<Outcome, MountHelperError> {
    if !host.has_cifs_utils {
        return Err(MountHelperError::MissingCifsUtils);
    }
    let paths = [
        &files.mount_unit,
        &files.automount_unit,
        &files.credentials,
        &files.mountpoint,
    ];
    if paths.iter().any(|path| fs::symlink_metadata(path).is_ok()) {
        return Err(MountHelperError::AlreadyExists(plan.remove_command.clone()));
    }
    terminal.say("Creates two systemd units and a root-only credentials file. Uses SMB 3.0;");
    terminal.say("there is no SMB1 fallback. Does not change fstab or move existing files.");
    terminal.say("The credential file contains your SMB password in plaintext, readable by root.");
    terminal.say("Share access is intended for this Linux user. Administrators can still access it.");
    if ask(terminal, "Type SETUP to continue: ")? != "SETUP" {
        terminal.say("Cancelled.");
        return Ok(Outcome::Cancelled);
    }
    let username = ask(terminal, "SMB username (DOMAIN\\username is optional): ")?;
    let password = terminal
        .ask_secret("SMB password: ")
        .map_err(MountHelperError::Terminal)?;
    let credential = credential_file_text(&username, &password)?;
    drop(password);
    let mut created = Vec::new();
    let result = create_and_start(plan, files, credential, host, &mut created);
    if let Err(error) = result {
        roll_back(plan, files, &created, host, terminal)?;
        return Err(error);
    }
    terminal.say(&format!(
        "\nMounted at {}\nIn OpenXplorer: Downloads → Properties → Location → enter this path → Check → Apply.",
        plan.mountpoint.display()
    ));
    terminal.say(
        "The server must be online for new downloads. Browser-specific download settings may also need \
         updating.",
    );
    terminal.say(&format!(
        "Removal (after restoring folder locations): {}",
        plan.remove_command
    ));
    Ok(Outcome::Done)
}

/// Creates the mount point and the files, recording each created file in
/// `created`, then enables the automount and mounts once to check the
/// account now rather than at the first download.
fn create_and_start(
    plan: &MountPlan,
    files: &ManagedFiles,
    credential: String,
    host: &mut Host<'_>,
    created: &mut Vec<PathBuf>,
) -> Result<(), MountHelperError> {
    DirBuilder::new()
        .mode(MOUNTPOINT_MODE)
        .create(&files.mountpoint)
        .map_err(|error| MountHelperError::io(&files.mountpoint, error))?;
    let contents = [
        (&files.credentials, credential, CREDENTIAL_MODE),
        (&files.mount_unit, plan.mount_unit.clone(), UNIT_MODE),
        (&files.automount_unit, plan.automount_unit.clone(), UNIT_MODE),
    ];
    for (path, text, mode) in contents {
        write_new_file(path, &text, mode)?;
        created.push(path.clone());
    }
    let (mount, automount) = unit_names(plan);
    host.systemd.systemctl(&["daemon-reload"], SYSTEMCTL_LIMIT)?;
    host.systemd
        .systemctl(&["enable", "--now", &automount], SYSTEMCTL_LIMIT)?;
    host.systemd.systemctl(&["start", &mount], SYSTEMCTL_LIMIT)
}

/// Stops and disables the new units and deletes what was `created`.
///
/// # Errors
///
/// The error of stopping the units, after which the configuration is kept
/// for manual recovery.
fn roll_back(
    plan: &MountPlan,
    files: &ManagedFiles,
    created: &[PathBuf],
    host: &mut Host<'_>,
    terminal: &mut dyn Terminal,
) -> Result<(), MountHelperError> {
    terminal.warn("Setup failed. Trying to stop and roll back the new mount configuration.");
    let (mount, automount) = unit_names(plan);
    if let Err(error) = host
        .systemd
        .systemctl(&["stop", &mount, &automount], SYSTEMCTL_LIMIT)
    {
        terminal.warn(&format!(
            "Could not stop safely. Configuration retained for manual recovery: {}",
            plan.remove_command
        ));
        return Err(error);
    }
    // Disabling may fail when enabling never happened; that is fine.
    let _ = host
        .systemd
        .systemctl(&["disable", &automount], ROLLBACK_DISABLE_LIMIT);
    for path in created.iter().rev() {
        let _ = fs::remove_file(path);
    }
    let _ = fs::remove_dir(&files.mountpoint);
    host.systemd.systemctl(&["daemon-reload"], SYSTEMCTL_LIMIT)
}

/// The `.mount` and `.automount` unit names of the plan.
fn unit_names(plan: &MountPlan) -> (String, String) {
    (format!("{}.mount", plan.unit), format!("{}.automount", plan.unit))
}

fn ask(terminal: &mut dyn Terminal, prompt: &str) -> Result<String, MountHelperError> {
    terminal.ask(prompt).map_err(MountHelperError::Terminal)
}
