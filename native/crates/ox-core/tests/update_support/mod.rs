// SPDX-License-Identifier: AGPL-3.0-only
//! Doubles shared by the updater tests: a GitHub that answers from
//! fixtures and a package manager that checks and records its commands.
//! Ports the fixtures of `desktop/tests/test_updater.py` (`release`,
//! `open_fixture`, `run_fixture` and `assert_idle_and_clean`). No network
//! connection, administrator prompt or package installation is made.

#![allow(dead_code, reason = "each test file uses a different part of the doubles")]

pub mod service;

use std::cell::RefCell;
use std::io::{self, Cursor, Read};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};

use ox_core::transfer::Cancellation;
use ox_core::update::{
    installer_name, CommandOutput, Confirmation, FetchError, InstallProgress, Installation, PackageCommand,
    PackageManager, ReleaseServer, ReleaseVersion, TrustedUrl, UpdateError, Updater, UpdaterParts,
    LATEST_RELEASE_URL, REPOSITORY,
};
use serde_json::{json, Value};

/// The running version in the fixtures.
pub const CURRENT: ReleaseVersion = ReleaseVersion::new(1, 0, 0);
/// The version the fixture release publishes.
pub const NEXT: ReleaseVersion = ReleaseVersion::new(1, 0, 1);
/// The fixture installer's bytes.
pub const PACKAGE: &[u8] = b"Fictional package bytes. Never an executable Debian package.\n";

/// Python's `release()`: GitHub's answer for a stable release of `version`
/// in `repository`, with the fixture installer.
pub fn release_answer(version: ReleaseVersion, repository: &str) -> Value {
    let name = installer_name(version);
    let digest = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, PACKAGE).expect("SHA-256");
    json!({
        "tag_name": format!("v{version}"),
        "draft": false,
        "prerelease": false,
        "body": "Fictional release notes.",
        "assets": [{
            "name": name,
            "browser_download_url": format!("{repository}/releases/download/v{version}/{name}"),
            "digest": format!("sha256:{digest}"),
            "size": PACKAGE.len(),
        }],
    })
}

/// The fixture release of [`NEXT`] in the original repository.
pub fn next_release() -> Value {
    release_answer(NEXT, REPOSITORY)
}

/// The fixture installer's download address.
pub fn next_installer_url() -> String {
    let name = installer_name(NEXT);
    format!("{REPOSITORY}/releases/download/v{NEXT}/{name}")
}

/// How the fixture GitHub answers one address.
#[derive(Debug, Clone)]
pub enum Response {
    /// These bytes.
    Body(Vec<u8>),
    /// These bytes, then a broken connection.
    BrokenAfter(Vec<u8>),
    /// A failed fetch.
    Refused(FetchError),
}

impl Response {
    /// `value` as JSON text.
    pub fn json(value: &Value) -> Self {
        Self::Body(value.to_string().into_bytes())
    }
}

/// A GitHub that answers the release endpoint and the installer address
/// from fixtures and records every address asked. Clones share state.
#[derive(Debug, Clone)]
pub struct FixtureServer {
    state: Arc<Mutex<ServerState>>,
}

#[derive(Debug)]
struct ServerState {
    latest: Response,
    installer: Response,
    opened: Vec<String>,
    pause: Option<Pause>,
}

/// Holds the next fetch until the test lets it go on.
#[derive(Debug)]
struct Pause {
    /// Told when the fetch has started.
    started: Sender<()>,
    /// Waited on before the fetch answers.
    resume: Receiver<()>,
}

impl FixtureServer {
    /// Answers with [`next_release`] and [`PACKAGE`].
    pub fn new() -> Self {
        let state = ServerState {
            latest: Response::json(&next_release()),
            installer: Response::Body(PACKAGE.to_vec()),
            opened: Vec::new(),
            pause: None,
        };
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    /// Answers the release endpoint with `response` from now on.
    pub fn answer_release(&self, response: Response) {
        self.lock().latest = response;
    }

    /// Answers the installer address with `response` from now on.
    pub fn answer_installer(&self, response: Response) {
        self.lock().installer = response;
    }

    /// Holds the next fetch: it tells `started`, then waits for `resume`.
    pub fn pause_next_open(&self, started: Sender<()>, resume: Receiver<()>) {
        self.lock().pause = Some(Pause { started, resume });
    }

    /// Every address asked so far.
    pub fn opened(&self) -> Vec<String> {
        self.lock().opened.clone()
    }

    fn lock(&self) -> MutexGuard<'_, ServerState> {
        self.state
            .lock()
            .expect("no fixture panics while holding its state")
    }
}

