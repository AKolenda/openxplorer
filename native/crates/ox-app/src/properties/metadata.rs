// SPDX-License-Identifier: AGPL-3.0-only
//! What the Properties dialog reads about one item.
//!
//! Ports `properties` in `v2.0.0:desktop/file_services.py`: the item itself is
//! queried without following a link, with its times, owner, mode and
//! access, and a file's default application is looked up by its content
//! type. The query blocks, so it runs on a GIO worker thread, as the
//! Python bridge ran it on a worker (`properties` in `dispatch`).

use gio::prelude::*;
use gtk::gio;
use ox_core::entry::{entry_from_info, Entry, EntryError, ATTRIBUTES};
use ox_core::location::is_network_filesystem;
use ox_core::location::{normalise, parent_location};
use ox_core::network::{read_mount_table, MountEntry};
use ox_core::permissions::Account;

use super::drive_facts::{drive_type, filesystem_name, read_block_facts, DriveType};

/// The attributes Properties asks for beyond a listing's
/// (`PROPERTY_ATTRS` in `file_services.py`).
const PROPERTY_ATTRIBUTES: &str = concat!(
    "time::created,time::access,access::can-read,access::can-write,",
    "access::can-execute,owner::user,owner::group,unix::mode,unix::uid,unix::gid,",
    "standard::symlink-target,metadata::custom-icon,standard::allocated-size",
);

/// The permission bits Properties shows (`& 0o7777`).
const PERMISSION_BITS: u32 = 0o7777;

/// What the Properties dialog shows about one item.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ItemProperties {
    /// The item as a listing describes it: name, type, size, modified.
    pub entry: Entry,
    /// The folder the item is in; `None` at the top of a location.
    pub parent_uri: Option<String>,
    /// When the item was created, in seconds since the Unix epoch.
    pub created: Option<u64>,
    /// When the item was last read.
    pub accessed: Option<u64>,
    /// The owner's user name.
    pub owner: Option<String>,
    /// The owner's group.
    pub group: Option<String>,
    /// The owner's user id.
    pub uid: Option<u32>,
    /// The group's id.
    pub gid: Option<u32>,
    /// The permission bits, such as `0o644`.
    pub mode: Option<u32>,
    /// Whether the user may read, write and run the item, as the backend
    /// reports it; `None` where it does not say.
    pub access: Access,
    /// Where a symbolic link points.
    pub link_target: Option<String>,
    /// A symbolic link to a folder: its name, like the folder's, has no
    /// extension (OPS-049).
    pub links_to_folder: bool,
    /// The name of the application a file opens with.
    pub default_app: Option<String>,
    /// What is mounted at the item, when a folder is a mount point.
    pub mount: Option<MountFacts>,
    /// The width and height of a local image, read from its header
    /// (PROP-013).
    pub dimensions: Option<(i32, i32)>,
    /// The item's own icon, a picture's URI (PROP-016).
    pub custom_icon: Option<String>,
    /// The bytes the item takes on disk (`standard::allocated-size`), as
    /// Windows Explorer's "Size on disk"; `None` when the backend does not
    /// report them.
    pub size_on_disk: Option<u64>,
}

/// A mount point's details: a drive's General tab and its Hardware tab
/// (PROP-004).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MountFacts {
    /// Where it is mounted.
    pub mounted_on: String,
    /// What is mounted, for example `/dev/sdb1` or `//nas/share`.
    pub mounted_from: String,
    /// The file system as Windows names it, for example `NTFS` or `ext4`.
    pub filesystem: String,
    /// The volume label, empty without one.
    pub label: String,
    /// "Local Disk", "Removable Disk", "Network Drive" or "CD Drive".
    pub drive_type: DriveType,
    /// Free and total bytes, where the file system reports them.
    pub space: Option<(u64, u64)>,
}

/// The user's access to an item, where the backend reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Access {
    /// The user may read the item (`access::can-read`).
    pub readable: Option<bool>,
    /// The user may change the item (`access::can-write`).
    pub writable: Option<bool>,
    /// The user may run the item (`access::can-execute`).
    pub executable: Option<bool>,
}

impl ItemProperties {
    /// The owner, with its id and name.
    pub(crate) fn owner_account(&self) -> Option<Account> {
        account(self.uid, self.owner.as_deref())
    }

    /// The group, with its id and name.
    pub(crate) fn group_account(&self) -> Option<Account> {
        account(self.gid, self.group.as_deref())
    }

    /// The item as the Permissions tab changes it.
    pub(in crate::properties) fn edited_item(&self) -> super::permissions_editor::EditedItem {
        super::permissions_editor::EditedItem {
            uri: self.entry.uri.clone(),
            mode: self.mode.unwrap_or_default() & PERMISSION_BITS,
            is_folder: self.entry.is_dir,
        }
    }

    /// The permission bits as Python's `oct()` writes them (`0o644`).
    pub(crate) fn mode_text(&self) -> Option<String> {
        self.mode.map(|mode| format!("0o{:o}", mode & PERMISSION_BITS))
    }
}

