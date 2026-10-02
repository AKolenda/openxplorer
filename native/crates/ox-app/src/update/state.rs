// SPDX-License-Identifier: AGPL-3.0-only
//! What the app knows about updates, and how the Software updates dialog,
//! the status bar and the About settings say it.
//!
//! Ports the states of `updatesDialog` in `v2.0.0:desktop/ui/app.js`: its
//! `release`, `checking` and `installed` variables and every status line,
//! word for word. Beyond the Python app, the state is shared by every
//! window, so the status bar and About show a release found by a check
//! in any window, and a build that cannot install updates itself says
//! what updates it instead ([`Installation::install_refusal`]).

use ox_core::update::{
    InstallProgress, Installation, ReleaseVersion, UpdateCheck, UpdateError, UpdateStatus,
};

/// A release newer than the running build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AvailableUpdate {
    /// The running version.
    pub(crate) current: ReleaseVersion,
    /// The release's version.
    pub(crate) latest: ReleaseVersion,
    /// How this build was installed, which decides whether it may install
    /// the release itself.
    pub(crate) installation: Installation,
}

/// What the app knows about updates, in every window.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum UpdateState {
    /// Nobody checked since the app started. Browsing never checks
    /// (UPD-001).
    #[default]
    NotChecked,
    /// GitHub is being asked.
    Checking,
    /// The running build is the latest release.
    UpToDate {
        /// The running version.
        current: ReleaseVersion,
    },
    /// A newer release exists.
    Available(AvailableUpdate),
    /// The check failed; the message is the reason.
    CheckFailed {
        /// Why, in the updater's words.
        reason: String,
    },
    /// An installation runs; `step` is its latest progress report.
    Installing {
        /// The version being installed.
        version: ReleaseVersion,
        /// The step it reached, once it reports one.
        step: Option<InstallProgress>,
    },
    /// The installation finished; the app must restart to use it.
    Installed {
        /// The version installed.
        version: ReleaseVersion,
    },
    /// The installation failed.
    InstallFailed {
        /// Why, in the updater's words.
        reason: String,
        /// It may have replaced files anyway, so the app must restart.
        needs_restart: bool,
    },
    /// An earlier installation waits for a restart; GitHub was not asked.
    RestartPending,
    /// Restart now failed.
    RestartFailed {
        /// Why, in the updater's words.
        reason: String,
    },
}

impl UpdateState {
    /// The state after a check that answered `result`.
    pub(crate) fn after_check(result: Result<UpdateCheck, UpdateError>, installation: Installation) -> Self {
        match result {
            Ok(UpdateCheck::Checked(status)) => Self::after_status(&status, installation),
            Ok(UpdateCheck::RestartPending { .. }) => Self::RestartPending,
            Err(error) => Self::CheckFailed {
                reason: error.to_string(),
            },
        }
    }

    /// The state a successful check's `status` describes.
    fn after_status(status: &UpdateStatus, installation: Installation) -> Self {
        if status.restart_required {
            return Self::RestartPending;
        }
        if !status.is_available {
            return Self::UpToDate {
                current: status.current_version,
            };
        }
        Self::Available(AvailableUpdate {
            current: status.current_version,
            latest: status.latest_version,
            installation,
        })
    }

    /// Whether a check or an installation runs, which disables Check
    /// again, Install update… and Restart now (`busy` in `updatesDialog`).
    pub(crate) fn is_busy(&self) -> bool {
        matches!(self, Self::Checking | Self::Installing { .. })
    }

    /// Whether an installation runs, which locks the application (UPD-005).
    pub(crate) fn is_installing(&self) -> bool {
        matches!(self, Self::Installing { .. })
    }

    /// Whether an installation changed the app's files, so only Restart
    /// now is offered (`installed` in `updatesDialog`).
    pub(crate) fn needs_restart(&self) -> bool {
        match self {
            Self::Installed { .. } | Self::RestartPending | Self::RestartFailed { .. } => true,
            Self::InstallFailed { needs_restart, .. } => *needs_restart,
            _ => false,
        }
    }

    /// The release that may be installed, when there is one.
    pub(crate) fn available(&self) -> Option<AvailableUpdate> {
        match self {
            Self::Available(update) => Some(*update),
            _ => None,
        }
    }

    /// The versions line of the dialog: "Installed: 1.1.4", and "·
    /// Available: 1.2.0" when a newer release exists.
    pub(crate) fn versions_text(&self, running: ReleaseVersion) -> String {
        match self {
            Self::Available(update) => ox_core::i18n::format_message(
                "Installed: {current} · Available: {latest}",
                &[
                    ("current", &(update.current).to_string()),
                    ("latest", &(update.latest).to_string()),
                ],
            ),
            Self::UpToDate { current } => {
                ox_core::i18n::format_message("Installed: {current}", &[("current", &(current).to_string())])
            }
            _ => {
                ox_core::i18n::format_message("Installed: {running}", &[("running", &(running).to_string())])
            }
        }
    }

