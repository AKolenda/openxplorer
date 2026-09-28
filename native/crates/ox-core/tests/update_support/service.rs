// SPDX-License-Identifier: AGPL-3.0-only
//! An update service over the updater doubles, for `update_service.rs` and
//! `update_restart.rs`. Ports the doubles of `BridgeTests` in
//! `desktop/tests/test_updater.py`. The launcher double never starts the
//! real launcher.

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};

use ox_core::transfer::Cancellation;
use ox_core::update::{
    Activity, AppRequest, Confirmation, InstallRequest, Installation, InstalledBuild, RestartLauncher,
    RuntimeIdentity, UpdateCheck, UpdateError, UpdatePhase, UpdateService, UpdateServiceParts,
};

use super::{UpdaterFixture, NEXT};

/// Every request a window can make.
pub const ALL_REQUESTS: [AppRequest; 7] = [
    AppRequest::Files,
    AppRequest::UpdateCheck,
    AppRequest::UpdateInstall,
    AppRequest::UpdateRestart,
    AppRequest::Environment,
    AppRequest::Quit,
    AppRequest::WindowChrome,
];

/// Records the commands it was asked to start and starts nothing.
#[derive(Debug, Clone, Default)]
pub struct RecordingLauncher {
    launched: Rc<RefCell<Vec<Vec<String>>>>,
}

impl RestartLauncher for RecordingLauncher {
    fn launch(&self, argv: &[&str]) -> Result<(), UpdateError> {
        let command = argv.iter().map(|part| (*part).to_owned()).collect();
        self.launched.borrow_mut().push(command);
        Ok(())
    }
}

/// An update service over the updater doubles, whose installed build is
/// an executable file the fixture installation may replace.
pub struct ServiceFixture {
    /// The updater doubles.
    pub updates: UpdaterFixture,
    /// The installed build.
    pub executable: PathBuf,
    launcher: RecordingLauncher,
    /// The service under test.
    pub service: UpdateService,
    /// Where its futures run.
    pub context: glib::MainContext,
}

impl ServiceFixture {
    /// A service with no update in progress.
    pub fn new() -> Self {
        let updates = UpdaterFixture::new();
        let executable = updates.root.path().join("openxplorer");
        fs::write(&executable, b"Fictional build 1.0.0").unwrap();
        let running = RuntimeIdentity::of_executable(&executable, "1.0.0").unwrap();
        let launcher = RecordingLauncher::default();
        let service = UpdateService::new(UpdateServiceParts {
            updater: updates.updater(Installation::DebianPackage),
            running,
            installed_build: InstalledBuild::Executable {
                path: executable.clone(),
                version: "1.0.0".to_owned(),
            },
            launcher: Box::new(launcher.clone()),
        });
        Self {
            updates,
            executable,
            launcher,
            service,
            context: glib::MainContext::new(),
        }
    }

    /// A service whose check found [`NEXT`].
    pub fn checked() -> Self {
        let fixture = Self::new();
        let checked = fixture
            .context
            .block_on(fixture.service.check(Cancellation::new()));
        assert!(matches!(checked, Ok(UpdateCheck::Checked(_))));
        fixture
    }

    /// A service whose installation replaced the installed build.
    pub fn waiting_for_restart() -> Self {
        let fixture = Self::checked();
        fixture.replace_build_during_installation();
        fixture.install(confirmed(Activity::Idle)).unwrap();
        assert_eq!(fixture.service.phase(), UpdatePhase::RestartRequired);
        fixture
    }

    /// Makes the installation replace the installed executable, as a
    /// package upgrade does.
    pub fn replace_build_during_installation(&self) {
        let executable = self.executable.clone();
        let replace = move || fs::write(&executable, b"Fictional build 1.0.1").unwrap();
        self.updates.packages.on_install(Arc::new(replace));
    }

    /// Runs an installation to its end.
    pub fn install(&self, request: InstallRequest) -> Result<(), UpdateError> {
        self.context.block_on(self.service.install(request, |_| {}))
    }

    /// Runs an installation and calls `observe` while it is running.
    pub fn while_installing(&self, observe: impl FnOnce(&UpdateService)) -> Result<(), UpdateError> {
        let (release, released) = mpsc::channel::<()>();
        let released = Mutex::new(released);
        let wait = move || released.lock().unwrap().recv().unwrap();
        self.updates.packages.on_install(Arc::new(wait));
        self.context.block_on(async {
            let installation = self.service.install(confirmed(Activity::Idle), |_| {});
            let watch = async {
                observe(&self.service);
                release.send(()).unwrap();
            };
            futures_util::future::join(installation, watch).await.0
        })
    }

    /// Every command the launcher was asked to start.
    pub fn launched(&self) -> Vec<Vec<String>> {
        self.launcher.launched.borrow().clone()
    }
}

/// A confirmed installation of [`NEXT`].
pub fn confirmed(activity: Activity) -> InstallRequest {
    InstallRequest {
        version: NEXT,
        confirmation: Confirmation::Confirmed,
        activity,
        cancel: Cancellation::new(),
    }
}
