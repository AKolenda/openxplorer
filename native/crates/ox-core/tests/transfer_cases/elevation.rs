// SPDX-License-Identifier: AGPL-3.0-only
//! Continue in "Destination Folder Access Denied" (OPS-047): the item runs
//! again into the destination folder as administrator.
//!
//! The double stands `GVfs`' `admin://` in with a second name for the
//! destination folder, a link beside it: the provider refuses every write
//! by the destination's own path, as the kernel refuses an ordinary user,
//! and lets writes through the link's path, as `gvfsd-admin` writes as
//! root. Moving between the two names is not native, as moving from
//! `file:` to `admin://` is not.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ox_core::transfer::{
    Cancellation, ConflictPolicy, FailedItem, FailureAnswer, FailureCause, MoveByCopyingItem, Node,
    TransferEngine, TransferError,
};

use crate::transfer_support::{
    local::{self, LocalNode, Provider},
    *,
};

/// Refuses writes by the path of `denied`, the destination folder.
struct DeniedDestination {
    denied: PathBuf,
    /// The administrator's name for it.
    elevated: PathBuf,
    /// Only files are refused, so a folder's own staging is made and the
    /// question is about an entry inside it.
    files_only: bool,
}

impl DeniedDestination {
    /// Refuses `path` when it is in the denied folder by its own name.
    fn check(&self, path: &Path) -> Result<(), TransferError> {
        if path.starts_with(&self.denied) {
            return Err(TransferError::PermissionDenied(format!(
                "Error opening {}: Permission denied",
                path.display()
            )));
        }
        Ok(())
    }
}

impl Provider for DeniedDestination {
    fn create_directory(&self, node: &LocalNode, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        if !self.files_only {
            self.check(node.local_path())?;
        }
        node.local_create_directory(cancel)
    }

    fn copy_file(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        self.check(&local::local_path_of(target))?;
        node.local_copy_file(target, cancel, progress)
    }

    fn move_native(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
    ) -> Result<(), TransferError> {
        let target_path = local::local_path_of(target);
        self.check(&target_path)?;
        // `file:` to `admin://` is another backend: no native move.
        if target_path.starts_with(&self.elevated) && !node.local_path().starts_with(&self.elevated) {
            return Err(TransferError::NotSupported("Operation not supported".into()));
        }
        node.local_move_native(target, cancel)
    }
}

/// A fixture whose destination may only be written through `elevated`, a
/// link to it, and an engine over it that answers each question with the
/// next of `answers` (Skip once they run out) and logs the questions.
struct Elevation {
    fixture: Fixture,
    elevated: PathBuf,
    asked: Arc<Mutex<Vec<FailedItem>>>,
    elevations: Arc<Mutex<Vec<String>>>,
}

impl Elevation {
    fn new() -> Self {
        let fixture = Fixture::new();
        let elevated = fixture.root.join("administrator view");
        symlink(&fixture.destination_folder, &elevated).expect("link the administrator view");
        Self {
            fixture,
            elevated,
            asked: Arc::default(),
            elevations: Arc::default(),
        }
    }

    /// An engine answering with `answers` in turn, whose elevation maps
    /// the destination to the administrator view, or refuses it with
    /// `refusal`.
    fn engine(&self, answers: &[FailureAnswer], refusal: Option<&str>) -> TransferEngine {
        self.engine_over(answers, refusal, false)
    }

    /// [`Self::engine`], refusing only files when `files_only`.
    fn engine_over(
        &self,
        answers: &[FailureAnswer],
        refusal: Option<&str>,
        files_only: bool,
    ) -> TransferEngine {
        let provider = Arc::new(DeniedDestination {
            denied: self.fixture.destination_folder.clone(),
            elevated: self.elevated.clone(),
            files_only,
        });
        let answers = Mutex::new(answers.to_vec());
        let asked = Arc::clone(&self.asked);
        let elevations = Arc::clone(&self.elevations);
        let destination = file_uri(&self.fixture.destination_folder);
        let elevated = file_uri(&self.elevated);
        let refusal = refusal.map(str::to_owned);
        self.fixture
            .engine(provider)
            .with_failure_question(move |item: &FailedItem| {
                asked.lock().expect("question log").push(item.clone());
                let mut answers = answers.lock().expect("answers");
                if answers.is_empty() {
                    FailureAnswer::Skip
                } else {
                    answers.remove(0)
                }
            })
            .with_move_by_copying_question(|_: &MoveByCopyingItem| {
                panic!("Continue already agreed to finish the move by copying")
            })
            .with_elevation(move |folder: &str| {
                elevations.lock().expect("elevation log").push(folder.to_owned());
                assert_eq!(folder, destination, "the folder the user chose is elevated");
                match &refusal {
                    Some(refusal) => Err(TransferError::failed(refusal.clone())),
                    None => Ok(elevated.clone()),
                }
            })
    }

