// SPDX-License-Identifier: AGPL-3.0-only
//! Making the app the default file manager and ZIP handler, and
//! restoring the previous handlers, against a recorded desktop.
//!
//! Ports `DefaultsTests` of `desktop/tests/test_rc3.py`, whose `run` double
//! answers `xdg-mime` from a dictionary; [`RecordedDesktop`] does the same
//! for [`MimeDefaults`]. The record file is real and lives in a temporary
//! folder.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::sync::{Arc, Mutex, MutexGuard};

use ox_core::integration::{
    DefaultApps, DefaultAppsError, DesktopId, MimeDefaults, MimeType, RestoreScope, ZipAssociation, APP_ID,
};
use tempfile::TempDir;

/// The file manager every type starts with, as in the Python fixture.
const DOLPHIN: &str = "org.kde.dolphin.desktop";

/// One `xdg-mime` call the desktop received.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Query(MimeType),
    Set(String, MimeType),
}

/// A desktop whose default handlers live in memory and which records
/// every call, like the Python fixture's `mapping` and `commands`.
#[derive(Debug, Clone)]
struct RecordedDesktop {
    state: Arc<Mutex<DesktopState>>,
}

#[derive(Debug)]
struct DesktopState {
    handlers: BTreeMap<MimeType, String>,
    calls: Vec<Call>,
    /// When false the desktop acknowledges changes but keeps its handlers.
    applies_changes: bool,
}

impl RecordedDesktop {
    /// Every type opens in Dolphin.
    fn with_dolphin() -> Self {
        let handlers = MimeType::ALL
            .into_iter()
            .map(|mime_type| (mime_type, DOLPHIN.to_owned()))
            .collect();
        let state = DesktopState {
            handlers,
            calls: Vec::new(),
            applies_changes: true,
        };
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    fn state(&self) -> MutexGuard<'_, DesktopState> {
        self.state
            .lock()
            .expect("no test thread panicked while holding the state")
    }

    fn handler(&self, mime_type: MimeType) -> String {
        self.state().handlers[&mime_type].clone()
    }

    /// Another application or the user sets a handler directly.
    fn choose(&self, mime_type: MimeType, handler: &str) {
        self.state().handlers.insert(mime_type, handler.to_owned());
    }

    fn calls(&self) -> Vec<Call> {
        self.state().calls.clone()
    }
}

impl MimeDefaults for RecordedDesktop {
    fn default_handler(&self, mime_type: MimeType) -> Result<String, DefaultAppsError> {
        let mut state = self.state();
        state.calls.push(Call::Query(mime_type));
        Ok(state.handlers[&mime_type].clone())
    }

    fn set_default_handler(&self, handler: &DesktopId, mime_type: MimeType) -> Result<(), DefaultAppsError> {
        let mut state = self.state();
        state
            .calls
            .push(Call::Set(handler.as_str().to_owned(), mime_type));
        if state.applies_changes {
            state.handlers.insert(mime_type, handler.as_str().to_owned());
        }
        Ok(())
    }
}

/// The Python fixture: a desktop where Dolphin handles everything and an
/// empty settings folder for the record.
struct Fixture {
    desktop: RecordedDesktop,
    defaults: DefaultApps<RecordedDesktop>,
    _settings: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let settings = tempfile::tempdir().expect("temporary settings folder");
        let desktop = RecordedDesktop::with_dolphin();
        let defaults = DefaultApps::with_mime_defaults(settings.path(), desktop.clone());
        Self {
            desktop,
            defaults,
            _settings: settings,
        }
    }

    fn handlers_of(&self, mime_types: &[MimeType]) -> Vec<String> {
        mime_types
            .iter()
            .map(|mime_type| self.desktop.handler(*mime_type))
            .collect()
    }
}

/// Ported from `desktop/tests/test_rc3.py::DefaultsTests::test_folders_and_zip_report_independently`
/// parity: INT-008, INT-010
#[test]
fn folder_and_zip_defaults_are_reported_independently() {
    let fixture = Fixture::new();

    fixture
        .defaults
        .make_default(ZipAssociation::Unchanged)
        .expect("make default");
    let status = fixture.defaults.status().expect("status");

    assert!(status.is_default());
    assert!(status.is_default_for_folder_types());
    assert!(!status.is_zip_default());
    assert_eq!(status.handler(MimeType::Zip), DOLPHIN);
}

