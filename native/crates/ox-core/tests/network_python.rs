// SPDX-License-Identifier: AGPL-3.0-only
//! Runs the network service and the Python modules it ports over the same
//! inputs, which must give the same answers: `server_key` of
//! `desktop/session_credentials.py`, `split_identity` of
//! `desktop/auth_bridge.py`, `parse_mounts`, `remote_root`,
//! `resolve_smb_path` and `mount_plan` of `desktop/mount_support.py`, and
//! `local_path` of `desktop/native_opening.py`. The keyring items are
//! compared in `network_keyring_python.rs`.
//!
//! Each Python script reads its inputs from the JSON file named by
//! `sys.argv[1]` and prints its answers as JSON. Every file is inside a
//! temporary directory.

mod python_support;

use std::fs;
use std::path::Path;

use ox_core::network::{
    fuse_export_path, mount_plan, parse_mount_table, resolve_smb_path, split_identity, DesktopUser,
    MountEntry, MountPlan, ServerKey,
};
use python_support::{as_array, python_answers, python_answers_with};
use serde_json::{json, Value};

/// Prints `server_key(uri)` for every input, or `error` where it raises.
const PYTHON_SERVER_KEYS: &str = r"
import json, sys
from session_credentials import server_key
answers = []
for uri in json.load(open(sys.argv[1])):
    try:
        key = server_key(uri)
        answers.append(list(key) if key else None)
    except (ValueError, TypeError):
        answers.append('error')
print(json.dumps(answers))
";

/// Ported from `desktop/tests/test_v05.py::CredentialsTests::test_host_key_not_share_key`,
/// `test_distinct_ports` and `test_no_alias_sharing`, over more addresses.
///
/// Where Python raises for an address that is not a valid location, the
/// Rust key is `None`: such an address has no server to sign in to.
///
/// parity: NET-014
#[test]
fn server_keys_match_session_credentials_py() {
    let uris = [
        "smb://NAS/a",
        "smb://nas/b",
        "smb://nas:445/a",
        "smb://nas:1445/a",
        "smb://nas:0/a",
        "smb://nas.local/a",
        "smb://10.0.0.1/a",
        "smb://[fe80::1]/share",
        r"\\Studio-NAS\Projects",
        "smb://nas",
        "smb://",
        "file:///home/demo",
        "mtp://[usb:001,010]/",
        "smb://user@nas/share",
        "https://example.invalid/",
    ];

    let expected = python_answers(PYTHON_SERVER_KEYS, &json!(uris));

    for (uri, python) in uris.iter().zip(as_array(&expected)) {
        let key = ServerKey::for_location(uri);
        let native = key.map_or(Value::Null, |key| json!([key.host(), key.port().to_string()]));
        let python = if python == "error" { &Value::Null } else { python };
        assert_eq!(&native, python, "{uri}");
    }
}

/// Prints `split_identity(username, default_domain)`, or the error message.
const PYTHON_SPLIT_IDENTITY: &str = r"
import json, sys
from auth_bridge import split_identity
answers = []
for username, domain in json.load(open(sys.argv[1])):
    try:
        answers.append({'identity': list(split_identity(username, domain))})
    except ValueError as error:
        answers.append({'error': str(error)})
print(json.dumps(answers))
";

/// Ported from `desktop/tests/test_v05.py::CredentialsTests::test_domain_username_supported`,
/// over the refused and edge cases of the sign-in dialog too.
///
/// parity: NET-011
#[test]
fn user_names_split_as_in_auth_bridge_py() {
    let longest = "a".repeat(512);
    let too_long = "a".repeat(513);
    let cases = [
        ("OFFICE\\sam", ""),
        ("  OFFICE\\sam  ", "WORKGROUP"),
        ("  sam ", "WORKGROUP"),
        ("sam", ""),
        ("", ""),
        ("\\sam", ""),
        ("OFFICE\\", ""),
        ("A\\B\\sam", ""),
        ("sam\nroot", ""),
        ("sam\r", ""),
        ("sam\0", ""),
        ("\tsam\u{a0}", ""),
        (longest.as_str(), ""),
        (too_long.as_str(), ""),
    ];

    let expected = python_answers(PYTHON_SPLIT_IDENTITY, &json!(cases));

    for ((username, domain), python) in cases.iter().zip(as_array(&expected)) {
        let native = match split_identity(username, domain) {
            Ok(identity) => json!({"identity": [identity.username, identity.domain]}),
            Err(error) => json!({"error": error.to_string()}),
        };
        assert_eq!(&native, python, "{username:?}");
    }
}

