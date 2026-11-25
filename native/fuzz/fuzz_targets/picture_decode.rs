// SPDX-License-Identifier: AGPL-3.0-only
//! A picture decoded as the decoder process decodes it (gdk-pixbuf's
//! loaders behind the app's guards), sent in pieces, and the answer
//! read back as the app reads it.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    ox_app::fuzzing::picture_decode(data);
});
