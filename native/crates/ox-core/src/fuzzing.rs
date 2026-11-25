// SPDX-License-Identifier: AGPL-3.0-only
//! Entry points for the fuzz targets in `native/fuzz`, built only with the
//! `fuzzing` feature, so the app and its packages never contain them.
//!
//! Each function takes arbitrary bytes, runs one parser of untrusted data
//! the way the app does, and panics only when the parser breaks a promise
//! (for example an extraction writing outside its new folder). The
//! regression tests of the parsers call the same functions with the
//! minimized inputs, so a fixed crash stays fixed.

pub mod archive;
pub mod formats;
pub mod text;