    /// The dialog's status line (`#update-status`).
    ///
    /// Every text is `updatesDialog`'s, which is one table of cases, so
    /// the match is longer than a function should be.
    pub(crate) fn status_text(&self) -> String {
        match self {
            Self::NotChecked | Self::Checking => "Checking for updates…".to_owned(),
            Self::UpToDate { .. } => "OpenXplorer is up to date.".to_owned(),
            Self::Available(update) if update.installation.can_install() => {
                "An update is available.".to_owned()
            }
            Self::Available(_) => "An update is available. Automatic installation is unavailable.".to_owned(),
            Self::CheckFailed { reason } => ox_core::i18n::format_message(
                "Could not check for updates. {reason}",
                &[("reason", &(reason).to_string())],
            ),
            Self::Installing { step: None, .. } => {
                "Preparing update. Approve the administrator prompt to install.".to_owned()
            }
            Self::Installing { step: Some(step), .. } => step.to_string(),
            Self::Installed { version } => ox_core::i18n::format_message(
                "OpenXplorer {version} is installed. Restart to use the update.",
                &[("version", &(version).to_string())],
            ),
            Self::InstallFailed {
                reason,
                needs_restart,
            } => {
                let recovery = if *needs_restart {
                    " Some application files changed. Restart OpenXplorer before continuing."
                } else {
                    " Check again to retry."
                };
                ox_core::i18n::format_message(
                    "Update installation did not complete. {reason}{recovery}",
                    &[
                        ("reason", &(reason).to_string()),
                        ("recovery", &(recovery).to_string()),
                    ],
                )
            }
            Self::RestartPending => "Restart OpenXplorer to finish updating.".to_owned(),
            Self::RestartFailed { reason } => ox_core::i18n::format_message(
                "Could not restart. {reason}",
                &[("reason", &(reason).to_string())],
            ),
        }
    }

