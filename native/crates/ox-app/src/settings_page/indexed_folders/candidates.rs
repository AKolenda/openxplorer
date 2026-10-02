// SPDX-License-Identifier: AGPL-3.0-only
//! The folders Settings offers the search index, and why.
//!
//! Ports the candidate list of `renderSettingsCache` in
//! `v2.0.0:desktop/ui/app.js` (SET-006): the folder shown before Settings opened,
//! the folders indexed before, Home, Quick access, saved shares, the Local
//! Disk and mounted drives, once each and in that order, never pages, GIO
//! virtual folders, devices or SMB servers. Each carries the reason the
//! mockup's "Suggested because" column shows, and its location.

use ox_core::location::{
    is_device_location, is_smb_server, same_location, split_location, LocationContext, LocationKind,
    VirtualPlace,
};
use ox_core::places::Place;
use ox_core::search::IndexRoot;
use ox_core::settings::Bookmark;

use super::IndexCommand;
use crate::volumes::VolumeRow;

/// The root of the local file system (`add('file:///','Local Disk')`).
const LOCAL_DISK_URI: &str = "file:///";

/// Why a folder is suggested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SuggestionReason {
    /// The folder shown before Settings opened.
    OpenFolder,
    /// A folder the index kept before and no longer does.
    IndexedBefore,
    /// The home folder, which holds the standard folders.
    Home,
    /// A standard folder in Quick access, such as Documents.
    StandardFolder,
    /// A folder the user pinned to Quick access.
    Pinned,
    /// A saved network share.
    SavedShare,
    /// The whole local disk.
    LocalDisk,
    /// A mounted drive.
    MountedDrive,
}

impl SuggestionReason {
    /// The "Suggested because" column's text.
    pub(crate) const fn text(self) -> &'static str {
        match self {
            SuggestionReason::OpenFolder => "Folder you came from",
            SuggestionReason::IndexedBefore => "Indexed before",
            SuggestionReason::Home => "Contains the others",
            SuggestionReason::StandardFolder => "Standard folder",
            SuggestionReason::Pinned => "Pinned",
            SuggestionReason::SavedShare => "Saved share",
            SuggestionReason::LocalDisk => "Whole disk",
            SuggestionReason::MountedDrive => "Mounted drive",
        }
    }
}

/// Where the candidate folders come from.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CandidateSources<'a> {
    /// The folder shown before Settings opened (`state.settingsOrigin`).
    pub origin: Option<&'a str>,
    /// The folders the index knows, enabled or not.
    pub roots: &'a [IndexRoot],
    /// Quick access: standard folders and pins.
    pub quick_access: &'a [Place],
    /// The saved network shares.
    pub shares: &'a [Bookmark],
    /// The drives and devices the volume monitor reports.
    pub volumes: &'a [VolumeRow],
    /// Names the folders and says where they are.
    pub locations: &'a LocationContext,
}

/// A folder the search index can take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexCandidate {
    /// The folder's canonical URI.
    pub uri: String,
    /// Its name in the list.
    pub label: String,
    /// Where it is: a path, or `\\server\share` for SMB.
    pub path: String,
    /// Whether it is on an SMB share, reached as `smb://` or inside a
    /// mounted CIFS share, which shows the network bar.
    pub is_network: bool,
    /// The "Location" column: "Local disk", "Network drive" or
    /// "SMB · server".
    pub location: String,
    /// Why it is suggested.
    pub reason: SuggestionReason,
}

impl IndexCandidate {
    /// The command that indexes it under its label.
    pub(crate) fn index_command(&self) -> IndexCommand {
        IndexCommand::Index {
            uri: self.uri.clone(),
            label: self.label.clone(),
        }
    }
}

/// The candidates of `renderSettingsCache`, in its order and once each.
pub(crate) fn index_candidates(sources: &CandidateSources<'_>) -> Vec<IndexCandidate> {
    let home = sources.locations.home_uri();
    let mut offers: Vec<Offer<'_>> = Vec::new();
    offers.extend(sources.origin.map(Offer::open_folder));
    offers.extend(sources.roots.iter().map(Offer::indexed_before));
    offers.push(Offer::labelled(&home, "Home", SuggestionReason::Home));
    offers.extend(sources.quick_access.iter().map(Offer::quick_access));
    offers.extend(sources.shares.iter().map(Offer::saved_share));
    offers.push(Offer::labelled(
        LOCAL_DISK_URI,
        "Local Disk",
        SuggestionReason::LocalDisk,
    ));
    offers.extend(sources.volumes.iter().filter_map(Offer::mounted));
    let mut candidates: Vec<IndexCandidate> = Vec::new();
    for offer in offers {
        let is_listed = candidates
            .iter()
            .any(|candidate| same_location(&candidate.uri, offer.uri));
        if !is_listed && can_be_indexed(offer.uri) {
            candidates.push(offer.into_candidate(sources.locations));
        }
    }
    candidates
}

