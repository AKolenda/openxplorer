// SPDX-License-Identifier: AGPL-3.0-only
//! Permanent deletion of folders under `admin://` ("Open as
//! administrator").
//!
//! **Threat model.** `gvfsd-admin` does every operation as root, and `GVfs`
//! has no handle that pins a folder, so a folder's tree can only be deleted
//! by path (`remote_delete`): every step is resolved again from `/`. Another
//! local account that can rename, create or remove any name that walk
//! resolves can swap a folder for a symbolic link between two steps, and
//! root then deletes wherever the link leads. The walk would also go into
//! a drive, share or bind mount inside the folder.
//!
//! **What makes the walk safe.** If every folder the walk resolves is owned
//! by root and neither group- nor other-writable, on a filesystem that
//! reports owners and modes truthfully, then only root can rename, create
//! or remove a name the walk uses, and the walk cannot be redirected. So a
//! folder is deleted only when that holds, and is otherwise refused before
//! anything is deleted. It is refused when:
//!
//! - any folder on its path from `/`, the folder itself or any folder
//!   inside it is not owned by root or has the group-write or the
//!   other-write bit. The sticky bit does not redeem such a folder: in a
//!   sticky folder anybody can still create the name the walk looks up
//!   next. A POSIX ACL that lets anybody else write raises the group-write
//!   bit, because with an ACL those bits show the ACL mask, which limits
//!   every named user and group entry. Owner and mode are read without
//!   following links.
//! - a symbolic link on its path is in a folder that fails that test. A
//!   link in a folder that passes cannot be replaced by anybody but root,
//!   so it is followed (Fedora Atomic's `/home` is a link to `var/home`),
//!   and the folders of its target are judged the same way, up to
//!   [`MAX_LINKS`] links.
//! - it is a mount point, or the mount table shows anything mounted inside
//!   it.
//! - any folder on its path, or the folder, is on a filesystem whose owners
//!   and modes an ordinary user could have written: anything but the local
//!   kernel filesystems of [`TRUSTED_FILESYSTEMS`] (FUSE and network
//!   filesystems are not among them), ZFS with `NFSv4` ACLs (their entries
//!   do not show in the mode), and anything mounted under `/media` or
//!   `/run/media` or from a loop device, which is where desktop users mount
//!   their own drives and disk images, whose owners say whatever their
//!   maker wrote. Inside a Flatpak, whose mount table is not the host's,
//!   every folder is refused.
//! - any folder on its path, or the folder, is on a drive an ordinary user
//!   could have mounted wherever it is mounted: one `/etc/fstab` lets users
//!   mount, one libmount records as mounted by a user, or a removable,
//!   hot-pluggable or image device ([`user_mounts`]). It is refused too
//!   when a user may mount a drive on its path or inside it.
//!
//! While it deletes, the walk asks about each folder again before entering
//! it, and stops at one that fails the owner and mode test or is on
//! another device than the folder, which catches a drive mounted after the
//! check. Like `rm --one-file-system`, that also stops at a Btrfs
//! subvolume inside the folder.
//!
//! Single files and links keep the plain path deletion: one removal of one
//! name has nothing below it to redirect. SMB shares and phones are out of
//! scope: their servers resolve the paths, not a root process here. The
//! write guard (XFER-020) asks about every item before it is removed, the
//! nesting limit applies and the deletion can be cancelled, as in
//! `remote_delete`.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use gio::prelude::*;

use super::local_delete::{contained_mount_refusal, mount_inside_refusal, mount_point_refusal};
use super::query::{enumerate, node_kind};
use super::remote_delete::{self, FolderCheck};
use crate::integration::Sandbox;
use crate::sizes::mounts::{self, Mount};
use crate::transfer::{nesting_error, Cancellation, NodeKind, TransferError, WriteGuard, MAX_DEPTH};

mod user_mounts;

use user_mounts::UserMounts;

/// The most symbolic links followed on the way to a folder, as the kernel
/// allows (`MAXSYMLINKS`); a loop of links is refused instead of followed.
const MAX_LINKS: usize = 40;

/// The only account trusted to own the folders the walk resolves.
const ROOT: u32 = 0;

/// The group-write and other-write bits.
const WRITABLE_BY_OTHERS: u32 = 0o022;

/// What the checks read about each item, without following links.
const CHECKED_ATTRIBUTES: &str =
    "standard::type,standard::name,standard::symlink-target,unix::uid,unix::mode,unix::device";

