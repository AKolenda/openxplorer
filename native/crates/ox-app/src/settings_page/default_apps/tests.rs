// SPDX-License-Identifier: AGPL-3.0-only
//! The Default apps page against a table of default applications in
//! memory, so no test changes the associations of the session it runs in.
//!
//! Ports the page behaviour of `renderDefaultStatus`, `changeDefault` and
//! `changeZipDefault` in `v2.0.0:desktop/ui/app.js`, and `DefaultsTests` of
//! `v2.0.0:desktop/tests/test_rc3.py` as seen from the page.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use gtk::prelude::*;
use ox_core::integration::{MimeType, Sandbox, APP_ID};

use super::super::pages::{Category, SettingsView};
use super::super::row::SettingRow;
use super::super::status_card::StatusCard;
use super::super::SettingsPage;
use crate::integration::{DesktopIntegration, IntegrationFolders, MimeBackend};
use crate::test_support::harness::{capture, descendants, wait_until, Fixture, TestWindow};

/// The file manager every type starts with, as in the Python fixture.
const DOLPHIN: &str = "org.kde.dolphin.desktop";

/// A window on Default apps whose defaults live in memory.
struct DefaultAppsTest {
    test: TestWindow,
    page: SettingsPage,
    handlers: Arc<Mutex<BTreeMap<MimeType, String>>>,
    _fixture: Fixture,
    /// Holds the integration's records and session files.
    _root: tempfile::TempDir,
}

impl DefaultAppsTest {
    /// Every type opens in Dolphin; Settings shows Default apps.
    fn open() -> Self {
        let fixture = Fixture::standard();
        let (backend, handlers) = MimeBackend::in_memory(DOLPHIN);
        let root = tempfile::tempdir().expect("a temporary folder");
        let folders = IntegrationFolders::inside(root.path());
        let test = TestWindow::open_prepared(&fixture.uri(), |context| {
            let integration = DesktopIntegration::with_mime_backend(&folders, Sandbox::Host, backend);
            context.use_desktop_integration(integration);
        });
        test.window
            .open_settings(Some(SettingsView::Category(Category::DefaultApps)));
        let page = descendants::<SettingsPage>(&test.window)
            .into_iter()
            .next()
            .expect("the window has a Settings page");
        let default_apps = Self {
            test,
            page,
            handlers,
            _fixture: fixture,
            _root: root,
        };
        default_apps.wait_for_status("the first status");
        default_apps
    }

    fn handler(&self, mime_type: MimeType) -> String {
        let handlers = self.handlers.lock().unwrap_or_else(PoisonError::into_inner);
        handlers.get(&mime_type).cloned().unwrap_or_default()
    }

    /// The pages this page's controls are on: Default apps, and ZIP &
    /// archives, which shows the ZIP route.
    fn sections(&self) -> [super::super::section::SettingsSection; 2] {
        [Category::DefaultApps, Category::Archives].map(|category| self.page.category_section(category))
    }

    fn row(&self, title: &str) -> SettingRow {
        let rows = self.sections().into_iter().flat_map(|section| section.rows());
        rows.into_iter()
            .find(|row| row.text().title == title)
            .unwrap_or_else(|| panic!("Default apps has a row titled {title:?}"))
    }

    /// The button labelled `label` on Default apps or ZIP & archives.
    fn button(&self, label: &str) -> gtk::Button {
        let buttons = self
            .sections()
            .into_iter()
            .flat_map(|section| descendants::<gtk::Button>(&section));
        buttons
            .into_iter()
            .find(|button| button.label().as_deref() == Some(label))
            .unwrap_or_else(|| panic!("Default apps has a {label:?} button"))
    }

    /// The value a route row shows.
    fn route(&self, title: &str) -> String {
        let control = self.row(title).controls().into_iter().next();
        let value = control
            .and_downcast::<gtk::Label>()
            .expect("the route shows its app");
        value.text().to_string()
    }

    fn card_title(&self) -> String {
        let section = self.page.category_section(Category::DefaultApps);
        let card = descendants::<StatusCard>(&section).into_iter().next();
        card.expect("Default apps has a status card").title()
    }

    fn switch_of(&self, title: &str) -> gtk::Switch {
        let control = self.row(title).controls().into_iter().next();
        control
            .and_downcast::<gtk::Switch>()
            .expect("the option is a switch")
    }

    fn wait_for_status(&self, what: &str) {
        wait_until(what, || {
            self.button("Make OpenXplorer default").is_sensitive() && self.route("Folders") != super::CHECKING
        });
    }
}

