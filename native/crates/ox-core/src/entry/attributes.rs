// SPDX-License-Identifier: AGPL-3.0-only
//! Typed readers for optional `GFileInfo` attributes.
//!
//! Replaces the `info.get_*()` calls of `entry_from_info` in
//! `desktop/gio_backend.py`. Only attributes that are present are read:
//! since version 2.76 the typed getters (`g_file_info_get_size` and friends)
//! log a critical warning for a missing attribute, and backends such as
//! gvfsd-smb-browse omit many of them.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

/// A string attribute; `None` when missing or empty.
pub(super) fn string_attribute(info: &gio::FileInfo, attribute: &str) -> Option<String> {
    info.attribute_string(attribute)
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty())
}

/// `Some(flag)` when the backend reported the attribute, `None` when it did
/// not (callers then let the operation itself decide).
pub(super) fn optional_boolean(info: &gio::FileInfo, attribute: &str) -> Option<bool> {
    info.has_attribute(attribute).then(|| info.boolean(attribute))
}

/// A path attribute (`thumbnail::path`, `trash::orig-path`), byte for byte.
///
/// These are byte strings and need not be UTF-8, so they are read through
/// GIO's escaped text form and unescaped back to the exact bytes.
pub(super) fn path_attribute(info: &gio::FileInfo, attribute: &str) -> Option<PathBuf> {
    let bytes = match info.attribute_type(attribute) {
        gio::FileAttributeType::ByteString => {
            let escaped = info.attribute_as_string(attribute)?;
            unescape_byte_string(&escaped)
        }
        gio::FileAttributeType::String => info.attribute_string(attribute)?.as_bytes().to_vec(),
        _ => return None,
    };
    if bytes.is_empty() {
        return None;
    }
    Some(PathBuf::from(OsString::from_vec(bytes)))
}

/// The length of one `\xNN` escape.
const ESCAPE_LENGTH: usize = 4;

/// Reverses GIO's `escape_byte_string`: every byte outside printable ASCII,
/// and every backslash, is written as `\xNN`.
fn unescape_byte_string(escaped: &str) -> Vec<u8> {
    let bytes = escaped.as_bytes();
    let mut unescaped = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if let Some(byte) = escaped_byte_at(bytes, index) {
            unescaped.push(byte);
            index += ESCAPE_LENGTH;
        } else {
            unescaped.push(bytes[index]);
            index += 1;
        }
    }
    unescaped
}

/// The byte encoded by a `\xNN` sequence starting at `index`, if any.
fn escaped_byte_at(bytes: &[u8], index: usize) -> Option<u8> {
    let [b'\\', b'x', high, low] = *bytes.get(index..index + ESCAPE_LENGTH)? else {
        return None;
    };
    Some(hex_value(high)? * 16 + hex_value(low)?)
}

fn hex_value(digit: u8) -> Option<u8> {
    let value = char::from(digit).to_digit(16)?;
    u8::try_from(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescapes_gio_byte_strings() {
        assert_eq!(
            unescape_byte_string(r"/a\x5cb/\xc3\xa9\xff"),
            b"/a\\b/\xc3\xa9\xff".to_vec()
        );
    }

    #[test]
    fn incomplete_or_invalid_escapes_stay_literal() {
        assert_eq!(unescape_byte_string(r"\x4"), b"\\x4".to_vec());
        assert_eq!(unescape_byte_string(r"\xzz"), b"\\xzz".to_vec());
    }

    #[test]
    fn empty_and_missing_strings_are_none() {
        let info = gio::FileInfo::new();
        info.set_attribute_string("standard::content-type", "");
        assert_eq!(string_attribute(&info, "standard::content-type"), None);
        assert_eq!(string_attribute(&info, "standard::target-uri"), None);
        assert_eq!(path_attribute(&info, "trash::orig-path"), None);
    }
}
