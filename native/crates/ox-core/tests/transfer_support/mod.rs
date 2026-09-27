// SPDX-License-Identifier: AGPL-3.0-only
//! Shared test doubles and fixtures for the `transfer_*` integration tests.
//!
//! | Module | Test double |
//! |---|---|
//! | `local` | Local files behind the [`Node`] contract, with overridable behaviour |
//! | `device` | A `GVfs` MTP destination built on `local` |
//! | `faults` | Local files that fail at a chosen step of a copy or replacement |
//! | `mtp_device` | A simulated phone behind real `mtp://` URIs for the production adapter |
//! | `versions` | The previous-version write guard |

pub mod device;
pub mod faults;
pub mod local;
pub mod mtp_device;
pub mod versions;

use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ox_core::gio_node::GioNode;
use ox_core::transfer::{
    is_own_staging_name, Cancellation, ConflictPolicy, Node, TransferEngine, TransferError, TransferMode,
    TransferResult,
};

use local::{file_uri, LocalNode, Provider};

/// A temporary source folder and destination folder, like `setUp` in the
/// Python transfer tests (`self.src` and `self.dst` there).
pub struct Fixture {
    /// Removes the temporary folder when the fixture is dropped.
    _temp: tempfile::TempDir,
    /// The temporary folder holding both of the others.
    pub root: PathBuf,
    /// The folder the test's sources are created in, called `source`.
    pub source_folder: PathBuf,
    /// The destination folder of copies and moves.
    pub destination_folder: PathBuf,
    /// The cancellation every run of this fixture uses.
    pub cancel: Cancellation,
    /// The cleanup delays the engine asked for, recorded instead of slept.
    sleeps: Arc<Mutex<Vec<Duration>>>,
}

impl Fixture {
    /// `source` and `destination` in a new temporary folder.
    pub fn new() -> Self {
        Self::with_destination("destination")
    }

    /// `source` and a destination folder called `name`.
    pub fn with_destination(name: &str) -> Self {
        let temp = tempfile::tempdir().expect("create a temporary folder");
        let root = temp.path().to_path_buf();
        let source_folder = root.join("source");
        let destination_folder = root.join(name);
        fs::create_dir(&source_folder).expect("create source");
        fs::create_dir(&destination_folder).expect("create destination");
        Self {
            _temp: temp,
            root,
            source_folder,
            destination_folder,
            cancel: Cancellation::new(),
            sleeps: Arc::default(),
        }
    }

    /// An engine over `provider` whose cleanup delays are recorded instead
    /// of slept.
    pub fn engine(&self, provider: Arc<dyn Provider>) -> TransferEngine {
        let sleeps = Arc::clone(&self.sleeps);
        TransferEngine::new(LocalNode::factory(provider))
            .with_sleep(move |delay| sleeps.lock().expect("sleep log").push(delay))
    }

    /// The delays the engine asked for, in seconds.
    pub fn sleeps(&self) -> Vec<f64> {
        let sleeps = self.sleeps.lock().expect("sleep log");
        sleeps.iter().map(Duration::as_secs_f64).collect()
    }

    /// Runs an operation into `target` (the destination folder by default).
    pub fn try_run(
        &self,
        engine: &mut TransferEngine,
        paths: &[&Path],
        mode: TransferMode,
        policy: ConflictPolicy,
        target: Option<&Path>,
    ) -> Result<TransferResult, TransferError> {
        let uris: Vec<String> = paths.iter().map(|path| file_uri(path)).collect();
        let target_uri = file_uri(target.unwrap_or(&self.destination_folder));
        engine.run(mode, &uris, Some(&target_uri), policy, &self.cancel)
    }

    /// Copies `paths` into the destination folder with a new engine over
    /// `provider`.
    pub fn copy(
        &self,
        provider: Arc<dyn Provider>,
        paths: &[&Path],
        policy: ConflictPolicy,
    ) -> TransferResult {
        let mut engine = self.engine(provider);
        self.run(&mut engine, paths, TransferMode::Copy, policy, None)
    }

    /// Like [`Fixture::try_run`] for runs that must not be refused.
    pub fn run(
        &self,
        engine: &mut TransferEngine,
        paths: &[&Path],
        mode: TransferMode,
        policy: ConflictPolicy,
        target: Option<&Path>,
    ) -> TransferResult {
        self.try_run(engine, paths, mode, policy, target)
            .expect("the run is accepted")
    }

