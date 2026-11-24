// SPDX-License-Identifier: AGPL-3.0-only
//! Extracts an archive held in memory into a new folder, as Extract All
//! does, and checks that nothing lands outside it.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ox_core::fuzzing::archive;

fuzz_target!(|data: &[u8]| {
    archive::extract(data);
});
