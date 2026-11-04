// SPDX-License-Identifier: AGPL-3.0-only
//! The line protocol the privileged helper writes and the app reads.
//!
//! The helper runs in its own process under `pkexec`, so everything the app
//! learns about a deletion arrives on the helper's standard output. The
//! format is deliberately plain text, one record per line, so that a
//! maintainer can run the helper in a terminal and read what it did.
//!
//! ```text
//! deleted\t<path>
//! skipped\t<path>\t<reason>
//! failed\t<path>\t<message>
//! refused\t<message>
//! cancelled
//! done\t<deleted>\t<skipped>\t<failed>
//! ```
//!
//! Fields are separated by a single tab. `done` is the last line of a walk
//! that started; `refused` is the only line of a request that was turned
//! down before anything was deleted.
//!
//! A path is a filesystem path, which is bytes rather than text on Linux,
//! so every field is escaped: a backslash becomes `\\`, a tab `\t`, a
//! newline `\n`, a carriage return `\r`, and any other control character
//! becomes `\xHH` for each of its bytes, with two lowercase hexadecimal
//! digits. A field that is valid UTF-8 keeps the rest of its text as it is,
//! so a name with accents or in a non-Latin script stays readable in a
//! terminal; one that is not valid UTF-8 has every byte outside printable
//! ASCII written as `\xHH`. Either way a record never spans more than one
//! line and a path can never be mistaken for a field separator.

use std::ffi::OsString;
use std::fmt::Write;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

/// One thing the helper did to one item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    /// The item is gone.
    Deleted(PathBuf),
    /// The item was left in place on purpose.
    Skipped {
        /// The item that is still there.
        path: PathBuf,
        /// Why the walk left it alone.
        reason: String,
    },
    /// The item could not be removed; the walk went on.
    Failed {
        /// The item that is still there.
        path: PathBuf,
        /// What the system said when it could not be removed.
        message: String,
    },
}

/// How many items the walk deleted, left in place and failed on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    /// Items that are gone.
    pub deleted: u64,
    /// Items the walk left alone on purpose.
    pub skipped: u64,
    /// Items the walk could not remove.
    pub failed: u64,
}

impl Totals {
    /// True when nothing was left behind.
    pub fn is_complete(self) -> bool {
        self.skipped == 0 && self.failed == 0
    }

    /// Counts one record.
    pub(crate) fn count(&mut self, record: &Record) {
        match record {
            Record::Deleted(_) => self.deleted += 1,
            Record::Skipped { .. } => self.skipped += 1,
            Record::Failed { .. } => self.failed += 1,
        }
    }
}

/// One line of the helper's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// What the helper did to one item.
    Record(Record),
    /// The request was turned down before anything was deleted.
    Refused(String),
    /// The walk stopped because the app cancelled it.
    Cancelled,
    /// The walk finished; these are its totals.
    Done(Totals),
}

impl Line {
    /// The line as the helper writes it, without its line break.
    pub fn encode(&self) -> String {
        match self {
            Line::Record(Record::Deleted(path)) => format!("deleted\t{}", escape_path(path)),
            Line::Record(Record::Skipped { path, reason }) => {
                format!("skipped\t{}\t{}", escape_path(path), escape(reason.as_bytes()))
            }
            Line::Record(Record::Failed { path, message }) => {
                format!("failed\t{}\t{}", escape_path(path), escape(message.as_bytes()))
            }
            Line::Refused(message) => format!("refused\t{}", escape(message.as_bytes())),
            Line::Cancelled => "cancelled".to_owned(),
            Line::Done(totals) => {
                format!("done\t{}\t{}\t{}", totals.deleted, totals.skipped, totals.failed)
            }
        }
    }

    /// The line a helper wrote, or `None` for one this version does not
    /// know: an older app must ignore a record a newer helper adds rather
    /// than treat the deletion as broken.
    pub fn decode(line: &str) -> Option<Self> {
        // A field never holds a real tab: everything that could break the
        // format is escaped, so the fields are exactly the parts between
        // the separators, and a record with the wrong number of them did
        // not come from a helper this app understands.
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            ["deleted", path] => Some(Line::Record(Record::Deleted(unescape_path(path)?))),
            ["skipped", path, reason] => Some(Line::Record(Record::Skipped {
                path: unescape_path(path)?,
                reason: unescape_text(reason)?,
            })),
            ["failed", path, message] => Some(Line::Record(Record::Failed {
                path: unescape_path(path)?,
                message: unescape_text(message)?,
            })),
            ["refused", message] => Some(Line::Refused(unescape_text(message)?)),
            ["cancelled"] => Some(Line::Cancelled),
            ["done", deleted, skipped, failed] => Some(Line::Done(Totals {
                deleted: deleted.parse().ok()?,
                skipped: skipped.parse().ok()?,
                failed: failed.parse().ok()?,
            })),
            _ => None,
        }
    }
}

/// A path as one escaped field.
fn escape_path(path: &Path) -> String {
    escape(path.as_os_str().as_bytes())
}

/// `bytes` with everything that could break the line format escaped.
fn escape(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => escape_text(text),
        Err(_) => escape_bytes(bytes),
    }
}

