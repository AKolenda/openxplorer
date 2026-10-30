// SPDX-License-Identifier: AGPL-3.0-only
//! The walk the privileged helper runs: the descriptor-pinned deletion of
//! [`crate::gio_node`]'s local items, with the rules a program running as
//! root needs on top.
//!
//! The `file:` walk may resolve the symbolic links of the folder that holds
//! the selected item, because it runs as the user who can already reach
//! everything on that path. This walk runs as root, so it may not do so
//! blindly: a link anywhere in the path could have been planted by whoever
//! can write to one of its folders, and following it would hand them the
//! deletion. Every component from `/` down is therefore opened one at a
//! time, without following links, and a link found on the way is followed
//! only when nobody but root could have put it there
//! ([`TrustedOwner`]). That is what makes `/home` as a link to
//! `/var/home` work while a link under a world-writable folder does not.
//!
//! Inside the tree the rules are the ones `local_delete` already enforces:
//! names are resolved one at a time relative to a descriptor the walk holds
//! open, a folder is checked to still be the folder that was opened before
//! and after its items are removed, a symbolic link is removed as a link,
//! and nothing on another mount is entered or removed. Where the `file:`
//! walk refuses the whole deletion over a mount inside the tree, this one
//! leaves that subtree alone and reports it: a full `admin://` folder is
//! deleted as Windows Explorer deletes one, so the parts that were removed
//! stay removed and the user is told exactly what was not.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};

use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::fs::{self, AtFlags, Dir, FileType, Mode, OFlags, Stat};

use super::report::{Record, Totals};
use crate::i18n::{format_message, gettext};
use crate::safe_walk::{mount_of, require_same_item, require_still_named, Mount};
use crate::transfer::{nesting_error, Cancellation, TransferError, MAX_DEPTH};

/// Reads the mount an item is on. The real walk always reports what the
/// kernel says; a test replaces the answer for one path, so that the
/// rules about mounts can be checked without a second filesystem, a
/// removable drive or a mount namespace.
///
/// This seam exists only in a test build, so nothing a release build
/// runs can weaken a mount check.
#[cfg(test)]
pub(crate) type MountProbe = Box<dyn Fn(&Path, Mount) -> Mount>;

/// Run just before each item is handled. Only a test installs one, and
/// only a test build has anywhere to put it.
#[cfg(test)]
pub(crate) type ItemHook = Box<dyn Fn(&Path)>;

/// The mount probe of a real deletion: whatever the kernel reports.
#[cfg(test)]
pub(crate) fn kernel_mounts() -> MountProbe {
    Box::new(|_path, mount| mount)
}

/// How many symbolic links one resolution of the item's folder may
/// follow before it is a loop or a deliberate pile.
pub(super) const MAX_LINK_HOPS: usize = 40;

/// Directory modes that let somebody other than the owner change what is
/// in a folder: group write and other write. The sticky bit does not
/// redeem them — in a world-writable sticky folder such as `/tmp`,
/// anybody can still create the name the walk is about to look up.
const WRITABLE_BY_OTHERS: u32 = 0o022;

/// Who may own a symbolic link, and the directories above it, for the
/// walk to follow that link.
///
/// Outside the tests this is root and nothing else, and a release build
/// cannot widen it: the struct has no field at all unless `cfg(test)` is
/// set, so there is nothing to set. A test needs the extra uid because
/// an unprivileged process cannot create a root-owned link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TrustedOwner {
    /// One further uid the tests trust.
    #[cfg(test)]
    also: Option<u32>,
}

impl TrustedOwner {
    /// Root, and nobody else. This is what the helper always runs with.
    pub(crate) const ROOT: Self = Self {
        #[cfg(test)]
        also: None,
    };

    /// Root and `uid`, for a test that cannot create a root-owned link.
    #[cfg(test)]
    pub(crate) const fn root_and(uid: u32) -> Self {
        Self { also: Some(uid) }
    }

