// SPDX-License-Identifier: AGPL-3.0-only
//! What a drive's Properties say about it, as Windows 11's General tab
//! names it: its volume label, its type ("Local Disk", "Removable Disk",
//! "Network Drive", "CD Drive") and its file system ("NTFS", "FAT32",
//! "exFAT", or Linux's own, such as "ext4").
//!
//! A drive is a folder of the kernel's mount table. Its block device's
//! label, file system and FAT version come from udev's database, as
//! UDisks reads them, and whether it is removable from sysfs, which
//! Windows' "Removable Disk" follows: a USB stick says so, a USB hard
//! disk does not. Where neither can be read, as inside the Flatpak, the
//! mount table's file system type is shown and the label is empty.
//! Everything here reads local files only and never mounts anything.

use std::path::{Path, PathBuf};

use ox_core::location::is_network_filesystem;

/// udev's database of devices, one file per device number.
const UDEV_DATA: &str = "/run/udev/data";

/// The kernel's block devices, by name.
const SYS_BLOCK: &str = "/sys/class/block";

/// What kind of drive Windows calls a volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DriveType {
    /// A fixed disk: "Local Disk".
    Local,
    /// A disk the kernel marks removable, such as a USB stick or SD card.
    Removable,
    /// A share mounted over the network.
    Network,
    /// An optical disc.
    Cd,
}

impl DriveType {
    /// Windows' words for it on the General tab's Type row.
    pub(crate) fn label(self) -> String {
        match self {
            DriveType::Local => ox_core::i18n::gettext("Local Disk"),
            DriveType::Removable => ox_core::i18n::gettext("Removable Disk"),
            DriveType::Network => ox_core::i18n::gettext("Network Drive"),
            DriveType::Cd => ox_core::i18n::gettext("CD Drive"),
        }
    }
}

/// The block device's facts, read from udev and sysfs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct BlockFacts {
    /// The volume label, empty without one.
    pub label: String,
    /// The file system udev found on it (`ID_FS_TYPE`), such as `ntfs`.
    pub filesystem: Option<String>,
    /// The file system's version (`ID_FS_VERSION`), such as `FAT32`.
    pub version: Option<String>,
    /// Whether the kernel marks the disk removable.
    pub removable: bool,
    /// Whether it is an optical drive (`sr0`).
    pub optical: bool,
}

/// Reads what udev and sysfs know about the device `source` mounts, such
/// as `/dev/sdb1`; empty facts for a source that is not a block device.
pub(crate) fn read_block_facts(source: &str) -> BlockFacts {
    if !source.starts_with("/dev/") {
        return BlockFacts::default();
    }
    let Some(name) = std::fs::canonicalize(source)
        .ok()
        .and_then(|device| device.file_name().map(|name| name.to_string_lossy().into_owned()))
    else {
        return BlockFacts::default();
    };
    let sys = Path::new(SYS_BLOCK).join(&name);
    let mut facts = std::fs::read_to_string(sys.join("dev"))
        .ok()
        .and_then(|number| {
            std::fs::read_to_string(Path::new(UDEV_DATA).join(format!("b{}", number.trim()))).ok()
        })
        .map(|data| udev_facts(&data))
        .unwrap_or_default();
    facts.removable = disk_of(&sys).is_some_and(|disk| {
        std::fs::read_to_string(disk.join("removable")).is_ok_and(|flag| flag.trim() == "1")
    });
    facts.optical = name.starts_with("sr");
    facts
}

/// The sysfs folder of the disk the block device at `sys` is on: its
/// parent for a partition, else itself.
fn disk_of(sys: &Path) -> Option<PathBuf> {
    let device = std::fs::canonicalize(sys).ok()?;
    if device.join("partition").exists() {
        return device.parent().map(Path::to_path_buf);
    }
    Some(device)
}

