// SPDX-License-Identifier: AGPL-3.0-only
//! Drives an ordinary user can mount, whatever filesystem they hold.
//!
//! An ext4 or XFS drive's owners and modes are written by the kernel, but
//! only from what is on the drive. On a drive the user brought, a stick or
//! a disk image, they say whatever its maker wrote, root and `0755`
//! included. So a mount is not believed when:
//!
//! - `/etc/fstab` lets users mount it: an entry for its mount point or its
//!   source has `user`, `users`, `owner` or `group` among its options, or
//!   `x-udisks-auth`, which lets `udisks` mount it for a user who asks.
//! - libmount's `/run/mount/utab` records it with a `user=` option, which
//!   `mount` writes when a user, not root, mounted it.
//! - its block device, or a device under it (a LUKS or LVM volume, a RAID),
//!   is removable or hot-pluggable: the disk's `removable` flag is set, a
//!   device above it in `/sys` is marked removable (Thunderbolt and USB
//!   ports), or it is attached through USB, an SD card slot, IEEE 1394 or
//!   Thunderbolt. A loop device (a disk image) counts too. The root
//!   filesystem is exempt: the system mounted it at boot, wherever it is,
//!   and nobody can mount anything at `/` in its place.
//! - it is on a local disk filesystem whose block device cannot be found
//!   in `/sys`.
//!
//! A user `fstab` entry that is not mounted (often `noauto`) matters too:
//! a user could mount their own drive there while the copy runs. So a
//! destination is also refused when such an entry's mount point is on its
//! path or inside it.
//!
//! Kept from the up-front refusal of 2.0.12 (#194), which the privileged
//! deletion helper replaced for deletes; a copy as administrator still
//! goes through `gvfsd-admin` by path, so it keeps these rules.
//!
//! Paths in `fstab` and `utab` are decoded like the mount table's (`\040`
//! for a space), and `UUID=`, `LABEL=`, `PARTUUID=`, `PARTLABEL=` and `ID=`
//! sources are resolved through `/dev/disk/by-*` to the kernel's device
//! name, as `mount` resolves them.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use crate::sizes::mounts::{unescape_octal, Mount};

/// The `fstab` options that let an ordinary user mount an entry.
const USER_OPTIONS: &[&str] = &["user", "users", "owner", "group", "x-udisks-auth"];

/// Local filesystems that live on a block device, which must be found.
const DISK_FILESYSTEMS: &[&str] = &["ext2", "ext3", "ext4", "xfs", "btrfs", "f2fs", "bcachefs", "jfs"];

/// The `fstab` source tags and the `/dev/disk` folder that resolves each.
const SOURCE_TAGS: &[(&str, &str)] = &[
    ("UUID=", "by-uuid"),
    ("LABEL=", "by-label"),
    ("PARTUUID=", "by-partuuid"),
    ("PARTLABEL=", "by-partlabel"),
    ("ID=", "by-id"),
];

/// How many devices deep a stack of devices (a LUKS volume on LVM on a
/// RAID) is followed; anything deeper is taken as removable.
const MAX_STACKED_DEVICES: usize = 8;

/// What the system says about who can mount which drive.
pub(super) struct UserMounts {
    /// Where `/etc`, `/run`, `/sys` and `/dev` are: `/`, except in tests.
    system: PathBuf,
    /// The `fstab` entries users may mount.
    fstab: Vec<FstabEntry>,
    /// The mounts `utab` records as a user's.
    utab: Vec<UtabEntry>,
}

/// An `fstab` entry users may mount.
struct FstabEntry {
    /// Its mount point as written, and with links resolved when it exists.
    points: Vec<PathBuf>,
    /// Its source as written, when it is a path, such as `/dev/sdb1`.
    source_path: Option<Vec<u8>>,
    /// The kernel name of its source's device, such as `sdb1`, when the
    /// device is there.
    device: Option<OsString>,
}

