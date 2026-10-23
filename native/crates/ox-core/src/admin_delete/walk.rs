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
/// kernel says; a test replaces the answer for one path, so that the rule
/// can be checked without a second filesystem or a mount namespace.
pub(crate) type MountProbe = Box<dyn Fn(&Path, Mount) -> Mount>;

/// Run just before each item is handled. Only a test installs one.
pub(crate) type ItemHook = Box<dyn Fn(&Path)>;

/// The mount probe of a real deletion: whatever the kernel reports.
pub(crate) fn kernel_mounts() -> MountProbe {
    Box::new(|_path, mount| mount)
}

/// How many symbolic links one resolution of the item's folder may
/// follow before it is a loop or a deliberate pile.
const MAX_LINK_HOPS: usize = 40;

/// Directory modes that let somebody other than the owner change what is
/// in a folder: group write and other write. The sticky bit does not
/// redeem them — in a world-writable sticky folder such as `/tmp`,
/// anybody can still create the name the walk is about to look up.
const WRITABLE_BY_OTHERS: u32 = 0o022;

/// Who may own a symbolic link, and the directories above it, for the
/// walk to follow that link.
///
/// Outside the tests this is root and nothing else: [`TrustedOwner::ROOT`]
/// is the only value that can be built, because the constructor that
/// takes a uid exists only under `cfg(test)`. A test needs it because an
/// unprivileged process cannot create a root-owned link, so it trusts
/// root and itself and builds the chain under its own home.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TrustedOwner {
    /// One further uid the tests trust. It is always `None` in a real
    /// build: the only constructor that sets it is `cfg(test)`.
    also: Option<u32>,
}

impl TrustedOwner {
    /// Root, and nobody else. This is what the helper always runs with.
    pub(crate) const ROOT: Self = Self { also: None };

    /// Root and `uid`, for a test that cannot create a root-owned link.
    #[cfg(test)]
    pub(crate) const fn root_and(uid: u32) -> Self {
        Self { also: Some(uid) }
    }

    /// Whether something owned by `uid` is trusted.
    fn allows(self, uid: u32) -> bool {
        uid == 0 || self.also == Some(uid)
    }
}

/// What the helper's walk may do.
///
/// This type never leaves the crate. Its two seams exist so that the rules
/// a privileged walk has to obey can be tested on an ordinary temporary
/// folder, as the ordinary test user: a race that needs the tree to change
/// at one exact moment, and a mount that would otherwise need a second
/// filesystem. A real deletion uses [`Options::default`], which has
/// neither.
pub(crate) struct Options {
    /// The deepest nesting walked, as everywhere else in the engine.
    pub(crate) max_depth: usize,
    /// How the walk learns which mount a folder is on.
    pub(crate) mounts: MountProbe,
    /// Run just before each item is handled, after the walk has decided
    /// what it is holding. A test changes the tree here.
    pub(crate) at_item: Option<ItemHook>,
    /// Who may own a symbolic link on the way to the item, and the
    /// folders above it. Always [`TrustedOwner::ROOT`] in a real build.
    pub(crate) trusted: TrustedOwner,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            max_depth: MAX_DEPTH,
            mounts: kernel_mounts(),
            at_item: None,
            trusted: TrustedOwner::ROOT,
        }
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
    let parent = ParentWalk::new(options.trusted).open(parent_path)?;
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
        if let Some(at_item) = &self.options.at_item {
            at_item(path);
        }
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
        Ok((self.options.mounts)(path, mount_of(dir, name, flags)?))
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
    // which quietly drops a `.` and would let `/etc/.` through as `/etc`.
    // Nothing a privileged helper is given is normalised: a path that is
    // not already the item's own path is refused.
    let raw = target.as_os_str().as_bytes();
    let trimmed = raw.strip_suffix(b"/").unwrap_or(raw);
    if trimmed.is_empty() {
        return Err(refusal(gettext("Filesystem roots cannot be deleted.")));
    }
    let plain = trimmed.starts_with(b"/")
        && trimmed
            .split(|byte| *byte == b'/')
            .skip(1)
            .all(|part| !part.is_empty() && part != b"." && part != b"..");
    if !plain {
        return Err(refusal(gettext(
            "An administrator deletion needs the item’s own absolute path.",
        )));
    }
    target
        .file_name()
        .ok_or_else(|| refusal(gettext("Filesystem roots cannot be deleted.")))
}

/// Flags for a descriptor that only anchors the `*at` calls below it:
/// the helper never reads or writes through one.
const ANCHOR: OFlags = OFlags::PATH.union(OFlags::DIRECTORY).union(OFlags::CLOEXEC);

/// One resolution of the folder that holds the item being deleted.
///
/// This is the step a privileged helper cannot take by path. Every
/// component is opened relative to a descriptor the walk already holds,
/// so nothing is resolved twice, and a symbolic link is followed only
/// under the rule in [`ParentWalk::follow`].
struct ParentWalk {
    /// Who may own a link, and the folders above it.
    trusted: TrustedOwner,
    /// Links followed so far, across the whole resolution.
    hops: usize,
}

/// A folder the walk holds open, and what it knows about the way there.
struct Reached {
    /// The descriptor, which is what every further step resolves against.
    folder: OwnedFd,
    /// The path walked so far. It is only ever shown to the user; the
    /// walk never resolves it again.
    path: PathBuf,
    /// Whether every folder opened on the way here, this one included,
    /// is owned by a trusted user and writable by nobody else.
    ///
    /// This is the invariant the whole rule rests on. Checking a link
    /// with `fstatat` and then reading it with `readlinkat` is two
    /// calls, so in general the name could be swapped in between — but
    /// only by somebody who can write to the folder that holds it. When
    /// every folder from `/` down is writable by the trusted user alone,
    /// nobody else can have created the link, and nobody else can
    /// replace it between the two calls either. The check and the read
    /// then describe the same link, and following it cannot hand the
    /// deletion to anyone.
    trusted: bool,
}

