// SPDX-License-Identifier: AGPL-3.0-only
//! Everything the `openxplorer-admin-delete` program does.
//!
//! The program takes one absolute path, and the device and inode the app
//! saw for that item before the administrator prompt (`--item
//! <device>:<inode>`), which the item has to still be. It is started by
//! `pkexec` and runs as root, so it reads no settings, opens no network
//! connection, and trusts neither its working directory nor its
//! environment: every path it deletes is resolved from `/` through
//! descriptors it opened itself. The one thing it takes from the
//! environment is the desktop's language, and only a plain language name
//! is used to pick its package's catalogue in `/usr/share/locale`.
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
use crate::transfer::{Cancellation, ItemIdentity, TransferError};

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

/// The option that names the item's device and inode.
pub(super) const ITEM_OPTION: &str = "--item";

/// `identity` as the value of [`ITEM_OPTION`].
pub(super) fn item_argument(identity: ItemIdentity) -> String {
    format!("{}:{}", identity.device, identity.inode)
}

/// The identity an [`ITEM_OPTION`] value names.
fn parse_item(value: &OsString) -> Option<ItemIdentity> {
    let (device, inode) = value.to_str()?.split_once(':')?;
    Some(ItemIdentity {
        device: device.parse().ok()?,
        inode: inode.parse().ok()?,
    })
}

/// What the program was asked to do.
enum Request {
    /// Delete this item, which must still be the one the identity names,
    /// and everything inside it.
    Delete(PathBuf, ItemIdentity),
    /// Print the usage text and stop.
    Usage,
    /// The arguments make no sense; the message says why.
    Refuse(String),
}

/// Runs the helper program and returns its exit status.
pub fn admin_delete_command(arguments: impl IntoIterator<Item = OsString>) -> u8 {
    install_catalog();
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
        Request::Delete(path, expected) => delete(&mut output, &path, expected),
    }
}

/// The usage text, which `verify_layout.py` runs the installed helper to
/// check that it starts at all.
const USAGE: &str = "\
Usage: openxplorer-admin-delete --item <device>:<inode> [--] <absolute path>

Permanently deletes one item, and everything inside it, with the rights it
is started with, if the item is still the one with that device and inode
number (as GIO reports them: the device in 32 bits). OpenXplorer starts it through pkexec after the user
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
    let mut item = None;
    let mut only_paths = false;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if !only_paths && argument == "--" {
            only_paths = true;
        } else if !only_paths && (argument == "--help" || argument == "-h") {
            return Request::Usage;
        } else if !only_paths && argument == ITEM_OPTION && item.is_none() {
            match arguments.next().as_ref().and_then(parse_item) {
                Some(identity) => item = Some(identity),
                None => return Request::Refuse(item_refusal()),
            }
        } else if !only_paths && argument.as_encoded_bytes().starts_with(b"-") {
            return Request::Refuse(gettext("This helper takes one path and no options but --item."));
        } else {
            paths.push(argument);
        }
    }
    let Some(item) = item else {
        return Request::Refuse(item_refusal());
    };
    match paths.len() {
        1 => Request::Delete(PathBuf::from(paths.remove(0)), item),
        _ => Request::Refuse(gettext("This helper deletes exactly one item at a time.")),
    }
}

/// The refusal for a request without the item's device and inode.
fn item_refusal() -> String {
    gettext("This helper needs the item’s device and inode, as --item <device>:<inode>.")
}

