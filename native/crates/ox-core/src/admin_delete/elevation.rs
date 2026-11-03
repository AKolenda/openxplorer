// SPDX-License-Identifier: AGPL-3.0-only
//! Starting the privileged helper, and reading back what it did.
//!
//! The app itself never runs as root. To delete an `admin://` folder it
//! starts `openxplorer-admin-delete` through `pkexec`, which asks the
//! desktop's authentication agent about the helper's own polkit action
//! (`native/packaging/data/admin-delete.policy.in`). That action is
//! `auth_admin_keep`, so an administrator authenticates once and the
//! answer is remembered for a short while, which is what makes deleting
//! several items in a row bearable.
//!
//! The step that elevates is replaceable. The tests run exactly the walk
//! the helper runs, in-process and as the ordinary test user, so the rules
//! this change is about are tested without anything in the suite ever
//! becoming root.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use super::report::{Line, Record, Totals};
use crate::i18n::{format_message, gettext};
use crate::transfer::{Cancellation, TransferError};

/// The helper program's file name, as every package installs it.
pub const HELPER_COMMAND: &str = "openxplorer-admin-delete";

/// Where a build or a test says the helper is, instead of beside the app.
const HELPER_VARIABLE: &str = "OX_ADMIN_DELETE_HELPER";

/// How often the watcher looks whether the user cancelled.
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// Deletes `path` with administrator rights, reporting each item to
/// `report`, and returns what the walk left behind.
pub type Elevation =
    Arc<dyn Fn(&Path, &Cancellation, &mut dyn FnMut(Record)) -> Result<Totals, TransferError> + Send + Sync>;

/// The replacement for `pkexec`, when one was installed.
static ELEVATION: RwLock<Option<Elevation>> = RwLock::new(None);

/// Replaces the step that elevates, or restores `pkexec` with `None`.
///
/// This exists for the tests, which run the helper's walk in-process as
/// the ordinary test user, and for an embedder that already has its own
/// administrator prompt. It changes the whole process, so a test that uses
/// it holds [`crate::test_support::elevation_lock`].
///
/// # Panics
///
/// If another thread panicked while holding the lock on the replacement.
pub fn set_elevation(elevation: Option<Elevation>) {
    *ELEVATION.write().expect("the elevation lock is not poisoned") = elevation;
}

/// Runs the deletion of `path` with administrator rights.
pub(super) fn elevated_delete(
    path: &Path,
    cancel: &Cancellation,
    report: &mut dyn FnMut(Record),
) -> Result<Totals, TransferError> {
    let installed = ELEVATION
        .read()
        .expect("the elevation lock is not poisoned")
        .clone();
    match installed {
        Some(elevation) => elevation(path, cancel, report),
        None => through_pkexec(path, cancel, report),
    }
}

/// Where the helper is installed, which is beside the running app.
///
/// Every package installs the app and its helper together, so the helper
/// is found relative to the program rather than looked up on `PATH`, which
/// a user could point elsewhere. The two layouts are the Debian one, whose
/// app lives in `/opt/<package>/bin`, and the FHS one of the RPM and Arch
/// packages, whose app lives in `/usr/bin`.
///
/// A Flatpak never has one, whatever is installed or configured: a helper
/// started there would walk the sandbox's own view of the filesystem, not
/// the host's that `gvfsd-admin` and the user's `admin://` path mean, so
/// it could delete a different tree from the one the user confirmed.
///
/// # Errors
///
/// [`TransferError::NotSupported`] when no helper is installed, and always
/// in a Flatpak, which has no host `pkexec` to start one with either.
pub fn helper_path() -> Result<PathBuf, TransferError> {
    helper_path_in(
        crate::integration::Sandbox::detect(),
        std::env::var_os(HELPER_VARIABLE),
        std::env::current_exe,
    )
}

/// [`helper_path`] in `sandbox`, with `configured` read from
/// `OX_ADMIN_DELETE_HELPER` and `program` saying where the app runs from.
fn helper_path_in(
    sandbox: crate::integration::Sandbox,
    configured: Option<std::ffi::OsString>,
    program: impl FnOnce() -> std::io::Result<PathBuf>,
) -> Result<PathBuf, TransferError> {
    // Asked first, so that not even the variable a build or a test sets
    // can start a helper inside the sandbox.
    if sandbox.is_flatpak() {
        return Err(no_helper());
    }
    if let Some(configured) = configured {
        return Ok(PathBuf::from(configured));
    }
    let program = program()
        .map_err(|error| TransferError::failed(format!("The running program has no path: {error}")))?;
    helper_candidates(&program)
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(no_helper)
}

/// The refusal for a build that has no helper to start.
fn no_helper() -> TransferError {
    TransferError::NotSupported(gettext(
        "This build has no administrator deletion helper, so a whole folder cannot be \
         deleted under admin://. Delete the items inside it one at a time, or install the \
         distribution package of OpenXplorer.",
    ))
}

/// Where the helper may be, for an app installed at `program`, in the
/// order the layouts of `native/tools/package_data.py` put it.
fn helper_candidates(program: &Path) -> [PathBuf; 2] {
    let folder = program.parent().unwrap_or(Path::new("/"));
    let name = program.file_name().unwrap_or_default();
    [
        // FHS (RPM, Arch): /usr/bin/<package> -> /usr/libexec/<package>/
        folder.join("../libexec").join(name).join(HELPER_COMMAND),
        // Debian: /opt/<package>/bin/<package> -> /opt/<package>/libexec/
        folder.join("../libexec").join(HELPER_COMMAND),
    ]
}