    /// Whether something owned by `uid` is trusted. A release build
    /// reads nothing from `self`, because there is nothing in it: root
    /// is the only answer.
    #[cfg_attr(not(test), expect(clippy::unused_self, reason = "no field outside the tests"))]
    fn allows(self, uid: u32) -> bool {
        #[cfg(test)]
        if self.also == Some(uid) {
            return true;
        }
        uid == 0
    }
}

/// What the helper's walk may do.
///
/// This type never leaves the crate. Its two seams exist so that the
/// rules a privileged walk has to obey can be tested on an ordinary
/// temporary folder, as the ordinary test user: a race that needs the
/// tree to change at one exact moment, and a mount that would otherwise
/// need a second filesystem or a removable drive. Both are behind
/// `cfg(test)`, so a release build has nowhere to put either and can
/// only run [`Options::default`]. `trusted` needs no such gate:
/// [`TrustedOwner::ROOT`] is the only value of that type a release
/// build can construct.
pub(crate) struct Options {
    /// The deepest nesting walked, as everywhere else in the engine.
    pub(crate) max_depth: usize,
    /// How the walk learns which mount a folder or link is on.
    #[cfg(test)]
    pub(crate) mounts: MountProbe,
    /// Run just before each item is handled, after the walk has decided
    /// what it is holding. A test changes the tree here.
    #[cfg(test)]
    pub(crate) at_item: Option<ItemHook>,
    /// Who may own a symbolic link on the way to the item, and the
    /// folders above it. Always [`TrustedOwner::ROOT`] in a real build.
    pub(crate) trusted: TrustedOwner,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            max_depth: MAX_DEPTH,
            #[cfg(test)]
            mounts: kernel_mounts(),
            #[cfg(test)]
            at_item: None,
            trusted: TrustedOwner::ROOT,
        }
    }
}

impl Options {
    /// The mount an item is on, as this walk sees it: what the kernel
    /// reports, or what a test put in its place. A release build has no
    /// seam to read, so it returns what it was given.
    #[cfg_attr(not(test), expect(clippy::unused_self, reason = "no seam outside the tests"))]
    fn observed_mount(&self, path: &Path, mount: Mount) -> Mount {
        #[cfg(test)]
        return (self.mounts)(path, mount);
        #[cfg(not(test))]
        {
            let _ = path;
            mount
        }
    }

    /// Runs the test hook for `path`, if a test installed one. A
    /// release build has none, so this does nothing.
    #[cfg_attr(not(test), expect(clippy::unused_self, reason = "no seam outside the tests"))]
    fn at_item(&self, path: &Path) {
        #[cfg(test)]
        if let Some(at_item) = &self.at_item {
            at_item(path);
        }
        #[cfg(not(test))]
        let _ = path;
    }
}

/// Permanently deletes `target` and everything inside it, reporting each
/// item to `report` as it goes.
///
/// # Errors
///
/// A refusal before anything is deleted (a relative path, a path through
/// `..`, a filesystem root, a mount point, or a symbolic link in the path
/// that holds the item that nobody can vouch for),
/// [`TransferError::Cancelled`], the nesting limit,
/// and the refusal that stops the walk when the tree is changed under it.
/// Items that could not be removed, and subtrees on another mount, are
/// reported rather than returned: the walk goes on and its totals say what
/// is left.
pub(crate) fn delete_tree(
    target: &Path,
    cancel: &Cancellation,
    options: &Options,
    report: &mut dyn FnMut(Record),
) -> Result<Totals, TransferError> {
    cancel.check()?;
    let name = check_target(target)?;
    // `check_target` refused a path without a parent, so the item has one.
    let parent_path = target.parent().unwrap_or(Path::new("/"));
    let parent = ParentWalk::new(options)?.open(parent_path)?;
    let mut walk = Walk {
        cancel,
        options,
        report,
        totals: Totals::default(),
        drive: Mount {
            id: None,
            device: (0, 0),
        },
    };
    walk.drive = walk.mount_of(parent.as_fd(), c"", AtFlags::EMPTY_PATH, parent_path)?;
    // A mount point is the drive itself, not an item on it: removing it
    // would empty whatever is mounted there. A bind mount of a single file
    // is a mount point too, so this is checked without following links and
    // before the item's kind is looked at.
    let target_mount = walk.mount_of(parent.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW, target)?;
    if !walk.drive.is_same_as(target_mount) {
        return Err(mount_point_refusal(target));
    }
    walk.delete_at(parent.as_fd(), name, target, 0)?;
    Ok(walk.totals)
}

