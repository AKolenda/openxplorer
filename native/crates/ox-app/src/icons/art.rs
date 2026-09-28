// SPDX-License-Identifier: AGPL-3.0-only
//! What an icon shows: a glyph, a place's coloured glyph, the folder, a
//! file's type, a ZIP archive, or a network location on the green bar.
//!
//! Replaces the choice between `icon()`, `folderIcon`, `zipFolderIcon`,
//! `fileIcon` and `networkIcon` in `desktop/ui/app.js`. [`ArtImage`] shows
//! the result from real icons.
//!
//! Network locations follow the owner's reading of Windows Explorer (icon
//! mapping, 2026-09-27 and 2026-09-28): a share stands on the green bar as
//! the folder, a mapped drive (a location whose label names a drive
//! letter, such as "Studio NAS (Z:)") as a drive, a server as a server, and
//! a share or mapped drive that is not connected carries a red cross. The
//! same art shows a network location everywhere it appears: the sidebar,
//! the cards of This PC and Network, the tabs and the details pane.
//!
//! [`ArtImage`]: crate::icons::ArtImage

use ox_core::entry::Entry;
use ox_core::places::{KnownFolder, NetworkKind, NetworkLocation};

use super::file_type::{is_zip, FileType};
use super::tint::Tint;
use super::Icon;
use crate::places::network_row;

/// What an icon shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Art {
    /// One glyph in the text colour of its widget.
    Glyph(Icon),
    /// One glyph in a place's own colour (Home, This PC, Network, a
    /// standard folder), as the sidebar draws them.
    TintedGlyph(Icon, Tint),
    /// The yellow folder.
    Folder,
    /// A ZIP archive: the folder with the zip badge in its corner.
    ZipFolder,
    /// A file, in the colour icon of its type.
    File(FileType),
    /// A network location on the green network bar.
    Network(NetworkArt),
}

/// A network location as its icon shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct NetworkArt {
    /// The picture on the bar.
    pub(crate) place: NetworkPlace,
    /// Whether the location is connected; a share or mapped drive that is
    /// not shows the red cross.
    pub(crate) connection: Connection,
}

/// The picture a network location shows on the green bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum NetworkPlace {
    /// A shared folder (`\\server\share`): the yellow folder.
    Share,
    /// A share mapped as a drive: the drive glyph.
    MappedDrive,
    /// An SMB server listing its shares: the server glyph.
    Server,
    /// A standard folder relocated onto a network mount: its own glyph, in
    /// its colour (`networkIcon(19, glyph)` in app.js).
    KnownFolder(KnownFolder),
}

/// Whether a network location is connected now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Connection {
    /// Mounted, or never checked (a share listed inside a server).
    Connected,
    /// Saved or visited, but not mounted now.
    Disconnected,
}

impl Connection {
    /// The connection of a location GIO reports as mounted or not.
    pub(crate) const fn from_mounted(is_mounted: bool) -> Self {
        if is_mounted {
            Connection::Connected
        } else {
            Connection::Disconnected
        }
    }
}

/// Where a folder is stored, which decides whether its icon stands on the
/// green network bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Storage {
    /// On this computer.
    Local,
    /// On an SMB share or a local network mount.
    Network,
}

impl Art {
    /// A connected network share: the folder on the green bar.
    pub(crate) const SHARE: Art = Art::Network(NetworkArt {
        place: NetworkPlace::Share,
        connection: Connection::Connected,
    });

    /// The art of a listed item: a share in a server listing on the bar, a
    /// folder, a ZIP archive as a folder with the zip badge, else the
    /// colour icon of the file's type. A folder whose name ends in `.zip`
    /// stays a plain folder.
    pub(crate) fn for_entry(entry: &Entry) -> Self {
        if entry.is_dir && entry.is_virtual {
            return Art::SHARE;
        }
        if entry.is_dir {
            return Art::Folder;
        }
        let content_type = entry.content_type.as_deref();
        if is_zip(&entry.name, content_type) {
            return Art::ZipFolder;
        }
        Art::File(FileType::of(&entry.name, content_type))
    }

    /// The art of a file known only by its name, such as a member of a ZIP
    /// archive: a ZIP as the folder with the zip badge, else the colour
    /// icon of the file's type.
    pub(crate) fn for_file_name(name: &str) -> Self {
        if is_zip(name, None) {
            return Art::ZipFolder;
        }
        Art::File(FileType::of(name, None))
    }

