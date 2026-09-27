// SPDX-License-Identifier: AGPL-3.0-only
//! What the window shows for a location: titles, the address bar text,
//! breadcrumbs, the Up target and whether new items can be created there.
//!
//! Ports `baseName`, `parentUri`, `displayUri`, `titleFor`, `deviceParts`,
//! `deviceRoot`, `deviceMountName`, `breadcrumbSegments`, `sameLocation`,
//! `isSmbShareRoot`, `readonlyLocation`, `writableLocation` and
//! `networkLocation` from `desktop/ui/app.js`, extended with the virtual
//! places in [`VirtualPlace`].
//!
//! The web UI read the home folder, mounted devices, snapshot roots and
//! network mounts from its `state.env`; here they live in a
//! [`LocationContext`]. The free functions use an empty context: devices
//! are then called "Connected device".

use std::path::PathBuf;

use super::normalise::{file_uri, is_smb_server};
use super::parts::{split_location, url_scheme, DeviceMatch, LocationParts};
use super::text::{decode_uri_component, strip_one_trailing_slash};
use super::virtual_place::{VirtualFolder, VirtualPlace};
use super::{Crumb, DEVICE_SCHEMES};

/// Name of a device whose mount is not known.
const UNKNOWN_DEVICE: &str = "Connected device";

/// Name of the local root folder.
const LOCAL_DISK: &str = "Local Disk";

/// Path components that mark a read-only snapshot (Btrfs/NAS snapshots
/// and Windows "Previous versions" over SMB).
const SNAPSHOT_DIRECTORIES: [&str; 3] = [".snapshot", ".snapshots", "#snapshot"];

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
    /// mounts whose file system type passes [`is_network_filesystem`].
    pub network_mounts: Vec<PathBuf>,
}

