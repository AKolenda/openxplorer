// SPDX-License-Identifier: AGPL-3.0-only
//! Runs Python scripts against the modules in `desktop/`, the behavioural
//! specification, for the interoperability tests of `settings_interop.rs`,
//! `places_composition.rs`, `network_python.rs`,
//! `network_keyring_python.rs` and `network_secret_service.rs`.
#![allow(
    dead_code,
    reason = "each test crate that includes this module uses a different part of it"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// `python3 -u -c <script> <paths...>` with `desktop/` on the module path.
/// The script reads the paths as `sys.argv[1:]`.
pub fn python(script: &str, paths: &[&Path]) -> Command {
    let desktop = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../desktop");
    let mut command = Command::new("python3");
    command
        .arg("-u")
        .arg("-c")
        .arg(script)
        .args(paths)
        .env("PYTHONPATH", desktop);
    command
}

/// Runs `script` to completion and returns what it printed; a failing
/// script fails the test with its error output.
pub fn run_python(script: &str, paths: &[&Path]) -> String {
    printed_by(python(script, paths))
}

/// Runs `script` with `inputs` written to a JSON file, whose path the
/// script reads as `sys.argv[1]`, and returns what it printed, parsed as
/// JSON.
pub fn python_answers(script: &str, inputs: &Value) -> Value {
    python_answers_with(script, inputs, |_| {})
}

/// [`python_answers`], after `configure` has prepared the command, for
/// example its environment.
pub fn python_answers_with(script: &str, inputs: &Value, configure: impl FnOnce(&mut Command)) -> Value {
    let folder = tempfile::tempdir().expect("temporary folder");
    let input_file = folder.path().join("inputs.json");
    fs::write(&input_file, inputs.to_string()).expect("inputs written");
    let mut command = python(script, &[&input_file]);
    configure(&mut command);
    let printed = printed_by(command);
    serde_json::from_str(&printed).expect("the script prints JSON")
}

/// The list a script printed.
pub fn as_array(value: &Value) -> &[Value] {
    value.as_array().expect("the script prints a list")
}

/// Runs `command` to completion and returns what it printed; a failing
/// command fails the test with its error output.
fn printed_by(mut command: Command) -> String {
    let output = command
        .output()
        .expect("Python 3 is required for the interoperability tests");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("Python emitted UTF-8")
}
