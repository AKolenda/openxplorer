// SPDX-License-Identifier: AGPL-3.0-only
//! The lines the update helper writes on its standard output, and how the
//! app reads them back. Ports nothing: the Python app ran `pkexec apt-get`
//! itself.
//!
//! Each line is one JSON object with a `report` field. The helper never
//! writes a message for people: a failure is reported by what went wrong
//! ([`HelperFailure`]), and the app turns it back into its own
//! [`UpdateError`], whose text is the app's, in the desktop's language. So
//! every error the update dialog shows reads exactly as it did when the
//! app downloaded and checked the installer itself.
//!
//! A line the app cannot read is ignored, as `admin_delete`'s protocol
//! does: pkexec, APT or a newer helper may print more than this version
//! knows.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{ReleaseVersion, UpdateError};

/// One step or the outcome of the helper's installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelperReport {
    /// The helper is downloading and verifying the installer.
    Downloading,
    /// The installer is verified and APT is starting; cancelling no longer
    /// stops it.
    Installing,
    /// The package manager confirmed this version.
    Installed(ReleaseVersion),
    /// The installation was refused or failed; nothing was installed
    /// unless APT itself failed part-way.
    Failed(HelperFailure),
}

/// Why the helper refused or failed, without a message: the app has the
/// words. Each case is the [`UpdateError`] of the same name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "failure", rename_all = "kebab-case")]
pub enum HelperFailure {
    /// [`UpdateError::NoStableRelease`].
    NoStableRelease,
    /// [`UpdateError::InvalidTag`].
    InvalidTag,
    /// [`UpdateError::UnsupportedVersion`].
    UnsupportedVersion,
    /// [`UpdateError::InvalidAssetList`].
    InvalidAssetList,
    /// [`UpdateError::MissingInstaller`].
    MissingInstaller,
    /// [`UpdateError::MissingDigest`].
    MissingDigest,
    /// [`UpdateError::InvalidInstallerSize`].
    InvalidInstallerSize,
    /// [`UpdateError::UntrustedLocation`].
    UntrustedLocation,
    /// [`UpdateError::ResponseTooLarge`].
    ResponseTooLarge,
    /// [`UpdateError::InvalidResponse`].
    InvalidResponse,
    /// [`UpdateError::CheckRefused`].
    CheckRefused {
        /// The HTTP status.
        status: u32,
    },
    /// [`UpdateError::DownloadRefused`].
    DownloadRefused {
        /// The HTTP status.
        status: u32,
    },
    /// [`UpdateError::Unreachable`].
    Unreachable,
    /// [`UpdateError::Cancelled`].
    Cancelled,
    /// [`UpdateError::TaskRunning`]: another helper is installing.
    TaskRunning,
    /// [`UpdateError::NotChecked`]: the version asked for is not the latest
    /// release, or not newer than the installed one.
    NotChecked,
    /// [`UpdateError::InstallUnavailable`]: the `openxplorer` package is not
    /// installed, so the helper has nothing to update.
    NotInstalled,
    /// [`UpdateError::InstallerTooLarge`].
    InstallerTooLarge,
    /// [`UpdateError::ChecksumMismatch`].
    ChecksumMismatch,
    /// [`UpdateError::MetadataMismatch`].
    MetadataMismatch,
    /// [`UpdateError::InstallFailed`].
    InstallFailed {
        /// The end of APT's error output.
        details: String,
    },
    /// [`UpdateError::InstallNotConfirmed`].
    InstallNotConfirmed,
    /// [`UpdateError::PackageToolFailed`].
    PackageToolFailed {
        /// The tool's path, one of [`PACKAGE_TOOLS`].
        program: String,
        /// Its exit status.
        status: i32,
    },
    /// [`UpdateError::PackageToolTimedOut`].
    PackageToolTimedOut {
        /// The tool's path, one of [`PACKAGE_TOOLS`].
        program: String,
        /// Its time limit.
        seconds: u64,
    },
    /// Anything else, such as a file-system error in the helper's private
    /// folder, with the operating system's own words. Shown as
    /// [`UpdateError::InstallFailed`].
    Other {
        /// What went wrong.
        details: String,
    },
}

/// The package tools a failure may name, so the app's error keeps a
/// static program path.
const PACKAGE_TOOLS: [&str; 3] = ["/usr/bin/dpkg-deb", "/usr/bin/dpkg-query", "/usr/bin/apt-get"];

