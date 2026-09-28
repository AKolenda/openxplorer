// SPDX-License-Identifier: AGPL-3.0-only
//! Shared by the `location_*` integration tests: the outcome format of
//! `python.json` and a collector that reports every mismatch of a table at
//! once. Include it with
//! `#[path = "location_fixtures/support.rs"] mod support;`.
#![allow(
    dead_code,
    reason = "each test crate that includes this module uses a different part of it"
)]

use std::fmt::Debug;

use ox_core::location::LocationError;
use serde::de::DeserializeOwned;
use serde::Deserialize;

/// Parses a committed fixture. The drift test keeps it well-formed.
pub fn parse_fixture<T: DeserializeOwned>(json: &str) -> T {
    serde_json::from_str(json).expect("the committed fixture matches its Rust types")
}

/// What `desktop/core.py` answered for one input; see `generate_python.py`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome<T> {
    /// `core.py` returned this value.
    Value(T),
    /// A `raise` in `core.py` refused the input with this message. Users
    /// see it, so the Rust port must use the same words.
    Error(String),
    /// Python's standard library refused the input in its own words. The
    /// Rust port must refuse it too, but may word the message differently.
    Rejected(String),
}

impl<T: PartialEq> Outcome<T> {
    /// True when the Rust port's `actual` result gives the same answer.
    pub fn is_matched_by(&self, actual: &Result<T, LocationError>) -> bool {
        match (self, actual) {
            (Outcome::Value(expected), Ok(value)) => expected == value,
            (Outcome::Error(message), Err(error)) => *message == error.to_string(),
            (Outcome::Rejected(_), Err(_)) => true,
            _ => false,
        }
    }
}

/// One input and `core.py`'s answer for it.
#[derive(Debug, Deserialize)]
pub struct Case<T = String> {
    /// The text passed to the function.
    pub input: String,
    /// What `core.py` answered.
    pub outcome: Outcome<T>,
}

/// The inputs a Rust function answers differently from the code it ports.
#[derive(Debug)]
pub struct Mismatches {
    /// The ported function, for example `normalise_location`.
    function: &'static str,
    /// One line per mismatching input.
    found: Vec<String>,
}

impl Mismatches {
    /// An empty collection for `function`.
    pub fn new(function: &'static str) -> Self {
        Self {
            function,
            found: Vec::new(),
        }
    }

    /// Records `input` unless `actual` equals `expected`.
    pub fn expect_equal<T: PartialEq + Debug>(&mut self, input: impl Debug, expected: &T, actual: &T) {
        if expected != actual {
            self.record(input, expected, actual);
        }
    }

    /// Records `input` unless `actual` gives the answer `expected` holds.
    pub fn expect_outcome<T: PartialEq + Debug>(
        &mut self,
        input: impl Debug,
        expected: &Outcome<T>,
        actual: &Result<T, LocationError>,
    ) {
        if !expected.is_matched_by(actual) {
            self.record(input, expected, actual);
        }
    }

    /// Fails the test with every recorded mismatch.
    pub fn assert_none(self) {
        assert!(
            self.found.is_empty(),
            "{} answers {} input(s) differently:\n{}",
            self.function,
            self.found.len(),
            self.found.join("\n")
        );
    }

    fn record(&mut self, input: impl Debug, expected: impl Debug, actual: impl Debug) {
        let line = format!("  {input:?}: expected {expected:?}, Rust gave {actual:?}");
        self.found.push(line);
    }
}
