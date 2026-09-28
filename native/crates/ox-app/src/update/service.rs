// SPDX-License-Identifier: AGPL-3.0-only
//! The update service of the running executable: GitHub, the system's
//! package tools and the fixed restart launcher.
//!
//! Ports the module-level `Updater(root=ROOT)` and `RUNTIME` of
//! `desktop/winspace.py`: the service compares the build installed after
//! an update with the one running, which the Python app identified by
//! hashing its files at startup.

use std::io;
use std::path::{Path, PathBuf};

use gtk::gio;
use ox_core::update::{
    InstalledBuild, ReleaseVersion, RuntimeIdentity, SessionLauncher, UpdateService, UpdateServiceParts,
    Updater, RUNTIME_PROTOCOL,
};

/// The file the kernel keeps for the running executable. Reading it gives
/// the program this process started from, even after a package upgrade
/// replaced the file on disk, so the running build's identity is right
/// whenever it is read.
const RUNNING_EXECUTABLE: &str = "/proc/self/exe";

/// What Linux appends to the path of an executable that was replaced
/// while it ran.
const REPLACED_SUFFIX: &str = " (deleted)";

/// The version of the running build, from the workspace's
/// `Cargo.toml`.
///
/// # Panics
///
/// Never for a valid build: the workspace version is a plain
/// `MAJOR.MINOR.PATCH`, which a test checks.
pub(super) fn running_version() -> ReleaseVersion {
    env!("CARGO_PKG_VERSION")
        .parse()
        .expect("the workspace version is MAJOR.MINOR.PATCH")
}

/// The path of this build's executable, as installed: `std::env::current_exe`
/// without the suffix Linux adds once an upgrade replaced it, or the
/// running file itself when the path cannot be read.
pub(crate) fn this_executable() -> PathBuf {
    let Ok(path) = std::env::current_exe() else {
        return PathBuf::from(RUNNING_EXECUTABLE);
    };
    let text = path.to_string_lossy();
    match text.strip_suffix(REPLACED_SUFFIX) {
        Some(installed) => PathBuf::from(installed),
        None => path,
    }
}

/// The update service of the build installed at `executable`. Reading the
/// running build's identity reads the whole executable, so it runs on a
/// worker thread.
pub(super) async fn for_executable(executable: PathBuf) -> UpdateService {
    let version = running_version();
    let identity = gio::spawn_blocking(move || running_identity(version)).await;
    // Data safety: an identity that cannot be read matches no installed
    // build, so after any installation the app waits for a restart.
    let running = identity
        .ok()
        .and_then(Result::ok)
        .unwrap_or_else(|| unknown_identity(version));
    UpdateService::new(UpdateServiceParts {
        updater: Updater::for_executable(version, &executable),
        running,
        installed_build: InstalledBuild::Executable {
            path: executable,
            version: version.to_string(),
        },
        launcher: Box::new(SessionLauncher),
    })
}

/// The identity of the running build.
///
/// # Errors
///
/// Any error reading the running executable.
pub(crate) fn running_identity(version: ReleaseVersion) -> io::Result<RuntimeIdentity> {
    RuntimeIdentity::of_executable(Path::new(RUNNING_EXECUTABLE), &version.to_string())
}

/// An identity that matches no build: this version, without a digest.
pub(crate) fn unknown_identity(version: ReleaseVersion) -> RuntimeIdentity {
    RuntimeIdentity {
        version: version.to_string(),
        protocol: RUNTIME_PROTOCOL,
        build: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_workspace_version_is_a_release_version() {
        assert_eq!(running_version().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn the_running_executable_has_a_digest() {
        let identity = running_identity(running_version()).expect("the test binary is readable");
        assert_eq!(identity.build.len(), 64, "{identity:?}");
        assert_ne!(identity, unknown_identity(running_version()));
    }
}
