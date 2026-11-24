// SPDX-License-Identifier: AGPL-3.0-only
//! TAR and 7z archives whose checksums are made right before they are
//! read, and TARs compressed here with gzip, bzip2, XZ or Zstandard, so
//! mutations reach the parsers behind the checksums and decompressors.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ox_core::fuzzing::archive;

fuzz_target!(|data: &[u8]| {
    archive::fixed_up(data);
});
