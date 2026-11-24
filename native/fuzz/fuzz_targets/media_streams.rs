// SPDX-License-Identifier: AGPL-3.0-only
//! A video or audio file read as the media reader process reads it:
//! GStreamer's discoverer and parsers (ID3, MP4, Matroska, Ogg, FLAC,
//! WAV and the rest) and the app's tag and rate handling.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    ox_app::fuzzing::media_streams(data);
});
