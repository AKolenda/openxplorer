// SPDX-License-Identifier: AGPL-3.0-only
//! The Location tab of a standard folder in a real window: checking a new
//! location, applying it with consent, and moving the old folder's files
//! through the transfer engine. Every folder, `user-dirs.dirs` included,
//! lies in a temporary directory; `xdg-user-dirs-update` is simulated.

use std::fs;
use std::path::{Path, PathBuf};

use gtk::prelude::*;
use ox_core::folder_locations::{FolderRelocation, RelocationError, UserDirsUpdater};
use ox_core::location::file_uri;
use ox_core::places::{FolderLocations, KnownFolder};

use super::file_ops_support::open_dialog;
use super::item_dialogs::press;
use crate::integration::BraveDialog;
use crate::properties::LocationPanel;
use crate::test_support::harness::{descendants, wait_until, TestWindow};

/// Writes the requested line into `user-dirs.dirs`, as the real tool does.
#[derive(Debug)]
struct WritingUpdater {
    user_dirs_file: PathBuf,
}

impl UserDirsUpdater for WritingUpdater {
    fn set_folder(&self, folder: KnownFolder, path: &Path) -> Result<(), RelocationError> {
        let line = format!("XDG_{}_DIR=\"{}\"\n", folder.xdg_key(), path.display());
        fs::write(&self.user_dirs_file, line).map_err(|_| RelocationError::UpdaterFailed)
    }
}

/// A home whose Documents folder holds a file, a folder and a file whose
/// name the new location already has, and an empty-but-one new location.
struct Folders {
    _root: tempfile::TempDir,
    home: PathBuf,
    config: PathBuf,
    documents: PathBuf,
    destination: PathBuf,
}

impl Folders {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("the test home has room for folders");
        let base = fs::canonicalize(root.path()).expect("the folder resolves");
        let home = base.join("home");
        let config = base.join("config");
        let documents = home.join("Documents");
        let destination = base.join("data").join("My documents");
        for folder in [&config, &documents.join("Taxes"), &destination] {
            fs::create_dir_all(folder).expect("the fixture folder is created");
        }
        fs::write(documents.join("plan.txt"), b"old plan").expect("a file");
        fs::write(documents.join("Taxes").join("2025.txt"), b"taxes").expect("a nested file");
        fs::write(documents.join("notes.txt"), b"old notes").expect("a conflicting file");
        fs::write(destination.join("notes.txt"), b"new notes").expect("the file already there");
        Self {
            _root: root,
            home,
            config,
            documents,
            destination,
        }
    }

    fn locations(&self) -> FolderLocations {
        FolderLocations::new(self.home.clone(), &self.config)
    }

    /// The relocation over these folders, which lie under the system's
    /// temporary folder and so accept no temporary roots.
    fn relocation(&self, state: &Path) -> FolderRelocation {
        let updater = WritingUpdater {
            user_dirs_file: self.config.join("user-dirs.dirs"),
        };
        FolderRelocation::new(self.locations(), state.to_owned())
            .with_updater(updater)
            .with_mount_reader(Box::new(|| Ok(Vec::new())))
            .with_temporary_roots(Vec::new())
    }
}

