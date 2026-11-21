// SPDX-License-Identifier: AGPL-3.0-only
//! What the app does about its own crashes: it writes every panic, with
//! its backtrace, to `crash.log` in the user's cache folder
//! (`~/.cache/winspace/crash.log`), and it notices on the next start that
//! the last run ended abnormally.
//!
//! The tabs are saved every few seconds while the app runs
//! (`window/session_restore.rs`), so a crash that no panic hook sees (a C
//! library aborting, the kernel killing the app) still leaves them saved,
//! and "Reopen my tabs" reopens them with nothing said about the crash.
//! Only when the last two runs both ended abnormally within their first
//! [`START_SECONDS`] is the restore skipped once, quietly, so a tab whose
//! folder takes the app down cannot make it crash at every start; an app
//! killed later (at logout without a session manager, say) never counts
//! towards that.
//!
//! A run is marked while it runs ([`RUN_MARKER`]); a clean exit, and an
//! end of the session (after the tabs are saved), remove the mark. A mark
//! left at start is an abnormal end.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use gtk::glib;

/// The crash log, in the app's cache folder.
pub(crate) const CRASH_LOG: &str = "crash.log";

/// The mark of a running app, in the app's cache folder; it holds how many
/// runs before it ended abnormally in a row while they started.
const RUN_MARKER: &str = "running";

/// The size past which the crash log keeps only its newest half.
const LOG_LIMIT: u64 = 512 * 1024;

/// How many abnormal ends in a row while starting skip the tabs' restore
/// once.
const CRASH_LOOP: u32 = 2;

/// How long a run must last to have started well.
pub(crate) const START_SECONDS: u32 = 60;

/// The app's cache folder, `winspace` (a compatibility name).
pub(crate) fn cache_folder() -> PathBuf {
    glib::user_cache_dir().join("winspace")
}

/// How the run before this one ended, found once at start.
static PREVIOUS: OnceLock<PreviousRun> = OnceLock::new();

/// How the runs before this one ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PreviousRun {
    /// The last run ended abnormally.
    pub crashed: bool,
    /// How many runs in a row ended abnormally while they started, the
    /// last one first.
    pub abnormal_starts: u32,
}

impl PreviousRun {
    /// Whether the saved tabs should be reopened: not after
    /// [`CRASH_LOOP`] runs in a row that ended abnormally while starting.
    pub(crate) fn may_restore_tabs(self) -> bool {
        self.abnormal_starts < CRASH_LOOP
    }
}

/// Writes every panic to the crash log before the usual report on
/// standard error. Installed once, by the app (not its helper
/// processes).
pub(crate) fn install_panic_hook() {
    let folder = cache_folder();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| (*text).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "a panic without a message".to_owned());
        let location = info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_default();
        let backtrace = std::backtrace::Backtrace::force_capture();
        let entry = format!(
            "OpenXplorer {}: thread '{}' panicked at {location}: {message}\n{backtrace}\n",
            env!("CARGO_PKG_VERSION"),
            thread.name().unwrap_or("unnamed"),
        );
        append_to_log(&folder, &entry);
        previous(info);
    }));
}

/// Appends `entry`, stamped with the time, to the crash log in `folder`,
/// keeping the log under [`LOG_LIMIT`]. Failures are ignored: nothing
/// about logging a crash may fail louder than the crash.
pub(crate) fn append_to_log(folder: &Path, entry: &str) {
    let _ = fs::DirBuilder::new().recursive(true).mode(0o700).create(folder);
    let path = folder.join(CRASH_LOG);
    if fs::metadata(&path).is_ok_and(|metadata| metadata.len() > LOG_LIMIT) {
        if let Ok(old) = fs::read(&path) {
            let keep = old
                .len()
                .saturating_sub(usize::try_from(LOG_LIMIT / 2).unwrap_or(0));
            let _ = fs::write(&path, old.get(keep..).unwrap_or_default());
        }
    }
    let stamp = glib::DateTime::now_local()
        .ok()
        .and_then(|now| now.format_iso8601().ok())
        .map(|now| now.to_string())
        .unwrap_or_default();
    if let Ok(mut log) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&path)
    {
        let _ = writeln!(log, "[{stamp}] {entry}");
    }
}

/// Runs `work`, a main-thread step that handles what a file gave (a
/// selection's details, a preview, an archive's listing), and catches a
/// panic in it: the panic hook has written it to the crash log, and the
/// step is skipped instead of aborting the app, which a panic does when
/// it reaches GTK's C callbacks. `None` after a panic.
///
/// Catching is sound here because nothing is left borrowed (`RefCell`
/// guards are released while unwinding) and the step only changes what
/// widgets show; at worst a pane shows stale content until the next
/// selection.
pub(crate) fn guarded<T>(what: &str, work: impl FnOnce() -> T) -> Option<T> {
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
    if caught.is_err() {
        glib::g_warning!(
            ox_core::LOG_DOMAIN,
            "Showing {what} failed, and was skipped; see crash.log"
        );
    }
    caught.ok()
}

/// Marks this run as running in `folder`, and finds how the runs before
/// it ended. Called once, when the app starts.
pub(crate) fn mark_running(folder: &Path, app_id: &str) -> PreviousRun {
    let marker = marker_path(folder, app_id);
    let previous = match fs::read_to_string(&marker) {
        Ok(text) => {
            // A run that started well wrote "started"; one still starting,
            // how many runs before it ended abnormally while starting.
            let starting = text.trim().parse::<u32>().ok();
            PreviousRun {
                crashed: true,
                abnormal_starts: starting.map_or(0, |count| count.saturating_add(1)),
            }
        }
        Err(_) => PreviousRun::default(),
    };
    let _ = fs::DirBuilder::new().recursive(true).mode(0o700).create(folder);
    let _ = fs::write(&marker, format!("{}\n", previous.abnormal_starts));
    if previous.crashed {
        let skipped = if previous.may_restore_tabs() {
            ""
        } else {
            " while starting, again; the saved tabs are not reopened this time"
        };
        append_to_log(folder, &format!("The last run ended abnormally{skipped}.\n"));
    }
    previous
}

