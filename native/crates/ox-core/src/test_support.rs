// SPDX-License-Identifier: AGPL-3.0-only
//! Fixtures shared by the unit tests of several modules.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

/// The permission bits of `path` itself, not of a link's target, for
/// example `0o600`, including the set-id and sticky bits.
pub(crate) fn permission_bits(path: &Path) -> u32 {
    fs::symlink_metadata(path).expect("the path exists").mode() & 0o7777
}

/// A new temporary folder for one test, removed when dropped.
pub(crate) fn temporary_folder() -> TempDir {
    tempfile::tempdir().expect("the test home has room for a temporary folder")
}

/// Creates a named pipe (FIFO) at `path` with the system `mkfifo`, for the
/// safety tests that check that a FIFO in place of a file never blocks.
pub(crate) fn make_fifo(path: &Path) {
    let status = Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("mkfifo (GNU coreutils) is required for the FIFO safety tests");
    assert!(
        status.success(),
        "mkfifo failed to create the FIFO fixture: {status}"
    );
}

/// Where a test that needs its own mount namespace runs. The test calls
/// this first: in the test process it runs the test again in a new user
/// and mount namespace (`unshare`), checks that it passed there and returns
/// `false`; in that namespace it returns `true`, and the test can bind-mount
/// folders without root and without changing the machine's mounts. Where
/// unprivileged user namespaces are blocked (Ubuntu's AppArmor setting
/// `kernel.apparmor_restrict_unprivileged_userns`), it says so and returns
/// `false` without running the test.
///
/// `test` is the test's `module_path!()` and name, joined by `::`.
pub(crate) fn in_private_mount_namespace(test: &str) -> bool {
    const RUNNING: &str = "OX_TEST_IN_MOUNT_NAMESPACE";
    let test = test.strip_prefix("ox_core::").unwrap_or(test);
    if std::env::var(RUNNING).is_ok_and(|running| running == test) {
        return true;
    }
    let namespace = ["--user", "--map-root-user", "--mount", "--"];
    let available = Command::new("unshare")
        .args(namespace)
        .arg("true")
        .output()
        .is_ok_and(|probe| probe.status.success());
    if !available {
        eprintln!("skipped {test}: unprivileged user and mount namespaces are not available here");
        return false;
    }
    let test_binary = std::env::current_exe().expect("the test binary has a path");
    let run = Command::new("unshare")
        .args(namespace)
        .arg(test_binary)
        .args(["--exact", test, "--test-threads=1", "--nocapture"])
        .env(RUNNING, test)
        .output()
        .expect("unshare runs the test binary");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        run.status.success(),
        "{test} failed in its mount namespace:\n{output}"
    );
    assert!(
        output.contains("1 passed"),
        "{test} did not run in its mount namespace:\n{output}"
    );
    false
}

/// A bind mount of `source` on `target` in the test's own mount namespace,
/// undone when dropped so the temporary folder can be removed.
pub(crate) struct BindMount(std::path::PathBuf);

impl BindMount {
    /// Binds `source` on `target`; call only where
    /// [`in_private_mount_namespace`] returned `true`.
    pub(crate) fn new(source: &Path, target: &Path) -> Self {
        let status = Command::new("mount")
            .arg("--bind")
            .arg(source)
            .arg(target)
            .status()
            .expect("mount (util-linux) is required for the mount tests");
        assert!(status.success(), "the bind mount failed: {status}");
        Self(target.to_path_buf())
    }
}

impl Drop for BindMount {
    fn drop(&mut self) {
        let _ = Command::new("umount").arg(&self.0).status();
    }
}
