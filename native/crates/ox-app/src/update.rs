// SPDX-License-Identifier: AGPL-3.0-only
//! The app's own updater, as the windows use it: "Check for updates",
//! the Software updates dialog, the update notice of the status bar and
//! About, and the launch guard behind `--restart`.
//!
//! Ports `updatesDialog` in `desktop/ui/app.js`, the update branches of
//! `dispatch` and `main` in `desktop/winspace.py`, and the launch checks of
//! `desktop/runtime_guard.py`. Checking, downloading, verifying and
//! installing are ox-core's port of `desktop/updater.py`
//! ([`ox_core::update`]): the same GitHub endpoints (the original
//! repository and the openxplorer organisation), the same installer name,
//! size and SHA-256 checks and the same `pkexec apt-get` installation.
//! This module adds only what the interface needs, so there is one update
//! mechanism.
//!
//! [`Updates`] is shared by every window of the application: a release
//! found in one window shows in the status bar of all of them, and while
//! an update installs every window is locked (UPD-005).
//!
//! | Module | Responsibility |
//! |---|---|
//! | `state` | [`UpdateState`] and its texts |
//! | `service` | The [`UpdateService`] of this executable |
//! | `dialog` | [`UpdateDialog`], "Software updates" |
//! | `launch_guard` | `--restart`, `--version` and the outdated-instance check |

mod dialog;
mod launch_guard;
mod service;
mod state;
#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::transfer::Cancellation;
use ox_core::update::{
    Activity, AppRequest, Confirmation, InstallRequest, ReleaseVersion, UpdatePhase, UpdateService,
};

pub(crate) use dialog::UpdateDialog;
pub(crate) use launch_guard::{LaunchCheck, RuntimeInfo};
pub(crate) use state::UpdateState;

/// Emitted whenever [`Updates::state`] changes.
const STATE_CHANGED: &str = "state-changed";

mod imp {
    use std::cell::{OnceCell, RefCell};
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::OnceLock;

    use gtk::glib;
    use gtk::glib::subclass::Signal;
    use gtk::subclass::prelude::*;
    use ox_core::update::UpdateService;

    use super::{UpdateState, STATE_CHANGED};

    /// Private state of [`super::Updates`].
    #[derive(Debug, Default)]
    pub(crate) struct Updates {
        /// This build's executable, for the service made on first use.
        pub(super) executable: OnceCell<PathBuf>,
        /// The update service, made when it is first needed: making it
        /// reads the whole executable.
        pub(super) service: OnceCell<Rc<UpdateService>>,
        /// What the app knows about updates now.
        pub(super) state: RefCell<UpdateState>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Updates {
        const NAME: &'static str = "OxUpdates";
        type Type = super::Updates;
    }

    impl ObjectImpl for Updates {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![Signal::builder(STATE_CHANGED).build()])
        }
    }
}

glib::wrapper! {
    /// The application's updates: its state, and the checks,
    /// installations and restarts the windows start.
    pub(crate) struct Updates(ObjectSubclass<imp::Updates>);
}

impl Updates {
    /// The updates of the running build.
    pub(crate) fn for_this_build() -> Self {
        Self::for_executable(service::this_executable())
    }

    /// The updates of the build whose executable is `executable`.
    fn for_executable(executable: PathBuf) -> Self {
        let updates: Self = glib::Object::new();
        updates
            .imp()
            .executable
            .set(executable)
            .expect("a new Updates has no executable yet");
        updates
    }

    /// Updates through `service`, for tests with a simulated GitHub and
    /// package manager.
    #[cfg(test)]
    pub(crate) fn with_service(service: UpdateService) -> Self {
        let updates: Self = glib::Object::new();
        updates
            .imp()
            .service
            .set(Rc::new(service))
            .expect("a new Updates has no service yet");
        updates
    }

    /// The version of the running build.
    pub(crate) fn running_version() -> ReleaseVersion {
        service::running_version()
    }

    /// What the app knows about updates now.
    pub(crate) fn state(&self) -> UpdateState {
        self.imp().state.borrow().clone()
    }

    /// Calls `on_change` whenever [`Self::state`] changes.
    pub(crate) fn connect_state_changed(&self, on_change: impl Fn(&Self) + 'static) -> glib::SignalHandlerId {
        self.connect_local(STATE_CHANGED, false, move |values| {
            let updates = values[0]
                .get::<Self>()
                .expect("the signal is emitted by an Updates object");
            on_change(&updates);
            None
        })
    }