/// One run of the helper's walk.
struct Walk<'a> {
    cancel: &'a Cancellation,
    options: &'a Options,
    report: &'a mut dyn FnMut(Record),
    totals: Totals,
    /// The mount the item is on; nothing on another mount is touched.
    drive: Mount,
}

impl Walk<'_> {
    /// Deletes `name` inside the pinned folder `parent`; `path` is its full
    /// path, which is only reported, never resolved again.
    fn delete_at(
        &mut self,
        parent: BorrowedFd<'_>,
        name: &OsStr,
        path: &Path,
        depth: usize,
    ) -> Result<(), TransferError> {
        self.cancel.check()?;
        if depth > self.options.max_depth {
            return Err(nesting_error());
        }
        self.options.at_item(path);
        let seen = match fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(seen) => seen,
            Err(errno) => {
                self.failed(path, errno);
                return Ok(());
            }
        };
        if depth > 0 {
            // A drive, share or bind mount inside the tree, including a
            // bind mount of one file. The item itself was checked before
            // the walk started.
            let mount = self.mount_of(parent, name, AtFlags::SYMLINK_NOFOLLOW, path)?;
            if !self.drive.is_same_as(mount) {
                self.skipped(path);
                return Ok(());
            }
        }
        if FileType::from_raw_mode(seen.st_mode) == FileType::Directory {
            return self.delete_folder(parent, name, path, &seen, depth);
        }
        // `unlinkat` removes the name, never what a link points at. If the
        // name raced into a folder it fails here rather than following it.
        match fs::unlinkat(parent, name, AtFlags::empty()) {
            Ok(()) => self.deleted(path),
            Err(errno) => self.failed(path, errno),
        }
        Ok(())
    }

    /// Empties and removes the folder `name`, which was `seen` as a folder
    /// without following links.
    fn delete_folder(
        &mut self,
        parent: BorrowedFd<'_>,
        name: &OsStr,
        path: &Path,
        seen: &Stat,
        depth: usize,
    ) -> Result<(), TransferError> {
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let folder = match fs::openat(parent, name, flags, Mode::empty()) {
            Ok(folder) => folder,
            Err(errno) => {
                self.failed(path, errno);
                return Ok(());
            }
        };
        let opened = fs::fstat(&folder)?;
        // The name was swapped for another folder between the `statat` and
        // the open. Everything below here would be someone else's tree, so
        // the walk stops instead of reporting one item and going on.
        require_same_item(seen, &opened)?;
        // A drive mounted on this folder, including one mounted while the
        // walk was running: the descriptor is already on that drive, so
        // nothing inside it is read.
        let mount = self.mount_of(folder.as_fd(), c"", AtFlags::EMPTY_PATH, path)?;
        if !self.drive.is_same_as(mount) {
            self.skipped(path);
            return Ok(());
        }
        let entries = match Dir::read_from(&folder) {
            Ok(entries) => entries,
            Err(errno) => {
                self.failed(path, errno);
                return Ok(());
            }
        };
        for entry in entries {
            self.cancel.check()?;
            let entry = match entry {
                Ok(entry) => entry,
                Err(errno) => {
                    self.failed(path, errno);
                    continue;
                }
            };
            let child_name = OsStr::from_bytes(entry.file_name().to_bytes());
            if child_name == "." || child_name == ".." {
                continue;
            }
            // Stop if the folder's name was swapped after it was opened.
            // All I/O still goes through its descriptor, so even a swap
            // after this check cannot redirect the next item's deletion.
            require_still_named(parent, name, &opened)?;
            self.delete_at(folder.as_fd(), child_name, &path.join(child_name), depth + 1)?;
        }
        self.cancel.check()?;
        require_still_named(parent, name, &opened)?;
        match fs::unlinkat(parent, name, AtFlags::REMOVEDIR) {
            Ok(()) => self.deleted(path),
            Err(errno) => self.failed(path, errno),
        }
        Ok(())
    }

    /// The mount of `name` in `dir`, as the walk's probe reports it.
    fn mount_of<P: rustix::path::Arg + Copy>(
        &self,
        dir: BorrowedFd<'_>,
        name: P,
        flags: AtFlags,
        path: &Path,
    ) -> Result<Mount, TransferError> {
        Ok(self.options.observed_mount(path, mount_of(dir, name, flags)?))
    }

    /// Records that `path` is gone.
    fn deleted(&mut self, path: &Path) {
        self.record(Record::Deleted(path.to_path_buf()));
    }

    /// Records that `path` was left in place because it is on another drive.
    fn skipped(&mut self, path: &Path) {
        self.record(Record::Skipped {
            path: path.to_path_buf(),
            reason: gettext("A drive, share or bind mount is here. Deleting never goes into another drive."),
        });
    }

    /// Records that `path` could not be removed, and goes on.
    fn failed(&mut self, path: &Path, errno: rustix::io::Errno) {
        self.record(Record::Failed {
            path: path.to_path_buf(),
            message: std::io::Error::from(errno).to_string(),
        });
    }

    /// Counts a record and hands it to the caller.
    fn record(&mut self, record: Record) {
        self.totals.count(&record);
        (self.report)(record);
    }
}