/// True for the mount types whose folders count as network folders:
/// `cifs` and `smb3`. An empty type counts as `cifs`, as in the web UI's
/// `networkLocation`.
pub fn is_network_filesystem(fstype: &str) -> bool {
    matches!(fstype, "" | "cifs" | "smb3")
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
    pub fn device_name(&self, uri: &str) -> String {
        let root = device_root(uri);
        let device = self
            .devices
            .iter()
            .find(|device| root.is_some() && device_root(&device.uri) == root);
        match device {
            Some(device) if !device.label.is_empty() => device.label.clone(),
            _ => UNKNOWN_DEVICE.to_string(),
        }
    }

    /// The last path component for tab labels and dialogs: the device or
    /// server name at a root, "Local Disk" for `/`, and the place title for
    /// a virtual place. Unparseable input is returned unchanged.
    pub fn base_name(&self, uri: &str) -> String {
        if let Some(place) = VirtualPlace::from_uri(uri) {
            return place.title().to_string();
        }
        if let Some(Ok(folder)) = VirtualFolder::parse(uri) {
            return folder.segments.last().cloned().unwrap_or_default();
        }
        let Some(parts) = location_parts(uri) else {
            return uri.to_string();
        };
        let last = parts.path.split('/').rfind(|part| !part.is_empty());
        let Some(name) = decode_uri_component(last.unwrap_or_default()) else {
            return uri.to_string();
        };
        if !name.is_empty() {
            name
        } else if is_device(&parts) {
            self.device_name(uri)
        } else if !parts.netloc.is_empty() {
            parts.netloc
        } else {
            LOCAL_DISK.to_string()
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
        if let Some(Ok(folder)) = VirtualFolder::parse(uri) {
            return with_subpath(folder.place.title(), &folder.segments.join("/"));
        }
        let Some(parts) = location_parts(uri) else {
            return uri.to_string();
        };
        let Some(path) = decode_uri_component(&parts.path) else {
            return uri.to_string();
        };
        match parts.scheme.as_str() {
            "smb" => format!("\\\\{}{}", parts.netloc, path.replace('/', "\\")),
            "file" => path,
            _ if is_device(&parts) => with_subpath(&self.device_name(uri), path.trim_matches('/')),
            _ => uri.to_string(),
        }
    }

    /// Breadcrumb buttons from the root to `uri`. Local folders start at
    /// `/`, SMB at the server, devices at the device name and virtual
    /// folders at their title; the app's pages are a single crumb.
    pub fn breadcrumbs(&self, uri: &str) -> Vec<Crumb> {
        let single = || vec![Crumb::new(uri, uri)];
        if let Some(place) = VirtualPlace::from_uri(uri) {
            return vec![Crumb::new(place.title(), place.uri())];
        }
        if let Some(Ok(folder)) = VirtualFolder::parse(uri) {
            return virtual_crumbs(&folder);
        }
        let Some(parts) = location_parts(uri) else {
            return single();
        };
        let (first, mut prefix) = match parts.scheme.as_str() {
            "smb" => {
                let prefix = format!("smb://{}", parts.netloc);
                (Crumb::new(&parts.netloc, format!("{prefix}/")), prefix)
            }
            "file" => (Crumb::new("/", "file:///"), "file://".to_string()),
            _ if is_device(&parts) => {
                let prefix = format!("{}://{}", parts.scheme, parts.netloc);
                (Crumb::new(self.device_name(uri), format!("{prefix}/")), prefix)
            }
            _ => return single(),
        };
        let mut crumbs = vec![first];
        for part in parts.path.split('/').filter(|part| !part.is_empty()) {
            let Some(label) = decode_uri_component(part) else {
                return single();
            };
            prefix = format!("{prefix}/{part}");
            crumbs.push(Crumb::new(label, prefix.clone()));
        }
        crumbs
    }

    /// True inside a read-only snapshot: a `.snapshot`, `.snapshots`,
    /// `#snapshot`, `@GMT-…` or `.zfs/snapshot` component, or a configured
    /// snapshot root. The web UI's `readonlyLocation`.
    pub fn is_snapshot_location(&self, uri: &str) -> bool {
        if uri.is_empty() {
            return false;
        }
        let decoded = location_parts(uri)
            .and_then(|parts| decode_uri_component(&parts.path))
            .unwrap_or_default();
        let components: Vec<&str> = decoded.split('/').collect();
        let marked = components
            .iter()
            .any(|part| SNAPSHOT_DIRECTORIES.contains(part) || part.starts_with("@GMT-"));
        let zfs = components.windows(2).any(|pair| pair == [".zfs", "snapshot"]);
        let under_root = self.snapshot_roots.iter().any(|root| {
            let prefix = format!("{}/", strip_one_trailing_slash(root));
            same_location(uri, root) || uri.starts_with(&prefix)
        });
        marked || zfs || under_root
    }

    /// True where New and Paste may create items: a real folder that is not
    /// an SMB server listing, a virtual place or a snapshot.
    pub fn writable_location(&self, uri: &str) -> bool {
        !uri.is_empty()
            && VirtualPlace::from_uri(uri).is_none()
            && VirtualFolder::parse(uri).is_none()
            && !is_smb_server(uri)
            && !self.is_snapshot_location(uri)
    }

    /// True for SMB locations and for local folders inside a mounted
    /// CIFS/SMB3 share; such folders get the network icon.
    pub fn network_location(&self, uri: &str) -> bool {
        if uri.starts_with("smb:") {
            return true;
        }
        if !uri.starts_with("file:") {
            return false;
        }
        let Some(decoded) = location_parts(uri).and_then(|parts| decode_uri_component(&parts.path)) else {
            return false;
        };
        let path = match strip_one_trailing_slash(&decoded) {
            "" => "/",
            path => path,
        };
        self.network_mounts
            .iter()
            .any(|mount| is_same_or_below(path, &mount.to_string_lossy()))
    }
}

/// [`LocationContext::base_name`] without device names or a home folder.
pub fn base_name(uri: &str) -> String {
    LocationContext::default().base_name(uri)
}

/// [`LocationContext::title_for`] with the real home folder.
pub fn title_for(uri: &str) -> String {
    LocationContext::default().title_for(uri)
}

/// [`LocationContext::display_location`] without device names.
pub fn display_location(uri: &str) -> String {
    LocationContext::default().display_location(uri)
}

/// [`LocationContext::breadcrumbs`] without device names.
pub fn breadcrumbs(uri: &str) -> Vec<Crumb> {
    LocationContext::default().breadcrumbs(uri)
}

/// The separator the address bar draws before crumb `index` of `uri`:
/// none before the first crumb or right after the `/` root, `\` on SMB and
/// `/` elsewhere.
pub fn crumb_divider(uri: &str, crumbs: &[Crumb], index: usize) -> Option<&'static str> {
    let after_local_root = index == 1 && crumbs.first().is_some_and(|crumb| crumb.label == "/");
    if index == 0 || after_local_root {
        None
    } else if uri.starts_with("smb:") {
        Some("\\")
    } else {
        Some("/")
    }
}