/// The JSON form of a line.
#[derive(Serialize, Deserialize)]
#[serde(tag = "report", rename_all = "kebab-case")]
enum Wire {
    Downloading,
    Installing,
    Installed { version: String },
    Failed(HelperFailure),
}

impl HelperReport {
    /// The line for this report, without its newline.
    ///
    /// # Panics
    ///
    /// Never: every report is plain JSON.
    pub fn encode(&self) -> String {
        let wire = match self {
            Self::Downloading => Wire::Downloading,
            Self::Installing => Wire::Installing,
            Self::Installed(version) => Wire::Installed {
                version: version.to_string(),
            },
            Self::Failed(failure) => Wire::Failed(failure.clone()),
        };
        serde_json::to_string(&wire).expect("a report is always valid JSON")
    }

    /// The report `line` holds, or `None` for any other line.
    pub fn decode(line: &str) -> Option<Self> {
        let report = match serde_json::from_str(line).ok()? {
            Wire::Downloading => Self::Downloading,
            Wire::Installing => Self::Installing,
            Wire::Installed { version } => Self::Installed(version.parse().ok()?),
            Wire::Failed(failure) => Self::Failed(failure),
        };
        Some(report)
    }
}

impl HelperFailure {
    /// The failure that reports `error`.
    pub fn of(error: &UpdateError) -> Self {
        match error {
            UpdateError::NoStableRelease => Self::NoStableRelease,
            UpdateError::InvalidTag => Self::InvalidTag,
            UpdateError::UnsupportedVersion => Self::UnsupportedVersion,
            UpdateError::InvalidAssetList => Self::InvalidAssetList,
            UpdateError::MissingInstaller => Self::MissingInstaller,
            UpdateError::MissingDigest => Self::MissingDigest,
            UpdateError::InvalidInstallerSize => Self::InvalidInstallerSize,
            UpdateError::UntrustedLocation => Self::UntrustedLocation,
            UpdateError::ResponseTooLarge => Self::ResponseTooLarge,
            UpdateError::InvalidResponse => Self::InvalidResponse,
            UpdateError::CheckRefused { status } => Self::CheckRefused { status: *status },
            UpdateError::DownloadRefused { status } => Self::DownloadRefused { status: *status },
            UpdateError::Unreachable => Self::Unreachable,
            UpdateError::Cancelled => Self::Cancelled,
            UpdateError::TaskRunning => Self::TaskRunning,
            UpdateError::NotChecked => Self::NotChecked,
            UpdateError::InstallUnavailable { .. } => Self::NotInstalled,
            UpdateError::InstallerTooLarge => Self::InstallerTooLarge,
            UpdateError::ChecksumMismatch => Self::ChecksumMismatch,
            UpdateError::MetadataMismatch => Self::MetadataMismatch,
            UpdateError::InstallFailed { details } => Self::InstallFailed {
                details: details.clone(),
            },
            UpdateError::InstallNotConfirmed => Self::InstallNotConfirmed,
            UpdateError::PackageToolFailed { program, status } => Self::PackageToolFailed {
                program: (*program).to_owned(),
                status: *status,
            },
            UpdateError::PackageToolTimedOut { program, limit } => Self::PackageToolTimedOut {
                program: (*program).to_owned(),
                seconds: limit.as_secs(),
            },
            other => Self::Other {
                details: other.to_string(),
            },
        }
    }

