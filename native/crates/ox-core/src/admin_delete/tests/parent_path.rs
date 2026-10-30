// SPDX-License-Identifier: AGPL-3.0-only
//! Resolving the folder that holds the item, which is the step a
//! privileged helper cannot take by path.
//!
//! Two rules decide whether a symbolic link on that path is followed:
//!
//! 1. A link **on the same mount as `/`** is followed when root owns it
//!    and every folder above it is root-owned and writable by nobody
//!    else. This is what lets Fedora Atomic's `/home -> var/home` work.
//! 2. A link **on any other mount** is never followed, whatever it
//!    claims about its owner, because what a filesystem says about
//!    itself is only as honest as whoever provided it. Deleting items
//!    on such a mount is not affected.
//!
//! Crossing into another mount while walking *down* is allowed, and
//! Atomic needs it: `/home` is a link on the root mount whose target
//! lives under `/var`, which is usually a mount of its own.
//!
//! Each test asserts the exact refusal the walk builds, so a test
//! cannot pass because some *other* rule happened to fire. Where a real
//! system already has the shape a rule is about, the test uses it with
//! the rules a real deletion runs with (`TrustedOwner::ROOT`); where it
//! does not, a fixture under the test user's home stands in, with the
//! trusted owner widened to that user, who cannot create a root-owned
//! link.

use std::fs;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, Mode, OFlags};
use tempfile::TempDir;

use super::super::walk::{
    self, not_a_folder_refusal, other_mount_refusal, reachable_link_refusal, too_many_links_refusal,
    untrusted_link_refusal, Options, TrustedOwner,
};
use crate::safe_walk::Mount;
use crate::test_support::temporary_folder;
use crate::transfer::{Cancellation, TransferError};

/// Where a machine that cannot host a trusted link must fail instead of
/// saying so and moving on.
const REQUIRE_TRUSTED_HOME: &str = "OX_REQUIRE_TRUSTED_HOME";

/// The owner these tests trust when they need a link followed: root, as
/// in a real build, and the user running the test, which is the only
/// uid it can give a link.
fn trusted_here() -> TrustedOwner {
    TrustedOwner::root_and(rustix::process::getuid().as_raw())
}

/// Options that resolve the item's folder with `trusted`, and otherwise
/// delete exactly as a real run does.
fn trusting(trusted: TrustedOwner) -> Options {
    Options {
        trusted,
        ..Options::default()
    }
}

/// Deletes `target`, reporting nothing, and returns how it ended.
fn delete(target: &Path, options: &Options) -> Result<(), TransferError> {
    walk::delete_tree(target, &Cancellation::new(), options, &mut |_record| {}).map(|_totals| ())
}

/// Resolves the folder that holds an item, without deleting anything.
fn resolve(folder: &Path, options: &Options) -> Result<OwnedFd, TransferError> {
    walk::resolve_parent(folder, options)
}

/// The device and inode of `path`, following links, so that a resolved
/// descriptor can be compared with the folder it should have reached.
fn identity_of(path: &Path) -> (u64, u64) {
    let status = fs::metadata(path).expect("the folder exists");
    (status.dev(), status.ino())
}

/// The device and inode a descriptor names.
fn identity_of_descriptor(folder: &OwnedFd) -> (u64, u64) {
    let status = rustix::fs::fstat(folder).expect("the descriptor can be inspected");
    (status.st_dev, status.st_ino)
}

/// Gives `path` the mode `bits`.
fn set_mode(path: &Path, bits: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(bits)).expect("the mode is set");
}

/// Creates `path` as a folder nobody but its owner can change. A folder
/// made with `create_dir` follows the umask, which is `0o002` on several
/// distributions and would leave it group-writable; what these tests are
/// about is the rule, not the umask of the machine running them.
fn make_dir(path: &Path) -> PathBuf {
    fs::create_dir_all(path).expect("the folder is created");
    set_mode(path, 0o755);
    path.to_path_buf()
}

/// Says that a test did nothing, loudly, and fails instead when
/// `OX_REQUIRE_TRUSTED_HOME` is set, so a continuous-integration run
/// cannot go green while these tests quietly test nothing.
pub(super) fn skip(reason: &str) {
    assert!(
        std::env::var_os(REQUIRE_TRUSTED_HOME).is_none(),
        "{REQUIRE_TRUSTED_HOME} is set, so this is a failure: {reason}"
    );
    eprintln!("SKIPPED (nothing was tested): {reason}");
}

