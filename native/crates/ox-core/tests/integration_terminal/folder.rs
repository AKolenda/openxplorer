// SPDX-License-Identifier: AGPL-3.0-only
//! Choosing and checking the folder a terminal opens in. Ports the
//! `prepare_directory` and `checked_directory` cases of `TerminalTests`
//! and `DispatchTests::test_terminal_branch_connected`.

use std::cell::RefCell;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use ox_core::entry::{inspect, Entry, EntryError, EntryKind};
use ox_core::integration::{
    checked_directory, open_terminal_in_background, prepare_directory, DirectoryChecks, PreparedDirectory,
    Sandbox, TerminalError,
};
use ox_core::location::file_uri;
use ox_core::transfer::Cancellation;

use super::{real_path, temporary_folder};
use crate::integration_support::item;

/// The metadata, local path and write guard of the Python fixture's
/// `prepare`, recording what they were asked.
#[derive(Debug)]
struct TestChecks {
    entry: Entry,
    local_path: Option<PathBuf>,
    inspected: RefCell<Vec<String>>,
    guarded: RefCell<Vec<String>>,
}

impl TestChecks {
    /// A folder whose local path is `local_path`.
    fn folder_at(local_path: &Path) -> Self {
        Self::new(item(EntryKind::Directory, "folder"), Some(local_path.to_owned()))
    }

    fn new(entry: Entry, local_path: Option<PathBuf>) -> Self {
        Self {
            entry,
            local_path,
            inspected: RefCell::default(),
            guarded: RefCell::default(),
        }
    }
}

impl DirectoryChecks for TestChecks {
    type Refusal = String;

    fn inspect(&self, uri: &str, _cancel: &Cancellation) -> Result<Entry, EntryError> {
        self.inspected.borrow_mut().push(uri.to_owned());
        Ok(self.entry.clone())
    }

    fn local_path(&self, _uri: &str) -> Option<PathBuf> {
        self.local_path.clone()
    }

    fn check_writable(&self, uri: &str) -> Result<(), String> {
        self.guarded.borrow_mut().push(uri.to_owned());
        Ok(())
    }
}

/// The real query and a `file://` lookup, as the app wires them.
struct GioChecks;

impl DirectoryChecks for GioChecks {
    type Refusal = String;

    fn inspect(&self, uri: &str, cancel: &Cancellation) -> Result<Entry, EntryError> {
        inspect(uri, Some(cancel.cancellable()))
    }

    fn local_path(&self, uri: &str) -> Option<PathBuf> {
        gio::prelude::FileExt::path(&gio::File::for_uri(uri))
    }

    fn check_writable(&self, _uri: &str) -> Result<(), String> {
        Ok(())
    }
}

