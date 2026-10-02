// SPDX-License-Identifier: AGPL-3.0-only
//! How this build of the app was installed, which decides whether it may
//! update itself. Ports `can_install` in `v2.0.0:desktop/updater.py`; the
//! Flatpak and other-package cases are new.

use std::ffi::OsStr;
use std::path::Path;

use rustix::fs::Access;

/// Where the Debian package installs the application, relative to the
/// file-system root.
const PACKAGE_ROOT: &str = "opt/openxplorer";

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
/// file-system root: `/usr` for Debian packages and `/snap` for Snaps.
const PACKAGE_MANAGED_PREFIXES: [&str; 2] = ["usr", "snap"];

/// The administrator's own prefix inside `/usr`, relative to the
/// file-system root. `make install` and `cargo install --root /usr/local`
/// install here; no package manager does.
const LOCAL_PREFIX: &str = "usr/local";

/// The folder of an installation prefix that holds its programs, as in
/// `/opt/openxplorer/bin/openxplorer`.
const PROGRAM_FOLDER: &str = "bin";

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
    /// Installed under `/usr` (but not `/usr/local`) or `/snap` by another
    /// package manager, which also updates it.
    OtherPackage,
    /// Built from source, installed into `/usr/local`, or unpacked by hand,
    /// for example into another folder of `/opt`.
    Unpackaged,
}

impl Installation {
    /// Detects how the application in `app_root` was installed.
    ///
    /// `filesystem_root` is `/` except in tests, which build a fake system
    /// in a temporary folder.
    ///
    /// Safety rule "in-app installation only for the packaged build"
    /// (`can_install` in `v2.0.0:desktop/updater.py`): only the Debian package's
    /// exact folder with executable `pkexec`, `apt-get`, `dpkg-deb` and
    /// `openxplorer` may install; a source build never runs the package
    /// manager.
    pub fn detect(app_root: &Path, filesystem_root: &Path) -> Self {
        if filesystem_root.join(FLATPAK_MARKER).is_file() {
            return Self::Flatpak;
        }
        if app_root == filesystem_root.join(PACKAGE_ROOT) {
            return if has_install_tools(filesystem_root) {
                Self::DebianPackage
            } else {
                Self::DebianPackageWithoutTools
            };
        }
        if is_package_managed(app_root, filesystem_root) {
            Self::OtherPackage
        } else {
            Self::Unpackaged
        }
    }

    /// Detects how the native app whose executable is at `executable` was
    /// installed, as [`Installation::detect`] does for its installation
    /// folder. Pass the resolved path, as [`std::env::current_exe`] gives
    /// it, not the path of a symbolic link to it.
    ///
    /// The installation folder of `<prefix>/bin/<program>` is `<prefix>`;
    /// of any other executable, the folder it is in. So the native Debian
    /// package must keep its executable in `/opt/openxplorer` or
    /// `/opt/openxplorer/bin`, with `/usr/bin/openxplorer` as a link or
    /// launcher, as the Python package does; an executable directly in
    /// `/usr/bin` counts as [`Installation::OtherPackage`]. That layout is
    /// decided with the native packaging (UPD-017).
    pub fn detect_for_executable(executable: &Path, filesystem_root: &Path) -> Self {
        Self::detect(installation_folder(executable), filesystem_root)
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
                crate::i18n::gettext_static(
                    "In-app installation requires the installed Debian package and polkit. Use GitHub \
                 Releases for this build.",
                )
            }
            Self::Flatpak => crate::i18n::gettext_static(
                "This build is a Flatpak. Update OpenXplorer with GNOME Software or flatpak update.",
            ),
            Self::OtherPackage => crate::i18n::gettext_static(
                "This build was installed by a system package manager. Update OpenXplorer with it, \
                 for example in GNOME Software.",
            ),
        }
    }
}

/// Whether `app_root` is in a folder only a system package manager
/// installs into.
fn is_package_managed(app_root: &Path, filesystem_root: &Path) -> bool {
    let in_managed_prefix = PACKAGE_MANAGED_PREFIXES
        .iter()
        .any(|prefix| app_root.starts_with(filesystem_root.join(prefix)));
    let in_local_prefix = app_root.starts_with(filesystem_root.join(LOCAL_PREFIX));
    in_managed_prefix && !in_local_prefix
}

/// The installation folder of the executable at `executable`: `<prefix>`
/// for `<prefix>/bin/<program>`, otherwise the folder it is in.
fn installation_folder(executable: &Path) -> &Path {
    let Some(folder) = executable.parent() else {
        return executable;
    };
    let is_program_folder = folder.file_name() == Some(OsStr::new(PROGRAM_FOLDER));
    match folder.parent() {
        Some(prefix) if is_program_folder => prefix,
        _ => folder,
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

    #[test]
    fn the_installation_folder_of_a_program_in_bin_is_its_prefix() {
        struct FolderCase {
            executable: &'static str,
            folder: &'static str,
        }
        let cases = [
            FolderCase {
                executable: "/opt/openxplorer/bin/openxplorer",
                folder: "/opt/openxplorer",
            },
            FolderCase {
                executable: "/opt/openxplorer/openxplorer",
                folder: "/opt/openxplorer",
            },
            FolderCase {
                executable: "/usr/bin/openxplorer",
                folder: "/usr",
            },
            FolderCase {
                executable: "/bin/openxplorer",
                folder: "/",
            },
            FolderCase {
                executable: "/openxplorer",
                folder: "/",
            },
        ];

        for case in cases {
            let folder = installation_folder(Path::new(case.executable));

            assert_eq!(folder, Path::new(case.folder), "{}", case.executable);
        }
    }
}
