// SPDX-License-Identifier: AGPL-3.0-only
//! The package-manager commands of an installation, and what their output
//! must say. Ports the `run` calls of `Updater.install` in
//! `desktop/updater.py`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use super::release::INSTALLER_ARCHITECTURE;
use super::{ReleaseVersion, UpdateError};

/// How long `dpkg-deb` and `dpkg-query` may run.
const PACKAGE_QUERY_TIME_LIMIT: Duration = Duration::from_secs(30);

/// The Debian package name.
const PACKAGE_NAME: &str = "openxplorer";

/// How many characters of APT's error output a failure message keeps.
const MAX_FAILURE_CHARS: usize = 2000;

/// One of the three commands an installation runs, always with a fixed
/// program path and fixed arguments around the downloaded installer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageCommand {
    /// `dpkg-deb -f <installer> Package Version Architecture`: what the
    /// downloaded installer says it is.
    InspectInstaller(PathBuf),
    /// `pkexec apt-get -y --no-remove install <installer>`: the
    /// installation, behind the administrator prompt.
    Install(PathBuf),
    /// `dpkg-query -W -f=${Status}\n${Version} openxplorer`: what is
    /// installed now.
    QueryInstalled,
}

impl PackageCommand {
    /// The program's absolute path.
    pub fn program(&self) -> &'static str {
        match self {
            Self::InspectInstaller(_) => "/usr/bin/dpkg-deb",
            Self::Install(_) => "/usr/bin/pkexec",
            Self::QueryInstalled => "/usr/bin/dpkg-query",
        }
    }

    /// The whole command line, program first. No shell ever reads it.
    pub fn argv(&self) -> Vec<OsString> {
        let mut argv = vec![OsString::from(self.program())];
        argv.extend(self.arguments());
        argv
    }

    /// The command line after the program.
    pub(super) fn arguments(&self) -> Vec<OsString> {
        let mut arguments = Vec::new();
        match self {
            Self::InspectInstaller(installer) => {
                arguments.push("-f".into());
                arguments.push(installer.into());
                arguments.extend(["Package", "Version", "Architecture"].map(OsString::from));
            }
            Self::Install(installer) => {
                // --no-remove refuses a dependency resolution that would
                // remove packages.
                arguments.extend(["/usr/bin/apt-get", "-y", "--no-remove", "install"].map(OsString::from));
                arguments.push(installer.into());
            }
            Self::QueryInstalled => {
                arguments.extend(["-W", "-f=${Status}\n${Version}", PACKAGE_NAME].map(OsString::from));
            }
        }
        arguments
    }

    /// How long the command may run before it is stopped.
    ///
    /// Safety rule "never interrupt the package manager"
    /// (`desktop/updater.py`: "No timeout: killing apt/dpkg mid-install can
    /// damage package state"): the installation has no limit.
    pub fn time_limit(&self) -> Option<Duration> {
        match self {
            Self::InspectInstaller(_) | Self::QueryInstalled => Some(PACKAGE_QUERY_TIME_LIMIT),
            Self::Install(_) => None,
        }
    }
}

/// What a finished command printed and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    /// The exit status, or minus the signal that ended it, as Python's
    /// `returncode`.
    pub exit_status: i32,
    /// Standard output, as text.
    pub stdout: String,
    /// Standard error, as text.
    pub stderr: String,
}

impl CommandOutput {
    /// Whether the command exited with status 0.
    pub fn is_success(&self) -> bool {
        self.exit_status == 0
    }
}

/// Runs package commands: [`SystemPackageManager`](super::SystemPackageManager)
/// in the app, a recording double in tests.
pub trait PackageManager: Send + Sync {
    /// Runs `command` to completion, or until its
    /// [`time limit`](PackageCommand::time_limit).
    ///
    /// # Errors
    ///
    /// [`UpdateError::Io`] if it cannot start, and
    /// [`UpdateError::PackageToolTimedOut`] if it was stopped at its limit.
    fn run(&self, command: &PackageCommand) -> Result<CommandOutput, UpdateError>;
}

/// Checks what `dpkg-deb` says the installer is.
///
/// Safety rule "the installer is what the release says"
/// (`Updater.install`): the fields must be exactly Package `openxplorer`,
/// the checked version and the installer's architecture, and nothing else,
/// before the administrator prompt appears.
///
/// # Errors
///
/// [`UpdateError::PackageToolFailed`] if `dpkg-deb` failed and
/// [`UpdateError::MetadataMismatch`] for any other package.
pub(super) fn check_installer_fields(
    output: &CommandOutput,
    version: ReleaseVersion,
) -> Result<(), UpdateError> {
    require_success(output, "/usr/bin/dpkg-deb")?;
    let version_text = version.to_string();
    let expected = BTreeMap::from([
        ("Package", PACKAGE_NAME),
        ("Version", version_text.as_str()),
        ("Architecture", INSTALLER_ARCHITECTURE),
    ]);
    if control_fields(&output.stdout) == expected {
        Ok(())
    } else {
        Err(UpdateError::MetadataMismatch)
    }
}

