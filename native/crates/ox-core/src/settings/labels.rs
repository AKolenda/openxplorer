// SPDX-License-Identifier: AGPL-3.0-only
//! The labels a pin or share gets when the user gives none.
//!
//! Ports the label fallbacks of `Settings.__init__`, `bookmark` and
//! `pin_many` in `desktop/core.py`. Checking a label the user did give is
//! [`safe_label`](crate::location::safe_label).

use crate::location::{split_location, unquote_lossy, LocationParts};

/// The label of a location that has no usable name.
const FALLBACK_LABEL: &str = "Folder";

/// The label a pin or share gets from `bookmark` and when read from the
/// file: the decoded last path component, or "Folder" for a path ending in
/// `/`. Python: `unquote(path).split('/')[-1] or 'Folder'`.
pub(crate) fn bookmark_fallback_label(uri: &str) -> String {
    let Ok(parts) = split_location(uri) else {
        return FALLBACK_LABEL.to_owned();
    };
    let decoded_path = unquote_lossy(&parts.path);
    let last_component = decoded_path.rsplit('/').next().unwrap_or_default();
    if last_component.is_empty() {
        FALLBACK_LABEL.to_owned()
    } else {
        last_component.to_owned()
    }
}

/// The label a dragged-in pin gets from `pin_many`: the first non-empty of
/// the last path name (ignoring trailing slashes), the SMB host and the
/// authority, else "Folder". Python: `name or host or parts.netloc or
/// 'Folder'`.
pub(crate) fn pin_fallback_label(uri: &str) -> String {
    let Ok(parts) = split_location(uri) else {
        return FALLBACK_LABEL.to_owned();
    };
    let candidates = [last_path_name(&parts), smb_host(&parts), parts.netloc];
    candidates
        .into_iter()
        .find(|label| !label.is_empty())
        .unwrap_or_else(|| FALLBACK_LABEL.to_owned())
}

/// The decoded last component of the path, ignoring trailing slashes;
/// empty for a root. Python's `unquote(path).rstrip('/').split('/')[-1]`.
pub(crate) fn last_path_name(parts: &LocationParts) -> String {
    let decoded_path = unquote_lossy(&parts.path);
    let name = decoded_path.trim_end_matches('/').rsplit('/').next();
    name.unwrap_or_default().to_owned()
}

/// The host of an `smb://` location; empty for any other scheme.
fn smb_host(parts: &LocationParts) -> String {
    if parts.scheme == "smb" {
        parts.hostname().unwrap_or_default()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::safe_label;

    /// parity: SAFE-018, SIDE-007
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

    /// parity: SIDE-007, NET-017
    #[test]
    fn fallback_labels_match_bookmark_and_pin_rules() {
        assert_eq!(
            bookmark_fallback_label("file:///home/a/Work%20Files"),
            "Work Files"
        );
        assert_eq!(bookmark_fallback_label("file:///"), "Folder");
        assert_eq!(bookmark_fallback_label("mtp://[usb:001,002]/"), "Folder");
        assert_eq!(bookmark_fallback_label("smb://nas/Team%20files"), "Team files");
        assert_eq!(pin_fallback_label("smb://nas/"), "nas");
        assert_eq!(pin_fallback_label("smb://nas/work"), "work");
        assert_eq!(pin_fallback_label("mtp://[usb:001,002]/"), "[usb:001,002]");
        assert_eq!(pin_fallback_label("file:///"), "Folder");
    }
}
