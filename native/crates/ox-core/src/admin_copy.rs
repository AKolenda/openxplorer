// SPDX-License-Identifier: AGPL-3.0-only
//! Finishing a copy or move as administrator: Windows' Continue in
//! "Destination Folder Access Denied" (OPS-047).
//!
//! Ports no Python module. The app never runs as root and adds no
//! privilege of its own here: the item is copied again into the same
//! folder under `admin://`, the `GVfs` location "Open as administrator"
//! already uses, so `GVfs` and polkit ask for the administrator's
//! password and `gvfsd-admin` writes. What the copy replaces, a whole
//! folder included, is still removed through [`crate::admin_delete`]'s
//! helper.
//!
//! **Threat model.** `gvfsd-admin` writes as root by path, resolving the
//! destination from `/` again at every step. Another local account that
//! can rename, create or remove a name on that path could swap a folder
//! for a symbolic link between two steps, and root would then write
//! wherever the link leads. A drive an ordinary user mounted, or could
//! mount there, could do the same through owners and links it made up.
//!
//! So a destination is written as administrator only when nobody but root
//! can change any folder on its path, as 2.0.12's refusal of `admin://`
//! deletes required (#192, #194). It is refused, before anything is
//! written or any password is asked for, when:
//!
//! - any folder on its path from `/`, or the destination itself, is not
//!   owned by root or has the group-write or other-write bit (a POSIX ACL
//!   that lets anybody else write shows as group write). The sticky bit
//!   does not redeem one. Owner and mode are read without following links.
//! - a symbolic link on its path is in a folder that fails that test. A
//!   link in a folder that passes is followed (Fedora Atomic's `/home`),
//!   and where it leads is judged the same way, up to [`MAX_LINKS`] links.
//! - a folder on its path is on a filesystem whose owners and modes an
//!   ordinary user could have written (anything but
//!   [`TRUSTED_FILESYSTEMS`], ZFS with `NFSv4` ACLs, anything under
//!   `/media` or `/run/media`, a loop device), or on a drive a user could
//!   have mounted ([`user_mounts`]), or under a mount point a user may
//!   mount a drive on.
//! - such a drive is mounted inside the destination, or a user may mount
//!   one there.
//! - the app runs in a Flatpak, whose view of the filesystem is not the
//!   one `gvfsd-admin` writes to, or the destination is not a folder on
//!   this computer.
//!
//! Because only root can change what passed, nothing can change between
//! this check and the copy that would make it wrong.

mod user_mounts;

use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use gio::prelude::*;

use crate::i18n::{format_message, gettext};
use crate::integration::Sandbox;
use crate::sizes::mounts::{self, Mount};
use crate::transfer::TransferError;

use user_mounts::UserMounts;

/// The most symbolic links followed on the way to the destination, as the
/// kernel allows (`MAXSYMLINKS`).
const MAX_LINKS: usize = 40;

/// The only account trusted to own the folders on the path.
const ROOT: u32 = 0;

/// The group-write and other-write bits.
const WRITABLE_BY_OTHERS: u32 = 0o022;

/// The filesystems whose owners and modes only the kernel writes, as
/// 2.0.12 trusted them.
const TRUSTED_FILESYSTEMS: &[&str] = &[
    "ext2", "ext3", "ext4", "xfs", "btrfs", "f2fs", "bcachefs", "jfs", "zfs", "tmpfs", "ramfs", "overlay",
];

/// Where desktop users mount their own drives and disk images (`udisks2`).
const USER_MOUNT_FOLDERS: &[&str] = &["/media", "/run/media"];

/// The scheme of `GVfs`' administrator locations.
const ADMIN_SCHEME: &str = "admin";

/// The `admin://` URI of the folder at `folder_uri`, a `file:` folder on
/// this computer, once it is safe to write there as administrator. The URI
/// names the folder with every link on its path resolved, so `gvfsd-admin`
/// writes to the very folder that was checked.
///
/// # Errors
///
/// [`TransferError::NotSupported`] when this system has no `admin://`
/// location or the folder is not on this computer, and a refusal naming
/// the folder when the module documentation's rules refuse it.
pub fn elevated_destination(folder_uri: &str) -> Result<String, TransferError> {
    if !admin_supported() {
        return Err(no_admin());
    }
    elevated_destination_with(folder_uri, &Rules::default())
}

