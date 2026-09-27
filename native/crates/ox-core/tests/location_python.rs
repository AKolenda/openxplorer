// SPDX-License-Identifier: AGPL-3.0-only
//! Compatibility cases captured from desktop/core.py; see location_fixtures/README.md.

use std::path::Path;

use ox_core::location::{self, LocationError};
use serde_json::Value;

const CORE_MESSAGES: &[&str] = &[
    "A connected-device address must include a device identifier and path.",
    "Invalid connected-device identifier.",
    "Encoded control characters are not allowed.",
    "Enter a non-empty file name, not “.” or “..”.",
    "A name cannot contain slashes or control characters.",
    "This name is longer than 255 bytes.",
    "Enter a local folder path or an SMB address.",
    "Control characters are not allowed in an address.",
    "Use a server name without credentials, for example \\\\nas\\share.",
    "Windows drive letters are not Linux paths. Use /home/… or \\\\server\\share.",
    "Only local paths, smb:// locations and connected devices are supported in this build.",
    "Do not put a username or password in the address. Use the OpenXplorer sign-in dialog.",
    "In a URL, encode “?” as %3F and “#” as %23, or enter a normal file/UNC path.",
    "For network folders, use smb://server/share rather than file://server/…",
    "A file URL must contain an absolute path.",
    "Use an unescaped server name without credentials or control characters.",
    "Enter an SMB server name, for example smb://nas/Projects.",
    "Invalid SMB port.",
    "Enter a shared folder such as \\\\nas\\Projects, not only the server name.",
    "This file name is too long to generate a duplicate name.",
    "A sidebar label must be at most 120 characters and contain no control characters.",
    "Open the network share first, then select files or folders inside it. The share itself cannot be renamed, moved, copied or trashed here.",
    "Open the device storage first, then select files or folders inside it. The device itself cannot be moved or copied.",
];

fn compare(
    label: &str,
    input: &str,
    expected: &Value,
    actual: Result<String, LocationError>,
    failures: &mut Vec<String>,
) {
    let ok = match (expected.get("ok"), &actual) {
        (Some(want), Ok(got)) => want.as_str() == Some(got.as_str()),
        (None, Err(error)) => match expected["err"].as_str() {
            Some(message) if CORE_MESSAGES.contains(&message) => message == error.0,
            _ => true,
        },
        _ => false,
    };
    if !ok {
        failures.push(format!("{label} {input:?}: python {expected} rust {actual:?}"));
    }
}

#[test]
fn matches_python() {
    let text = include_str!("location_fixtures/python.json");
    let data: Value = serde_json::from_str(text).expect("json");
    let home = Path::new("/home/test");
    let mut failures = Vec::new();
    for case in data["normalise"].as_array().unwrap() {
        let value = case[0].as_str().unwrap();
        compare(
            "normalise",
            value,
            &case[2],
            location::normalise_location(value, None, home),
            &mut failures,
        );
    }
    for case in data["relative"].as_array().unwrap() {
        let value = case[0].as_str().unwrap();
        let base = case[1].as_str().unwrap();
        let label = format!("relative to {base:?}");
        compare(
            &label,
            value,
            &case[2],
            location::normalise_location(value, Some(base), home),
            &mut failures,
        );
    }
    for case in data["names"].as_array().unwrap() {
        let value = case[0].as_str().unwrap();
        compare(
            "name",
            value,
            &case[1],
            location::validate_name(value).map(str::to_string),
            &mut failures,
        );
    }
    for case in data["copies"].as_array().unwrap() {
        let value = case[0].as_str().unwrap();
        let count = case[1].as_u64().unwrap() as u32;
        let is_dir = case[2].as_bool().unwrap();
        let label = format!("copy {count} dir={is_dir}");
        compare(
            &label,
            value,
            &case[3],
            location::try_new_copy_name(value, count, is_dir),
            &mut failures,
        );
    }
    for case in data["labels"].as_array().unwrap() {
        let value = case[0].as_str().unwrap();
        let fallback = case[1].as_str().unwrap();
        compare(
            "label",
            value,
            &case[2],
            location::safe_label(value, fallback),
            &mut failures,
        );
    }
    for case in data["items"].as_array().unwrap() {
        let value = case[0].as_str().unwrap();
        compare(
            "item",
            value,
            &case[1],
            location::require_item_uri(value),
            &mut failures,
        );
    }
    for case in data["shares"].as_array().unwrap() {
        let value = case[0].as_str().unwrap();
        compare(
            "share",
            value,
            &case[1],
            location::require_share(value),
            &mut failures,
        );
    }
    for case in data["servers"].as_array().unwrap() {
        let value = case[0].as_str().unwrap();
        if location::is_smb_server(value) != case[1].as_bool().unwrap() {
            failures.push(format!("server {value:?}: python {}", case[1]));
        }
    }
    for case in data["devices"].as_array().unwrap() {
        let value = case[0].as_str().unwrap();
        if location::is_device_location(value) != case[1].as_bool().unwrap() {
            failures.push(format!("device {value:?}: python {}", case[1]));
        }
    }
    for case in data["splits"].as_array().unwrap() {
        let value = case[0].as_str().unwrap();
        let actual = location::split_location(value);
        let expected = &case[1];
        let ok = match (expected.get("ok"), &actual) {
            (Some(want), Ok(parts)) => {
                let got = [
                    &parts.scheme,
                    &parts.netloc,
                    &parts.path,
                    &parts.query,
                    &parts.fragment,
                ];
                want.as_array()
                    .unwrap()
                    .iter()
                    .zip(got)
                    .all(|(w, g)| w.as_str() == Some(g.as_str()))
            }
            (None, Err(_)) => true,
            _ => false,
        };
        if !ok {
            failures.push(format!("split {value:?}: python {expected} rust {actual:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
