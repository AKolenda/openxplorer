// SPDX-License-Identifier: AGPL-3.0-only
//! The persistent network mount assistant's plan: a reviewable systemd SMB
//! automount for a share without a stable Linux path.
//!
//! Ports `mount_plan` in `desktop/mount_support.py`. Planning writes
//! nothing and mounts nothing; the plan's command runs the administrator
//! helper (`openxplorer-mount-share`, in `mount_helper`)
//! in the user's own terminal.

use std::path::PathBuf;

use crate::location::{require_share, split_location, unquote_lossy, LocationError};

/// The administrator helper the plan's commands run.
const HELPER: &str = "/usr/bin/openxplorer-mount-share";
/// The folder holding the managed mount points.
const MOUNT_ROOT: &str = "/mnt/winspace";
/// The root-only folder holding the managed credential files.
const CREDENTIAL_ROOT: &str = "/etc/winspace/mount-credentials";
/// The first line of every file the helper manages.
const MANAGED_MARKER: &str = "# Managed by OpenXplorer's explicit mount setup tool.";

/// A reviewable plan for an on-demand SMB 3.0 mount of one share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountPlan {
    /// The share as `mount.cifs` names it: `//server/share`.
    pub share: String,
    /// `u<uid>s<hash>`: names the mount point, units and credential file.
    pub key: String,
    /// Where the share is mounted: `/mnt/winspace/<key>`.
    pub mountpoint: PathBuf,
    /// The Linux path of the planned location, inside the mount point.
    pub target_path: PathBuf,
    /// The systemd unit name, without `.mount` or `.automount`.
    pub unit: String,
    /// The root-only credential file the helper writes.
    pub credentials: PathBuf,
    /// The text of the `.mount` unit.
    pub mount_unit: String,
    /// The text of the `.automount` unit.
    pub automount_unit: String,
    /// The setup command for the user's terminal. It holds no password.
    pub command: String,
    /// The command that removes the managed mount again.
    pub remove_command: String,
}

/// Why the assistant cannot plan a mount, in the assistant's wording.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MountPlanError {
    /// Not an SMB shared folder.
    #[error(transparent)]
    Location(#[from] LocationError),
    /// A port, or a host name the unit files cannot carry.
    #[error("The persistent mount assistant supports a hostname or IPv4 address without a port.")]
    UnsupportedServer,
    /// A share name outside the allowed characters.
    #[error(
        "This share name needs manual mounting. The assistant allows letters, numbers, spaces, dots, _, $, \
         and hyphens."
    )]
    UnsupportedShareName,
    /// An empty, `.` or `..` path component, or a root or invalid user.
    #[error("Invalid mount destination or user identity.")]
    InvalidDestination,
}

/// The desktop account the mount belongs to: its files appear owned by
/// this user and group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopUser {
    /// The user id; root (0) is refused.
    pub uid: u32,
    /// The primary group id.
    pub gid: u32,
}

impl DesktopUser {
    /// The user running the app.
    pub fn current() -> Self {
        Self {
            uid: rustix::process::getuid().as_raw(),
            gid: rustix::process::getgid().as_raw(),
        }
    }
}

/// Plans an on-demand mount of `address` for `user`.
///
/// Safety rule (SAFE-021): deliberately narrower than SMB addressing in
/// general. No port, a plain host name or IPv4 address, and share names of
/// letters, digits, spaces and `._$-`, so no user-provided syntax,
/// credential or mount path reaches the unit files or mount options.
///
/// # Errors
///
/// A [`MountPlanError`] saying what the assistant cannot handle.
pub fn mount_plan(address: &str, user: DesktopUser) -> Result<MountPlan, MountPlanError> {
    let uri = require_share(address)?;
    let parts = split_location(&uri)?;
    let server = parts.hostname().unwrap_or_default();
    if parts.port()?.is_some() || !is_plain_host_name(&server) {
        return Err(MountPlanError::UnsupportedServer);
    }
    let decoded = unquote_lossy(&parts.path);
    let components: Vec<&str> = decoded.trim_matches('/').split('/').collect();
    let Some((share, folders)) = components.split_first() else {
        return Err(MountPlanError::InvalidDestination);
    };
    if !is_plain_share_name(share) {
        return Err(MountPlanError::UnsupportedShareName);
    }
    let has_bad_component = components
        .iter()
        .any(|component| matches!(*component, "" | "." | ".."));
    if has_bad_component || user.uid == 0 {
        return Err(MountPlanError::InvalidDestination);
    }
    Ok(plan_for(&server, share, folders, user))
}

