// SPDX-License-Identifier: AGPL-3.0-only
//! The opt-in "Show in folder" registration against disposable XDG
//! folders.
//!
//! Ports `RevealTests` of `desktop/tests/test_v07.py` and the
//! compatibility checks of `RebrandTests` in `desktop/tests/test_v08.py`.
//! The files are real; the folders are temporary.

use std::fs;
use std::os::unix::fs::{symlink, MetadataExt};
use std::path::{Path, PathBuf};

use ox_core::integration::{
    RevealError, RevealPaths, RevealRegistration, Sandbox, AUTOSTART_FILE, MANAGED_MARKER, SERVICE_FILE,
};
use tempfile::TempDir;

/// A registration whose settings, configuration and data folders are
/// inside one empty temporary folder, as in the Python fixture.
struct Fixture {
    root: TempDir,
    registration: RevealRegistration,
}

impl Fixture {
    fn new() -> Self {
        Self::in_sandbox(Sandbox::Host)
    }

    fn in_sandbox(sandbox: Sandbox) -> Self {
        let root = tempfile::tempdir().expect("temporary folder");
        let paths = RevealPaths {
            settings: root.path().join("winspace"),
            config_home: root.path().join("config"),
            data_home: root.path().join("data"),
        };
        let registration = RevealRegistration::new(&paths, sandbox);
        Self { root, registration }
    }

    fn files(&self) -> Vec<PathBuf> {
        self.registration.managed_files().map(Path::to_owned).collect()
    }

    /// The first managed file, the D-Bus service file, with its folder.
    fn service_file(&self) -> PathBuf {
        let path = self.files().remove(0);
        fs::create_dir_all(path.parent().expect("a parent folder")).expect("folder");
        path
    }

    fn is_root_empty(&self) -> bool {
        fs::read_dir(self.root.path()).expect("list").next().is_none()
    }
}

/// Ported from `desktop/tests/test_v07.py::RevealTests::test_disabled_by_default`
/// parity: INT-015, INT-016
#[test]
fn show_in_folder_is_off_until_enabled() {
    let fixture = Fixture::new();

    assert!(!fixture.registration.is_enabled());
    assert!(fixture.is_root_empty());
}

/// Ported from `desktop/tests/test_v07.py::RevealTests::test_enable_exact_per_user_files`
/// parity: INT-015
#[test]
fn enabling_writes_exactly_two_private_per_user_files() {
    let fixture = Fixture::new();

    fixture.registration.enable().expect("enable");

    assert!(fixture.registration.is_enabled());
    let files = fixture.files();
    assert_eq!(files.len(), 2);
    for file in files {
        assert_eq!(
            fs::metadata(&file).expect("stat").mode() & 0o777,
            0o600,
            "{}",
            file.display()
        );
    }
}

/// Ported from `desktop/tests/test_v07.py::RevealTests::test_enable_idempotent`
/// parity: INT-015
#[test]
fn enabling_twice_is_harmless() {
    let fixture = Fixture::new();

    fixture.registration.enable().expect("first enable");
    fixture.registration.enable().expect("second enable");

    assert!(fixture.registration.is_enabled());
}

/// Ported from `desktop/tests/test_v07.py::RevealTests::test_disable_exact_files`
/// parity: INT-015
#[test]
fn disabling_removes_exactly_the_managed_files() {
    let fixture = Fixture::new();
    fixture.registration.enable().expect("enable");

    let disabled = fixture.registration.disable().expect("disable");

    assert!(!fixture.registration.is_enabled());
    assert!(fixture.files().iter().all(|file| !file.exists()));
    assert!(disabled.preserved_modified_files.is_empty());
}

/// Ported from `desktop/tests/test_v07.py::RevealTests::test_refuse_foreign_override`
/// parity: INT-015, INT-016
#[test]
fn a_foreign_override_is_refused_and_left_unchanged() {
    let fixture = Fixture::new();
    let service = fixture.service_file();
    fs::write(&service, "other manager").expect("foreign override");

    let refused = fixture.registration.enable();

    assert!(
        matches!(&refused, Err(RevealError::ForeignOverride(path)) if *path == service),
        "{refused:?}"
    );
    assert_eq!(
        refused.expect_err("refused").to_string(),
        format!(
            "An existing user override needs review before enabling OpenXplorer: {}",
            service.display()
        )
    );
    assert_eq!(fs::read_to_string(&service).expect("read"), "other manager");
}

/// Ported from `desktop/tests/test_v07.py::RevealTests::test_preserve_modified_override`
/// parity: INT-015
#[test]
fn disabling_keeps_a_file_the_user_modified() {
    let fixture = Fixture::new();
    fixture.registration.enable().expect("enable");
    let service = fixture.service_file();
    fs::write(&service, "my modified override").expect("modify");

    let disabled = fixture.registration.disable().expect("disable");

    assert_eq!(
        fs::read_to_string(&service).expect("read"),
        "my modified override"
    );
    assert!(disabled.preserved_modified_files.contains(&service));
}

