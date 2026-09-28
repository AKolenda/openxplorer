// SPDX-License-Identifier: AGPL-3.0-only
//! Whether the app runs on the host or inside a Flatpak sandbox.
//!
//! The Python app only ever ran on the host (the Debian package), so this
//! module has no Python counterpart. Inside Flatpak the per-user files the
//! integrations change (`mimeapps.list`, the D-Bus service and autostart
//! files, Brave's preferences) are host files that the sandbox cannot see
//! at their host paths, and the host's programs are not on the sandbox's
//! file system. The integrations therefore reach the host only through
//! `flatpak-spawn --host` and the desktop portals, never by writing host
//! files directly; see [`HostCommand`](super::host_command::HostCommand).

use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The file Flatpak places at the root of every sandbox.
const FLATPAK_INFO_FILE: &str = "/.flatpak-info";

/// The variable Flatpak sets to the application ID inside the sandbox.
const FLATPAK_ID_VARIABLE: &str = "FLATPAK_ID";

/// Where a Flatpak sandbox shows the host's `/usr`, `/bin` and `/etc` when
/// the app has the `host-os` file system permission.
const HOST_ROOT_IN_FLATPAK: &str = "/run/host";

/// Where the app runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sandbox {
    /// Directly on the host, as the Debian package does.
    Host,
    /// Inside a Flatpak sandbox.
    Flatpak,
}

impl Sandbox {
    /// Detects the sandbox from `FLATPAK_ID` and `/.flatpak-info`, the two
    /// markers Flatpak provides; either one is enough.
    pub fn detect() -> Self {
        let flatpak_id = env::var_os(FLATPAK_ID_VARIABLE);
        let has_flatpak_info = Path::new(FLATPAK_INFO_FILE).exists();
        Self::from_markers(flatpak_id.as_deref(), has_flatpak_info)
    }

    /// The sandbox that the markers describe: a non-empty `FLATPAK_ID` or
    /// an existing `/.flatpak-info` means Flatpak.
    pub fn from_markers(flatpak_id: Option<&OsStr>, has_flatpak_info: bool) -> Self {
        let has_flatpak_id = flatpak_id.is_some_and(|id| !id.is_empty());
        if has_flatpak_id || has_flatpak_info {
            Self::Flatpak
        } else {
            Self::Host
        }
    }

    /// True inside a Flatpak sandbox.
    pub fn is_flatpak(self) -> bool {
        self == Self::Flatpak
    }

    /// The directory under which the host's system files appear: `/` on
    /// the host, `/run/host` inside Flatpak.
    pub fn host_root(self) -> PathBuf {
        match self {
            Self::Host => PathBuf::from("/"),
            Self::Flatpak => PathBuf::from(HOST_ROOT_IN_FLATPAK),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One way the sandbox markers can be set, and the sandbox it means.
    struct MarkerCase {
        flatpak_id: Option<&'static str>,
        has_flatpak_info: bool,
        expected: Sandbox,
    }

    #[test]
    fn either_flatpak_marker_means_flatpak() {
        let cases = [
            MarkerCase {
                flatpak_id: None,
                has_flatpak_info: false,
                expected: Sandbox::Host,
            },
            MarkerCase {
                flatpak_id: Some(""),
                has_flatpak_info: false,
                expected: Sandbox::Host,
            },
            MarkerCase {
                flatpak_id: Some("io.winspace.Development"),
                has_flatpak_info: false,
                expected: Sandbox::Flatpak,
            },
            MarkerCase {
                flatpak_id: None,
                has_flatpak_info: true,
                expected: Sandbox::Flatpak,
            },
        ];
        for case in cases {
            let flatpak_id = case.flatpak_id.map(OsStr::new);

            let sandbox = Sandbox::from_markers(flatpak_id, case.has_flatpak_info);

            assert_eq!(sandbox, case.expected, "{:?}", case.flatpak_id);
        }
    }

    #[test]
    fn host_files_appear_under_run_host_inside_flatpak() {
        assert_eq!(Sandbox::Host.host_root(), Path::new("/"));
        assert_eq!(Sandbox::Flatpak.host_root(), Path::new("/run/host"));
    }
}
