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
//! | `shared` | Helpers the `gio_node` test binary uses too |
//! | `uri`, `fifo` | Test-file URIs and named pipes, shared with the `ops_*` tests (`tests/common/`) |

pub mod device;
pub mod faults;
#[path = "../common/fifo.rs"]
mod fifo;
pub mod local;
pub mod mtp_device;
mod shared;
#[path = "../common/uri.rs"]
mod uri;
pub mod versions;

use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ox_core::transfer::{
    is_own_staging_name, Cancellation, ConflictPolicy, Node, NodeKind, Operation, Progress, TransferEngine,
    TransferError, TransferResult,
};

pub use fifo::make_fifo;
use local::{LocalNode, Provider};
pub use shared::{gio_engine, mode_of, set_mode, RestoreOwnerAccess};
pub use uri::file_uri;

/// What a test run asks the engine to do. Copies and moves go into the
/// fixture's destination folder unless the variant names another folder;
/// Trash and permanent delete take neither a folder nor a policy.
#[derive(Debug, Clone, Copy)]
pub enum Request<'a> {
    /// Copy into the fixture's destination folder.
    Copy(ConflictPolicy),
    /// Move into the fixture's destination folder.
    Move(ConflictPolicy),
    /// Copy into the given folder.
    CopyInto(&'a Path, ConflictPolicy),
    /// Move into the given folder.
    MoveInto(&'a Path, ConflictPolicy),
    /// Move to the Trash.
    Trash,
    /// Delete permanently.
    Delete,
}

impl<'a> Request<'a> {
    /// The folder this request names, when it is not the fixture's
    /// destination folder.
    fn named_folder(self) -> Option<&'a Path> {
        match self {
            Request::CopyInto(folder, _) | Request::MoveInto(folder, _) => Some(folder),
            Request::Copy(_) | Request::Move(_) | Request::Trash | Request::Delete => None,
        }
    }

    /// The engine operation; copies and moves go into the folder at
    /// `destination_folder`, a URI.
    fn to_operation(self, destination_folder: &str) -> Operation<'_> {
        match self {
            Request::Copy(policy) | Request::CopyInto(_, policy) => Operation::Copy {
                destination_folder,
                policy,
            },
            Request::Move(policy) | Request::MoveInto(_, policy) => Operation::Move {
                destination_folder,
                policy,
            },
            Request::Trash => Operation::Trash,
            Request::Delete => Operation::Delete,
        }
    }
}

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

    /// Runs `request` over the items at `paths` with `engine`.
    ///
    /// # Errors
    ///
    /// The engine's refusal of the whole run, before any item is started.
    pub fn try_run(
        &self,
        engine: &mut TransferEngine,
        paths: &[&Path],
        request: Request<'_>,
    ) -> Result<TransferResult, TransferError> {
        let uris: Vec<String> = paths.iter().map(|path| file_uri(path)).collect();
        let folder = request.named_folder().unwrap_or(&self.destination_folder);
        let folder_uri = file_uri(folder);
        engine.run(request.to_operation(&folder_uri), &uris, &self.cancel)
    }

    /// A source `name` holding `incoming` and an existing item `name` in the
    /// destination folder holding `existing`; returns the source's path.
    pub fn replacement_source(&self, name: &str, incoming: &str, existing: &str) -> PathBuf {
        let source = self.source_folder.join(name);
        write(&source, incoming);
        write(&self.destination_folder.join(name), existing);
        source
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
        self.run(&mut engine, paths, Request::Copy(policy))
    }

    /// Like [`Fixture::try_run`] for runs that must not be refused.
    pub fn run(&self, engine: &mut TransferEngine, paths: &[&Path], request: Request<'_>) -> TransferResult {
        self.try_run(engine, paths, request).expect("the run is accepted")
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

/// True when `path` exists, including a dangling link (Python's
/// `os.path.lexists`).
pub fn exists_without_following_links(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// `count` random bytes from the kernel, so a copy cannot pass by accident.
pub fn random_bytes(count: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; count];
    fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .expect("read /dev/urandom");
    bytes
}

/// The file inside a folder made by [`create_source`].
pub const INNER_FILE: &str = "inner";

/// Creates the source of a test that runs for a file and for a folder: a
/// file at `path` holding `content`, or a folder at `path` whose file
/// [`INNER_FILE`] holds it.
///
/// # Panics
///
/// For a `kind` other than a file or a folder.
pub fn create_source(path: &Path, kind: NodeKind, content: &str) {
    match kind {
        NodeKind::File => write(path, content),
        NodeKind::Directory => {
            fs::create_dir(path).expect("create the source folder");
            write(&path.join(INNER_FILE), content);
        }
        NodeKind::Symlink | NodeKind::Special => {
            panic!("a test source is a file or a folder, not {kind:?}")
        }
    }
}

/// True for the progress a copy reports while it transfers a file's bytes
/// (`Copying a.txt · 8,192 / 35,000 bytes`), as opposed to the per-item
/// progress of the batch.
pub fn is_byte_progress(progress: &Progress) -> bool {
    progress.label.starts_with("Copying ")
}

/// Progress that cancels the run at the first byte progress, like the
/// user pressing Cancel while a file is being copied.
pub fn cancel_at_first_byte_progress(cancel: Cancellation) -> impl FnMut(Progress) + Send + 'static {
    move |progress| {
        if is_byte_progress(&progress) {
            cancel.cancel();
        }
    }
}

/// Progress that makes another program take `final_name` at the first
/// byte progress, so publishing the copy finds its final name taken.
pub fn take_name_while_copying(final_name: PathBuf) -> impl FnMut(Progress) + Send + 'static {
    move |progress| {
        if is_byte_progress(&progress) && !exists_without_following_links(&final_name) {
            write(&final_name, "another program");
        }
    }
}
