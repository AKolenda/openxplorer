// SPDX-License-Identifier: AGPL-3.0-only
//! The parsers that cross a privilege or network boundary: the command
//! lines of the root helpers (mounting a share, deleting as
//! administrator, installing an update), the planned mount of a typed
//! share, the update server's release and trusted addresses, and the
//! account and share lists of the system, with search words and saved
//! state.
//!
//! The first byte of the input picks the parser; the rest is its input.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

use crate::network::{mount_plan, DesktopUser};
use crate::update::{parse_release, ReleaseVersion, TrustedUrl};

/// The parsers, in the order the first byte picks them.
const PARSERS: u8 = 9;

/// Runs the parser the first byte of `data` picks on the rest.
///
/// # Panics
///
/// When a mount plan holds text the typed share smuggled past its rules.
pub fn parse(data: &[u8]) {
    let Some((&which, input)) = data.split_first() else {
        return;
    };
    let text = String::from_utf8_lossy(input);
    match which % PARSERS {
        0 => {
            for line in text.lines().take(8) {
                plan(line);
            }
        }
        1 => {
            let words: Vec<String> = text.split('\0').map(str::to_owned).collect();
            crate::network::fuzz_mount_helper(words, &text);
        }
        2 => {
            let words: Vec<OsString> = input
                .split(|&byte| byte == 0)
                .map(|word| OsString::from_vec(word.to_vec()))
                .collect();
            crate::admin_delete::fuzz_request(words.clone());
            crate::update::fuzz_install_request(words);
        }
        3 => {
            if let Ok(answer) = serde_json::from_slice::<serde_json::Value>(input) {
                let current: ReleaseVersion = "2.0.1".parse().expect("a version");
                let _ = parse_release(&answer, current);
            }
        }
        4 => {
            for line in text.lines().take(8) {
                let _ = TrustedUrl::parse(line);
                let _ = line.parse::<ReleaseVersion>();
            }
        }
        5 => {
            let (words, name) = text.split_once('\n').unwrap_or((&text, "Report 2024.final.pdf"));
            let _ = crate::search::TypedQuery::parse(words);
            let pattern = crate::search::NamePattern::new(words);
            let _ = pattern.matches(name);
            crate::search::fuzz_query(words, &name.to_lowercase());
        }
        6 => {
            crate::permissions::fuzz_accounts(&text);
            crate::network::fuzz_usershares(&text);
        }
        7 => {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(input) {
                let _ = crate::session::SavedSession::from_json(&value);
                let _ = crate::session::TabSnapshot::from_json(&value);
            }
        }
        _ => {
            let _ = crate::search::is_wildcard(&text);
        }
    }
}

/// The mount a typed share asks for, checked against the promises of
/// `mount_plan` (SAFE-021): only a plain host and share name reach the
/// unit files and mount options, and the location stays in the mount.
fn plan(line: &str) {
    let user = DesktopUser { uid: 1000, gid: 1000 };
    let Ok(plan) = mount_plan(line, user) else {
        return;
    };
    let (server, share) = plan
        .share
        .strip_prefix("//")
        .and_then(|rest| rest.split_once('/'))
        .expect("the share is //server/share");
    assert!(
        server
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-".contains(c)),
        "server {server:?} from {line:?}"
    );
    assert!(
        share
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " ._$-".contains(c)),
        "share {share:?} from {line:?}"
    );
    let reference = mount_plan("smb://server/share", user).expect("a plain share");
    for (made, plain) in [
        (&plan.mount_unit, &reference.mount_unit),
        (&plan.automount_unit, &reference.automount_unit),
    ] {
        assert_eq!(
            made.lines().count(),
            plain.lines().count(),
            "{made:?} from {line:?}"
        );
        assert!(
            !made.chars().any(|c| c.is_control() && c != '\n'),
            "{made:?} from {line:?}"
        );
    }
    for command in [&plan.command, &plan.remove_command] {
        assert!(
            !command.chars().any(char::is_control),
            "{command:?} from {line:?}"
        );
        let words = glib::shell_parse_argv(command.as_str()).expect("the command parses");
        assert_eq!(
            words.get(3).and_then(|word| word.to_str()),
            Some(plan.share.as_str())
        );
    }
    assert!(
        plan.target_path.starts_with(&plan.mountpoint),
        "{}",
        plan.target_path.display()
    );
    assert!(
        !plan.target_path.components().any(|part| matches!(
            part,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )),
        "{} from {line:?}",
        plan.target_path.display()
    );
}