/// The filesystems whose owners and modes only the kernel writes: local
/// filesystems on a device the administrator mounted, memory filesystems
/// and overlays (Fedora Atomic's root is one), which only root mounts where
/// root can see them. A user's own FUSE or network mount, or a filesystem
/// without Unix owners (FAT, exFAT, NTFS), is not among them.
const TRUSTED_FILESYSTEMS: &[&str] = &[
    "ext2", "ext3", "ext4", "xfs", "btrfs", "f2fs", "bcachefs", "jfs", "zfs", "tmpfs", "ramfs", "overlay",
];

/// Where desktop users mount their own drives and disk images (`udisks2`).
const USER_MOUNT_FOLDERS: &[&str] = &["/media", "/run/media"];

/// Permanently deletes `file`, under `admin://`, and for a folder
/// everything inside it, after the checks in the module documentation.
/// `kind` is what the caller found `file` to be, without following links;
/// `None` asks.
///
/// # Errors
///
/// A refusal of those checks before anything is deleted, and every error
/// of [`remote_delete::delete_tree`].
pub(super) fn delete_tree(
    file: &gio::File,
    kind: Option<NodeKind>,
    cancel: &Cancellation,
    guard: Option<&WriteGuard>,
) -> Result<(), TransferError> {
    delete_tree_with(file, kind, cancel, guard, &Rules::default())
}

/// [`delete_tree`] under `rules`.
fn delete_tree_with(
    file: &gio::File,
    kind: Option<NodeKind>,
    cancel: &Cancellation,
    guard: Option<&WriteGuard>,
    rules: &Rules,
) -> Result<(), TransferError> {
    cancel.check()?;
    let checks = Checks { rules, cancel };
    let kind = match kind {
        Some(kind) => kind,
        None => node_kind(checks.query(file)?.file_type()),
    };
    if kind != NodeKind::Directory {
        return remote_delete::delete_tree(file, Some(kind), cancel, guard);
    }
    let device = match checks.check_item(file)? {
        Checked::Folder { device } => device,
        Checked::NotAFolder(kind) => return remote_delete::delete_tree(file, Some(kind), cancel, guard),
    };
    let check = |folder: &gio::File, info: &gio::FileInfo, depth: usize| {
        let path = path_of(folder);
        checks.require_closed(&path, info)?;
        if checks.device(folder, info) == Some(device) {
            Ok(())
        } else if depth == 0 {
            Err(mount_point_refusal(&path))
        } else {
            Err(contained_mount_refusal(&path))
        }
    };
    let folders = FolderCheck {
        attributes: CHECKED_ATTRIBUTES,
        check: &check,
    };
    remote_delete::delete_checked_tree(file, cancel, guard, &folders)
}

/// What [`Checks::check_item`] found.
enum Checked {
    /// A folder that may be deleted by path, on the device `device`.
    Folder { device: u32 },
    /// Something else, which is deleted as an item.
    NotAFolder(NodeKind),
}

/// What the checks trust. Only tests can change it.
#[derive(Default)]
struct Rules {
    #[cfg(test)]
    seams: Seams,
}

/// The stand-ins tests use for what an ordinary user cannot set up: a
/// root-owned folder, a mount table, a drive mounted part way through.
#[cfg(test)]
#[derive(Default)]
struct Seams {
    /// The owner trusted instead of root.
    owner: Option<u32>,
    /// The mount table read instead of this process's.
    mounts: Option<Vec<Mount>>,
    /// Folders above this one (the test's own folder) are not judged: they
    /// are the machine's, such as a world-writable `/tmp`.
    judged_from: Option<PathBuf>,
    /// The device reported for a folder instead of its own.
    device: Option<Box<DeviceSeen>>,
    /// The folder read in place of `/` for `/etc/fstab`, libmount's
    /// `utab`, `/sys` and `/dev`.
    system: Option<PathBuf>,
}

/// What a test reports as the device of the folder at a path, given the
/// device GIO reported.
#[cfg(test)]
type DeviceSeen = dyn Fn(&Path, u32) -> u32;

impl Rules {
    /// The account trusted to own every folder the walk resolves.
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

    /// The mount table the walk shares with `gvfsd-admin`, or `None` when
    /// it cannot be read or is not the host's.
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

    /// The device of the folder at `path`, which GIO reported as `device`.
    #[cfg_attr(not(test), allow(clippy::unused_self))]
    fn device(&self, path: &Path, device: u32) -> u32 {
        #[cfg(test)]
        if let Some(seen) = &self.seams.device {
            return seen(path, device);
        }
        let _ = path;
        device
    }
}

/// The checks of one deletion.
struct Checks<'a> {
    rules: &'a Rules,
    cancel: &'a Cancellation,
}