    /// What updates this build instead, under the status line, when a
    /// release is available that the app cannot install itself: for a
    /// Flatpak, GNOME Software or `flatpak update`.
    pub(crate) fn installation_hint(&self) -> Option<&'static str> {
        let update = self.available()?;
        let installation = update.installation;
        (!installation.can_install()).then(|| installation.install_refusal())
    }

    /// The status bar's word on updates: `None` while there is nothing to
    /// act on, else the line its "Check for updates" button adds to its
    /// tooltip.
    pub(crate) fn status_bar_notice(&self) -> Option<String> {
        match self {
            Self::Available(update) => Some(ox_core::i18n::format_message(
                "OpenXplorer {latest} is available.",
                &[("latest", &(update.latest).to_string())],
            )),
            _ if self.needs_restart() => Some("Restart OpenXplorer to finish updating.".to_owned()),
            _ => None,
        }
    }

    /// The description of the Check for updates row in About.
    pub(crate) fn about_summary(&self, running: ReleaseVersion) -> String {
        match self {
            Self::NotChecked => ox_core::i18n::format_message(
                "Installed: {running}. Look for a newer OpenXplorer release.",
                &[("running", &(running).to_string())],
            ),
            Self::Available(update) => ox_core::i18n::format_message(
                "OpenXplorer {latest} is available. Installed: {current}.",
                &[
                    ("latest", &(update.latest).to_string()),
                    ("current", &(update.current).to_string()),
                ],
            ),
            other => format!("{} {}", other.versions_text(running), other.status_text()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUNNING: ReleaseVersion = ReleaseVersion::new(1, 1, 4);
    const LATEST: ReleaseVersion = ReleaseVersion::new(1, 2, 0);

    fn available(installation: Installation) -> UpdateState {
        UpdateState::Available(AvailableUpdate {
            current: RUNNING,
            latest: LATEST,
            installation,
        })
    }

    fn checked(is_available: bool, restart_required: bool) -> UpdateStatus {
        UpdateStatus {
            current_version: RUNNING,
            latest_version: if is_available { LATEST } else { RUNNING },
            is_available,
            notes: String::new(),
            release_url: String::new(),
            can_install: true,
            restart_required,
        }
    }

    /// A state and the status line the dialog shows for it.
    struct StatusCase {
        state: UpdateState,
        status: &'static str,
    }

    /// Ported from the status lines of `updatesDialog` in
    /// `v2.0.0:desktop/ui/app.js`.
    ///
    /// parity: UPD-001, UPD-006
    #[test]
    fn every_state_has_the_python_dialogs_status_line() {
        let cases = [
            StatusCase {
                state: UpdateState::Checking,
                status: "Checking for updates…",
            },
            StatusCase {
                state: UpdateState::UpToDate { current: RUNNING },
                status: "OpenXplorer is up to date.",
            },
            StatusCase {
                state: available(Installation::DebianPackage),
                status: "An update is available.",
            },
            StatusCase {
                state: available(Installation::Flatpak),
                status: "An update is available. Automatic installation is unavailable.",
            },
            StatusCase {
                state: UpdateState::CheckFailed {
                    reason: "Could not reach GitHub. Check your connection and try again.".to_owned(),
                },
                status: "Could not check for updates. Could not reach GitHub. Check your connection and \
                         try again.",
            },
            StatusCase {
                state: UpdateState::Installing {
                    version: LATEST,
                    step: None,
                },
                status: "Preparing update. Approve the administrator prompt to install.",
            },
            StatusCase {
                state: UpdateState::Installing {
                    version: LATEST,
                    step: Some(InstallProgress::Downloading(LATEST)),
                },
                status: "Downloading OpenXplorer 1.2.0…",
            },
            StatusCase {
                state: UpdateState::Installed { version: LATEST },
                status: "OpenXplorer 1.2.0 is installed. Restart to use the update.",
            },
            StatusCase {
                state: UpdateState::InstallFailed {
                    reason: "Installation was cancelled or failed. denied".to_owned(),
                    needs_restart: false,
                },
                status: "Update installation did not complete. Installation was cancelled or failed. \
                         denied Check again to retry.",
            },
            StatusCase {
                state: UpdateState::InstallFailed {
                    reason: "Installation was cancelled or failed. denied".to_owned(),
                    needs_restart: true,
                },
                status: "Update installation did not complete. Installation was cancelled or failed. \
                         denied Some application files changed. Restart OpenXplorer before continuing.",
            },
            StatusCase {
                state: UpdateState::RestartPending,
                status: "Restart OpenXplorer to finish updating.",
            },
            StatusCase {
                state: UpdateState::RestartFailed {
                    reason: "No installed update is waiting for restart.".to_owned(),
                },
                status: "Could not restart. No installed update is waiting for restart.",
            },
        ];
        for case in cases {
            assert_eq!(case.state.status_text(), case.status, "{:?}", case.state);
        }
    }

    /// parity: UPD-001
    #[test]
    fn the_versions_line_names_the_available_release() {
        assert_eq!(
            available(Installation::DebianPackage).versions_text(RUNNING),
            "Installed: 1.1.4 · Available: 1.2.0"
        );
        assert_eq!(UpdateState::Checking.versions_text(RUNNING), "Installed: 1.1.4");
    }

    #[test]
    fn a_check_becomes_up_to_date_available_or_a_pending_restart() {
        let installation = Installation::DebianPackage;
        let up_to_date =
            UpdateState::after_check(Ok(UpdateCheck::Checked(checked(false, false))), installation);
        assert_eq!(up_to_date, UpdateState::UpToDate { current: RUNNING });
        let newer = UpdateState::after_check(Ok(UpdateCheck::Checked(checked(true, false))), installation);
        assert_eq!(newer, available(installation));
        let installed = UpdateState::after_check(Ok(UpdateCheck::Checked(checked(true, true))), installation);
        assert_eq!(installed, UpdateState::RestartPending);
        let pending = UpdateCheck::RestartPending {
            installed_version: Some(LATEST),
        };
        assert_eq!(
            UpdateState::after_check(Ok(pending), installation),
            UpdateState::RestartPending
        );
    }

    /// A Flatpak never installs updates itself; the dialog says to use
    /// Flatpak instead.
    ///
    /// parity: UPD-004
    #[test]
    fn a_build_that_cannot_install_says_what_updates_it() {
        let flatpak = available(Installation::Flatpak);
        let hint = flatpak.installation_hint().expect("a Flatpak explains itself");
        assert!(hint.contains("flatpak update"), "{hint}");
        let source = available(Installation::Unpackaged);
        assert_eq!(
            source.installation_hint(),
            Some(
                "In-app installation requires the installed Debian package and polkit. Use GitHub \
                 Releases for this build."
            )
        );
        assert_eq!(available(Installation::DebianPackage).installation_hint(), None);
    }

    #[test]
    fn only_a_restart_is_offered_once_files_changed() {
        assert!(UpdateState::Installed { version: LATEST }.needs_restart());
        assert!(UpdateState::RestartPending.needs_restart());
        let failed_cleanly = UpdateState::InstallFailed {
            reason: String::new(),
            needs_restart: false,
        };
        assert!(!failed_cleanly.needs_restart());
        assert!(!available(Installation::DebianPackage).needs_restart());
    }

    #[test]
    fn the_status_bar_speaks_only_when_there_is_something_to_do() {
        assert_eq!(UpdateState::NotChecked.status_bar_notice(), None);
        assert_eq!(
            UpdateState::UpToDate { current: RUNNING }.status_bar_notice(),
            None
        );
        assert_eq!(
            available(Installation::DebianPackage).status_bar_notice(),
            Some("OpenXplorer 1.2.0 is available.".to_owned())
        );
        assert_eq!(
            UpdateState::Installed { version: LATEST }.status_bar_notice(),
            Some("Restart OpenXplorer to finish updating.".to_owned())
        );
    }
}