    /// Asserts no `.winspace-transfer-*` staging is left in the destination
    /// folder (`no_stage` in the Python tests).
    pub fn assert_no_staging(&self) {
        let names = list(&self.destination_folder);
        let staged: Vec<&String> = names
            .iter()
            .filter(|name| name.starts_with(".winspace-transfer-"))
            .collect();
        assert!(staged.is_empty(), "staging left behind: {staged:?}");
    }

    /// Engine-made names anywhere below the destination folder, relative to
    /// it: staging, backups and payloads.
    pub fn leftovers(&self) -> Vec<String> {
        let mut found = Vec::new();
        collect_leftovers(&self.destination_folder, &self.destination_folder, &mut found);
        found.sort();
        found
    }
}

/// Adds the engine-made names in `folder` and below it to `found`, relative
/// to `root`.
fn collect_leftovers(root: &Path, folder: &Path, found: &mut Vec<String>) {
    for entry in fs::read_dir(folder).expect("list a folder") {
        let path = entry.expect("read an entry").path();
        let name = path.file_name().expect("entries have names").to_string_lossy();
        if name.starts_with(".winspace-") || name == "payload" {
            let relative = path.strip_prefix(root).expect("below the root");
            found.push(relative.display().to_string());
        }
        let metadata = fs::symlink_metadata(&path).expect("stat an entry");
        if metadata.is_dir() {
            collect_leftovers(root, &path, found);
        }
    }
}

/// An engine resolving every URI with the production [`GioNode`], without
/// a write guard.
pub fn gio_engine() -> TransferEngine {
    TransferEngine::new(Arc::new(|uri: &str| {
        Ok(Box::new(GioNode::new(uri)) as Box<dyn Node>)
    }))
}

/// Gives a folder and every folder below it owner access again when
/// dropped, so the temporary folder can be removed even after a test that
/// made folders read-only failed midway.
pub struct RestoreOwnerAccess(PathBuf);

impl RestoreOwnerAccess {
    /// Restores owner access to `folder` and below when dropped.
    pub fn new(folder: &Path) -> Self {
        Self(folder.to_path_buf())
    }
}

impl Drop for RestoreOwnerAccess {
    fn drop(&mut self) {
        restore_owner_access(&self.0);
    }
}

/// Makes `folder`, if it is a folder, and every folder below it owner-only
/// and writable.
fn restore_owner_access(folder: &Path) {
    let is_folder = fs::symlink_metadata(folder).is_ok_and(|metadata| metadata.is_dir());
    if !is_folder {
        return;
    }
    set_mode(folder, 0o700);
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        restore_owner_access(&entry.path());
    }
}

/// True for an item named like the engine's own staging
/// (`.winspace-transfer-<32 hex>.part`), like `is_own_staging_name` in the
/// Python tests.
pub fn is_staging(node: &dyn Node) -> bool {
    is_own_staging_name(&node.display_name())
}

/// True for a path whose last component is an engine staging name.
pub fn is_staging_path(path: &Path) -> bool {
    path.file_name()
        .and_then(OsStr::to_str)
        .is_some_and(is_own_staging_name)
}

/// True for an item named like the engine's replacement backups
/// (`.winspace-replaced-<32 hex>.backup`).
pub fn is_backup(node: &dyn Node) -> bool {
    node.display_name().starts_with(".winspace-replaced-")
}

/// The `file://` URI of `path`.
pub fn uri(path: &Path) -> String {
    file_uri(path)
}

/// Writes `text` to a new or existing file.
pub fn write(path: &Path, text: &str) {
    fs::write(path, text).expect("write a test file");
}

/// Reads a text file.
pub fn read(path: &Path) -> String {
    fs::read_to_string(path).expect("read a test file")
}

/// The sorted names in a folder.
pub fn list(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .expect("list a folder")
        .map(|entry| {
            entry
                .expect("read an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// `lexists`: true for a dangling link too.
pub fn lexists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// The permission bits of `path`, without following a link.
pub fn mode_of(path: &Path) -> u32 {
    fs::symlink_metadata(path).expect("stat").permissions().mode() & 0o7777
}

/// Sets the permission bits of `path`.
pub fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod");
}

/// `count` random bytes from the kernel, so a copy cannot pass by accident.
pub fn random_bytes(count: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; count];
    fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .expect("read /dev/urandom");
    bytes
}

/// Creates a named pipe (a special file) with the system `mkfifo`.
pub fn mkfifo(path: &Path) {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("run mkfifo");
    assert!(status.success(), "mkfifo failed");
}
