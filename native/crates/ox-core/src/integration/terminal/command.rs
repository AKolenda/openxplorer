// SPDX-License-Identifier: AGPL-3.0-only
//! Running a program the user chose in the terminal: a script dropped on
//! (DND-026) or a custom command of Open with (OPEN-014), with the
//! terminal kept open after it ends when asked.
//!
//! Safety rule "names are never code": the program and its arguments
//! follow the terminal's command option as they are (for Tilix, which
//! splits one string itself, and Ghostty, which hands one string to
//! `/bin/sh -c`, each quoted so it splits back exactly); the terminal's
//! shell runs one fixed script that calls the program with its arguments
//! as positional parameters.

use std::ffi::OsString;
use std::path::Path;

use super::emulator::{CommandStyle, Terminal};
use super::launch::directory_arguments;

/// The shell script a terminal runs a program through to keep its output
/// on screen: it runs the program with its arguments, then waits for
/// Enter. Names reach it only as positional arguments.
pub const HOLD_SCRIPT: &str =
    r#""$@"; status=$?; printf '\n%s' 'Press Enter to close this window.'; read -r _; exit "$status""#;

/// `command` run in `terminal`, starting in `folder`: through
/// [`HOLD_SCRIPT`], named `hold_name` (its `$0`), when its window should
/// stay open after it ends.
///
/// The terminal is told `folder` too, because some start a program in
/// their own working folder or the home folder otherwise; run the command
/// in `folder` as well, for the `XTerm` family. A `folder` that is not an
/// absolute path free of control characters is left to the working folder.
pub fn command_in_terminal(
    terminal: &Terminal,
    folder: &Path,
    hold_name: Option<&str>,
    command: Vec<OsString>,
) -> Vec<OsString> {
    let kind = terminal.kind();
    let mut wrapped = vec![terminal.executable().as_os_str().to_owned()];
    wrapped.extend(kind.leading_options().iter().map(OsString::from));
    if is_plain_absolute(folder) {
        wrapped.extend(directory_arguments(kind, folder));
    }
    let mut program = Vec::new();
    if let Some(name) = hold_name {
        program.extend(["/bin/sh", "-c", HOLD_SCRIPT, name].map(OsString::from));
    }
    program.extend(command);
    match kind.command_style() {
        CommandStyle::After(option) => {
            wrapped.push(option.into());
            wrapped.extend(program);
        }
        // kitty and foot have no option that introduces the program: they
        // run their trailing arguments.
        CommandStyle::Trailing => wrapped.extend(program),
        // One argument after the option is taken as it is and split with
        // `g_shell_parse_argv`, which undoes `g_shell_quote` exactly.
        CommandStyle::QuotedAfter(option) => {
            wrapped.push(option.into());
            wrapped.push(quoted_command(&program));
        }
        // `/bin/sh -c` splits the quoted words back exactly, and nothing
        // in them is expanded.
        CommandStyle::ShellCommand { options, option } => {
            wrapped.extend(options.iter().map(OsString::from));
            let mut argument = OsString::from(option);
            argument.push(quoted_command(&program));
            wrapped.push(argument);
        }
    }
    wrapped
}

/// `program`, each argument quoted with `g_shell_quote` (single quotes,
/// as `/bin/sh` reads them) and joined with spaces.
fn quoted_command(program: &[OsString]) -> OsString {
    let mut quoted = OsString::new();
    for (index, argument) in program.iter().enumerate() {
        if index > 0 {
            quoted.push(" ");
        }
        quoted.push(glib::shell_quote(argument));
    }
    quoted
}