/// Prints `mount_plan(address, uid, gid)` or the error message.
const PYTHON_MOUNT_PLANS: &str = r"
import json, sys
from mount_support import mount_plan
answers = []
for address, uid, gid in json.load(open(sys.argv[1])):
    try:
        answers.append({'plan': mount_plan(address, uid, gid)})
    except ValueError as error:
        answers.append({'error': str(error)})
print(json.dumps(answers))
";

/// Ported from `desktop/tests/test_v05.py::SettingsWindowsTests::test_mapped_path_plan_requires_admin_not_automatic`,
/// comparing every field of every plan and every refusal.
///
/// parity: NET-027, SAFE-021
#[test]
fn mount_plans_match_mount_support_py() {
    let long_share = format!("smb://nas/{}", "s".repeat(81));
    let cases = [
        ("smb://nas/Downloads", 1000, 1000),
        (r"\\NAS\My Share\Sub dir", 1000, 1001),
        ("smb://192.168.1.10/Media$/Films", 1001, 100),
        ("smb://nas/share%2Fsub", 1000, 1000),
        ("smb://nas:1445/share", 1000, 1000),
        ("smb://nas_x/share", 1000, 1000),
        ("smb://nas/sh@re", 1000, 1000),
        ("smb://nas/.hidden", 1000, 1000),
        (long_share.as_str(), 1000, 1000),
        ("smb://nas/share", 0, 0),
        ("smb://nas/", 1000, 1000),
        ("file:///home/demo", 1000, 1000),
    ];

    let expected = python_answers(PYTHON_MOUNT_PLANS, &json!(cases));

    for ((address, uid, gid), python) in cases.iter().zip(as_array(&expected)) {
        let user = DesktopUser { uid: *uid, gid: *gid };
        let native = match mount_plan(address, user) {
            Ok(plan) => json!({"plan": plan_as_python(&plan)}),
            Err(error) => json!({"error": error.to_string()}),
        };
        assert_eq!(&native, python, "{address}");
    }
}

/// A plan in the form of the dictionary `mount_plan` returns.
fn plan_as_python(plan: &MountPlan) -> Value {
    json!({
        "share": plan.share,
        "key": plan.key,
        "mountpoint": plan.mountpoint,
        "targetPath": plan.target_path,
        "unit": plan.unit,
        "credentials": plan.credentials,
        "mountUnit": plan.mount_unit,
        "automountUnit": plan.automount_unit,
        "command": plan.command,
        "removeCommand": plan.remove_command,
    })
}

/// A mount table with ordinary filesystems, kernel SMB mounts, a bind
/// mount of a share's subfolder, escaped names and malformed lines.
const MOUNT_INFO: &str = "\
22 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw
36 35 98:0 /mnt1 /mnt/parent rw,noatime master:1 - ext3 /dev/root rw,errors=continue
40 1 0:40 / /mnt/nas rw,relatime shared:5 - cifs //nas/share rw,vers=3.0
41 1 0:41 /sub\\040dir /mnt/bind\\040point rw - cifs //NAS/Share rw
42 1 0:42 / /mnt/deep - smb3 //nas/share/Film rw
43 1 0:43 / /mnt/other rw - cifs //other:1445/data
44 1 0:44 / /media/usb rw - vfat /dev/sdb1 rw
bad line
45 1 0:45 / /x - smb3
";

/// Prints `parse_mounts`, `remote_root` of each mount, and
/// `resolve_smb_path` of each location.
const PYTHON_MOUNT_TABLE: &str = r"
import json, sys
from mount_support import parse_mounts, remote_root, resolve_smb_path
inputs = json.load(open(sys.argv[1]))
mounts = parse_mounts(inputs['table'])
resolved = []
for uri in inputs['uris']:
    try:
        resolved.append({'path': resolve_smb_path(uri, mounts)})
    except ValueError as error:
        resolved.append({'error': str(error)})
