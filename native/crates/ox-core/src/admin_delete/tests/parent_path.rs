// SPDX-License-Identifier: AGPL-3.0-only
//! Resolving the folder that holds the item, which is the step a
//! privileged helper cannot take by path.
//!
//! A link on that path is followed only when nobody but a trusted user
//! could have put it there: the link's own owner is trusted, every
//! folder from `/` down to it is owned by a trusted user and writable
//! by nobody else, and all of them are on a mount only root could have
//! made. In a real build the trusted user is root and only root, which
//! is what lets `/home` be a link to `/var/home` while a link under a
//! world-writable folder, or on a stick the user plugged in, stays
//! refused.
//!
//! Each test asserts the refusal it expects by building the same
//! refusal the walk builds, so a test cannot pass because some *other*
//! rule happened to fire.
//!
//! An unprivileged test cannot create a root-owned link, so the tests
//! that need a link followed trust root and the user running them, and
//! build their fixtures under that user's own home, where the whole
//! chain from `/` is normally writable by nobody else. Where a machine's
//! home does not allow that, those tests say so loudly and do nothing;
//! set `OX_REQUIRE_TRUSTED_HOME=1` to turn that into a failure instead,
//! which is what a continuous-integration run should do. The tests that
//! assert a refusal need no such thing and always run.

use std::fs;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::super::walk::{
    self, not_a_folder_refusal, reachable_link_refusal, supplied_mount_refusal, too_many_links_refusal,
    untrusted_link_refusal, Options, TrustedOwner,
};
use crate::test_support::temporary_folder;
use crate::transfer::{Cancellation, TransferError};

/// Where a machine that cannot host a trusted link must fail instead of
/// saying so and moving on.
const REQUIRE_TRUSTED_HOME: &str = "OX_REQUIRE_TRUSTED_HOME";

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

/// A folder under the test user's own home, with its whole path from
/// `/` writable by nobody but trusted users, so a link created in it is
/// one the walk may follow.
///
/// Returns `None`, saying loudly why, where this machine's home does not
/// allow that — a group-writable home, or one under a world-writable
/// folder. With `OX_REQUIRE_TRUSTED_HOME=1` it fails the test instead,
/// so a continuous-integration run cannot go green while the tests that
/// need it quietly do nothing.
fn trusted_base() -> Option<TempDir> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let base = tempfile::Builder::new()
        .prefix("openxplorer-parent-path-")
        .tempdir_in(&home)
        .ok()?;
    set_mode(base.path(), 0o755);
    // The same question the walk asks, asked here only to decide whether
    // the machine can host the fixture at all. What the tests assert is
    // the walk's own answer, never this one.
    let mine = rustix::process::getuid().as_raw();
    let mut folder = Some(base.path());
    while let Some(path) = folder {
        let status = fs::metadata(path).ok()?;
        if (status.uid() != 0 && status.uid() != mine) || status.mode() & 0o022 != 0 {
            let reason = format!(
                "this machine's home cannot host a trusted link: {} is a folder others can \
                 change, or that neither root nor this user owns",
                path.display()
            );
            assert!(
                std::env::var_os(REQUIRE_TRUSTED_HOME).is_none(),
                "{REQUIRE_TRUSTED_HOME} is set, so this is a failure: {reason}"
            );
            eprintln!("SKIPPED (nothing was tested): {reason}");
            return None;
        }
        folder = path.parent();
    }
    Some(base)
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

// ---------------------------------------------------------------- //
// Links that are followed. These need a home the walk can trust.    //
// ---------------------------------------------------------------- //

/// The shape of Fedora Atomic's `/home` as a link to `/var/home`: a link
/// nobody but a trusted user could have created, under folders nobody
/// else can change, on the root filesystem. Refusing it would make
/// administrator deletion useless on those systems, so it is followed.
///
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

/// `/home -> /var/home` is written relative on some systems, and a
/// target may lead back up with `..` before it leads down again. Both
/// resolve against the descriptor the walk holds, never by path.
///
/// parity: XFER-015
#[test]
fn a_relative_link_target_and_one_through_dotdot_are_followed() {
    let Some(base) = trusted_base() else { return };
    let (_real, selected) = tree_to_delete(base.path());
    let beside = make_dir(&base.path().join("beside"));
    // relative -> real, and up -> ../real read from inside "beside".
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
    assert_eq!(
        refused,
        Err(reachable_link_refusal(&middle.join("second"), &middle))
    );
    assert!(selected.exists(), "nothing was deleted");
}