    /// The art of a network location called `label`: a server, or a share
    /// shown as a folder or, when its label names a drive letter, a drive.
    pub(crate) fn for_network_location(kind: NetworkKind, label: &str, connection: Connection) -> Self {
        let place = match kind {
            NetworkKind::Server => NetworkPlace::Server,
            NetworkKind::Share | NetworkKind::Mount => NetworkPlace::for_share_label(label),
        };
        Art::Network(NetworkArt { place, connection })
    }

    /// The art of a row of the Network list, as the sidebar shows it.
    pub(crate) fn for_network_row(location: &NetworkLocation) -> Self {
        let connection = Connection::from_mounted(location.is_connected);
        Self::for_network_location(location.kind, &location.label, connection)
    }

    /// The art of the SMB location `uri`, for its tab and the details pane:
    /// the art its row of `network` has in the sidebar (a server, a share
    /// or a mapped drive, with the red cross while it is not connected),
    /// else a share on the network bar, as for a folder inside a share
    /// (`networkIcon` for every SMB tab in app.js).
    pub(crate) fn for_smb_location(uri: &str, network: &[NetworkLocation]) -> Self {
        network_row(network, uri).map_or(Art::SHARE, Art::for_network_row)
    }

    /// The art of a Quick access folder: a standard folder's glyph (in its
    /// colour, when it has one) or the yellow folder for a pin, standing on
    /// the green bar when the folder is on the network.
    pub(crate) fn for_quick_access(folder: Option<KnownFolder>, storage: Storage) -> Self {
        match (folder, storage) {
            (Some(folder), Storage::Network) => Art::Network(NetworkArt {
                place: NetworkPlace::KnownFolder(folder),
                connection: Connection::Connected,
            }),
            (Some(folder), Storage::Local) => Self::for_known_folder(folder),
            (None, Storage::Network) => Art::SHARE,
            (None, Storage::Local) => Art::Folder,
        }
    }

    /// A standard folder's glyph, in its colour when it has one; the yellow
    /// folder for the Public folder, which has no glyph.
    pub(crate) fn for_known_folder(folder: KnownFolder) -> Self {
        let Some(glyph) = Icon::for_known_folder(folder) else {
            return Art::Folder;
        };
        match Tint::for_known_folder(folder) {
            Some(tint) => Art::TintedGlyph(glyph, tint),
            None => Art::Glyph(glyph),
        }
    }
}

impl NetworkPlace {
    /// A share is shown as a mapped drive when `label` ends in a drive
    /// letter in brackets, as Windows names one ("Media (M:)").
    fn for_share_label(label: &str) -> Self {
        if names_a_drive_letter(label) {
            NetworkPlace::MappedDrive
        } else {
            NetworkPlace::Share
        }
    }

    /// True for the pictures that can show the red cross: Windows marks
    /// only disconnected shares and mapped drives.
    pub(crate) const fn can_be_disconnected(self) -> bool {
        matches!(self, NetworkPlace::Share | NetworkPlace::MappedDrive)
    }
}

