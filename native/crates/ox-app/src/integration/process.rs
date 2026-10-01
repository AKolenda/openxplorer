// SPDX-License-Identifier: AGPL-3.0-only
//! Starting a program the user chose: a program dropped on (DND-026) or a
//! custom command of Open with (OPEN-014), directly or in the terminal,
//! with the terminal kept open after it ends when asked.
//!
//! Safety rule "names are never code": a command is a list of arguments,
//! never a line a shell parses; the terminal's shell runs one fixed script
//! that calls the program with its arguments as they are.

use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

use ox_core::integration::{Sandbox, Terminal, TerminalKind};

/// The shell script a terminal runs a program through to keep its output
/// on screen: it runs the program with its arguments, then waits for
/// Enter. Names reach it only as positional arguments.
pub(crate) const HOLD_SCRIPT: &str =
    r#""$@"; status=$?; printf '\n%s' 'Press Enter to close this window.'; read -r _; exit "$status""#;

/// Runs a program on the host from inside Flatpak.
const FLATPAK_SPAWN: &str = "flatpak-spawn";

/// Variables that would make GNOME Terminal open a tab in the terminal the
/// app was started from, instead of a window of its own.
const INHERITED_TERMINAL_VARIABLES: [&str; 2] = ["GNOME_TERMINAL_SCREEN", "GNOME_TERMINAL_SERVICE"];

/// The option after which `kind` runs the rest of its arguments as a
/// command.
pub(crate) fn command_option(kind: TerminalKind) -> &'static str {
    match kind {
        TerminalKind::GnomeTerminal | TerminalKind::Console => "--",
        TerminalKind::XfceTerminal => "-x",
        TerminalKind::Konsole | TerminalKind::XTerm | TerminalKind::UXTerm => "-e",
    }
}

/// `command` run in `terminal`: through the hold script, named
/// `hold_name`, when its window should stay open after it ends.
pub(crate) fn in_terminal(
    terminal: &Terminal,
    hold_name: Option<&str>,
    command: Vec<OsString>,
) -> Vec<OsString> {
    let mut wrapped = vec![
        terminal.executable().as_os_str().to_owned(),
        command_option(terminal.kind()).into(),
    ];
    if let Some(name) = hold_name {
        wrapped.extend(["/bin/sh", "-c", HOLD_SCRIPT, name].map(OsString::from));
    }
    wrapped.extend(command);
    wrapped
}

/// Runs `command` in `folder` without waiting for it: on the host through
/// `flatpak-spawn --host` when the app is a Flatpak, in a process group of
/// its own, and without GNOME Terminal's variables, so a terminal opens a
/// window of its own.
pub(crate) fn spawn_command(command: &[OsString], folder: &Path, sandbox: Sandbox) -> std::io::Result<()> {
    let Some((program, arguments)) = command.split_first() else {
        return Ok(());
    };
    let mut process = if sandbox.is_flatpak() {
        let mut directory = OsString::from("--directory=");
        directory.push(folder);
        let mut host = Command::new(FLATPAK_SPAWN);
        host.arg("--host").arg(directory).arg(program);
        host
    } else {
        let mut local = Command::new(program);
        local.current_dir(folder);
        local
    };
    process.args(arguments).stdin(Stdio::null()).process_group(0);
    for variable in INHERITED_TERMINAL_VARIABLES {
        process.env_remove(variable);
    }
    let mut child = process.spawn()?;
    // Reaped when it exits, so it never lingers as a zombie.
    std::thread::spawn(move || child.wait());
    Ok(())
}