/// The failure of the installation step, if APT did not succeed: the
/// administrator prompt was refused (126, 127) or APT failed.
pub(super) fn installation_failure(output: &CommandOutput) -> Option<UpdateError> {
    if output.is_success() {
        return None;
    }
    let report = if output.stderr.is_empty() {
        &output.stdout
    } else {
        &output.stderr
    };
    Some(UpdateError::InstallFailed {
        details: last_chars(report.trim(), MAX_FAILURE_CHARS),
    })
}

/// Checks that `dpkg-query` reports `version` as installed.
///
/// Safety rule "success means the package manager confirms it"
/// (`Updater.install`): APT's exit status alone does not mark an update
/// installed.
///
/// # Errors
///
/// [`UpdateError::PackageToolFailed`] if `dpkg-query` failed and
/// [`UpdateError::InstallNotConfirmed`] for any other answer.
pub(super) fn check_installed_version(
    output: &CommandOutput,
    version: ReleaseVersion,
) -> Result<(), UpdateError> {
    require_success(output, "/usr/bin/dpkg-query")?;
    let expected = format!("install ok installed\n{version}");
    if output.stdout.trim() == expected {
        Ok(())
    } else {
        Err(UpdateError::InstallNotConfirmed)
    }
}

/// `Name: value` lines, as Python's
/// `dict(line.split(': ', 1) for line in ... if ': ' in line)`: a later
/// duplicate wins.
fn control_fields(text: &str) -> BTreeMap<&str, &str> {
    text.lines().filter_map(|line| line.split_once(": ")).collect()
}

/// Python's `check=True`: a failed query is an error of its own.
fn require_success(output: &CommandOutput, program: &'static str) -> Result<(), UpdateError> {
    if output.is_success() {
        Ok(())
    } else {
        Err(UpdateError::PackageToolFailed {
            program,
            status: output.exit_status,
        })
    }
}

/// The last `count` characters of `text`.
fn last_chars(text: &str, count: usize) -> String {
    let skipped = text.chars().count().saturating_sub(count);
    text.chars().skip(skipped).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(exit_status: i32, stdout: &str, stderr: &str) -> CommandOutput {
        CommandOutput {
            exit_status,
            stdout: stdout.to_owned(),
            stderr: stderr.to_owned(),
        }
    }

    #[test]
    fn a_later_duplicate_field_wins_and_other_lines_are_ignored() {
        let fields = control_fields("Package: a\nnot a field\nPackage: b\nNote: x: y\n");

        assert_eq!(fields, BTreeMap::from([("Package", "b"), ("Note", "x: y")]));
    }

    #[test]
    fn an_extra_field_is_a_different_package() {
        let version = ReleaseVersion::new(1, 0, 1);
        let fields = "Package: openxplorer\nVersion: 1.0.1\nArchitecture: all\nEssential: yes\n";

        let result = check_installer_fields(&output(0, fields, ""), version);

        assert!(matches!(result, Err(UpdateError::MetadataMismatch)));
    }

    #[test]
    fn apt_failure_reports_the_end_of_stderr_or_else_stdout() {
        let long_error = format!("{}END", "x".repeat(3000));

        let from_stderr = installation_failure(&output(100, "ignored", &long_error)).unwrap();
        let from_stdout = installation_failure(&output(126, " refused \n", "")).unwrap();

        let message = from_stderr.to_string();
        assert!(message.starts_with("Installation was cancelled or failed. xxx"));
        assert!(message.ends_with("END"));
        assert_eq!(
            message.chars().count(),
            "Installation was cancelled or failed. ".len() + 2000
        );
        assert_eq!(
            from_stdout.to_string(),
            "Installation was cancelled or failed. refused"
        );
        assert!(installation_failure(&output(0, "", "warnings")).is_none());
    }

    #[test]
    fn a_failed_query_is_its_own_error() {
        let result = check_installed_version(&output(1, "", ""), ReleaseVersion::new(1, 0, 1));

        assert_eq!(
            result.unwrap_err().to_string(),
            "The package tool /usr/bin/dpkg-query failed (exit status 1)."
        );
    }
}
