// SPDX-License-Identifier: AGPL-3.0-only
//! Compatibility cases captured from desktop/ui/app.js; see location_fixtures/README.md.

use std::path::PathBuf;

use ox_core::format::pretty_bytes;
use ox_core::location::{self, DeviceLabel, LocationContext};
use serde_json::{json, Value};

#[test]
fn matches_javascript() {
    let text = include_str!("location_fixtures/javascript.json");
    let data: Value = serde_json::from_str(text).expect("json");
    let context = LocationContext {
        home: Some(PathBuf::from("/home/test")),
        devices: vec![
            DeviceLabel {
                uri: "mtp://[usb:001,010]/".into(),
                label: "Sample Phone".into(),
            },
            DeviceLabel {
                uri: "afc://blank/".into(),
                label: String::new(),
            },
        ],
        snapshot_roots: vec!["file:///srv/snaps".into(), "smb://nas/backup/".into()],
        network_mounts: vec!["/mnt/nas".into(), "/mnt/s3/".into(), "/mnt/plain".into()],
    };
    let mut failures = Vec::new();
    for case in data["uris"].as_array().unwrap() {
        let uri = case["uri"].as_str().unwrap();
        let crumbs: Vec<Value> = context
            .breadcrumbs(uri)
            .into_iter()
            .map(|c| json!({"label": c.label, "uri": c.uri}))
            .collect();
        let parent = location::parent_location(uri);
        let js_parent = case["parentUri"].as_str().map(|p| {
            if location::is_device_location(uri) && !p.ends_with("]/") && p.matches('/').count() > 3 {
                p.trim_end_matches('/').to_string()
            } else {
                p.to_string()
            }
        });
        let got = json!({
            "uri": uri,
            "baseName": context.base_name(uri),
            "parentUri": parent,
            "displayUri": context.display_location(uri),
            "titleFor": context.title_for(uri),
            "writable": context.writable_location(uri),
            "shareRoot": location::is_smb_share_root(uri),
            "readonly": context.is_snapshot_location(uri),
            "crumbs": crumbs,
            "network": context.network_location(uri),
            "deviceRoot": location::device_root(uri),
        });
        let mut want = case.clone();
        want["parentUri"] = json!(js_parent);
        for key in want.as_object().unwrap().keys() {
            if want[key] != got[key] {
                failures.push(format!("{key} {uri:?}: js {} rust {}", want[key], got[key]));
            }
        }
    }
    for case in data["pairs"].as_array().unwrap() {
        let (a, b) = (case[0].as_str().unwrap(), case[1].as_str().unwrap());
        if location::same_location(a, b) != case[2].as_bool().unwrap() {
            failures.push(format!("same {a:?} {b:?}: js {}", case[2]));
        }
    }
    for case in data["sizes"].as_array().unwrap() {
        let bytes = case[0].as_u64().unwrap();
        let want = case[1].as_str().unwrap();
        if pretty_bytes(bytes) != want {
            failures.push(format!("size {bytes}: js {want} rust {}", pretty_bytes(bytes)));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
