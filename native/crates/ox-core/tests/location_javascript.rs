// SPDX-License-Identifier: AGPL-3.0-only
//! Compares the location display helpers and [`pretty_bytes`] with the web
//! UI's helpers in `desktop/ui/app.js`, using the answers
//! `generate_javascript.cjs` captured in `javascript.json`; see
//! `location_fixtures/README.md`. `location_fixture_drift.rs` proves the
//! captured answers are still what `app.js` says.

#[path = "location_fixtures/support.rs"]
mod support;

use std::fmt::Debug;
use std::path::PathBuf;

use ox_core::format::pretty_bytes;
use ox_core::location::{self, Crumb, DeviceLabel, LocationContext};
use serde::Deserialize;
use support::{parse_fixture, Mismatches};

/// The tables of `javascript.json`.
#[derive(Debug, Deserialize)]
struct JavascriptFixture {
    /// The `state.env` the helpers ran with.
    environment: Environment,
    /// Every URI helper's answer for each captured URI.
    uris: Vec<UriCase>,
    /// `sameLocation` answers.
    pairs: Vec<PairCase>,
    /// `prettyBytes` answers.
    sizes: Vec<SizeCase>,
}

/// The session facts the helpers read from `state.env`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Environment {
    /// The home folder as a `file://` URI.
    home: String,
    /// Mounted and unmounted devices.
    mounts: Vec<Mount>,
    /// Folders that hold read-only snapshots.
    snapshot_roots: Vec<String>,
    /// Mount points from the mount table.
    stable_mounts: Vec<StableMount>,
}

/// A volume-monitor mount.
#[derive(Debug, Deserialize)]
struct Mount {
    /// The mount's display name.
    label: String,
    /// Its root URI.
    uri: String,
    /// False for a device that is known but not mounted.
    mounted: bool,
}

/// A mount point from the mount table; `fstype` may be missing.
#[derive(Debug, Deserialize)]
struct StableMount {
    path: String,
    fstype: Option<String>,
}

impl StableMount {
    /// The web UI's test in `networkLocation`: a CIFS or SMB3 mount point
    /// with a path, where a missing type counts as CIFS.
    fn is_network_share(&self) -> bool {
        let fstype = self.fstype.as_deref().unwrap_or_default();
        !self.path.is_empty() && location::is_network_filesystem(fstype)
    }
}

impl Environment {
    /// The [`LocationContext`] the native app builds from the same facts:
    /// only mounted devices, and only CIFS/SMB3 mount points with a path,
    /// as the web UI's `deviceMountName` and `networkLocation` filter them.
    fn to_context(&self) -> LocationContext {
        let home = self.home.strip_prefix("file://").unwrap_or(&self.home);
        let devices = self
            .mounts
            .iter()
            .filter(|mount| mount.mounted)
            .map(|mount| DeviceLabel {
                uri: mount.uri.clone(),
                label: mount.label.clone(),
            })
            .collect();
        let network_mounts = self
            .stable_mounts
            .iter()
            .filter(|mount| mount.is_network_share())
            .map(|mount| PathBuf::from(&mount.path))
            .collect();
        LocationContext {
            home: Some(PathBuf::from(home)),
            devices,
            snapshot_roots: self.snapshot_roots.clone(),
            network_mounts,
        }
    }
}

/// Every helper's answer for one URI, keyed by the `app.js` function name.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one field per yes-or-no helper in app.js; the JSON fixes the shape"
)]
struct UriCase {
    /// The URI every helper was given.
    uri: String,
    /// `baseName`.
    base_name: String,
    /// `parentUri`.
    parent_uri: Option<String>,
    /// `displayUri`.
    display_uri: String,
    /// `titleFor`.
    title_for: String,
    /// `writableLocation`.
    writable: bool,
    /// `isSmbShareRoot`.
    share_root: bool,
    /// `readonlyLocation`: inside a snapshot.
    readonly: bool,
    /// `breadcrumbSegments`.
    crumbs: Vec<CrumbCase>,
    /// `networkLocation`.
    network: bool,
    /// `deviceRoot`.
    device_root: Option<String>,
}

impl UriCase {
    /// The web UI's breadcrumbs as [`Crumb`]s.
    fn breadcrumbs(&self) -> Vec<Crumb> {
        self.crumbs
            .iter()
            .map(|crumb| Crumb::new(&crumb.label, &crumb.uri))
            .collect()
    }
}

/// One segment of `breadcrumbSegments`.
#[derive(Debug, Deserialize)]
struct CrumbCase {
    /// The decoded text on the button.
    label: String,
    /// The location the button opens.
    uri: String,
}