/// Checks the path the helper was asked to delete and returns its name.
///
/// The helper is started by `pkexec` and runs as root, so it trusts
/// nothing about its argument: a relative path would be resolved against
/// whatever working directory it inherited, and a `..` would leave the
/// folder the user is looking at. Both are refused rather than normalised,
/// because a normalised path is no longer the path the user confirmed.
fn check_target(target: &Path) -> Result<&OsStr, TransferError> {
    // Split on the separator itself rather than with `Path::components`,
    // which quietly drops a `.` and a trailing slash and would let
    // `/etc/.` through as `/etc`. Nothing a privileged helper is given
    // is normalised: a path that is not already the item's own path,
    // byte for byte, is refused rather than guessed at, and the refusal
    // names what is wrong with it.
    let raw = target.as_os_str().as_bytes();
    if raw == b"/" {
        return Err(filesystem_root_refusal());
    }
    if raw.ends_with(b"/") {
        return Err(trailing_slash_refusal(target));
    }
    let plain = raw.starts_with(b"/")
        && raw
            .split(|byte| *byte == b'/')
            .skip(1)
            .all(|part| !part.is_empty() && part != b"." && part != b"..");
    if !plain {
        return Err(own_absolute_path_refusal());
    }
    target.file_name().ok_or_else(filesystem_root_refusal)
}

/// The refusal to delete a whole filesystem.
pub(super) fn filesystem_root_refusal() -> TransferError {
    refusal(gettext("Filesystem roots cannot be deleted."))
}

/// The refusal for a path that is not already the item's own.
pub(super) fn own_absolute_path_refusal() -> TransferError {
    refusal(gettext(
        "An administrator deletion needs the item’s own absolute path.",
    ))
}

/// The refusal for a path with a slash on the end.
///
/// A trailing slash is not refused for being dangerous but for being a
/// different thing: `/srv/data/` names what is inside the folder, while
/// the helper is given the folder itself. Accepting it would mean
/// trimming it, and the helper normalises nothing, so it says what is
/// wrong and lets the caller pass the path it meant.
pub(super) fn trailing_slash_refusal(path: &Path) -> TransferError {
    refusal(format_message(
        "{path} ends with a slash, so it names a folder’s contents rather than the item \
         itself. An administrator deletion is given the item’s own path, with nothing \
         added or taken away.",
        &[("path", &path.display().to_string())],
    ))
}

/// Flags for a descriptor that only anchors the `*at` calls below it,
/// and names the object itself rather than what a link points at: the
/// helper never reads or writes through one.
const ANCHOR: OFlags = OFlags::PATH.union(OFlags::NOFOLLOW).union(OFlags::CLOEXEC);