/// A mount `utab` records as a user's.
struct UtabEntry {
    /// Its mount ID, which newer libmount versions record.
    id: Option<u64>,
    /// Its mount point.
    target: PathBuf,
}

impl UserMounts {
    /// Reads `fstab` and `utab` under `system`. Either may be missing:
    /// without `/etc/fstab` no entry lets a user mount anything, and
    /// without `utab` no mount was made by a user through `mount`. One
    /// that exists but cannot be read is treated the same way; the device
    /// checks still apply to every mount.
    pub(super) fn read(system: &Path) -> Self {
        let read = |path: &str| fs::read(system.join(path)).unwrap_or_default();
        let fstab = parse_fstab(&read("etc/fstab"))
            .into_iter()
            .map(|(source, target)| FstabEntry {
                points: with_links_resolved(target),
                device: resolve_source(system, &source),
                source_path: source.starts_with(b"/").then_some(source),
            })
            .collect();
        Self {
            system: system.to_owned(),
            fstab,
            utab: parse_utab(&read("run/mount/utab")),
        }
    }

    /// Whether `mount` may be a drive an ordinary user mounted or supplied.
    pub(super) fn could_be_users(&self, mount: &Mount) -> bool {
        let devices = self.block_devices(mount);
        let names: Vec<&OsStr> = devices.iter().filter_map(|device| device.file_name()).collect();
        let in_fstab = self.fstab.iter().any(|entry| {
            entry.points.contains(&mount.point)
                || entry.source_path.as_ref() == Some(&mount.source)
                || entry
                    .device
                    .as_deref()
                    .is_some_and(|device| names.contains(&device))
        });
        let in_utab = self.utab.iter().any(|entry| match entry.id {
            Some(id) => id == mount.id,
            None => entry.target == mount.point,
        });
        let removable = if mount.point == Path::new("/") {
            false
        } else if devices.is_empty() {
            DISK_FILESYSTEMS.contains(&mount.filesystem.as_str())
        } else {
            devices.iter().any(|device| self.is_removable(device, 0))
        };
        in_fstab || in_utab || removable
    }

    /// A mount point of a user `fstab` entry at `path` or above it.
    pub(super) fn point_over(&self, path: &Path) -> Option<&Path> {
        self.points().find(|point| path.starts_with(point))
    }

    /// A mount point of a user `fstab` entry at `path` or inside it.
    pub(super) fn point_inside(&self, path: &Path) -> Option<&Path> {
        self.points().find(|point| point.starts_with(path))
    }

    /// Every mount point of a user `fstab` entry.
    fn points(&self) -> impl Iterator<Item = &Path> {
        self.fstab
            .iter()
            .flat_map(|entry| &entry.points)
            .map(PathBuf::as_path)
    }

    /// The `/sys` folders of the block devices `mount` is on: the one its
    /// device number names, or the one its source names (Btrfs reports an
    /// anonymous device number), and the other devices of a Btrfs
    /// filesystem that spans several.
    fn block_devices(&self, mount: &Mount) -> Vec<PathBuf> {
        let sys = self.system.join("sys");
        let (major, minor) = mount.device;
        let mut devices: Vec<PathBuf> = fs::canonicalize(sys.join(format!("dev/block/{major}:{minor}")))
            .into_iter()
            .collect();
        if devices.is_empty() {
            // bcachefs names every device of a filesystem, joined with `:`.
            for source in mount.source.split(|&byte| byte == b':') {
                let name = source
                    .starts_with(b"/")
                    .then(|| device_name(&self.system, source))
                    .flatten();
                if let Some(device) =
                    name.and_then(|name| fs::canonicalize(sys.join("class/block").join(name)).ok())
                {
                    devices.push(device);
                }
            }
        }
        if mount.filesystem == "btrfs" {
            let names: Vec<OsString> = devices
                .iter()
                .filter_map(|device| device.file_name().map(OsStr::to_owned))
                .collect();
            devices.extend(btrfs_siblings(&sys, &names));
        }
        devices
    }

