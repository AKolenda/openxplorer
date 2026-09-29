// SPDX-License-Identifier: AGPL-3.0-only
//! The helper's file rules, its credential file and its command line.

use super::*;

/// Ported from `desktop/tests/test_terminal_security.py::AdditionalSecurityTests::test_admin_helper_checks_existing_parent_chain`
///
/// The temporary folder's parents (such as the world-writable `/tmp`,
/// or a home owned by the user) are unsafe even though the directory
/// itself looks private.
///
/// parity: NET-028, SAFE-021
#[test]
fn an_unsafe_parent_chain_is_refused() {
    let root = tempfile::tempdir().expect("temporary folder");
    let owned = root.path().join("owned");
    DirBuilder::new()
        .mode(0o700)
        .create(&owned)
        .expect("private folder");

    let refused = AdministrativeTree::system().secure_directory(&owned, 0o700);

    assert!(
        matches!(refused, Err(MountHelperError::UnsafeDirectory(_))),
        "{refused:?}"
    );
}

/// parity: SAFE-021
#[test]
fn relative_and_traversing_paths_are_refused() {
    for path in ["etc/winspace", "/etc/../tmp"] {
        let refused = AdministrativeTree::system().secure_directory(Path::new(path), 0o755);
        assert!(
            matches!(refused, Err(MountHelperError::NotAbsolute)),
            "{path}: {refused:?}"
        );
    }
}

/// parity: NET-028, SAFE-021
#[test]
fn a_new_file_gets_its_mode_and_never_replaces_anything() {
    let root = tempfile::tempdir().expect("temporary folder");
    let credential = root.path().join("credential");

    write_new_file(&credential, "username=sam\n", 0o600).expect("a new file");

    assert_eq!(mode(&credential), 0o600);
    assert!(write_new_file(&credential, "other", 0o600).is_err());
    assert_eq!(
        fs::read_to_string(&credential).expect("readable"),
        "username=sam\n"
    );
}

/// parity: SAFE-021
#[test]
fn a_symlink_is_never_followed() {
    let root = tempfile::tempdir().expect("temporary folder");
    let target = root.path().join("target");
    fs::write(&target, "unchanged").expect("target file");
    let link = root.path().join("link");
    symlink(&target, &link).expect("symlink");

    assert!(write_new_file(&link, "password=x\n", 0o600).is_err());
    assert_eq!(fs::read_to_string(&target).expect("readable"), "unchanged");
}

/// parity: NET-028
#[test]
fn the_credential_file_names_user_password_and_optional_domain() {
    assert_eq!(
        credential_file_text(" OFFICE\\sam ", "secret").expect("valid"),
        "username=sam\npassword=secret\ndomain=OFFICE\n"
    );
    assert_eq!(
        credential_file_text("sam", "secret").expect("valid"),
        "username=sam\npassword=secret\n"
    );
}

/// parity: NET-028
#[test]
fn credentials_that_would_break_the_file_are_refused() {
    let invalid = [("", "secret"), ("sam", "line\nbreak"), ("sam\0", "secret")];
    for (username, password) in invalid {
        let refused = credential_file_text(username, password);
        assert!(
            matches!(refused, Err(MountHelperError::InvalidCredentials)),
            "{username:?}"
        );
    }
    let without_user = credential_file_text("OFFICE\\", "secret");
    assert!(matches!(without_user, Err(MountHelperError::MissingUsername)));
}

/// The command line of `mount_share.py`: `--share` is required, `--plan`
/// and `--remove` are flags, and `-h` answers first.
///
/// parity: NET-028
#[test]
fn the_command_line_matches_the_python_helper() {
    let parsed = |words: &[&str]| parse(words.iter().map(|word| (*word).to_owned()));

    assert_eq!(
        parsed(&["--share", "//nas/Downloads", "--remove"]),
        Ok(Request::Run(arguments(true, false)))
    );
    assert_eq!(
        parsed(&["--plan", "--share=//nas/Downloads"]),
        Ok(Request::Run(arguments(false, true)))
    );
    assert_eq!(parsed(&["--share", "x", "-h"]), Ok(Request::Help));
    assert_eq!(parsed(&["--plan"]), Err(UsageError::MissingShare));
    assert_eq!(parsed(&["--share"]), Err(UsageError::MissingValue));
    assert_eq!(
        parsed(&["--share", "x", "--force"]),
        Err(UsageError::Unrecognized("--force".into()))
    );
}

#[test]
fn the_desktop_account_is_read_from_the_user_database() {
    assert_eq!(
        parse_passwd_entry("sam:x:1000:1000:Sam,,,:/home/sam:/bin/bash\n"),
        Some(account())
    );
    assert_eq!(parse_passwd_entry("broken"), None);
}
