// SPDX-License-Identifier: AGPL-3.0-only
//! Mount assistant cases, including
//! `test_mapped_path_plan_requires_admin_not_automatic` of
//! `v2.0.0:desktop/tests/test_v05.py`. Expected plans were produced by
//! `mount_plan` in `v2.0.0:desktop/mount_support.py`, so the helper finds units
//! written by either app.

use std::path::PathBuf;

use super::*;

/// The first desktop account of a Zorin installation.
const DESKTOP_USER: DesktopUser = DesktopUser { uid: 1000, gid: 1000 };

fn plan(address: &str) -> MountPlan {
    mount_plan(address, DESKTOP_USER).expect("a plannable share")
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::SettingsWindowsTests::test_mapped_path_plan_requires_admin_not_automatic`
///
/// parity: NET-027, SAFE-021
#[test]
fn the_plan_needs_an_administrator_and_holds_no_password() {
    let plan = plan("smb://nas/Downloads");

    assert!(plan.command.contains("sudo"), "{}", plan.command);
    assert!(!plan.command.contains("password="), "{}", plan.command);
}

/// parity: NET-027, NET-028
#[test]
fn the_plan_matches_the_python_assistant() {
    let plan = plan("smb://nas/Downloads");

    let expected = MountPlan {
        share: "//nas/Downloads".into(),
        key: "u1000sd14e42e53b".into(),
        mountpoint: PathBuf::from("/mnt/winspace/u1000sd14e42e53b"),
        target_path: PathBuf::from("/mnt/winspace/u1000sd14e42e53b"),
        unit: "mnt-winspace-u1000sd14e42e53b".into(),
        credentials: PathBuf::from("/etc/winspace/mount-credentials/u1000sd14e42e53b"),
        mount_unit: "# Managed by OpenXplorer's explicit mount setup tool.\n[Unit]\n\
                     Description=OpenXplorer SMB mount u1000sd14e42e53b\n[Mount]\nWhat=//nas/Downloads\n\
                     Where=/mnt/winspace/u1000sd14e42e53b\nType=cifs\n\
                     Options=credentials=/etc/winspace/mount-credentials/u1000sd14e42e53b,uid=1000,gid=1000,\
                     file_mode=0600,dir_mode=0700,forceuid,forcegid,nosuid,nodev,noexec,vers=3.0,_netdev\n\
                     TimeoutSec=20\n"
            .into(),
        automount_unit: "# Managed by OpenXplorer's explicit mount setup tool.\n[Unit]\n\
                         Description=OpenXplorer on-demand SMB mount u1000sd14e42e53b\n[Automount]\n\
                         Where=/mnt/winspace/u1000sd14e42e53b\nTimeoutIdleSec=300\n[Install]\n\
                         WantedBy=multi-user.target\n"
            .into(),
        command: "sudo /usr/bin/openxplorer-mount-share --share //nas/Downloads".into(),
        remove_command: "sudo /usr/bin/openxplorer-mount-share --share //nas/Downloads --remove".into(),
    };
    assert_eq!(plan, expected);
}

/// parity: NET-027
#[test]
fn a_share_with_spaces_is_quoted_and_its_subfolder_kept() {
    let user = DesktopUser { uid: 1000, gid: 1001 };

    let plan = mount_plan(r"\\NAS\My Share\Sub dir", user).expect("a plannable share");

    assert_eq!(plan.share, "//nas/My Share");
    assert_eq!(plan.key, "u1000sfef7703171");
    assert_eq!(
        plan.target_path,
        PathBuf::from("/mnt/winspace/u1000sfef7703171/Sub dir")
    );
    assert_eq!(
        plan.command,
        "sudo /usr/bin/openxplorer-mount-share --share '//nas/My Share'"
    );
    assert!(
        plan.mount_unit.contains(",uid=1000,gid=1001,"),
        "{}",
        plan.mount_unit
    );
}

struct Refusal {
    address: &'static str,
    uid: u32,
    error: MountPlanError,
}

/// parity: NET-027, SAFE-021
#[test]
fn unsupported_shares_are_refused_in_the_assistant_wording() {
    let cases = [
        Refusal {
            address: "smb://nas:1445/share",
            uid: 1000,
            error: MountPlanError::UnsupportedServer,
        },
        Refusal {
            address: "smb://nas_x/share",
            uid: 1000,
            error: MountPlanError::UnsupportedServer,
        },
        Refusal {
            address: "smb://nas/sh@re",
            uid: 1000,
            error: MountPlanError::UnsupportedShareName,
        },
        Refusal {
            address: "smb://nas/share",
            uid: 0,
            error: MountPlanError::InvalidDestination,
        },
    ];
    for case in cases {
        let user = DesktopUser {
            uid: case.uid,
            gid: 1000,
        };

        let refused = mount_plan(case.address, user);

        assert_eq!(refused, Err(case.error), "{}", case.address);
    }
    assert_eq!(
        MountPlanError::UnsupportedServer.to_string(),
        "The persistent mount assistant supports a hostname or IPv4 address without a port."
    );
    assert_eq!(
        MountPlanError::UnsupportedShareName.to_string(),
        "This share name needs manual mounting. The assistant allows letters, numbers, spaces, dots, _, $, and \
         hyphens."
    );
}

#[test]
fn only_shared_folders_can_be_planned() {
    let refused = mount_plan("smb://nas/", DESKTOP_USER);

    assert!(matches!(refused, Err(MountPlanError::Location(_))), "{refused:?}");
}

#[test]
fn shell_quoting_matches_python_shlex() {
    assert_eq!(shell_quote("//nas/Downloads"), "//nas/Downloads");
    assert_eq!(shell_quote(""), "''");
    assert_eq!(shell_quote("it's"), r#"'it'"'"'s'"#);
    assert_eq!(shell_quote("$HOME"), "'$HOME'");
}