/// True when `label` ends in a drive letter in brackets, such as "(Z:)".
fn names_a_drive_letter(label: &str) -> bool {
    let Some(inside) = label.trim_end().strip_suffix(":)") else {
        return false;
    };
    let mut letter_and_bracket = inside.chars().rev();
    let letter = letter_and_bracket.next();
    let bracket = letter_and_bracket.next();
    letter.is_some_and(|letter| letter.is_ascii_alphabetic()) && bracket == Some('(')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        file_entry as file, folder_entry as folder, studio_nas_mapped_drive, studio_nas_server,
    };

    /// parity: LOOK-016
    #[test]
    fn an_smb_location_shows_the_art_of_its_network_row_or_a_share() {
        let network = [studio_nas_server(), studio_nas_mapped_drive()];
        let art_of = |uri: &str| Art::for_smb_location(uri, &network);
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

    /// parity: ARC-001
    #[test]
    fn a_folder_named_zip_keeps_the_plain_folder() {
        assert_eq!(Art::for_entry(&folder("Archive.zip")), Art::Folder);
        assert_eq!(Art::for_entry(&file("Archive.zip")), Art::ZipFolder);
    }

    /// parity: LOOK-015
    #[test]
    fn names_without_a_known_type_share_the_document() {
        assert_eq!(Art::for_entry(&file("ls")), Art::for_entry(&file("cargo")));
        assert_eq!(Art::for_entry(&file("ls")), Art::File(FileType::Document));
        assert_eq!(Art::for_entry(&file("Report.PDF")), Art::File(FileType::Document));
        assert_eq!(Art::for_entry(&file("Notes.txt")), Art::File(FileType::Text));
    }

    /// parity: LOOK-016
    #[test]
    fn a_share_in_a_server_listing_stands_on_the_network_bar() {
        let mut share = folder("media");
        share.is_virtual = true;
        assert_eq!(Art::for_entry(&share), Art::SHARE);
    }

    /// A network location and the picture it shows on the bar.
    struct NetworkCase {
        kind: NetworkKind,
        label: &'static str,
        expected: NetworkPlace,
    }

    /// parity: LOOK-016
    #[test]
    fn shares_with_a_drive_letter_show_a_drive_and_servers_a_server() {
        let cases = [
            NetworkCase {
                kind: NetworkKind::Share,
                label: "Shared library",
                expected: NetworkPlace::Share,
            },
            NetworkCase {
                kind: NetworkKind::Share,
                label: "Studio NAS (Z:)",
                expected: NetworkPlace::MappedDrive,
            },
            NetworkCase {
                kind: NetworkKind::Mount,
                label: "Media (m:) ",
                expected: NetworkPlace::MappedDrive,
            },
            NetworkCase {
                kind: NetworkKind::Share,
                label: "Backups (2024:)",
                expected: NetworkPlace::Share,
            },
            NetworkCase {
                kind: NetworkKind::Server,
                label: "studio-nas (Z:)",
                expected: NetworkPlace::Server,
            },
        ];
        for case in cases {
            let art = Art::for_network_location(case.kind, case.label, Connection::Connected);
            let expected = Art::Network(NetworkArt {
                place: case.expected,
                connection: Connection::Connected,
            });
            assert_eq!(art, expected, "{}", case.label);
        }
    }

    /// parity: LOOK-016
    #[test]
    fn a_network_row_shows_its_kind_label_and_connection() {
        let crossed_out_drive = Art::Network(NetworkArt {
            place: NetworkPlace::MappedDrive,
            connection: Connection::Disconnected,
        });
        assert_eq!(
            Art::for_network_row(&studio_nas_mapped_drive()),
            crossed_out_drive
        );
        let server = Art::Network(NetworkArt {
            place: NetworkPlace::Server,
            connection: Connection::Connected,
        });
        assert_eq!(Art::for_network_row(&studio_nas_server()), server);
    }

    #[test]
    fn only_shares_and_mapped_drives_show_that_they_are_disconnected() {
        assert!(NetworkPlace::Share.can_be_disconnected());
        assert!(NetworkPlace::MappedDrive.can_be_disconnected());
        assert!(!NetworkPlace::Server.can_be_disconnected());
        let documents = NetworkPlace::KnownFolder(KnownFolder::Documents);
        assert!(!documents.can_be_disconnected());
        assert_eq!(Connection::from_mounted(false), Connection::Disconnected);
        assert_eq!(Connection::from_mounted(true), Connection::Connected);
    }

    /// A Quick access folder, where it is stored, and the art it gets.
    struct QuickAccessCase {
        folder: Option<KnownFolder>,
        storage: Storage,
        expected: Art,
    }

    /// parity: LOOK-016
    #[test]
    fn quick_access_shows_tinted_glyphs_and_folders_on_the_bar_when_shared() {
        let music_on_network = Art::Network(NetworkArt {
            place: NetworkPlace::KnownFolder(KnownFolder::Music),
            connection: Connection::Connected,
        });
        let cases = [
            QuickAccessCase {
                folder: Some(KnownFolder::Downloads),
                storage: Storage::Local,
                expected: Art::TintedGlyph(Icon::ArrowDownload, Tint::KnownFolder(KnownFolder::Downloads)),
            },
            QuickAccessCase {
                folder: Some(KnownFolder::Music),
                storage: Storage::Network,
                expected: music_on_network,
            },
            QuickAccessCase {
                folder: None,
                storage: Storage::Local,
                expected: Art::Folder,
            },
            QuickAccessCase {
                folder: None,
                storage: Storage::Network,
                expected: Art::SHARE,
            },
        ];
        for case in cases {
            let art = Art::for_quick_access(case.folder, case.storage);
            assert_eq!(art, case.expected, "{:?} on {:?}", case.folder, case.storage);
        }
    }

    #[test]
    fn templates_have_an_untinted_glyph_and_public_the_folder() {
        assert_eq!(
            Art::for_known_folder(KnownFolder::Templates),
            Art::Glyph(Icon::Document)
        );
        assert_eq!(Art::for_known_folder(KnownFolder::Public), Art::Folder);
    }
}
