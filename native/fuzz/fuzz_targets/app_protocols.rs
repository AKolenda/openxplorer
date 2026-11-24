// SPDX-License-Identifier: AGPL-3.0-only
//! The lines the media player process and the app write each other, a
//! portal's parent window, the file dialog's typed names, the command
//! line and the Settings search box.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    ox_app::fuzzing::app_protocols(data);
});