/// Deletes `path`, writing a record for every item, and returns the status.
fn delete(output: &mut dyn Write, path: &std::path::Path, expected: ItemIdentity) -> u8 {
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
    let result = walk::delete_tree(path, Some(expected), &cancel, &options, &mut report_record);
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

/// Speaks the desktop's language, as the app that started the helper does,
/// from the catalogue of the package the helper belongs to.
fn install_catalog() {
    let Some(domain) = std::env::current_exe().ok().as_deref().and_then(domain_of) else {
        return;
    };
    let languages: Vec<String> = glib::language_names().iter().map(ToString::to_string).collect();
    crate::i18n::install_system(&domain, &languages);
}

/// The gettext domain of the package that installed the helper at
/// `helper`, which is the package's name (`native/tools/package_data.py`):
/// `/usr/libexec/<package>/openxplorer-admin-delete` in the RPM and Arch
/// layouts and `/opt/<package>/libexec/openxplorer-admin-delete` in the
/// Debian one. Anything else, a build tree for one, has none.
fn domain_of(helper: &std::path::Path) -> Option<String> {
    let folder = helper.parent()?;
    let package = if folder.file_name()? == "libexec" {
        // Debian: /opt/<package>/libexec/
        folder.parent()?.file_name()?
    } else if folder.parent()?.file_name()? == "libexec" {
        // FHS: /usr/libexec/<package>/
        folder.file_name()?
    } else {
        return None;
    };
    let package = package.to_str()?;
    crate::i18n::is_plain_name(package).then(|| package.to_owned())
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

    /// The helper reads the catalogue of the package it came with, so
    /// what it reports is in the app's language.
    ///
    /// parity: XFER-015, INT-031
    #[test]
    fn the_helper_speaks_the_language_of_its_own_package() {
        let domain = |path: &str| domain_of(Path::new(path));

        assert_eq!(
            domain("/usr/libexec/openxplorer/openxplorer-admin-delete").as_deref(),
            Some("openxplorer")
        );
        assert_eq!(
            domain("/opt/openxplorer-native/libexec/openxplorer-admin-delete").as_deref(),
            Some("openxplorer-native")
        );
        assert_eq!(
            domain("/home/me/native/target/debug/openxplorer-admin-delete"),
            None
        );
        assert_eq!(domain("/usr/libexec/.hidden/openxplorer-admin-delete"), None);
    }

    /// A language name from the user's environment never leads the
    /// helper, which runs as root, out of the system's locale folder.
    ///
    /// parity: XFER-015, INT-031
    #[test]
    fn a_language_that_is_not_a_plain_name_is_not_read() {
        use crate::i18n::is_plain_name;

        for plain in ["zh_CN", "zh_CN.UTF-8", "sr@latin", "pt-BR"] {
            assert!(is_plain_name(plain), "{plain}");
        }
        for odd in ["", "..", "../../tmp/x", "zh_CN/../..", ".hidden", "a b"] {
            assert!(!is_plain_name(odd), "{odd:?}");
        }
    }

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

        let item = ItemIdentity {
            device: 2049,
            inode: 131_074,
        };

        assert!(matches!(
            request(&["--item", "2049:131074", "--", "/etc/x"]),
            Request::Delete(path, found) if path == Path::new("/etc/x") && found == item
        ));
        assert!(matches!(
            request(&["--item", "2049:131074", "/etc/x"]),
            Request::Delete(..)
        ));
        // A path of its own that begins with a dash is still a path.
        assert!(matches!(
            request(&["--item", "1:2", "--", "-odd"]),
            Request::Delete(..)
        ));
        assert!(matches!(request(&["--help"]), Request::Usage));
        assert!(matches!(
            request(&["--item", "1:2", "--force", "/etc/x"]),
            Request::Refuse(_)
        ));
        assert!(matches!(
            request(&["--item", "1:2", "/etc/x", "/etc/y"]),
            Request::Refuse(_)
        ));
        assert!(matches!(request(&["--item", "1:2"]), Request::Refuse(_)));
        assert!(matches!(request(&[]), Request::Refuse(_)));
    }

    /// The helper deletes nothing it was not told the identity of: a
    /// request without `--item`, or with one that is not a device and an
    /// inode, is refused, and so is a second `--item`.
    ///
    /// parity: XFER-015
    #[test]
    fn the_helper_needs_the_identity_of_the_item() {
        let request =
            |arguments: &[&str]| read_request(arguments.iter().map(|argument| OsString::from(*argument)));
        let identity = ItemIdentity {
            device: u64::from(u32::MAX),
            inode: u64::MAX,
        };

        assert!(matches!(request(&["--", "/etc/x"]), Request::Refuse(_)));
        assert!(matches!(request(&["/etc/x"]), Request::Refuse(_)));
        assert!(matches!(request(&["--item", "/etc/x"]), Request::Refuse(_)));
        assert!(matches!(
            request(&["--item", "1:x", "/etc/x"]),
            Request::Refuse(_)
        ));
        assert!(matches!(
            request(&["--item", "-1:2", "/etc/x"]),
            Request::Refuse(_)
        ));
        assert!(matches!(
            request(&["--item", "1:2", "--item", "3:4", "/etc/x"]),
            Request::Refuse(_)
        ));
        assert!(matches!(
            read_request([
                OsString::from(ITEM_OPTION),
                OsString::from(item_argument(identity)),
                OsString::from("/etc/x"),
            ]),
            Request::Delete(_, found) if found == identity
        ));
    }
}
