// SPDX-License-Identifier: AGPL-3.0-only
//! Application updates: checking GitHub for a newer release, installing it
//! through the system package manager, restarting into it, and noticing a
//! running instance whose files an upgrade replaced.
//!
//! Ports `v2.0.0:desktop/updater.py`, `v2.0.0:desktop/runtime_guard.py` and the update
//! rules of `v2.0.0:desktop/winspace.py`. Nothing here depends on GTK; the
//! interface calls [`UpdateService`] from the main thread, which runs the
//! blocking work on GIO worker threads.
//!
//! The safety rules, each documented where it is enforced:
//!
//! - Only fixed upstream HTTPS endpoints are contacted, and redirects must
//!   stay on them ([`TrustedUrl`]).
//! - A release is used only if it is stable and publishes the exact
//!   installer of the original repository or the openxplorer
//!   organisation, with a SHA-256 digest and a size of 1 byte to 100 MiB
//!   ([`parse_release`]).
//! - Installing needs the user's confirmation and a check for exactly that
//!   version; the download is private, verified by size, digest and
//!   `dpkg-deb`, and always deleted ([`Updater::install`]).
//! - Where the package installed the update helper, the app hands it only
//!   the version; the helper fetches, verifies and installs that release
//!   as root under its own polkit action, which lets an administrator at
//!   the machine update without a password while "Install updates without
//!   asking for a password" is on, as it is by default
//!   ([`install_update_command`], [`disable_passwordless`]).
//! - Only the packaged build installs; Flatpak and other packages update
//!   through their own package manager ([`Installation`]).
//! - APT is never interrupted ([`PackageCommand::time_limit`]), and the
//!   application stays locked until the installation's worker has ended,
//!   even if nobody awaits it any more ([`UpdateService::install`]).
//! - While an update installs, and until the restart after it, windows
//!   leave files alone ([`UpdateService::check_request`]).
//! - A restart runs only the fixed launcher ([`RESTART_COMMAND`]), and a
//!   running instance is asked to quit, never killed ([`InstanceGuard`]).
//!
//! GitHub's asset digest verifies the download; it is not an independent
//! publisher signature.
//!
//! | Module | Responsibility | Ports |
//! |---|---|---|
//! | `version` | [`ReleaseVersion`] | `updater.py` |
//! | `trust` | [`TrustedUrl`] and the fixed endpoints | `updater.py` |
//! | `release` | [`parse_release`]: GitHub's answer, checked | `updater.py` |
//! | `server` | [`ReleaseServer`] and the bounded release answer | `updater.py` |
//! | `github` | [`GitHubReleases`], the libsoup client | `updater.py` |
//! | `installation` | [`Installation`]: how this build was installed | `updater.py` |
//! | `download` | The private, verified download | `updater.py` |
//! | `package` | [`PackageCommand`] and what its output must say | `updater.py` |
//! | `install_helper` | [`install_update_command`]: the root helper | new |
//! | `helper_report` | [`HelperReport`]: what the helper tells the app | new |
//! | `passwordless` | [`PasswordlessUpdates`]: the setting, on by default, that skips the password | new |
//! | `process` | Running package tools and the launcher | `updater.py`, `winspace.py` |
//! | `updater` | [`Updater`]: one check or installation at a time | `updater.py` |
//! | `service` | [`UpdateService`]: the application-wide rules | `winspace.py` |
//! | `restart` | [`RESTART_COMMAND`] and [`RestartLauncher`] | `winspace.py` |
//! | `identity` | [`RuntimeIdentity`] of a build | `runtime_guard.py` |
//! | `instance` | [`InstanceGuard`]: an outdated running instance | `runtime_guard.py` |
//! | `session_bus` | [`SessionBus`], the guard's D-Bus calls | `runtime_guard.py` |
//! | `diagnosis` | [`Diagnosis`], the `--diagnose` report | `winspace.py` |
//! | `error` | [`UpdateError`] and [`InstanceError`] | all of them |

mod diagnosis;
mod download;
mod error;
mod github;
mod helper_report;
mod identity;
mod install_helper;
mod installation;
mod instance;
mod package;
mod passwordless;
mod process;
mod release;
mod restart;
mod server;
mod service;
mod session_bus;
mod trust;
mod updater;
mod version;

pub use diagnosis::{AssociationStatus, Associations, Diagnosis, ShowInFolder};
pub use error::{InstanceError, UpdateError};
pub use github::GitHubReleases;
pub use helper_report::{HelperFailure, HelperReport};
pub use identity::{InstalledBuild, RuntimeIdentity, RUNTIME_PROTOCOL};
pub use install_helper::{
    install_as_root, install_update_command, RootInstallParts, ENVIRONMENT, INSTALL_HELPER_COMMAND,
    STAGING_FOLDER,
};
pub use installation::Installation;
pub use instance::{InstanceBus, InstanceGuard, InstanceStatus, LaunchMode, StopTiming};
pub use package::{CommandOutput, PackageCommand, PackageManager};
pub use passwordless::{
    disable_passwordless, enable_passwordless, PasswordlessError, PasswordlessUpdates, RuleFiles,
    DISABLE_PASSWORDLESS, ENABLE_PASSWORDLESS, PASSWORDLESS_RECORD, PASSWORDLESS_RULE, RULE_TEMPLATE,
};
pub use process::SystemPackageManager;
pub use release::{
    installer_name, parse_release, Installer, Release, Sha256Digest, MAX_INSTALLER_SIZE, MAX_NOTES_CHARS,
};
pub use restart::{RestartLauncher, SessionLauncher, RESTART_COMMAND};
pub use server::{FetchError, ReleaseServer};
pub use service::{
    Activity, AppRequest, InstallRequest, UpdateCheck, UpdatePhase, UpdateService, UpdateServiceParts,
};
pub use session_bus::{application_object_path, SessionBus, QUIT_ACTION, RUNTIME_INFO_ACTION};
pub use trust::{TrustedUrl, LATEST_RELEASE_URL, RELEASE_REPOSITORIES, REPOSITORY, TRUSTED_HOSTS};
pub use updater::{Confirmation, InstallProgress, UpdateStatus, Updater, UpdaterParts};
pub use version::ReleaseVersion;
