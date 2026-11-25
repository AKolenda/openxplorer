// SPDX-License-Identifier: AGPL-3.0-only
//! The app's own picture header readers (PNG, JPEG, GIF, BMP, WebP), the
//! PNG end and GIF first-frame cutters fed in pieces, and the answers
//! of the picture decoder and vector drawer processes.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    ox_app::fuzzing::picture_headers(data);
});
