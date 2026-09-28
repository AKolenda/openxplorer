// SPDX-License-Identifier: AGPL-3.0-only
//! Which kind of location a URI names, for the rules that treat network
//! shares and real folders differently, and the art an SMB location shows
//! in its tab and in the details pane.
//!
//! Ports the scheme tests of `desktop/ui/app.js`
//! (`uri.startsWith('smb:')`, and `startsWith('file:')` for a new window).
//! The locations the window holds are canonical, with a lower-case scheme
//! (`normalise_location`, GIO), so the prefix test is exact.

use ox_core::places::NetworkLocation;

use crate::icons::Art;
use crate::places::network_row;

/// Whether `uri` is on an SMB share: its tab, address bar, card and
/// details pane say "network".
pub(crate) fn is_smb_location(uri: &str) -> bool {
    uri.starts_with("smb:")
}

/// The art of the SMB location `uri`, for its tab and the details pane:
/// the art its row of `network` has in the sidebar (a server, a share or a
/// mapped drive, with the red cross while it is not connected), else a
/// share on the network bar, as for a folder inside a share
/// (`networkIcon` for every SMB tab in app.js).
pub(crate) fn smb_location_art(uri: &str, network: &[NetworkLocation]) -> Art {
    network_row(network, uri).map_or(Art::SHARE, Art::for_network_row)
}

/// Whether `uri` is a folder on this computer or on an SMB share, rather
/// than a landing page, a device or another virtual place. A new window
/// starts in such a folder (`newWindow` in app.js).
pub(crate) fn is_local_or_smb_location(uri: &str) -> bool {
    uri.starts_with("file:") || is_smb_location(uri)
}

#[cfg(test)]
mod tests {
    use ox_core::places::NetworkKind;

    use super::*;
    use crate::icons::Connection;
    use crate::test_support::{studio_nas_mapped_drive, studio_nas_server};

    /// parity: LOOK-016
    #[test]
    fn an_smb_location_shows_the_art_of_its_network_row_or_a_share() {
        let network = [studio_nas_server(), studio_nas_mapped_drive()];
        let art_of = |uri: &str| smb_location_art(uri, &network);
        let server = Art::for_network_location(NetworkKind::Server, "studio-nas", Connection::Connected);
        assert_eq!(art_of("smb://studio-nas/"), server);
        let crossed_out_drive =
            Art::for_network_location(NetworkKind::Share, "Studio NAS (Z:)", Connection::Disconnected);
        assert_eq!(art_of("smb://studio-nas/projects"), crossed_out_drive);
        assert_eq!(
            art_of("smb://studio-nas/projects/2024"),
            Art::SHARE,
            "a folder inside"
        );
        assert_eq!(art_of("smb://other-nas/media"), Art::SHARE, "not in the list");
    }

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
