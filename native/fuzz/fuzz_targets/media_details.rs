// SPDX-License-Identifier: AGPL-3.0-only
//! What the media reader takes from a picture's start (PNG, JPEG and BMP
//! headers, EXIF, XMP), the reader's JSON as the app reads it, the
//! gallery's EXIF dates, Rotate left and right, and a thumbnailer's entry
//! and picture.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    ox_app::fuzzing::media_details(data);
});