impl Checks<'_> {
    /// Checks the item `file` and, for a folder, everything the walk will
    /// resolve, before anything is deleted.
    fn check_item(&self, file: &gio::File) -> Result<Checked, TransferError> {
        let shown = path_of(file);
        let (top, mut names) = names_of(file);
        let Some(name) = names.pop() else {
            return Err(TransferError::failed(crate::i18n::gettext(
                "Filesystem roots cannot be changed as items.",
            )));
        };
        let mounts = self.rules.mounts().ok_or_else(|| others_can_change(&shown))?;
        let users = UserMounts::read(&self.rules.system());
        let resolver = Resolver {
            checks: self,
            mounts: &mounts,
            users: &users,
        };
        let (folder, folder_path) = resolver.resolve(&top, names)?;
        let item = folder.child(&name);
        let item_path = folder_path.join(&name);
        let info = self.query(&item)?;
        let kind = node_kind(info.file_type());
        if kind != NodeKind::Directory {
            return Ok(Checked::NotAFolder(kind));
        }
        resolver.require_trusted(&item_path, &shown, &info)?;
        refuse_mounts(&item_path, &shown, &mounts)?;
        if let Some(point) = users.point_inside(&item_path) {
            let inside = point.strip_prefix(&item_path).unwrap_or(point);
            return Err(users_can_mount(&shown.join(inside)));
        }
        let device = self
            .device(&item, &info)
            .ok_or_else(|| others_can_change(&shown))?;
        self.scan(file, &shown, device, 0)?;
        Ok(Checked::Folder { device })
    }

    /// Checks every folder inside `folder`, at nesting `depth`, which is
    /// shown as `path`: owned by root, closed to others, on `device`.
    fn scan(&self, folder: &gio::File, path: &Path, device: u32, depth: usize) -> Result<(), TransferError> {
        for (child, info) in enumerate(folder, CHECKED_ATTRIBUTES, Some(self.cancel))? {
            if depth + 1 > MAX_DEPTH {
                return Err(nesting_error());
            }
            // Only folders are entered; the deletion walk asks about an
            // item whose kind the listing left out before it enters it.
            if info.file_type() != gio::FileType::Directory {
                continue;
            }
            let child_path = path.join(info.name());
            self.require_closed(&child_path, &info)?;
            if self.device(&child, &info) != Some(device) {
                return Err(mount_inside_refusal(&child_path));
            }
            self.scan(&child, &child_path, device, depth + 1)?;
        }
        Ok(())
    }

    /// Refuses the folder at `path`, described by `info`, unless only the
    /// trusted account can change it.
    fn require_closed(&self, path: &Path, info: &gio::FileInfo) -> Result<(), TransferError> {
        if !self.rules.judges(path) {
            return Ok(());
        }
        let owner = info
            .has_attribute("unix::uid")
            .then(|| info.attribute_uint32("unix::uid"));
        let mode = info
            .has_attribute("unix::mode")
            .then(|| info.attribute_uint32("unix::mode"));
        match (owner, mode) {
            (Some(owner), Some(mode)) if owner == self.rules.owner() && mode & WRITABLE_BY_OTHERS == 0 => {
                Ok(())
            }
            _ => Err(others_can_change(path)),
        }
    }

    /// The device of `file`, which `info` describes, when GIO reports one.
    fn device(&self, file: &gio::File, info: &gio::FileInfo) -> Option<u32> {
        info.has_attribute("unix::device").then(|| {
            self.rules
                .device(&path_of(file), info.attribute_uint32("unix::device"))
        })
    }

    /// What `file` is, never what a link to it points at.
    fn query(&self, file: &gio::File) -> Result<gio::FileInfo, TransferError> {
        self.cancel.check()?;
        let info = file.query_info(
            CHECKED_ATTRIBUTES,
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            Some(self.cancel.cancellable()),
        )?;
        Ok(info)
    }
}

/// Resolves the folders on an item's path from `/` the way the kernel will,
/// judging each folder it passes and following only links in folders that
/// passed.
struct Resolver<'a> {
    checks: &'a Checks<'a>,
    mounts: &'a [Mount],
    users: &'a UserMounts,
}

