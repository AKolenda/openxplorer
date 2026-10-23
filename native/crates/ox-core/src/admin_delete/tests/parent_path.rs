// SPDX-License-Identifier: AGPL-3.0-only
//! Resolving the folder that holds the item, which is the step a
//! privileged helper cannot take by path.
//!
//! A link on that path is followed only when nobody but a trusted user
//! could have put it there: the link's own owner is trusted, and every
//! folder from `/` down to it is owned by a trusted user and writable by
//! nobody else. In a real build the trusted user is root and only root,
//! which is what lets `/home` be a link to `/var/home` while a link
//! under a world-writable folder stays refused.
//!
//! An unprivileged test cannot create a root-owned link, so these tests
//! trust root and the user running them, and build their fixtures under
//! that user's own home, where the whole chain from `/` is normally
//! writable by nobody else.

use std::fs;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::super::walk::{self, Options, TrustedOwner};
use crate::test_support::temporary_folder;
use crate::transfer::{Cancellation, TransferError};

/// The owner these tests trust: root, as in a real build, and the user
/// running the test, which is the only uid it can give a link.
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

/// A folder under the test user's own home, with a file inside it, whose
/// whole path from `/` is writable by nobody but trusted users, so a
/// link created in it is one the walk may follow.
///
/// Returns `None`, saying why, where this machine's home does not meet
/// that — a group-writable home, or one under a world-writable folder.
/// The rule is then untestable here, not broken.
fn trusted_base() -> Option<TempDir> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let base = tempfile::Builder::new()
        .prefix("openxplorer-parent-path-")
        .tempdir_in(&home)
        .ok()?;
    // A temporary folder follows the umask, which is 0o002 on several
    // distributions and would make the fixture group-writable. The
    // fixture decides nothing here: the chain above it does.
    set_mode(base.path(), 0o755);
    // The same question the walk asks, asked here only to decide whether
    // the machine can host the fixture at all. What the tests assert is
    // the walk's own answer, never this one.
    let mine = rustix::process::getuid().as_raw();
    let mut folder = Some(base.path());
    while let Some(path) = folder {
        let status = fs::metadata(path).ok()?;
        if (status.uid() != 0 && status.uid() != mine) || status.mode() & 0o022 != 0 {
            eprintln!(
                "skipped: {} is not a folder only its owner can change, so this machine's home \
                 cannot host a trusted link",
                path.display()
            );
            return None;
        }
        folder = path.parent();
    }
    Some(base)
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

/// The shape of Fedora Atomic's `/home` as a link to `/var/home`: a link
/// nobody but a trusted user could have created, under folders nobody
/// else can change. Refusing it would make administrator deletion
/// useless on those systems, so it is followed.
///
/// parity: XFER-015
#[test]
fn a_trusted_link_on_the_path_is_followed() {
    let Some(base) = trusted_base() else { return };
    let (real, selected) = tree_to_delete(base.path());
    symlink(&real, base.path().join("link")).expect("the link is created");
    let through_the_link = base.path().join("link/selected");

    let result = delete(&through_the_link, &trusting(trusted_here()));

    assert_eq!(result, Ok(()));
    assert!(!selected.exists(), "the selected folder is gone");
    assert_eq!(
        fs::read(real.join("keep.txt")).expect("nothing else was touched"),
        b"keep me"
    );
}

/// The same link, with the same folders above it, is refused as soon as
/// its owner is not one the walk trusts. This is the rule a real build
/// always runs with, where the only trusted owner is root.
///
/// parity: XFER-015
#[test]
fn a_link_owned_by_an_untrusted_user_is_refused() {
    let root = temporary_folder();
    let (real, selected) = tree_to_delete(root.path());
    symlink(&real, root.path().join("link")).expect("the link is created");
    let through_the_link = root.path().join("link/selected");

    // TrustedOwner::ROOT is what the helper runs with, and the test user
    // is not root, so the link it just made is one no administrator
    // deletion may follow.
    let result = delete(&through_the_link, &trusting(TrustedOwner::ROOT));

    let message = result.expect_err("the link is refused").to_string();
    assert!(
        message.contains(&root.path().join("link").display().to_string()),
        "the refusal does not name the link: {message}"
    );
    assert!(message.contains("does not own"), "{message}");
    assert!(selected.exists(), "nothing was deleted");
}

/// A link the trusted user owns is still refused when somebody else can
/// change a folder on the way to it: they could swap the link for
/// another between the moment it is checked and the moment it is read.
///
/// parity: XFER-015
#[test]
fn a_trusted_link_under_a_folder_others_can_change_is_refused() {
    let Some(base) = trusted_base() else { return };
    let open_to_all = make_dir(&base.path().join("shared"));
    let (real, selected) = tree_to_delete(&open_to_all);
    symlink(&real, open_to_all.join("link")).expect("the link is created");
    let through_the_link = open_to_all.join("link/selected");
    let trusted = trusting(trusted_here());

    let followed = delete(&through_the_link.join("inner/data.txt"), &trusted);
    // Anyone can now create and remove names in the folder that holds
    // the link, so the link is no longer one only its owner could have
    // put there.
    set_mode(&open_to_all, 0o777);
    let refused = delete(&through_the_link, &trusted);
    set_mode(&open_to_all, 0o755);

    assert_eq!(
        followed,
        Ok(()),
        "the same link is followed while the folder is closed"
    );
    let message = refused.expect_err("the link is refused").to_string();
    assert!(
        message.contains(&open_to_all.display().to_string()),
        "the refusal does not name the folder others can change: {message}"
    );
    assert!(selected.exists(), "nothing was deleted");
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
    // first -> middle, and middle/second -> real, so the path to the
    // item goes through two links.
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
    let message = refused.expect_err("the second link is refused").to_string();
    assert!(
        message.contains(&middle.join("second").display().to_string()),
        "the refusal does not name the second link: {message}"
    );
    assert!(selected.exists(), "nothing was deleted");
}

/// Two links pointing at each other never resolve. The walk counts the
/// links it has followed and gives up rather than going round for ever.
///
/// parity: XFER-015
#[test]
fn a_loop_of_links_is_refused() {
    let Some(base) = trusted_base() else { return };
    symlink(base.path().join("second"), base.path().join("first")).expect("the first link");
    symlink(base.path().join("first"), base.path().join("second")).expect("the second link");

    let result = delete(&base.path().join("first/item"), &trusting(trusted_here()));

    let message = result.expect_err("the loop is refused").to_string();
    assert!(message.contains("loop"), "{message}");
}
