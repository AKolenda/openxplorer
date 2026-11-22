// SPDX-License-Identifier: AGPL-3.0-only
//! Brings the settings of the release's Flatpak bundle to the Flathub build.
//!
//! The bundle is `io.winspace.Development` and keeps its settings in
//! `~/.var/app/io.winspace.Development/config/winspace`; the Flathub build is
//! `app.openxplorer.OpenXplorer`, whose sandbox has its own
//! `~/.var/app/app.openxplorer.OpenXplorer`. On the Flathub build's first
//! launch, before anything reads its settings, the bundle's settings folder
//! is copied over once, so a user who moves to Flathub keeps their pinned
//! folders, views and preferences. Nothing is copied when the Flathub build
//! already has settings, when the bundle has none, or when the sandbox
//! cannot read them; the app then starts with the defaults, as any first
//! launch does. Saved SMB passwords need no copy: both read the desktop's
//! Secret Service under the same schema.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use ox_core::settings::Settings;

/// The bundle's application ID, whose sandbox data folder holds its settings.
const BUNDLE_APP_ID: &str = "io.winspace.Development";

/// Copies the bundle's settings into this build's settings folder once, in
/// the Flathub build only. A failure is reported and otherwise ignored.
pub(crate) fn adopt_bundle_settings() {
    if !crate::config::IS_FLATHUB {
        return;
    }
    let source = bundle_settings_folder(&gtk::glib::home_dir());
    let target = Settings::default_directory();
    if let Err(error) = adopt(&source, &target) {
        eprintln!(
            "OpenXplorer: could not copy the settings of {}: {error}",
            source.display()
        );
    }
}

/// The bundle's settings folder under `home`.
fn bundle_settings_folder(home: &Path) -> PathBuf {
    home.join(".var/app").join(BUNDLE_APP_ID).join("config/winspace")
}

/// Copies `source` to `target` unless `target` exists or `source` is not a
/// readable folder. The copy is made beside `target` and renamed into
/// place, so an interrupted copy is never taken for settings.
fn adopt(source: &Path, target: &Path) -> io::Result<()> {
    if target.exists() || !source.is_dir() {
        return Ok(());
    }
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::other("the settings folder has no parent"))?;
    fs::create_dir_all(parent)?;
    let staging = parent.join(".winspace-from-bundle");
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    let copied = copy_folder(source, &staging).and_then(|()| fs::rename(&staging, target));
    if copied.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    copied
}

/// Copies the regular files and folders under `source` to a new private
/// folder `target`, skipping links and special files.
fn copy_folder(source: &Path, target: &Path) -> io::Result<()> {
    fs::create_dir(target)?;
    fs::set_permissions(target, fs::Permissions::from_mode(0o700))?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let destination = target.join(entry.file_name());
        if kind.is_dir() {
            copy_folder(&entry.path(), &destination)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &destination)?;
            fs::set_permissions(&destination, fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(())
}
