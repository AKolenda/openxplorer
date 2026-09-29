// SPDX-License-Identifier: AGPL-3.0-only
//! What the window shows for a location: titles, the address bar text,
//! breadcrumbs and the Up target.
//!
//! Ports `baseName`, `parentUri`, `displayUri`, `titleFor`, `locationParts`,
//! `deviceParts`, `deviceRoot`, `deviceMountName`, `breadcrumbSegments` and
//! `sameLocation` from `desktop/ui/app.js`, and the crumb dividers of its
//! `renderNavigation`, extended with the virtual places in [`VirtualPlace`].
//! Whether a location is writable, a snapshot or a network folder is
//! decided in `classify.rs`.
//!
//! The web UI read the home folder, mounted devices, snapshot roots and
//! network mounts from its `state.env`; here they live in a
//! [`LocationContext`]. `LocationContext::default()` knows no devices, so
//! it calls every device "Connected device".

use std::path::PathBuf;

use super::device_uri::DeviceUriMatch;
use super::normalise::file_uri;
use super::parts::{split_location, split_scheme, LocationKind, LocationParts};
use super::text::{decode_uri_component, strip_one_trailing_slash};
use super::virtual_place::{VirtualFolder, VirtualPlace};
use super::{location_kind, Crumb};

/// Name of a device whose mount is not known.
const UNKNOWN_DEVICE: &str = "Connected device";

/// Name of the local root folder.
const LOCAL_DISK: &str = "Local Disk";

/// A mounted phone, camera or iOS device and the name to show for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceLabel {
    /// Any URI on the device; only its root (`mtp://[usb:001,010]/`) is used.
    pub uri: String,
    /// The mount's display name, for example "Pixel 7".
    pub label: String,
}

/// The session facts the display helpers need, gathered by the app from
/// the volume monitor, mount table and settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocationContext {
    /// The user's home folder; `None` means `glib::home_dir()`.
    pub home: Option<PathBuf>,
    /// Mounted portable devices. Leave out mounts that are not mounted, as
    /// the web UI's `deviceMountName` did.
    pub devices: Vec<DeviceLabel>,
    /// Canonical URIs of folders that hold read-only snapshots.
    pub snapshot_roots: Vec<String>,
    /// Local mount points of CIFS/SMB3 shares (`/mnt/nas`): the stable
    /// mounts whose file system type passes
    /// [`is_network_filesystem`](super::is_network_filesystem).
    pub network_mounts: Vec<PathBuf>,
}

impl LocationContext {
    /// The user's home folder.
    pub fn home_path(&self) -> PathBuf {
        self.home.clone().unwrap_or_else(glib::home_dir)
    }

    /// The canonical `file://` URI of the home folder.
    pub fn home_uri(&self) -> String {
        file_uri(&self.home_path())
    }

    /// The label of the mounted device `uri` is on, or "Connected device".
    pub fn device_name(&self, uri: &str) -> &str {
        let Some(root) = device_root(uri) else {
            return UNKNOWN_DEVICE;
        };
        let device = self
            .devices
            .iter()
            .find(|device| device_root(&device.uri).as_ref() == Some(&root));
        match device {
            Some(device) if !device.label.is_empty() => &device.label,
            _ => UNKNOWN_DEVICE,
        }
    }

    /// The last path component for tab labels and dialogs: the device or
    /// server name at a root, "Local Disk" for `/`, and the place title for
    /// a virtual place. Unparseable input is returned unchanged.
    pub fn base_name(&self, uri: &str) -> String {
        if let Some(place) = VirtualPlace::from_uri(uri) {
            return place.title().to_string();
        }
        if let Ok(Some(folder)) = VirtualFolder::parse(uri) {
            return folder.segments.last().cloned().unwrap_or_default();
        }
        let Some(parts) = location_parts(uri) else {
            return uri.to_string();
        };
        let last_component = parts.path.split('/').rfind(|component| !component.is_empty());
        let Some(name) = decode_uri_component(last_component.unwrap_or_default()) else {
            return uri.to_string();
        };
        if name.is_empty() {
            self.root_name(uri, parts)
        } else {
            name
        }
    }