/// One resolution of the folder that holds the item being deleted.
///
/// This is the step a privileged helper cannot take by path. Each
/// component is looked up exactly once, with one `openat` relative to a
/// descriptor the walk already holds, and everything after that asks
/// that descriptor rather than the name again. A symbolic link is
/// followed only under the rule in [`ParentWalk::follow`].
struct ParentWalk<'a> {
    /// The rules this walk runs with, and the test seams.
    options: &'a Options,
    /// Which mount `/` is on, read once when the walk starts. A link is
    /// followed only on this mount.
    root_mount: Mount,
    /// Links followed so far, across the whole resolution.
    hops: usize,
}

/// A folder the walk holds open, and what it knows about the way there.
struct Reached {
    /// The descriptor, which is what every further step resolves
    /// against. The name it was opened under is never looked up again.
    folder: OwnedFd,
    /// The path walked so far. It is only ever shown to the user; the
    /// walk never resolves it again.
    path: PathBuf,
    /// Whether every folder opened on the way here, this one included,
    /// is one nobody but root could have changed: on the same mount as
    /// `/`, owned by a trusted user, and writable by nobody else. A
    /// folder on any other mount is never trusted, because what it says
    /// about its own contents is only as honest as whoever provided
    /// that filesystem.
    ///
    /// This is the invariant the whole rule rests on, and it is what
    /// makes following a link safe. The walk looks a component up once,
    /// with `openat(…, O_PATH | O_NOFOLLOW)`, and then asks that
    /// descriptor what it is, who owns it, which mount it is on and,
    /// for a link, where it points (`readlinkat(fd, "")`). There is no
    /// second lookup by name, so the object that was authorised and the
    /// object that is read are the same object, whatever happens to the
    /// name meanwhile. The flag answers the remaining question: a
    /// descriptor can only have been handed a link worth following if
    /// nobody untrusted could have created that link in the first
    /// place. On the root mount that is exactly what root-owned,
    /// root-only-writable folders mean, because an unprivileged user
    /// cannot mount anything onto it: every inode on it was written
    /// through the permissions above.
    ///
    /// This says nothing about where a link may *lead*. Once a link is
    /// followed, the walk moves down into whatever is there, crossing
    /// into other mounts freely — Fedora Atomic's `/home` is a link to
    /// `var/home`, and `/var` is usually a mount of its own.
    trusted: bool,
}

impl<'a> ParentWalk<'a> {
    /// A resolution with the rules in `options`.
    ///
    /// # Errors
    ///
    /// `/` cannot be opened or inspected.
    fn new(options: &'a Options) -> Result<Self, TransferError> {
        let root = Path::new("/");
        let folder = fs::open(root, ANCHOR | OFlags::DIRECTORY, Mode::empty())
            .map_err(|errno| opening_failed(root, errno))?;
        let root_mount = mount_of(folder.as_fd(), c"", AtFlags::EMPTY_PATH)?;
        Ok(Self {
            options,
            root_mount: options.observed_mount(root, root_mount),
            hops: 0,
        })
    }

    /// Opens the absolute path `folder`, starting from `/`.
    fn open(&mut self, folder: &Path) -> Result<OwnedFd, TransferError> {
        let reached = self.root()?;
        Ok(self.descend(reached, folder)?.folder)
    }

    /// Opens `/`, where every resolution starts.
    fn root(&mut self) -> Result<Reached, TransferError> {
        let root = Path::new("/");
        let folder = fs::open(root, ANCHOR | OFlags::DIRECTORY, Mode::empty())
            .map_err(|errno| opening_failed(root, errno))?;
        let trusted = self.is_trusted(&folder, root)?;
        Ok(Reached {
            folder,
            path: root.to_path_buf(),
            trusted,
        })
    }

    /// Walks every component of `path` below `from`.
    fn descend(&mut self, from: Reached, path: &Path) -> Result<Reached, TransferError> {
        let mut reached = from;
        for part in path.components() {
            reached = match part {
                Component::Normal(name) => self.step(reached, name)?,
                Component::ParentDir => self.step_up(&reached)?,
                Component::RootDir => self.root()?,
                Component::CurDir | Component::Prefix(_) => reached,
            };
        }
        Ok(reached)
    }