    fn set_state(&self, state: UpdateState) {
        self.imp().state.replace(state);
        self.emit_by_name::<()>(STATE_CHANGED, &[]);
    }

    /// Asks GitHub for the latest release, unless a check or an
    /// installation runs already. Only the user starts a check: browsing
    /// and search never do (UPD-001).
    pub(crate) fn check(&self) {
        if self.state().is_busy() {
            return;
        }
        self.set_state(UpdateState::Checking);
        glib::spawn_future_local(glib::clone!(
            #[strong(rename_to = updates)]
            self,
            async move {
                let service = updates.service().await;
                let result = service.check(Cancellation::new()).await;
                updates.set_state(UpdateState::after_check(result, service.installation()));
            }
        ));
    }

    /// Installs the release the last check found, after the user
    /// confirmed with "Install update…". `activity` says whether any
    /// window has work running, which refuses the installation (UPD-005).
    pub(crate) fn install(&self, activity: Activity) {
        let Some(update) = self.state().available() else {
            return;
        };
        let version = update.latest;
        self.set_state(UpdateState::Installing { version, step: None });
        glib::spawn_future_local(glib::clone!(
            #[strong(rename_to = updates)]
            self,
            async move {
                let service = updates.service().await;
                let request = InstallRequest {
                    version,
                    confirmation: Confirmation::Confirmed,
                    activity,
                    cancel: Cancellation::new(),
                };
                let report = |step| {
                    let step = Some(step);
                    updates.set_state(UpdateState::Installing { version, step });
                };
                let result = service.install(request, report).await;
                updates.set_state(state_after_installation(result, &service, version));
            }
        ));
    }

    /// Restart now: starts the installed launcher, which asks this
    /// instance to quit safely. `writes` says whether any window writes
    /// files, which refuses the restart (UPD-007).
    pub(crate) fn restart(&self, writes: Activity) {
        let Some(service) = self.imp().service.get() else {
            return;
        };
        if let Err(error) = service.restart(writes) {
            let reason = error.to_string();
            self.set_state(UpdateState::RestartFailed { reason });
        }
    }

    /// Why a window may not close now, if it may not: an update is
    /// installing (UPD-005).
    pub(crate) fn close_refusal(&self) -> Option<String> {
        let service = self.imp().service.get()?;
        service
            .check_close_window()
            .err()
            .map(|refusal| refusal.to_string())
    }

    /// Why the application may not quit now, if it may not.
    pub(crate) fn quit_refusal(&self) -> Option<String> {
        let service = self.imp().service.get()?;
        service.check_quit().err().map(|refusal| refusal.to_string())
    }

    /// Why a window may not write files now, if it may not: an update
    /// installed files and waits for the restart (UPD-006). The running
    /// binary is unaffected, so browsing goes on; writes wait, as every
    /// file request did in the Python app.
    pub(crate) fn file_refusal(&self) -> Option<String> {
        let service = self.imp().service.get()?;
        service
            .check_request(AppRequest::Files)
            .err()
            .map(|refusal| refusal.to_string())
    }

    /// Why another window may not open now, if it may not: an update is
    /// installing or waits for its restart (UPD-005, UPD-006).
    pub(crate) fn new_window_refusal(&self) -> Option<String> {
        let service = self.imp().service.get()?;
        service
            .check_new_window()
            .err()
            .map(|refusal| refusal.to_string())
    }

    /// The update service, made on first use.
    async fn service(&self) -> Rc<UpdateService> {
        let imp = self.imp();
        if let Some(service) = imp.service.get() {
            return Rc::clone(service);
        }
        let executable = imp
            .executable
            .get()
            .cloned()
            .expect("an Updates without a service was made for an executable");
        let made = service::for_executable(executable).await;
        // Two first uses may both make one; the first one made is kept.
        Rc::clone(imp.service.get_or_init(|| Rc::new(made)))
    }
}

/// The state after an installation of `version` that ended with `result`.
///
/// Ported from the `catch` of "Install update…" in `updatesDialog`: after
/// a failure the Python dialog checked again to learn whether files
/// changed; the service knows that already ([`UpdatePhase`]).
fn state_after_installation(
    result: Result<(), ox_core::update::UpdateError>,
    service: &UpdateService,
    version: ReleaseVersion,
) -> UpdateState {
    match result {
        Ok(()) => UpdateState::Installed { version },
        Err(error) => UpdateState::InstallFailed {
            reason: error.to_string(),
            needs_restart: service.phase() == UpdatePhase::RestartRequired,
        },
    }
}
