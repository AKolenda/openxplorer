// SPDX-License-Identifier: AGPL-3.0-only
//! Which kind of location a URI names, for the rules that treat network
//! shares and real folders differently.
//!
//! Ports the scheme tests of `desktop/ui/app.js`
//! (`uri.startsWith('smb:')`, and `startsWith('file:')` for a new window).
//! The locations the window holds are canonical, with a lower-case scheme
//! (`normalise_location`, GIO), so the prefix test is exact.

/// Whether `uri` is on an SMB share: its tab, address bar, card and
/// details pane say "network".
pub(crate) fn is_smb_location(uri: &str) -> bool {
    uri.starts_with("smb:")
}

/// Whether `uri` is a folder on this computer or on an SMB share, rather
/// than a landing page, a device or another virtual place. A new window
/// starts in such a folder (`newWindow` in app.js).
pub(crate) fn is_local_or_smb_location(uri: &str) -> bool {
    uri.starts_with("file:") || is_smb_location(uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_smb_addresses_are_network_shares() {
        assert!(is_smb_location("smb://nas/media"));
        assert!(!is_smb_location("file:///srv/smb"));
        assert!(!is_smb_location("mtp://phone/"));
    }

    #[test]
    fn a_new_window_can_start_in_a_local_or_smb_folder_only() {
        assert!(is_local_or_smb_location("file:///home/demo"));
        assert!(is_local_or_smb_location("smb://nas/media"));
        assert!(!is_local_or_smb_location("ox:pc"));
        assert!(!is_local_or_smb_location("trash:///"));
    }
}