/// A folder for a fixture whose whole path from `/` is on the same
/// mount as `/` and writable by nobody but trusted users, so a link
/// created in it is one the walk may follow.
///
/// Where it goes matters, and `$HOME` is the wrong answer. The check
/// driver `native/tools/check.py` gives every test executable a
/// disposable `HOME` under `TMPDIR`, and `/tmp` is world-writable, so
/// a fixture there is refused by rule 1 whoever owns it, which is what
/// every one of these tests would then be measuring. The default is
/// therefore the crate's own folder, which `CARGO_MANIFEST_DIR` fixes
/// at compile time and which lives in the checkout, normally owned by
/// whoever runs the tests and writable by nobody else.
/// `OX_TEST_TRUSTED_BASE` names another folder for a machine where
/// that does not hold, and `$HOME` is tried after it.
///
/// Each fixture is a [`TempDir`] and removes itself, but a test run
/// that is killed outright leaves one behind, named
/// `openxplorer-parent-path-*`. In the checkout `.gitignore` covers
/// that, and `native/parity/test_check.py` skips the pattern because
/// the links inside one cannot be copied; run as root the leftover is
/// instead in whichever of the folders below answered, so it is worth
/// looking in `/root`, `/srv`, `/opt`, `/usr/local` and `/var/lib`
/// after a run that was interrupted.
///
/// Returns `None`, saying loudly why, where no candidate qualifies.
fn trusted_base() -> Option<TempDir> {
    let mut refused = Vec::new();
    for candidate in base_candidates() {
        let base = match tempfile::Builder::new()
            .prefix("openxplorer-parent-path-")
            .tempdir_in(&candidate)
        {
            Ok(base) => base,
            Err(error) => {
                refused.push(format!("{}: {error}", candidate.display()));
                continue;
            }
        };
        set_mode(base.path(), 0o755);
        match untrusted_folder_above(base.path()) {
            None => return Some(base),
            Some(reason) => refused.push(reason),
        }
    }
    skip(&format!(
        "no folder on this machine can host a link the walk would follow: {}. Set \
         OX_TEST_TRUSTED_BASE to a folder on the same mount as / whose whole path is \
         writable by nobody but its owner.",
        refused.join("; ")
    ));
    None
}

/// Folders that belong to root on any ordinary system, tried when the
/// tests run as root and nothing closer qualifies.
///
/// Run as root, the checkout is usually owned by somebody else, which
/// makes it a folder the walk does not trust, and a disposable `HOME`
/// under `TMPDIR` is refused for being under a world-writable folder.
/// These are the same folders `native/tools/check_admin_delete_mounts.py`
/// builds its workspace in, for the same reason.
const ROOT_OWNED_FOLDERS: [&str; 5] = ["/root", "/srv", "/opt", "/usr/local", "/var/lib"];

/// Where a fixture may live, best first.
fn base_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(named) = std::env::var_os("OX_TEST_TRUSTED_BASE") {
        candidates.push(PathBuf::from(named));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home));
    }
    if rustix::process::getuid().as_raw() == 0 {
        candidates.extend(ROOT_OWNED_FOLDERS.iter().map(PathBuf::from));
    }
    candidates
}

/// Why `base` cannot hold a followable link, or `None` when it can.
///
/// This asks the standard library, not the walk, so that removing the
/// rule under test makes these tests fail rather than quietly skip.
fn untrusted_folder_above(base: &Path) -> Option<String> {
    if !same_device_as_root(base) {
        return Some(format!("{} is not on the same mount as /", base.display()));
    }
    let mine = rustix::process::getuid().as_raw();
    let mut folder = Some(base);
    while let Some(path) = folder {
        let status = fs::metadata(path).ok()?;
        if (status.uid() != 0 && status.uid() != mine) || status.mode() & 0o022 != 0 {
            return Some(format!(
                "{} is a folder others can change, or that neither root nor this user owns",
                path.display()
            ));
        }
        folder = path.parent();
    }
    None
}

/// Whether `path` is on the same filesystem as `/`, asked of the
/// standard library rather than of the walk. The tests guard on this
/// one so that removing the rule under test makes them fail rather
/// than quietly skip.
fn same_device_as_root(path: &Path) -> bool {
    let (Ok(here), Ok(root)) = (fs::metadata(path), fs::metadata("/")) else {
        return false;
    };
    here.dev() == root.dev()
}

