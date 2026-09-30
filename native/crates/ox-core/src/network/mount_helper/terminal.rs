// SPDX-License-Identifier: AGPL-3.0-only
//! The helper's terminal: what it prints, the answers it reads, and the
//! password, which is typed without echo as Python's `getpass` read it.
//!
//! While the password is typed the terminal delivers keys one at a time
//! and Ctrl+C is read as a key, not a signal, so cancelling there always
//! gives the terminal its echo back, as `getpass` did in a `finally`.

use std::io::{self, BufRead, IsTerminal, Read, Write};

use rustix::fd::AsFd;
use rustix::termios::{self, LocalModes, OptionalActions, SpecialCodeIndex, Termios};

/// Ctrl+C, which cancels the password.
const CANCEL: u8 = 0x03;
/// Ctrl+D, which ends the input.
const END_OF_INPUT: u8 = 0x04;
/// Ctrl+U, which erases the whole password typed so far.
const ERASE_LINE: u8 = 0x15;
/// Backspace and Delete, which erase the last character.
const ERASE: [u8; 2] = [0x08, 0x7f];

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
        let mut output = io::stdout();
        output.write_all(prompt.as_bytes())?;
        output.flush()?;
        let input = io::stdin();
        let _hidden = HiddenInput::start(&input)?;
        read_secret(&mut input.lock())
    }
}

/// The terminal with echo, line editing and signal keys off; dropping it
/// restores the terminal, whatever ended the read.
struct HiddenInput<Fd: AsFd> {
    input: Fd,
    shown: Termios,
}

impl<Fd: AsFd> HiddenInput<Fd> {
    fn start(input: Fd) -> io::Result<Self> {
        let shown = termios::tcgetattr(&input)?;
        let mut hidden = shown.clone();
        hidden
            .local_modes
            .remove(LocalModes::ECHO | LocalModes::ICANON | LocalModes::ISIG);
        hidden.special_codes[SpecialCodeIndex::VMIN] = 1;
        hidden.special_codes[SpecialCodeIndex::VTIME] = 0;
        termios::tcsetattr(&input, OptionalActions::Flush, &hidden)?;
        Ok(Self { input, shown })
    }
}

impl<Fd: AsFd> Drop for HiddenInput<Fd> {
    fn drop(&mut self) {
        // Nothing more can be done if restoring fails.
        let _ = termios::tcsetattr(&self.input, OptionalActions::Flush, &self.shown);
        // The typed line break was not shown, so the next output starts on
        // a new line.
        println!();
    }
}

/// Reads a password typed key by key until Enter: Backspace erases a
/// character, Ctrl+U the whole password, and Ctrl+C cancels.
///
/// # Errors
///
/// [`io::ErrorKind::Interrupted`] for Ctrl+C,
/// [`io::ErrorKind::UnexpectedEof`] when the input ends before anything
/// was typed, and the I/O error of reading.
fn read_secret(input: &mut impl Read) -> io::Result<String> {
    let mut typed: Vec<u8> = Vec::new();
    let mut key = [0_u8];
    loop {
        if input.read(&mut key)? == 0 || key[0] == END_OF_INPUT {
            if typed.is_empty() {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            break;
        }
        match key[0] {
            CANCEL => return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled")),
            b'\n' | b'\r' => break,
            ERASE_LINE => typed.clear(),
            byte if ERASE.contains(&byte) => {
                // Drops a whole UTF-8 character: its continuation bytes,
                // then its first byte.
                while typed.pop().is_some_and(|byte| byte & 0xC0 == 0x80) {}
            }
            byte => typed.push(byte),
        }
    }
    Ok(String::from_utf8_lossy(&typed).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Backspace erases a whole character, Ctrl+U the password, Ctrl+C
    /// cancels and Enter ends it.
    ///
    /// parity: NET-028
    #[test]
    fn a_hidden_password_is_edited_and_cancelled_like_getpass() {
        let read = |keys: &[u8]| read_secret(&mut &keys[..]);

        assert_eq!(read("sé\x7fe\x7fecret\n".as_bytes()).expect("typed"), "secret");
        assert_eq!(read(b"old\x15new\r").expect("typed"), "new");
        let cancelled = read(b"sec\x03ret\n").expect_err("cancelled");
        assert_eq!(cancelled.kind(), io::ErrorKind::Interrupted);
        assert_eq!(
            read(b"").expect_err("no input").kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}
