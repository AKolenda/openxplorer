// SPDX-License-Identifier: AGPL-3.0-only
//! How quickly the executable opens a window: the first start, and
//! another window while the app runs.
//!
//! The app writes the steps of its start to standard error when
//! `OX_STARTUP_TRACE=1` is set (`startup_trace.rs`), each with the
//! monotonic clock this test reads before it starts the process. The
//! budgets are several times what a test build takes on an ordinary
//! computer, so they hold on a busy one, and still catch a start that
//! waits for something it should not: reading the whole executable before
//! a later launch handed its command line over took a test build two
//! seconds, where the handover now takes a tenth of that.
//!
//! Like `launch.rs`, these start `openxplorer-native` itself, so they run
//! only inside the isolation `native/tools/check.py` sets up; anywhere
//! else they skip, so a process never reaches the live desktop.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// The executable under test.
const APP: &str = env!("CARGO_BIN_EXE_openxplorer-native");

/// How long the first window may take to show its folder's files, from
/// the moment the process is started.
const FIRST_WINDOW_BUDGET: Duration = Duration::from_secs(5);

/// How long a later launch may take to hand its command line to the
/// running instance and exit.
const HANDOVER_BUDGET: Duration = Duration::from_secs(1);

/// How long another window may take to show its files while the app
/// runs, from the moment the later launch is started.
const ANOTHER_WINDOW_BUDGET: Duration = Duration::from_secs(4);

/// How long anything may take before the test gives up waiting.
const PATIENCE: Duration = Duration::from_secs(60);

/// Whether the test runs in check.py's isolation (see `launch.rs`).
fn isolated() -> bool {
    let temporary = std::env::temp_dir();
    let is_private = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .is_some_and(|path| path.starts_with(&temporary))
    };
    std::env::var_os("OX_ISOLATED_SESSION").is_some_and(|marker| marker == "1")
        && temporary != Path::new("/tmp")
        && ["HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME"]
            .into_iter()
            .all(is_private)
        && std::env::var_os("DISPLAY").is_some()
}

/// Microseconds on the monotonic clock the app's trace uses.
fn now() -> i64 {
    gio::glib::monotonic_time()
}

/// The running app, its traced steps as they arrive, stopped when the
/// test ends however it ends.
struct TracedApp {
    child: Child,
    /// Each step's times, in the order written.
    steps: Arc<Mutex<HashMap<String, Vec<i64>>>>,
}

impl TracedApp {
    fn start(arguments: &[&str]) -> Self {
        let mut child = Command::new(APP)
            .args(arguments)
            .env("OX_STARTUP_TRACE", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the app starts");
        let steps: Arc<Mutex<HashMap<String, Vec<i64>>>> = Arc::default();
        let stderr = child.stderr.take().expect("standard error is piped");
        let collected = Arc::clone(&steps);
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let mut words = line.split(' ');
                if words.next() != Some("ox-startup") {
                    eprintln!("{line}");
                    continue;
                }
                if let (Some(step), Some(Ok(time))) = (words.next(), words.next().map(str::parse::<i64>)) {
                    let mut steps = collected.lock().expect("the steps can be locked");
                    steps.entry(step.to_owned()).or_default().push(time);
                }
            }
        });
        Self { child, steps }
    }

    /// When `step` was written for the `nth` time (from 0), waiting for it.
    fn step(&mut self, step: &str, nth: usize) -> i64 {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let time = self
                .steps
                .lock()
                .expect("the steps can be locked")
                .get(step)
                .and_then(|times| times.get(nth).copied());
            if let Some(time) = time {
                return time;
            }
            let exited = self.child.try_wait().expect("the process can be asked");
            assert!(exited.is_none(), "the app exited ({exited:?}) before {step}");
            assert!(Instant::now() < deadline, "timed out waiting for {step}");
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for TracedApp {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Runs the app with `arguments` to its end, tracing its steps, and
/// returns how long it took and the steps it wrote.
fn run_timed(arguments: &[&str]) -> (Duration, Vec<String>) {
    let started = Instant::now();
    let mut child = Command::new(APP)
        .args(arguments)
        .env("OX_STARTUP_TRACE", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the app starts");
    let stderr = child.stderr.take().expect("standard error is piped");
    let reader = thread::spawn(move || {
        BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| {
                let step = line.strip_prefix("ox-startup ")?.split(' ').next()?;
                Some(step.to_owned())
            })
            .collect::<Vec<String>>()
    });
    loop {
        if let Some(status) = child.try_wait().expect("the process can be asked") {
            assert!(status.success(), "openxplorer {arguments:?} ended with {status}");
            let elapsed = started.elapsed();
            return (elapsed, reader.join().expect("standard error is read"));
        }
        assert!(
            started.elapsed() < PATIENCE,
            "openxplorer {arguments:?} did not finish"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn milliseconds(microseconds: i64) -> Duration {
    Duration::from_micros(u64::try_from(microseconds).expect("steps follow the launch"))
}

/// The first window shows its folder's files within the budget, and
/// another window, opened while the app runs, does too: the later launch
/// hands its command line to the running instance without reading the
/// whole executable first, which it would trace as `executable-digest`.
///
/// parity: INT-001, PERF-003
#[test]
fn windows_open_within_their_budgets() {
    if !isolated() {
        eprintln!("skipped: run inside native/tools/check.py, which gives the app a private session");
        return;
    }
    let folder = tempfile::tempdir().expect("a temporary folder");
    for number in 0..200 {
        std::fs::write(folder.path().join(format!("report {number:03}.txt")), "text").expect("a file");
    }
    let folder = folder.path().to_str().expect("a UTF-8 path").to_owned();

    let launched = now();
    let mut app = TracedApp::start(&[&folder]);
    let listed = milliseconds(app.step("listed", 0) - launched);
    let painted = milliseconds(app.step("painted", 0) - launched);
    eprintln!("first window: painted after {painted:?}, listed after {listed:?}");
    assert!(
        listed <= FIRST_WINDOW_BUDGET,
        "the first window listed its folder after {listed:?}, more than {FIRST_WINDOW_BUDGET:?}"
    );
    app.step("idle", 0);

    for nth in 0..2 {
        let launched = now();
        let (launcher, steps) = run_timed(&["--new-window", &folder]);
        assert!(
            !steps.iter().any(|step| step == "executable-digest"),
            "the later launch read its whole executable before handing its command line over: {steps:?}"
        );
        let listed = milliseconds(app.step("window-listed", nth) - launched);
        eprintln!("another window: launcher done after {launcher:?}, listed after {listed:?}");
        assert!(
            launcher <= HANDOVER_BUDGET,
            "the later launch took {launcher:?} to hand its command line over, more than {HANDOVER_BUDGET:?}"
        );
        assert!(
            listed <= ANOTHER_WINDOW_BUDGET,
            "another window listed its folder after {listed:?}, more than {ANOTHER_WINDOW_BUDGET:?} \
             (the launcher took {launcher:?})"
        );
    }

    run_timed(&["--quit"]);
    let deadline = Instant::now() + PATIENCE;
    while app.child.try_wait().expect("the process can be asked").is_none() {
        assert!(Instant::now() < deadline, "the app did not quit");
        thread::sleep(Duration::from_millis(50));
    }
}