/// The folder Up goes to, or `None` at a root, a server listing, a device
/// root, a virtual place root or an app page. The result is canonical: no
/// trailing slash except at a root.
pub fn parent_location(uri: &str) -> Option<String> {
    if VirtualPlace::from_uri(uri).is_some() {
        return None;
    }
    if let Some(folder) = VirtualFolder::parse(uri) {
        let mut folder = folder.ok()?;
        folder.segments.pop()?;
        return Some(folder.uri());
    }
    let parts = location_parts(uri)?;
    let path = strip_one_trailing_slash(&parts.path);
    if path.is_empty() {
        return None;
    }
    let parent = match path.rfind('/') {
        Some(0) | None => "/",
        Some(slash) => &path[..slash],
    };
    Some(format!("{}://{}{parent}", parts.scheme, parts.netloc))
}

/// True when two URIs differ at most by one trailing slash.
pub fn same_location(a: &str, b: &str) -> bool {
    strip_one_trailing_slash(a) == strip_one_trailing_slash(b)
}

/// True for a whole SMB server or share (`smb://nas/` or `smb://nas/share`),
/// which cannot be renamed, moved or trashed.
pub fn is_smb_share_root(uri: &str) -> bool {
    location_parts(uri).is_some_and(|parts| {
        let segment_count = parts.path.split('/').filter(|part| !part.is_empty()).count();
        parts.scheme == "smb" && segment_count <= 1
    })
}

/// The root of the device `uri` is on (`mtp://[usb:001,010]/`), or `None`
/// for anything but `mtp:`, `gphoto2:` and `afc:` locations.
pub fn device_root(uri: &str) -> Option<String> {
    let device = DeviceMatch::parse(uri)?;
    let scheme = device.scheme.to_ascii_lowercase();
    DEVICE_SCHEMES
        .contains(&scheme.as_str())
        .then(|| format!("{scheme}://{}/", device.authority))
}

/// Splits a `scheme://` location for display; `None` for plain paths,
/// authority-less URIs and malformed input, which are shown unchanged.
fn location_parts(uri: &str) -> Option<LocationParts> {
    let (_, rest) = url_scheme(uri)?;
    if !rest.starts_with("//") {
        return None;
    }
    split_location(uri).ok()
}

fn is_device(parts: &LocationParts) -> bool {
    DEVICE_SCHEMES.contains(&parts.scheme.as_str())
}

/// `Pixel 7 / DCIM/Camera`, or just the name when `subpath` is empty.
fn with_subpath(name: &str, subpath: &str) -> String {
    if subpath.is_empty() {
        name.to_string()
    } else {
        format!("{name} / {subpath}")
    }
}

fn virtual_crumbs(folder: &VirtualFolder) -> Vec<Crumb> {
    let mut crumbs = vec![Crumb::new(folder.place.title(), folder.place.uri())];
    let mut current = VirtualFolder {
        place: folder.place,
        segments: Vec::new(),
    };
    for segment in &folder.segments {
        current.segments.push(segment.clone());
        crumbs.push(Crumb::new(segment, current.uri()));
    }
    crumbs
}

/// `path` equals `root` or lies below it.
fn is_same_or_below(path: &str, root: &str) -> bool {
    if root.is_empty() {
        return false;
    }
    let prefix = format!("{}/", strip_one_trailing_slash(root));
    path == root || path.starts_with(&prefix)
}