/// Group write alone is enough for somebody else to replace the link,
/// so it is refused exactly as world write is.
///
/// parity: XFER-015
#[test]
fn a_trusted_link_under_a_folder_others_can_change_is_refused() {
    let Some(base) = trusted_base() else { return };
    let open_to_all = make_dir(&base.path().join("shared"));
    let (real, selected) = tree_to_delete(&open_to_all);
    symlink(&real, open_to_all.join("link")).expect("the link is created");
    let link = open_to_all.join("link");
    let through_the_link = open_to_all.join("link/selected");
    let trusted = trusting(trusted_here());

    let followed = delete(&through_the_link.join("inner/data.txt"), &trusted);
    set_mode(&open_to_all, 0o775);
    let group_writable = delete(&through_the_link, &trusted);
    set_mode(&open_to_all, 0o777);
    let world_writable = delete(&through_the_link, &trusted);
    set_mode(&open_to_all, 0o755);

    assert_eq!(
        followed,
        Ok(()),
        "the same link is followed while the folder is closed"
    );
    assert_eq!(group_writable, Err(reachable_link_refusal(&link, &open_to_all)));
    assert_eq!(world_writable, Err(reachable_link_refusal(&link, &open_to_all)));
    assert!(selected.exists(), "nothing was deleted");
}

/// The hop limit is a limit, not a hint: a chain exactly as long as it
/// allows resolves, and one link more does not.
///
/// parity: XFER-015
#[test]
fn a_chain_of_links_resolves_up_to_the_hop_limit_and_no_further() {
    let Some(base) = trusted_base() else { return };
    let (_real, selected) = tree_to_delete(base.path());
    // link 1 -> link 2 -> … -> link N -> real, so walking "link 1"
    // costs exactly N hops.
    let link = |number: usize| base.path().join(format!("link {number}"));
    let limit = walk::MAX_LINK_HOPS;
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

/// A mount a user can supply says whatever its maker wants about owner
/// and mode, so a link on one is never followed, however trustworthy it
/// claims to be. Deleting on such a mount is not affected.
///
/// parity: XFER-015
#[test]
fn a_link_on_a_mount_a_user_could_have_supplied_is_refused() {
    let Some(base) = trusted_base() else { return };
    let (real, selected) = tree_to_delete(base.path());
    let link = base.path().join("link");
    symlink(&real, &link).expect("the link is created");
    let supplied = link.clone();
    let on_a_stick = Options {
        // Everything the walk would have believed about this link now
        // comes from a filesystem somebody plugged in.
        mount_trust: Box::new(move |path, trusted| path != supplied && trusted),
        ..trusting(trusted_here())
    };

    let refused = delete(&link.join("selected"), &on_a_stick);
    let deleting_still_works = delete(&selected.join("inner"), &on_a_stick);

    assert_eq!(refused, Err(supplied_mount_refusal(&link)));
    assert_eq!(
        deleting_still_works,
        Ok(()),
        "only link-following is off on such a mount, not deletion"
    );
}

// ---------------------------------------------------------------- //
// Refusals. These need nothing of the machine and always run.       //
// ---------------------------------------------------------------- //

/// The same link, with the same folders above it, is refused as soon as
/// its owner is not one the walk trusts. This is the rule a real build
/// always runs with, where the only trusted owner is root.
///
/// parity: XFER-015
#[test]
fn a_link_owned_by_an_untrusted_user_is_refused() {
    let root = temporary_folder();
    let (real, selected) = tree_to_delete(root.path());
    let link = root.path().join("link");
    symlink(&real, &link).expect("the link is created");

    // TrustedOwner::ROOT is what the helper runs with, and the test user
    // is not root, so the link it just made is one no administrator
    // deletion may follow.
    let result = delete(&link.join("selected"), &trusting(TrustedOwner::ROOT));

    assert_eq!(result, Err(untrusted_link_refusal(&link)));
    assert!(selected.exists(), "nothing was deleted");
}

/// A folder the walk does not trust stops link-following below it, but
/// it does not stop the walk: an ordinary deletion through that folder
/// goes ahead, because no link was involved.
///
/// parity: XFER-015
#[test]
fn an_untrusted_folder_stops_links_below_it_without_stopping_the_walk() {
    let root = temporary_folder();
    let (real, selected) = tree_to_delete(root.path());
    let link = real.join("link");
    symlink(&selected, &link).expect("the link is created");
    // The test user owns every folder here and is not root, so with the
    // rules a real deletion runs with none of them is trusted.
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
/// Anything that would have to be normalised first is refused, because
/// a normalised path is no longer the path the user confirmed.
///
/// parity: XFER-015
#[test]
fn only_the_items_own_absolute_path_is_accepted() {
    let refused = [
        "/",
        "relative/folder",
        "/tmp/a/../b",
        "/tmp/.",
        "/tmp/a/./b",
        "/tmp/a//b",
        "/tmp/trailing/",
    ];

    for path in refused {
        let result = delete(Path::new(path), &trusting(trusted_here()));

        let message = result.expect_err(&format!("{path} is refused")).to_string();
        assert!(
            message.contains("absolute path") || message.contains("Filesystem roots"),
            "{path} was refused for the wrong reason: {message}"
        );
    }
}