    /// Enters `name` inside the folder `at` holds open.
    fn step(&mut self, at: Reached, name: &OsStr) -> Result<Reached, TransferError> {
        let path = at.path.join(name);
        // The one lookup of this component. `O_PATH | O_NOFOLLOW` names
        // the entry itself, link or folder, and nothing is read or
        // written through it.
        let entry = fs::openat(&at.folder, name, ANCHOR, Mode::empty())
            .map_err(|errno| opening_failed(&path, errno))?;
        let seen = fs::fstat(&entry).map_err(|errno| opening_failed(&path, errno))?;
        match FileType::from_raw_mode(seen.st_mode) {
            FileType::Symlink => self.follow(at, &entry, &path, seen.st_uid),
            FileType::Directory => {
                let trusted = at.trusted && self.is_trusted(&entry, &path)?;
                Ok(Reached {
                    folder: entry,
                    path,
                    trusted,
                })
            }
            // Everything on the way to the item has to be a folder.
            _ => Err(not_a_folder_refusal(&path)),
        }
    }

    /// Goes up to the folder that holds the one `at` holds open. `..`
    /// is resolved by the kernel against the descriptor, never by path,
    /// and it is judged like any other folder the walk opens.
    fn step_up(&mut self, at: &Reached) -> Result<Reached, TransferError> {
        let path = at.path.parent().unwrap_or(Path::new("/")).to_path_buf();
        let folder = fs::openat(&at.folder, "..", ANCHOR | OFlags::DIRECTORY, Mode::empty())
            .map_err(|errno| opening_failed(&path, errno))?;
        let trusted = at.trusted && self.is_trusted(&folder, &path)?;
        Ok(Reached {
            folder,
            path,
            trusted,
        })
    }

    /// Follows the symbolic link the descriptor `link` names, which
    /// `owner` owns, if and only if nobody but root could have put it
    /// there: root owns it, it is on the same mount as `/`, and every
    /// folder above it is root-owned, writable by nobody else and on
    /// that same mount.
    fn follow(
        &mut self,
        at: Reached,
        link: &OwnedFd,
        path: &Path,
        owner: u32,
    ) -> Result<Reached, TransferError> {
        if !self.options.trusted.allows(owner) {
            return Err(untrusted_link_refusal(path));
        }
        if !self.is_on_the_root_mount(link, path)? {
            return Err(other_mount_refusal(path));
        }
        if !at.trusted {
            return Err(reachable_link_refusal(path, &at.path));
        }
        self.hops += 1;
        if self.hops > MAX_LINK_HOPS {
            return Err(too_many_links_refusal(path));
        }
        // Read from the descriptor, not from the name: this is the same
        // object the checks above were about.
        let target = fs::readlinkat(link, "", Vec::new()).map_err(|errno| opening_failed(path, errno))?;
        let target = PathBuf::from(OsString::from_vec(target.into_bytes()));
        // An absolute target starts again at `/`; a relative one carries
        // on from the folder that holds the link, with what the walk
        // already knows about the way there. Either way every component
        // of the target goes through the same rule, so a link inside it
        // is judged exactly as this one was — and the target may well
        // lead onto another mount, which is allowed.
        let from = if target.is_absolute() { self.root()? } else { at };
        self.descend(from, &target)
    }

    /// Whether the folder `entry` names is one nobody but a trusted
    /// user could have changed.
    fn is_trusted(&mut self, entry: &OwnedFd, path: &Path) -> Result<bool, TransferError> {
        let status = fs::fstat(entry).map_err(|errno| opening_failed(path, errno))?;
        if !self.options.trusted.allows(status.st_uid) || status.st_mode & WRITABLE_BY_OTHERS != 0 {
            return Ok(false);
        }
        self.is_on_the_root_mount(entry, path)
    }

