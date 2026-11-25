// SPDX-License-Identifier: AGPL-3.0-only
//! Lists, inspects and previews an archive held in memory: the ZIP, 7z and
//! TAR (plain, gzip, bzip2, XZ and Zstandard) readers, the extraction plan
//! and the decoders of one previewed member. The first byte picks the name
//! the archive is opened under, since the readers look at the extension.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ox_core::fuzzing::archive;

fuzz_target!(|data: &[u8]| {
    archive::list_inspect_preview(data);
});
