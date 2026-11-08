// SPDX-License-Identifier: AGPL-3.0-only
//! Items whose names hold a line break or bytes that are not UTF-8, which
//! another program made: Linux allows any byte in a name but `/` and NUL.
//! The app refuses such characters only in what is typed (an address, a
//! new name); an item already listed is renamed, copied and moved to the
//! Recycle Bin like any other, as Windows Explorer handles every item it
//! lists.

mod ops_support;
#[path = "ops_support/private_trash.rs"]
mod private_trash;

use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use ox_core::ops::{rename_item, run_transfer, OperationContext, TransferRequest};
use ox_core::transfer::{ConflictPolicy, TransferMode};

use ops_support::{block_on, file_uri};
use private_trash::require_private_trash;

/// The odd names: a line break, and Latin-1 bytes that are not UTF-8.
fn odd_names() -> [&'static OsStr; 2] {
    [
        OsStr::new("Line\nbreak.txt"),
        OsStr::from_bytes(b"Caf\xe9 menu.txt"),
    ]
}

/// A file named `name` in `folder`, holding `name`'s bytes.
fn odd_file(folder: &Path, name: &OsStr) -> PathBuf {
    let path = folder.join(name);
    fs::write(&path, name.as_bytes()).expect("an odd file");
    path
}

/// Runs a transfer of `paths`, failing the test on any error.
fn transfer(mode: TransferMode, paths: &[&Path], destination: Option<&Path>) {
    let request = TransferRequest {
        mode,
        uris: paths.iter().map(|path| file_uri(path)).collect(),
        destination_folder: destination.map(file_uri),
        policy: ConflictPolicy::Skip,
    };
    let outcome = block_on(run_transfer(&request, &OperationContext::default(), |_| {}))
        .unwrap_or_else(|error| panic!("{mode:?} was refused: {error}"));
    assert!(outcome.result.errors.is_empty(), "{:?}", outcome.result.errors);
    assert_eq!(outcome.result.done.len(), paths.len(), "{:?}", outcome.result);
}

/// A listed item with an odd name is renamed.
///
/// parity: OPS-008, OPS-035
#[test]
fn an_item_with_an_odd_name_is_renamed() {
    let temp = tempfile::tempdir().unwrap();
    for (number, name) in odd_names().into_iter().enumerate() {
        let path = odd_file(temp.path(), name);
        let new_name = format!("Renamed {number}.txt");
        let renamed = block_on(rename_item(
            &file_uri(&path),
            &new_name,
            &OperationContext::default(),
        ));
        assert!(renamed.is_ok(), "{name:?}: {renamed:?}");
        assert!(!path.exists(), "{name:?}");
        assert_eq!(fs::read(temp.path().join(&new_name)).unwrap(), name.as_bytes());
    }
}

/// A listed item with an odd name is copied, keeping its name.
///
/// parity: OPS-035
#[test]
fn an_item_with_an_odd_name_is_copied() {
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let names = odd_names();
    let paths: Vec<PathBuf> = names.iter().map(|name| odd_file(source.path(), name)).collect();
    let borrowed: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();

    transfer(TransferMode::Copy, &borrowed, Some(destination.path()));

    for name in names {
        assert_eq!(fs::read(destination.path().join(name)).unwrap(), name.as_bytes());
        assert!(source.path().join(name).exists(), "the original stays");
    }
}

/// Lets one test at a time use the shared Recycle Bin.
static RECYCLE_BIN: Mutex<()> = Mutex::new(());

/// A listed item with an odd name goes to the Recycle Bin.
///
/// parity: OPS-018
#[test]
fn an_item_with_an_odd_name_goes_to_the_recycle_bin() {
    require_private_trash();
    let _bin = RECYCLE_BIN.lock().unwrap_or_else(PoisonError::into_inner);
    let temp = tempfile::tempdir().unwrap();
    let names = odd_names();
    let paths: Vec<PathBuf> = names.iter().map(|name| odd_file(temp.path(), name)).collect();
    let borrowed: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();

    transfer(TransferMode::Trash, &borrowed, None);

    for path in &paths {
        assert!(!path.exists(), "{path:?} is still there");
    }
}
