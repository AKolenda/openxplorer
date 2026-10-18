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
use ox_core::integration::{MimeType, Sandbox, APP_ID, SUPER_E};

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
    root: tempfile::TempDir,
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
            root,
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

    /// The portal configuration Open and Save dialogs are enabled in: the
    /// user's, for the test's KDE desktop.
    fn portal_file(&self) -> std::path::PathBuf {
        self.root
            .path()
            .join("config/xdg-desktop-portal/kde-portals.conf")
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
    // Show in folder writes session files and the dialogs the portal's
    // configuration; this test leaves both alone.
    default_apps
        .switch_of(super::INCLUDE_SHOW_IN_FOLDER.title)
        .set_active(false);
    default_apps
        .switch_of(super::ALSO_FILE_DIALOGS.title)
        .set_active(false);

    default_apps.button("Make OpenXplorer default").emit_clicked();
    // The dialogs were left out, which the card names.
    wait_until("the page to say so", || {
        default_apps.card_title()
            == "OpenXplorer is your default file explorer, except for Open and Save dialogs"
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
    assert!(
        !default_apps.portal_file().exists(),
        "the dialogs were not chosen"
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
    // Make default would enable the dialogs, which changes what applies.
    default_apps
        .switch_of(super::ALSO_FILE_DIALOGS.title)
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

/// Folders alone do not make `OpenXplorer` the default for everything:
/// the card says when other apps' Open and Save dialogs, such as Brave's
/// uploads, still use the desktop's, and Make default with its dialogs
/// option, on by default, sends them to `OpenXplorer` too. Restore
/// previous gives them back with the handlers.
///
/// parity: INT-009, INT-032
#[gtk::test]
fn make_default_also_takes_open_and_save_dialogs() {
    let default_apps = DefaultAppsTest::open();
    let dialogs = default_apps.switch_of(super::ALSO_FILE_DIALOGS.title);
    assert!(dialogs.is_active(), "on by default");
    assert!(dialogs.is_sensitive(), "the opt-in works outside Flatpak");
    default_apps
        .switch_of(super::INCLUDE_SHOW_IN_FOLDER.title)
        .set_active(false);
    // Folders made default without the dialogs, as by an earlier version.
    dialogs.set_active(false);
    default_apps.button("Make OpenXplorer default").emit_clicked();
    wait_until("the card to name the dialogs", || {
        default_apps.card_title()
            == "OpenXplorer is your default file explorer, except for Open and Save dialogs"
    });
    assert_eq!(default_apps.handler(MimeType::Directory), APP_ID);

    dialogs.set_active(true);
    default_apps.button("Make OpenXplorer default").emit_clicked();
    wait_until("the card to say so", || {
        default_apps.card_title() == "OpenXplorer is your default file explorer"
    });

    let written = std::fs::read_to_string(default_apps.portal_file()).expect("the portal configuration");
    assert!(
        written.contains(&format!(
            "org.freedesktop.impl.portal.FileChooser={}",
            crate::config::APP_ID
        )),
        "{written}"
    );
    assert_eq!(
        default_apps.test.window.shown_message_text(),
        "Requested associations updated. For other apps' Open and Save dialogs, click Apply now, or log \
         out and back in."
    );
    assert!(default_apps
        .row(super::FILE_DIALOGS.title)
        .shown_description()
        .starts_with("Open and Save dialogs: OpenXplorer."));
    wait_until("Restore previous to be offered", || {
        default_apps.button("Restore previous").is_sensitive()
    });

    default_apps.button("Restore previous").emit_clicked();
    wait_until("Dolphin to open folders again", || {
        default_apps.handler(MimeType::Directory) == DOLPHIN
    });
    default_apps.wait_for_status("the restored status");
    assert!(
        !default_apps.portal_file().exists(),
        "the portal configuration the app created is removed"
    );
    assert_eq!(
        default_apps.row(super::FILE_DIALOGS.title).shown_description(),
        "Open and Save dialogs: the desktop's."
    );
    assert_eq!(
        default_apps.test.window.shown_message_text(),
        "Previous recorded handlers restored. For other apps' Open and Save dialogs to return to the \
         desktop's, click Apply now, or log out and back in."
    );
}

/// Make default finishes dialogs enabled before KDE's own apps were
/// covered: it adds their missing login script, as Enable does.
///
/// parity: INT-032
#[gtk::test]
fn make_default_adds_the_kde_login_script_to_dialogs_enabled_before() {
    let default_apps = DefaultAppsTest::open();
    default_apps
        .switch_of(super::INCLUDE_SHOW_IN_FOLDER.title)
        .set_active(false);
    default_apps.button("Make OpenXplorer default").emit_clicked();
    wait_until("the card to say so", || {
        default_apps.card_title() == "OpenXplorer is your default file explorer"
    });
    let script = default_apps
        .root
        .path()
        .join("config/plasma-workspace/env/openxplorer-file-dialogs.sh");
    assert!(script.is_file(), "the test's desktop is KDE Plasma");
    // As enabled by a version without the script.
    std::fs::remove_file(&script).expect("the login script");
    default_apps.wait_for_status("the status after the change");

    default_apps.button("Make OpenXplorer default").emit_clicked();
    wait_until("the login script again", || script.is_file());
    default_apps.wait_for_status("the status after the change");
}

/// Restore previous undoes a recorded Enable of the dialogs even when the
/// portal's configuration cannot be read, and says it failed instead of
/// reporting everything restored.
///
/// parity: INT-032
#[gtk::test]
fn restore_previous_reports_dialogs_it_could_not_give_back() {
    let default_apps = DefaultAppsTest::open();
    default_apps
        .switch_of(super::INCLUDE_SHOW_IN_FOLDER.title)
        .set_active(false);
    default_apps.button("Make OpenXplorer default").emit_clicked();
    wait_until("the card to say so", || {
        default_apps.card_title() == "OpenXplorer is your default file explorer"
    });
    // A folder where the configuration was cannot be read as a file, so
    // the dialogs no longer read as enabled; the record remains.
    let file = default_apps.portal_file();
    std::fs::remove_file(&file).expect("the portal configuration");
    std::fs::create_dir(&file).expect("a folder in its place");
    default_apps.wait_for_status("the status after the change");

    default_apps.button("Restore previous").emit_clicked();
    wait_until("Dolphin to open folders again", || {
        default_apps.handler(MimeType::Directory) == DOLPHIN
    });
    default_apps.wait_for_status("the restored status");

    let message = default_apps.test.window.shown_message_text();
    assert!(
        message.ends_with(&file.display().to_string()),
        "the failure names the configuration: {message}"
    );
    assert!(
        default_apps
            .root
            .path()
            .join("winspace/file-dialogs.json")
            .is_file(),
        "the record stays for a later Restore"
    );
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

/// The Super+E switch takes Super+E from Dolphin for `OpenXplorer`'s New
/// window action, as Win+E opens a new File Explorer window, and turning
/// it off gives it back; the line
/// under it says to log out and back in, which KDE Plasma needs.
///
/// parity: INT-033
#[gtk::test]
fn the_super_e_switch_takes_super_e_from_dolphin_and_gives_it_back() {
    let default_apps = DefaultAppsTest::open();
    let row = default_apps.row(super::SUPER_E.title);
    let switch = row
        .controls()
        .into_iter()
        .find_map(|control| control.downcast::<gtk::Switch>().ok())
        .expect("Super+E is an on/off switch");
    let after_login = "Log out and back in after changing this for Super+E to follow.";
    wait_until("the shortcut status", || switch.is_sensitive());
    assert_eq!(row.shown_description(), after_login);
    assert!(!switch.is_active(), "Dolphin has Super+E");
    let table = default_apps.page.context().desktop_integration().shortcut_table();
    let keys_of = |component: &str| {
        let table = table.lock().unwrap_or_else(PoisonError::into_inner);
        table
            .iter()
            .find(|(action, _)| action.component == component)
            .map(|(_, keys)| keys.clone())
            .unwrap_or_default()
    };
    let ours = format!("{}.desktop", crate::config::APP_ID);

    switch.set_active(true);

    wait_until("Super+E to be OpenXplorer's", || keys_of(&ours) == [SUPER_E]);
    let owner = table
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|(_, keys)| keys.contains(&SUPER_E))
        .map(|(action, _)| action.action.clone());
    assert_eq!(owner.as_deref(), Some("NewWindow"), "Super+E opens a new window");
    assert!(keys_of(DOLPHIN).is_empty());
    wait_until("the status after the change", || switch.is_sensitive());
    assert!(switch.is_active(), "the switch stays on");
    assert_eq!(row.shown_description(), after_login);

    switch.set_active(false);

    wait_until("Super+E to be Dolphin's again", || keys_of(DOLPHIN) == [SUPER_E]);
    assert!(keys_of(&ours).is_empty());
    wait_until("the status after the change", || switch.is_sensitive());
    assert!(!switch.is_active());
}