fn prepare(uri: &str, checks: &TestChecks) -> Result<PreparedDirectory, TerminalError> {
    prepare_directory(uri, checks, &Cancellation::new())
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_local_directory`
/// parity: OPEN-017
#[test]
fn a_local_folder_opens_in_itself() {
    let root = temporary_folder();

    let prepared = prepare(&file_uri(root.path()), &TestChecks::folder_at(root.path())).expect("prepared");

    assert_eq!(prepared.path, real_path(root.path()));
    assert!(!prepared.is_network);
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_fresh_file_uses_parent`
/// parity: OPEN-017
#[test]
fn a_file_opens_its_folder() {
    let root = temporary_folder();
    let checks = TestChecks::new(item(EntryKind::File, "movie.mp4"), Some(root.path().to_owned()));
    let root_uri = file_uri(root.path());

    let prepared = prepare(&format!("{root_uri}/movie.mp4"), &checks).expect("prepared");

    assert_eq!(prepared.uri, root_uri);
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_folder_with_file_extension`
/// parity: OPEN-017
#[test]
fn a_folder_named_like_a_file_opens_in_itself() {
    let root = temporary_folder();
    let folder = root.path().join("media.mp4");
    fs::create_dir(&folder).expect("folder");

    let prepared = prepare(&file_uri(&folder), &TestChecks::folder_at(&folder)).expect("prepared");

    assert_eq!(prepared.uri, file_uri(&folder));
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_smb_export_is_local_shell`
/// parity: OPEN-006, OPEN-017
#[test]
fn a_share_opens_as_a_local_shell_in_its_mount() {
    let root = temporary_folder();

    let prepared =
        prepare("smb://studio-nas/Projects", &TestChecks::folder_at(root.path())).expect("prepared");

    assert!(prepared.is_network);
    assert_eq!(prepared.path, real_path(root.path()));
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_bare_server_rejected_before_inspection`
/// parity: OPEN-017
#[test]
fn a_server_listing_is_refused_before_it_is_queried() {
    let root = temporary_folder();
    let checks = TestChecks::folder_at(root.path());

    let refused = prepare("smb://studio-nas", &checks);

    assert!(
        matches!(refused, Err(TerminalError::ServerListing)),
        "{refused:?}"
    );
    assert!(checks.inspected.borrow().is_empty());
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_smb_missing_fuse_mount_explains`
/// parity: OPEN-006, OPEN-017
#[test]
fn a_share_without_a_local_mount_explains_how_to_mount_it() {
    let checks = TestChecks::new(item(EntryKind::Directory, "Projects"), Some(PathBuf::new()));

    let refused = prepare("smb://studio-nas/Projects", &checks);

    assert!(
        matches!(refused, Err(TerminalError::NeedsLocalMount)),
        "{refused:?}"
    );
    assert!(refused.expect_err("refused").to_string().contains("gvfs-fuse"));
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_symbolic_link_rejected`
/// parity: OPEN-017
#[test]
fn a_symbolic_link_is_refused() {
    let root = temporary_folder();
    let link = Entry {
        is_symlink: true,
        ..item(EntryKind::Symlink, "link")
    };

    let refused = prepare(
        &file_uri(root.path()),
        &TestChecks::new(link, Some(root.path().to_owned())),
    );

    assert!(
        matches!(refused, Err(TerminalError::LinkOrSpecialFile)),
        "{refused:?}"
    );
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_special_file_rejected`
/// parity: OPEN-017
#[test]
fn a_special_file_is_refused() {
    let root = temporary_folder();
    let checks = TestChecks::new(item(EntryKind::Special, "pipe"), Some(root.path().to_owned()));

    let refused = prepare(&file_uri(root.path()), &checks);

    assert!(
        matches!(refused, Err(TerminalError::LinkOrSpecialFile)),
        "{refused:?}"
    );
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_unknown_metadata_rejected`
/// parity: OPEN-017
#[test]
fn an_item_of_unknown_type_is_refused() {
    let root = temporary_folder();
    let checks = TestChecks::new(item(EntryKind::Unknown, "thing"), Some(root.path().to_owned()));

    let refused = prepare(&file_uri(root.path()), &checks);

    assert!(
        matches!(refused, Err(TerminalError::LinkOrSpecialFile)),
        "{refused:?}"
    );
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_snapshot_rejected`
/// parity: OPEN-017
#[test]
fn a_snapshot_folder_is_refused() {
    let root = temporary_folder();

    let refused = prepare(
        "smb://studio-nas/Projects/.zfs/snapshot/auto-2026-01-01",
        &TestChecks::folder_at(root.path()),
    );

    let message = refused.expect_err("refused").to_string();
    assert!(message.starts_with("Previous-version"), "{message}");
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_snapshot_alias_rejected_after_resolving`
/// parity: OPEN-017
#[test]
fn a_link_into_a_snapshot_is_refused_after_resolving() {
    let root = temporary_folder();
    let snapshot = root.path().join(".zfs/snapshot/day");
    fs::create_dir_all(&snapshot).expect("snapshot");
    let alias = root.path().join("alias");
    symlink(&snapshot, &alias).expect("alias");

    let refused = prepare(&file_uri(&alias), &TestChecks::folder_at(&alias));

    assert!(
        matches!(refused, Err(TerminalError::PreviousVersion)),
        "{refused:?}"
    );
}

/// A snapshot folder whose name is Latin-1, as on an older CIFS share,
/// behind a link with a UTF-8 name. Python's `unquote` still finds the
/// `.snapshot` of the resolved folder.
/// parity: OPEN-017
#[test]
fn a_link_into_a_snapshot_with_a_non_utf8_name_is_refused() {
    let root = temporary_folder();
    let snapshot = root.path().join(".snapshot").join(OsStr::from_bytes(b"caf\xE9"));
    fs::create_dir_all(snapshot.join("sub")).expect("snapshot");
    let latest = root.path().join("latest");
    symlink(&snapshot, &latest).expect("link");
    let folder = latest.join("sub");

    let refused = prepare(&file_uri(&folder), &TestChecks::folder_at(&folder));

    assert!(
        matches!(refused, Err(TerminalError::PreviousVersion)),
        "{refused:?}"
    );
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_custom_snapshot_guard_called_for_local_alias`
/// parity: OPEN-017
#[test]
fn the_write_guard_also_checks_the_resolved_local_folder() {
    let root = temporary_folder();
    let checks = TestChecks::folder_at(root.path());

    prepare("smb://studio-nas/Projects", &checks).expect("prepared");

    let local_uri = file_uri(&real_path(root.path()));
    assert!(
        checks.guarded.borrow().contains(&local_uri),
        "{:?}",
        checks.guarded.borrow()
    );
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_absolute_cwd_required`
/// parity: OPEN-020
#[test]
fn a_relative_folder_is_refused() {
    let refused = checked_directory(Path::new("relative"));

    assert!(
        matches!(refused, Err(TerminalError::InvalidDirectory)),
        "{refused:?}"
    );
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_directory_must_exist`
/// parity: OPEN-020
#[test]
fn a_missing_folder_is_refused() {
    let root = temporary_folder();

    let refused = checked_directory(&root.path().join("missing"));

    let is_not_found = matches!(&refused, Err(TerminalError::Io { error, .. }) if error.kind() == std::io::ErrorKind::NotFound);
    assert!(is_not_found, "{refused:?}");
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_cwd_file_rejected`
/// parity: OPEN-020
#[test]
fn a_file_is_not_a_starting_folder() {
    let root = temporary_folder();
    let file = root.path().join("a");
    fs::write(&file, "x").expect("file");

    let refused = checked_directory(&file);

    assert!(
        matches!(refused, Err(TerminalError::NotADirectory)),
        "{refused:?}"
    );
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_cwd_controls_rejected`
/// parity: OPEN-020
#[test]
fn a_folder_with_control_characters_is_refused() {
    let root = temporary_folder();

    let refused = checked_directory(&root.path().join("a\nb"));

    assert!(
        matches!(refused, Err(TerminalError::InvalidDirectory)),
        "{refused:?}"
    );
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_uri_schemes_rejected`
/// parity: OPEN-017
#[test]
fn web_script_and_credential_addresses_are_refused() {
    let root = temporary_folder();
    let checks = TestChecks::folder_at(root.path());

    for uri in [
        "https://example.org",
        "javascript:alert(1)",
        "smb://user:password@studio-nas/Projects",
    ] {
        let refused = prepare(uri, &checks);

        assert!(
            matches!(refused, Err(TerminalError::Location(_))),
            "{uri}: {refused:?}"
        );
    }
    assert!(checks.inspected.borrow().is_empty());
}

/// Ported from `v2.0.0:desktop/tests/test_terminal_security.py::TerminalTests::test_cancellation_checked`
/// parity: OPEN-017
#[test]
fn a_cancelled_request_stops() {
    let root = temporary_folder();
    let cancel = Cancellation::new();
    cancel.cancel();

    let refused = prepare_directory(
        &file_uri(root.path()),
        &TestChecks::folder_at(root.path()),
        &cancel,
    );

    assert!(matches!(refused, Err(TerminalError::Cancelled)), "{refused:?}");
}

/// Ported from `v2.0.0:desktop/tests/test_rc2.py::DispatchTests::test_terminal_branch_connected`
/// parity: OPEN-017
#[test]
fn a_real_folder_is_prepared_through_gio() {
    let root = temporary_folder();

    let prepared =
        prepare_directory(&file_uri(root.path()), &GioChecks, &Cancellation::new()).expect("prepared");

    assert_eq!(prepared.path, real_path(root.path()));
}

/// parity: OPEN-017
#[test]
fn the_whole_request_runs_on_a_worker_thread_and_stops_when_cancelled() {
    let root = temporary_folder();
    let cancel = Cancellation::new();
    cancel.cancel();

    let request = open_terminal_in_background(file_uri(root.path()), GioChecks, Sandbox::Host, cancel);
    let refused = glib::MainContext::new().block_on(request);

    assert!(matches!(refused, Err(TerminalError::Cancelled)), "{refused:?}");
}