/// Opening Documents' Properties shows the Location tab; a checked and
/// consented new location is applied, then Windows 11's question moves the
/// files, and a name the new folder has already is asked about and, when
/// skipped, stays in the old folder: no file is lost or overwritten.
///
/// parity: PROP-017, PROP-031
#[gtk::test]
fn the_location_tab_moves_documents_and_offers_to_move_its_files() {
    let folders = Folders::new();
    let state = folders.home.join(".config-state");
    let test =
        TestWindow::open_with_standard_folders(&file_uri(&folders.home), folders.locations(), |context| {
            context.use_folder_relocation(folders.relocation(&state));
        });
    test.activate("properties-of", Some(&file_uri(&folders.documents)));
    let panel = location_panel(&test);
    assert!(
        !panel.sync_brave_box().is_visible(),
        "Brave's box is for Downloads"
    );
    let field = panel.location_field();
    wait_until("the current location", || {
        field.text() == folders.documents.to_string_lossy().as_ref()
    });

    field.set_text(&folders.destination.to_string_lossy());
    press(&panel, "Check location");
    let checked = format!("Local folder · {}", folders.destination.display());
    wait_until("the check", || panel.status_text() == checked);
    assert!(!panel.can_apply(), "Apply waits for consent");
    panel.set_consent(true);
    assert!(panel.can_apply());
    press(&panel, "Apply location");

    let question = open_dialog(&test);
    assert_eq!(question.title_text(), "Move files");
    assert_eq!(question.button_labels(), ["Don't move", "Move files"]);
    let user_dirs = fs::read_to_string(folders.config.join("user-dirs.dirs")).expect("the new setting");
    assert!(user_dirs.contains(&folders.destination.display().to_string()));
    question.press("Move files");
    let conflict = open_dialog(&test);
    assert_eq!(conflict.title_text(), "Items already exist");
    conflict.press("Skip duplicates");

    let report = open_dialog(&test);
    assert_eq!(
        report.title_text(),
        "Operation result",
        "the skipped name is reported"
    );
    report.press("OK");
    wait_until("the status", || {
        panel.status_text().contains("Some items stayed in")
    });
    let read = |path: PathBuf| fs::read_to_string(path).expect("the file is kept");
    assert_eq!(read(folders.destination.join("Taxes").join("2025.txt")), "taxes");
    assert_eq!(read(folders.destination.join("notes.txt")), "new notes");
    assert_eq!(read(folders.documents.join("notes.txt")), "old notes");
    assert!(!folders.documents.join("plan.txt").exists());
}

/// Brave's follow-up shows for Downloads only, starts unticked and, when
/// ticked, opens Brave's own dialog for the new folder after Apply.
///
/// parity: PROP-018
#[gtk::test]
fn a_ticked_brave_follow_up_opens_brave_s_dialog_after_moving_downloads() {
    let folders = Folders::new();
    let state = folders.home.join(".config-state");
    let test =
        TestWindow::open_with_standard_folders(&file_uri(&folders.home), folders.locations(), |context| {
            context.use_folder_relocation(folders.relocation(&state));
        });
    let downloads = folders.home.join("Downloads");
    fs::create_dir(&downloads).expect("the Downloads folder is created");
    test.activate("properties-of", Some(&file_uri(&downloads)));
    let panel = location_panel(&test);
    let sync_brave = panel.sync_brave_box();
    assert!(sync_brave.is_visible() && !sync_brave.is_active());

    sync_brave.set_active(true);
    let field = panel.location_field();
    wait_until("the current location", || !field.text().is_empty());
    field.set_text(&folders.destination.to_string_lossy());
    press(&panel, "Check location");
    wait_until("the check", || panel.status_text().starts_with("Local folder"));
    panel.set_consent(true);
    press(&panel, "Apply location");

    wait_until("Brave's dialog", || brave_dialog(&test).is_some());
    brave_dialog(&test).expect("it is open").close();
}

/// The Location tab of the Properties dialog `test` shows.
fn location_panel(test: &TestWindow) -> LocationPanel {
    let frame = test.wait_for_dialog("the Properties dialog");
    let panels = descendants::<LocationPanel>(&frame);
    panels
        .into_iter()
        .next()
        .expect("a standard folder has a Location tab")
}

/// Brave's download-folder dialog over `test`'s window, if one shows.
fn brave_dialog(test: &TestWindow) -> Option<BraveDialog> {
    let toplevels = gtk::Window::list_toplevels().into_iter();
    let mut dialogs = toplevels.filter_map(|window| window.downcast::<BraveDialog>().ok());
    dialogs.find(|dialog| dialog.transient_for().as_ref() == Some(test.window.upcast_ref()))
}
