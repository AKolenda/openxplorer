// SPDX-License-Identifier: AGPL-3.0-only
//! Everything the `openxplorer-admin-delete` program does.
//!
//! The program takes one absolute path and nothing else. It is started by
//! `pkexec` and runs as root, so it reads no settings, opens no network
//! connection, and trusts neither its working directory nor its
//! environment: every path it touches is resolved from `/` through
//! descriptors it opened itself.
//!
//! It writes one record per item on its standard output in the format
//! [`super::report`] documents, and it stops when it reads a line `cancel`
//! on its standard input. Its exit status says how the walk ended:
//!
//! | Status | Meaning |
//! |---|---|
//! | 0 | Everything the item held was deleted |
//! | 1 | The walk finished with items left in place |
//! | 2 | The request was refused; nothing was deleted |
//! | 3 | The app cancelled the walk |
//! | 4 | The walk stopped part-way, because the tree changed under it |

use std::ffi::OsString;
use std::io::{BufRead, Write};
use std::path::PathBuf;

use super::report::{Line, Record, Totals};
use super::walk;
use crate::i18n::gettext;
use crate::transfer::{Cancellation, TransferError};

/// Everything was deleted.
pub(super) const COMPLETE: u8 = 0;
/// The walk finished, but something is still there.
pub(super) const INCOMPLETE: u8 = 1;
/// The request was refused before anything was deleted.
pub(super) const REFUSED: u8 = 2;
/// The app cancelled the walk.
pub(super) const CANCELLED: u8 = 3;
/// The walk stopped part-way.
pub(super) const STOPPED: u8 = 4;

/// What the program was asked to do.
enum Request {
    /// Delete this item and everything inside it.
    Delete(PathBuf),
    /// Print the usage text and stop.
    Usage,
    /// The arguments make no sense; the message says why.
    Refuse(String),
}

/// Runs the helper program and returns its exit status.
pub fn admin_delete_command(arguments: impl IntoIterator<Item = OsString>) -> u8 {
    let mut output = std::io::stdout().lock();
    match read_request(arguments) {
        Request::Usage => {
            let _ = write!(output, "{USAGE}");
            COMPLETE
        }
        Request::Refuse(message) => {
            report(&mut output, &Line::Refused(message));
            REFUSED
        }
        Request::Delete(path) => delete(&mut output, &path),
    }
}

/// The usage text, which `verify_layout.py` runs the installed helper to
/// check that it starts at all.
const USAGE: &str = "\
Usage: openxplorer-admin-delete [--] <absolute path>

Permanently deletes one item, and everything inside it, with the rights it
is started with. OpenXplorer starts it through pkexec after the user
confirms a permanent delete under admin://; it is not meant to be run by
hand. It follows a symbolic link on the way to the item only when nobody
but root could have put it there, never follows one inside the item,
never leaves the drive the item is on, and refuses a filesystem root, a
mount point, a relative path and a path through '..'.

It writes one tab-separated record per item on standard output, and stops
when it reads a line 'cancel' on standard input.
";

/// Reads the command line.
fn read_request(arguments: impl IntoIterator<Item = OsString>) -> Request {
    let mut paths: Vec<OsString> = Vec::new();
    let mut only_paths = false;
    for argument in arguments {
        if !only_paths && argument == "--" {
            only_paths = true;
        } else if !only_paths && (argument == "--help" || argument == "-h") {
            return Request::Usage;
        } else if !only_paths && argument.as_encoded_bytes().starts_with(b"-") {
            return Request::Refuse(gettext("This helper takes one path and no options."));
        } else {
            paths.push(argument);
        }
    }
    match paths.len() {
        1 => Request::Delete(PathBuf::from(paths.remove(0))),
        _ => Request::Refuse(gettext("This helper deletes exactly one item at a time.")),
    }
}

/// Deletes `path`, writing a record for every item, and returns the status.
fn delete(output: &mut dyn Write, path: &std::path::Path) -> u8 {
    let cancel = Cancellation::new();
    let watcher = watch_standard_input(&cancel);
    let options = walk::Options::default();
    // What the walk reported before it ended, which an error needs: a
    // refusal comes before the first record, while a stop part-way comes
    // after some of the tree is already gone.
    let mut reported = Totals::default();
    let mut report_record = |record: Record| {
        reported.count(&record);
        report(output, &Line::Record(record));
    };
    let result = walk::delete_tree(path, &cancel, &options, &mut report_record);
    watcher.stop();
    let status = match result {
        Ok(totals) if totals.is_complete() => COMPLETE,
        Ok(_) => INCOMPLETE,
        Err(TransferError::Cancelled) => {
            report(output, &Line::Cancelled);
            CANCELLED
        }
        Err(error) if reported == Totals::default() => {
            report(output, &Line::Refused(error.to_string()));
            return REFUSED;
        }
        Err(error) => {
            report(
                output,
                &Line::Record(Record::Failed {
                    path: path.to_path_buf(),
                    message: error.to_string(),
                }),
            );
            reported.failed += 1;
            STOPPED
        }
    };
    report(output, &Line::Done(reported));
    status
}

/// Writes one record and flushes it, so the app sees progress as it
/// happens rather than when the helper exits.
fn report(output: &mut dyn Write, line: &Line) {
    let _ = writeln!(output, "{}", line.encode());
    let _ = output.flush();
}

/// Cancels the walk when the app asks for it.
struct InputWatcher {
    thread: Option<std::thread::JoinHandle<()>>,
}

impl InputWatcher {
    /// Lets the watching thread go. Its read of standard input ends when
    /// the app closes the pipe, which happens as the helper exits.
    fn stop(mut self) {
        self.thread.take();
    }
}

/// Starts the thread that cancels `cancel` when standard input carries a
/// line `cancel`.
fn watch_standard_input(cancel: &Cancellation) -> InputWatcher {
    let cancel = cancel.clone();
    let thread = std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if asks_to_cancel(&line) {
                cancel.cancel();
                return;
            }
        }
    });
    InputWatcher { thread: Some(thread) }
}

/// Whether a line the app sent asks the walk to stop.
fn asks_to_cancel(line: &str) -> bool {
    line.trim() == "cancel"
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// parity: XFER-015
    #[test]
    fn only_a_cancel_line_stops_the_walk() {
        assert!(asks_to_cancel("cancel"));
        assert!(asks_to_cancel("cancel\r"));
        assert!(!asks_to_cancel("cancelled"));
        assert!(!asks_to_cancel(""));
    }

    /// The helper is started by pkexec with one path, so anything else is
    /// a mistake worth refusing rather than guessing at.
    ///
    /// parity: XFER-015
    #[test]
    fn the_helper_takes_one_path_and_no_options() {
        let request =
            |arguments: &[&str]| read_request(arguments.iter().map(|argument| OsString::from(*argument)));

        assert!(matches!(request(&["--", "/etc/x"]), Request::Delete(path) if path == Path::new("/etc/x")));
        assert!(matches!(request(&["/etc/x"]), Request::Delete(_)));
        // A path of its own that begins with a dash is still a path.
        assert!(matches!(request(&["--", "-odd"]), Request::Delete(_)));
        assert!(matches!(request(&["--help"]), Request::Usage));
        assert!(matches!(request(&["--force"]), Request::Refuse(_)));
        assert!(matches!(request(&["/etc/x", "/etc/y"]), Request::Refuse(_)));
        assert!(matches!(request(&[]), Request::Refuse(_)));
    }
}