impl ParentWalk {
    /// A resolution that trusts links owned by `trusted`.
    fn new(trusted: TrustedOwner) -> Self {
        Self { trusted, hops: 0 }
    }

    /// Opens the absolute path `folder`, starting from `/`.
    fn open(&mut self, folder: &Path) -> Result<OwnedFd, TransferError> {
        let reached = self.root()?;
        Ok(self.descend(reached, folder)?.folder)
    }

    /// Opens `/`, where every resolution starts.
    fn root(&self) -> Result<Reached, TransferError> {
        let root = Path::new("/");
        let folder = fs::open(root, ANCHOR, Mode::empty()).map_err(|errno| opening_failed(root, errno))?;
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
                // A link's target may lead back up. `..` is resolved by
                // the kernel against the descriptor, never by path, and
                // it can only reach a folder this walk already opened
                // and already judged.
                Component::ParentDir => step_up(&reached)?,
                Component::RootDir => self.root()?,
                Component::CurDir | Component::Prefix(_) => reached,
            };
        }
        Ok(reached)
    }

    /// Enters `name` inside the folder `at` holds open.
    fn step(&mut self, at: Reached, name: &OsStr) -> Result<Reached, TransferError> {
        let path = at.path.join(name);
        let seen = fs::statat(&at.folder, name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|errno| opening_failed(&path, errno))?;
        if FileType::from_raw_mode(seen.st_mode) == FileType::Symlink {
            return self.follow(at, name, &path, seen.st_uid);
        }
        // `O_DIRECTORY` with `O_NOFOLLOW` also refuses a name that became
        // a link after the check above, so the check only decides whether
        // the link is followed, never whether one is followed by mistake.
        let folder = fs::openat(&at.folder, name, ANCHOR | OFlags::NOFOLLOW, Mode::empty())
            .map_err(|errno| opening_failed(&path, errno))?;
        let trusted = at.trusted && self.is_trusted(&folder, &path)?;
        Ok(Reached {
            folder,
            path,
            trusted,
        })
    }

    /// Follows the symbolic link `name`, which `owner` owns, if and only
    /// if nobody but a trusted user could have put it there.
    fn follow(
        &mut self,
        at: Reached,
        name: &OsStr,
        path: &Path,
        owner: u32,
    ) -> Result<Reached, TransferError> {
        if !self.trusted.allows(owner) {
            return Err(untrusted_link_refusal(path));
        }
        if !at.trusted {
            return Err(reachable_link_refusal(path, &at.path));
        }
        self.hops += 1;
        if self.hops > MAX_LINK_HOPS {
            return Err(too_many_links_refusal(path));
        }
        let target =
            fs::readlinkat(&at.folder, name, Vec::new()).map_err(|errno| opening_failed(path, errno))?;
        let target = PathBuf::from(OsString::from_vec(target.into_bytes()));
        // An absolute target starts again at `/`; a relative one carries
        // on from the folder that holds the link, with what the walk
        // already knows about the way there. Either way every component
        // of the target goes through the same rule, so a link inside it
        // is judged exactly as this one was.
        let from = if target.is_absolute() { self.root()? } else { at };
        self.descend(from, &target)
    }

    /// Whether `folder` is owned by a trusted user and writable by
    /// nobody else.
    fn is_trusted(&self, folder: &OwnedFd, path: &Path) -> Result<bool, TransferError> {
        let status = fs::fstat(folder).map_err(|errno| opening_failed(path, errno))?;
        Ok(self.trusted.allows(status.st_uid) && status.st_mode & WRITABLE_BY_OTHERS == 0)
    }
}

/// Goes up to the folder that holds the one `at` holds open. `..` is
/// resolved by the kernel against the descriptor, never by path, and it
/// can only reach a folder the walk already opened and already judged.
fn step_up(at: &Reached) -> Result<Reached, TransferError> {
    let path = at.path.parent().unwrap_or(Path::new("/")).to_path_buf();
    let folder =
        fs::openat(&at.folder, "..", ANCHOR, Mode::empty()).map_err(|errno| opening_failed(&path, errno))?;
    Ok(Reached {
        folder,
        path,
        trusted: at.trusted,
    })
}

/// The refusal for a link on the item's path that the administrator does
/// not own.
fn untrusted_link_refusal(path: &Path) -> TransferError {
    refusal(format_message(
        "{path} is a shortcut (a symbolic link) that the administrator does not own. An \
         administrator deletion follows one only when nobody else could have created it, \
         because a shortcut someone else controls could point anywhere. Open the folder it \
         leads to and delete the item there.",
        &[("path", &path.display().to_string())],
    ))
}

/// The refusal for a link that somebody other than the administrator
/// could replace, because a folder above it is writable by others.
fn reachable_link_refusal(path: &Path, folder: &Path) -> TransferError {
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
fn too_many_links_refusal(path: &Path) -> TransferError {
    refusal(format_message(
        "The path to {path} leads through more than {limit} shortcuts (symbolic links), so it \
         may be a loop. Nothing was deleted.",
        &[
            ("path", &path.display().to_string()),
            ("limit", &MAX_LINK_HOPS.to_string()),
        ],
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
