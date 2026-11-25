// SPDX-License-Identifier: AGPL-3.0-only
//! The parsers across a privilege or network boundary: the root helpers'
//! command lines, mount plans, update releases and trusted addresses,
//! account and share lists, search words and saved sessions. The first
//! byte picks the parser.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    ox_core::fuzzing::formats::parse(data);
});
