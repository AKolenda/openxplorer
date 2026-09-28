// SPDX-License-Identifier: AGPL-3.0-only
//! Whether Brave is running for this user.
//!
//! Ports `browser_running` in `desktop/brave_integration.py`. Brave
//! rewrites its preferences when it exits, so a change made while it runs
//! would be lost or, worse, mixed with Brave's own write. The app
//! therefore changes nothing while any Brave process runs, and never
//! stops one.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// The program names of Brave's processes, including the crash handler
/// that outlives the windows.
const BRAVE_PROGRAMS: [&str; 6] = [
    "brave",
    "brave-browser",
    "brave-browser-stable",
    "brave-browser-beta",
    "brave-browser-nightly",
    "brave_crashpad_handler",
];

/// Tells whether Brave is running. [`ProcessTable`] looks at `/proc`;
/// tests supply a closure.
pub trait BraveActivity {
    /// True if Brave may be running, including when that cannot be told.
    fn is_running(&self) -> bool;
}

impl<F: Fn() -> bool> BraveActivity for F {
    fn is_running(&self) -> bool {
        self()
    }
}

/// The processes listed in a `/proc` folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessTable {
    root: PathBuf,
}

impl ProcessTable {
    /// The system's processes, in `/proc`.
    pub fn system() -> Self {
        Self::at(Path::new("/proc"))
    }

    /// The processes listed in `root`, laid out like `/proc`.
    pub fn at(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
        }
    }
}

impl BraveActivity for ProcessTable {
    fn is_running(&self) -> bool {
        is_brave_running(&self.root)
    }
}

/// True if a process of this user in `proc_root` is Brave.
///
/// Safety rule "fail closed" (`browser_running` in `brave_integration.py`):
/// a process whose command line cannot be read counts as Brave when its
/// name mentions Brave or cannot be read either, and a process table that
/// cannot be listed counts as Brave running.
fn is_brave_running(proc_root: &Path) -> bool {
    let Ok(entries) = fs::read_dir(proc_root) else {
        return true;
    };
    for entry in entries {
        let Ok(entry) = entry else {
            return true;
        };
        let is_process = entry.file_name().as_bytes().iter().all(u8::is_ascii_digit);
        if is_process && may_be_brave(&entry.path()) {
            return true;
        }
    }
    false
}

/// The ID of the user the app runs as. `GCredentials` records it on
/// Linux; for a desktop app it is the same as Python's `os.getuid()`.
pub(super) fn current_user_id() -> u32 {
    gio::Credentials::new()
        .unix_user()
        .expect("GCredentials holds the user ID on Linux")
}

/// Whether the process folder `process` may be one of this user's Brave
/// processes. A process that exited meanwhile is not.
fn may_be_brave(process: &Path) -> bool {
    match runs_brave_program(process) {
        Ok(is_brave) => is_brave,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => name_may_be_brave(process),
        Err(_) => true,
    }
}

/// Whether `process` belongs to this user and runs a Brave program.
fn runs_brave_program(process: &Path) -> io::Result<bool> {
    if fs::metadata(process)?.uid() != current_user_id() {
        return Ok(false);
    }
    let command_line = fs::read(process.join("cmdline"))?;
    let program = command_line.split(|byte| *byte == 0).next().unwrap_or_default();
    let program_name = Path::new(OsStr::from_bytes(program))
        .file_name()
        .unwrap_or_default();
    Ok(BRAVE_PROGRAMS.iter().any(|name| OsStr::new(name) == program_name))
}

/// Whether the short process name mentions Brave; true when it cannot be
/// read.
fn name_may_be_brave(process: &Path) -> bool {
    match fs::read_to_string(process.join("comm")) {
        Ok(name) => glib::casefold(&name).contains("brave"),
        Err(_) => true,
    }
}
