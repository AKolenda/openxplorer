// SPDX-License-Identifier: AGPL-3.0-only
//! What happens before the application starts: `--version`, `--check`,
//! `--diagnose`, `--quit`, `--restart`, and the check for a running
//! instance an upgrade left outdated; and the `runtime-info` action that
//! lets a later launch run that check against this process.
//!
//! Ports `main` and `confirm_restart` in `v2.0.0:desktop/winspace.py` and the
//! `runtime-info` action of `OpenXplorer.startup`; the guard itself is
//! ox-core's port of `v2.0.0:desktop/runtime_guard.py`
//! ([`InstanceGuard`]). Its safety rules hold here: a running instance
//! is asked to quit, never killed, and only after the user agreed or
//! asked for `--restart`.

use std::io::Write;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::update::{
    InstanceBus, InstanceError, InstanceGuard, InstanceStatus, LaunchMode, RuntimeIdentity, SessionBus,
    RUNTIME_INFO_ACTION,
};

use super::service::{running_identity, running_version, unknown_identity};
use crate::config::APP_ID;

/// The option that restarts the running instance into the installed build.
const RESTART_OPTION: &str = "--restart";
/// The option that prints the version.
const VERSION_OPTION: &str = "--version";
/// The option that names the libraries and the build.
const CHECK_OPTION: &str = "--check";
/// The option that prints the installed and running builds as JSON.
const DIAGNOSE_OPTION: &str = "--diagnose";
/// The option that asks the running instance to quit safely.
const QUIT_OPTION: &str = "--quit";
/// The option of the Show in folder service, which never asks anything.
const SERVICE_OPTION: &str = "--filemanager-service";

/// The options only the launcher handles, without their dashes, and what
/// `--help` says about each (`argument_parser` in `winspace.py`). The
/// application registers them too, so `--help` lists them, but they never
/// reach it: the guard exits or removes them first.
pub(crate) const LAUNCHER_OPTIONS: [(&str, &str); 4] = [
    ("check", "Check native libraries without opening a window"),
    (
        "restart",
        "Safely quit the current process and launch the installed build; never force active transfers",
    ),
    (
        "diagnose",
        "Print installed and running build identities without filenames or credentials",
    ),
    ("version", "Print the installed application version"),
];

/// The exit status of a launch the guard stopped, as in the Python app.
const GUARD_FAILURE: u8 = 3;

/// What a launch as root prints before it exits (`main` in winspace.py).
const RUN_AS_USER: &str =
    crate::i18n::message_id("Run OpenXplorer as your regular desktop user, not with sudo.");

/// The user ID of root.
const ROOT_USER_ID: u32 = 0;

/// What the launch does after the guard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LaunchCheck {
    /// Start the application with these arguments: the command line
    /// without `--restart`, which belongs to this launcher, not to the
    /// running instance it replaces.
    Continue(Vec<String>),
    /// Exit at once with this status.
    Exit(glib::ExitCode),
}

impl LaunchCheck {
    /// Runs the launch checks of `main` on `arguments`, the whole command
    /// line including the program: prints the version, asks the running
    /// instance to quit for `--quit` and `--restart`, and asks the user
    /// before replacing an outdated instance. Root is refused after
    /// `--version` and before everything else.
    pub(crate) fn run(arguments: Vec<String>) -> Self {
        Self::run_as(arguments, effective_user_id())
    }

    /// The root check alone, for a launch that skips the others (a
    /// snapshot): `Some` exit when the effective user is root.
    pub(crate) fn root_refusal() -> Option<glib::ExitCode> {
        refuse_root(effective_user_id())
    }

