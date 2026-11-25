// SPDX-License-Identifier: AGPL-3.0-only
//! The small parsers of ox-core: locations, clipboard file lists, file
//! dialog requests, mount tables, user-dirs, catalogs, snapshot lists,
//! bookmarks, service menus and names. The first byte picks the parser.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ox_core::fuzzing::text;

fuzz_target!(|data: &[u8]| {
    text::parse(data);
});