/// Whether `GVfs` offers `admin://` here.
fn admin_supported() -> bool {
    gio::Vfs::default()
        .supported_uri_schemes()
        .iter()
        .any(|scheme| scheme == ADMIN_SCHEME)
}

/// The refusal without `admin://`.
fn no_admin() -> TransferError {
    TransferError::NotSupported(gettext(
        "Administrator access is not available. Install your distribution's GVfs \
         administrator backend to change this folder as administrator.",
    ))
}

/// [`elevated_destination`] under `rules`, without asking whether `GVfs`
/// has `admin://`.
fn elevated_destination_with(folder_uri: &str, rules: &Rules) -> Result<String, TransferError> {
    // An `admin://` folder is checked by its path like any other.
    let path = local_folder(&as_file_uri(folder_uri))?;
    let resolved = Checks::new(rules, &path)?.check(&path)?;
    admin_uri_of(&resolved)
}

/// The `admin://` URI under which the item at `uri`, a `file:` item on
/// this computer, is deleted as administrator: Continue in a Recycle Bin or
/// delete run (OPS-047), once the folder that holds it passes the module
/// documentation's rules. Only root can then change that folder or its
/// path, so `gvfsd-admin`'s removal of a file by its path cannot be
/// redirected; a folder is removed by [`crate::admin_delete`]'s helper,
/// which walks it pinned. The URI names the item with every link on its
/// folder's path resolved.
///
/// # Errors
///
/// As [`elevated_destination`]'s refusals, and a refusal for a filesystem
/// root.
pub fn elevated_item(uri: &str) -> Result<String, TransferError> {
    if !admin_supported() {
        return Err(no_admin());
    }
    elevated_item_with(uri, &Rules::default())
}

/// [`elevated_item`] under `rules`, without asking whether `GVfs` has
/// `admin://`.
fn elevated_item_with(uri: &str, rules: &Rules) -> Result<String, TransferError> {
    let path = local_folder(&as_file_uri(uri))?;
    let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(TransferError::failed(gettext(
            "Filesystem roots cannot be copied, moved or trashed as items.",
        )));
    };
    let resolved = Checks::new(rules, folder)?.check(folder)?;
    admin_uri_of(&resolved.join(name))
}

/// The `admin://` URI of the local `path`.
fn admin_uri_of(path: &Path) -> Result<String, TransferError> {
    let uri = glib::filename_to_uri(path, None).map_err(|error| TransferError::failed(error.to_string()))?;
    let local = uri.strip_prefix("file://").unwrap_or(&uri);
    Ok(format!("{ADMIN_SCHEME}://{local}"))
}

/// Checks that the existing folder at `uri`, under `admin://` or `file:`,
/// may be written as administrator by the rules of the module
/// documentation: the folder a Replace run as administrator merges into
/// (OPS-047). Once it passes, only root can change it or any folder on its
/// path, so the merge's writes into it cannot be redirected.
///
/// # Errors
///
/// As [`elevated_destination`]'s refusals.
pub fn check_elevated_folder(uri: &str) -> Result<(), TransferError> {
    check_elevated_folder_with(uri, &Rules::default())
}

/// [`check_elevated_folder`] under `rules`.
fn check_elevated_folder_with(uri: &str, rules: &Rules) -> Result<(), TransferError> {
    let path = local_folder(&as_file_uri(uri))?;
    Checks::new(rules, &path)?.check(&path)?;
    Ok(())
}

/// `uri` with an `admin://` scheme read as the `file:` location it names;
/// any other URI as it is.
pub fn as_file_uri(uri: &str) -> String {
    match uri.strip_prefix("admin://") {
        Some(rest) => format!("file://{rest}"),
        None => uri.to_owned(),
    }
}

/// The absolute path of the `file:` folder `uri` on this computer.
fn local_folder(uri: &str) -> Result<PathBuf, TransferError> {
    let not_local = || {
        TransferError::NotSupported(format_message(
            "{uri} is not on this computer, so it can't be changed as administrator.",
            &[("uri", uri)],
        ))
    };
    if !uri.starts_with("file:") {
        return Err(not_local());
    }
    let (path, host) = glib::filename_from_uri(uri).map_err(|_| not_local())?;
    if host.is_some_and(|host| !host.is_empty() && host != "localhost") || !path.is_absolute() {
        return Err(not_local());
    }
    Ok(path)
}

