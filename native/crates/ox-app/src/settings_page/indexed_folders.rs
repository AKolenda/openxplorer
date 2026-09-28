// SPDX-License-Identifier: AGPL-3.0-only
//! The Indexed folders page: the folders the search index can take, and a
//! field to add another.
//!
//! Ports `renderSettingsCache` and the "Add a folder" field of
//! `renderSettingsPage` in `desktop/ui/app.js` (SET-006, SET-007). The
//! Python app listed these inline, in a long list that filled the page;
//! here they open from the "Folders to index" row. Indexing is not in the
//! native preview yet, so each folder shows "Not cached" and its switch
//! waits for cached search, but the list offers the same folders, once
//! each: the folder shown before Settings opened, Home, Quick access,
//! saved shares, the Local Disk and mounted drives, never pages, devices
//! or SMB servers.

use std::cell::RefCell;

use gtk::prelude::*;
use ox_core::location::{is_device_location, is_smb_server, same_location, LocationContext, VirtualPlace};
use ox_core::places::Place;
use ox_core::settings::Bookmark;

use super::category_page::{CategoryPage, PageKind};
use super::group::SettingsGroup;
use super::parts;
use super::row::{Availability, ControlName, RowLayout, SettingRow};
use super::search::RowText;
use crate::icons::{Art, ArtImage};
use crate::volumes::VolumeRow;
use crate::window::{ButtonStyle, Milestone};

/// The root of the local file system (`add('file:///','Local Disk')`).
const LOCAL_DISK_URI: &str = "file:///";

/// A folder's picture in the list (`folderIcon(24)` in app.js).
const FOLDER_ART: i32 = 28;

/// The page's line under its title: the Python section's help text.
const LEAD: &str = "Check a folder to index the names and paths of its files and subfolders. SMB \
                    folders work too. File contents are never cached. Open a protected share and \
                    sign in before indexing it.";

const ADD_FOLDER: RowText = RowText {
    title: "Add a folder",
    description: "A path relative to the folder you came from, or a share such as \\\\nas\\share.",
    keywords: "",
};

/// Where the candidate folders come from.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CandidateSources<'a> {
    /// The folder shown before Settings opened (`state.settingsOrigin`).
    pub origin: Option<&'a str>,
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
}

impl IndexCandidate {
    /// Whether the folder is on an SMB share, which shows the network bar.
    fn is_network(&self) -> bool {
        self.uri.starts_with("smb:")
    }
}

/// The candidates of `renderSettingsCache`, in its order and once each.
pub(crate) fn index_candidates(sources: &CandidateSources<'_>) -> Vec<IndexCandidate> {
    let home = sources.locations.home_uri();
    let mut offers: Vec<Offer<'_>> = Vec::new();
    offers.extend(sources.origin.map(Offer::untitled));
    offers.push(Offer::labelled(&home, "Home"));
    let pins = sources.quick_access.iter();
    offers.extend(pins.map(|place| Offer::labelled(&place.uri, &place.label)));
    let shares = sources.shares.iter();
    offers.extend(shares.map(|share| Offer::labelled(&share.uri, &share.label)));
    offers.push(Offer::labelled(LOCAL_DISK_URI, "Local Disk"));
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

/// A folder offered to the list, which keeps it unless it is listed
/// already or cannot be indexed.
#[derive(Debug, Clone, Copy)]
struct Offer<'a> {
    uri: &'a str,
    /// Its name, or `None` to call it by its title (`titleFor`).
    label: Option<&'a str>,
}

impl<'a> Offer<'a> {
    fn labelled(uri: &'a str, label: &'a str) -> Self {
        Self {
            uri,
            label: Some(label),
        }
    }

    fn untitled(uri: &'a str) -> Self {
        Self { uri, label: None }
    }

    /// A mounted drive; `None` for one that still has to be mounted.
    fn mounted(volume: &'a VolumeRow) -> Option<Self> {
        Some(Self::labelled(volume.uri()?, &volume.label))
    }

    fn into_candidate(self, locations: &LocationContext) -> IndexCandidate {
        let label = match self.label {
            Some(label) => label.to_owned(),
            None => locations.title_for(self.uri),
        };
        IndexCandidate {
            uri: self.uri.to_owned(),
            label,
            path: locations.display_location(self.uri),
        }
    }
}

/// Whether `uri` is a folder the index can take: not an app page, a GIO
/// virtual folder, a device or an SMB server.
fn can_be_indexed(uri: &str) -> bool {
    let is_place = VirtualPlace::from_uri(uri).is_some();
    !uri.is_empty() && !is_place && !is_device_location(uri) && !is_smb_server(uri)
}