    /// [`Self::run`] for the effective user `user_id`.
    fn run_as(arguments: Vec<String>, user_id: u32) -> Self {
        let parsed = crate::arguments::Arguments::parse(&arguments);
        let has = |option: &str| parsed.has(option);
        if has(VERSION_OPTION) {
            println!("OpenXplorer {}", running_version());
            return Self::Exit(glib::ExitCode::SUCCESS);
        }
        if let Some(status) = refuse_root(user_id) {
            return Self::Exit(status);
        }
        if has(CHECK_OPTION) {
            println!("{}", super::report::check_report());
            return Self::Exit(glib::ExitCode::SUCCESS);
        }
        let outcome = if has(DIAGNOSE_OPTION) {
            super::report::diagnosis().map(|report| {
                println!("{}", report.to_json());
                Self::Exit(glib::ExitCode::SUCCESS)
            })
        } else if has(QUIT_OPTION) {
            quit_running_instance().map(|()| Self::Exit(glib::ExitCode::SUCCESS))
        } else {
            let mode = if has(RESTART_OPTION) {
                LaunchMode::Restart
            } else {
                LaunchMode::Normal
            };
            require_current(mode, has(SERVICE_OPTION)).map(|()| Self::Continue(without_restart(arguments)))
        };
        outcome.unwrap_or_else(|error| {
            eprintln!("{error}");
            let _ = std::io::stderr().flush();
            Self::Exit(glib::ExitCode::from(GUARD_FAILURE))
        })
    }
}

/// Safety rule "never run as root" (`os.geteuid()==0` in `main`): a file
/// manager running under sudo would create root-owned files in the user's
/// folders and bypass every permission. A launch whose effective user is
/// root says so and exits with status 1; only `--version` runs before
/// this check.
fn refuse_root(user_id: u32) -> Option<glib::ExitCode> {
    if user_id != ROOT_USER_ID {
        return None;
    }
    eprintln!("{RUN_AS_USER}");
    let _ = std::io::stderr().flush();
    Some(glib::ExitCode::FAILURE)
}

/// This process's effective user ID, which `GCredentials` records on
/// Linux; sudo makes it root's.
fn effective_user_id() -> u32 {
    gio::Credentials::new()
        .unix_user()
        .expect("GCredentials holds the effective user ID on Linux")
}

/// `arguments` without `--restart`.
fn without_restart(arguments: Vec<String>) -> Vec<String> {
    crate::arguments::without_option(arguments, RESTART_OPTION)
}

/// `--quit`: asks the running instance, if any, to quit safely and waits
/// until it has (`args.quit` in `main`).
fn quit_running_instance() -> Result<(), InstanceError> {
    let guard = InstanceGuard::new(SessionBus::connect(APP_ID)?);
    let Some(owner) = ox_core::update::InstanceBus::owner(guard.bus())? else {
        return Ok(());
    };
    guard.stop(&owner)
}

/// Replaces a running instance that is not this build, with the user's
/// consent; `--restart` replaces any running instance (`require_current`).
/// The Show in folder service never asks: it runs at login, unseen.
///
/// Reading this build's identity reads the whole executable, so it is
/// done only when another instance runs, and a normal launch skips it when
/// that instance runs this very file.
fn require_current(mode: LaunchMode, is_service: bool) -> Result<(), InstanceError> {
    let guard = InstanceGuard::new(SessionBus::connect(APP_ID)?);
    require_current_on(&guard, mode, is_service, this_identity)
}

/// [`require_current`] on `guard`'s bus, reading this build's identity
/// with `identity` when it is needed.
fn require_current_on<B: InstanceBus>(
    guard: &InstanceGuard<B>,
    mode: LaunchMode,
    is_service: bool,
    identity: impl FnOnce() -> RuntimeIdentity,
) -> Result<(), InstanceError> {
    let Some(owner) = guard.bus().owner()? else {
        return Ok(());
    };
    // The instance runs this very file, so this very build: a normal
    // launch goes on without reading tens of megabytes to learn it. While
    // a process runs its executable the file cannot be rewritten, and an
    // upgrade puts another file in its place ([`ExecutableFile`]).
    let runs_this_file = || {
        guard
            .bus()
            .reported_executable(&owner)
            .is_some_and(|running| super::service::running_file().as_ref() == Some(&running))
    };
    if mode == LaunchMode::Normal && runs_this_file() {
        return Ok(());
    }
    let installed = identity();
    let confirm: &dyn Fn(&InstanceStatus) -> bool = &confirm_restart;
    let confirm = (!is_service).then_some(confirm);
    guard.require_current(&installed, mode, confirm).map(drop)
}