/// Builds the plan for a validated `server` and `share`.
fn plan_for(server: &str, share: &str, folders: &[&str], user: DesktopUser) -> MountPlan {
    // Safety rule (SAFE-021): hash-based names avoid systemd unit escaping
    // and option injection.
    let key = format!("u{}s{}", user.uid, share_identity(server, share));
    let mountpoint = format!("{MOUNT_ROOT}/{key}");
    let credentials = format!("{CREDENTIAL_ROOT}/{key}");
    let source = format!("//{server}/{share}");
    let mut target_path = PathBuf::from(&mountpoint);
    target_path.extend(folders);
    let command = shell_join(&["sudo", HELPER, "--share", &source]);
    let remove_command = shell_join(&["sudo", HELPER, "--share", &source, "--remove"]);
    MountPlan {
        mount_unit: mount_unit(&key, &source, &mountpoint, &cifs_options(&credentials, user)),
        automount_unit: automount_unit(&key, &mountpoint),
        unit: format!("mnt-winspace-{key}"),
        share: source,
        key,
        mountpoint: PathBuf::from(mountpoint),
        target_path,
        credentials: PathBuf::from(credentials),
        command,
        remove_command,
    }
}

/// A host name or IPv4 address: `[A-Za-z0-9][A-Za-z0-9.-]{0,252}`.
fn is_plain_host_name(server: &str) -> bool {
    let mut characters = server.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    let rest_is_allowed = characters.all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    first.is_ascii_alphanumeric() && rest_is_allowed && server.len() <= 253
}

/// A share name the assistant handles: `[A-Za-z0-9][A-Za-z0-9 ._$-]{0,79}`.
fn is_plain_share_name(share: &str) -> bool {
    let mut characters = share.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    let rest_is_allowed = characters.all(|c| c.is_ascii_alphanumeric() || " ._$-".contains(c));
    first.is_ascii_alphanumeric() && rest_is_allowed && share.len() <= 80
}

/// The first 10 hex digits of the SHA-256 of `server/share`, case-folded.
/// Both are ASCII once validated, so ASCII lower case is their case fold.
fn share_identity(server: &str, share: &str) -> String {
    let identity = format!("{}/{}", server.to_ascii_lowercase(), share.to_ascii_lowercase());
    let digest = glib::compute_checksum_for_string(glib::ChecksumType::Sha256, &identity)
        .expect("SHA-256 is always available in GLib");
    digest[..10].to_owned()
}

/// The `mount.cifs` options.
///
/// Safety rule (NET-028): SMB 3.0 only, with no SMB1 fallback; every file
/// belongs to `user` and only they may read it; no set-uid programs,
/// devices or executables.
fn cifs_options(credentials: &str, user: DesktopUser) -> String {
    let DesktopUser { uid, gid } = user;
    format!(
        "credentials={credentials},uid={uid},gid={gid},file_mode=0600,dir_mode=0700,forceuid,forcegid,\
         nosuid,nodev,noexec,vers=3.0,_netdev"
    )
}

/// The `.mount` unit: mounts `source` at `mountpoint` with `options`.
fn mount_unit(key: &str, source: &str, mountpoint: &str, options: &str) -> String {
    format!(
        "{MANAGED_MARKER}\n\
         [Unit]\n\
         Description=OpenXplorer SMB mount {key}\n\
         [Mount]\n\
         What={source}\n\
         Where={mountpoint}\n\
         Type=cifs\n\
         Options={options}\n\
         TimeoutSec=20\n"
    )
}

/// The `.automount` unit: mounts on first access, unmounts after five idle
/// minutes.
fn automount_unit(key: &str, mountpoint: &str) -> String {
    format!(
        "{MANAGED_MARKER}\n\
         [Unit]\n\
         Description=OpenXplorer on-demand SMB mount {key}\n\
         [Automount]\n\
         Where={mountpoint}\n\
         TimeoutIdleSec=300\n\
         [Install]\n\
         WantedBy=multi-user.target\n"
    )
}

/// Python's `shlex.join`: each word quoted for a POSIX shell when needed.
fn shell_join(words: &[&str]) -> String {
    let quoted: Vec<String> = words.iter().map(|word| shell_quote(word)).collect();
    quoted.join(" ")
}

/// Python's `shlex.quote`.
fn shell_quote(word: &str) -> String {
    let is_safe = |c: char| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c);
    if !word.is_empty() && word.chars().all(is_safe) {
        return word.to_owned();
    }
    format!("'{}'", word.replace('\'', r#"'"'"'"#))
}

#[cfg(test)]
mod tests;
