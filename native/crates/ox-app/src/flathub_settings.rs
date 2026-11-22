// SPDX-License-Identifier: AGPL-3.0-only
//! Brings the settings of the release's Flatpak bundle to the Flathub build.
//!
//! The bundle is `io.winspace.Development` and keeps its settings in
//! `~/.var/app/io.winspace.Development/config/winspace`; the Flathub build is
//! `app.openxplorer.OpenXplorer`, whose settings folder is
//! `~/.var/app/app.openxplorer.OpenXplorer/config/winspace`. Flatpak hides
//! other apps' `~/.var/app` folders even from `--filesystem=host`, so the
//! sandbox cannot read the bundle's folder itself. The host can, and the
//! Flathub build's own data folder is the same path on the host as in the
//! sandbox, so on the first launch, before anything reads the settings,
//! `flatpak-spawn --host cp` copies the bundle's folder beside this build's
//! settings folder, and the copy is renamed into place. The app already
//! talks to `org.freedesktop.Flatpak` for Open in Terminal, so this needs
//! no other permission.
//!
//! The settings folder is then made, copied or not, so later launches never
//! try again. Nothing is copied when the Flathub build already has settings
//! or the bundle has none; the app then starts with the defaults, as any
//! first launch does. Saved SMB passwords need no copy: both read the
//! desktop's Secret Service under the same schema.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use ox_core::settings::Settings;

/// The bundle's application ID, whose sandbox data folder holds its settings.
const BUNDLE_APP_ID: &str = "io.winspace.Development";

/// How long the copy on the host may take before it is given up.
const COPY_TIMEOUT: Duration = Duration::from_secs(10);

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

/// Copies `source` on the host to `target` unless `target` exists, then
/// makes sure `target` exists. The copy is made beside `target` and renamed
/// into place, so an interrupted copy is never taken for settings.
fn adopt(source: &Path, target: &Path) -> io::Result<()> {
    if target.exists() {
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
    // A host that cannot run the copy counts as one without the bundle.
    if copy_on_host(source, &staging).unwrap_or(false) && staging.is_dir() {
        fs::rename(&staging, target)?;
    } else {
        let _ = fs::remove_dir_all(&staging);
        fs::create_dir(target)?;
    }
    fs::set_permissions(target, fs::Permissions::from_mode(0o700))
}

/// Runs `cp -R source staging` on the host; false when `source` is missing
/// or the copy failed or ran out of time.
fn copy_on_host(source: &Path, staging: &Path) -> io::Result<bool> {
    let mut child = Command::new("flatpak-spawn")
        .args(["--host", "cp", "-R", "--"])
        .arg(source)
        .arg(staging)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + COPY_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status.success());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
