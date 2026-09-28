// SPDX-License-Identifier: AGPL-3.0-only
//! How this build of the app was installed, which decides whether it may
//! update itself. Ports `can_install` in `desktop/updater.py`; the
//! Flatpak and other-package cases are new.

use std::path::Path;

use rustix::fs::Access;

/// Where the Debian package installs the application.
const PACKAGE_ROOT: &str = "/opt/openxplorer";

/// The programs an in-app installation runs, relative to the file-system
/// root: the polkit prompt, APT, the package inspector and the launcher
/// that restarts into the new version.
const INSTALL_TOOLS: [&str; 4] = [
    "usr/bin/pkexec",
    "usr/bin/apt-get",
    "usr/bin/dpkg-deb",
    "usr/bin/openxplorer",
];

/// The file Flatpak places at the root of every sandbox.
const FLATPAK_MARKER: &str = ".flatpak-info";

/// Folders only a system package manager installs into, relative to the
/// file-system root.
const SYSTEM_PREFIXES: [&str; 3] = ["usr", "opt", "snap"];

/// How this build was installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installation {
    /// The Debian package in `/opt/openxplorer`, with polkit, APT and the
    /// launcher available: the app can install updates itself.
    DebianPackage,
    /// The Debian package's folder, but a program the installation needs
    /// is missing or not executable.
    DebianPackageWithoutTools,
    /// A Flatpak. Flatpak updates it (GNOME Software or `flatpak update`),
    /// never the app itself.
    Flatpak,
    /// Installed under `/usr`, `/opt` or `/snap` by another package
    /// manager, which also updates it.
    OtherPackage,
    /// Built from source or unpacked by hand.
    Unpackaged,
}

impl Installation {
    /// Detects how the application in `app_root` was installed.
    ///
    /// `filesystem_root` is `/` except in tests, which build a fake system
    /// in a temporary folder.
    ///
    /// Safety rule "in-app installation only for the packaged build"
    /// (`can_install` in `desktop/updater.py`): only the Debian package's
    /// exact folder with executable `pkexec`, `apt-get`, `dpkg-deb` and
    /// `openxplorer` may install; a source build never runs the package
    /// manager.
    pub fn detect(app_root: &Path, filesystem_root: &Path) -> Self {
        if filesystem_root.join(FLATPAK_MARKER).is_file() {
            return Self::Flatpak;
        }
        if app_root == filesystem_root.join(PACKAGE_ROOT.trim_start_matches('/')) {
            return if has_install_tools(filesystem_root) {
                Self::DebianPackage
            } else {
                Self::DebianPackageWithoutTools
            };
        }
        let is_system_folder = SYSTEM_PREFIXES
            .iter()
            .any(|prefix| app_root.starts_with(filesystem_root.join(prefix)));
        if is_system_folder {
            Self::OtherPackage
        } else {
            Self::Unpackaged
        }
    }

    /// Whether the app may download and install updates itself.
    pub fn can_install(self) -> bool {
        self == Self::DebianPackage
    }

    /// Why this build cannot install updates itself, and what to use
    /// instead. The Debian and unpackaged wording is the Python app's.
    pub fn install_refusal(self) -> &'static str {
        match self {
            Self::DebianPackage | Self::DebianPackageWithoutTools | Self::Unpackaged => {
                "In-app installation requires the installed Debian package and polkit. Use GitHub \
                 Releases for this build."
            }
            Self::Flatpak => {
                "This build is a Flatpak. Update OpenXplorer with GNOME Software or flatpak update."
            }
            Self::OtherPackage => {
                "This build was installed by a system package manager. Update OpenXplorer with it, \
                 for example in GNOME Software."
            }
        }
    }
}

/// Whether every program in [`INSTALL_TOOLS`] is an executable file.
fn has_install_tools(filesystem_root: &Path) -> bool {
    INSTALL_TOOLS
        .iter()
        .map(|tool| filesystem_root.join(tool))
        .all(|tool| is_executable_file(&tool))
}

/// Python's `Path(p).is_file() and os.access(p, os.X_OK)`.
fn is_executable_file(path: &Path) -> bool {
    path.is_file() && rustix::fs::access(path, Access::EXEC_OK).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_debian_package_can_install() {
        let installations = [
            Installation::DebianPackageWithoutTools,
            Installation::Flatpak,
            Installation::OtherPackage,
            Installation::Unpackaged,
        ];

        assert!(Installation::DebianPackage.can_install());
        assert!(installations
            .iter()
            .all(|installation| !installation.can_install()));
    }

    #[test]
    fn a_flatpak_is_told_to_update_through_flatpak() {
        let refusal = Installation::Flatpak.install_refusal();

        assert!(refusal.contains("flatpak update"), "{refusal}");
    }
}