    /// Whether the block device whose `/sys` folder is `device` can be
    /// unplugged, or is an image, or is built on such a device.
    fn is_removable(&self, device: &Path, depth: usize) -> bool {
        let sys = self.system.join("sys");
        let Ok(inside) = device.strip_prefix(&sys) else {
            return true;
        };
        if depth > MAX_STACKED_DEVICES || inside.iter().any(is_hot_pluggable_bus) {
            return true;
        }
        // A loop device's partitions sit inside its folder, which alone
        // has `loop/`.
        let marked_removable = device
            .ancestors()
            .take_while(|folder| folder.starts_with(&sys) && *folder != sys)
            .any(|folder| {
                folder.join("loop").exists()
                    || fs::read_to_string(folder.join("removable"))
                        .is_ok_and(|flag| matches!(flag.trim(), "1" | "removable"))
            });
        if marked_removable {
            return true;
        }
        let Ok(below) = fs::read_dir(device.join("slaves")) else {
            return false;
        };
        below.flatten().any(|entry| {
            fs::canonicalize(entry.path()).map_or(true, |below| self.is_removable(&below, depth + 1))
        })
    }
}

/// Whether a `/sys` folder name is a bus whose devices can be unplugged:
/// USB, SD and MMC cards, IEEE 1394, Thunderbolt.
fn is_hot_pluggable_bus(name: &OsStr) -> bool {
    let name = name.as_bytes();
    name.starts_with(b"usb")
        || name.starts_with(b"mmc")
        || contains(name, b"firewire")
        || contains(name, b"ieee1394")
        || contains(name, b"thunderbolt")
}

/// Whether `text` contains `part`.
fn contains(text: &[u8], part: &[u8]) -> bool {
    text.windows(part.len()).any(|window| window == part)
}

/// The `/sys` folders of the other devices of each Btrfs filesystem that
/// has one of the devices `names`.
fn btrfs_siblings(sys: &Path, names: &[OsString]) -> Vec<PathBuf> {
    let Ok(filesystems) = fs::read_dir(sys.join("fs/btrfs")) else {
        return Vec::new();
    };
    let mut siblings = Vec::new();
    for filesystem in filesystems.flatten() {
        let Ok(members) = fs::read_dir(filesystem.path().join("devices")) else {
            continue;
        };
        let members: Vec<_> = members.flatten().collect();
        if members.iter().any(|member| names.contains(&member.file_name())) {
            siblings.extend(
                members
                    .iter()
                    .filter_map(|member| fs::canonicalize(member.path()).ok()),
            );
        }
    }
    siblings
}

/// The kernel name of the device at `path` (such as `/dev/mapper/home`,
/// which is `dm-0`), when it is under `/dev` in `system`.
fn device_name(system: &Path, path: &[u8]) -> Option<OsString> {
    let relative = Path::new(OsStr::from_bytes(path.strip_prefix(b"/")?));
    let resolved = fs::canonicalize(system.join(relative)).ok()?;
    resolved
        .starts_with(system.join("dev"))
        .then(|| resolved.file_name().map(OsStr::to_owned))
        .flatten()
}

/// The kernel name of the device an `fstab` source names, when it is there.
fn resolve_source(system: &Path, source: &[u8]) -> Option<OsString> {
    if source.starts_with(b"/") {
        return device_name(system, source);
    }
    let (tag, folder) = SOURCE_TAGS
        .iter()
        .find(|(tag, _)| source.starts_with(tag.as_bytes()))?;
    let value = unquote(&source[tag.len()..]);
    let mut names = vec![udev_encoded(value)];
    if *tag == "UUID=" {
        names.push(udev_encoded(&value.to_ascii_lowercase()));
    }
    names.into_iter().find_map(|name| {
        let mut link = b"/dev/disk/".to_vec();
        link.extend_from_slice(folder.as_bytes());
        link.push(b'/');
        link.extend_from_slice(&name);
        device_name(system, &link)
    })
}