print(json.dumps({'mounts': mounts, 'roots': [remote_root(m) for m in mounts], 'resolved': resolved}))
";

/// Ported from `desktop/tests/test_v05.py::SettingsWindowsTests::test_mount_resolution`,
/// over a whole mount table.
///
/// parity: NET-026
#[test]
fn mount_table_and_smb_paths_match_mount_support_py() {
    let uris = [
        "smb://nas/share/folder/file.pdf",
        "smb://NAS/SHARE/folder",
        "smb://nas/share/sub dir/a",
        "smb://nas/share/Sub dir/a",
        "smb://nas/share/Film/cut.mp4",
        "smb://other:1445/data/x",
        "smb://other/data/x",
        "smb://unknown/share",
        "smb://nas/",
    ];
    let inputs = json!({"table": MOUNT_INFO, "uris": uris});

    let expected = python_answers(PYTHON_MOUNT_TABLE, &inputs);

    let mounts = parse_mount_table(MOUNT_INFO);
    let parsed: Vec<Value> = mounts.iter().map(mount_as_python).collect();
    assert_eq!(&Value::from(parsed), &expected["mounts"]);
    let roots: Vec<Option<String>> = mounts.iter().map(MountEntry::remote_root).collect();
    assert_eq!(&json!(roots), &expected["roots"]);
    for (uri, python) in uris.iter().zip(as_array(&expected["resolved"])) {
        let native = match resolve_smb_path(uri, &mounts) {
            Ok(path) => json!({"path": path}),
            Err(error) => json!({"error": error.to_string()}),
        };
        assert_eq!(&native, python, "{uri}");
    }
}

/// A mount in the form of the dictionaries `parse_mounts` returns.
fn mount_as_python(mount: &MountEntry) -> Value {
    json!({
        "root": mount.root,
        "path": mount.path,
        "fstype": mount.filesystem,
        "source": mount.source,
        "options": mount.options,
    })
}

/// Prints `local_path(uri)` for every input. The runtime directory, and so
/// the `GVfs` FUSE root, is the test's.
const PYTHON_LOCAL_PATHS: &str = r"
import json, sys
from native_opening import local_path
print(json.dumps([local_path(uri) for uri in json.load(open(sys.argv[1]))]))
";

/// The FUSE fallback of `local_path` finds the same export as the Python
/// app. Host names are made up so that no kernel mount of the machine
/// running the test can match first.
///
/// parity: NET-026, OPEN-006
#[test]
fn fuse_export_paths_match_native_opening_py() {
    let runtime = tempfile::tempdir().expect("temporary runtime folder");
    let fuse_root = runtime.path().join("gvfs");
    let exports = [
        "smb-share:server=parity-nas,share=projects,user=sam",
        "smb-share:port=1445,server=parity-nas,share=archive",
        "smb-share:server=parity%2dother,share=Design%20files",
        "sftp:host=parity-nas",
    ];
    for export in exports {
        fs::create_dir_all(fuse_root.join(export)).expect("export folder");
    }
    let uris = [
        "smb://parity-nas/Projects/Film/cut.mp4",
        "smb://PARITY-NAS/projects",
        "smb://parity-nas:1445/Archive/2024",
        "smb://parity-nas/archive",
        "smb://parity-other/design%20files/logo.svg",
        "smb://parity-nas/",
        "smb://unknown-parity-host/share",
    ];

    let expected = python_local_paths(runtime.path(), &uris);

    for (uri, python) in uris.iter().zip(as_array(&expected)) {
        let native = fuse_export_path(uri, &fuse_root);
        assert_eq!(&json!(native), python, "{uri}");
    }
}

/// Python's `local_path` of every one of `uris`, with `runtime` as the
/// runtime directory that holds the FUSE root.
fn python_local_paths(runtime: &Path, uris: &[&str]) -> Value {
    python_answers_with(PYTHON_LOCAL_PATHS, &json!(uris), |command| {
        command.env("XDG_RUNTIME_DIR", runtime);
    })
}
