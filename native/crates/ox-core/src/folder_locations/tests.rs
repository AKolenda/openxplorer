// SPDX-License-Identifier: AGPL-3.0-only
//! Tests of moving a standard folder. `v2.0.0:desktop/tests` has only
//! `test_mount_resolution` for this; the rest follow `folder_locations.py`.

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::*;

/// A home folder, its configuration and state folders, and a data folder
/// for destinations, all in one temporary folder.
struct Fixture {
    _root: tempfile::TempDir,
    home: PathBuf,
    config: PathBuf,
    state: PathBuf,
    data: PathBuf,
    volatile: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("a temporary folder");
        let folder = |name: &str| {
            let path = root.path().join(name);
            fs::create_dir(&path).expect("the fixture folder is created");
            fs::canonicalize(&path).expect("the fixture folder resolves")
        };
        Self {
            home: folder("home"),
            config: folder("config"),
            state: root.path().join("state"),
            data: folder("data"),
            volatile: folder("volatile"),
            _root: root,
        }
    }

    fn user_dirs_file(&self) -> PathBuf {
        self.config.join("user-dirs.dirs")
    }

    /// A new folder `name` under the data folder.
    fn destination(&self, name: &str) -> PathBuf {
        let path = self.data.join(name);
        fs::create_dir_all(&path).expect("the destination is created");
        path
    }

    /// The relocation under test, writing through `updater`, with `mounts`
    /// as the mount table and the fixture's `volatile` folder as the only
    /// temporary root.
    fn relocation(&self, updater: FakeUpdater, mounts: Vec<MountEntry>) -> FolderRelocation {
        let locations = FolderLocations::new(self.home.clone(), &self.config);
        FolderRelocation::new(locations, self.state.clone())
            .with_updater(updater)
            .with_mount_reader(Box::new(move || Ok(mounts.clone())))
            .with_temporary_roots(vec![self.volatile.clone()])
    }

    fn updater(&self, behaviour: UpdaterBehaviour) -> FakeUpdater {
        FakeUpdater {
            user_dirs_file: self.user_dirs_file(),
            behaviour,
            calls: Arc::default(),
        }
    }
}

/// What the fake `xdg-user-dirs-update` does.
#[derive(Debug, Clone, Copy)]
enum UpdaterBehaviour {
    /// Writes the requested line, as the real tool does.
    Writes,
    /// Exits successfully without writing anything.
    IgnoresTheRequest,
    /// Is not installed.
    Missing,
}

#[derive(Debug, Clone)]
struct FakeUpdater {
    user_dirs_file: PathBuf,
    behaviour: UpdaterBehaviour,
    calls: Arc<AtomicUsize>,
}

impl UserDirsUpdater for FakeUpdater {
    fn set_folder(&self, folder: KnownFolder, path: &Path) -> Result<(), RelocationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.behaviour {
            UpdaterBehaviour::Writes => {
                let line = format!("XDG_{}_DIR=\"{}\"\n", folder.xdg_key(), path.display());
                fs::write(&self.user_dirs_file, line).expect("the fake tool writes the file");
                Ok(())
            }
            UpdaterBehaviour::IgnoresTheRequest => Ok(()),
            UpdaterBehaviour::Missing => Err(RelocationError::UpdaterMissing),
        }
    }
}

fn cifs_mount(path: &Path) -> MountEntry {
    MountEntry {
        root: "/".into(),
        path: path.to_string_lossy().into_owned(),
        filesystem: "cifs".into(),
        source: "//nas/media".into(),
        options: "rw".into(),
    }
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).expect("the path exists").permissions().mode() & 0o777
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// parity: PROP-031
#[test]
fn a_local_folder_is_checked_through_its_symlinks() {
    let fixture = Fixture::new();
    let real = fixture.destination("Incoming");
    let link = fixture.data.join("link-to-incoming");
    symlink(&real, &link).expect("the symlink is created");
    let relocation = fixture.relocation(fixture.updater(UpdaterBehaviour::Writes), Vec::new());

    let checked = relocation
        .check(KnownFolder::Downloads, &text(&link))
        .expect("a writable folder passes");

    assert_eq!(checked.path, real);
    assert_eq!(checked.uri, crate::location::file_uri(&real));
    assert!(!checked.is_network);
    assert_eq!(checked.previous, fixture.home.join("Downloads"));
}