/// What the checks trust. Only tests can change it.
#[derive(Default)]
struct Rules {
    #[cfg(test)]
    seams: Seams,
}

/// The stand-ins tests use for what an ordinary user cannot set up: a
/// root-owned folder, a mount table, a system's `fstab` and `/sys`.
#[cfg(test)]
#[derive(Default)]
struct Seams {
    /// The owner trusted instead of root.
    owner: Option<u32>,
    /// The mount table read instead of this process's.
    mounts: Option<Vec<Mount>>,
    /// Folders above this one (the test's own folder) are not judged:
    /// they are the machine's, such as a world-writable `/tmp`.
    judged_from: Option<PathBuf>,
    /// The folder read in place of `/` for `/etc/fstab`, libmount's
    /// `utab`, `/sys` and `/dev`.
    system: Option<PathBuf>,
}

impl Rules {
    /// The account trusted to own every folder on the path.
    #[cfg_attr(not(test), allow(clippy::unused_self))]
    fn owner(&self) -> u32 {
        #[cfg(test)]
        if let Some(owner) = self.seams.owner {
            return owner;
        }
        ROOT
    }

    /// Whether the folder at `path` is judged by owner and mode.
    #[cfg_attr(not(test), allow(clippy::unused_self))]
    fn judges(&self, path: &Path) -> bool {
        #[cfg(test)]
        if let Some(from) = &self.seams.judged_from {
            return path.starts_with(from);
        }
        let _ = path;
        true
    }

    /// The mount table `gvfsd-admin` shares with this process, or `None`
    /// when it cannot be read or is not the host's.
    #[cfg_attr(not(test), allow(clippy::unused_self))]
    fn mounts(&self) -> Option<Vec<Mount>> {
        #[cfg(test)]
        if let Some(mounts) = &self.seams.mounts {
            return Some(mounts.clone());
        }
        if Sandbox::detect().is_flatpak() {
            return None;
        }
        mounts::read_mounts().ok()
    }

    /// Where `/etc/fstab`, libmount's `utab`, `/sys` and `/dev` are.
    #[cfg_attr(not(test), allow(clippy::unused_self))]
    fn system(&self) -> PathBuf {
        #[cfg(test)]
        if let Some(system) = &self.seams.system {
            return system.clone();
        }
        PathBuf::from("/")
    }
}

/// The checks of one destination.
struct Checks<'a> {
    rules: &'a Rules,
    mounts: Vec<Mount>,
    users: UserMounts,
}

impl<'a> Checks<'a> {
    /// The checks under `rules`, refusing `shown` when the mount table
    /// cannot be believed.
    fn new(rules: &'a Rules, shown: &Path) -> Result<Self, TransferError> {
        let mounts = rules.mounts().ok_or_else(|| others_can_change(shown))?;
        Ok(Self {
            rules,
            mounts,
            users: UserMounts::read(&rules.system()),
        })
    }

    /// Checks the destination `path` and returns it with every link on
    /// its path resolved.
    fn check(&self, path: &Path) -> Result<PathBuf, TransferError> {
        let resolved = self.resolve(path)?;
        self.refuse_users_mounts_inside(&resolved, path)?;
        Ok(resolved)
    }

    /// Resolves `path` from `/` as the kernel will, judging each folder it
    /// passes and following only links in folders that passed.
    fn resolve(&self, path: &Path) -> Result<PathBuf, TransferError> {
        let root = PathBuf::from("/");
        self.require_trusted(&root, path)?;
        let mut reached = PathBuf::from("/");
        let mut pending: VecDeque<OsString> = names(path).into();
        let mut links = 0;
        while let Some(name) = pending.pop_front() {
            if name == ".." {
                reached.pop();
                continue;
            }
            if name.is_empty() || name == "." {
                continue;
            }
            let next = reached.join(&name);
            let metadata = fs::symlink_metadata(&next).map_err(|_| others_can_change(path))?;
            let kind = metadata.file_type();
            if kind.is_dir() {
                self.require_trusted(&next, path)?;
                reached = next;
            } else if kind.is_symlink() {
                // The folder holding the link passed, so only root can
                // replace it: where it leads is resolved and judged in turn.
                links += 1;
                let target = fs::read_link(&next).ok().filter(|_| links <= MAX_LINKS);
                let Some(target) = target else {
                    return Err(others_can_change(path));
                };
                if target.is_absolute() {
                    reached = PathBuf::from("/");
                }
                for name in names(&target).into_iter().rev() {
                    pending.push_front(name);
                }
            } else {
                return Err(not_a_folder(path));
            }
        }
        Ok(reached)
    }