    fn asked(&self) -> Vec<FailedItem> {
        self.asked.lock().expect("question log").clone()
    }

    fn elevations(&self) -> usize {
        self.elevations.lock().expect("elevation log").len()
    }
}

/// Continue copies the item into the folder as administrator. The
/// question named the folder the user chose, and nothing is reported
/// afterwards.
///
/// parity: OPS-047
#[test]
fn continue_copies_the_item_as_administrator() {
    let test = Elevation::new();
    let fixture = &test.fixture;
    let source = fixture.source_folder.join("setup.exe");
    write(&source, "installer");
    let mut engine = test.engine(&[FailureAnswer::Elevate], None);

    let result = fixture.run(&mut engine, &[&source], Request::Copy(ConflictPolicy::Skip));

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(read(&fixture.destination_folder.join("setup.exe")), "installer");
    assert_eq!(read(&source), "installer");
    let asked = test.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].cause, FailureCause::AccessDenied);
    assert_eq!(
        asked[0].destination_folder.as_deref(),
        Some(file_uri(&fixture.destination_folder).as_str())
    );
    fixture.assert_no_staging();
}

/// A move finishes as administrator by copying, without asking "Move by
/// copying?" again, and removes the original.
///
/// parity: OPS-047, XFER-013
#[test]
fn continue_moves_the_item_as_administrator() {
    let test = Elevation::new();
    let fixture = &test.fixture;
    let source = fixture.source_folder.join("Tools");
    fs::create_dir(&source).expect("a folder");
    write(&source.join("tool.cfg"), "settings");
    let mut engine = test.engine(&[FailureAnswer::Elevate], None);

    let result = fixture.run(&mut engine, &[&source], Request::Move(ConflictPolicy::Skip));

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(
        read(&fixture.destination_folder.join("Tools/tool.cfg")),
        "settings"
    );
    assert!(!source.exists(), "the original is removed");
    assert_eq!(test.asked().len(), 1);
}

/// Continue answers for this item only; with "Do this for all current
/// items" every later item goes straight to the administrator's folder
/// without asking, and the folder is checked once.
///
/// parity: OPS-047
#[test]
fn continue_for_all_writes_every_later_item_as_administrator() {
    for (answer, questions, elevations) in [(FailureAnswer::Elevate, 3, 3), (FailureAnswer::ElevateAll, 1, 1)]
    {
        let test = Elevation::new();
        let fixture = &test.fixture;
        let names = ["one.txt", "two.txt", "three.txt"];
        let sources: Vec<PathBuf> = names
            .iter()
            .map(|name| {
                let source = fixture.source_folder.join(name);
                write(&source, name);
                source
            })
            .collect();
        let paths: Vec<&Path> = sources.iter().map(PathBuf::as_path).collect();
        let mut engine = test.engine(&[answer; 3], None);

        let result = fixture.run(&mut engine, &paths, Request::Copy(ConflictPolicy::Skip));

        assert!(result.errors.is_empty(), "{answer:?}: {result:?}");
        for name in names {
            assert_eq!(read(&fixture.destination_folder.join(name)), name, "{answer:?}");
        }
        assert_eq!(test.asked().len(), questions, "{answer:?}");
        assert_eq!(test.elevations(), elevations, "{answer:?}");
    }
}

/// A destination that may not be written as administrator is refused
/// before anything is written there, and the refusal is asked about, so
/// the user sees why; Skip then leaves the item out.
///
/// parity: OPS-047, XFER-015
#[test]
fn a_refused_elevation_is_asked_about_and_writes_nothing() {
    let test = Elevation::new();
    let fixture = &test.fixture;
    let source = fixture.source_folder.join("setup.exe");
    write(&source, "installer");
    let refusal = "Other users can change the folder.";
    let mut engine = test.engine(&[FailureAnswer::Elevate, FailureAnswer::Skip], Some(refusal));

    let result = fixture.run(&mut engine, &[&source], Request::Copy(ConflictPolicy::Skip));

    assert!(list(&fixture.destination_folder).is_empty());
    let asked = test.asked();
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[1].cause, FailureCause::Other);
    assert_eq!(asked[1].error, refusal);
    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert_eq!(result.presented, result.errors, "the user saw it");
}