/// The identity of this build, or one that matches no running instance
/// when the executable cannot be read.
pub(super) fn this_identity() -> RuntimeIdentity {
    let version = running_version();
    running_identity(version).unwrap_or_else(|_| unknown_identity(version))
}

/// Asks whether to restart the outdated instance `status` describes
/// (`confirm_restart` in `winspace.py`), before the application runs.
fn confirm_restart(status: &InstanceStatus) -> bool {
    if gtk::init().is_err() {
        return false;
    }
    let running = status
        .running
        .as_ref()
        .map_or("an older release", |identity| identity.version.as_str());
    let detail = ox_core::i18n::format_message("The installed version is {running_version}; the existing background process is {running}. A restart closes existing windows. File operations must finish first; they will not be force-stopped.", &[("running_version", &running_version().to_string()), ("running", running)]);
    let dialog = gtk::AlertDialog::builder()
        .message(ox_core::i18n::gettext_static(
            "Restart OpenXplorer to finish updating",
        ))
        .detail(detail)
        .buttons(["Not now", "Restart OpenXplorer"])
        .cancel_button(0)
        .default_button(1)
        .modal(true)
        .build();
    let answer = glib::MainContext::default().block_on(dialog.choose_future(None::<&gtk::Window>));
    answer == Ok(1)
}

/// The stateful `runtime-info` application action, whose state is this
/// build's identity as JSON (INT-022), naming the executable file the
/// process runs. A later launch reads it over D-Bus to tell whether this
/// process runs the installed build.
#[derive(Debug)]
pub(crate) struct RuntimeInfo;

