// SPDX-License-Identifier: AGPL-3.0-only
//! Locations as the window presents them.
//!
//! Titles, the address text, breadcrumbs and the Up target come from
//! ox-core's [`LocationContext`], the port of `titleFor`, `displayUri`,
//! `breadcrumbSegments` and `deviceMountName` in `desktop/ui/app.js`, which
//! the app's JavaScript fixtures check. This module adds only what the
//! window draws itself:
//!
//! - the landing pages ([`Page`]) with their subtitle and glyph, a subset
//!   of ox-core's [`VirtualPlace`]s;
//! - the context for one window, built from the volume monitor's rows
//!   ([`location_context`]), so a phone is called by its mount name.

use std::path::PathBuf;

use ox_core::location::{DeviceLabel, LocationContext, VirtualPlace};

use crate::icons::Icon;
use crate::volumes::{VolumeKind, VolumeRow};

/// A place the window draws itself instead of a folder listing.
///
/// Only these of ox-core's [`VirtualPlace`]s have a page yet; the Recycle
/// Bin and Recent arrive with their milestones. The legacy Home page is
/// not one of them: as in the Python app, `home:` opens the home folder.
/// This PC and Network are landing pages in the folder pane; Settings
/// takes the place of the whole browsing area, as `.settings-open` does in
/// `desktop/ui/style.css`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    /// Quick access, devices and drives, and saved network locations.
    ThisPc,
    /// Connected and saved network locations.
    Network,
    /// The Settings page.
    Settings,
}

impl Page {
    /// Every page, in sidebar order, then Settings.
    pub(crate) const ALL: [Page; 3] = [Page::ThisPc, Page::Network, Page::Settings];

    /// The ox-core place the page draws.
    const fn place(self) -> VirtualPlace {
        match self {
            Page::ThisPc => VirtualPlace::ThisPc,
            Page::Network => VirtualPlace::Network,
            Page::Settings => VirtualPlace::Settings,
        }
    }

    /// The page that draws `place`, or `None` for a place without a page.
    fn from_place(place: VirtualPlace) -> Option<Page> {
        Self::ALL.into_iter().find(|page| page.place() == place)
    }

    /// The canonical URI that identifies the page in tab history.
    pub(crate) fn uri(self) -> &'static str {
        self.place().uri()
    }

    /// The page for a URI, including the web UI's spellings (`pc:`,
    /// `network:`).
    pub(crate) fn from_uri(uri: &str) -> Option<Page> {
        VirtualPlace::from_uri(uri).and_then(Self::from_place)
    }

    /// The page whose title was typed into the address bar ("this pc").
    ///
    /// Settings is never typed by title: the address bar is hidden on it,
    /// and a folder called Settings must still open when its name is typed.
    pub(crate) fn from_title(text: &str) -> Option<Page> {
        let page = VirtualPlace::from_title(text).and_then(Self::from_place);
        page.filter(|page| *page != Page::Settings)
    }

    /// Heading, tab title and breadcrumb label.
    pub(crate) fn title(self) -> &'static str {
        self.place().title()
    }

    /// The line under the heading (`renderLanding` and `renderNetwork`).
    pub(crate) const fn subtitle(self) -> &'static str {
        match self {
            Page::ThisPc => "Folders, devices, and connected storage.",
            Page::Network => "Find shared storage on your local network, or enter an address.",
            Page::Settings => "Your explorer, your way.",
        }
    }

    /// Glyph for the sidebar, the address bar and the tab: a laptop for
    /// This PC, connected nodes for Network and the gear for Settings.
    pub(crate) const fn icon(self) -> Icon {
        match self {
            Page::ThisPc => Icon::Laptop,
            Page::Network => Icon::Organization,
            Page::Settings => Icon::Settings,
        }
    }
}

/// True for the home folder's legacy page spellings (`home:` and
/// `ox:home`), which open the home folder, as `realLocation` in app.js.
pub(crate) fn is_home_alias(text: &str) -> bool {
    VirtualPlace::from_uri(text.trim()) == Some(VirtualPlace::Home)
}

/// The display context of a window: its home folder and the devices that
/// are mounted now. Only mounted devices count, as in `deviceMountName`.
pub(crate) fn location_context(home: PathBuf, volumes: &[VolumeRow]) -> LocationContext {
    LocationContext {
        home: Some(home),
        devices: volumes.iter().filter_map(device_label).collect(),
        ..LocationContext::default()
    }
}

/// The label of a mounted phone, camera or iOS device; `None` for drives
/// and for devices that still have to be mounted.
fn device_label(row: &VolumeRow) -> Option<DeviceLabel> {
    if row.kind != VolumeKind::Device {
        return None;
    }
    let uri = row.uri()?.to_owned();
    let label = row.label.clone();
    Some(DeviceLabel { uri, label })
}

#[cfg(test)]
mod tests {
    use ox_core::location::parent_location;

    use super::*;
    use crate::volumes::{locations, MountFacts, VolumeState};

    /// The home folder of the tests' windows.
    const DEMO_HOME: &str = "/home/demo";

    const PHONE_FOLDER: &str = "mtp://[usb:001,010]/Internal%20storage/DCIM";

