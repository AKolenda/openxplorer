// SPDX-License-Identifier: AGPL-3.0-only
//! Removing the mount's configuration.

use super::*;

/// Removal needs REMOVE and units exactly as written; it deletes the
/// configuration and an empty mount point, never anything else.
///
/// parity: NET-028
#[test]
fn removal_needs_unchanged_units_and_deletes_only_the_configuration() {
    let root = FakeRoot::new();
    root.set_up();
    let mut systemd = RecordedSystemd::default();
    let mut terminal = ScriptedTerminal::answering(&["remove"]);
    let kept = run(
        &arguments(true, false),
        &account(),
        &mut root.host(&mut systemd),
        &mut terminal,
    );
    assert_eq!(kept.expect("cancelled"), Outcome::Cancelled);
    assert!(root.credentials().exists());

    let mut terminal = ScriptedTerminal::answering(&["REMOVE"]);
    let removed = run(
        &arguments(true, false),
        &account(),
        &mut root.host(&mut systemd),
        &mut terminal,
    );

    assert_eq!(removed.expect("removed"), Outcome::Done);
    for path in [
        root.credentials(),
        root.mount_unit(),
        root.automount_unit(),
        root.mountpoint(),
    ] {
        assert!(!path.exists(), "{} was removed", path.display());
    }
    assert_eq!(
        systemd.commands,
        [
            format!("stop {UNIT}.mount {UNIT}.automount"),
            format!("disable {UNIT}.automount"),
            "daemon-reload".to_owned(),
        ]
    );

    root.set_up();
    fs::write(root.automount_unit(), "edited").expect("edited unit");
    let mut terminal = ScriptedTerminal::answering(&["REMOVE"]);
    let refused = run(
        &arguments(true, false),
        &account(),
        &mut root.host(&mut systemd),
        &mut terminal,
    );
    assert!(
        matches!(refused, Err(MountHelperError::EditedUnits)),
        "{refused:?}"
    );
    assert!(root.credentials().exists());
}