/// Make `OpenXplorer` default takes over folders and SMB links only, as
/// the card then says, and Restore previous puts Dolphin back; each change
/// says what it did.
///
/// parity: INT-008, INT-009, INT-011, INT-030
#[gtk::test]
fn making_openxplorer_the_default_can_be_undone() {
    let default_apps = DefaultAppsTest::open();
    assert_eq!(
        default_apps.card_title(),
        "OpenXplorer isn't your default file explorer yet"
    );
    assert!(
        !default_apps.button("Restore previous").is_sensitive(),
        "nothing is recorded yet"
    );
    assert!(
        default_apps
            .switch_of(super::INCLUDE_SHOW_IN_FOLDER.title)
            .is_active(),
        "on by default"
    );
    assert!(
        !default_apps.switch_of("Also open ZIP files").is_active(),
        "off by default"
    );
    // Show in folder writes session files; this test leaves it alone.
    default_apps
        .switch_of(super::INCLUDE_SHOW_IN_FOLDER.title)
        .set_active(false);

    default_apps.button("Make OpenXplorer default").emit_clicked();
    wait_until("the page to say so", || {
        default_apps.card_title() == "OpenXplorer is your default file explorer"
    });

    assert_eq!(default_apps.handler(MimeType::Directory), APP_ID);
    assert_eq!(default_apps.handler(MimeType::SmbLink), APP_ID);
    assert_eq!(
        default_apps.handler(MimeType::Zip),
        DOLPHIN,
        "ZIP files were not chosen"
    );
    assert_eq!(default_apps.route("Folders"), "OpenXplorer");
    assert_eq!(
        default_apps.test.window.shown_message_text(),
        "Requested associations updated. Review each status below."
    );
    capture(&default_apps.test.window, "native-default-apps-made-default.png");
    wait_until("Restore previous to be offered", || {
        default_apps.button("Restore previous").is_sensitive()
    });

    default_apps.button("Restore previous").emit_clicked();
    wait_until("Dolphin to open folders again", || {
        default_apps.handler(MimeType::Directory) == DOLPHIN
    });
    default_apps.wait_for_status("the restored status");
    assert_eq!(default_apps.handler(MimeType::SmbLink), DOLPHIN);
    assert_eq!(
        default_apps.test.window.shown_message_text(),
        "Previous recorded handlers restored."
    );
}

/// While a change runs, the Open and Save dialogs' Enable, Apply now and
/// Restore are off too, so Enable and Restore never overlap; the status
/// read after the change turns on what applies.
///
/// parity: INT-032
#[gtk::test]
fn the_dialog_buttons_wait_while_a_change_runs() {
    let default_apps = DefaultAppsTest::open();
    let mut buttons: Vec<gtk::Button> = default_apps
        .row(super::FILE_DIALOGS.title)
        .controls()
        .into_iter()
        .filter_map(|control| control.downcast::<gtk::Button>().ok())
        .collect();
    assert_eq!(buttons.len(), 2, "Apply now and Enable");
    buttons.push(default_apps.button("Restore Open and Save dialogs"));
    let before: Vec<bool> = buttons.iter().map(gtk::Button::is_sensitive).collect();
    default_apps
        .switch_of(super::INCLUDE_SHOW_IN_FOLDER.title)
        .set_active(false);

    default_apps.button("Make OpenXplorer default").emit_clicked();

    for button in &buttons {
        assert!(
            !button.is_sensitive(),
            "{:?} waits for the change",
            button.label()
        );
    }
    default_apps.wait_for_status("the status after the change");
    let after: Vec<bool> = buttons.iter().map(gtk::Button::is_sensitive).collect();
    assert_eq!(after, before, "the status turns them back on as before");
}

/// Use `OpenXplorer` for ZIPs takes every ZIP type and leaves folders
/// alone; Restore ZIP handler gives them back.
///
/// parity: INT-012
#[gtk::test]
fn zip_files_are_taken_over_and_given_back_on_their_own() {
    let default_apps = DefaultAppsTest::open();
    assert!(!default_apps.button("Restore ZIP handler").is_sensitive());

    default_apps.button("Use OpenXplorer for ZIPs").emit_clicked();
    wait_until("ZIP files to open in OpenXplorer", || {
        !default_apps.button("Use OpenXplorer for ZIPs").is_sensitive()
    });

    for zip_type in MimeType::ZIP_TYPES {
        assert_eq!(default_apps.handler(zip_type), APP_ID, "{zip_type:?}");
    }
    assert_eq!(default_apps.handler(MimeType::Directory), DOLPHIN);
    assert_eq!(
        default_apps.row(super::ZIP_FILES.title).shown_description(),
        "ZIP opening: OpenXplorer. This is separate from folder defaults."
    );
    assert_eq!(
        default_apps.test.window.shown_message_text(),
        "ZIP files now open in OpenXplorer."
    );
    wait_until("Restore ZIP handler to be offered", || {
        default_apps.button("Restore ZIP handler").is_sensitive()
    });

    default_apps.button("Restore ZIP handler").emit_clicked();
    wait_until("Dolphin to open ZIP files again", || {
        default_apps.handler(MimeType::Zip) == DOLPHIN
    });
    wait_until("the toast", || {
        default_apps.test.window.shown_message_text() == "Previous ZIP handlers restored."
    });
}

/// Show in folder says it is not enabled until the user enables it; the
/// test is refused while `OpenXplorer` does not answer `FileManager1`.
///
/// parity: INT-016
#[gtk::test]
fn show_in_folder_starts_disabled_and_its_test_needs_the_service() {
    let default_apps = DefaultAppsTest::open();
    assert_eq!(
        default_apps
            .row(super::BRAVE_AND_OTHER_APPS.title)
            .shown_description(),
        "Show in folder: not enabled. Folder associations alone do not control every browser route."
    );

    default_apps.button("Test").emit_clicked();

    wait_until("the refusal", || {
        default_apps.test.window.shown_message_text()
            == "OpenXplorer does not own Show in folder yet. Close other file managers, or log out and \
                back in after enabling."
    });
}