/// The label, file system and version in udev's record `data` of one
/// device.
pub(crate) fn udev_facts(data: &str) -> BlockFacts {
    let mut facts = BlockFacts::default();
    let property = |line: &str, key: &str| {
        line.strip_prefix("E:")
            .and_then(|line| line.strip_prefix(key))
            .and_then(|line| line.strip_prefix('='))
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    for line in data.lines() {
        if let Some(label) = property(line, "ID_FS_LABEL_ENC") {
            facts.label = decode_udev(&label);
        } else if let Some(filesystem) = property(line, "ID_FS_TYPE") {
            facts.filesystem = Some(filesystem);
        } else if let Some(version) = property(line, "ID_FS_VERSION") {
            facts.version = Some(version);
        }
    }
    facts
}

/// `text` with udev's `\x20` escapes turned back into the bytes they
/// stand for.
pub(crate) fn decode_udev(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let escape = bytes.get(at..at + 4).filter(|chunk| chunk.starts_with(b"\\x"));
        let byte = escape
            .and_then(|chunk| std::str::from_utf8(&chunk[2..]).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(byte) = byte {
            decoded.push(byte);
            at += 4;
        } else {
            decoded.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// The file system's name as Windows writes it ("NTFS", "FAT32",
/// "exFAT", "CDFS", "UDF"), or Linux's own name for one Windows does not
/// know, such as "ext4": udev's when it found one, else the mount's.
pub(crate) fn filesystem_name(mounted: &str, block: &BlockFacts) -> String {
    let filesystem = block.filesystem.as_deref().unwrap_or(mounted);
    match filesystem {
        "vfat" | "msdos" if block.version.as_deref() == Some("FAT32") => "FAT32".to_owned(),
        "vfat" | "msdos" => "FAT".to_owned(),
        "exfat" => "exFAT".to_owned(),
        "ntfs" | "ntfs3" => "NTFS".to_owned(),
        "iso9660" => "CDFS".to_owned(),
        "udf" => "UDF".to_owned(),
        other => other.to_owned(),
    }
}

/// The kind of drive mounted from `block` with the file system type
/// `mounted`; `remote` when GIO says the file system is on the network.
pub(crate) fn drive_type(mounted: &str, remote: bool, block: &BlockFacts) -> DriveType {
    let filesystem = block.filesystem.as_deref().unwrap_or(mounted);
    if block.optical || matches!(filesystem, "iso9660" | "udf") {
        DriveType::Cd
    } else if remote
        || (!mounted.is_empty() && is_network_filesystem(mounted))
        || matches!(mounted, "nfs" | "nfs4" | "fuse.sshfs" | "9p")
    {
        DriveType::Network
    } else if block.removable {
        DriveType::Removable
    } else {
        DriveType::Local
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn udev_records_give_the_label_file_system_and_version() {
        let data = "S:disk/by-label/MY\\x20STICK\nE:ID_FS_LABEL=MY_STICK\nE:ID_FS_LABEL_ENC=MY\\x20STICK\n\
                    E:ID_FS_TYPE=vfat\nE:ID_FS_VERSION=FAT32\nE:ID_FS_UUID=1234-ABCD\n";

        let facts = udev_facts(data);

        assert_eq!(facts.label, "MY STICK");
        assert_eq!(facts.filesystem.as_deref(), Some("vfat"));
        assert_eq!(facts.version.as_deref(), Some("FAT32"));
    }

    #[test]
    fn escapes_decode_to_utf8_and_a_lone_backslash_stays() {
        assert_eq!(decode_udev("Fotos\\x20\\xc3\\xa9t\\xc3\\xa9"), "Fotos été");
        assert_eq!(decode_udev("a\\b\\x2"), "a\\b\\x2");
    }

    #[test]
    fn file_systems_read_as_windows_names_them() {
        let fat32 = BlockFacts {
            filesystem: Some("vfat".to_owned()),
            version: Some("FAT32".to_owned()),
            ..BlockFacts::default()
        };
        let fat16 = BlockFacts {
            version: Some("FAT16".to_owned()),
            ..fat32.clone()
        };
        let ntfs_3g = BlockFacts {
            filesystem: Some("ntfs".to_owned()),
            ..BlockFacts::default()
        };

        assert_eq!(filesystem_name("vfat", &fat32), "FAT32");
        assert_eq!(filesystem_name("vfat", &fat16), "FAT");
        assert_eq!(filesystem_name("fuseblk", &ntfs_3g), "NTFS");
        assert_eq!(filesystem_name("exfat", &BlockFacts::default()), "exFAT");
        assert_eq!(filesystem_name("ext4", &BlockFacts::default()), "ext4");
    }

    #[test]
    fn drives_are_local_removable_network_or_cd() {
        let stick = BlockFacts {
            removable: true,
            ..BlockFacts::default()
        };
        let disc = BlockFacts {
            optical: true,
            ..BlockFacts::default()
        };

        assert_eq!(
            drive_type("ext4", false, &BlockFacts::default()),
            DriveType::Local
        );
        assert_eq!(drive_type("vfat", false, &stick), DriveType::Removable);
        assert_eq!(
            drive_type("cifs", false, &BlockFacts::default()),
            DriveType::Network
        );
        assert_eq!(
            drive_type("nfs4", false, &BlockFacts::default()),
            DriveType::Network
        );
        assert_eq!(
            drive_type("fuse.foo", true, &BlockFacts::default()),
            DriveType::Network
        );
        assert_eq!(drive_type("iso9660", false, &disc), DriveType::Cd);
    }
}
