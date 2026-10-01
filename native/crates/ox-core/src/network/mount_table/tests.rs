// SPDX-License-Identifier: AGPL-3.0-only
//! Mount table cases, including `test_mount_resolution` of
//! `v2.0.0:desktop/tests/test_v05.py`. The expected values were produced by
//! `v2.0.0:desktop/mount_support.py`.

use std::path::PathBuf;

use super::*;

/// A mount table with an ordinary filesystem, a CIFS share, a bind mount
/// of a share's subfolder whose names hold escaped spaces, a line without
/// the separator and a line without a source.
const MOUNT_INFO_SAMPLE: &str = "\
36 35 98:0 /mnt1 /mnt/parent rw,noatime master:1 - ext3 /dev/root rw,errors=continue
40 1 0:40 / /mnt/nas rw,relatime shared:5 - cifs //nas/share rw,vers=3.0
41 1 0:41 /sub\\040dir /mnt/bind\\040point rw - cifs //NAS/Share rw
bad line
42 1 0:42 / /x - smb3
";

fn cifs_mount(path: &str, source: &str, root: &str) -> MountEntry {
    MountEntry {
        root: root.into(),
        path: path.into(),
        filesystem: "cifs".into(),
        source: source.into(),
        options: String::new(),
    }
}

fn resolve(uri: &str, mounts: &[MountEntry]) -> Option<PathBuf> {
    resolve_smb_path(uri, mounts).expect("an SMB shared folder")
}

/// parity: NET-026
#[test]
fn mount_lines_are_parsed_and_unescaped() {
    let mounts = parse_mount_table(MOUNT_INFO_SAMPLE);

    let expected = [
        MountEntry {
            root: "/mnt1".into(),
            path: "/mnt/parent".into(),
            filesystem: "ext3".into(),
            source: "/dev/root".into(),
            options: "rw,errors=continue".into(),
        },
        MountEntry {
            options: "rw,vers=3.0".into(),
            ..cifs_mount("/mnt/nas", "//nas/share", "/")
        },
        MountEntry {
            options: "rw".into(),
            ..cifs_mount("/mnt/bind point", "//NAS/Share", "/sub dir")
        },
    ];
    assert_eq!(mounts, expected);
}

#[test]
fn octal_escapes_become_characters_and_others_stay() {
    assert_eq!(unescape_mount_field(r"a\134b\777"), "a\\b\u{1ff}");
    assert_eq!(unescape_mount_field(r"tab\011end\04"), "tab\tend\\04");
}

/// parity: NET-026
#[test]
fn smb_mounts_name_their_share_or_bound_subfolder() {
    let mounts = parse_mount_table(MOUNT_INFO_SAMPLE);

    let roots: Vec<Option<String>> = mounts.iter().map(MountEntry::remote_root).collect();

    let expected = [
        None,
        Some("smb://nas/share".into()),
        Some("smb://nas/Share/sub%20dir".into()),
    ];
    assert_eq!(roots, expected);
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::SettingsWindowsTests::test_mount_resolution`
///
/// parity: NET-026
#[test]
fn a_share_path_resolves_inside_its_cifs_mount() {
    let mounts = [cifs_mount("/mnt/nas", "//nas/share", "/")];

    let local = resolve("smb://nas/share/folder/file.pdf", &mounts);

    assert_eq!(local, Some(PathBuf::from("/mnt/nas/folder/file.pdf")));
}

/// parity: NET-026
#[test]
fn the_most_specific_mount_wins_and_host_and_share_ignore_case() {
    let mounts = parse_mount_table(MOUNT_INFO_SAMPLE);

    assert_eq!(
        resolve("smb://nas/share/sub dir/a", &mounts),
        Some(PathBuf::from("/mnt/bind point/a"))
    );
    assert_eq!(
        resolve("smb://NAS/SHARE/sub dir/a", &mounts),
        Some(PathBuf::from("/mnt/bind point/a"))
    );
    // Below the share, names keep their case: `Sub dir` is not `sub dir`.
    assert_eq!(
        resolve("smb://nas/share/Sub dir/a", &mounts),
        Some(PathBuf::from("/mnt/nas/Sub dir/a"))
    );
    assert_eq!(resolve("smb://other/share/a", &mounts), None);
}

#[test]
fn only_shared_folders_resolve() {
    let refused = resolve_smb_path("smb://nas/", &[]);
    assert!(refused.is_err());
    assert!(resolve_smb_path("file:///mnt/nas", &[]).is_err());
}

#[test]
fn the_longest_mount_point_holds_a_path_and_names_are_whole() {
    let mounts = [
        cifs_mount("/", "/dev/root", "/"),
        cifs_mount("/mnt/nas", "//nas/share", "/"),
        cifs_mount("/mnt/nas/deeper", "//nas/other", "/"),
    ];

    let holder = |path: &str| mount_for_path(path, &mounts).map(|mount| mount.path.as_str());

    assert_eq!(holder("/mnt/nas/deeper/file"), Some("/mnt/nas/deeper"));
    assert_eq!(holder("/mnt/nas"), Some("/mnt/nas"));
    assert_eq!(holder("/mnt/nas-other/file"), Some("/"));
}

/// Only kernel SMB mounts become Network rows, named after their mount
/// point (`stable` in `environment` of `v2.0.0:desktop/winspace.py`).
///
/// parity: NET-006, NET-018
#[test]
fn only_smb_mounts_become_stable_network_mounts() {
    let mounts = parse_mount_table(MOUNT_INFO_SAMPLE);

    let stable: Vec<StableMount> = mounts.iter().filter_map(MountEntry::to_stable_mount).collect();

    let expected = [
        StableMount {
            path: PathBuf::from("/mnt/nas"),
            label: String::new(),
            filesystem: "cifs".into(),
        },
        StableMount {
            path: PathBuf::from("/mnt/bind point"),
            label: String::new(),
            filesystem: "cifs".into(),
        },
    ];
    assert_eq!(stable, expected);
}
