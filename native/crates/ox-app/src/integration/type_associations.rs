// SPDX-License-Identifier: AGPL-3.0-only
//! The applications associated with a file type, and the user's changes to
//! them (OPEN-026), as Dolphin's file-type options in Properties offer.
//!
//! Reads and writes go through GIO, which keeps the user's choices in
//! `~/.config/mimeapps.list` only: setting a default, adding an
//! application to a type, removing one the user added, and resetting the
//! type to the system's associations. The types `OpenXplorer` itself can
//! be the default for (folders, SMB links and ZIP, [`MimeType`]) are
//! changed only in Settings > Default apps, which records what it replaced
//! (INT-008 to INT-012), so they are refused here.

use std::path::PathBuf;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::integration::{MimeType, APP_ID};

/// The `mimeapps.list` group of associations the user added.
const ADDED_GROUP: &str = "Added Associations";

/// Why a type cannot be changed here.
pub(crate) const PROTECTED_TYPE: &str =
    "Folder, SMB and ZIP associations are changed in Settings > Default apps.";

/// One application associated with a file type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TypeApplication {
    /// Its desktop ID.
    pub(crate) id: String,
    /// Its name.
    pub(crate) name: String,
    /// It is the type's default.
    pub(crate) is_default: bool,
    /// The user added it to the type, so it can be removed.
    pub(crate) is_added: bool,
}

/// True for a type whose associations only Settings > Default apps
/// changes.
pub(crate) fn is_protected(content_type: &str) -> bool {
    MimeType::ALL.iter().any(|protected| protected.as_str() == content_type)
}

/// The applications associated with `content_type`, the default first,
/// then by name; never `OpenXplorer` or a hidden launcher.
pub(crate) fn type_applications(content_type: &str) -> Vec<TypeApplication> {
    let default_id = gio::AppInfo::default_for_type(content_type, false).and_then(|app| app.id());
    let added = added_ids(content_type);
    let mut applications: Vec<TypeApplication> = gio::AppInfo::all_for_type(content_type)
        .into_iter()
        .filter(|app| app.should_show())
        .filter_map(|app| {
            let id = app.id()?.to_string();
            (id != APP_ID).then(|| TypeApplication {
                is_default: default_id.as_deref() == Some(id.as_str()),
                is_added: added.contains(&id),
                name: app.display_name().to_string(),
                id,
            })
        })
        .collect();
    applications.sort_by_key(|application| (!application.is_default, application.name.to_lowercase()));
    applications
}

/// Every visible application not yet associated with `content_type`, by
/// name: those that can be added to it.
pub(crate) fn other_applications(content_type: &str) -> Vec<(String, String)> {
    let associated: Vec<String> = type_applications(content_type)
        .into_iter()
        .map(|application| application.id)
        .collect();
    let mut others: Vec<(String, String)> = gio::AppInfo::all()
        .into_iter()
        .filter(|app| app.should_show() && (app.supports_files() || app.supports_uris()))
        .filter_map(|app| Some((app.id()?.to_string(), app.display_name().to_string())))
        .filter(|(id, _)| id != APP_ID && !associated.contains(id))
        .collect();
    others.sort_by_key(|(_, name)| name.to_lowercase());
    others.dedup_by(|left, right| left.0 == right.0);
    others
}

/// A change to a type's associations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TypeChange {
    /// Make the application the default.
    SetDefault(String),
    /// Associate the application with the type.
    Add(String),
    /// Remove an association the user added.
    Remove(String),
    /// Forget every change the user made to the type.
    Reset,
}

/// Applies `change` to `content_type` in the user's `mimeapps.list`.
///
/// # Errors
///
/// The message to show: the type is protected, the application is not
/// installed, or GIO could not write the change.
pub(crate) fn change_type(content_type: &str, change: &TypeChange) -> Result<(), String> {
    if is_protected(content_type) {
        return Err(PROTECTED_TYPE.to_owned());
    }
    let changed = match change {
        TypeChange::SetDefault(id) => application(id)?.set_as_default_for_type(content_type),
        TypeChange::Add(id) => application(id)?.add_supports_type(content_type),
        TypeChange::Remove(id) => application(id)?.remove_supports_type(content_type),
        TypeChange::Reset => {
            gio::AppInfo::reset_type_associations(content_type);
            Ok(())
        }
    };
    changed.map_err(|error| error.to_string())
}

/// The installed application `id`.
fn application(id: &str) -> Result<gio::AppInfo, String> {
    gio::AppInfo::all()
        .into_iter()
        .find(|app| app.id().is_some_and(|app_id| app_id == id))
        .ok_or_else(|| "That application is not installed.".to_owned())
}

/// The applications the user added to `content_type`, from the user's
/// `mimeapps.list`.
fn added_ids(content_type: &str) -> Vec<String> {
    let file = glib::KeyFile::new();
    let path: PathBuf = glib::user_config_dir().join("mimeapps.list");
    if file.load_from_file(path, glib::KeyFileFlags::NONE).is_err() {
        return Vec::new();
    }
    file.string_list(ADDED_GROUP, content_type)
        .map(|ids| ids.iter().map(ToString::to_string).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::test_support::harness::wait_until;

    const VIEWER: &str = "org.openxplorer.TestViewer.desktop";
    const TEST_TYPE: &str = "application/x-openxplorer-test";

    /// An application added to a type is listed as added, can be removed,
    /// made the default, and goes again when the type is reset; the
    /// app's own types are refused.
    ///
    /// parity: OPEN-026
    #[gtk::test]
    fn an_added_application_is_removed_made_default_and_reset() {
        let data = glib::user_data_dir();
        assert!(data.starts_with(std::env::temp_dir()), "a private data folder");
        let folder = data.join("applications");
        fs::create_dir_all(&folder).expect("the data folder is writable");
        let entry = "[Desktop Entry]\nType=Application\nName=Test viewer\nExec=true %F\n";
        fs::write(folder.join(VIEWER), entry).expect("the data folder is writable");
        wait_until("GIO to list the viewer", || {
            other_applications(TEST_TYPE).iter().any(|(id, _)| id == VIEWER)
        });
        let listed = || -> Vec<(String, bool, bool)> {
            type_applications(TEST_TYPE)
                .into_iter()
                .map(|application| (application.id, application.is_default, application.is_added))
                .collect()
        };

        change_type(TEST_TYPE, &TypeChange::Add(VIEWER.into())).expect("added");
        // The only application of a type is its default too.
        assert_eq!(listed(), [(VIEWER.to_owned(), true, true)]);
        change_type(TEST_TYPE, &TypeChange::Remove(VIEWER.into())).expect("removed");
        assert_eq!(listed(), []);
        change_type(TEST_TYPE, &TypeChange::SetDefault(VIEWER.into())).expect("made default");
        assert!(listed()[0].1, "the default");
        change_type(TEST_TYPE, &TypeChange::Reset).expect("reset");
        assert_eq!(listed(), []);
        assert_eq!(
            change_type("inode/directory", &TypeChange::Reset),
            Err(PROTECTED_TYPE.to_owned())
        );
        fs::remove_file(folder.join(VIEWER)).expect("the test entry is removed");
    }
}