/// True for an absolute path without control characters, which every
/// terminal takes as its starting folder as it is.
fn is_plain_absolute(folder: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let bytes = folder.as_os_str().as_bytes();
    bytes.first() == Some(&b'/') && !bytes.iter().any(|byte| *byte < 0x20 || *byte == 0x7f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integration::terminal::{Terminal, TerminalKind};

    /// parity: OPEN-020
    #[test]
    fn a_program_follows_the_terminals_own_command_option() {
        let folder = Path::new("/home/ada/My scripts");
        let command = vec![OsString::from("/usr/bin/htop")];
        let cases: [(TerminalKind, &[&str]); 9] = [
            (
                TerminalKind::GnomeTerminal,
                &["--working-directory=/home/ada/My scripts", "--", "/usr/bin/htop"],
            ),
            // kitty and foot run the program itself, so no option comes
            // between them.
            (
                TerminalKind::Kitty,
                &["--directory=/home/ada/My scripts", "/usr/bin/htop"],
            ),
            (
                TerminalKind::Foot,
                &["--working-directory=/home/ada/My scripts", "/usr/bin/htop"],
            ),
            (
                TerminalKind::Ghostty,
                &[
                    "--working-directory=/home/ada/My scripts",
                    "--gtk-single-instance=false",
                    "--quit-after-last-window-closed=true",
                    "--shell-integration=none",
                    "--initial-command='/usr/bin/htop'",
                ],
            ),
            (
                TerminalKind::Alacritty,
                &["--working-directory=/home/ada/My scripts", "-e", "/usr/bin/htop"],
            ),
            (
                TerminalKind::WezTerm,
                &["start", "--cwd", "/home/ada/My scripts", "--", "/usr/bin/htop"],
            ),
            (
                TerminalKind::Ptyxis,
                &[
                    "--new-window",
                    "--working-directory=/home/ada/My scripts",
                    "--",
                    "/usr/bin/htop",
                ],
            ),
            (
                TerminalKind::Terminator,
                &[
                    "--no-dbus",
                    "--working-directory=/home/ada/My scripts",
                    "-x",
                    "/usr/bin/htop",
                ],
            ),
            (TerminalKind::XTerm, &["-e", "/usr/bin/htop"]),
        ];

        for (kind, arguments) in cases {
            let executable = Path::new("/usr/bin").join(kind.program_name());
            let terminal = Terminal::new(executable.clone(), kind).expect("absolute");

            let wrapped = command_in_terminal(&terminal, folder, None, command.clone());

            let mut expected = vec![executable.into_os_string()];
            expected.extend(arguments.iter().map(OsString::from));
            assert_eq!(wrapped, expected, "{kind:?}");
        }
    }

    /// Tilix joins the arguments after `-e` with spaces, quoting only those
    /// with a space, and splits them again; one argument, each part quoted,
    /// comes back exactly.
    ///
    /// parity: OPEN-020, SAFE-015
    #[test]
    fn tilix_gets_the_program_as_one_quoted_string_that_splits_back_exactly() {
        let tilix = Terminal::new("/usr/bin/tilix".into(), TerminalKind::Tilix).expect("absolute");
        let program = vec![
            OsString::from("/home/ada/run it.sh"),
            OsString::from("it's \"quoted\" $(touch PWNED) `x` \\ ;&|"),
            OsString::new(),
        ];

        let wrapped = command_in_terminal(&tilix, Path::new("/home/ada"), Some("hold"), program.clone());

        assert_eq!(
            wrapped[..3],
            ["/usr/bin/tilix", "--working-directory=/home/ada", "-e"].map(OsString::from)
        );
        assert_eq!(wrapped.len(), 4, "{wrapped:?}");
        let split = glib::shell_parse_argv(&wrapped[3]).expect("parses");
        let mut expected = ["/bin/sh", "-c", HOLD_SCRIPT, "hold"]
            .map(OsString::from)
            .to_vec();
        expected.extend(program);
        assert_eq!(split, expected);
    }

    /// Ghostty runs its command with `/bin/sh -c`: before 1.2 every `-e`
    /// command, joined unquoted, and in every version an
    /// `--initial-command=`. Each argument is quoted, so the shell gives
    /// the program exactly the names it was given and runs none of them.
    ///
    /// parity: OPEN-020, SAFE-015
    #[test]
    fn ghostty_gets_the_program_as_one_shell_command_that_runs_it_exactly() {
        let ghostty = Terminal::new("/usr/bin/ghostty".into(), TerminalKind::Ghostty).expect("absolute");
        let folder = tempfile::tempdir().expect("a temporary folder");
        let witness = folder.path().join("PWNED");
        let arguments = vec![
            OsString::from("/home/ada/run it.sh"),
            OsString::from(format!(
                "it's \"quoted\" $(touch {0}) `touch {0}` \\ ;&| *",
                witness.display()
            )),
            OsString::from("two\nlines"),
            OsString::new(),
        ];
        let mut program = vec![OsString::from("printf"), OsString::from("%s\\0")];
        program.extend(arguments.clone());

        let wrapped = command_in_terminal(&ghostty, folder.path(), None, program);

        assert!(!wrapped.iter().any(|argument| argument == "-e"), "{wrapped:?}");
        let command = wrapped
            .last()
            .and_then(|argument| argument.to_str())
            .and_then(|argument| argument.strip_prefix("--initial-command="))
            .expect("the command is the last argument");
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", command])
            .current_dir(folder.path())
            .output()
            .expect("sh runs");
        assert!(output.status.success(), "{output:?}");
        let printed: Vec<OsString> = output
            .stdout
            .split(|byte| *byte == 0)
            .map(|part| OsString::from(String::from_utf8_lossy(part).into_owned()))
            .collect();
        let mut expected = arguments;
        expected.push(OsString::new());
        assert_eq!(printed, expected, "each argument arrives as it is");
        assert!(!witness.exists(), "nothing in a name runs");
    }

    #[test]
    fn a_folder_with_a_control_character_is_left_to_the_working_folder() {
        let gnome =
            Terminal::new("/usr/bin/gnome-terminal".into(), TerminalKind::GnomeTerminal).expect("absolute");

        let wrapped = command_in_terminal(
            &gnome,
            Path::new("/home/ada/a\nb"),
            None,
            vec!["/usr/bin/htop".into()],
        );

        assert_eq!(
            wrapped,
            ["/usr/bin/gnome-terminal", "--", "/usr/bin/htop"].map(OsString::from)
        );
    }
}
