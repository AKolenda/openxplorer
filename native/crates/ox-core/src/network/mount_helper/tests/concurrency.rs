// SPDX-License-Identifier: AGPL-3.0-only
//! Competing helpers and failed exclusive creates in a synthetic tree.

use super::*;

#[test]
fn setup_and_removal_lock_out_competitors_during_confirmation() {
    for removing in [false, true] {
        let root = FakeRoot::new();
        if removing {
            root.set_up();
        }
        let mut terminal = ScriptedTerminal::answering(if removing {
            &["REMOVE"]
        } else {
            &["SETUP", "sam", "secret"]
        });
        terminal.on_ask = Some(Box::new(|| {
            for competing_removal in [false, true] {
                let mut systemd = RecordedSystemd::default();
                let mut competing_terminal = ScriptedTerminal::default();
                let refused = run(
                    &arguments(competing_removal, false),
                    &account(),
                    &mut root.host(&mut systemd),
                    &mut competing_terminal,
                );
                assert!(matches!(refused, Err(MountHelperError::Busy)), "{refused:?}");
                assert!(systemd.commands.is_empty());
                assert!(!competing_terminal.printed.contains("Type "));
            }
            let mut systemd = RecordedSystemd::default();
            let plan = run(
                &arguments(false, true),
                &account(),
                &mut root.host(&mut systemd),
                &mut ScriptedTerminal::default(),
            );
            assert_eq!(plan.expect("plans do not take the lock"), Outcome::Done);
            assert!(systemd.commands.is_empty());
        }));
        let mut systemd = RecordedSystemd::default();

        let result = run(
            &arguments(removing, false),
            &account(),
            &mut root.host(&mut systemd),
            &mut terminal,
        );

        assert_eq!(result.expect("original operation"), Outcome::Done);
        let credential_folder = root.credentials().parent().expect("parent").to_path_buf();
        assert!(credential_folder.is_dir(), "removal must preserve the lock inode");
        let lock = root.tree().lock_directory(&credential_folder);
        assert!(lock.is_ok(), "the operation releases its lock on return");
    }
}

#[test]
fn losing_mountpoint_creation_does_not_stop_or_delete_the_existing_mount() {
    let root = FakeRoot::new();
    let mut systemd = RecordedSystemd::default();
    let mut terminal = ScriptedTerminal::answering(&["SETUP", "sam", "secret"]);
    terminal.on_ask = Some(Box::new(|| {
        // Model an administrator or older helper that does not take the
        // lock, winning the exclusive create after our preflight check.
        fs::create_dir(root.mountpoint()).expect("competing mount point");
        for path in [root.credentials(), root.mount_unit(), root.automount_unit()] {
            fs::write(path, "other setup").expect("competing file");
        }
    }));

    let failed = run(
        &arguments(false, false),
        &account(),
        &mut root.host(&mut systemd),
        &mut terminal,
    );

    assert!(matches!(failed, Err(MountHelperError::Io { .. })));
    assert!(systemd.commands.is_empty());
    assert!(root.mountpoint().is_dir());
    for path in [root.credentials(), root.mount_unit(), root.automount_unit()] {
        assert_eq!(
            fs::read_to_string(path).expect("existing file remains"),
            "other setup"
        );
    }
}

#[test]
fn losing_a_file_create_cleans_only_our_files_without_calling_systemd() {
    for collision in 0..3 {
        let root = FakeRoot::new();
        let paths = [root.credentials(), root.mount_unit(), root.automount_unit()];
        let mut systemd = RecordedSystemd {
            failing: Some("stop"),
            ..RecordedSystemd::default()
        };
        let mut terminal = ScriptedTerminal::answering(&["SETUP", "sam", "secret"]);
        terminal.on_ask = Some(Box::new(|| {
            fs::write(&paths[collision], "other file").expect("competing file");
        }));

        let failed = run(
            &arguments(false, false),
            &account(),
            &mut root.host(&mut systemd),
            &mut terminal,
        );

        assert!(matches!(failed, Err(MountHelperError::Io { .. })));
        assert!(systemd.commands.is_empty());
        assert!(!root.mountpoint().exists());
        for (index, path) in paths.iter().enumerate() {
            if index == collision {
                assert_eq!(
                    fs::read_to_string(path).expect("existing file remains"),
                    "other file"
                );
            } else {
                assert!(!path.exists(), "only our completed files are removed");
            }
        }
    }
}