    /// The window and tab title: "Home" for the Home page and the home
    /// folder, the place title for other virtual places, else
    /// [`base_name`](Self::base_name).
    pub fn title_for(&self, uri: &str) -> String {
        if same_location(uri, &self.home_uri()) {
            return VirtualPlace::Home.title().to_string();
        }
        self.base_name(uri)
    }

    /// Text for the editable address bar and tooltips: a plain path for
    /// local folders, `\\server\share\...` for SMB, `Device / path` for
    /// devices, `Recycle Bin / path` inside virtual folders, else the URI.
    pub fn display_location(&self, uri: &str) -> String {
        if let Some(place) = VirtualPlace::from_uri(uri) {
            return place.title().to_string();
        }
        if let Ok(Some(folder)) = VirtualFolder::parse(uri) {
            return with_subpath(folder.place.title(), &folder.segments.join("/"));
        }
        let Some(parts) = location_parts(uri) else {
            return uri.to_string();
        };
        let Some(path) = decode_uri_component(&parts.path) else {
            return uri.to_string();
        };
        match parts.kind() {
            LocationKind::Local => path,
            LocationKind::Smb => format!("\\\\{}{}", parts.authority, path.replace('/', "\\")),
            LocationKind::Device => with_subpath(self.device_name(uri), path.trim_matches('/')),
            LocationKind::Remote | LocationKind::Other => uri.to_string(),
        }
    }

    /// Breadcrumb buttons from the root to `uri`. Local folders start at
    /// `/`, SMB at the server, devices at the device name and virtual
    /// folders at their title; the app's pages are a single crumb.
    pub fn breadcrumbs(&self, uri: &str) -> Vec<Crumb> {
        if let Some(place) = VirtualPlace::from_uri(uri) {
            return vec![Crumb::new(place.title(), place.uri())];
        }
        if let Ok(Some(folder)) = VirtualFolder::parse(uri) {
            return virtual_crumbs(&folder);
        }
        self.folder_crumbs(uri)
            .unwrap_or_else(|| vec![Crumb::new(uri, uri)])
    }

    /// The name of a root folder: the device label, the server, or
    /// "Local Disk".
    fn root_name(&self, uri: &str, parts: LocationParts) -> String {
        if parts.is_device() {
            self.device_name(uri).to_string()
        } else if !parts.authority.is_empty() {
            parts.authority
        } else {
            LOCAL_DISK.to_string()
        }
    }

    /// The crumbs of a local, SMB or device folder: the root crumb, then
    /// one per path component, each opening the still-escaped URI up to
    /// that component. `None` when the address bar shows `uri` as a single
    /// crumb.
    fn folder_crumbs(&self, uri: &str) -> Option<Vec<Crumb>> {
        let parts = location_parts(uri)?;
        let root = self.root_crumb(uri, &parts)?;
        let mut crumb_uri = strip_one_trailing_slash(&root.uri).to_string();
        let mut crumbs = vec![root];
        for component in parts.path.split('/').filter(|component| !component.is_empty()) {
            let label = decode_uri_component(component)?;
            crumb_uri = format!("{crumb_uri}/{component}");
            crumbs.push(Crumb::new(label, crumb_uri.clone()));
        }
        Some(crumbs)
    }

    /// The first crumb: `/` for local folders, the server for SMB and the
    /// device name for devices.
    fn root_crumb(&self, uri: &str, parts: &LocationParts) -> Option<Crumb> {
        match parts.kind() {
            LocationKind::Local => Some(Crumb::new("/", "file:///")),
            LocationKind::Smb | LocationKind::Remote => Some(Crumb::new(&parts.authority, root_uri(parts))),
            LocationKind::Device => Some(Crumb::new(self.device_name(uri), root_uri(parts))),
            LocationKind::Other => None,
        }
    }
}

/// The separator the address bar draws before crumb `index` of
/// [`LocationContext::breadcrumbs`]: none before the first crumb or right
/// after the `/` root of a local folder, `\` on SMB and `/` elsewhere, as
/// the web UI's `renderNavigation`. The scheme decides, as app.js's
/// `startsWith('smb:')` does for the canonical URIs the window shows.
pub fn crumb_divider(uri: &str, index: usize) -> Option<&'static str> {
    match (location_kind(uri), index) {
        (_, 0) | (LocationKind::Local, 1) => None,
        (LocationKind::Smb, _) => Some("\\"),
        _ => Some("/"),
    }
}