    /// The app's error for this failure.
    pub fn into_error(self) -> UpdateError {
        match self {
            Self::NoStableRelease => UpdateError::NoStableRelease,
            Self::InvalidTag => UpdateError::InvalidTag,
            Self::UnsupportedVersion => UpdateError::UnsupportedVersion,
            Self::InvalidAssetList => UpdateError::InvalidAssetList,
            Self::MissingInstaller => UpdateError::MissingInstaller,
            Self::MissingDigest => UpdateError::MissingDigest,
            Self::InvalidInstallerSize => UpdateError::InvalidInstallerSize,
            Self::UntrustedLocation => UpdateError::UntrustedLocation,
            Self::ResponseTooLarge => UpdateError::ResponseTooLarge,
            Self::InvalidResponse => UpdateError::InvalidResponse,
            Self::CheckRefused { status } => UpdateError::CheckRefused { status },
            Self::DownloadRefused { status } => UpdateError::DownloadRefused { status },
            Self::Unreachable => UpdateError::Unreachable,
            Self::Cancelled => UpdateError::Cancelled,
            Self::TaskRunning => UpdateError::TaskRunning,
            Self::NotChecked => UpdateError::NotChecked,
            Self::NotInstalled => UpdateError::InstallUnavailable {
                installation: super::Installation::DebianPackageWithoutTools,
            },
            Self::InstallerTooLarge => UpdateError::InstallerTooLarge,
            Self::ChecksumMismatch => UpdateError::ChecksumMismatch,
            Self::MetadataMismatch => UpdateError::MetadataMismatch,
            Self::InstallFailed { details } | Self::Other { details } => {
                UpdateError::InstallFailed { details }
            }
            Self::InstallNotConfirmed => UpdateError::InstallNotConfirmed,
            Self::PackageToolFailed { program, status } => match package_tool(&program) {
                Some(program) => UpdateError::PackageToolFailed { program, status },
                None => UpdateError::InstallFailed { details: program },
            },
            Self::PackageToolTimedOut { program, seconds } => match package_tool(&program) {
                Some(program) => UpdateError::PackageToolTimedOut {
                    program,
                    limit: Duration::from_secs(seconds),
                },
                None => UpdateError::InstallFailed { details: program },
            },
        }
    }
}

/// The static path of the package tool `program` names.
fn package_tool(program: &str) -> Option<&'static str> {
    PACKAGE_TOOLS.into_iter().find(|tool| *tool == program)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every report reads back as itself.
    #[test]
    fn every_report_reads_back_as_itself() {
        let reports = [
            HelperReport::Downloading,
            HelperReport::Installing,
            HelperReport::Installed(ReleaseVersion::new(2, 0, 16)),
            HelperReport::Failed(HelperFailure::ChecksumMismatch),
            HelperReport::Failed(HelperFailure::DownloadRefused { status: 404 }),
            HelperReport::Failed(HelperFailure::InstallFailed {
                details: "E: line one\n\"quoted\"\tline two".to_owned(),
            }),
        ];

        for report in reports {
            let line = report.encode();

            assert!(!line.contains('\n'), "{line}");
            assert_eq!(HelperReport::decode(&line), Some(report), "{line}");
        }
    }

    /// pkexec's and APT's own lines, and a version that is not a release
    /// version, are not reports.
    #[test]
    fn other_lines_are_not_reports() {
        let lines = [
            "",
            "Reading package lists...",
            "{}",
            r#"{"report":"installed","version":"../../2.0.16"}"#,
            r#"{"report":"installed","version":"2.0.16-1"}"#,
            r#"{"report":"failed","failure":"made-up"}"#,
            r#"{"report":"unknown"}"#,
        ];

        for line in lines {
            assert_eq!(HelperReport::decode(line), None, "{line}");
        }
    }

    /// The app shows the helper's failures in its own words, the same as
    /// when it downloaded and checked the installer itself.
    #[test]
    fn a_failure_becomes_the_apps_own_error_again() {
        let errors = [
            UpdateError::NoStableRelease,
            UpdateError::NotChecked,
            UpdateError::ChecksumMismatch,
            UpdateError::MetadataMismatch,
            UpdateError::InstallNotConfirmed,
            UpdateError::Unreachable,
            UpdateError::DownloadRefused { status: 503 },
            UpdateError::InstallFailed {
                details: "E: Sub-process /usr/bin/dpkg returned an error code (1)".to_owned(),
            },
            UpdateError::PackageToolFailed {
                program: "/usr/bin/dpkg-deb",
                status: 2,
            },
            UpdateError::PackageToolTimedOut {
                program: "/usr/bin/dpkg-query",
                limit: Duration::from_secs(30),
            },
        ];

        for error in errors {
            let line = HelperReport::Failed(HelperFailure::of(&error)).encode();
            let Some(HelperReport::Failed(failure)) = HelperReport::decode(&line) else {
                panic!("{line} is a failure");
            };

            assert_eq!(failure.into_error().to_string(), error.to_string(), "{line}");
        }
    }

    /// A tool the app does not know is not trusted to be a program path.
    #[test]
    fn an_unknown_tool_is_shown_as_a_failed_installation() {
        let failure = HelperFailure::PackageToolFailed {
            program: "/tmp/fixture-tool".to_owned(),
            status: 1,
        };

        let error = failure.into_error();

        assert!(matches!(error, UpdateError::InstallFailed { .. }), "{error:?}");
    }
}
