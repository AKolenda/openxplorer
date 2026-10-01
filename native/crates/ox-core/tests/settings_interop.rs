// SPDX-License-Identifier: AGPL-3.0-only
//! Cross-language checks of `settings.json` and its `flock` protocol
//! against `v2.0.0:desktop/core.py`.
//!
//! Each Python script gets the settings directory as `sys.argv[1]`. Every
//! file is inside a temporary directory; the user's settings are untouched.

mod python_support;

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use ox_core::settings::{BookmarkAction, BookmarkKind, BookmarkRequest, PreferencesUpdate, Settings, Theme};
use python_support::{python, run_python};
use serde_json::{json, Value};

/// Changes preferences, a share and a pin through the Python app's
/// `Settings`.
const PYTHON_CHANGES_SETTINGS: &str = r"
import sys
from pathlib import Path
from core import Settings
store = Settings(Path(sys.argv[1]))
store.update_preferences({'theme': 'dark', 'autoIndex': False, 'networkInterval': 300, 'columnWidths': {'name': 333.5}})
store.bookmark('add', 'share', 'smb://NAS/Team files', 'Shared work')
store.pin_many([{'uri': 'mtp://[usb:001,002]/Storage', 'label': 'Phone'}])
";

/// Reads the settings without a warning, changes the text size and prints
/// the resulting snapshot as JSON.
const PYTHON_CHANGES_TEXT_SIZE_AND_PRINTS: &str = r"
import json, sys
from pathlib import Path
from core import Settings
store = Settings(Path(sys.argv[1]))
assert not store.warning, store.warning
store.update_preferences({'textSize': 125})
print(json.dumps(store.snapshot()))
";

/// Tries to take `settings.lock` without waiting and prints `blocked` if
/// another process holds it.
const PYTHON_TRIES_THE_LOCK: &str = r"
import fcntl, sys
from pathlib import Path
with (Path(sys.argv[1]) / 'settings.lock').open('r+') as lock:
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        print('blocked')
    else:
        raise AssertionError('Python acquired a lock already held by Rust')
";

/// Takes `settings.lock`, prints `locked`, waits for a line on standard
/// input, then saves the dark theme while still holding the lock.
const PYTHON_SAVES_DARK_WHILE_LOCKED: &str = r"
import fcntl, sys
from pathlib import Path
from core import Settings
root = Path(sys.argv[1])
with (root / 'settings.lock').open('r+') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    print('locked', flush=True)
    sys.stdin.readline()
    store = Settings(root)
    store.data['preferences']['theme'] = 'dark'
    store.save()
";

/// parity: SET-014, SIDE-022
#[test]
fn python_and_rust_mutations_preserve_each_others_settings() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("winspace");
    run_python(PYTHON_CHANGES_SETTINGS, &[directory.as_path()]);

    let mut rust = Settings::open(&directory);
    assert!(rust.warning().is_none());
    assert_eq!(rust.data().preferences.theme, Theme::Dark);
    assert_eq!(rust.data().shares[0].uri, "smb://nas/Team%20files");
    let show_hidden = PreferencesUpdate {
        show_hidden: Some(true),
        ..PreferencesUpdate::default()
    };
    rust.update_preferences(&show_hidden).unwrap();
    let work = BookmarkRequest::new("/home/demo/Work (1)", "Work");
    rust.bookmark(BookmarkAction::Add, BookmarkKind::Pin, &work)
        .unwrap();
    let printed = run_python(PYTHON_CHANGES_TEXT_SIZE_AND_PRINTS, &[directory.as_path()]);

    let from_python: Value = serde_json::from_str(&printed).unwrap();
    assert_eq!(from_python, rust.snapshot().to_json());
    assert_eq!(from_python["preferences"]["columnWidths"], json!({"name": 334}));
    assert_eq!(from_python["preferences"]["textSize"], 125);
    assert_eq!(from_python["preferences"]["autoIndex"], false);
    assert_eq!(from_python["preferences"]["showHidden"], true);
    assert_eq!(from_python["pins"][1]["uri"], "file:///home/demo/Work%20%281%29");
}

/// The lock call `SettingsLock` uses conflicts with Python's `fcntl.flock`.
/// That a change holds this lock throughout is checked in
/// `settings/tests.rs` (`a_change_holds_the_settings_lock_until_it_is_written`).
/// parity: SET-014
#[test]
fn python_flock_conflicts_with_a_rust_file_lock() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("settings.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    file.lock().unwrap();

    let printed = run_python(PYTHON_TRIES_THE_LOCK, &[temporary.path()]);

    assert_eq!(printed.trim(), "blocked");
}

/// [`PYTHON_SAVES_DARK_WHILE_LOCKED`] running in the background. Dropping
/// it kills and reaps the process, even if an assertion fails while it
/// holds the lock.
struct LockingPython {
    process: Child,
}

impl LockingPython {
    /// Starts the script and returns once it holds `settings.lock`.
    fn start(directory: &Path) -> Self {
        let process = python(PYTHON_SAVES_DARK_WHILE_LOCKED, &[directory])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut locking = Self { process };
        let stdout = locking.process.stdout.take().unwrap();
        let mut ready = String::new();
        BufReader::new(stdout).read_line(&mut ready).unwrap();
        assert_eq!(ready.trim(), "locked");
        locking
    }

    /// Lets the script save the dark theme and release the lock.
    fn release(&mut self) {
        let stdin = self.process.stdin.as_mut().unwrap();
        stdin.write_all(b"release\n").unwrap();
    }

    /// Waits for the script to exit and checks that it succeeded.
    fn assert_success(mut self) {
        assert!(self.process.wait().unwrap().success());
    }
}

impl Drop for LockingPython {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// parity: SET-014, SIDE-022
#[test]
fn rust_settings_mutation_waits_for_python_and_reloads_after_unlock() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("winspace");
    let mut rust = Settings::open(&directory);
    rust.update_preferences(&PreferencesUpdate::default()).unwrap();
    let mut python = LockingPython::start(&directory);

    let (done, completed) = mpsc::channel();
    let mutation = thread::spawn(move || {
        let work = BookmarkRequest::new("/home/demo/Work", "Work");
        let result = rust.bookmark(BookmarkAction::Add, BookmarkKind::Pin, &work);
        done.send(result).unwrap();
    });
    let while_locked = completed.recv_timeout(Duration::from_millis(100));
    assert!(matches!(while_locked, Err(mpsc::RecvTimeoutError::Timeout)));
    python.release();
    completed
        .recv_timeout(Duration::from_secs(10))
        .expect("settings writer resumed after Python unlock")
        .unwrap();
    mutation.join().unwrap();
    python.assert_success();

    let saved = Settings::open(&directory).snapshot();
    assert_eq!(saved.preferences.theme, Theme::Dark);
    assert_eq!(saved.pins[0].uri, "file:///home/demo/Work");
}
