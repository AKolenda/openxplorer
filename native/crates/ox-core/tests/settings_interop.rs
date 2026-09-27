// SPDX-License-Identifier: AGPL-3.0-only
//! Cross-language settings and flock protocol checks against desktop/core.py.
//! Every file is inside a temporary directory; the user's settings are untouched.

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use ox_core::settings::{BookmarkAction, BookmarkKind, PreferencesUpdate, Settings};
use serde_json::{json, Value};

fn python(script: &str, directory: &Path) -> Command {
    let desktop = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../desktop");
    let mut command = Command::new("python3");
    command
        .arg("-u")
        .arg("-c")
        .arg(script)
        .arg(directory)
        .env("PYTHONPATH", desktop);
    command
}

fn run_python(script: &str, directory: &Path) -> String {
    let output = python(script, directory)
        .output()
        .expect("Python 3 is required for settings interoperability tests");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("Python emitted UTF-8")
}

#[test]
fn python_and_rust_mutations_preserve_each_others_settings() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("winspace");
    run_python(
        r#"
import sys
from pathlib import Path
from core import Settings
store = Settings(Path(sys.argv[1]))
store.update_preferences({'theme': 'dark', 'autoIndex': False, 'networkInterval': 300, 'columnWidths': {'name': 333.5}})
store.bookmark('add', 'share', 'smb://NAS/Team files', 'Shared work')
store.pin_many([{'uri': 'mtp://[usb:001,002]/Storage', 'label': 'Phone'}])
"#,
        &directory,
    );
    let mut rust = Settings::open(&directory);
    assert!(rust.warning().is_none());
    assert_eq!(rust.data().preferences.theme, "dark");
    assert_eq!(rust.data().shares[0].uri, "smb://nas/Team%20files");
    rust.update_preferences(&PreferencesUpdate {
        show_hidden: Some(true),
        ..PreferencesUpdate::default()
    })
    .unwrap();
    rust.bookmark(
        BookmarkAction::Add,
        BookmarkKind::Pin,
        "/home/demo/Work (1)",
        "Work",
    )
    .unwrap();
    let read = run_python(
        r#"
import json, sys
from pathlib import Path
from core import Settings
store = Settings(Path(sys.argv[1]))
assert not store.warning, store.warning
store.update_preferences({'textSize': 125})
print(json.dumps(store.snapshot()))
"#,
        &directory,
    );
    let from_python: Value = serde_json::from_str(&read).unwrap();
    assert_eq!(from_python, rust.snapshot().to_json());
    assert_eq!(from_python["preferences"]["columnWidths"], json!({"name": 334}));
    assert_eq!(from_python["preferences"]["textSize"], 125);
    assert_eq!(from_python["preferences"]["autoIndex"], false);
    assert_eq!(from_python["preferences"]["showHidden"], true);
    assert_eq!(from_python["pins"][1]["uri"], "file:///home/demo/Work%20%281%29");
}

#[test]
fn python_flock_observes_a_rust_file_lock() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("settings.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    file.lock().unwrap();
    let result = run_python(
        r#"
import fcntl, sys
from pathlib import Path
with (Path(sys.argv[1]) / 'settings.lock').open('r+') as lock:
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        print('blocked')
    else:
        raise AssertionError('Python acquired a lock already held by Rust')
"#,
        temporary.path(),
    );
    assert_eq!(result.trim(), "blocked");
}

/// Reap the helper even if an assertion fails while it holds the lock.
struct PythonChild(Child);

impl Drop for PythonChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn rust_settings_mutation_waits_for_python_and_reloads_after_unlock() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("winspace");
    let mut rust = Settings::open(&directory);
    rust.save().unwrap();
    let child = python(
        r#"
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
"#,
        &directory,
    )
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .spawn()
    .unwrap();
    let mut child = PythonChild(child);
    let mut output = BufReader::new(child.0.stdout.take().unwrap());
    let mut ready = String::new();
    output.read_line(&mut ready).unwrap();
    assert_eq!(ready.trim(), "locked");

    let (done, completed) = mpsc::channel();
    let mutation = std::thread::spawn(move || {
        let result = rust.bookmark(BookmarkAction::Add, BookmarkKind::Pin, "/home/demo/Work", "Work");
        done.send(result).unwrap();
    });
    assert!(matches!(
        completed.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    child.0.stdin.as_mut().unwrap().write_all(b"release\n").unwrap();
    completed
        .recv_timeout(Duration::from_secs(10))
        .expect("settings writer resumed after Python unlock")
        .unwrap();
    mutation.join().unwrap();
    assert!(child.0.wait().unwrap().success());
    let data = Settings::open(&directory).snapshot();
    assert_eq!(data.preferences.theme, "dark");
    assert_eq!(data.pins[0].uri, "file:///home/demo/Work");
}
