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
/// unprivileged user namespaces are blocked (the `AppArmor` setting
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

/// A staging item on a simulated phone (`GVfs` MTP), or the folder holding
/// it. Asking about or deleting the item answers "not found", which on a
/// phone may be false, so only the folder's listing tells whether the item
/// is really gone: it lists the item while `listed` is true. Only what
/// staging cleanup asks is answered.
#[derive(Clone, Copy)]
pub(crate) struct PhoneStage {
    /// The staging item's name.
    name: &'static str,
    /// True for the folder holding the item, false for the item.
    folder: bool,
    /// Whether the folder's listing still shows the item.
    listed: bool,
}

impl PhoneStage {
    /// The staging item `name`, still listed by its folder when `listed`.
    pub(crate) fn item(name: &'static str, listed: bool) -> Self {
        Self {
            name,
            folder: false,
            listed,
        }
    }

    fn not_found(&self) -> crate::transfer::TransferError {
        crate::transfer::TransferError::NotFound(format!(
            "Error removing file {}: No such file",
            crate::transfer::Node::uri(self)
        ))
    }
}

impl crate::transfer::Node for PhoneStage {
    fn uri(&self) -> String {
        if self.folder {
            "mtp://phone.example/Folder".to_owned()
        } else {
            format!("mtp://phone.example/Folder/{}", self.name)
        }
    }

    fn name(&self) -> std::ffi::OsString {
        if self.folder { "Folder" } else { self.name }.into()
    }

    fn path(&self) -> Option<std::path::PathBuf> {
        None
    }

    fn child(&self, _name: &std::ffi::OsStr) -> Box<dyn crate::transfer::Node> {
        unreachable!("staging cleanup does not ask for a child")
    }

    fn parent(&self) -> Option<Box<dyn crate::transfer::Node>> {
        (!self.folder).then(|| {
            Box::new(PhoneStage {
                folder: true,
                ..*self
            }) as Box<dyn crate::transfer::Node>
        })
    }

    fn exists(&self, _cancel: Option<&crate::transfer::Cancellation>) -> bool {
        self.folder || self.listed
    }

    fn info(
        &self,
        _cancel: Option<&crate::transfer::Cancellation>,
    ) -> Result<crate::transfer::NodeInfo, crate::transfer::TransferError> {
        Err(self.not_found())
    }

    fn is_directory(
        &self,
        _cancel: Option<&crate::transfer::Cancellation>,
    ) -> Result<bool, crate::transfer::TransferError> {
        Ok(self.folder)
    }

    fn children(
        &self,
        _cancel: Option<&crate::transfer::Cancellation>,
    ) -> Result<Vec<Box<dyn crate::transfer::Node>>, crate::transfer::TransferError> {
        let item = PhoneStage {
            folder: false,
            ..*self
        };
        Ok(if self.folder && self.listed {
            vec![Box::new(item)]
        } else {
            Vec::new()
        })
    }
    fn create_directory(
        &self,
        _cancel: Option<&crate::transfer::Cancellation>,
    ) -> Result<(), crate::transfer::TransferError> {
        unreachable!("staging cleanup creates nothing")
    }

    fn copy_file(
        &self,
        _target: &dyn crate::transfer::Node,
        _cancel: &crate::transfer::Cancellation,
        _progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), crate::transfer::TransferError> {
        unreachable!("staging cleanup copies nothing")
    }

    fn move_native(
        &self,
        _target: &dyn crate::transfer::Node,
        _cancel: Option<&crate::transfer::Cancellation>,
    ) -> Result<(), crate::transfer::TransferError> {
        unreachable!("staging cleanup moves nothing")
    }

    fn replace_native(
        &self,
        _target: &dyn crate::transfer::Node,
        _cancel: Option<&crate::transfer::Cancellation>,
    ) -> Result<(), crate::transfer::TransferError> {
        unreachable!("staging cleanup replaces nothing")
    }

    fn delete(&self) -> Result<(), crate::transfer::TransferError> {
        Err(self.not_found())
    }

    fn delete_staging(
        &self,
        _created: Option<crate::transfer::ItemIdentity>,
    ) -> Result<(), crate::transfer::TransferError> {
        Err(self.not_found())
    }

    fn trash(&self, _cancel: &crate::transfer::Cancellation) -> Result<(), crate::transfer::TransferError> {
        unreachable!("staging cleanup never uses the Trash")
    }

    fn can_trash(
        &self,
        _cancel: Option<&crate::transfer::Cancellation>,
    ) -> Result<bool, crate::transfer::TransferError> {
        unreachable!("staging cleanup never uses the Trash")
    }

    fn delete_tree(
        &self,
        _cancel: &crate::transfer::Cancellation,
        _guard: Option<&crate::transfer::WriteGuard>,
    ) -> Result<(), crate::transfer::TransferError> {
        unreachable!("staging cleanup deletes by delete or delete_staging")
    }

    fn has_sibling_staging(&self) -> bool {
        true
    }
}
