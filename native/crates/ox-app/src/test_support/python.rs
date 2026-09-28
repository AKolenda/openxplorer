// SPDX-License-Identifier: AGPL-3.0-only
//! Runs the Python app's own settings code against a test's settings
//! directory, so the GTK tests prove both apps read and write the same
//! file, as `ox-core/tests/python_support` does for ox-core.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Prints the preferences the Python app reads from `sys.argv[1]`, one
/// `key=value` line each, in Python's spelling (`True`, `None`).
const PRINTS_PREFERENCES: &str = r"
import sys
from pathlib import Path
from core import Settings
preferences = Settings(Path(sys.argv[1])).snapshot()['preferences']
for key in ('theme', 'textSize', 'contextMenu', 'autoIndex', 'networkInterval', 'sidebarWidth', 'columnWidths'):
    print(f'{key}={preferences.get(key)}')
";

/// Saves the preferences in `sys.argv[2]`, a Python literal, through the
/// Python app's `update_preferences`.
const SAVES_PREFERENCES: &str = r"
import ast, sys
from pathlib import Path
from core import Settings
Settings(Path(sys.argv[1])).update_preferences(ast.literal_eval(sys.argv[2]))
";

/// Runs `script` with `desktop/` on the module path and `arguments` as
/// `sys.argv[1:]`, and returns what it printed; a failing script fails the
/// test with its error output.
fn run_python(script: &str, arguments: &[&str]) -> String {
    let desktop = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../desktop");
    let output = Command::new("python3")
        .arg("-c")
        .arg(script)
        .args(arguments)
        .env("PYTHONPATH", desktop)
        .output()
        .expect("Python 3 is required for the interoperability tests");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("Python prints UTF-8")
}

/// The preference `key` as the Python app reads it from the settings in
/// `directory`, such as `dark` for `theme`.
pub(crate) fn python_preference(directory: &Path, key: &str) -> String {
    let directory = directory.to_string_lossy();
    let printed = run_python(PRINTS_PREFERENCES, &[&directory]);
    let prefix = format!("{key}=");
    let line = printed.lines().find_map(|line| line.strip_prefix(&prefix));
    line.unwrap_or_else(|| panic!("the Python app has no preference {key}"))
        .to_owned()
}

/// Saves `preferences`, a Python dictionary literal such as
/// `{'theme': 'dark'}`, to the settings in `directory` as the Python app
/// does.
pub(crate) fn python_saves_preferences(directory: &Path, preferences: &str) {
    let directory = directory.to_string_lossy();
    run_python(SAVES_PREFERENCES, &[&directory, preferences]);
}