/// The list of candidate folders on the page, which the window refills
/// whenever the places change.
#[derive(Debug)]
pub(super) struct FolderList {
    /// The group the folders are listed in.
    group: SettingsGroup,
    /// The folders listed now.
    shown: RefCell<Vec<IndexCandidate>>,
}

impl FolderList {
    /// Replaces the listed folders with `candidates`.
    pub(super) fn show(&self, candidates: &[IndexCandidate]) {
        self.group.remove_rows();
        for candidate in candidates {
            self.group.add_plain_row(&candidate_row(candidate));
        }
        self.shown.replace(candidates.to_vec());
    }

    /// The folders listed now.
    #[cfg(test)]
    pub(super) fn shown(&self) -> Vec<IndexCandidate> {
        self.shown.borrow().clone()
    }
}

/// The Indexed folders page and its list of folders.
pub(super) fn build() -> (CategoryPage, FolderList) {
    let indexed = CategoryPage::new("Indexed folders", LEAD, PageKind::Subpage);
    let pending = Availability::Unported(Milestone::SearchAndMetadata);
    let add_group = SettingsGroup::new("Add folders to the index");
    add_group.set_shared_availability(pending);
    add_group.add_row(&add_folder_row(pending));
    indexed.append_group(&add_group);
    let group = SettingsGroup::new("Folders");
    group.set_shared_availability(pending);
    indexed.append_group(&group);
    let list = FolderList {
        group,
        shown: RefCell::new(Vec::new()),
    };
    (indexed, list)
}

/// The path field and "Add" (SET-007), waiting for cached search.
fn add_folder_row(pending: Availability) -> SettingRow {
    let row = SettingRow::new(ADD_FOLDER);
    let field = gtk::Entry::builder()
        .placeholder_text("Add a folder: /home/you/Projects or \\\\nas\\share")
        .hexpand(true)
        .width_chars(36)
        .build();
    field.update_property(&[gtk::accessible::Property::Label("Folder to cache")]);
    row.add_control(&field, ControlName::RowTitle);
    row.add_control(
        &parts::button("Add", ButtonStyle::Bordered),
        ControlName::OwnLabel,
    );
    row.set_roomy_layout(RowLayout::ControlsBelow);
    row.set_availability(pending);
    row
}

/// A folder's row: its picture, name and path, "Not cached", and a switch
/// named "Cache <label>" as the Python checkbox is.
fn candidate_row(candidate: &IndexCandidate) -> gtk::Box {
    let row = gtk::Box::builder()
        .spacing(14)
        .css_classes(["setting-row", "folder-row"])
        .build();
    let art = if candidate.is_network() {
        Art::SHARE
    } else {
        Art::Folder
    };
    row.append(&ArtImage::new(art, FOLDER_ART));
    let texts = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    texts.append(&parts::wrapped_label(&candidate.label, "setting-title"));
    texts.append(&parts::wrapped_label(&candidate.path, "setting-description"));
    row.append(&texts);
    row.append(&parts::value_label("Not cached"));
    let switch = parts::switch();
    switch.set_sensitive(false);
    let name = format!("Cache {}", candidate.label);
    switch.update_property(&[gtk::accessible::Property::Label(&name)]);
    row.append(&switch);
    row.set_tooltip_text(Some(&Milestone::SearchAndMetadata.notice()));
    row
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ox_core::places::KnownFolder;

    use super::*;
    use crate::volumes::{VolumeKind, VolumeState};

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
                can_unmount: true,
            },
        }
    }

    fn share(label: &str, uri: &str) -> Bookmark {
        Bookmark {
            uri: uri.to_owned(),
            label: label.to_owned(),
        }
    }

    /// Ported from the candidate rules of `renderSettingsCache` in
    /// `desktop/ui/app.js`.
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
            origin: Some("file:///home/demo/Projects"),
            quick_access: &quick_access,
            shares: &shares,
            volumes: &volumes,
            locations: &locations,
        };

        let candidates = index_candidates(&sources);

        let labels: Vec<&str> = candidates
            .iter()
            .map(|candidate| candidate.label.as_str())
            .collect();
        assert_eq!(
            labels,
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

    #[test]
    fn pages_and_virtual_folders_are_never_indexed() {
        let locations = locations();
        let sources = CandidateSources {
            origin: Some(VirtualPlace::ThisPc.uri()),
            quick_access: &[],
            shares: &[],
            volumes: &[],
            locations: &locations,
        };

        let candidates = index_candidates(&sources);

        let labels: Vec<&str> = candidates
            .iter()
            .map(|candidate| candidate.label.as_str())
            .collect();
        assert_eq!(labels, ["Home", "Local Disk"]);
        assert!(!can_be_indexed("trash:///"));
        assert!(!can_be_indexed("network:///"));
    }
}