/// `root` as a candidate, named and placed by `locations`.
pub(super) fn indexed_root(root: &IndexRoot, locations: &LocationContext) -> IndexCandidate {
    Offer::indexed_before(root).into_candidate(locations)
}

/// A folder offered to the list, which keeps it unless it is listed
/// already or cannot be indexed.
#[derive(Debug, Clone, Copy)]
struct Offer<'a> {
    uri: &'a str,
    /// Its name, or `None` to call it by its title (`titleFor`).
    label: Option<&'a str>,
    reason: SuggestionReason,
}

impl<'a> Offer<'a> {
    fn labelled(uri: &'a str, label: &'a str, reason: SuggestionReason) -> Self {
        Self {
            uri,
            label: Some(label),
            reason,
        }
    }

    fn open_folder(uri: &'a str) -> Self {
        Self {
            uri,
            label: None,
            reason: SuggestionReason::OpenFolder,
        }
    }

    fn indexed_before(root: &'a IndexRoot) -> Self {
        Self::labelled(&root.uri, &root.label, SuggestionReason::IndexedBefore)
    }

    fn quick_access(place: &'a Place) -> Self {
        let reason = if place.known_folder.is_some() {
            SuggestionReason::StandardFolder
        } else {
            SuggestionReason::Pinned
        };
        Self::labelled(&place.uri, &place.label, reason)
    }

    fn saved_share(share: &'a Bookmark) -> Self {
        Self::labelled(&share.uri, &share.label, SuggestionReason::SavedShare)
    }

    /// A mounted drive; `None` for one that still has to be mounted.
    fn mounted(volume: &'a VolumeRow) -> Option<Self> {
        let uri = volume.uri()?;
        Some(Self::labelled(uri, &volume.label, SuggestionReason::MountedDrive))
    }

    fn into_candidate(self, locations: &LocationContext) -> IndexCandidate {
        let label = match self.label {
            Some(label) if !label.is_empty() => label.to_owned(),
            _ => locations.title_for(self.uri),
        };
        let is_network = locations.is_network_location(self.uri);
        IndexCandidate {
            uri: self.uri.to_owned(),
            label,
            path: locations.display_location(self.uri),
            is_network,
            location: location_text(self.uri, is_network),
            reason: self.reason,
        }
    }
}

/// The "Location" column for `uri`.
fn location_text(uri: &str, is_network: bool) -> String {
    let parts = split_location(uri).ok();
    let share = parts.filter(|parts| parts.kind() == LocationKind::Smb);
    match share {
        Some(parts) => ox_core::i18n::format_message(
            "SMB · {authority}",
            &[("authority", &(parts.authority).to_string())],
        ),
        None if is_network => "Network drive".to_owned(),
        None => "Local disk".to_owned(),
    }
}