/// Whether the walk says `path` is on the same mount as `/`.
fn on_the_root_mount(path: &Path) -> bool {
    let anchor = rustix::fs::open(
        path,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .expect("the folder is opened");
    walk::is_on_the_root_mount(&anchor, &Options::default())
}

/// A folder `real` holding `keep.txt`, and `selected` inside it, under
/// `base`. Returns the real folder and the item to delete inside it.
fn tree_to_delete(base: &Path) -> (PathBuf, PathBuf) {
    let real = make_dir(&base.join("real"));
    let selected = make_dir(&real.join("selected"));
    make_dir(&selected.join("inner"));
    fs::write(selected.join("inner/data.txt"), b"delete me").expect("its item is written");
    fs::write(real.join("keep.txt"), b"keep me").expect("the bystander is written");
    (real, selected)
}

/// A mount probe that reports `mount` for `elsewhere` and the truth for
/// everything else.
fn mount_of_one_path(elsewhere: PathBuf, mount: Mount) -> walk::MountProbe {
    Box::new(move |path, real| if path == elsewhere { mount } else { real })
}

/// The mount `/` is really on, as the walk reads it.
fn root_mount_of_this_machine() -> Mount {
    let root = rustix::fs::open(
        "/",
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .expect("/ is opened");
    crate::safe_walk::mount_of(rustix::fd::AsFd::as_fd(&root), c"", AtFlags::EMPTY_PATH)
        .expect("the mount of / can be read")
}

/// A mount nothing else is on.
const ANOTHER_DRIVE: Mount = Mount {
    id: Some(u64::MAX),
    device: (u32::MAX, u32::MAX),
};

/// The uid of `nobody`, which no system gives to a real person. A test
/// running as root hands a fixture to it to mean "somebody root is not".
const NOBODY: u32 = 65534;

/// Makes `path` owned by somebody the walk does not trust, so that a
/// test about an untrusted owner tests the same rule however it is run.
///
/// Run as an ordinary user, the fixture already belongs to somebody who
/// is not root, and nothing is needed. Run as root, every fixture would
/// belong to root and the rule would never fire, so ownership goes to
/// `nobody`. Links are changed as links, never through them.
pub(super) fn make_untrusted(path: &Path) {
    if rustix::process::getuid().as_raw() != 0 {
        return;
    }
    std::os::unix::fs::lchown(path, Some(NOBODY), None).expect("root can give the fixture away");
}

/// A mount probe that tells the truth until it has been asked about
/// `elsewhere` `after` times, and reports `mount` from then on, while
/// recording every path it was asked about.
///
/// A test that relies on *which* of several questions about one path it
/// is answering can then check that premise instead of assuming it: the
/// recorded list says what the walk really asked, and in what order.
fn mount_of_one_path_after(
    elsewhere: PathBuf,
    mount: Mount,
    after: usize,
) -> (walk::MountProbe, Arc<Mutex<Vec<PathBuf>>>) {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let recording = Arc::clone(&asked);
    let probe: walk::MountProbe = Box::new(move |path, real| {
        let mut asked = recording.lock().expect("the record is not poisoned");
        asked.push(path.to_path_buf());
        let seen = asked.iter().filter(|seen| *seen == &elsewhere).count();
        if path == elsewhere && seen > after {
            mount
        } else {
            real
        }
    });
    (probe, asked)
}

/// A mount probe that tells the truth and, the first time it is asked
/// about `when`, runs `change`.
///
/// The walk asks this between opening a name and reading what it holds,
/// so a test can change the tree at exactly that moment and show that
/// what the walk goes on to read is the thing it opened, not the name.
fn mount_probe_that_changes_the_tree(
    when: PathBuf,
    change: impl Fn() + Send + Sync + 'static,
) -> walk::MountProbe {
    let done = std::sync::atomic::AtomicBool::new(false);
    Box::new(move |path, real| {
        if path == when && !done.swap(true, std::sync::atomic::Ordering::SeqCst) {
            change();
        }
        real
    })
}

// ---------------------------------------------------------------- //
// Rules 1 and 2 against this machine's own mounts, with the rules a //
// real deletion runs with.                                         //
// ---------------------------------------------------------------- //

/// Rule 1 on a real system link. `/bin`, `/sbin`, `/lib` and `/lib64`
/// are root-owned links on the root mount on every merged-`/usr`
/// distribution, which is the same shape as Fedora Atomic's
/// `/home -> var/home`. With the rules a real deletion runs with, the
/// walk follows one and lands on the folder it names.
///
/// parity: XFER-015
#[test]
fn a_root_owned_link_on_the_system_drive_is_followed() {
    let Some(link) = ["/bin", "/sbin", "/lib", "/lib64"]
        .into_iter()
        .map(Path::new)
        .find(|path| {
            fs::symlink_metadata(path).is_ok_and(|status| status.is_symlink() && status.uid() == 0)
                && same_device_as_root(&fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
        })
    else {
        skip("this machine has no root-owned system link such as /bin to resolve");
        return;
    };
    let leads_to = fs::canonicalize(link).expect("the link leads somewhere");

    let resolved = resolve(link, &trusting(TrustedOwner::ROOT));

    let resolved = resolved.expect("a root-owned link on the system drive is followed");
    assert_eq!(
        identity_of_descriptor(&resolved),
        identity_of(&leads_to),
        "{} did not resolve to {}",
        link.display(),
        leads_to.display()
    );
}

/// Rule 2 on a real system link. `/dev/fd` is a root-owned link to
/// `/proc/self/fd`, and `/dev` is a mount of its own, so however
/// trustworthy the link looks it is refused.
///
/// parity: XFER-015
#[test]
fn a_root_owned_link_on_another_drive_is_refused() {
    let link = Path::new("/dev/fd");
    let root_owned_link =
        fs::symlink_metadata(link).is_ok_and(|status| status.is_symlink() && status.uid() == 0);
    if !root_owned_link || same_device_as_root(Path::new("/dev")) {
        skip("this machine has no root-owned link on a mount of its own, such as /dev/fd");
        return;
    }

    let resolved = resolve(link, &trusting(TrustedOwner::ROOT));

    assert_eq!(resolved.err(), Some(other_mount_refusal(link)));
}

// ---------------------------------------------------------------- //
// Links that are followed. These need a home the walk can trust.    //
// ---------------------------------------------------------------- //

/// parity: XFER-015
#[test]
fn a_trusted_link_on_the_path_is_followed() {
    let Some(base) = trusted_base() else { return };
    let (real, selected) = tree_to_delete(base.path());
    symlink(&real, base.path().join("link")).expect("the link is created");

    let result = delete(&base.path().join("link/selected"), &trusting(trusted_here()));

    assert_eq!(result, Ok(()));
    assert!(!selected.exists(), "the selected folder is gone");
    assert_eq!(
        fs::read(real.join("keep.txt")).expect("nothing else was touched"),
        b"keep me"
    );
}

/// `/home -> var/home` is written relative on some systems, and a
/// target may lead back up with `..` before it leads down again. Both
/// resolve against the descriptor the walk holds, never by path.
///
/// parity: XFER-015
#[test]
fn a_relative_link_target_and_one_through_dotdot_are_followed() {
    let Some(base) = trusted_base() else { return };
    let (_real, selected) = tree_to_delete(base.path());
    let beside = make_dir(&base.path().join("beside"));
    symlink("real", base.path().join("relative")).expect("the relative link");
    symlink("../real", beside.join("up")).expect("the link through ..");

    let through_relative = delete(
        &base.path().join("relative/selected/inner"),
        &trusting(trusted_here()),
    );
    let through_dotdot = delete(&beside.join("up/selected"), &trusting(trusted_here()));

    assert_eq!(through_relative, Ok(()));
    assert_eq!(through_dotdot, Ok(()));
    assert!(!selected.exists(), "the selected folder is gone");
}

/// Where a link leads is not restricted: Fedora Atomic's `/home` is a
/// link on the root mount whose target is under `/var`, a mount of its
/// own. Only the link itself has to be on the root mount.
///
/// parity: XFER-015
#[test]
fn a_link_whose_target_is_on_another_drive_is_still_followed() {
    let Some(base) = trusted_base() else { return };
    let (real, selected) = tree_to_delete(base.path());
    symlink(&real, base.path().join("link")).expect("the link is created");
    let elsewhere = Options {
        // The folder the link leads to reports a mount of its own, as
        // /var does on Atomic.
        mounts: mount_of_one_path(real.clone(), ANOTHER_DRIVE),
        ..trusting(trusted_here())
    };

    let result = delete(&base.path().join("link/selected"), &elsewhere);

    assert_eq!(result, Ok(()), "the walk may cross mounts going down");
    assert!(!selected.exists(), "the selected folder is gone");
}

/// A link's target is walked under the same rule, component by
/// component, so a second link inside it is judged exactly as the first
/// was rather than being taken on trust.
///
/// parity: XFER-015
#[test]
fn a_link_whose_target_holds_another_link_is_checked_again() {
    let Some(base) = trusted_base() else { return };
    let middle = make_dir(&base.path().join("middle"));
    let (real, selected) = tree_to_delete(base.path());
    symlink(&middle, base.path().join("first")).expect("the first link is created");
    symlink(&real, middle.join("second")).expect("the second link is created");
    let through_both = base.path().join("first/second/selected");
    let trusted = trusting(trusted_here());

    let followed = delete(&through_both.join("inner/data.txt"), &trusted);
    // Only the folder that holds the *second* link is opened up. The
    // first link and its own folders are untouched, so a walk that
    // stopped checking after the first one would follow this.
    set_mode(&middle, 0o777);
    let refused = delete(&through_both, &trusted);
    set_mode(&middle, 0o755);

    assert_eq!(
        followed,
        Ok(()),
        "both links are followed while both folders are closed"
    );
    assert_eq!(
        refused,
        Err(reachable_link_refusal(&middle.join("second"), &middle))
    );
    assert!(selected.exists(), "nothing was deleted");
}

/// Group write alone is enough for somebody else to replace the link,
/// so it is refused exactly as world write is. Only link-following
/// stops: an ordinary deletion through the same folder goes ahead.
///
/// parity: XFER-015
#[test]
fn a_trusted_link_under_a_folder_others_can_change_is_refused() {
    let Some(base) = trusted_base() else { return };
    let open_to_all = make_dir(&base.path().join("shared"));
    let (real, selected) = tree_to_delete(&open_to_all);
    let link = open_to_all.join("link");
    symlink(&real, &link).expect("the link is created");
    let through_the_link = open_to_all.join("link/selected");
    let trusted = trusting(trusted_here());

    let followed = delete(&through_the_link.join("inner/data.txt"), &trusted);
    set_mode(&open_to_all, 0o775);
    let group_writable = delete(&through_the_link, &trusted);
    set_mode(&open_to_all, 0o777);
    let world_writable = delete(&through_the_link, &trusted);
    // Writable by others but not by the group: neither bit is the one
    // that matters on its own.
    set_mode(&open_to_all, 0o757);
    let other_writable = delete(&through_the_link, &trusted);
    let without_a_link = delete(&selected.join("inner"), &trusted);
    set_mode(&open_to_all, 0o755);

    assert_eq!(
        followed,
        Ok(()),
        "the same link is followed while the folder is closed"
    );
    assert_eq!(group_writable, Err(reachable_link_refusal(&link, &open_to_all)));
    assert_eq!(world_writable, Err(reachable_link_refusal(&link, &open_to_all)));
    assert_eq!(other_writable, Err(reachable_link_refusal(&link, &open_to_all)));
    assert_eq!(
        without_a_link,
        Ok(()),
        "a folder others can change stops links, not the walk"
    );
}

/// The hop limit is a limit, not a hint: a chain exactly as long as it
/// allows resolves, and one link more does not.
///
/// parity: XFER-015
#[test]
fn a_chain_of_links_resolves_up_to_the_hop_limit_and_no_further() {
    let Some(base) = trusted_base() else { return };
    let (_real, selected) = tree_to_delete(base.path());
    let link = |number: usize| base.path().join(format!("link {number}"));
    let limit = walk::MAX_LINK_HOPS;
    // The fixture is built from the limit, so the assertions below say
    // nothing about what the limit is. Keep it inside the band that
    // makes it a limit: deep enough for the link chains a real system
    // has (Fedora Atomic's /home -> var/home is one, and a chain of a
    // few is easy to arrive at), and short enough to bound both the
    // work and the depth, because the walk follows a link by calling
    // `descend` again and a long enough chain overflows the stack
    // rather than reaching the refusal. Raising it to 4000 makes
    // a_loop_of_links_is_refused abort the whole test process.
    assert!(
        (8..=64).contains(&limit),
        "the hop limit is {limit}, which is outside the band this rule means"
    );
    symlink("real", link(limit + 1)).expect("the last link");
    for number in (1..=limit).rev() {
        symlink(format!("link {}", number + 1), link(number)).expect("a link in the chain");
    }
    let trusted = trusting(trusted_here());

    // Starting at link 2 leaves exactly `limit` links to follow.
    let at_the_limit = delete(&link(2).join("selected/inner"), &trusted);
    let one_too_many = delete(&link(1).join("selected"), &trusted);

    assert_eq!(at_the_limit, Ok(()), "a chain of exactly {limit} links resolves");
    assert_eq!(one_too_many, Err(too_many_links_refusal(&link(limit + 1))));
    assert!(selected.exists(), "the longer chain deleted nothing");
}

/// Two links pointing at each other never resolve. The walk counts the
/// links it has followed and gives up rather than going round for ever,
/// and the refusal is its own, not the kernel's `ELOOP`.
///
/// parity: XFER-015
#[test]
fn a_loop_of_links_is_refused() {
    let Some(base) = trusted_base() else { return };
    symlink("second", base.path().join("first")).expect("the first link");
    symlink("first", base.path().join("second")).expect("the second link");

    let result = delete(&base.path().join("first/item"), &trusting(trusted_here()));

    // The last link followed before the count ran out; which of the two
    // it is depends only on the parity of the limit.
    let last = base.path().join(if walk::MAX_LINK_HOPS.is_multiple_of(2) {
        "first"
    } else {
        "second"
    });
    assert_eq!(result, Err(too_many_links_refusal(&last)));
}

/// Rule 2 against a fixture: the same link that was followed above is
/// refused as soon as it reports a mount of its own, and deleting on
/// that mount still works.
///
/// parity: XFER-015
#[test]
fn a_link_on_another_drive_is_refused_while_deleting_there_still_works() {
    let Some(base) = trusted_base() else { return };
    let (real, selected) = tree_to_delete(base.path());
    let link = base.path().join("link");
    symlink(&real, &link).expect("the link is created");
    let on_a_stick = Options {
        mounts: mount_of_one_path(link.clone(), ANOTHER_DRIVE),
        ..trusting(trusted_here())
    };

    let refused = delete(&link.join("selected"), &on_a_stick);
    let deleting_still_works = delete(&selected.join("inner"), &on_a_stick);

    assert_eq!(refused, Err(other_mount_refusal(&link)));
    assert_eq!(
        deleting_still_works,
        Ok(()),
        "only link-following is off on such a mount, not deletion"
    );
}

/// Where the kernel reports no mount ID (`statx` before Linux 5.8, or a
/// container's seccomp filter), the walk compares device numbers
/// instead of refusing every link: an attacker's filesystem always has
/// a device of its own, so the rule still holds, and `/home` as a link
/// keeps working on those systems.
///
/// parity: XFER-015
#[test]
fn without_mount_ids_the_device_decides() {
    let Some(base) = trusted_base() else { return };
    let (real, selected) = tree_to_delete(base.path());
    let link = base.path().join("link");
    symlink(&real, &link).expect("the link is created");
    let no_ids = |extra: Option<(PathBuf, Mount)>| Options {
        mounts: Box::new(move |path, mount| {
            let mount = match &extra {
                Some((elsewhere, replacement)) if path == elsewhere => *replacement,
                _ => mount,
            };
            // As a kernel that does not report STATX_MNT_ID answers.
            Mount { id: None, ..mount }
        }),
        ..trusting(trusted_here())
    };

    let on_the_system_drive = delete(&link.join("selected/inner"), &no_ids(None));
    let on_another_device = delete(
        &link.join("selected"),
        &no_ids(Some((link.clone(), ANOTHER_DRIVE))),
    );

    assert_eq!(
        on_the_system_drive,
        Ok(()),
        "the device still says it is the system drive"
    );
    assert_eq!(on_another_device, Err(other_mount_refusal(&link)));
    assert!(selected.exists(), "the refused deletion removed nothing");
}

/// A mount ID is reused once the mount that had it goes away, so the
/// ID alone does not say "the same mount". The device number is
/// compared with it, and a link whose mount claims the root mount's ID
/// on another device is refused.
///
/// parity: XFER-015
#[test]
fn a_reused_mount_id_on_another_device_is_not_the_root_mount() {
    let Some(base) = trusted_base() else { return };
    let (real, selected) = tree_to_delete(base.path());
    let link = base.path().join("link");
    symlink(&real, &link).expect("the link is created");
    let root_id = rustix::fs::statx(
        rustix::fs::CWD,
        "/",
        AtFlags::empty(),
        rustix::fs::StatxFlags::MNT_ID,
    )
    .expect("/ can be inspected")
    .stx_mnt_id;
    let reused = Options {
        mounts: mount_of_one_path(
            link.clone(),
            Mount {
                id: Some(root_id),
                device: (u32::MAX, u32::MAX),
            },
        ),
        ..trusting(trusted_here())
    };

    let result = delete(&link.join("selected"), &reused);

    assert_eq!(result, Err(other_mount_refusal(&link)));
    assert!(selected.exists(), "nothing was deleted");
}

/// Every folder above a link has to be trusted, not just the one that
/// holds it. A folder anybody can write to can be renamed within the
/// folder that holds it, so whoever can write to it chooses which of
/// the root-owned folders inside it the walk ends up in, and so which
/// root-owned link gets followed. The link and its own folder can be
/// beyond reproach and it still must not be followed.
///
/// Catches: dropping `at.trusted &&` from the folder arm of
/// `ParentWalk::step`.
///
/// parity: XFER-015
#[test]
fn a_link_below_a_folder_others_can_change_is_refused() {
    let Some(base) = trusted_base() else { return };
    let open_to_all = make_dir(&base.path().join("shared"));
    // Nothing about this folder or the link in it is open to anyone:
    // only the folder *above* them is.
    let closed = make_dir(&open_to_all.join("closed"));
    let (real, selected) = tree_to_delete(base.path());
    let link = closed.join("link");
    symlink(&real, &link).expect("the link is created");
    let through_the_link = closed.join("link/selected");
    let trusted = trusting(trusted_here());

    let followed = delete(&through_the_link.join("inner/data.txt"), &trusted);
    set_mode(&open_to_all, 0o775);
    let group_writable = delete(&through_the_link, &trusted);
    set_mode(&open_to_all, 0o777);
    let world_writable = delete(&through_the_link, &trusted);
    // Writable by others but not by the group: neither bit is the one
    // that matters on its own.
    set_mode(&open_to_all, 0o757);
    let other_writable = delete(&through_the_link, &trusted);
    set_mode(&open_to_all, 0o755);

    assert_eq!(
        followed,
        Ok(()),
        "the link is followed while every folder above it is closed"
    );
    // The refusal names the folder that holds the link, which is itself
    // closed: what is wrong is further up.
    assert_eq!(group_writable, Err(reachable_link_refusal(&link, &closed)));
    assert_eq!(world_writable, Err(reachable_link_refusal(&link, &closed)));
    assert_eq!(other_writable, Err(reachable_link_refusal(&link, &closed)));
    assert!(selected.exists(), "nothing was deleted");
}

/// A folder on a drive somebody else could have provided is not trusted
/// either, so a link below it is refused however genuine the link is.
/// Here the link really is on the root mount; only a folder above it
/// reports otherwise.
///
/// Catches: leaving the mount check out of `ParentWalk::is_trusted`, so
/// that only owner and mode decide whether a folder is trusted.
///
/// parity: XFER-015
#[test]
fn a_link_below_a_folder_on_another_drive_is_refused() {
    let Some(base) = trusted_base() else { return };
    let mid = make_dir(&base.path().join("mid"));
    let (real, selected) = tree_to_delete(base.path());
    let link = mid.join("link");
    symlink(&real, &link).expect("the link is created");
    let elsewhere = Options {
        mounts: mount_of_one_path(mid.clone(), ANOTHER_DRIVE),
        ..trusting(trusted_here())
    };

    let result = delete(&link.join("selected"), &elsewhere);

    assert_eq!(result, Err(reachable_link_refusal(&link, &mid)));
    assert!(selected.exists(), "nothing was deleted");
}

/// The link the walk checked is the link it reads. The walk opens the
/// name once and then asks the descriptor, so renaming another link
/// over that name afterwards changes nothing: the target it resolves is
/// the one the descriptor points at.
///
/// The probe runs between the `openat` and the `readlinkat`, which is
/// exactly the window a second lookup by name would sit in.
///
/// Catches: reading the link with `readlinkat(parent, name)` instead of
/// `readlinkat(link, "")`.
///
/// parity: XFER-015
#[test]
fn the_link_that_was_checked_is_the_link_that_is_read() {
    let Some(base) = trusted_base() else { return };
    let was_there = make_dir(&base.path().join("was there"));
    let swapped_in = make_dir(&base.path().join("swapped in"));
    let link = base.path().join("link");
    let other = base.path().join("other");
    symlink(&was_there, &link).expect("the link the walk opens");
    symlink(&swapped_in, &other).expect("the link somebody renames over it");
    let renaming = (other.clone(), link.clone());
    let swap_them = Options {
        mounts: mount_probe_that_changes_the_tree(link.clone(), move || {
            fs::rename(&renaming.0, &renaming.1).expect("the name is taken over");
        }),
        ..trusting(trusted_here())
    };

    let resolved = resolve(&link, &swap_them).expect("the link is followed");

    assert_eq!(
        identity_of_descriptor(&resolved),
        identity_of(&was_there),
        "the walk resolved the link that replaced the name, not the one it opened"
    );
    // The swap really happened, so the test is not passing by accident.
    assert_eq!(
        fs::read_link(&link).expect("the name still holds a link"),
        swapped_in
    );
}

/// `..` in a link's target is resolved against the folder the walk
/// holds open, not against the text of the path it came from. Those are
/// the same thing until somebody moves the folder, and then they are
/// not: a path-string `..` would strip the last name off the path the
/// walk was given and look *there*, while the descriptor leads to
/// wherever the folder it is holding now lives.
///
/// The fixture gives those two answers different contents, so the test
/// says which one the walk reached rather than merely that it reached
/// something. The folder is moved from inside the mount probe, which
/// runs while the walk is holding it open and before it resolves the
/// `..`.
///
/// parity: XFER-015
#[test]
fn a_dotdot_in_a_link_target_follows_the_open_folder_not_the_path() {
    let Some(base) = trusted_base() else { return };
    let was = make_dir(&base.path().join("was"));
    let now = make_dir(&base.path().join("now"));
    // Two folders of the same name, one under each parent. Which one
    // the walk lands in says which `..` it used.
    let by_the_path = make_dir(&was.join("prize"));
    let by_the_handle = make_dir(&now.join("prize"));
    let moving = make_dir(&was.join("moving"));
    // The link is inside `moving`, and leads back up and into `prize`.
    symlink("../prize", moving.join("up")).expect("the link through ..");
    let (from, to) = (moving.clone(), now.join("moving"));
    let move_it = Options {
        mounts: mount_probe_that_changes_the_tree(moving.join("up"), move || {
            fs::rename(&from, &to).expect("the folder is moved");
        }),
        ..trusting(trusted_here())
    };

    // The link itself names the prize folder, so resolving the link
    // resolves that folder.
    let resolved = resolve(&was.join("moving/up"), &move_it);

    // The move really happened, so the two answers really do differ.
    assert!(now.join("moving").is_dir(), "the probe did not move the folder");
    assert!(
        !was.join("moving").exists(),
        "the folder is no longer where it was"
    );
    let resolved = resolved.expect("the link is followed");
    assert_eq!(
        identity_of_descriptor(&resolved),
        identity_of(&by_the_handle),
        "the walk resolved .. against the path it was given, not the folder it held open"
    );
    assert_ne!(
        identity_of_descriptor(&resolved),
        identity_of(&by_the_path),
        "the two answers must differ, or this test proves nothing"
    );
}

/// A mount ID says which mount, and the device says which filesystem.
/// Both have to match `/`, so a mount that is on the root device but is
/// not the root mount — a bind mount of part of it — is refused. The
/// companion test covers the other way round.
///
/// Catches: comparing the device alone and ignoring the mount ID.
///
/// parity: XFER-015
#[test]
fn another_mount_on_the_root_device_is_not_the_root_mount() {
    let Some(base) = trusted_base() else { return };
    let (real, selected) = tree_to_delete(base.path());
    let link = base.path().join("link");
    symlink(&real, &link).expect("the link is created");
    let root_device = root_mount_of_this_machine().device;
    let bind_mount = Options {
        mounts: mount_of_one_path(
            link.clone(),
            Mount {
                id: Some(u64::MAX),
                device: root_device,
            },
        ),
        ..trusting(trusted_here())
    };

    let result = delete(&link.join("selected"), &bind_mount);

    assert_eq!(result, Err(other_mount_refusal(&link)));
    assert!(selected.exists(), "nothing was deleted");
}

/// `..` is judged like any other folder the walk opens. A folder that
/// was closed when the walk descended through it, and that somebody
/// opens up before the walk comes back to it through a link's `..`,
/// stops the links below it.
///
/// The probe opens the folder up at the moment the walk steps into the
/// folder beside it, which is after the first look and before the `..`.
///
/// Catches: `ParentWalk::step_up` carrying `at.trusted` over instead of
/// judging the folder it reaches.
///
/// parity: XFER-015
#[test]
fn a_folder_opened_up_after_the_walk_passed_it_stops_a_link_below_it() {
    let Some(base) = trusted_base() else { return };
    let (real, selected) = tree_to_delete(base.path());
    // A second link inside the target, which is what the walk judges
    // once `..` has taken it back to the folder that was opened up.
    symlink("selected", real.join("second")).expect("the second link");
    let beside = make_dir(&base.path().join("beside"));
    symlink("../real", beside.join("up")).expect("the link through ..");
    let opened_up = base.path().to_path_buf();
    let meanwhile = Options {
        mounts: mount_probe_that_changes_the_tree(beside.clone(), move || {
            set_mode(&opened_up, 0o777);
        }),
        ..trusting(trusted_here())
    };

    let result = delete(&beside.join("up/second/inner"), &meanwhile);
    // Read the mode before putting it back: the test means nothing if
    // the probe never ran, and a probe that never runs would leave the
    // folder closed and the link followed.
    let was_opened_up = fs::metadata(base.path()).expect("the base exists").mode() & 0o777;
    set_mode(base.path(), 0o755);

    assert_eq!(was_opened_up, 0o777, "the probe did not open the folder up");
    assert_eq!(result, Err(reachable_link_refusal(&real.join("second"), &real)));
    assert!(selected.exists(), "nothing was deleted");
}

/// `/` is judged like any other folder, not taken on trust because it
/// is `/`. Where it is not trusted, no link below it is followed.
///
/// The walk asks which mount a folder is on twice for `/`: once to
/// learn what the root mount *is*, and once to decide whether `/`
/// itself is trusted. Only the second may be answered differently, or
/// the root mount itself would change and the link would be refused
/// for being on another mount instead. So this test does depend on
/// that order, and it checks it rather than trusting it: the probe
/// records every question, and the first assertion is that the first
/// two were about `/`. If the walk ever stops asking twice, or asks
/// about something else first, this test fails on that assertion and
/// says so, instead of passing for a different reason than the one it
/// means to test.
///
/// Catches: `ParentWalk::root` setting `trusted` to `true` instead of
/// asking.
///
/// parity: XFER-015
#[test]
fn the_root_folder_is_judged_like_any_other() {
    let Some(base) = trusted_base() else { return };
    let (real, selected) = tree_to_delete(base.path());
    let link = base.path().join("link");
    symlink(&real, &link).expect("the link is created");
    let (mounts, asked) = mount_of_one_path_after(PathBuf::from("/"), ANOTHER_DRIVE, 1);
    let root_not_trusted = Options {
        mounts,
        ..trusting(trusted_here())
    };

    let result = delete(&link.join("selected"), &root_not_trusted);

    let asked = asked.lock().expect("the record is not poisoned").clone();
    let root = PathBuf::from("/");
    assert_eq!(
        asked.iter().take(2).collect::<Vec<_>>(),
        vec![&root, &root],
        "the walk no longer asks about / twice before anything else, so this test is \
         answering a different question than it means to: {asked:?}"
    );
    assert_eq!(result, Err(reachable_link_refusal(&link, base.path())));
    assert!(selected.exists(), "nothing was deleted");
}

// ---------------------------------------------------------------- //
// Refusals. These need nothing of the machine and always run.       //
// ---------------------------------------------------------------- //

/// The owner rule on its own: the same link, with the same folders
/// above it, is refused as soon as its owner is not one the walk
/// trusts. This is what a real build always does, where only root is
/// trusted. It stops links, not the walk: an ordinary deletion through
/// the same folders goes ahead.
///
/// parity: XFER-015
#[test]
fn an_untrusted_owner_stops_links_below_it_without_stopping_the_walk() {
    let root = temporary_folder();
    let (real, selected) = tree_to_delete(root.path());
    let link = real.join("link");
    symlink(&selected, &link).expect("the link is created");
    // Run as an ordinary user this link already belongs to somebody who
    // is not root; run as root it is given away, so that the rule being
    // tested is the same one either way.
    make_untrusted(&link);
    let as_root_would = trusting(TrustedOwner::ROOT);

    let through_a_link = delete(&link.join("inner"), &as_root_would);
    let without_a_link = delete(&selected.join("inner"), &as_root_would);

    assert_eq!(through_a_link, Err(untrusted_link_refusal(&link)));
    assert_eq!(without_a_link, Ok(()), "the walk itself is not stopped");
    assert!(!selected.join("inner").exists());
}

/// Everything on the way to the item has to be a folder. A file in the
/// middle of the path is a mistake, not something to open.
///
/// parity: XFER-015
#[test]
fn something_that_is_not_a_folder_on_the_path_is_refused() {
    let root = temporary_folder();
    let file = root.path().join("file.txt");
    fs::write(&file, b"not a folder").expect("the file is written");

    let result = delete(&file.join("inside"), &trusting(trusted_here()));

    assert_eq!(result, Err(not_a_folder_refusal(&file)));
    assert!(file.exists(), "nothing was deleted");
}

/// The helper is given the item's own absolute path and nothing else.
/// Anything that would have to be normalised first is refused, with the
/// refusal that names what is wrong with it.
///
/// parity: XFER-015
#[test]
fn only_the_items_own_absolute_path_is_accepted() {
    let options = trusting(trusted_here());
    let refusal_for = |path: &str| delete(Path::new(path), &options).expect_err("refused");

    assert_eq!(refusal_for("/"), walk::filesystem_root_refusal());
    for path in [
        "relative/folder",
        "/tmp/a/../b",
        "/tmp/.",
        "/tmp/a/./b",
        "/tmp/a//b",
    ] {
        assert_eq!(refusal_for(path), walk::own_absolute_path_refusal(), "{path}");
    }
    for path in ["/tmp/trailing/", "/tmp/"] {
        assert_eq!(
            refusal_for(path),
            walk::trailing_slash_refusal(Path::new(path)),
            "{path}"
        );
    }
}

/// The mount every link is measured against is `/` itself, read from a
/// descriptor rather than from a path or a table. `/dev` is a mount of
/// its own on any ordinary Linux system, and the walk sees that.
///
/// parity: XFER-015
#[test]
fn the_root_mount_is_the_one_every_link_is_measured_against() {
    if same_device_as_root(Path::new("/dev")) {
        skip("/dev is not a filesystem of its own on this machine");
        return;
    }

    assert!(on_the_root_mount(Path::new("/")));
    assert!(!on_the_root_mount(Path::new("/dev")));
}