/// `value` without the double or single quotes around it.
fn unquote(value: &[u8]) -> &[u8] {
    for quote in *b"\"'" {
        if value.len() >= 2 && value.first() == Some(&quote) && value.last() == Some(&quote) {
            return &value[1..value.len() - 1];
        }
    }
    value
}

/// `value` as udev names its links in `/dev/disk/by-*`: letters, digits,
/// `#+-.:=@_` and multibyte UTF-8 characters as they are, and every other
/// byte as `\xHH`.
fn udev_encoded(value: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(value.len());
    let mut rest = value;
    while let Some(&byte) = rest.first() {
        let length = utf8_length(rest);
        if length > 1 {
            encoded.extend_from_slice(&rest[..length]);
            rest = &rest[length..];
            continue;
        }
        if byte.is_ascii_alphanumeric() || b"#+-.:=@_".contains(&byte) {
            encoded.push(byte);
        } else {
            encoded.extend_from_slice(format!("\\x{byte:02x}").as_bytes());
        }
        rest = &rest[1..];
    }
    encoded
}

/// The length of the valid UTF-8 character `text` starts with, or 1.
fn utf8_length(text: &[u8]) -> usize {
    let length = match text[0] {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return 1,
    };
    match text.get(..length) {
        Some(character) if std::str::from_utf8(character).is_ok() => length,
        _ => 1,
    }
}

/// The source and mount point of each `fstab` entry users may mount,
/// noauto entries included, with octal escapes decoded. Comments, blank
/// lines and lines with fewer than two fields are skipped, as by
/// libmount. An entry without options has the defaults, which do not let
/// users mount it.
fn parse_fstab(table: &[u8]) -> Vec<(Vec<u8>, PathBuf)> {
    table
        .split(|&byte| byte == b'\n')
        .filter_map(|line| {
            let mut fields = line
                .split(|&byte| byte == b' ' || byte == b'\t')
                .filter(|field| !field.is_empty());
            let source = fields.next().filter(|source| !source.starts_with(b"#"))?;
            let target = fields.next()?;
            let options = fields.nth(1).unwrap_or_default();
            let users_may = options
                .split(|&byte| byte == b',')
                .any(|option| USER_OPTIONS.iter().any(|user| option == user.as_bytes()));
            users_may.then(|| {
                let target = PathBuf::from(OsString::from_vec(unescape_octal(target)));
                (unescape_octal(source), target)
            })
        })
        .filter(|(_, target)| target.is_absolute())
        .collect()
}

/// The mounts of a libmount `utab` with a `user=` option. Its lines are
/// `KEY=value` fields (`ID=`, `SRC=`, `TARGET=`, `ROOT=`, `OPTS=`, …)
/// separated by spaces, with octal escapes in the values.
fn parse_utab(table: &[u8]) -> Vec<UtabEntry> {
    table
        .split(|&byte| byte == b'\n')
        .filter_map(|line| {
            let mut id = None;
            let mut target = None;
            let mut by_user = false;
            for field in line.split(|&byte| byte == b' ').filter(|field| !field.is_empty()) {
                if let Some(value) = field.strip_prefix(b"ID=") {
                    id = std::str::from_utf8(value)
                        .ok()
                        .and_then(|value| value.parse().ok());
                } else if let Some(value) = field.strip_prefix(b"TARGET=") {
                    target = Some(PathBuf::from(OsString::from_vec(unescape_octal(value))));
                } else if let Some(value) = field.strip_prefix(b"OPTS=") {
                    by_user = unescape_octal(value).split(|&byte| byte == b',').any(|option| {
                        option.starts_with(b"user=") || option == b"user" || option == b"users"
                    });
                }
            }
            by_user.then_some(UtabEntry { id, target: target? })
        })
        .collect()
}

/// `target`, and `target` with its links resolved when that differs.
fn with_links_resolved(target: PathBuf) -> Vec<PathBuf> {
    match fs::canonicalize(&target) {
        Ok(resolved) if resolved != target => vec![target, resolved],
        _ => vec![target],
    }
}