    /// Whether `entry` is on the same mount as `/`.
    ///
    /// Owner and mode are worth reading only on a filesystem nobody but
    /// root could have provided, and on Linux the root mount is exactly
    /// that: an unprivileged user cannot mount anything onto it, while
    /// a USB stick, a loop image or a FUSE filesystem is always a mount
    /// of its own, whatever it claims about the owners and modes inside
    /// it.
    ///
    /// The question is asked of two pinned descriptors, never of a path
    /// and never of `/proc/self/mountinfo`, which is a table that could
    /// have changed since it was read. Where the kernel reports mount
    /// IDs (`statx` `STATX_MNT_ID`, Linux 5.8 and later) both the ID
    /// and the device number must match: the ID alone can be reused
    /// after a mount goes away, and the device alone cannot tell a bind
    /// mount of part of the root filesystem from the root filesystem.
    ///
    /// Where the kernel reports no mount ID, the walk falls back to the
    /// device number rather than refusing every link. The attack this
    /// rule is about needs a filesystem the attacker supplied, and any
    /// such filesystem has a device number of its own, so the fallback
    /// still stops it. What it gives up is telling a bind mount of part
    /// of the root filesystem apart from the root filesystem, and
    /// making one needs privileges an attacker here does not have. The
    /// alternative, refusing every link on a kernel that merely lacks a
    /// `statx` flag, would break `/home` as a link to `/var/home` on
    /// those systems for no gain.
    fn is_on_the_root_mount(&self, entry: &OwnedFd, path: &Path) -> Result<bool, TransferError> {
        let mount = mount_of(entry.as_fd(), c"", AtFlags::EMPTY_PATH)?;
        let mount = self.options.observed_mount(path, mount);
        Ok(match (self.root_mount.id, mount.id) {
            (Some(root), Some(here)) => root == here && self.root_mount.device == mount.device,
            _ => self.root_mount.device == mount.device,
        })
    }
}

/// Resolves the folder that holds an item, the way a deletion does,
/// for the tests of the rules about links on that path.
///
/// # Errors
///
/// Whatever the resolution refuses.
#[cfg(test)]
pub(super) fn resolve_parent(folder: &Path, options: &Options) -> Result<OwnedFd, TransferError> {
    ParentWalk::new(options)?.open(folder)
}

/// Whether `entry` is on the same mount as `/`, as the walk decides it.
/// Only the tests ask directly; the walk asks while it resolves.
#[cfg(test)]
pub(super) fn is_on_the_root_mount(entry: &OwnedFd, options: &Options) -> bool {
    ParentWalk::new(options)
        .expect("/ can be opened")
        .is_on_the_root_mount(entry, Path::new("/"))
        .expect("the mount can be read")
}

/// The refusal for a link on the item's path that the administrator does
/// not own.
pub(super) fn untrusted_link_refusal(path: &Path) -> TransferError {
    refusal(format_message(
        "{path} is a shortcut (a symbolic link) that the administrator does not own. An \
         administrator deletion follows one only when nobody else could have created it, \
         because a shortcut someone else controls could point anywhere. Open the folder it \
         leads to and delete the item there.",
        &[("path", &path.display().to_string())],
    ))
}

/// The refusal for a link that is not on the root filesystem. What such
/// a filesystem says about the owners and modes inside it is only as
/// honest as whoever provided it.
pub(super) fn other_mount_refusal(path: &Path) -> TransferError {
    refusal(format_message(
        "{path} is a shortcut (a symbolic link) on a drive, share or folder other than this \
         computer’s own system drive, so what it says about itself cannot be relied on. \
         An administrator deletion does not follow one. Open the folder it leads to and delete \
         the item there. Deleting items on that drive still works.",
        &[("path", &path.display().to_string())],
    ))
}

/// The refusal for a link that somebody other than the administrator
/// could replace, because a folder above it is writable by others.
pub(super) fn reachable_link_refusal(path: &Path, folder: &Path) -> TransferError {
    refusal(format_message(
        "{path} is a shortcut (a symbolic link) inside {folder}, which other users can \
         change. An administrator deletion does not follow a shortcut anyone could replace \
         while it is being read. Open the folder it leads to and delete the item there.",
        &[
            ("path", &path.display().to_string()),
            ("folder", &folder.display().to_string()),
        ],
    ))
}