impl ReleaseServer for FixtureServer {
    fn open(&self, url: &TrustedUrl, _cancel: &Cancellation) -> Result<Box<dyn Read>, FetchError> {
        let pause = self.lock().pause.take();
        if let Some(pause) = pause {
            pause.started.send(()).expect("the test waits for the fetch");
            pause.resume.recv().expect("the test lets the fetch go on");
        }
        let mut state = self.lock();
        state.opened.push(url.as_str().to_owned());
        let response = if url.as_str() == LATEST_RELEASE_URL {
            state.latest.clone()
        } else {
            assert_eq!(
                url.as_str(),
                next_installer_url(),
                "only the checked installer is fetched"
            );
            state.installer.clone()
        };
        match response {
            Response::Body(bytes) => Ok(Box::new(Cursor::new(bytes))),
            Response::BrokenAfter(bytes) => Ok(Box::new(Cursor::new(bytes).chain(BrokenConnection))),
            Response::Refused(error) => Err(error),
        }
    }
}

/// A connection that breaks on the first read.
struct BrokenConnection;

impl Read for BrokenConnection {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::ConnectionReset,
            "Fictional disconnect",
        ))
    }
}

/// Runs before the fixture `pkexec apt-get install` answers, for example to
/// replace installed files or to wait.
pub type InstallHook = Arc<dyn Fn() + Send + Sync>;

/// A package manager that checks each command as Python's `run_fixture`
/// does, records it and answers from fixtures. Clones share state.
#[derive(Clone)]
pub struct PackageFixture {
    state: Arc<Mutex<PackageState>>,
}

struct PackageState {
    updates_folder: PathBuf,
    commands: Vec<PackageCommand>,
    inspection: CommandOutput,
    installation: CommandOutput,
    query: CommandOutput,
    install_hook: Option<InstallHook>,
}

impl PackageFixture {
    /// Answers like a successful installation of [`NEXT`] from a download
    /// in `updates_folder`.
    pub fn new(updates_folder: &Path) -> Self {
        let state = PackageState {
            updates_folder: updates_folder.to_path_buf(),
            commands: Vec::new(),
            inspection: success(&format!(
                "Package: openxplorer\nVersion: {NEXT}\nArchitecture: all\n"
            )),
            installation: success(""),
            query: success(&format!("install ok installed\n{NEXT}")),
            install_hook: None,
        };
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    /// Answers `dpkg-deb` with `output` from now on.
    pub fn answer_inspection(&self, output: CommandOutput) {
        self.lock().inspection = output;
    }

    /// Answers `pkexec apt-get install` with `output` from now on.
    pub fn answer_installation(&self, output: CommandOutput) {
        self.lock().installation = output;
    }

    /// Answers `dpkg-query` with `output` from now on.
    pub fn answer_query(&self, output: CommandOutput) {
        self.lock().query = output;
    }

    /// Runs `hook` whenever the installation command runs.
    pub fn on_install(&self, hook: InstallHook) {
        self.lock().install_hook = Some(hook);
    }

    /// Every command run so far.
    pub fn commands(&self) -> Vec<PackageCommand> {
        self.lock().commands.clone()
    }

    /// The programs run so far, in order.
    pub fn programs(&self) -> Vec<&'static str> {
        self.commands().iter().map(PackageCommand::program).collect()
    }

    /// Every installer path a command named.
    pub fn installer_paths(&self) -> Vec<PathBuf> {
        let named = self.commands().into_iter().filter_map(|command| match command {
            PackageCommand::InspectInstaller(path) | PackageCommand::Install(path) => Some(path),
            PackageCommand::QueryInstalled => None,
        });
        named.collect()
    }

    fn lock(&self) -> MutexGuard<'_, PackageState> {
        self.state
            .lock()
            .expect("no fixture panics while holding its state")
    }
}

impl PackageManager for PackageFixture {
    fn run(&self, command: &PackageCommand) -> Result<CommandOutput, UpdateError> {
        let mut state = self.lock();
        state.commands.push(command.clone());
        match command {
            PackageCommand::InspectInstaller(installer) => {
                assert_private_download(installer, &state.updates_folder);
                assert!(command.time_limit().is_some(), "dpkg-deb has a time limit");
                Ok(state.inspection.clone())
            }
            PackageCommand::Install(installer) => {
                assert!(installer.is_file(), "the verified installer is still there");
                assert_eq!(command.time_limit(), None, "APT is never interrupted");
                let hook = state.install_hook.clone();
                let answer = state.installation.clone();
                drop(state);
                if let Some(hook) = hook {
                    hook();
                }
                Ok(answer)
            }
            PackageCommand::QueryInstalled => Ok(state.query.clone()),
        }
    }
}

