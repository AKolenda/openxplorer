// SPDX-License-Identifier: AGPL-3.0-only
//! Where a shortcut leads, as Windows resolves a shortcut before it opens
//! it.
//!
//! A shortcut here is a symbolic link ([`crate::entry::Entry::is_shortcut`]).
//! Opening one opens what it points to, as Windows Explorer opens the
//! target of a `.lnk`: a folder at its own address, so Up and the address
//! bar show where the folder really is, and a file under its own name, so
//! the application sees the file's type. A link to a link is followed to
//! the end, at most [`MAX_HOPS`] links, as the kernel follows them.
//!
//! When nothing is at the end, Windows asks "Problem with Shortcut": the
//! item "has been changed or moved", and offers to delete the shortcut.
//! When the target is on a drive or a share that is not connected, Windows
//! says instead that "the drive or network connection that the shortcut
//! refers to is unavailable", and offers nothing to delete, since the
//! target is fine once the drive is back. Here such a drive is a removable
//! drive or mounted share under `/media/<user>/<name>`,
//! `/run/media/<user>/<name>` or `/mnt/<name>`, or a share mounted by
//! `GVfs` under `<runtime>/gvfs/<share>`, whose folder is missing or holds
//! no mounted drive. A link that leads back to itself (a loop) never
//! reaches an item: it is broken, as a link to a deleted item is.
//!
//! Every look is a `lstat` or a `readlink` of a local path, run on a
//! worker thread by the caller, so a link onto a share that stopped
//! answering never blocks the window.

use std::path::{Component, Path, PathBuf};

/// How many links are followed before the chain counts as a loop, as
/// Linux's `MAXSYMLINKS`.
pub const MAX_HOPS: usize = 40;

/// Where a shortcut leads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutDestination {
    /// An item is there: the path at the end of the links, which is not a
    /// link itself.
    Found(PathBuf),
    /// Nothing is there, or the links loop: the path the last link names.
    Missing(PathBuf),
    /// The target is on a drive or share that is not connected: the path
    /// the last link names, and the drive's mount folder.
    Unavailable {
        /// The path the last link names.
        target: PathBuf,
        /// The folder the drive or share is mounted at when it is
        /// connected.
        mount: PathBuf,
    },
}

impl ShortcutDestination {
    /// The name Windows' "Problem with Shortcut" gives the item: the last
    /// part of the path the shortcut names.
    pub fn item_name(&self) -> String {
        let path = match self {
            Self::Found(path) | Self::Missing(path) | Self::Unavailable { target: path, .. } => path,
        };
        path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }
}

/// Follows the shortcut at `link` to where it leads. Blocking: call it on a
/// worker thread.
///
/// # Errors
///
/// `link` cannot be read as a link: it is gone, or it is not a link.
pub fn resolve_shortcut(link: &Path) -> std::io::Result<ShortcutDestination> {
    resolve_with(link, &|path| is_mount_point(path))
}

/// Where a shortcut that names `path` leads, such as a `.desktop` link's
/// `file:` address: [`ShortcutDestination::Found`] when something is
/// there, otherwise missing or unavailable as [`resolve_shortcut`] tells
/// them apart. Blocking: call it on a worker thread.
pub fn destination_of(path: &Path) -> ShortcutDestination {
    let path = lexically_normal(path);
    if std::fs::symlink_metadata(&path).is_ok() {
        return ShortcutDestination::Found(path);
    }
    missing(path, &|folder| is_mount_point(folder))
}

/// [`resolve_shortcut`], with `is_mounted` telling whether a folder has a
/// drive mounted on it.
fn resolve_with(link: &Path, is_mounted: &dyn Fn(&Path) -> bool) -> std::io::Result<ShortcutDestination> {
    let mut current = link.to_path_buf();
    let mut named = std::fs::read_link(&current)?;
    for _ in 0..MAX_HOPS {
        let next = lexically_normal(&resolve_against(&current, &named));
        match std::fs::symlink_metadata(&next) {
            Ok(metadata) if metadata.file_type().is_symlink() => match std::fs::read_link(&next) {
                Ok(target) => {
                    current = next;
                    named = target;
                }
                Err(_) => return Ok(missing(next, is_mounted)),
            },
            Ok(_) => return Ok(ShortcutDestination::Found(next)),
            Err(_) => return Ok(missing(next, is_mounted)),
        }
    }
    // The links loop, or are longer than the system follows.
    Ok(ShortcutDestination::Missing(lexically_normal(&resolve_against(
        &current, &named,
    ))))
}

/// `named`, as the link at `link` names it, resolved against the link's
/// folder when it is relative.
fn resolve_against(link: &Path, named: &Path) -> PathBuf {
    if named.is_absolute() {
        return named.to_path_buf();
    }
    link.parent().unwrap_or_else(|| Path::new("/")).join(named)
}

/// `path` without `.` parts, and with each `..` taking away the part before
/// it, as an address shows it. A link's own `..` is meant this way: the
/// kernel resolves it against the folder the link names, which is what
/// [`resolve_with`] has followed up to here.
fn lexically_normal(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normal.pop() {
                    normal.push("/");
                }
                if normal.as_os_str().is_empty() {
                    normal.push("/");
                }
            }
            other => normal.push(other.as_os_str()),
        }
    }
    normal
}

/// Nothing is at `target`: broken, or on a drive that is not connected.
fn missing(target: PathBuf, is_mounted: &dyn Fn(&Path) -> bool) -> ShortcutDestination {
    match drive_folder(&target) {
        Some(mount) if !is_mounted(&mount) => ShortcutDestination::Unavailable { target, mount },
        _ => ShortcutDestination::Missing(target),
    }
}

