// SPDX-License-Identifier: AGPL-3.0-only
//! Mounting on demand and Map network location, without a server: the
//! isolated test session has none, so the mount step is simulated where a
//! test needs one to succeed.

use std::cell::Cell;
use std::future::{self, Ready};

use super::*;
use crate::network::test_support::with_prompts;

/// A read that fails with `failures` in turn, then succeeds, counting its
/// calls.
struct ScriptedRead {
    failures: Vec<EntryError>,
    calls: Cell<usize>,
}

impl ScriptedRead {
    fn failing_with(failures: Vec<EntryError>) -> Self {
        Self {
            failures,
            calls: Cell::new(0),
        }
    }

    /// Reads once: the next scripted failure, or the listing.
    fn run(&self) -> Ready<Result<&'static str, EntryError>> {
        let call = self.calls.get();
        self.calls.set(call + 1);
        let result = match self.failures.get(call) {
            Some(failure) => Err(failure.clone()),
            None => Ok("listed"),
        };
        future::ready(result)
    }
}

fn not_mounted() -> EntryError {
    EntryError::NotMounted("The share is not mounted".into())
}

/// Runs `retry_after_mount` with a mount step that ends with `mount_result`;
/// returns the result and how often the mount step ran.
fn retry(
    read: &ScriptedRead,
    mount_result: Result<(), NetworkError>,
) -> (Result<&'static str, MountedReadError<EntryError>>, usize) {
    let mounts = Cell::new(0);
    let mount = || {
        mounts.set(mounts.get() + 1);
        async move { mount_result }
    };
    let result = glib::MainContext::new().block_on(retry_after_mount(mount, || read.run()));
    (result, mounts.get())
}

/// parity: NET-004
#[test]
fn a_read_of_an_unmounted_share_mounts_it_and_reads_again() {
    let read = ScriptedRead::failing_with(vec![not_mounted()]);

    let (result, mounts) = retry(&read, Ok(()));

    assert!(matches!(result, Ok("listed")), "{result:?}");
    assert_eq!(mounts, 1);
    assert_eq!(read.calls.get(), 2);
}

/// parity: NET-004
#[test]
fn a_share_is_mounted_at_most_once_per_read() {
    let read = ScriptedRead::failing_with(vec![not_mounted(), not_mounted()]);

    let (result, mounts) = retry(&read, Ok(()));

    assert!(
        matches!(result, Err(MountedReadError::Read(EntryError::NotMounted(_)))),
        "{result:?}"
    );
    assert_eq!(mounts, 1);
    assert_eq!(read.calls.get(), 2);
}

/// parity: NET-004
#[test]
fn other_read_failures_mount_nothing() {
    let denied = EntryError::PermissionDenied("Permission denied".into());
    let read = ScriptedRead::failing_with(vec![denied]);

    let (result, mounts) = retry(&read, Ok(()));

    assert!(
        matches!(
            result,
            Err(MountedReadError::Read(EntryError::PermissionDenied(_)))
        ),
        "{result:?}"
    );
    assert_eq!(mounts, 0);
    assert_eq!(read.calls.get(), 1);
}

/// parity: NET-004
#[test]
fn a_failed_mount_is_reported_without_reading_again() {
    let read = ScriptedRead::failing_with(vec![not_mounted()]);

    let (result, mounts) = retry(&read, Err(NetworkError::Cancelled));

    assert!(
        matches!(result, Err(MountedReadError::Mount(NetworkError::Cancelled))),
        "{result:?}"
    );
    assert_eq!(mounts, 1);
    assert_eq!(read.calls.get(), 1);
}

/// Regression: the Python app took `GVfs`'s base name, which is `/` for a
/// share root, so a share mapped without a display name was saved with the
/// sidebar label "/".
///
/// parity: NET-001, NET-017
#[test]
fn a_mapped_share_is_named_after_its_folder() {
    assert_eq!(share_name("smb://nas/Projects"), "Projects");
    assert_eq!(share_name("smb://nas/Design%20files"), "Design files");
    assert_eq!(share_name("smb://nas/Projects/Film"), "Film");
}

struct RefusedAddress {
    address: &'static str,
    label: &'static str,
    expected: &'static str,
}

/// Map network location refuses a bad address or label before anything
/// is mounted, and the dialog shows the message.
///
/// parity: NET-001
#[test]
fn map_network_location_refuses_bad_input_before_mounting() {
    let cases = [
        RefusedAddress {
            address: r"\\nas",
            label: "",
            expected: "Enter a shared folder such as \\\\nas\\Projects, not only the server name.",
        },
        RefusedAddress {
            address: "smb://nas/Projects",
            label: "Line\nbreak",
            expected: "A sidebar label must be at most 120 characters and contain no control characters.",
        },
    ];
    with_prompts(|fixture| {
        let registry = SignOutRegistry::default();
        for case in &cases {
            let refused = fixture.block_on(connect_share(
                &fixture.prompts,
                &registry,
                case.address,
                case.label,
            ));

            let message = refused.expect_err("the input is refused").to_string();
            assert_eq!(message, case.expected, "{}", case.address);
        }
        assert_eq!(fixture.prompter.shown_count(), 0, "no sign-in was asked for");
    });
}

#[test]
fn an_invalid_address_is_not_mounted() {
    with_prompts(|fixture| {
        let refused = fixture.block_on(mount_location(&fixture.prompts, "https://example.invalid/"));

        assert!(matches!(refused, Err(NetworkError::Location(_))), "{refused:?}");
    });
}
