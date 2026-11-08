// SPDX-License-Identifier: AGPL-3.0-only
//! "Install updates without asking for a password", which is off by
//! default: the update helper turns it on and off only as root. New in the
//! native app.
//!
//! The helper runs here as the ordinary test user, so these tests prove it
//! refuses to change anything for anyone but root. Writing and deleting
//! the rule is tested in `ox_core::update::passwordless` on a fake system,
//! and the package's files in `native/tools/test_package_data.py`.

use std::process::Command;

use ox_core::update::{PASSWORDLESS_RECORD, PASSWORDLESS_RULE};

const HELPER: &str = env!("CARGO_BIN_EXE_openxplorer-install-update");

/// Whether this test runs as root, where running the helper would change
/// the real system: these tests then do nothing.
fn is_root() -> bool {
    rustix::process::geteuid().is_root()
}

/// A caller that is not root is refused before anything is read or
/// written, whichever way it asks.
/// parity: UPD-003
#[test]
fn a_caller_that_is_not_root_cannot_change_the_setting() {
    if is_root() {
        return;
    }
    let rule_before = std::fs::symlink_metadata(PASSWORDLESS_RULE).is_ok();
    let record_before = std::fs::symlink_metadata(PASSWORDLESS_RECORD).is_ok();
    for argument in ["--enable-passwordless", "--disable-passwordless"] {
        let output = Command::new(HELPER).arg(argument).output().unwrap();

        assert_eq!(output.status.code(), Some(2), "{argument}");
        let message = String::from_utf8_lossy(&output.stderr);
        assert!(message.contains("runs only as root"), "{argument}: {message}");
    }
    assert_eq!(std::fs::symlink_metadata(PASSWORDLESS_RULE).is_ok(), rule_before);
    assert_eq!(
        std::fs::symlink_metadata(PASSWORDLESS_RECORD).is_ok(),
        record_before
    );
}

/// Anything with the setting's arguments is refused as a request, before
/// the helper even looks who started it.
#[test]
fn the_setting_takes_no_path_or_second_argument() {
    if is_root() {
        return;
    }
    let refused: [&[&str]; 4] = [
        &["--enable-passwordless", "/tmp/fixture.rules"],
        &["--disable-passwordless", "--enable-passwordless"],
        &["--enable-passwordless=/tmp/fixture.rules"],
        &["--enable"],
    ];
    for arguments in refused {
        let output = Command::new(HELPER).args(arguments).output().unwrap();

        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        let message = String::from_utf8_lossy(&output.stderr);
        assert!(!message.contains("runs only as root"), "{arguments:?}: {message}");
    }
}

/// The usage text names the setting's two arguments.
#[test]
fn the_usage_names_both_arguments() {
    let output = Command::new(HELPER).arg("--help").output().unwrap();

    assert!(output.status.success());
    let usage = String::from_utf8_lossy(&output.stdout);
    assert!(usage.contains("--enable-passwordless"), "{usage}");
    assert!(usage.contains("--disable-passwordless"), "{usage}");
}