    /// Refuses the folder at `path`, part of the destination `shown`,
    /// unless only root can change it and it is on a filesystem whose
    /// owners and modes can be believed, on a drive no ordinary user could
    /// have mounted there or could mount above it.
    fn require_trusted(&self, path: &Path, shown: &Path) -> Result<(), TransferError> {
        if self.rules.judges(path) {
            let metadata = fs::symlink_metadata(path).map_err(|_| others_can_change(shown))?;
            let closed = metadata.uid() == self.rules.owner() && metadata.mode() & WRITABLE_BY_OTHERS == 0;
            if !metadata.is_dir() || !closed {
                return Err(others_can_change(shown));
            }
        }
        let mount = match mount_of(path, &self.mounts) {
            Some(mount) if is_trusted_filesystem(mount) => mount,
            _ => return Err(others_can_change(shown)),
        };
        if self.users.could_be_users(mount) || self.users.point_over(path).is_some() {
            return Err(users_can_mount(shown));
        }
        Ok(())
    }

    /// Refuses the destination `path`, shown as `shown`, when a drive whose
    /// owners cannot be believed is mounted inside it, or a user may mount
    /// one there.
    fn refuse_users_mounts_inside(&self, path: &Path, shown: &Path) -> Result<(), TransferError> {
        let untrusted = self.mounts.iter().any(|mount| {
            mount.point.starts_with(path)
                && mount.point != path
                && (!is_trusted_filesystem(mount) || self.users.could_be_users(mount))
        });
        if untrusted || self.users.point_inside(path).is_some() {
            return Err(users_can_mount(shown));
        }
        Ok(())
    }
}

/// The names that lead from `/` to `path`, `..` included.
fn names(path: &Path) -> Vec<OsString> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_owned()),
            Component::ParentDir => Some("..".into()),
            Component::RootDir | Component::CurDir | Component::Prefix(_) => None,
        })
        .collect()
}

/// The mount that holds `path`: the one with the deepest mount point above
/// it, and of several on one mount point the last, which is on top.
fn mount_of<'m>(path: &Path, mounts: &'m [Mount]) -> Option<&'m Mount> {
    mounts
        .iter()
        .filter(|mount| path.starts_with(&mount.point))
        .max_by_key(|mount| mount.point.components().count())
}

/// Whether owners and modes on `mount` are written only by the kernel.
fn is_trusted_filesystem(mount: &Mount) -> bool {
    let known = TRUSTED_FILESYSTEMS.contains(&mount.filesystem.as_str());
    // ZFS reports its ACL type: NFSv4 entries do not show in the mode.
    let options = || mount.options.split(',');
    let acls_in_mode =
        mount.filesystem != "zfs" || options().any(|option| option == "posixacl" || option == "noacl");
    let users_own = USER_MOUNT_FOLDERS
        .iter()
        .any(|folder| mount.point.starts_with(folder));
    let image = mount.source.starts_with(b"/dev/loop");
    known && acls_in_mode && !users_own && !image
}

/// The refusal for a destination `path` that others can change, or that
/// is on a drive whose owners cannot be believed.
fn others_can_change(path: &Path) -> TransferError {
    TransferError::failed(format_message(
        "Other users can change {path}, or it is on a drive whose permissions can't be checked, \
         so items there can't be safely changed as administrator.",
        &[("path", &path.display().to_string())],
    ))
}

/// The refusal for a destination `path` on a drive an ordinary user could
/// have mounted, or where one could mount a drive.
fn users_can_mount(path: &Path) -> TransferError {
    TransferError::failed(format_message(
        "{path} is on a removable drive or in a place where other users can mount drives, \
         so items there can't be safely changed as administrator.",
        &[("path", &path.display().to_string())],
    ))
}

/// The refusal for a destination `path` that is not a folder.
fn not_a_folder(path: &Path) -> TransferError {
    TransferError::failed(format_message(
        "{path} is not a folder, so items can't be copied there as administrator.",
        &[("path", &path.display().to_string())],
    ))
}

#[cfg(test)]
mod tests;