/// The folder Up goes to, or `None` at a root, a server listing, a device
/// root, a virtual place root or an app page. The result is canonical: no
/// trailing slash except at a root.
pub fn parent_location(uri: &str) -> Option<String> {
    if VirtualPlace::from_uri(uri).is_some() {
        return None;
    }
    // A malformed address inside a virtual folder has no parent either.
    let virtual_folder = VirtualFolder::parse(uri).ok()?;
    if let Some(mut folder) = virtual_folder {
        folder.segments.pop()?;
        return Some(folder.uri());
    }
    let parts = location_parts(uri)?;
    let path = strip_one_trailing_slash(&parts.path);
    if path.is_empty() {
        return None;
    }
    let parent = match path.rsplit_once('/') {
        Some((parent, _)) if !parent.is_empty() => parent,
        _ => "/",
    };
    Some(format!("{}://{}{parent}", parts.scheme, parts.authority))
}

/// True when two URIs differ at most by one trailing slash.
pub fn same_location(a: &str, b: &str) -> bool {
    strip_one_trailing_slash(a) == strip_one_trailing_slash(b)
}

/// The root of the device `uri` is on (`mtp://[usb:001,010]/`), or `None`
/// for anything but `mtp:`, `gphoto2:` and `afc:` locations.
pub fn device_root(uri: &str) -> Option<String> {
    let device = DeviceUriMatch::parse(uri)?.to_parts();
    device.is_device().then(|| root_uri(&device))
}

/// Splits a `scheme://` location for display, like the web UI's
/// `locationParts`; `None` for plain paths, authority-less URIs and
/// malformed input, which are shown unchanged.
pub(super) fn location_parts(uri: &str) -> Option<LocationParts> {
    let (_, after_scheme) = split_scheme(uri)?;
    if !after_scheme.starts_with("//") {
        return None;
    }
    split_location(uri).ok()
}

/// `scheme://authority/`: the root of an SMB server or a device.
fn root_uri(parts: &LocationParts) -> String {
    format!("{}://{}/", parts.scheme, parts.authority)
}

/// `Pixel 7 / DCIM/Camera`, or just the name when `subpath` is empty.
fn with_subpath(name: &str, subpath: &str) -> String {
    if subpath.is_empty() {
        name.to_string()
    } else {
        format!("{name} / {subpath}")
    }
}