impl RuntimeInfo {
    /// Adds the action to `app`. Its identity is read on a worker thread,
    /// as it reads the whole executable; until then the state has no
    /// digest, which matches no build.
    pub(crate) fn install(app: &impl IsA<gio::ActionMap>) {
        let version = running_version();
        // The file this process runs is named at once: a later launch of
        // that same file needs neither its digest nor to wait for it.
        let file = super::service::running_file();
        let state = unknown_identity(version).to_action_state_naming(file.as_ref());
        let action = gio::SimpleAction::new_stateful(RUNTIME_INFO_ACTION, None, &state.to_variant());
        app.add_action(&action);
        glib::spawn_future_local(async move {
            let identity = gio::spawn_blocking(move || running_identity(version)).await;
            if let Ok(Ok(identity)) = identity {
                action.set_state(&identity.to_action_state_naming(file.as_ref()).to_variant());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use ox_core::update::ExecutableFile;

    use super::*;

    fn arguments(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    /// The restart flag belongs to the fresh launcher, not the running
    /// instance's command line (`main` in `winspace.py`).
    ///
    /// parity: UPD-009
    #[test]
    fn the_restart_option_is_not_passed_on() {
        let passed = without_restart(arguments(&["openxplorer", "--restart", "/tmp"]));
        assert_eq!(passed, arguments(&["openxplorer", "/tmp"]));
        let passed = without_restart(arguments(&[
            "openxplorer",
            "--restart",
            "--",
            "--restart",
            "--quit",
            "--version",
        ]));
        assert_eq!(
            passed,
            arguments(&["openxplorer", "--", "--restart", "--quit", "--version"])
        );
    }

    /// Root is refused with status 1; any other user goes on to the
    /// running-instance checks.
    ///
    /// parity: SAFE-008
    #[test]
    fn a_launch_as_root_is_refused_with_status_1() {
        assert_eq!(refuse_root(ROOT_USER_ID), Some(glib::ExitCode::FAILURE));
        assert_eq!(refuse_root(1000), None);
        assert_eq!(
            LaunchCheck::run_as(arguments(&["openxplorer", "--", "--version"]), ROOT_USER_ID),
            LaunchCheck::Exit(glib::ExitCode::FAILURE),
            "a positional --version must not bypass the root check"
        );
        assert_eq!(
            LaunchCheck::run_as(arguments(&["openxplorer", "--version"]), ROOT_USER_ID),
            LaunchCheck::Exit(glib::ExitCode::SUCCESS),
            "--version runs before the root check"
        );
        assert_eq!(
            LaunchCheck::run_as(arguments(&["openxplorer", "--quit"]), ROOT_USER_ID),
            LaunchCheck::Exit(glib::ExitCode::FAILURE),
            "root is refused before --quit reaches the running instance"
        );
        assert_eq!(
            RUN_AS_USER,
            "Run OpenXplorer as your regular desktop user, not with sudo."
        );
    }

    /// A running instance answering as [`InstanceBus`] does, which says
    /// what it runs and notes whether it was asked to quit.
    struct RunningInstance {
        identity: RuntimeIdentity,
        file: Option<ExecutableFile>,
        asked_to_quit: std::cell::Cell<bool>,
    }

    impl InstanceBus for RunningInstance {
        fn owner(&self) -> Result<Option<String>, InstanceError> {
            Ok((!self.asked_to_quit.get()).then(|| ":1.7".to_owned()))
        }

        fn reported_identity(&self, _owner: &str) -> Option<RuntimeIdentity> {
            Some(self.identity.clone())
        }

        fn reported_executable(&self, _owner: &str) -> Option<ExecutableFile> {
            self.file.clone()
        }

        fn request_quit(&self, _owner: &str) -> Result<(), InstanceError> {
            self.asked_to_quit.set(true);
            Ok(())
        }
    }

    fn instance_running(file: Option<ExecutableFile>) -> InstanceGuard<RunningInstance> {
        InstanceGuard::new(RunningInstance {
            identity: running_identity(running_version()).expect("the test binary is readable"),
            file,
            asked_to_quit: std::cell::Cell::new(false),
        })
    }

    fn this_file() -> ExecutableFile {
        super::super::service::running_file().expect("the test binary can be looked at")
    }

    /// Another file, an instance that names none (an older release) and
    /// `--restart` still compare builds as before (UPD-008, UPD-009).
    ///
    /// parity: UPD-008, UPD-009
    #[test]
    fn a_launch_of_another_file_still_compares_builds() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let other = folder.path().join("openxplorer");
        std::fs::write(&other, b"another build").expect("a file can be written");
        let other = ExecutableFile::of_path(&other).expect("the file can be looked at");
        for running in [Some(other), None] {
            let guard = instance_running(running);
            let digest_read = std::cell::Cell::new(false);
            let outcome = require_current_on(&guard, LaunchMode::Normal, false, || {
                digest_read.set(true);
                this_identity()
            });
            assert!(outcome.is_ok(), "the same build runs: {outcome:?}");
            assert!(digest_read.get(), "another file is compared by its digest");
        }

        let guard = instance_running(Some(this_file()));
        let outcome = require_current_on(&guard, LaunchMode::Restart, false, this_identity);
        assert!(outcome.is_ok(), "{outcome:?}");
        assert!(
            guard.bus().asked_to_quit.get(),
            "--restart replaces even this build"
        );
    }

    /// parity: INT-022
    #[gtk::test]
    fn the_runtime_info_action_reports_this_builds_identity() {
        let actions = gio::SimpleActionGroup::new();
        RuntimeInfo::install(&actions);
        let expected = running_identity(running_version()).expect("the test binary is readable");
        crate::test_support::harness::wait_until("the identity to be read", || {
            let state = actions.action_state(RUNTIME_INFO_ACTION);
            let text = state.and_then(|state| state.get::<String>());
            text.and_then(|text| RuntimeIdentity::from_action_state(&text)) == Some(expected.clone())
        });
        let state = actions
            .action_state(RUNTIME_INFO_ACTION)
            .and_then(|state| state.get::<String>())
            .expect("the action has a text state");
        assert_eq!(
            ExecutableFile::from_action_state(&state),
            Some(this_file()),
            "the report still names the file once the digest is in"
        );
    }
}