/// Ported from `desktop/tests/test_rc3.py::DefaultsTests::test_existing_make_default_does_not_hijack_zip`
/// parity: INT-008, INT-009
#[test]
fn making_the_folder_default_leaves_zip_files_alone() {
    let fixture = Fixture::new();

    fixture
        .defaults
        .make_default(ZipAssociation::Unchanged)
        .expect("make default");

    assert_eq!(fixture.handlers_of(&MimeType::ZIP_TYPES), [DOLPHIN; 3]);
}

/// Ported from `desktop/tests/test_rc3.py::DefaultsTests::test_explicit_zip_option_changes_all_zip_aliases`
/// parity: INT-009, INT-012
#[test]
fn including_zip_takes_over_every_zip_alias() {
    let fixture = Fixture::new();

    let status = fixture
        .defaults
        .make_default(ZipAssociation::Included)
        .expect("make default");

    assert!(status.is_zip_default());
    assert_eq!(fixture.handlers_of(&MimeType::ALL), [APP_ID; 5]);
}

/// Ported from `desktop/tests/test_rc3.py::DefaultsTests::test_zip_only_keeps_folder_default`
/// parity: INT-012
#[test]
fn the_zip_default_leaves_folder_types_with_their_handler() {
    let fixture = Fixture::new();

    fixture.defaults.make_zip_default().expect("zip default");

    assert_eq!(fixture.handlers_of(&MimeType::FOLDER_TYPES), [DOLPHIN; 2]);
    assert!(fixture.defaults.status().expect("status").is_zip_default());
}

/// Ported from `desktop/tests/test_rc3.py::DefaultsTests::test_zip_restore_does_not_restore_folders`
/// parity: INT-011, INT-012
#[test]
fn restoring_zip_keeps_openxplorer_for_folders() {
    let fixture = Fixture::new();
    fixture
        .defaults
        .make_default(ZipAssociation::Included)
        .expect("make default");

    fixture
        .defaults
        .restore(RestoreScope::ZipOnly)
        .expect("restore ZIP");

    let status = fixture.defaults.status().expect("status");
    assert!(status.is_default_for_folder_types());
    assert!(!status.is_zip_default());
}

/// Ported from `desktop/tests/test_rc3.py::DefaultsTests::test_restore_does_not_overwrite_new_user_choice`
/// parity: INT-011
#[test]
fn restoring_keeps_a_handler_the_user_chose_later() {
    let fixture = Fixture::new();
    fixture.defaults.make_zip_default().expect("zip default");
    fixture
        .desktop
        .choose(MimeType::Zip, "org.gnome.FileRoller.desktop");

    fixture
        .defaults
        .restore(RestoreScope::ZipOnly)
        .expect("restore ZIP");

    assert_eq!(
        fixture.desktop.handler(MimeType::Zip),
        "org.gnome.FileRoller.desktop"
    );
}

/// Ported from `desktop/tests/test_rc3.py::DefaultsTests::test_reapply_preserves_original_backup`
/// parity: INT-008
#[test]
fn reapplying_keeps_the_original_handler_on_record() {
    let fixture = Fixture::new();
    fixture.defaults.make_zip_default().expect("first zip default");
    fixture.defaults.make_zip_default().expect("second zip default");

    fixture
        .defaults
        .restore(RestoreScope::ZipOnly)
        .expect("restore ZIP");

    assert_eq!(fixture.desktop.handler(MimeType::Zip), DOLPHIN);
}

/// Ported from `desktop/tests/test_rc3.py::DefaultsTests::test_unconfirmed_install_is_read_only`
/// parity: INT-010
#[test]
fn reading_the_status_only_queries() {
    let fixture = Fixture::new();

    fixture.defaults.status().expect("status");

    let calls = fixture.desktop.calls();
    assert!(!calls.is_empty());
    assert!(
        calls.iter().all(|call| matches!(call, Call::Query(_))),
        "{calls:?}"
    );
    assert!(!fixture.defaults.record_path().exists());
}