/// Runs the helper under `pkexec` and reads its records.
fn through_pkexec(
    path: &Path,
    cancel: &Cancellation,
    report: &mut dyn FnMut(Record),
) -> Result<Totals, TransferError> {
    let helper = helper_path()?;
    let mut child = Command::new("pkexec")
        .arg(&helper)
        .arg("--")
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| {
            TransferError::failed(format_message(
                "The administrator deletion helper could not be started with pkexec. {reason}",
                &[("reason", &error.to_string())],
            ))
        })?;
    let watcher = watch_for_cancellation(&mut child, cancel);
    let outcome = read_records(&mut child, report);
    watcher.stop();
    let status = child.wait().map_err(|error| {
        TransferError::failed(format!("The administrator deletion helper was lost: {error}"))
    })?;
    match outcome {
        Outcome::Refused(message) => Err(TransferError::failed(message)),
        Outcome::Cancelled => Err(TransferError::Cancelled),
        Outcome::Done(totals) => Ok(totals),
        // pkexec itself refuses with 126 (not authorised) and 127 (the
        // helper could not be run), neither of which writes a record.
        Outcome::NothingReported => Err(TransferError::failed(format_message(
            "The administrator deletion did not run ({status}). Your desktop may have refused \
             the authentication, or no administrator deletion helper is installed.",
            &[("status", &status.to_string())],
        ))),
    }
}

/// What the helper's output amounted to.
enum Outcome {
    Refused(String),
    Cancelled,
    Done(Totals),
    NothingReported,
}

/// Reads the helper's records, handing each to `report`.
fn read_records(child: &mut Child, report: &mut dyn FnMut(Record)) -> Outcome {
    let Some(output) = child.stdout.take() else {
        return Outcome::NothingReported;
    };
    let mut outcome = Outcome::NothingReported;
    for line in BufReader::new(output).lines().map_while(Result::ok) {
        // A line this version does not know is ignored: a newer helper may
        // report more than this app asks about.
        match Line::decode(&line) {
            Some(Line::Record(record)) => report(record),
            Some(Line::Refused(message)) => outcome = Outcome::Refused(message),
            Some(Line::Cancelled) => outcome = Outcome::Cancelled,
            // A cancelled walk also reports its totals; the app needs to
            // know it was cancelled, not that it merely left items behind.
            Some(Line::Done(_)) if matches!(outcome, Outcome::Cancelled) => {}
            Some(Line::Done(totals)) => outcome = Outcome::Done(totals),
            None => {}
        }
    }
    outcome
}

/// Tells the helper to stop once the user cancels.
struct Watcher {
    stopped: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Watcher {
    /// Ends the watch and waits for its thread, which closes the helper's
    /// standard input.
    fn stop(mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Starts the thread that asks the helper to stop when `cancel` is
/// cancelled. The helper reads `cancel` on its standard input.
fn watch_for_cancellation(child: &mut Child, cancel: &Cancellation) -> Watcher {
    let stopped = Arc::new(AtomicBool::new(false));
    let Some(mut input) = child.stdin.take() else {
        return Watcher {
            stopped,
            thread: None,
        };
    };
    let watching = Arc::clone(&stopped);
    let cancel = cancel.clone();
    let thread = std::thread::spawn(move || {
        while !watching.load(Ordering::SeqCst) {
            if cancel.is_cancelled() {
                let _ = input.write_all(b"cancel\n");
                let _ = input.flush();
                return;
            }
            std::thread::sleep(CANCEL_POLL);
        }
    });
    Watcher {
        stopped,
        thread: Some(thread),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integration::Sandbox;

    /// Kept from the up-front refusal this walk replaced: a Flatpak never
    /// deletes a whole `admin://` folder, even with a helper configured,
    /// and the program's own path is not even looked at.
    ///
    /// parity: XFER-015, OPS-039
    #[test]
    fn a_flatpak_never_starts_a_helper() {
        let configured = Some(std::ffi::OsString::from(
            "/opt/openxplorer/libexec/openxplorer-admin-delete",
        ));

        let result = helper_path_in(Sandbox::Flatpak, configured, || {
            panic!("the program's path was looked up inside a Flatpak")
        });

        assert_eq!(result, Err(no_helper()));
    }

    /// On the host the variable a build or a test sets is honoured, which
    /// is what the check above has to come before.
    ///
    /// parity: XFER-015
    #[test]
    fn outside_a_flatpak_the_configured_helper_is_used() {
        let configured = PathBuf::from("/opt/openxplorer/libexec/openxplorer-admin-delete");

        let result = helper_path_in(Sandbox::Host, Some(configured.clone().into_os_string()), || {
            panic!("the program's path was looked up although a helper was configured")
        });

        assert_eq!(result, Ok(configured));
    }

    /// The app finds its helper beside itself rather than on `PATH`,
    /// which a user could point elsewhere. These are the two layouts
    /// `native/tools/package_data.py` installs
    /// (`test_package_data.py::LAYOUT_CASES` holds the same paths).
    ///
    /// parity: XFER-015
    #[test]
    fn the_helper_is_found_where_every_package_installs_it() {
        let fhs = helper_candidates(Path::new("/usr/bin/openxplorer"));
        let debian = helper_candidates(Path::new("/opt/openxplorer/bin/openxplorer"));

        assert!(fhs.contains(&PathBuf::from(
            "/usr/bin/../libexec/openxplorer/openxplorer-admin-delete"
        )));
        assert!(debian.contains(&PathBuf::from(
            "/opt/openxplorer/bin/../libexec/openxplorer-admin-delete"
        )));
    }
}