/// The crumbs of an item in a virtual folder: the place title, then one
/// crumb per decoded component.
fn virtual_crumbs(folder: &VirtualFolder) -> Vec<Crumb> {
    let mut crumbs = vec![Crumb::new(folder.place.title(), folder.place.uri())];
    let mut ancestor = VirtualFolder {
        place: folder.place,
        segments: Vec::new(),
    };
    for segment in &folder.segments {
        ancestor.segments.push(segment.clone());
        crumbs.push(Crumb::new(segment, ancestor.uri()));
    }
    crumbs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::{HOME_URI, NETWORK_URI, PC_URI, RECENT_URI, SETTINGS_URI, TRASH_URI};

    const PHONE_ROOT: &str = "mtp://[usb:001,010]/";

    /// A session with the home folder `/home/test` and one mounted phone.
    fn context() -> LocationContext {
        LocationContext {
            home: Some(PathBuf::from("/home/test")),
            devices: vec![DeviceLabel {
                uri: PHONE_ROOT.to_string(),
                label: "Pixel 7".to_string(),
            }],
            ..LocationContext::default()
        }
    }

    /// parity: TAB-010
    #[test]
    fn tabs_are_titled_like_explorer() {
        let context = context();
        assert_eq!(context.title_for("file:///home/test"), "Home");
        assert_eq!(context.title_for("file:///home/test/"), "Home");
        assert_eq!(context.title_for(HOME_URI), "Home");
        assert_eq!(context.title_for(PC_URI), "This PC");
        assert_eq!(context.title_for(NETWORK_URI), "Network");
        assert_eq!(context.title_for(SETTINGS_URI), "Settings");
        assert_eq!(context.title_for(TRASH_URI), "Recycle Bin");
        assert_eq!(context.title_for("file:///"), "Local Disk");
        assert_eq!(context.title_for(PHONE_ROOT), "Pixel 7");
        assert_eq!(context.title_for("file:///tmp/a%20b"), "a b");
    }

    #[test]
    fn items_in_virtual_folders_are_shown_under_the_place_title() {
        let context = context();
        let item = "trash:///Old%20plans/draft.txt";
        assert_eq!(context.base_name(item), "draft.txt");
        assert_eq!(
            context.display_location(item),
            "Recycle Bin / Old plans/draft.txt"
        );
        assert_eq!(
            context.breadcrumbs(item),
            vec![
                Crumb::new("Recycle Bin", TRASH_URI),
                Crumb::new("Old plans", "trash:///Old%20plans"),
                Crumb::new("draft.txt", item),
            ]
        );
        assert_eq!(parent_location(item).as_deref(), Some("trash:///Old%20plans"));
        assert_eq!(
            parent_location("trash:///Old%20plans").as_deref(),
            Some(TRASH_URI)
        );
    }

    /// parity: NAV-010
    #[test]
    fn up_goes_to_the_parent_folder_and_stops_at_pages_and_roots() {
        let roots = [
            HOME_URI,
            PC_URI,
            NETWORK_URI,
            SETTINGS_URI,
            TRASH_URI,
            RECENT_URI,
            "file:///",
            "smb://nas/",
            PHONE_ROOT,
        ];
        for root in roots {
            assert_eq!(parent_location(root), None, "{root}");
        }
        assert_eq!(
            parent_location("file:///home/test").as_deref(),
            Some("file:///home")
        );
        assert_eq!(
            parent_location("smb://nas/share/folder/").as_deref(),
            Some("smb://nas/share")
        );
        assert_eq!(parent_location("smb://nas/share").as_deref(), Some("smb://nas/"));
        assert_eq!(
            parent_location("mtp://[usb:001,010]/DCIM").as_deref(),
            Some(PHONE_ROOT)
        );
    }

    /// parity: NAV-017
    #[test]
    fn crumb_dividers_follow_the_address_style() {
        let local = "file:///home/test";
        let local_crumbs = context().breadcrumbs(local);
        assert_eq!(local_crumbs[0], Crumb::new("/", "file:///"));
        assert_eq!(crumb_divider(local, 0), None);
        assert_eq!(crumb_divider(local, 1), None);
        assert_eq!(crumb_divider(local, 2), Some("/"));
        let share = "smb://nas/share";
        assert_eq!(crumb_divider(share, 0), None);
        assert_eq!(crumb_divider(share, 1), Some("\\"));
        assert_eq!(crumb_divider(share, 2), Some("\\"));
        let phone = "mtp://[usb:001,010]/DCIM";
        assert_eq!(crumb_divider(phone, 1), Some("/"));
        let item = "trash:///Old%20plans/draft.txt";
        assert_eq!(crumb_divider(item, 1), Some("/"));
    }

    /// A phone that is not among the mounted devices is shown as
    /// "Connected device", never by its raw USB identifier.
    ///
    /// parity: NAV-017
    #[test]
    fn unknown_devices_are_called_connected_device() {
        let without_devices = LocationContext::default();
        assert_eq!(without_devices.device_name(PHONE_ROOT), "Connected device");
        assert_eq!(without_devices.base_name(PHONE_ROOT), "Connected device");
        assert_eq!(
            without_devices.display_location("mtp://[usb:001,010]/DCIM"),
            "Connected device / DCIM"
        );
        assert_eq!(context().device_name(PHONE_ROOT), "Pixel 7");
        assert_eq!(context().device_name("file:///"), "Connected device");
    }

    #[test]
    fn one_trailing_slash_does_not_matter() {
        assert!(same_location("smb://nas/share/", "smb://nas/share"));
        assert!(!same_location("file:///a", "file:///b"));
        assert!(same_location("file:///", "file:///"));
    }

    /// parity: NAV-010
    #[test]
    fn a_folder_has_its_parent_as_up_target() {
        assert_eq!(
            parent_location("file:///srv/data").as_deref(),
            Some("file:///srv")
        );
    }
}