/// Ported from `desktop/tests/test_rc3.py::DefaultsTests::test_restore_all`
/// parity: INT-011
#[test]
fn restoring_everything_puts_every_recorded_handler_back() {
    let fixture = Fixture::new();
    fixture
        .defaults
        .make_default(ZipAssociation::Included)
        .expect("make default");

    fixture
        .defaults
        .restore(RestoreScope::Everything)
        .expect("restore");

    assert_eq!(fixture.handlers_of(&MimeType::ALL), [DOLPHIN; 5]);
}

/// Ported from `desktop/tests/test_rc3.py::DefaultsTests::test_bad_previous_handler_rejected`
/// parity: INT-008, SAFE-020
#[test]
fn a_handler_that_cannot_be_recorded_is_refused_before_any_change() {
    let fixture = Fixture::new();
    fixture.desktop.choose(MimeType::Zip, "evil;command.desktop");

    let refused = fixture.defaults.make_zip_default();

    assert!(
        matches!(refused, Err(DefaultAppsError::UnrecordableHandler)),
        "{refused:?}"
    );
    assert_eq!(
        refused.expect_err("refused").to_string(),
        "The current desktop handler cannot be safely recorded."
    );
    assert_eq!(fixture.desktop.handler(MimeType::Zip), "evil;command.desktop");
    assert!(!fixture.defaults.record_path().exists());
}

/// Ported from `desktop/tests/test_rc3.py::DefaultsTests::test_missing_handler_not_overpromised`
/// parity: INT-010
#[test]
fn a_missing_handler_is_reported_empty_and_not_as_openxplorer() {
    let fixture = Fixture::new();
    fixture.desktop.choose(MimeType::Zip, "");

    let status = fixture.defaults.status().expect("status");

    assert_eq!(status.handler(MimeType::Zip), "");
    assert!(!status.is_zip_default());
}

/// Ported from `desktop/tests/test_rc3.py::DefaultsTests::test_backup_is_private`
/// parity: INT-008
#[test]
fn the_record_of_previous_handlers_is_private() {
    let fixture = Fixture::new();

    fixture.defaults.make_zip_default().expect("zip default");

    let metadata = fs::metadata(fixture.defaults.record_path()).expect("record");
    assert_eq!(metadata.mode() & 0o777, 0o600);
}

/// Ported from `desktop/tests/test_v08.py::RebrandTests::test_desktop_id_retained`
/// parity: INT-029
#[test]
fn the_desktop_id_is_the_legacy_one() {
    assert_eq!(APP_ID, "io.winspace.Development.desktop");
}

/// parity: INT-011
#[test]
fn restoring_without_a_record_asks_for_the_desktop_settings() {
    let fixture = Fixture::new();

    let refused = fixture.defaults.restore(RestoreScope::Everything);

    assert!(
        matches!(refused, Err(DefaultAppsError::NoPreviousHandler)),
        "{refused:?}"
    );
    assert_eq!(
        refused.expect_err("refused").to_string(),
        "No previous handler was recorded. Choose one in your desktop settings."
    );
    assert!(fixture
        .desktop
        .calls()
        .iter()
        .all(|call| matches!(call, Call::Query(_))));
}

/// parity: INT-008
#[test]
fn a_change_the_desktop_does_not_confirm_is_reported() {
    let fixture = Fixture::new();
    fixture.desktop.state().applies_changes = false;

    let unconfirmed = fixture.defaults.make_default(ZipAssociation::Unchanged);

    assert!(
        matches!(unconfirmed, Err(DefaultAppsError::NotConfirmed)),
        "{unconfirmed:?}"
    );
    assert!(
        fixture.defaults.record_path().exists(),
        "recorded before changing"
    );
}

/// parity: INT-010
#[test]
fn the_status_can_be_read_on_a_worker_thread() {
    let fixture = Fixture::new();
    fixture.defaults.make_zip_default().expect("zip default");

    let reading = fixture.defaults.run_in_background(DefaultApps::status);
    let status = glib::MainContext::new().block_on(reading).expect("status");

    assert!(status.is_zip_default());
    assert!(status.can_restore_zip);
    assert!(status.can_restore);
}