/// Marks this run in `folder` as started well: if it ends abnormally
/// now, that does not count towards a crash loop.
pub(crate) fn mark_started(folder: &Path, app_id: &str) {
    let marker = marker_path(folder, app_id);
    if marker.exists() {
        let _ = fs::write(&marker, "started\n");
    }
}

/// Marks this run as ended cleanly in `folder`.
pub(crate) fn mark_clean_exit(folder: &Path, app_id: &str) {
    let _ = fs::remove_file(marker_path(folder, app_id));
}

fn marker_path(folder: &Path, app_id: &str) -> PathBuf {
    folder.join(format!("{RUN_MARKER}-{app_id}"))
}

/// Marks the app's run as running, and remembers how the run before it
/// ended for [`previous_run`].
pub(crate) fn start_run() {
    let previous = mark_running(&cache_folder(), crate::config::APP_ID);
    let _ = PREVIOUS.set(previous);
}

/// Marks the app's run as started well ([`mark_started`]).
pub(crate) fn survived_start() {
    mark_started(&cache_folder(), crate::config::APP_ID);
}

/// Marks the app's run as ended cleanly.
pub(crate) fn end_run() {
    mark_clean_exit(&cache_folder(), crate::config::APP_ID);
}

/// How the run before this one ended; a clean end before [`start_run`].
pub(crate) fn previous_run() -> PreviousRun {
    PREVIOUS.get().copied().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clean exit leaves no mark; a run that never exited is found at
    /// the next start, and two in a row that ended while starting skip
    /// the tabs' restore once. A run that started well and was killed
    /// later only counts as a crash.
    #[test]
    fn an_abnormal_end_is_found_at_the_next_start() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let folder = folder.path().join("winspace");
        let id = "io.example.Test";
        assert_eq!(mark_running(&folder, id), PreviousRun::default());
        mark_clean_exit(&folder, id);
        let clean = mark_running(&folder, id);
        assert!(!clean.crashed && clean.may_restore_tabs());
        // This run crashes while starting: no clean exit.
        let once = mark_running(&folder, id);
        assert!(once.crashed && once.may_restore_tabs());
        let twice = mark_running(&folder, id);
        assert_eq!(twice.abnormal_starts, 2);
        assert!(!twice.may_restore_tabs());
        mark_clean_exit(&folder, id);
        assert_eq!(mark_running(&folder, id), PreviousRun::default());
        // Runs that started well and were killed later.
        mark_started(&folder, id);
        let killed = mark_running(&folder, id);
        assert!(killed.crashed && killed.abnormal_starts == 0);
        mark_started(&folder, id);
        assert!(mark_running(&folder, id).may_restore_tabs());
        let log = fs::read_to_string(folder.join(CRASH_LOG)).expect("the crash log");
        assert!(log.contains("The last run ended abnormally."));
        assert!(log.contains("not reopened this time"));
    }

    /// A panic is written to the crash log with its message, where it
    /// happened and a backtrace; the log never grows past its limit.
    #[test]
    fn a_panic_is_written_to_the_crash_log() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let entry = format!(
            "thread 'main' panicked at src/example.rs:7: index out of bounds\n{}",
            std::backtrace::Backtrace::force_capture()
        );
        append_to_log(folder.path(), &entry);
        let log = fs::read_to_string(folder.path().join(CRASH_LOG)).expect("the crash log");
        assert!(log.contains("index out of bounds"));
        for _ in 0..200 {
            append_to_log(folder.path(), &"x".repeat(8 * 1024));
        }
        let size = fs::metadata(folder.path().join(CRASH_LOG))
            .expect("the log")
            .len();
        assert!(size <= LOG_LIMIT + 16 * 1024, "{size}");
    }

    /// A panicking thread with the hook installed reaches the log through
    /// the hook, and the test process goes on.
    #[test]
    fn the_hook_logs_a_panic_on_a_worker_thread() {
        let folder = cache_folder();
        install_panic_hook();
        let joined = std::thread::Builder::new()
            .name("archive-worker".to_owned())
            .spawn(|| panic!("a parser met a header it never foresaw"))
            .expect("a thread")
            .join();
        assert!(joined.is_err());
        let _ = std::panic::take_hook();
        let log = fs::read_to_string(folder.join(CRASH_LOG)).expect("the crash log");
        assert!(log.contains("thread 'archive-worker' panicked"));
        assert!(log.contains("a parser met a header it never foresaw"));
    }

    /// A panic in a guarded step is caught, the step skipped, and the
    /// next one runs, even from inside a GTK callback.
    #[gtk::test]
    fn a_panic_in_a_guarded_step_is_caught() {
        use gtk::prelude::*;

        assert_eq!(guarded("a test step", || 7), Some(7));
        let fired = std::rc::Rc::new(std::cell::Cell::new(false));
        let button = gtk::Button::new();
        let seen = std::rc::Rc::clone(&fired);
        button.connect_clicked(move |_| {
            let skipped = guarded("a test step", || -> u8 {
                let rows: Vec<u8> = Vec::new();
                rows[std::hint::black_box(3)]
            });
            seen.set(skipped.is_none());
        });
        button.emit_clicked();
        assert!(fired.get());
    }
}
