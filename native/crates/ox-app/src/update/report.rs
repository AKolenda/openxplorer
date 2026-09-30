// SPDX-License-Identifier: AGPL-3.0-only
//! `--check` and `--diagnose`: what this build is and what runs, printed
//! without opening a window.
//!
//! Ports the `args.check` and `args.diagnose` branches of `main` in
//! `desktop/winspace.py`. `--check` names the libraries the app runs on
//! and this build's digest; `--diagnose` prints [`Diagnosis`] as JSON.
//! Both only read: the diagnosis asks the bus daemon without starting any
//! service and `xdg-mime` for the current handlers.

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::integration::{
    DefaultApps, FileManagerBus, RequestNotOpened, RevealPaths, RevealRegistration, Sandbox,
};
use ox_core::settings::Settings;
use ox_core::update::{
    AssociationStatus, Associations, Diagnosis, InstanceError, InstanceGuard, SessionBus, ShowInFolder,
};

use super::launch_guard::this_identity;
use super::service::running_version;
use crate::config::APP_ID;

/// What `--check` prints: the version, GTK, GIO's VFS and the build
/// digest, one per line, as the Python app printed them (without
/// WebKitGTK, which the native app does not use).
pub(super) fn check_report() -> String {
    let vfs = gio::Vfs::default();
    let build = this_identity().build;
    let build = if build.is_empty() { "unknown".to_owned() } else { build };
    format!(
        "OpenXplorer {}\nGTK {}.{}.{}\nGIO/GVfs: {}\nBuild: {build}",
        running_version(),
        gtk::major_version(),
        gtk::minor_version(),
        gtk::micro_version(),
        vfs.type_().name(),
    )
}

/// What `--diagnose` prints: the installed and running builds, the
/// associations and Show in folder.
///
/// # Errors
///
/// An [`InstanceError`] if the session bus cannot be reached.
pub(super) fn diagnosis() -> Result<Diagnosis, InstanceError> {
    let bus = SessionBus::connect(APP_ID)?;
    let connection = bus.connection().clone();
    let instance = InstanceGuard::new(bus).status(&this_identity())?;
    let settings = Settings::default_directory();
    let sandbox = Sandbox::detect();
    let associations = match DefaultApps::new(&settings, sandbox).status() {
        Ok(status) => Associations::Read(AssociationStatus::from(&status)),
        Err(error) => Associations::Failed {
            error: error.to_string(),
        },
    };
    let enabled = RevealRegistration::new(&RevealPaths::for_user(&settings), sandbox).is_enabled();
    let observer = FileManagerBus::new(connection, |_, _| Err(RequestNotOpened), || {});
    let bus_status = glib::MainContext::default().block_on(observer.status());
    Ok(Diagnosis::new(
        instance,
        associations,
        ShowInFolder::new(bus_status, enabled),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `--check` names the version, the libraries and a build digest.
    ///
    /// parity: UPD-010
    #[test]
    fn the_check_names_the_libraries_and_the_build() {
        let report = check_report();
        let lines: Vec<&str> = report.lines().collect();
        assert_eq!(lines[0], format!("OpenXplorer {}", running_version()));
        assert!(lines[1].starts_with("GTK 4."), "{report}");
        assert!(lines[2].starts_with("GIO/GVfs: G"), "{report}");
        let build = lines[3].strip_prefix("Build: ").expect("the build line");
        assert_eq!(build.len(), 64, "a SHA-256 digest: {report}");
    }
}
