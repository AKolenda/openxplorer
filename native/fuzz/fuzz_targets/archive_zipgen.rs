// SPDX-License-Identifier: AGPL-3.0-only
//! ZIPs written from the input with every offset and checksum right but
//! what the input asks to be wrong (names, links, special files, Unicode
//! path and ZIP64 fields, data descriptors, encryption, methods), then
//! listed, previewed and extracted with the no-escape checks.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ox_core::fuzzing::archive;

fuzz_target!(|data: &[u8]| {
    archive::zip_generated(data);
});