/// Ported from `desktop/tests/test_v07.py::RevealTests::test_refuse_symlink`
/// parity: INT-015
#[test]
fn a_symlinked_file_is_refused() {
    let fixture = Fixture::new();
    let service = fixture.service_file();
    symlink(fixture.root.path().join("elsewhere"), &service).expect("symlink");

    let refused = fixture.registration.enable();

    assert!(
        matches!(&refused, Err(RevealError::Symlink(path)) if *path == service),
        "{refused:?}"
    );
    assert!(!fixture.root.path().join("elsewhere").exists());
}

/// Ported from `desktop/tests/test_v07.py::RevealTests::test_activation_does_not_open_ui`
/// parity: INT-015
#[test]
fn both_files_start_the_service_without_a_window() {
    assert!(SERVICE_FILE.contains("--filemanager-service"));
    assert!(AUTOSTART_FILE.contains("--filemanager-service"));
    assert!(AUTOSTART_FILE.contains("NoDisplay=true\n"));
}

/// Ported from `desktop/tests/test_v07.py::RevealTests::test_no_kill_or_system_files`
/// parity: INT-015
#[test]
fn nothing_outside_the_user_folders_is_written_or_stopped() {
    let fixture = Fixture::new();

    assert!(!SERVICE_FILE.contains("kill"));
    assert!(fixture
        .files()
        .iter()
        .all(|file| file.starts_with(fixture.root.path())));
}

/// Ported from `desktop/tests/test_v08.py::RebrandTests::test_marker_retained`
/// parity: INT-029
#[test]
fn the_managed_marker_is_the_legacy_one() {
    assert_eq!(
        MANAGED_MARKER,
        "# Managed by Winspace: file-manager-integration v1\n"
    );
    assert!(SERVICE_FILE.starts_with(MANAGED_MARKER));
    assert!(AUTOSTART_FILE.starts_with(MANAGED_MARKER));
}

/// Ported from `desktop/tests/test_v08.py::RebrandTests::test_previous_service_exact`
/// parity: INT-015, INT-029
#[test]
fn the_service_file_is_byte_for_byte_the_legacy_one() {
    let expected = format!(
        "{MANAGED_MARKER}[D-BUS Service]\nName=org.freedesktop.FileManager1\n\
         Exec=/usr/bin/winspace --filemanager-service\n"
    );

    assert_eq!(SERVICE_FILE, expected);
}

/// Ported from `desktop/tests/test_v08.py::RebrandTests::test_previous_autostart_recognized`
/// parity: INT-015, INT-029
#[test]
fn the_autostart_entry_keeps_the_legacy_name() {
    assert!(AUTOSTART_FILE.contains("Name=Winspace Show in Folder integration\n"));
}

/// The whole text of `AUTOSTART` in `desktop/reveal_integration.py`:
/// enabling and disabling compare the file byte for byte, so any change
/// would make an entry the Python app wrote look like a foreign override.
/// parity: INT-015, INT-029
#[test]
fn the_autostart_entry_is_byte_for_byte_the_legacy_one() {
    let expected = format!(
        "{MANAGED_MARKER}[Desktop Entry]\n\
         Type=Application\n\
         Name=Winspace Show in Folder integration\n\
         Comment=Handle explicit file-reveal requests without opening a window at login\n\
         Exec=/usr/bin/winspace --filemanager-service\n\
         Icon=io.winspace.Development\n\
         NoDisplay=true\n\
         X-GNOME-Autostart-enabled=true\n"
    );

    assert_eq!(AUTOSTART_FILE, expected);
}

/// parity: INT-015
#[test]
fn a_failed_write_removes_the_files_already_written() {
    let fixture = Fixture::new();
    // The autostart entry is written second; a file where its folder
    // belongs makes that write fail after the service file was written.
    fs::create_dir_all(fixture.root.path().join("config")).expect("folder");
    fs::write(fixture.root.path().join("config/autostart"), "not a folder").expect("blocker");

    let failed = fixture.registration.enable();

    assert!(matches!(failed, Err(RevealError::Io { .. })), "{failed:?}");
    assert!(fixture.files().iter().all(|file| !file.exists()));
}

/// parity: INT-015
#[test]
fn the_record_keeps_the_state_before_the_first_enable() {
    let fixture = Fixture::new();
    fixture.registration.enable().expect("first enable");
    fixture.registration.enable().expect("second enable");

    let record = fixture.root.path().join("winspace/reveal-integration.json");
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(&record).expect("record")).expect("JSON");

    let previous = record["previous"].as_object().expect("previous states");
    assert_eq!(previous.len(), 2);
    assert!(previous.values().all(serde_json::Value::is_null), "{previous:?}");

    fixture.registration.disable().expect("disable");
    assert!(!fixture
        .root
        .path()
        .join("winspace/reveal-integration.json")
        .exists());
}

/// parity: INT-015
#[test]
fn inside_flatpak_enabling_is_refused_without_writing() {
    let fixture = Fixture::in_sandbox(Sandbox::Flatpak);

    let refused = fixture.registration.enable();

    assert!(matches!(refused, Err(RevealError::Sandboxed)), "{refused:?}");
    assert!(fixture.is_root_empty());
}

/// parity: INT-015
#[test]
fn enabling_runs_on_a_worker_thread() {
    let fixture = Fixture::new();

    let enabling = fixture.registration.run_in_background(RevealRegistration::enable);
    glib::MainContext::new().block_on(enabling).expect("enable");

    assert!(fixture.registration.is_enabled());
}