/// Whether `uri` is a folder the index can take: not an app page, a GIO
/// virtual folder, a device or an SMB server.
fn can_be_indexed(uri: &str) -> bool {
    let is_place = VirtualPlace::from_uri(uri).is_some();
    !uri.is_empty() && !is_place && !is_device_location(uri) && !is_smb_server(uri)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ox_core::places::KnownFolder;

    use super::*;
    use crate::search::test_roots::enabled_root;
    use crate::volumes::{MountControls, VolumeKind, VolumeState};

    fn locations() -> LocationContext {
        LocationContext {
            home: Some(PathBuf::from("/home/demo")),
            ..LocationContext::default()
        }
    }

    fn pin(label: &str, uri: &str) -> Place {
        Place {
            label: label.to_owned(),
            uri: uri.to_owned(),
            known_folder: None,
            is_shared: false,
        }
    }

    fn mounted(label: &str, uri: &str, kind: VolumeKind) -> VolumeRow {
        VolumeRow {
            label: label.to_owned(),
            kind,
            state: VolumeState::Mounted {
                uri: uri.to_owned(),
                controls: MountControls::UNMOUNTABLE,
            },
        }
    }

    fn share(label: &str, uri: &str) -> Bookmark {
        Bookmark {
            uri: uri.to_owned(),
            label: label.to_owned(),
        }
    }

    /// Only `origin` and `quick_access` as sources, with no roots.
    fn sources<'a>(
        origin: Option<&'a str>,
        quick_access: &'a [Place],
        shares: &'a [Bookmark],
        locations: &'a LocationContext,
    ) -> CandidateSources<'a> {
        CandidateSources {
            origin,
            roots: &[],
            quick_access,
            shares,
            volumes: &[],
            locations,
        }
    }

    fn labels(candidates: &[IndexCandidate]) -> Vec<&str> {
        candidates
            .iter()
            .map(|candidate| candidate.label.as_str())
            .collect()
    }

    /// Ported from the candidate rules of `renderSettingsCache` in
    /// `v2.0.0:desktop/ui/app.js`.
    ///
    /// parity: SET-006
    #[test]
    fn candidates_are_the_origin_home_pins_shares_disk_and_drives_once_each() {
        let documents = Place {
            known_folder: Some(KnownFolder::Documents),
            ..pin("Documents", "file:///home/demo/Documents")
        };
        let quick_access = [documents, pin("Home again", "file:///home/demo/")];
        let shares = [
            share("Media (M:)", "smb://nas/media"),
            share("The server", "smb://nas/"),
        ];
        let volumes = [
            mounted("Backup", "file:///media/demo/Backup", VolumeKind::Drive),
            mounted("Pixel 7", "mtp://%5Busb%3A001%2C010%5D/", VolumeKind::Device),
        ];
        let locations = locations();
        let sources = CandidateSources {
            volumes: &volumes,
            ..sources(
                Some("file:///home/demo/Projects"),
                &quick_access,
                &shares,
                &locations,
            )
        };

        let candidates = index_candidates(&sources);

        assert_eq!(
            labels(&candidates),
            [
                "Projects",
                "Home",
                "Documents",
                "Media (M:)",
                "Local Disk",
                "Backup"
            ],
            "the server, the phone and a second spelling of Home are left out"
        );
        assert_eq!(candidates[3].path, "\\\\nas\\media");
        assert_eq!(candidates[4].path, "/");
    }

    /// The folders indexed before follow the origin, as `renderSettingsCache`
    /// lists `state.cache.roots` after it.
    ///
    /// parity: SET-006
    #[test]
    fn folders_indexed_before_follow_the_folder_shown_before() {
        let locations = locations();
        let mut root = enabled_root("file:///home/demo/Archive");
        root.label = "Archive".to_owned();
        let roots = [root];
        let sources = CandidateSources {
            roots: &roots,
            ..sources(Some("file:///home/demo/Projects"), &[], &[], &locations)
        };

        let candidates = index_candidates(&sources);

        assert_eq!(labels(&candidates)[..3], ["Projects", "Archive", "Home"]);
        assert_eq!(candidates[1].reason, SuggestionReason::IndexedBefore);
    }

    #[test]
    fn pages_and_virtual_folders_are_never_indexed() {
        let locations = locations();
        let sources = sources(Some(VirtualPlace::ThisPc.uri()), &[], &[], &locations);

        let candidates = index_candidates(&sources);

        assert_eq!(labels(&candidates), ["Home", "Local Disk"]);
        assert!(!can_be_indexed("trash:///"));
        assert!(!can_be_indexed("network:///"));
    }

    /// A share shows the network bar however it is reached, as `smb://` or
    /// inside a mounted CIFS share, as `networkLocation` in app.js decides.
    #[test]
    fn folders_on_shares_are_marked_as_network_folders() {
        let locations = LocationContext {
            network_mounts: vec![PathBuf::from("/mnt/media")],
            ..locations()
        };
        let quick_access = [pin("Films", "file:///mnt/media/Films")];
        let shares = [share("Projects", "smb://nas/projects")];
        let sources = sources(None, &quick_access, &shares, &locations);

        let candidates = index_candidates(&sources);

        let network: Vec<(&str, &str)> = candidates
            .iter()
            .filter(|candidate| candidate.is_network)
            .map(|candidate| (candidate.label.as_str(), candidate.location.as_str()))
            .collect();
        assert_eq!(
            network,
            [("Films", "Network drive"), ("Projects", "SMB · nas")],
            "Home and the Local Disk are local"
        );
    }

    #[test]
    fn each_candidate_says_why_it_is_suggested() {
        let documents = Place {
            known_folder: Some(KnownFolder::Documents),
            ..pin("Documents", "file:///home/demo/Documents")
        };
        let quick_access = [documents, pin("Clients", "file:///home/demo/Clients")];
        let locations = locations();
        let sources = sources(None, &quick_access, &[], &locations);

        let candidates = index_candidates(&sources);

        let reasons: Vec<&str> = candidates
            .iter()
            .map(|candidate| candidate.reason.text())
            .collect();
        assert_eq!(
            reasons,
            ["Contains the others", "Standard folder", "Pinned", "Whole disk"]
        );
    }
}
