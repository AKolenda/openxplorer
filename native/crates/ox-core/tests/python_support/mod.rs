// SPDX-License-Identifier: AGPL-3.0-only
//! Runs Python scripts against the modules in `desktop/`, the behavioural
//! specification, for the interoperability tests of `settings_interop.rs`
//! and `places_composition.rs`.

use std::path::{Path, PathBuf};
use std::process::Command;

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
    let output = python(script, paths)
        .output()
        .expect("Python 3 is required for the interoperability tests");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("Python emitted UTF-8")
}
