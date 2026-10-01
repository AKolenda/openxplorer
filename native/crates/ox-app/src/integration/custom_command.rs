// SPDX-License-Identifier: AGPL-3.0-only
//! A command line typed into Open with (OPEN-014), as KDE's Open With
//! dialog takes one: the command with `%f`/`%F` for the item's local path
//! and `%u`/`%U` for its address, run directly or in the terminal, which
//! can stay open after the command ends.
//!
//! The command is what the user typed and is split into arguments as a
//! shell would split it (`g_shell_parse_argv`), but no shell runs it, and
//! the item is put in as one argument of its own, whatever its name
//! ("names are never code"). Without a placeholder the item is the last
//! argument.

use std::ffi::OsString;
use std::path::Path;

use gtk::glib;
use ox_core::integration::{command_in_terminal, find_terminal, spawn_program, ExecutableSearch, Sandbox};

use super::applications::{LaunchTarget, PreparedLaunch};

/// The name the hold script runs under (`$0`).
const HOLD_SCRIPT_NAME: &str = "openxplorer-command";

/// Why a command needs a path the item does not have.
const NO_LOCAL_PATH: &str = "This item has no local path for %f. Use %u, or mount its share first.";

/// A command typed into Open with, and how to run it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CustomCommand {
    /// The command line.
    pub(crate) text: String,
    /// Run it in the terminal.
    pub(crate) in_terminal: bool,
    /// Keep the terminal open after it ends.
    pub(crate) keep_open: bool,
}

/// The arguments `text` runs with for `target`.
///
/// # Errors
///
/// The message to show: the line is empty or cannot be split, or it asks
/// for a local path the item does not have.
pub(crate) fn command_arguments(text: &str, target: &LaunchTarget) -> Result<Vec<OsString>, String> {
    let words = glib::shell_parse_argv(text).map_err(|error| error.message().to_owned())?;
    if words.is_empty() {
        return Err("Type a command to run.".to_owned());
    }
    let path = || match target {
        LaunchTarget::Path(path) => Ok(path.clone().into_os_string()),
        LaunchTarget::Uri(_) => Err(NO_LOCAL_PATH.to_owned()),
    };
    let uri = || match target {
        LaunchTarget::Path(path) => OsString::from(glib::filename_to_uri(path, None).unwrap_or_default()),
        LaunchTarget::Uri(uri) => OsString::from(uri),
    };
    let mut has_placeholder = false;
    let mut arguments = Vec::with_capacity(words.len() + 1);
    for word in &words {
        let argument = match word.to_str() {
            Some("%f" | "%F") => {
                has_placeholder = true;
                path()?
            }
            Some("%u" | "%U") => {
                has_placeholder = true;
                uri()
            }
            _ => word.clone(),
        };
        arguments.push(argument);
    }
    if !has_placeholder {
        arguments.push(match target {
            LaunchTarget::Path(path) => path.clone().into_os_string(),
            LaunchTarget::Uri(uri) => OsString::from(uri),
        });
    }
    Ok(arguments)
}

/// Runs `command` for the item `prepared` names, in the item's folder.
///
/// # Errors
///
/// The message to show: as [`command_arguments`], no terminal was found,
/// or the program did not start.
pub(crate) fn run_custom_command(command: &CustomCommand, prepared: &PreparedLaunch) -> Result<(), String> {
    let mut arguments = command_arguments(&command.text, &prepared.target)?;
    let sandbox = Sandbox::detect();
    if command.in_terminal {
        let terminal =
            find_terminal(&ExecutableSearch::for_sandbox(sandbox)).map_err(|error| error.to_string())?;
        let hold = command.keep_open.then_some(HOLD_SCRIPT_NAME);
        arguments = command_in_terminal(&terminal, hold, arguments);
    }
    let folder = match &prepared.target {
        LaunchTarget::Path(path) if prepared.is_folder => path.clone(),
        LaunchTarget::Path(path) => path.parent().unwrap_or(Path::new("/")).to_path_buf(),
        LaunchTarget::Uri(_) => glib::home_dir(),
    };
    spawn_program(&arguments, &folder, sandbox)
        .map_err(|error| format!("The command could not be started: {error}"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn words(arguments: &[OsString]) -> Vec<&str> {
        arguments
            .iter()
            .map(|argument| argument.to_str().unwrap())
            .collect()
    }

    /// parity: OPEN-014
    #[test]
    fn placeholders_take_the_item_as_one_argument_and_it_is_last_without_one() {
        let report = LaunchTarget::Path(PathBuf::from("/home/anna/My report; rm -rf ~.txt"));
        let share = LaunchTarget::Uri("smb://nas/share/a b.txt".to_owned());

        let with = command_arguments("gimp --new-window %f", &report).unwrap();
        let without = command_arguments("'my viewer' -n", &share).unwrap();

        assert_eq!(
            words(&with),
            ["gimp", "--new-window", "/home/anna/My report; rm -rf ~.txt"]
        );
        assert_eq!(words(&without), ["my viewer", "-n", "smb://nas/share/a b.txt"]);
        assert_eq!(
            command_arguments("viewer %f", &share),
            Err(NO_LOCAL_PATH.to_owned())
        );
        assert!(command_arguments("  ", &share).is_err());
    }
}
