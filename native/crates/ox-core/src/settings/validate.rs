// SPDX-License-Identifier: AGPL-3.0-only
//! Sidebar label checks and the text rules they share with the location
//! whitelist.
//!
//! Ports `safe_label` and the label fallbacks used by `Settings` in
//! `desktop/core.py`. Python's `str.strip()`, `str.isspace()` and its
//! `CONTROL` pattern (`[\x00-\x1f\x7f]`) are reproduced exactly, because a
//! label accepted by one application must be accepted by the other.

use super::SettingsError;
pub(crate) use crate::location::python_strip;
use crate::location::{split_location, unquote_lossy};

pub use crate::location::MAX_LABEL_CHARS;

/// Label used when a location has no usable name.
const FALLBACK_LABEL: &str = "Folder";

/// Validates a sidebar label. A blank label becomes `fallback`; a label
/// with control characters or more than 120 characters is rejected.
pub fn safe_label(value: &str, fallback: &str) -> Result<String, SettingsError> {
    crate::location::safe_label(value, fallback).map_err(Into::into)
}

/// The label a pin or share gets from `bookmark` and when read from the
/// file: the last component of the decoded path, or "Folder".
pub(crate) fn bookmark_fallback_label(uri: &str) -> String {
    let Ok(parts) = split_location(uri) else {
        return FALLBACK_LABEL.to_owned();
    };
    let decoded = unquote_lossy(&parts.path);
    let last = decoded.rsplit('/').next().unwrap_or_default();
    if last.is_empty() {
        FALLBACK_LABEL.to_owned()
    } else {
        last.to_owned()
    }
}

/// The label a dragged-in pin gets (`pin_many`): the last path component
/// ignoring trailing slashes, else the SMB host, else the authority, else
/// "Folder".
pub(crate) fn pin_fallback_label(uri: &str) -> String {
    let Ok(parts) = split_location(uri) else {
        return FALLBACK_LABEL.to_owned();
    };
    let decoded = unquote_lossy(&parts.path);
    let last = decoded
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    if !last.is_empty() {
        return last.to_owned();
    }
    let smb_host = if parts.scheme == "smb" {
        parts.hostname()
    } else {
        None
    };
    smb_host
        .filter(|host| !host.is_empty())
        .or_else(|| Some(parts.authority.clone()).filter(|authority| !authority.is_empty()))
        .unwrap_or_else(|| FALLBACK_LABEL.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_follow_the_python_rules() {
        assert_eq!(safe_label("  Work  ", "Folder").unwrap(), "Work");
        assert_eq!(safe_label("   ", "Folder").unwrap(), "Folder");
        assert_eq!(safe_label("", "Fallback").unwrap(), "Fallback");
        assert_eq!(safe_label("\u{1c}Work\u{a0}", "Folder").unwrap(), "Work");
        assert_eq!(safe_label(&"é".repeat(120), "x").unwrap(), "é".repeat(120));
        assert!(safe_label(&"é".repeat(121), "x").is_err());
        assert!(safe_label("bad\nlabel", "x").is_err());
        assert!(safe_label("bad\u{7f}", "x").is_err());
    }

    #[test]
    fn fallback_labels_match_bookmark_and_pin_rules() {
        assert_eq!(
            bookmark_fallback_label("file:///home/a/Work%20Files"),
            "Work Files"
        );
        assert_eq!(bookmark_fallback_label("file:///"), "Folder");
        assert_eq!(bookmark_fallback_label("mtp://[usb:001,002]/"), "Folder");
        assert_eq!(pin_fallback_label("smb://nas/"), "nas");
        assert_eq!(pin_fallback_label("smb://nas/work"), "work");
        assert_eq!(pin_fallback_label("mtp://[usb:001,002]/"), "[usb:001,002]");
        assert_eq!(pin_fallback_label("file:///"), "Folder");
    }
}