/// A destination that must be refused.
struct RefusedCase {
    name: &'static str,
    value: String,
    message: &'static str,
}

/// parity: PROP-031
#[test]
fn unsafe_destinations_are_refused_in_the_python_wording() {
    let fixture = Fixture::new();
    let file = fixture.data.join("notes.txt");
    fs::write(&file, "text").expect("the file is written");
    let volatile = fixture.volatile.join("session");
    fs::create_dir(&volatile).expect("the folder is created");
    let gvfs = fixture.destination("gvfs/smb-share:server=nas,share=media");
    let gvfs_mount = MountEntry {
        path: text(&fixture.data.join("gvfs")),
        filesystem: "fuse.gvfsd-fuse".into(),
        ..MountEntry::default()
    };
    let not_a_folder = "The new location must be an existing folder.";
    let whole_home = "Choose a dedicated folder, not your entire home directory or the filesystem root.";
    let cases = [
        RefusedCase {
            name: "missing",
            value: text(&fixture.data.join("absent")),
            message: not_a_folder,
        },
        RefusedCase {
            name: "a file",
            value: text(&file),
            message: not_a_folder,
        },
        RefusedCase {
            name: "the home folder",
            value: "~".into(),
            message: whole_home,
        },
        RefusedCase {
            name: "the root",
            value: "/".into(),
            message: whole_home,
        },
        RefusedCase {
            name: "temporary",
            value: text(&volatile),
            message: "Use a persistent location, not a temporary or per-login GVfs path.",
        },
        RefusedCase {
            name: "a GVfs session",
            value: text(&gvfs),
            message: "GVfs session paths cannot be used as persistent standard folders.",
        },
        RefusedCase {
            name: "an unmounted share",
            value: "smb://nas/media/Downloads".into(),
            message: "This SMB folder is not mounted at a stable Linux path. Use “Set up network mount”, or \
                      mount it with CIFS first. A sidebar bookmark alone is not enough.",
        },
    ];
    let relocation = fixture.relocation(fixture.updater(UpdaterBehaviour::Writes), vec![gvfs_mount]);

    for case in cases {
        let refused = relocation.check(KnownFolder::Downloads, &case.value);

        let message = refused.expect_err(case.name).to_string();
        assert_eq!(message, case.message, "{}", case.name);
    }
}

/// The real `/tmp`, `/run` and `/var/tmp` are the default temporary roots.
///
/// parity: PROP-031
#[test]
fn the_system_temporary_folders_are_refused_by_default() {
    let fixture = Fixture::new();
    let locations = FolderLocations::new(fixture.home.clone(), &fixture.config);
    let relocation = FolderRelocation::new(locations, fixture.state.clone())
        .with_updater(fixture.updater(UpdaterBehaviour::Writes))
        .with_mount_reader(Box::new(|| Ok(Vec::new())));

    let refused = relocation.check(KnownFolder::Pictures, "/tmp");

    assert!(matches!(refused, Err(RelocationError::Temporary)), "{refused:?}");
}

