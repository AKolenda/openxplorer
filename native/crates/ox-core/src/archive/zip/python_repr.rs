// SPDX-License-Identifier: AGPL-3.0-only
//! Names quoted the way Python's `repr` quotes them, so the messages of
//! [`ZipFormatError`](super::ZipFormatError) read as `zipfile`'s `%r` and
//! `!r` print them: `'doc.txt'`, `"Bob's.txt"` and `b'report.exe'`.
//!
//! Ports the quoting and escaping of `unicode_repr` in
//! `Objects/unicodeobject.c` and `bytes_repr` in `Objects/bytesobject.c`,
//! the C functions behind Python's `repr` of text and bytes.

use std::fmt::{self, Write};

/// A name displayed as Python's `repr` of a `str`.
///
/// Python also escapes the characters outside ASCII and the C1 controls
/// that it counts as unprintable, such as U+200B ZERO WIDTH SPACE; those
/// are written unchanged here.
#[derive(Debug, Clone, Copy)]
pub(super) struct PythonRepr<'a>(pub(super) &'a str);

/// Bytes displayed as Python's `repr` of a `bytes` object: `b'...'`, with
/// every byte outside printable ASCII escaped as `\xNN`.
#[derive(Debug, Clone, Copy)]
pub(super) struct PythonBytesRepr<'a>(pub(super) &'a [u8]);

impl fmt::Display for PythonRepr<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let quote = python_quote(self.0.contains('\''), self.0.contains('"'));
        formatter.write_char(quote)?;
        for character in self.0.chars() {
            if character.is_ascii() {
                write_ascii(formatter, character, quote)?;
            } else if character.is_control() {
                // The C1 controls, U+0080 to U+009F.
                write!(formatter, "\\x{:02x}", u32::from(character))?;
            } else {
                formatter.write_char(character)?;
            }
        }
        formatter.write_char(quote)
    }
}

impl fmt::Display for PythonBytesRepr<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let quote = python_quote(self.0.contains(&b'\''), self.0.contains(&b'"'));
        formatter.write_char('b')?;
        formatter.write_char(quote)?;
        for &byte in self.0 {
            if byte.is_ascii() {
                write_ascii(formatter, char::from(byte), quote)?;
            } else {
                write!(formatter, "\\x{byte:02x}")?;
            }
        }
        formatter.write_char(quote)
    }
}

/// The quote Python's `repr` puts around text: a single quote, unless the
/// text holds a single quote and no double quote.
fn python_quote(has_single_quote: bool, has_double_quote: bool) -> char {
    if has_single_quote && !has_double_quote {
        '"'
    } else {
        '\''
    }
}

/// Writes one ASCII `character` as Python's `repr` writes it between
/// `quote`s.
fn write_ascii(formatter: &mut fmt::Formatter<'_>, character: char, quote: char) -> fmt::Result {
    match character {
        '\\' => formatter.write_str("\\\\"),
        '\t' => formatter.write_str("\\t"),
        '\n' => formatter.write_str("\\n"),
        '\r' => formatter.write_str("\\r"),
        _ if character == quote => write!(formatter, "\\{quote}"),
        _ if character.is_ascii_control() => write!(formatter, "\\x{:02x}", u32::from(character)),
        _ => formatter.write_char(character),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A text and the `repr` Python 3.12 prints for it.
    struct ReprCase {
        text: &'static str,
        python_repr: &'static str,
    }

    #[test]
    fn names_are_quoted_as_python_repr_quotes_them() {
        let cases = [
            ReprCase {
                text: "doc.txt",
                python_repr: "'doc.txt'",
            },
            ReprCase {
                text: "Bob's.txt",
                python_repr: "\"Bob's.txt\"",
            },
            ReprCase {
                text: "a\"b",
                python_repr: "'a\"b'",
            },
            ReprCase {
                text: "both'\"",
                python_repr: "'both\\'\"'",
            },
            ReprCase {
                text: "tab\there\\\u{7f}\u{85}",
                python_repr: "'tab\\there\\\\\\x7f\\x85'",
            },
            ReprCase {
                text: "café",
                python_repr: "'café'",
            },
        ];
        for case in cases {
            assert_eq!(PythonRepr(case.text).to_string(), case.python_repr);
        }
    }

    #[test]
    fn bytes_are_quoted_with_a_b_and_escaped_outside_ascii() {
        assert_eq!(PythonBytesRepr(b"report.exe").to_string(), "b'report.exe'");
        assert_eq!(PythonBytesRepr(b"caf\xc3\xa9").to_string(), "b'caf\\xc3\\xa9'");
        assert_eq!(PythonBytesRepr(b"it's").to_string(), "b\"it's\"");
        assert_eq!(PythonBytesRepr(b"a\nb\0").to_string(), "b'a\\nb\\x00'");
    }
}