/// The folder a removable drive or share holding `path` is mounted at, if
/// `path` is on one: `/media/<user>/<name>`, `/run/media/<user>/<name>`,
/// `/mnt/<name>`, or `/run/user/<id>/gvfs/<share>`.
fn drive_folder(path: &Path) -> Option<PathBuf> {
    let parts: Vec<&std::ffi::OsStr> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part),
            _ => None,
        })
        .collect();
    let depth = match parts.as_slice() {
        [first, ..] if *first == "media" => 3,
        [first, second, ..] if *first == "run" && *second == "media" => 4,
        [first, second, _, fourth, ..] if *first == "run" && *second == "user" && *fourth == "gvfs" => 5,
        [first, ..] if *first == "mnt" => 2,
        _ => return None,
    };
    // The drive's own folder must be named, not just the folder of drives.
    if parts.len() < depth {
        return None;
    }
    let mut folder = PathBuf::from("/");
    folder.extend(&parts[..depth]);
    Some(folder)
}

/// Whether a drive is mounted on the folder `path`: it exists, and it is on
/// another device than its parent folder, or it is the same inode as its
/// parent (the root).
fn is_mount_point(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    let Ok(folder) = std::fs::symlink_metadata(path) else {
        return false;
    };
    let Some(parent) = path.parent() else {
        return true;
    };
    let Ok(parent) = std::fs::symlink_metadata(parent) else {
        return false;
    };
    folder.dev() != parent.dev() || folder.ino() == parent.ino()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;
    use crate::test_support::temporary_folder;

    fn never_mounted(_: &Path) -> bool {
        false
    }

    /// A shortcut to a file or a folder leads to it, through a chain of
    /// links and a relative link, at its own address.
    ///
    /// parity: OPEN-027
    #[test]
    fn a_shortcut_leads_to_its_item_through_links_and_relative_names() {
        let folder = temporary_folder();
        let projects = folder.path().join("Projects");
        std::fs::create_dir(&projects).expect("fixture folder");
        let song = folder.path().join("Morning Walk.mp3");
        std::fs::write(&song, b"ID3").expect("fixture file");
        let desktop = folder.path().join("Desktop");
        std::fs::create_dir(&desktop).expect("fixture folder");
        symlink(&song, desktop.join("Morning Walk.mp3 - Shortcut")).expect("link");
        symlink("../Projects", desktop.join("Projects - Shortcut")).expect("relative link");
        symlink(
            desktop.join("Projects - Shortcut"),
            desktop.join("Projects - Shortcut - Shortcut"),
        )
        .expect("link to a link");

        let resolve = |name: &str| resolve_with(&desktop.join(name), &never_mounted).expect("a link");

        assert_eq!(
            resolve("Morning Walk.mp3 - Shortcut"),
            ShortcutDestination::Found(song)
        );
        assert_eq!(
            resolve("Projects - Shortcut"),
            ShortcutDestination::Found(projects.clone())
        );
        assert_eq!(
            resolve("Projects - Shortcut - Shortcut"),
            ShortcutDestination::Found(projects)
        );
    }

    /// A shortcut whose item was deleted, or whose links loop, is broken,
    /// and names the item as Windows' "Problem with Shortcut" does.
    ///
    /// parity: OPEN-028
    #[test]
    fn a_deleted_item_or_a_loop_is_missing() {
        let folder = temporary_folder();
        let plan = folder.path().join("Old plan.txt");
        symlink(&plan, folder.path().join("Old plan.txt - Shortcut")).expect("dangling link");
        symlink("loop-b", folder.path().join("loop-a")).expect("loop");
        symlink("loop-a", folder.path().join("loop-b")).expect("loop");
        symlink("self loop", folder.path().join("self loop")).expect("loop");

        let deleted = resolve_with(&folder.path().join("Old plan.txt - Shortcut"), &never_mounted);
        let looped = resolve_with(&folder.path().join("loop-a"), &never_mounted).expect("a link");
        let itself = resolve_with(&folder.path().join("self loop"), &never_mounted).expect("a link");

        let deleted = deleted.expect("a link");
        assert_eq!(deleted, ShortcutDestination::Missing(plan));
        assert_eq!(deleted.item_name(), "Old plan.txt");
        assert!(matches!(looped, ShortcutDestination::Missing(_)), "{looped:?}");
        assert!(matches!(itself, ShortcutDestination::Missing(_)), "{itself:?}");
        assert_eq!(itself.item_name(), "self loop");
    }

    /// A shortcut onto a drive that is not connected is unavailable, not
    /// broken: the drive's folder is named, and the same item on a
    /// connected drive is simply missing.
    ///
    /// parity: OPEN-028
    #[test]
    fn a_target_on_a_drive_that_is_not_connected_is_unavailable() {
        let folder = temporary_folder();
        let link = folder.path().join("Photos on USB - Shortcut");
        symlink("/media/demo/BACKUP-USB/Photos/2026", &link).expect("link");

        let away = resolve_with(&link, &never_mounted).expect("a link");
        let present = resolve_with(&link, &|_| true).expect("a link");

        assert_eq!(
            away,
            ShortcutDestination::Unavailable {
                target: PathBuf::from("/media/demo/BACKUP-USB/Photos/2026"),
                mount: PathBuf::from("/media/demo/BACKUP-USB"),
            }
        );
        assert_eq!(
            present,
            ShortcutDestination::Missing(PathBuf::from("/media/demo/BACKUP-USB/Photos/2026"))
        );
    }

    /// `..` and `.` in a link's target are resolved as an address shows
    /// them.
    #[test]
    fn dot_parts_are_resolved() {
        assert_eq!(
            lexically_normal(Path::new("/a/b/../c/./d")),
            PathBuf::from("/a/c/d")
        );
        assert_eq!(lexically_normal(Path::new("/../a")), PathBuf::from("/a"));
    }
}