    /// The context of a window with no devices mounted.
    fn demo_context() -> LocationContext {
        location_context(PathBuf::from(DEMO_HOME), &[])
    }

    /// The context of a window with a Pixel 7 mounted.
    fn phone_context() -> LocationContext {
        let mount = MountFacts {
            name: "Pixel 7".into(),
            root_uri: "mtp://[usb:001,010]/".into(),
            shadowed: false,
            can_unmount: true,
        };
        location_context(PathBuf::from(DEMO_HOME), &locations(&[mount], &[]))
    }

    fn labels(context: &LocationContext, uri: &str) -> Vec<String> {
        context
            .breadcrumbs(uri)
            .into_iter()
            .map(|crumb| crumb.label)
            .collect()
    }

    #[test]
    fn pages_round_trip_through_their_uris() {
        for page in Page::ALL {
            assert_eq!(Page::from_uri(page.uri()), Some(page));
        }
        assert_eq!(Page::from_uri("pc:"), Some(Page::ThisPc));
        assert_eq!(Page::from_uri("file:///"), None);
        assert_eq!(Page::from_uri("trash:///"), None);
    }

    #[test]
    fn landing_pages_can_be_typed_by_title_but_settings_cannot() {
        assert_eq!(Page::from_title(" this pc "), Some(Page::ThisPc));
        assert_eq!(Page::from_title("Network"), Some(Page::Network));
        assert_eq!(Page::from_uri("network:"), Some(Page::Network));
        assert_eq!(
            Page::from_title("Settings"),
            None,
            "a folder may be called Settings"
        );
        assert_eq!(Page::from_uri("settings:"), Some(Page::Settings));
        assert_eq!(Page::from_title("/tmp"), None);
    }

    /// parity: TAB-010
    #[test]
    fn titles_follow_app_js() {
        let context = demo_context();
        assert_eq!(context.title_for("pc:"), "This PC");
        assert_eq!(context.title_for("file:///home/demo"), "Home");
        assert_eq!(context.title_for("file:///home/demo/"), "Home");
        assert_eq!(context.title_for("file:///srv/Brand%20assets"), "Brand assets");
        assert_eq!(context.display_location("pc:"), "This PC");
    }

    #[test]
    fn the_legacy_home_page_means_the_home_folder() {
        assert!(is_home_alias("home:"));
        assert!(is_home_alias("ox:home"));
        assert!(!is_home_alias("pc:"));
    }

    /// parity: NAV-017, TAB-010
    #[test]
    fn a_mounted_phone_is_called_by_its_mount_name() {
        let context = phone_context();
        assert_eq!(context.title_for("mtp://[usb:001,010]/"), "Pixel 7");
        assert_eq!(
            labels(&context, PHONE_FOLDER),
            ["Pixel 7", "Internal storage", "DCIM"]
        );
        assert_eq!(
            context.display_location(PHONE_FOLDER),
            "Pixel 7 / Internal storage/DCIM"
        );
    }

    /// parity: NAV-017
    #[test]
    fn an_unknown_device_is_a_connected_device() {
        let context = demo_context();
        assert_eq!(context.title_for("mtp://[usb:001,010]/"), "Connected device");
        assert_eq!(labels(&context, PHONE_FOLDER)[0], "Connected device");
    }

    #[test]
    fn drives_and_unmounted_devices_are_not_device_labels() {
        let unmounted_phone = VolumeRow {
            label: "Phone".into(),
            kind: VolumeKind::Device,
            state: VolumeState::Mountable {
                id: "mtp://[usb:001,011]/".into(),
            },
        };
        let disk = VolumeRow {
            label: "Disk".into(),
            kind: VolumeKind::Drive,
            state: VolumeState::Mounted {
                uri: "file:///media/u/Disk".into(),
                can_unmount: true,
            },
        };
        let context = location_context(PathBuf::from(DEMO_HOME), &[unmounted_phone, disk]);
        assert!(context.devices.is_empty());
    }

    /// parity: TAB-010
    #[test]
    fn a_home_folder_with_reserved_characters_is_titled_home() {
        let context = location_context(PathBuf::from("/home/o'brien (x)"), &[]);
        assert_eq!(context.title_for("file:///home/o%27brien%20%28x%29"), "Home");
    }

    /// parity: NAV-017
    #[test]
    fn smb_roots_are_labelled_with_the_server() {
        let context = demo_context();
        assert_eq!(context.title_for("smb://nas/"), "nas");
        let crumbs = labels(&context, "smb://studio-nas/projects/Design");
        assert_eq!(crumbs.first().map(String::as_str), Some("studio-nas"));
        assert_eq!(crumbs.last().map(String::as_str), Some("Design"));
    }

    /// parity: NAV-010
    #[test]
    fn pages_have_one_crumb_and_no_parent() {
        let context = demo_context();
        assert_eq!(context.breadcrumbs(Page::Network.uri()).len(), 1);
        assert_eq!(context.display_location(Page::ThisPc.uri()), "This PC");
        assert_eq!(parent_location(Page::ThisPc.uri()), None);
    }
}