/// Text with only the characters that would break a line, or move a
/// terminal's cursor, replaced. Everything else stays as it is.
fn escape_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            control if control.is_control() => {
                let mut buffer = [0_u8; 4];
                escaped.push_str(&escape_bytes(control.encode_utf8(&mut buffer).as_bytes()));
            }
            other => escaped.push(other),
        }
    }
    escaped
}

/// Bytes that are not text, written so that every one of them comes back.
fn escape_bytes(bytes: &[u8]) -> String {
    let mut escaped = String::with_capacity(bytes.len());
    for &byte in bytes {
        match byte {
            b'\\' => escaped.push_str("\\\\"),
            b'\t' => escaped.push_str("\\t"),
            b'\n' => escaped.push_str("\\n"),
            b'\r' => escaped.push_str("\\r"),
            0x20..=0x7e => escaped.push(byte as char),
            other => write!(escaped, "\\x{other:02x}").expect("a String never fails to grow"),
        }
    }
    escaped
}

/// The bytes an escaped field stands for, or `None` for a malformed one.
fn unescape(field: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(field.len());
    let mut characters = field.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            // A control character is always escaped, so an unescaped one
            // means the field did not come from this format.
            if character.is_control() {
                return None;
            }
            let mut buffer = [0_u8; 4];
            bytes.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
            continue;
        }
        match characters.next()? {
            '\\' => bytes.push(b'\\'),
            't' => bytes.push(b'\t'),
            'n' => bytes.push(b'\n'),
            'r' => bytes.push(b'\r'),
            'x' => {
                let digits: String = characters.by_ref().take(2).collect();
                bytes.push(u8::from_str_radix(&digits, 16).ok()?);
            }
            _ => return None,
        }
    }
    Some(bytes)
}

/// An escaped field as a path. An empty one is a mangled line: the
/// helper always names the item a record is about.
fn unescape_path(field: &str) -> Option<PathBuf> {
    let bytes = unescape(field)?;
    if bytes.is_empty() {
        return None;
    }
    Some(PathBuf::from(OsString::from_vec(bytes)))
}

/// An escaped field as a message. A helper's messages are translated
/// text, so one that is not valid UTF-8 is a mangled line, not a path.
fn unescape_text(field: &str) -> Option<String> {
    String::from_utf8(unescape(field)?).ok()
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    /// Every line form survives a round trip through the format.
    ///
    /// parity: XFER-015
    #[test]
    fn every_record_survives_encoding_and_decoding() {
        let lines = [
            Line::Record(Record::Deleted(PathBuf::from("/etc/a folder/file.txt"))),
            Line::Record(Record::Skipped {
                path: PathBuf::from("/etc/mounted"),
                reason: "another drive".to_owned(),
            }),
            Line::Record(Record::Failed {
                path: PathBuf::from("/etc/busy"),
                message: "Device or resource busy".to_owned(),
            }),
            Line::Refused("Filesystem roots cannot be deleted.".to_owned()),
            Line::Cancelled,
            Line::Done(Totals {
                deleted: 7,
                skipped: 1,
                failed: 2,
            }),
        ];

        for line in lines {
            assert_eq!(Line::decode(&line.encode()), Some(line.clone()), "{line:?}");
        }
    }

    /// A path is bytes: a newline, a tab and a byte that is not UTF-8 all
    /// have to come back unchanged, and none of them may split the line.
    ///
    /// parity: XFER-015
    #[test]
    fn a_path_with_a_newline_a_tab_and_invalid_utf8_stays_one_line() {
        let awkward = PathBuf::from(OsStr::from_bytes(b"/etc/od\nd\tname\\\xff"));

        let encoded = Line::Record(Record::Deleted(awkward.clone())).encode();

        assert!(
            !encoded.contains('\n') && encoded.matches('\t').count() == 1,
            "{encoded}"
        );
        assert_eq!(
            Line::decode(&encoded),
            Some(Line::Record(Record::Deleted(awkward)))
        );
    }

    /// A maintainer reads this output in a terminal, so a name that is
    /// ordinary text stays ordinary text.
    ///
    /// parity: XFER-015
    #[test]
    fn a_name_that_is_text_is_not_turned_into_hexadecimal() {
        let readable = PathBuf::from("/srv/Berichte/Müller/文書.txt");

        let encoded = Line::Record(Record::Deleted(readable.clone())).encode();

        assert_eq!(encoded, "deleted\t/srv/Berichte/Müller/文書.txt");
        assert_eq!(
            Line::decode(&encoded),
            Some(Line::Record(Record::Deleted(readable)))
        );
    }

    /// parity: XFER-015
    #[test]
    fn a_line_this_version_does_not_know_is_ignored_rather_than_misread() {
        assert_eq!(Line::decode("renamed\t/etc/x"), None);
        assert_eq!(Line::decode("deleted"), None);
        assert_eq!(Line::decode("deleted\t"), None);
        assert_eq!(Line::decode("deleted\t/etc/x\textra"), None);
        assert_eq!(Line::decode("cancelled\t"), None);
        assert_eq!(Line::decode("done\t1\t2"), None);
    }
}
