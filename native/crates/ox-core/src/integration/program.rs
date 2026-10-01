// SPDX-License-Identifier: AGPL-3.0-only
//! Starting a program the user chose without waiting for it: a program
//! dropped on (DND-026) or a custom command of Open with (OPEN-014).
//!
//! The command is a list of arguments, never a line a shell parses. It
//! runs on the host through `flatpak-spawn --host` inside Flatpak, in a
//! process group of its own and without GNOME Terminal's inherited
//! variables, so a terminal it starts opens a window of its own.

use std::ffi::OsString;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Stdio;
use std::thread;

use super::host_command::HostCommand;
use super::sandbox::Sandbox;
use super::terminal::INHERITED_TERMINAL_VARIABLES;

/// Runs `command`, the program and its arguments, in `folder` without
/// waiting for it, and reaps it on a background thread once it exits. An
/// empty command does nothing.
///
/// # Errors
///
/// The error of starting the program.
pub fn spawn_program(command: &[OsString], folder: &Path, sandbox: Sandbox) -> io::Result<()> {
    let Some((program, arguments)) = command.split_first() else {
        return Ok(());
    };
    let mut host_command = arguments
        .iter()
        .fold(HostCommand::new(program), HostCommand::arg)
        .current_dir(folder);
    for variable in INHERITED_TERMINAL_VARIABLES {
        host_command = host_command.env_remove(variable);
    }
    let mut child = host_command
        .to_command(sandbox)
        .stdin(Stdio::null())
        .process_group(0)
        .spawn()?;
    // If no thread can be started, the program still runs; it is reaped
    // when OpenXplorer exits.
    let _ = thread::Builder::new()
        .name("openxplorer-program".to_owned())
        .spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn a_program_starts_in_the_folder_with_its_arguments_as_they_are() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let odd_name = "it's $(touch pwned); name";
        let command = ["/bin/sh", "-c", r#"printf '%s' "$1" > out"#, "sh", odd_name].map(OsString::from);

        spawn_program(&command, folder.path(), Sandbox::Host).expect("the program starts");

        let out = folder.path().join("out");
        let deadline = Instant::now() + Duration::from_secs(10);
        while fs::read_to_string(&out).map_or(true, |text| text != odd_name) {
            assert!(
                Instant::now() < deadline,
                "the program wrote its argument in the folder"
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!folder.path().join("pwned").exists());
    }
}
