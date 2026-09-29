// SPDX-License-Identifier: AGPL-3.0-only
//! The helper's terminal: what it prints, the answers it reads, and the
//! password, which is typed without echo as Python's `getpass` read it.

use std::io::{self, BufRead, IsTerminal, Write};

use rustix::termios::{self, LocalModes, OptionalActions};

/// Where the helper talks to the administrator.
pub(super) trait Terminal {
    /// Whether a person can answer: standard input is a terminal.
    fn is_interactive(&self) -> bool;
    /// Prints `text` and a line break.
    fn say(&mut self, text: &str);
    /// Prints `text` and a line break as an error message.
    fn warn(&mut self, text: &str);
    /// Prints `prompt` and reads one line, without its line break, as
    /// Python's `input`.
    ///
    /// # Errors
    ///
    /// The I/O error of reading.
    fn ask(&mut self, prompt: &str) -> io::Result<String>;
    /// Like [`Terminal::ask`], but what is typed is not shown.
    ///
    /// # Errors
    ///
    /// The I/O error of reading or of turning echo off.
    fn ask_secret(&mut self, prompt: &str) -> io::Result<String>;
}

/// Standard input and output.
pub(super) struct ConsoleTerminal;

impl ConsoleTerminal {
    fn read_line(prompt: &str) -> io::Result<String> {
        let mut output = io::stdout();
        output.write_all(prompt.as_bytes())?;
        output.flush()?;
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
        let answer = line.strip_suffix('\n').unwrap_or(&line);
        Ok(answer.strip_suffix('\r').unwrap_or(answer).to_owned())
    }
}

impl Terminal for ConsoleTerminal {
    fn is_interactive(&self) -> bool {
        io::stdin().is_terminal()
    }

    fn say(&mut self, text: &str) {
        println!("{text}");
    }

    fn warn(&mut self, text: &str) {
        eprintln!("{text}");
    }

    fn ask(&mut self, prompt: &str) -> io::Result<String> {
        Self::read_line(prompt)
    }

    fn ask_secret(&mut self, prompt: &str) -> io::Result<String> {
        let input = io::stdin();
        let shown = termios::tcgetattr(&input)?;
        let mut hidden = shown.clone();
        hidden.local_modes.remove(LocalModes::ECHO);
        termios::tcsetattr(&input, OptionalActions::Flush, &hidden)?;
        let answer = Self::read_line(prompt);
        // Echo comes back whatever was read; the typed line break was not
        // shown, so the next output starts on a new line.
        termios::tcsetattr(&input, OptionalActions::Flush, &shown)?;
        println!();
        answer
    }
}
