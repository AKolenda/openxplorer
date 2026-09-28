// SPDX-License-Identifier: AGPL-3.0-only
//! Proves that the location fixtures still hold what the Python app answers.
//!
//! `python.json` and `javascript.json` record the answers of
//! `desktop/core.py` and of the display helpers in `desktop/ui/app.js`, and
//! the other `location_*` tests hold the Rust port to them. When the Python
//! app changes, recorded answers go stale without any test noticing. These
//! tests run both generators against the current `desktop/` sources and fail
//! when the answers differ from the committed files, naming each change.
//!
//! They need `python3` and Node.js. A missing tool fails the test: a skipped
//! check would look like a passing one.

use std::collections::BTreeSet;
use std::env;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// How many changed answers a failure lists.
const LISTED_CHANGES: usize = 20;

#[test]
fn python_fixture_matches_desktop_core_py() {
    if std::env::var_os("OX_DISTRO_CI").is_some() {
        eprintln!("skipped under OX_DISTRO_CI: {}", "the reference check needs the Python version the fixtures were generated with; the native job covers this");
        return;
    }
    let desktop = repository_path("desktop");
    let captured = run_generator(OsStr::new("python3"), "generate_python.py", &desktop);
    let committed = include_str!("location_fixtures/python.json");
    assert_fixture_is_current("python.json", committed, &captured);
}

#[test]
fn javascript_fixture_matches_desktop_app_js() {
    if std::env::var_os("OX_DISTRO_CI").is_some() {
        eprintln!("skipped under OX_DISTRO_CI: {}", "the reference check needs a current Node.js; distribution Node versions differ; the native job covers this");
        return;
    }
    let app = repository_path("desktop/ui/app.js");
    let captured = run_generator(&node_program(), "generate_javascript.cjs", &app);
    let committed = include_str!("location_fixtures/javascript.json");
    assert_fixture_is_current("javascript.json", committed, &captured);
}

/// The Node.js executable: `$OX_NODE` when set, else `node` from `PATH`.
///
/// `native/tools/check.py` runs tests with a disposable home folder. A
/// version-manager shim for `node` looks for its installed runtimes there,
/// and may try to download one; set `OX_NODE` to the executable the shim
/// runs (`node -p process.execPath`) to avoid that.
fn node_program() -> OsString {
    env::var_os("OX_NODE").unwrap_or_else(|| OsString::from("node"))
}

/// A path relative to the repository root.
fn repository_path(relative: &str) -> PathBuf {
    let crate_directory = Path::new(env!("CARGO_MANIFEST_DIR"));
    crate_directory.join("../../..").join(relative)
}

/// Runs a generator from `location_fixtures/` on `source` and returns the
/// JSON document it printed.
fn run_generator(interpreter: &OsStr, script: &str, source: &Path) -> String {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/location_fixtures");
    let output = Command::new(interpreter)
        .arg(fixtures.join(script))
        .arg(source)
        .output()
        .unwrap_or_else(|error| {
            let interpreter = interpreter.display();
            panic!("{interpreter} is required to check {script} against desktop/: {error}")
        });
    let errors = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{script} failed:\n{errors}");
    String::from_utf8(output.stdout).expect("the generators print UTF-8")
}

/// Compares the answers, not the text: formatting may differ.
fn assert_fixture_is_current(fixture: &str, committed: &str, captured: &str) {
    let committed: Value = serde_json::from_str(committed).expect("the committed fixture is JSON");
    let captured: Value = serde_json::from_str(captured).expect("the generator prints JSON");
    let mut changes = Vec::new();
    collect_changes("", &committed, &captured, &mut changes);
    let listed = &changes[..changes.len().min(LISTED_CHANGES)];
    assert!(
        changes.is_empty(),
        "{fixture} no longer matches desktop/ ({} changed answers). Regenerate it as \
         location_fixtures/README.md describes, review the changes and update the Rust port.\n{}",
        changes.len(),
        listed.join("\n")
    );
}

/// Appends one line per value that differs below `path`, descending into
/// objects and into arrays of equal length.
fn collect_changes(path: &str, committed: &Value, captured: &Value, changes: &mut Vec<String>) {
    if committed == captured {
        return;
    }
    match (committed, captured) {
        (Value::Object(committed), Value::Object(captured)) => {
            let keys: BTreeSet<&String> = committed.keys().chain(captured.keys()).collect();
            for key in keys {
                let old = committed.get(key).unwrap_or(&Value::Null);
                let new = captured.get(key).unwrap_or(&Value::Null);
                collect_changes(&format!("{path}.{key}"), old, new, changes);
            }
        }
        (Value::Array(committed), Value::Array(captured)) if committed.len() == captured.len() => {
            for (index, (old, new)) in committed.iter().zip(captured).enumerate() {
                collect_changes(&format!("{path}[{index}]"), old, new, changes);
            }
        }
        (Value::Array(committed), Value::Array(captured)) => {
            let counts = format!("{} entries committed, {} now", committed.len(), captured.len());
            changes.push(format!("  {path}: {counts}"));
        }
        _ => changes.push(format!("  {path}: committed {committed}, now {captured}")),
    }
}