/// Continue answered about an entry inside a folder runs the whole folder
/// again as administrator, without asking a second time about the item.
///
/// parity: OPS-047
#[test]
fn continue_about_an_entry_runs_the_whole_item_again() {
    let test = Elevation::new();
    let fixture = &test.fixture;
    let source = fixture.source_folder.join("Album");
    fs::create_dir(&source).expect("a folder");
    write(&source.join("cover.png"), "cover");
    write(&source.join("track.ogg"), "track");
    let mut engine = test.engine_over(&[FailureAnswer::Elevate], None, true);

    let result = fixture.run(&mut engine, &[&source], Request::Copy(ConflictPolicy::Skip));

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(read(&fixture.destination_folder.join("Album/cover.png")), "cover");
    assert_eq!(read(&fixture.destination_folder.join("Album/track.ogg")), "track");
    let asked = test.asked();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(asked[0].name.starts_with("Album/"), "about the entry: {asked:?}");
    fixture.assert_no_staging();
}

/// Written as administrator, the write guard (XFER-020) still refuses a
/// protected item where the user sees it: a later item of Continue for
/// all goes straight to the administrator's folder and is checked by its
/// own URI there.
///
/// parity: OPS-047, XFER-020
#[test]
fn the_write_guard_still_refuses_items_written_as_administrator() {
    let test = Elevation::new();
    let fixture = &test.fixture;
    let one = fixture.source_folder.join("one.txt");
    let two = fixture.source_folder.join("two.txt");
    write(&one, "one");
    write(&two, "two");
    let protected = file_uri(&fixture.destination_folder.join("two.txt"));
    let mut engine = test
        .engine(&[FailureAnswer::ElevateAll], None)
        .with_write_guard(move |uri: &str| {
            if uri == protected {
                Err(TransferError::failed("Previous-version locations are read-only."))
            } else {
                Ok(())
            }
        });

    let result = fixture.run(&mut engine, &[&one, &two], Request::Copy(ConflictPolicy::Skip));

    assert_eq!(read(&fixture.destination_folder.join("one.txt")), "one");
    assert!(!fixture.destination_folder.join("two.txt").exists(), "{result:?}");
    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert!(result.errors[0].contains("read-only"), "{result:?}");
}

/// A Replace written as administrator merges into an existing folder only
/// when the check passes for it: an existing folder another account could
/// change (group-writable, other-writable, sticky) is refused before
/// anything is written into it, and without a check every such merge is
/// refused. A folder the check accepts is merged.
///
/// parity: OPS-047, XFER-015
#[test]
fn a_replace_as_administrator_merges_only_into_folders_the_check_accepts() {
    use std::os::unix::fs::PermissionsExt;

    for (mode, installed, merged) in [
        (0o775, true, false),
        (0o757, true, false),
        (0o1777, true, false),
        (0o755, false, false),
        (0o755, true, true),
    ] {
        let test = Elevation::new();
        let fixture = &test.fixture;
        let source = fixture.source_folder.join("tmp");
        fs::create_dir(&source).expect("a folder");
        write(&source.join("x"), "incoming");
        let existing = fixture.destination_folder.join("tmp");
        fs::create_dir(&existing).expect("an existing folder");
        fs::set_permissions(&existing, fs::Permissions::from_mode(mode)).expect("set the mode");
        let checked = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&checked);
        let mut engine = test.engine(&[FailureAnswer::Elevate], None);
        if installed {
            engine = engine.with_elevated_merge_check(move |uri: &str| {
                log.lock().expect("check log").push(uri.to_owned());
                let path = local::path_from_uri(uri);
                let mode = fs::metadata(&path).expect("the folder").permissions().mode();
                if mode & 0o022 == 0 {
                    Ok(())
                } else {
                    Err(TransferError::failed("Other users can change the folder."))
                }
            });
        }

        let result = fixture.run(&mut engine, &[&source], Request::Copy(ConflictPolicy::Replace));

        let case = format!("{mode:o}, check installed: {installed}");
        assert_eq!(existing.join("x").exists(), merged, "{case}: {result:?}");
        assert_eq!(result.errors.is_empty(), merged, "{case}: {result:?}");
        if installed {
            let checked = checked.lock().expect("check log");
            assert!(
                checked.iter().any(|uri| uri.ends_with("/tmp")),
                "{case}: the existing folder was checked: {checked:?}"
            );
        }
        fs::set_permissions(&existing, fs::Permissions::from_mode(0o755)).expect("reset the mode");
    }
}