/// parity: PROP-031
#[test]
fn a_folder_without_write_access_is_refused() {
    let fixture = Fixture::new();
    let locked = fixture.destination("Locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).expect("the mode is set");
    let relocation = fixture.relocation(fixture.updater(UpdaterBehaviour::Writes), Vec::new());

    let refused = relocation.check(KnownFolder::Documents, &text(&locked));

    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).expect("the mode is restored");
    // The superuser may write anywhere, so the rule cannot show there.
    if !rustix::process::geteuid().is_root() {
        let message = refused.expect_err("read-only").to_string();
        assert_eq!(message, "You do not have write access to this folder.");
    }
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::SettingsWindowsTests::test_mount_resolution`.
///
/// parity: PROP-031
#[test]
fn an_smb_folder_resolves_through_the_kernel_mount_of_its_share() {
    let fixture = Fixture::new();
    let mount_point = fixture.destination("nas-media");
    let downloads = fixture.destination("nas-media/Downloads");
    let relocation = fixture.relocation(
        fixture.updater(UpdaterBehaviour::Writes),
        vec![cifs_mount(&mount_point)],
    );

    let checked = relocation
        .check(KnownFolder::Downloads, "smb://NAS/Media/Downloads")
        .expect("a mounted share passes");

    assert_eq!(checked.path, downloads);
    assert!(checked.is_network);
    assert_eq!(checked.source.as_deref(), Some("//nas/media"));
}

/// parity: PROP-031
#[test]
fn nothing_changes_without_consent() {
    let fixture = Fixture::new();
    let destination = fixture.destination("Incoming");
    let updater = fixture.updater(UpdaterBehaviour::Writes);
    let calls = Arc::clone(&updater.calls);
    let relocation = fixture.relocation(updater, Vec::new());

    let refused = relocation.apply(KnownFolder::Downloads, &text(&destination), Consent::Missing);

    let message = refused.expect_err("no consent").to_string();
    assert_eq!(message, "Confirm the new location before applying it.");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(!fixture.state.exists(), "no backup was made");
}

/// parity: PROP-031
#[test]
fn applying_backs_up_changes_verifies_and_remembers_the_previous_folder() {
    let fixture = Fixture::new();
    fs::write(fixture.user_dirs_file(), "XDG_DOWNLOAD_DIR=\"$HOME/Old\"\n").expect("the file is written");
    let destination = fixture.destination("Incoming");
    let relocation = fixture.relocation(fixture.updater(UpdaterBehaviour::Writes), Vec::new());

    let applied = relocation
        .apply(KnownFolder::Downloads, &text(&destination), Consent::Given)
        .expect("the change is applied");

    let LocationChange::Changed { backup } = &applied.change else {
        panic!("the folder moved: {applied:?}");
    };
    let backed_up = fs::read_to_string(backup).expect("the backup is readable");
    assert_eq!(backed_up, "XDG_DOWNLOAD_DIR=\"$HOME/Old\"\n");
    assert_eq!(mode(backup), 0o600);
    assert_eq!(mode(backup.parent().expect("a backup folder")), 0o700);
    let location = relocation.location_of(KnownFolder::Downloads);
    assert_eq!(location.path, destination);
    assert_eq!(location.previous_path, Some(fixture.home.join("Old")));
    assert_eq!(location.default_path, fixture.home.join("Downloads"));
}

/// parity: PROP-031
#[test]
fn applying_the_current_folder_changes_nothing() {
    let fixture = Fixture::new();
    let destination = fixture.destination("Incoming");
    let line = format!("XDG_DOWNLOAD_DIR=\"{}\"\n", destination.display());
    fs::write(fixture.user_dirs_file(), line).expect("the file is written");
    let updater = fixture.updater(UpdaterBehaviour::Writes);
    let calls = Arc::clone(&updater.calls);
    let relocation = fixture.relocation(updater, Vec::new());

    let applied = relocation
        .apply(KnownFolder::Downloads, &text(&destination), Consent::Given)
        .expect("the same folder is accepted");

    assert_eq!(applied.change, LocationChange::Unchanged);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

/// parity: PROP-031
#[test]
fn a_change_the_configuration_does_not_keep_is_reported_with_the_backup_kept() {
    let fixture = Fixture::new();
    let destination = fixture.destination("Incoming");
    let relocation = fixture.relocation(fixture.updater(UpdaterBehaviour::IgnoresTheRequest), Vec::new());

    let refused = relocation.apply(KnownFolder::Music, &text(&destination), Consent::Given);

    let message = refused.expect_err("not retained").to_string();
    assert!(message.starts_with("The folder configuration did not retain the requested path."));
    let backups = fs::read_dir(fixture.state.join("location-backups")).expect("the backup folder exists");
    assert_eq!(backups.count(), 1);
    assert_eq!(relocation.location_of(KnownFolder::Music).previous_path, None);
}

/// parity: PROP-031
#[test]
fn a_missing_xdg_user_dirs_asks_for_it_to_be_installed() {
    let fixture = Fixture::new();
    let destination = fixture.destination("Incoming");
    let relocation = fixture.relocation(fixture.updater(UpdaterBehaviour::Missing), Vec::new());

    let refused = relocation.apply(KnownFolder::Videos, &text(&destination), Consent::Given);

    let message = refused.expect_err("missing tool").to_string();
    assert_eq!(
        message,
        "Install xdg-user-dirs before changing a standard folder."
    );
}