/// `sameLocation(first, second)`.
#[derive(Debug, Deserialize)]
struct PairCase {
    first: String,
    second: String,
    /// The web UI's answer.
    same: bool,
}

/// `prettyBytes(bytes)`.
#[derive(Debug, Deserialize)]
struct SizeCase {
    bytes: u64,
    /// The web UI's text.
    text: String,
}

/// The committed answers of the `app.js` helpers.
fn fixture() -> JavascriptFixture {
    parse_fixture(include_str!("location_fixtures/javascript.json"))
}

/// Checks one helper for every captured URI: `expected` reads the web UI's
/// answer from the case and `actual` computes the Rust answer.
fn assert_every_uri<T: PartialEq + Debug>(
    helper: &'static str,
    expected: impl Fn(&UriCase) -> T,
    actual: impl Fn(&LocationContext, &str) -> T,
) {
    let fixture = fixture();
    let context = fixture.environment.to_context();
    let mut mismatches = Mismatches::new(helper);
    for case in &fixture.uris {
        mismatches.expect_equal(&case.uri, &expected(case), &actual(&context, &case.uri));
    }
    mismatches.assert_none();
}

/// The web UI's `parentUri` in the native app's canonical form. Below a
/// device root the web UI kept a trailing slash (`mtp://[usb:001,010]/DCIM/`);
/// navigation uses canonical URIs, which have one only at a root.
fn canonical_parent(case: &UriCase) -> Option<String> {
    let parent = case.parent_uri.as_deref()?;
    if !location::is_device_location(&case.uri) || is_root_uri(parent) {
        return Some(parent.to_string());
    }
    Some(parent.trim_end_matches('/').to_string())
}

/// True for `scheme://authority/`, whose only path is `/`.
fn is_root_uri(uri: &str) -> bool {
    let Some((_, after_scheme)) = uri.split_once("://") else {
        return false;
    };
    let authority = after_scheme.strip_suffix('/');
    authority.is_some_and(|authority| !authority.contains('/'))
}

#[test]
fn base_names_match_app_js() {
    assert_every_uri(
        "baseName",
        |case| case.base_name.clone(),
        LocationContext::base_name,
    );
}

/// parity: NAV-010
#[test]
fn parent_locations_match_app_js() {
    assert_every_uri("parentUri", canonical_parent, |_, uri| {
        location::parent_location(uri)
    });
}

#[test]
fn display_paths_match_app_js() {
    assert_every_uri(
        "displayUri",
        |case| case.display_uri.clone(),
        LocationContext::display_location,
    );
}

/// parity: TAB-010
#[test]
fn tab_titles_match_app_js() {
    assert_every_uri(
        "titleFor",
        |case| case.title_for.clone(),
        LocationContext::title_for,
    );
}

#[test]
fn writable_locations_match_app_js() {
    assert_every_uri(
        "writableLocation",
        |case| case.writable,
        LocationContext::is_writable_location,
    );
}

/// parity: OPS-035
#[test]
fn smb_share_roots_match_app_js() {
    assert_every_uri(
        "isSmbShareRoot",
        |case| case.share_root,
        |_, uri| location::is_smb_share_root(uri),
    );
}

#[test]
fn snapshot_locations_match_app_js() {
    assert_every_uri(
        "readonlyLocation",
        |case| case.readonly,
        LocationContext::is_snapshot_location,
    );
}

/// parity: NAV-017
#[test]
fn breadcrumbs_match_app_js() {
    assert_every_uri(
        "breadcrumbSegments",
        UriCase::breadcrumbs,
        LocationContext::breadcrumbs,
    );
}

/// parity: NET-006
#[test]
fn network_locations_match_app_js() {
    assert_every_uri(
        "networkLocation",
        |case| case.network,
        LocationContext::is_network_location,
    );
}

#[test]
fn device_roots_match_app_js() {
    assert_every_uri(
        "deviceRoot",
        |case| case.device_root.clone(),
        |_, uri| location::device_root(uri),
    );
}

#[test]
fn same_location_ignores_one_trailing_slash_like_app_js() {
    let mut mismatches = Mismatches::new("sameLocation");
    for case in &fixture().pairs {
        let actual = location::same_location(&case.first, &case.second);
        mismatches.expect_equal((&case.first, &case.second), &case.same, &actual);
    }
    mismatches.assert_none();
}

/// parity: VIEW-003
#[test]
fn sizes_match_app_js() {
    let mut mismatches = Mismatches::new("prettyBytes");
    for case in &fixture().sizes {
        mismatches.expect_equal(case.bytes, &case.text, &pretty_bytes(case.bytes));
    }
    mismatches.assert_none();
}
