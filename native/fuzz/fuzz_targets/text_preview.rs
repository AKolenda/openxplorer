// SPDX-License-Identifier: AGPL-3.0-only
//! The Preview pane's text (encoding detection, Markdown and code shown
//! as text) and rich text readers, and desktop entries followed as links.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    ox_app::fuzzing::text_preview(data);
});