/// The account `id` named `name`, or its number.
fn account(id: Option<u32>, name: Option<&str>) -> Option<Account> {
    let id = id?;
    let name = name.map_or_else(|| id.to_string(), str::to_owned);
    Some(Account { id, name })
}

/// Reads the properties of the item at `uri` on a GIO worker thread.
///
/// # Errors
///
/// Why the item could not be read: an address the location rules refuse,
/// a missing item, a share that must be mounted first, or any other GIO
/// failure.
pub(crate) async fn read_properties(uri: String) -> Result<ItemProperties, EntryError> {
    let worker = gio::spawn_blocking(move || read_properties_blocking(&uri));
    match worker.await {
        Ok(result) => result,
        // A panicking query is a bug; it surfaces where it is awaited.
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// [`read_properties`] on the calling thread.
fn read_properties_blocking(uri: &str) -> Result<ItemProperties, EntryError> {
    let file = gio::File::for_uri(&normalise(uri)?);
    let attributes = format!("{ATTRIBUTES},{PROPERTY_ATTRIBUTES}");
    let info = file.query_info(
        &attributes,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        gio::Cancellable::NONE,
    )?;
    let entry = entry_from_info(&file, &info);
    let default_app = default_app_for(&entry);
    let mount = if entry.is_dir { mount_at(&file) } else { None };
    let dimensions = image_dimensions(&file, &entry);
    Ok(ItemProperties {
        parent_uri: parent_location(&entry.uri),
        created: optional_u64(&info, "time::created"),
        accessed: optional_u64(&info, "time::access"),
        owner: optional_string(&info, "owner::user"),
        group: optional_string(&info, "owner::group"),
        uid: optional_u32(&info, "unix::uid"),
        gid: optional_u32(&info, "unix::gid"),
        mode: info
            .has_attribute("unix::mode")
            .then(|| info.attribute_uint32("unix::mode")),
        access: Access {
            readable: optional_bool(&info, "access::can-read"),
            writable: optional_bool(&info, "access::can-write"),
            executable: optional_bool(&info, "access::can-execute"),
        },
        link_target: link_target(&info),
        links_to_folder: info.is_symlink()
            && file.query_file_type(gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
                == gio::FileType::Directory,
        default_app,
        mount,
        dimensions,
        custom_icon: optional_string(&info, "metadata::custom-icon").filter(|icon| !icon.is_empty()),
        size_on_disk: info
            .has_attribute("standard::allocated-size")
            .then(|| info.attribute_uint64("standard::allocated-size")),
        entry,
    })
}

/// Whether the folder `uri` is a mount point of the kernel's mount table,
/// which makes it a drive. Reading the table never mounts anything.
pub(crate) fn is_mount_point(uri: &str) -> bool {
    let Some(path) = gio::File::for_uri(uri).path() else {
        return false;
    };
    let mounts = read_mount_table().unwrap_or_default();
    path.to_str()
        .is_some_and(|path| mounts.iter().any(|mount| mount.path == path))
}

/// Whether the drive mounted at `uri`, other than the system drive, can
/// keep a Customize choice, which is stored as `GVfs` metadata: a local
/// drive mounted read-write that takes metadata. Windows 11 has no
/// Customize on the system drive, here the root. Only the mount table is
/// read before GIO, so a network share that stopped answering is never
/// touched.
pub(crate) fn drive_keeps_customization(uri: &str) -> bool {
    let file = gio::File::for_uri(uri);
    let Some(path) = file.path() else {
        return false;
    };
    let Some(path) = path.to_str() else {
        return false;
    };
    let mounts = read_mount_table().unwrap_or_default();
    let Some(mount) = mounts.iter().rev().find(|mount| mount.path == path) else {
        return false;
    };
    is_customizable_mount(mount)
        && file
            .query_writable_namespaces(gio::Cancellable::NONE)
            .is_ok_and(|namespaces| {
                namespaces
                    .attributes()
                    .iter()
                    .any(|namespace| namespace.name() == "metadata")
            })
}

/// Whether the drive mounted as `mount` can keep a Customize choice, by
/// the mount table alone: not the system drive, not a network or FUSE
/// file system, and not mounted read-only.
fn is_customizable_mount(mount: &MountEntry) -> bool {
    let filesystem = mount.filesystem.as_str();
    let is_network = is_network_filesystem(filesystem)
        || matches!(
            filesystem,
            "smbfs"
                | "nfs"
                | "nfs4"
                | "afs"
                | "ceph"
                | "9p"
                | "glusterfs"
                | "lustre"
                | "davfs"
                | "ncpfs"
                | "coda"
                | "fuse"
        )
        || filesystem.starts_with("fuse.");
    let is_read_only = mount.options.split(',').any(|option| option == "ro");
    mount.path != "/" && !is_network && !is_read_only
}

/// What is mounted at the folder `file`, if it is a mount point of the
/// kernel's mount table. Reading the table never mounts anything.
fn mount_at(file: &gio::File) -> Option<MountFacts> {
    let path = file.path()?;
    let path = path.to_str()?;
    let mounts = read_mount_table().ok()?;
    let mount = mounts.into_iter().rev().find(|mount| mount.path == path)?;
    let info = file
        .query_filesystem_info(
            "filesystem::free,filesystem::size,filesystem::remote",
            gio::Cancellable::NONE,
        )
        .ok();
    let space = info.as_ref().and_then(|info| {
        let known = info.has_attribute("filesystem::free") && info.has_attribute("filesystem::size");
        known.then(|| {
            (
                info.attribute_uint64("filesystem::free"),
                info.attribute_uint64("filesystem::size"),
            )
        })
    });
    let remote = info
        .as_ref()
        .and_then(|info| optional_bool(info, "filesystem::remote"))
        .unwrap_or_default();
    let block = read_block_facts(&mount.source);
    Some(MountFacts {
        filesystem: filesystem_name(&mount.filesystem, &block),
        drive_type: drive_type(&mount.filesystem, remote, &block),
        label: block.label,
        mounted_on: mount.path,
        mounted_from: mount.source,
        space,
    })
}

/// The width and height of a local image, from its header only.
fn image_dimensions(file: &gio::File, entry: &Entry) -> Option<(i32, i32)> {
    let is_image = entry
        .content_type
        .as_deref()
        .is_some_and(|kind| kind.starts_with("image/"));
    if entry.is_dir || !is_image {
        return None;
    }
    let (_, width, height) = gtk::gdk_pixbuf::Pixbuf::file_info(file.path()?)?;
    Some((width, height))
}

/// The name of the application a file opens with; `None` for a folder
/// and for a type without one.
fn default_app_for(entry: &Entry) -> Option<String> {
    if entry.is_dir {
        return None;
    }
    let content_type = entry.content_type.as_deref()?;
    let app = gio::AppInfo::default_for_type(content_type, false)?;
    Some(app.display_name().to_string())
}

/// Where a symbolic link points. GIO sets the attribute only for links,
/// and reading a missing one is a critical warning, so it is looked up
/// first.
fn link_target(info: &gio::FileInfo) -> Option<String> {
    if !info.has_attribute("standard::symlink-target") {
        return None;
    }
    let target = info.symlink_target()?;
    Some(target.display().to_string())
}

fn optional_u64(info: &gio::FileInfo, attribute: &str) -> Option<u64> {
    info.has_attribute(attribute)
        .then(|| info.attribute_uint64(attribute))
}

fn optional_u32(info: &gio::FileInfo, attribute: &str) -> Option<u32> {
    info.has_attribute(attribute)
        .then(|| info.attribute_uint32(attribute))
}

fn optional_bool(info: &gio::FileInfo, attribute: &str) -> Option<bool> {
    info.has_attribute(attribute).then(|| info.boolean(attribute))
}

fn optional_string(info: &gio::FileInfo, attribute: &str) -> Option<String> {
    info.attribute_string(attribute).map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::file_entry;

    #[test]
    fn the_mode_reads_as_python_writes_it() {
        let properties = ItemProperties {
            entry: file_entry("notes.txt"),
            parent_uri: None,
            created: None,
            accessed: None,
            owner: None,
            group: None,
            uid: None,
            gid: None,
            mode: Some(0o100_644),
            access: Access::default(),
            link_target: None,
            links_to_folder: false,
            default_app: None,
            mount: None,
            dimensions: None,
            custom_icon: None,
            size_on_disk: None,
        };

        assert_eq!(properties.mode_text().as_deref(), Some("0o644"));
    }

    /// A drive at `path` of the file system `filesystem` mounted with
    /// `options`.
    fn mount(path: &str, filesystem: &str, options: &str) -> MountEntry {
        MountEntry {
            root: "/".to_owned(),
            path: path.to_owned(),
            filesystem: filesystem.to_owned(),
            source: "/dev/sdb1".to_owned(),
            options: options.to_owned(),
        }
    }

    /// Customize is for a local drive mounted read-write, other than the
    /// system drive: never the root, a network share or a read-only drive.
    #[test]
    fn only_a_local_writable_drive_other_than_the_root_is_customized() {
        assert!(is_customizable_mount(&mount(
            "/media/fictional/Data",
            "ext4",
            "rw"
        )));
        assert!(is_customizable_mount(&mount(
            "/mnt/data",
            "fuseblk",
            "rw,user_id=0"
        )));
        assert!(!is_customizable_mount(&mount("/", "ext4", "rw")));
        assert!(!is_customizable_mount(&mount("/mnt/disc", "iso9660", "ro")));
        assert!(!is_customizable_mount(&mount("/mnt/nas", "cifs", "rw")));
        assert!(!is_customizable_mount(&mount("/mnt/home", "nfs4", "rw")));
        assert!(!is_customizable_mount(&mount("/mnt/remote", "fuse.sshfs", "rw")));
    }
}