impl Resolver<'_> {
    /// The folder that `names`, from `top` (`/`), lead to, and its path
    /// with every link resolved.
    fn resolve(&self, top: &gio::File, names: Vec<OsString>) -> Result<(gio::File, PathBuf), TransferError> {
        let root = PathBuf::from("/");
        self.require_trusted(&root, &root, &self.checks.query(top)?)?;
        // The folders reached so far, below `/`, each with its name.
        let mut reached: Vec<(OsString, gio::File)> = Vec::new();
        let mut pending: VecDeque<OsString> = names.into();
        let mut links = 0;
        while let Some(name) = pending.pop_front() {
            if name == ".." {
                reached.pop();
                continue;
            }
            if name.is_empty() || name == "." {
                continue;
            }
            let parent = reached.last().map_or(top, |(_, folder)| folder);
            let file = parent.child(&name);
            let path = path_through(&reached).join(&name);
            let info = self.checks.query(&file)?;
            match info.file_type() {
                gio::FileType::Directory => {
                    self.require_trusted(&path, &path, &info)?;
                    reached.push((name, file));
                }
                // The folder holding the link passed, so only root can
                // replace it: where it leads is resolved and judged in turn.
                gio::FileType::SymbolicLink => {
                    links += 1;
                    let target = info.symlink_target().filter(|_| links <= MAX_LINKS);
                    let Some(target) = target else {
                        return Err(others_can_change(&path));
                    };
                    let mut names = Vec::new();
                    for component in target.components() {
                        match component {
                            Component::RootDir => reached.clear(),
                            Component::Normal(name) => names.push(name.to_owned()),
                            Component::ParentDir => names.push("..".into()),
                            Component::CurDir | Component::Prefix(_) => {}
                        }
                    }
                    for name in names.into_iter().rev() {
                        pending.push_front(name);
                    }
                }
                _ => return Err(others_can_change(&path)),
            }
        }
        let folder = reached.last().map_or(top, |(_, folder)| folder).clone();
        Ok((folder, path_through(&reached)))
    }

    /// Refuses the folder at `path`, shown as `shown` and described by
    /// `info`, unless only root can change it and it is on a filesystem
    /// whose owners and modes can be believed, on a drive no ordinary user
    /// could have mounted there or could mount above it.
    fn require_trusted(&self, path: &Path, shown: &Path, info: &gio::FileInfo) -> Result<(), TransferError> {
        self.checks
            .require_closed(path, info)
            .map_err(|_| others_can_change(shown))?;
        let mount = match mount_of(path, self.mounts) {
            Some(mount) if is_trusted_filesystem(mount) => mount,
            _ => return Err(others_can_change(shown)),
        };
        if self.users.could_be_users(mount) || self.users.point_over(path).is_some() {
            return Err(users_can_mount(shown));
        }
        Ok(())
    }
}

/// Refuses the folder at `path`, shown as `shown`, when the mount table
/// shows it is a mount point or that something is mounted inside it. The
/// table's paths are compared with `path`, whose links are resolved.
fn refuse_mounts(path: &Path, shown: &Path, mounts: &[Mount]) -> Result<(), TransferError> {
    let inside = mounts
        .iter()
        .filter_map(|mount| mount.point.strip_prefix(path).ok())
        .min();
    match inside {
        Some(inside) if inside.as_os_str().is_empty() => Err(mount_point_refusal(shown)),
        Some(inside) => Err(mount_inside_refusal(&shown.join(inside))),
        None => Ok(()),
    }
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

/// The refusal to delete a folder because others can change `path`, or it
/// is on a drive whose owners cannot be believed.
fn others_can_change(path: &Path) -> TransferError {
    TransferError::failed(crate::i18n::format_message(
        "Other users can change {path}, or it is on a drive whose permissions can't be checked, \
         so this folder can't be safely deleted as administrator.",
        &[("path", &path.display().to_string())],
    ))
}

/// The refusal to delete a folder because `path` is on a drive an
/// ordinary user could have mounted, or is where one could mount a drive.
fn users_can_mount(path: &Path) -> TransferError {
    TransferError::failed(crate::i18n::format_message(
        "{path} is on a removable drive or in a place where other users can mount drives, \
         so this folder can't be safely deleted as administrator.",
        &[("path", &path.display().to_string())],
    ))
}

/// The topmost folder above `file` (`/`) and the names leading from it to
/// `file`.
fn names_of(file: &gio::File) -> (gio::File, Vec<OsString>) {
    let mut names = Vec::new();
    let mut top = file.clone();
    while let Some(parent) = top.parent() {
        names.push(top.basename().map_or_else(OsString::new, PathBuf::into_os_string));
        top = parent;
    }
    names.reverse();
    (top, names)
}

/// The absolute path of `file` in its location, as the user sees it.
fn path_of(file: &gio::File) -> PathBuf {
    let (_, names) = names_of(file);
    let mut path = PathBuf::from("/");
    path.extend(names);
    path
}

/// The path of the folders `reached`, from `/`.
fn path_through(reached: &[(OsString, gio::File)]) -> PathBuf {
    let mut path = PathBuf::from("/");
    path.extend(reached.iter().map(|(name, _)| name));
    path
}

#[cfg(test)]
mod tests;
