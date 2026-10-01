// SPDX-License-Identifier: AGPL-3.0-only
//! Running a program the user chose in the terminal: a script dropped on
//! (DND-026) or a custom command of Open with (OPEN-014), with the
//! terminal kept open after it ends when asked.
//!
//! Safety rule "names are never code": the program and its arguments
//! follow the terminal's command option as they are; the terminal's shell
//! runs one fixed script that calls the program with its arguments as
//! positional parameters.

use std::ffi::OsString;

use super::emulator::Terminal;

/// The shell script a terminal runs a program through to keep its output
/// on screen: it runs the program with its arguments, then waits for
/// Enter. Names reach it only as positional arguments.
pub const HOLD_SCRIPT: &str =
    r#""$@"; status=$?; printf '\n%s' 'Press Enter to close this window.'; read -r _; exit "$status""#;

/// `command` run in `terminal`: through [`HOLD_SCRIPT`], named
/// `hold_name` (its `$0`), when its window should stay open after it ends.
pub fn command_in_terminal(
    terminal: &Terminal,
    hold_name: Option<&str>,
    command: Vec<OsString>,
) -> Vec<OsString> {
    let mut wrapped = vec![
        terminal.executable().as_os_str().to_owned(),
        terminal.kind().command_option().into(),
    ];
    if let Some(name) = hold_name {
        wrapped.extend(["/bin/sh", "-c", HOLD_SCRIPT, name].map(OsString::from));
    }
    wrapped.extend(command);
    wrapped
}