/// The refusal for a path that leads through one shortcut after another.
pub(super) fn too_many_links_refusal(path: &Path) -> TransferError {
    refusal(format_message(
        "The path to {path} leads through more than {limit} shortcuts (symbolic links), so it \
         may be a loop. Nothing was deleted.",
        &[
            ("path", &path.display().to_string()),
            ("limit", &MAX_LINK_HOPS.to_string()),
        ],
    ))
}

/// The refusal for something on the way to the item that is not a folder.
pub(super) fn not_a_folder_refusal(path: &Path) -> TransferError {
    refusal(format_message(
        "{path} is not a folder, so the item said to be inside it cannot be deleted.",
        &[("path", &path.display().to_string())],
    ))
}

/// The refusal for a folder on the item's path that cannot be opened.
fn opening_failed(path: &Path, errno: rustix::io::Errno) -> TransferError {
    refusal(format_message(
        "The folder {path} could not be opened for deletion. {reason}",
        &[
            ("path", &path.display().to_string()),
            ("reason", &std::io::Error::from(errno).to_string()),
        ],
    ))
}

/// The refusal to delete a mount point.
pub(crate) fn mount_point_refusal(path: &Path) -> TransferError {
    refusal(format_message(
        "{path} is where a drive or share is mounted, so it was not deleted. Unmount it first.",
        &[("path", &path.display().to_string())],
    ))
}

/// A refusal made before anything was deleted.
fn refusal(message: String) -> TransferError {
    TransferError::failed(message)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::test_support::temporary_folder;

    /// Opens `path` the way the walk does, so its trust can be asked.
    fn anchor(path: &Path) -> OwnedFd {
        fs::open(path, ANCHOR | OFlags::DIRECTORY, Mode::empty()).expect("the folder is opened")
    }

    /// Both halves of the owner-and-mode test, each on its own: a folder
    /// is trusted only while a trusted user owns it and nobody else can
    /// change it. The mount half is held out of the way here; the tests
    /// in `super::tests::parent_path` cover it.
    ///
    /// parity: XFER-015
    #[test]
    fn a_folder_is_trusted_only_while_a_trusted_user_alone_can_change_it() {
        let ours = temporary_folder();
        let theirs = temporary_folder();
        for folder in [ours.path(), theirs.path()] {
            std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o755))
                .expect("the mode is set");
        }
        // Run as root every fixture would belong to root, and the owner
        // rule would never fire, so this one is given to `nobody`.
        if rustix::process::getuid().as_raw() == 0 {
            std::os::unix::fs::lchown(theirs.path(), Some(65534), None)
                .expect("root can give the fixture away");
        }
        let mine = rustix::process::getuid().as_raw();
        let ask = |trusted: TrustedOwner, folder: &Path| {
            let options = Options {
                trusted,
                mounts: Box::new(|_path, mount| mount),
                ..Options::default()
            };
            ParentWalk::new(&options)
                .expect("/ can be opened")
                .is_trusted(&anchor(folder), folder)
                .expect("the folder can be inspected")
        };

        let owned_by_a_trusted_user = ask(TrustedOwner::root_and(mine), ours.path());
        let owned_by_somebody_else = ask(TrustedOwner::ROOT, theirs.path());
        std::fs::set_permissions(ours.path(), std::fs::Permissions::from_mode(0o775))
            .expect("the mode is set");
        let group_writable = ask(TrustedOwner::root_and(mine), ours.path());
        // Writable by others but not by the group: neither bit is the
        // one that matters on its own.
        std::fs::set_permissions(ours.path(), std::fs::Permissions::from_mode(0o757))
            .expect("the mode is set");
        let other_writable = ask(TrustedOwner::root_and(mine), ours.path());

        assert!(owned_by_a_trusted_user);
        assert!(!owned_by_somebody_else, "the owner is not one the walk trusts");
        assert!(!group_writable, "somebody in the group can change it");
        assert!(!other_writable, "anybody can change it");
    }
}
