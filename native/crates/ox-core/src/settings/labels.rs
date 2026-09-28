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
pub(super) fn bookmark_fallback_label(uri: &str) -> String {
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
pub(super) fn pin_fallback_label(uri: &str) -> String {
    let Ok(parts) = split_location(uri) else {
        return FALLBACK_LABEL.to_owned();
    };
    let candidates = [parts.last_name(), smb_host(&parts), parts.authority];
    candidates
        .into_iter()
        .find(|label| !label.is_empty())
        .unwrap_or_else(|| FALLBACK_LABEL.to_owned())
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
