// SPDX-License-Identifier: AGPL-3.0-only
//! What the Properties dialog reads about one item.
//!
//! Ports `properties` in `desktop/file_services.py`: the item itself is
//! queried without following a link, with its times, owner, mode and
//! access, and a file's default application is looked up by its content
//! type. The query blocks, so it runs on a GIO worker thread, as the
//! Python bridge ran it on a worker (`properties` in `dispatch`).

use gio::prelude::*;
use gtk::gio;
use ox_core::entry::{entry_from_info, Entry, EntryError, ATTRIBUTES};
use ox_core::location::{normalise, parent_location};
use ox_core::network::read_mount_table;

/// The attributes Properties asks for beyond a listing's
/// (`PROPERTY_ATTRS` in `file_services.py`).
const PROPERTY_ATTRIBUTES: &str = concat!(
    "time::created,time::access,access::can-read,access::can-write,",
    "access::can-execute,owner::user,owner::group,unix::mode,",
    "standard::symlink-target",
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
    /// The permission bits, such as `0o644`.
    pub mode: Option<u32>,
    /// Whether the user may read, write and run the item, as the backend
    /// reports it; `None` where it does not say.
    pub access: Access,
    /// Where a symbolic link points.
    pub link_target: Option<String>,
    /// The name of the application a file opens with.
    pub default_app: Option<String>,
    /// What is mounted at the item, when a folder is a mount point.
    pub mount: Option<MountFacts>,
}

/// A mount point's details for the General tab (PROP-004).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MountFacts {
    /// Where it is mounted.
    pub mounted_on: String,
    /// What is mounted, for example `/dev/sdb1` or `//nas/share`.
    pub mounted_from: String,
    /// The file system type, for example `ext4`.
    pub filesystem: String,
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
    /// The permission bits as Python's `oct()` writes them (`0o644`).
    pub(crate) fn mode_text(&self) -> Option<String> {
        self.mode.map(|mode| format!("0o{:o}", mode & PERMISSION_BITS))
    }
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
    Ok(ItemProperties {
        parent_uri: parent_location(&entry.uri),
        created: optional_u64(&info, "time::created"),
        accessed: optional_u64(&info, "time::access"),
        owner: optional_string(&info, "owner::user"),
        group: optional_string(&info, "owner::group"),
        mode: info
            .has_attribute("unix::mode")
            .then(|| info.attribute_uint32("unix::mode")),
        access: Access {
            readable: optional_bool(&info, "access::can-read"),
            writable: optional_bool(&info, "access::can-write"),
            executable: optional_bool(&info, "access::can-execute"),
        },
        link_target: link_target(&info),
        default_app,
        mount,
        entry,
    })
}

/// What is mounted at the folder `file`, if it is a mount point of the
/// kernel's mount table. Reading the table never mounts anything.
fn mount_at(file: &gio::File) -> Option<MountFacts> {
    let path = file.path()?;
    let path = path.to_str()?;
    let mounts = read_mount_table().ok()?;
    let mount = mounts.into_iter().rev().find(|mount| mount.path == path)?;
    let space = file
        .query_filesystem_info("filesystem::free,filesystem::size", gio::Cancellable::NONE)
        .ok()
        .and_then(|info| {
            let known = info.has_attribute("filesystem::free") && info.has_attribute("filesystem::size");
            known.then(|| {
                (
                    info.attribute_uint64("filesystem::free"),
                    info.attribute_uint64("filesystem::size"),
                )
            })
        });
    Some(MountFacts {
        mounted_on: mount.path,
        mounted_from: mount.source,
        filesystem: mount.filesystem,
        space,
    })
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
            mode: Some(0o100_644),
            access: Access::default(),
            link_target: None,
            default_app: None,
            mount: None,
        };

        assert_eq!(properties.mode_text().as_deref(), Some("0o644"));
    }
}