/// `run_fixture`'s checks of the installer `dpkg-deb` is asked about.
fn assert_private_download(installer: &Path, updates_folder: &Path) {
    let folder = installer.parent().expect("the installer is in a folder");
    assert_eq!(std::fs::read(installer).expect("the installer exists"), PACKAGE);
    assert_eq!(
        installer.file_name().unwrap().to_str(),
        Some(installer_name(NEXT).as_str())
    );
    assert_eq!(folder.parent(), Some(updates_folder));
    assert_eq!(mode(installer), 0o600);
    assert_eq!(mode(folder), 0o700);
}

/// A successful command that printed `stdout`.
pub fn success(stdout: &str) -> CommandOutput {
    CommandOutput {
        exit_status: 0,
        stdout: stdout.to_owned(),
        stderr: String::new(),
    }
}

/// A command that failed with `exit_status` and printed `stderr`.
pub fn failure(exit_status: i32, stderr: &str) -> CommandOutput {
    CommandOutput {
        exit_status,
        stdout: String::new(),
        stderr: stderr.to_owned(),
    }
}

/// The permission bits of `path`.
pub fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).expect("the path exists").mode() & 0o777
}

/// Installs `version` with the user's confirmation, collecting the
/// progress messages.
pub fn install(updater: &Updater, version: ReleaseVersion) -> (Result<(), UpdateError>, Vec<String>) {
    let progress = RefCell::new(Vec::new());
    let record = |step: InstallProgress| progress.borrow_mut().push(step.to_string());
    let result = updater.install(version, Confirmation::Confirmed, &record, &Cancellation::new());
    (result, progress.into_inner())
}

/// An updater of the packaged build of [`CURRENT`] with the doubles, and
/// the folder its downloads go to.
pub struct UpdaterFixture {
    /// Holds the updates folder.
    pub root: tempfile::TempDir,
    /// Where downloads go.
    pub updates_folder: PathBuf,
    /// The fixture GitHub.
    pub server: FixtureServer,
    /// The fixture package manager.
    pub packages: PackageFixture,
}

impl UpdaterFixture {
    /// Doubles that answer a successful update to [`NEXT`].
    pub fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix("openxplorer-updater-test-")
            .tempdir()
            .expect("a temporary folder");
        let updates_folder = root.path().join("updates");
        let packages = PackageFixture::new(&updates_folder);
        Self {
            root,
            updates_folder,
            server: FixtureServer::new(),
            packages,
        }
    }

    /// An updater of an `installation` build made of these doubles.
    pub fn updater(&self, installation: Installation) -> Updater {
        Updater::new(UpdaterParts {
            current_version: CURRENT,
            installation,
            updates_folder: self.updates_folder.clone(),
            server: Box::new(self.server.clone()),
            package_manager: Box::new(self.packages.clone()),
        })
    }

    /// A packaged updater whose check found [`NEXT`].
    pub fn checked_updater(&self) -> Updater {
        let updater = self.updater(Installation::DebianPackage);
        updater
            .check(&Cancellation::new())
            .expect("the fixture release is valid");
        updater
    }

    /// Python's `assert_idle_and_clean`: no download is left behind and
    /// every installer a command named is gone. That `updater` is idle is
    /// checked with a task that takes its lock and then fails at once.
    pub fn assert_idle_and_clean(&self, updater: &Updater) {
        let probe = probe_task(updater);
        assert!(
            matches!(probe, Err(UpdateError::NotChecked)),
            "the updater is idle: {probe:?}"
        );
        if self.updates_folder.exists() {
            let left_over = std::fs::read_dir(&self.updates_folder).unwrap().count();
            assert_eq!(left_over, 0, "no download is left behind");
        }
        for installer in self.packages.installer_paths() {
            assert!(!installer.exists(), "{} was deleted", installer.display());
        }
    }
}

/// Takes the updater's task lock and fails at once, with
/// [`UpdateError::NotChecked`] when the updater is idle and
/// [`UpdateError::TaskRunning`] while another task runs.
fn probe_task(updater: &Updater) -> Result<(), UpdateError> {
    let never_checked = ReleaseVersion::new(0, 0, 0);
    updater.install(
        never_checked,
        Confirmation::Confirmed,
        &|_| {},
        &Cancellation::new(),
    )
}
