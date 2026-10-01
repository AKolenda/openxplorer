// SPDX-License-Identifier: AGPL-3.0-only
//! Compares the location functions with `v2.0.0:desktop/core.py`, using the
//! answers `generate_python.py` captured in `python.json`; see
//! `location_fixtures/README.md`. `location_fixture_drift.rs` proves the
//! captured answers are still what `core.py` says.

#[path = "location_fixtures/support.rs"]
mod support;

use std::fmt::Debug;
use std::path::PathBuf;

use ox_core::location::{self, ItemKind, LocationError, LocationKind, LocationParts};
use serde::Deserialize;
use support::{parse_fixture, Case, Mismatches, Outcome};

/// The tables of `python.json` this file checks.
#[derive(Debug, Deserialize)]
struct PythonFixture {
    /// The home folder `core.py` resolved `~` and relative names against.
    home: PathBuf,
    /// `normalise_location(input, home=home)`.
    normalise: Vec<Case>,
    /// `normalise_location(input, base, home)`.
    relative: Vec<RelativeCase>,
    /// `validate_name(input)`.
    names: Vec<Case>,
    /// `new_copy_name(name, number, is_dir)`.
    copies: Vec<CopyNameCase>,
    /// `safe_label(input, fallback)`.
    labels: Vec<LabelCase>,
    /// `require_item_uri(input)`.
    items: Vec<Case>,
    /// `require_share(input)`.
    shares: Vec<Case>,
    /// `is_smb_server(input)`.
    servers: Vec<ServerCase>,
    /// `is_device_location(input)`.
    devices: Vec<DeviceCase>,
    /// `split_location(input)`.
    splits: Vec<Case<SplitParts>>,
}

/// A name typed in a folder: `normalise_location(input, base)`.
#[derive(Debug, Deserialize)]
struct RelativeCase {
    input: String,
    base: String,
    outcome: Outcome<String>,
}

/// `new_copy_name(name, number, is_directory)`: the "Keep both" name.
#[derive(Debug, Deserialize)]
struct CopyNameCase {
    name: String,
    number: u32,
    is_dir: bool,
    outcome: Outcome<String>,
}

impl CopyNameCase {
    /// The fixture's `is_dir` flag as the Rust port's [`ItemKind`].
    fn kind(&self) -> ItemKind {
        if self.is_dir {
            ItemKind::Folder
        } else {
            ItemKind::File
        }
    }
}

/// `safe_label(input, fallback)`: a sidebar label.
#[derive(Debug, Deserialize)]
struct LabelCase {
    input: String,
    fallback: String,
    outcome: Outcome<String>,
}

/// `is_smb_server(input)`.
#[derive(Debug, Deserialize)]
struct ServerCase {
    input: String,
    is_server: bool,
}

/// `is_device_location(input)`.
#[derive(Debug, Deserialize)]
struct DeviceCase {
    input: String,
    is_device: bool,
}

/// Python's `SplitResult` fields, compared with [`LocationParts`].
#[derive(Debug, PartialEq, Deserialize)]
struct SplitParts {
    scheme: String,
    /// Python's name for [`LocationParts::authority`].
    netloc: String,
    path: String,
    query: String,
    fragment: String,
}

impl From<LocationParts> for SplitParts {
    fn from(parts: LocationParts) -> Self {
        Self {
            scheme: parts.scheme,
            netloc: parts.authority,
            path: parts.path,
            query: parts.query,
            fragment: parts.fragment,
        }
    }
}

/// The committed answers of `core.py`.
fn fixture() -> PythonFixture {
    parse_fixture(include_str!("location_fixtures/python.json"))
}

/// True for SFTP, FTP, WebDAV and NFS addresses, which `core.py` refuses
/// and the native app browses (NET-029): a deliberate gain, checked by the
/// unit tests of `location::normalise` instead.
fn is_native_gain(input: &str) -> bool {
    location::location_kind(input) == LocationKind::Remote
}

/// Checks the Rust `ported` function against every case of a one-argument
/// table of `function`, except the addresses of [`is_native_gain`].
fn assert_cases_match<T: PartialEq + Debug>(
    function: &'static str,
    cases: &[Case<T>],
    ported: impl Fn(&str) -> Result<T, LocationError>,
) {
    let mut mismatches = Mismatches::new(function);
    for case in cases.iter().filter(|case| !is_native_gain(&case.input)) {
        mismatches.expect_outcome(&case.input, &case.outcome, &ported(&case.input));
    }
    mismatches.assert_none();
}

/// parity: NAV-034, NAV-035, DEV-005, SAFE-010
#[test]
fn typed_addresses_are_normalised_like_core_py() {
    let fixture = fixture();
    assert_cases_match("normalise_location", &fixture.normalise, |input| {
        location::normalise_location(input, None, &fixture.home)
    });
}

/// parity: NAV-034
#[test]
fn relative_names_resolve_against_their_folder_like_core_py() {
    let fixture = fixture();
    let mut mismatches = Mismatches::new("normalise_location with a base");
    for case in fixture
        .relative
        .iter()
        .filter(|case| !is_native_gain(&case.input))
    {
        let actual = location::normalise_location(&case.input, Some(&case.base), &fixture.home);
        mismatches.expect_outcome((&case.input, &case.base), &case.outcome, &actual);
    }
    mismatches.assert_none();
}

/// parity: OPS-006
#[test]
fn file_names_are_validated_like_core_py() {
    assert_cases_match("validate_name", &fixture().names, |input| {
        location::validate_name(input).map(str::to_string)
    });
}

/// parity: XFER-008
#[test]
fn copy_names_are_chosen_like_core_py() {
    let mut mismatches = Mismatches::new("new_copy_name");
    for case in &fixture().copies {
        let actual = location::new_copy_name(&case.name, case.number, case.kind());
        let input = (&case.name, case.number, case.is_dir);
        mismatches.expect_outcome(input, &case.outcome, &actual);
    }
    mismatches.assert_none();
}

/// parity: SAFE-018
#[test]
fn sidebar_labels_are_validated_like_core_py() {
    let mut mismatches = Mismatches::new("safe_label");
    for case in &fixture().labels {
        let actual = location::safe_label(&case.input, &case.fallback);
        mismatches.expect_outcome((&case.input, &case.fallback), &case.outcome, &actual);
    }
    mismatches.assert_none();
}

/// parity: OPS-035, NET-003
#[test]
fn share_and_device_roots_are_not_operation_items_like_core_py() {
    assert_cases_match("require_item_uri", &fixture().items, location::require_item_uri);
}

/// A saved network location must be a shared folder, not a server.
///
/// parity: NET-017
#[test]
fn shared_folders_are_required_like_core_py() {
    assert_cases_match("require_share", &fixture().shares, location::require_share);
}

#[test]
fn smb_server_listings_are_recognised_like_core_py() {
    let mut mismatches = Mismatches::new("is_smb_server");
    for case in &fixture().servers {
        let actual = location::is_smb_server(&case.input);
        mismatches.expect_equal(&case.input, &case.is_server, &actual);
    }
    mismatches.assert_none();
}

#[test]
fn device_locations_are_recognised_like_core_py() {
    let mut mismatches = Mismatches::new("is_device_location");
    for case in &fixture().devices {
        let actual = location::is_device_location(&case.input);
        mismatches.expect_equal(&case.input, &case.is_device, &actual);
    }
    mismatches.assert_none();
}

#[test]
fn locations_are_split_like_core_py() {
    assert_cases_match("split_location", &fixture().splits, |input| {
        location::split_location(input).map(SplitParts::from)
    });
}
