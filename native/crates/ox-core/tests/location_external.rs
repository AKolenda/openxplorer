// SPDX-License-Identifier: AGPL-3.0-only
//! Differential tests against `desktop/core.py` with the home folder
//! `/home/demo`: the `external` table of `python.json`, which
//! `generate_python.py` captures by running `normalise_location(value,
//! home=Path('/home/demo'))` and `require_item_uri` on the result. These
//! cases were first captured by hand; `location_fixture_drift.rs` now proves
//! they are still what `core.py` says.

#[path = "location_fixtures/support.rs"]
mod support;

use std::path::PathBuf;

use ox_core::location::{normalise_location, require_item_uri};
use serde::Deserialize;
use support::{parse_fixture, Case, Mismatches};

/// The part of `python.json` this file checks.
#[derive(Debug, Deserialize)]
struct PythonFixture {
    external: ExternalCases,
}

#[derive(Debug, Deserialize)]
struct ExternalCases {
    /// The home folder `core.py` resolved `~` and relative names against.
    home: PathBuf,
    /// `normalise_location(input, home=home)`.
    normalise: Vec<Case>,
    /// `require_item_uri` of the normalised input.
    items: Vec<Case>,
}

fn external_cases() -> ExternalCases {
    let fixture: PythonFixture = parse_fixture(include_str!("location_fixtures/python.json"));
    fixture.external
}

/// Ported from `desktop/core.py::normalise_location` (differential table):
/// the behaviour the entry classifier, the clipboard and file drops rely on.
///
/// parity: NAV-034, NAV-035
#[test]
fn normalise_matches_python() {
    let cases = external_cases();
    let mut mismatches = Mismatches::new("normalise_location");
    for case in &cases.normalise {
        let actual = normalise_location(&case.input, None, &cases.home);
        mismatches.expect_outcome(&case.input, &case.outcome, &actual);
    }
    mismatches.assert_none();
}

/// Ported from `desktop/core.py::require_item_uri` (differential table).
///
/// parity: OPS-035
#[test]
fn require_item_uri_matches_python() {
    let cases = external_cases();
    let mut mismatches = Mismatches::new("require_item_uri");
    for case in &cases.items {
        let normalised = normalise_location(&case.input, None, &cases.home);
        let actual = normalised.and_then(|uri| require_item_uri(&uri));
        mismatches.expect_outcome(&case.input, &case.outcome, &actual);
    }
    mismatches.assert_none();
}
