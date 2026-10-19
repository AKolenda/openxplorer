// SPDX-License-Identifier: AGPL-3.0-only
//! Formatting a removable drive through UDisks (DEV-012).
//!
//! New in the native app: the Python app opened GNOME Disks' Format
//! dialog, which follows the desktop's own theme. Here the app shows
//! Windows' Format dialog itself and asks the system's
//! `org.freedesktop.UDisks2` service to do the work, as GNOME Disks and
//! KDE Partition Manager do. UDisks checks with polkit, which may ask for
//! a password; the calls allow that, so the desktop's own authentication
//! dialog shows.
//!
//! A drive is named by its block device, such as `/dev/sdb1`. Formatting
//! unmounts it, makes the new file system with its label, and mounts it
//! again, so the drive stays in the sidebar as Windows keeps its drive
//! letter. A full format (Quick format unchecked) first overwrites the
//! whole drive with zeros, which takes as long as writing it.

use gio::prelude::*;

/// The bus name of the UDisks service, on the system bus.
pub const UDISKS_NAME: &str = "org.freedesktop.UDisks2";

/// The object of UDisks' manager.
const MANAGER_PATH: &str = "/org/freedesktop/UDisks2/Manager";

/// UDisks' manager: resolving a device, which file systems it can make.
const MANAGER_INTERFACE: &str = "org.freedesktop.UDisks2.Manager";

/// A block device: its size, label and file system, and `Format`.
const BLOCK_INTERFACE: &str = "org.freedesktop.UDisks2.Block";

/// A block device holding a file system that can be mounted.
const FILESYSTEM_INTERFACE: &str = "org.freedesktop.UDisks2.Filesystem";

/// The standard interface every object's properties are read through.
const PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";

/// How long reading the drive may take.
const QUERY_TIMEOUT_MS: i32 = 25_000;

/// No timeout: formatting, a full format above all, takes as long as the
/// drive needs, and the password prompt waits for the user.
const NO_TIMEOUT_MS: i32 = i32::MAX;

/// Drives larger than this get exFAT by default, as in Windows; FAT32
/// suits smaller ones.
const FAT32_DEFAULT_LIMIT: u64 = 32 * 1024 * 1024 * 1024;

/// The smallest drive offered FAT32: it needs at least 65,525 clusters,
/// about 33 MB with the smallest; with fewer `mkfs.fat` makes one out of
/// specification that devices may not read.
const FAT32_MIN_SIZE: u64 = 64 * 1024 * 1024;

/// The characters a FAT volume label cannot hold, as Windows refuses
/// them; `mkfs.fat` and UDisks refuse them too, after the drive was
/// unmounted.
const FAT_LABEL_FORBIDDEN: &str = "*?.,;:/\\|+=<>[]\"";

/// The prefix of the D-Bus errors UDisks answers with.
const UDISKS_ERROR: &str = "org.freedesktop.UDisks2.Error.";

/// A file system the Format dialog offers, in the order Windows lists
/// them, followed by Linux's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSystem {
    /// exFAT, for large drives used on any computer.
    ExFat,
    /// FAT32, which every device reads.
    Fat32,
    /// NTFS, Windows' own.
    Ntfs,
    /// ext4, Linux's own.
    Ext4,
}

impl FileSystem {
    /// Every file system, in the order the dialog lists them.
    pub const ALL: [Self; 4] = [Self::ExFat, Self::Fat32, Self::Ntfs, Self::Ext4];

    /// The type UDisks names it by, as `Format` takes it and `IdType`
    /// reports it.
    pub fn udisks_type(self) -> &'static str {
        match self {
            Self::ExFat => "exfat",
            Self::Fat32 => "vfat",
            Self::Ntfs => "ntfs",
            Self::Ext4 => "ext4",
        }
    }

    /// The name the dialog shows, as Windows writes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::ExFat => "exFAT",
            Self::Fat32 => "FAT32",
            Self::Ntfs => "NTFS",
            Self::Ext4 => "ext4",
        }
    }

    /// The file system UDisks reports as `id_type`, if the dialog offers
    /// it.
    pub fn from_udisks_type(id_type: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|system| system.udisks_type() == id_type)
    }

    /// Whether a drive of `size` bytes can hold the file system: FAT32
    /// needs [`FAT32_MIN_SIZE`].
    pub fn fits(self, size: u64) -> bool {
        self != Self::Fat32 || size >= FAT32_MIN_SIZE
    }

    /// The longest volume label the file system keeps, in the units
    /// [`Self::label_units`] counts; also the most characters it keeps.
    pub fn max_label_length(self) -> usize {
        match self {
            Self::Fat32 | Self::ExFat => 11,
            Self::Ext4 => 16,
            Self::Ntfs => 32,
        }
    }

    /// How much of the label `character` takes: bytes for FAT32 and ext4,
    /// UTF-16 code units for exFAT and NTFS, which keep their labels in
    /// UTF-16. UDisks refuses a longer label, after the drive was
    /// unmounted.
    fn label_units(self, character: char) -> usize {
        match self {
            Self::Fat32 | Self::Ext4 => character.len_utf8(),
            Self::ExFat | Self::Ntfs => character.len_utf16(),
        }
    }

    /// `label` as the file system keeps it: trimmed and cut to its
    /// length without splitting a character; for FAT32 in capitals as
    /// Windows writes them, without the characters FAT refuses.
    pub fn fitted_label(self, label: &str) -> String {
        let label = label.trim();
        let label = if self == Self::Fat32 {
            label
                .chars()
                .filter(|character| !character.is_control() && !FAT_LABEL_FORBIDDEN.contains(*character))
                .collect::<String>()
                .trim()
                .to_uppercase()
        } else {
            label.to_owned()
        };
        let mut used = 0;
        let fitted: String = label
            .chars()
            .take_while(|character| {
                used += self.label_units(*character);
                used <= self.max_label_length()
            })
            .collect();
        fitted.trim_end().to_owned()
    }
}

/// The file system the dialog chooses first among `available`: the
/// drive's own, as Windows does, else exFAT for a drive above 32 GB and
/// FAT32 for a smaller one, else the first available.
pub fn default_file_system(
    current: Option<FileSystem>,
    size: u64,
    available: &[FileSystem],
) -> Option<FileSystem> {
    let by_size = if size > FAT32_DEFAULT_LIMIT {
        FileSystem::ExFat
    } else {
        FileSystem::Fat32
    };
    current
        .into_iter()
        .chain([by_size])
        .chain(available.iter().copied())
        .find(|system| available.contains(system))
}

/// What UDisks says about the drive to format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveFacts {
    /// The UDisks object of its block device. UDisks names it after the
    /// device, such as `sdb1`, so another drive plugged in later can get
    /// the same object.
    pub object: String,
    /// The UDisks object of the drive the block device is on, named after
    /// its model and serial number; empty when UDisks knows none.
    pub drive: String,
    /// The UUID of its file system, empty without one.
    pub uuid: String,
    /// Its capacity in bytes.
    pub size: u64,
    /// Its volume label, empty without one.
    pub label: String,
    /// The file system it holds, if the dialog offers that one.
    pub file_system: Option<FileSystem>,
    /// Whether it is write-protected.
    pub read_only: bool,
    /// Whether it holds a file system UDisks can mount and unmount.
    pub has_file_system: bool,
}

impl DriveFacts {
    /// Whether `other` describes the same drive, holding the same file
    /// system: a drive swapped in at the same device differs in its
    /// drive, its UUID or its size.
    fn is_same_drive(&self, other: &Self) -> bool {
        (&self.object, &self.drive, &self.uuid, self.size)
            == (&other.object, &other.drive, &other.uuid, other.size)
    }
}

/// What the Format dialog asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatRequest {
    /// The new file system.
    pub file_system: FileSystem,
    /// The new volume label, fitted to the file system.
    pub label: String,
    /// Quick format: only a new file system; without it the drive is
    /// overwritten with zeros first.
    pub quick: bool,
}

/// Why the drive was not formatted. `Display` is the message the window
/// shows.
#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    /// UDisks cannot be reached; the D-Bus error is the source, kept out
    /// of the message.
    #[error(
        "{}",
        crate::i18n::gettext("Formatting needs the UDisks service, which is not available.")
    )]
    Unavailable(#[source] glib::Error),
    /// UDisks does not know the drive, which was removed.
    #[error("{}", crate::i18n::gettext("The drive is no longer connected."))]
    NotFound,
    /// The drive is write-protected.
    #[error("{}", crate::i18n::gettext("The drive is write-protected."))]
    ReadOnly,
    /// The user dismissed the password prompt.
    #[error("{}", crate::i18n::gettext("Formatting was cancelled."))]
    Cancelled,
    /// The system does not allow this user to format the drive.
    #[error("{}", crate::i18n::gettext("You are not allowed to format this drive."))]
    NotAuthorized,
    /// A program still uses the drive, so it cannot be unmounted.
    #[error(
        "{}",
        crate::i18n::gettext("The drive is in use. Close the files and programs using it, then try again.")
    )]
    Busy,
    /// Formatting failed for another reason, which UDisks names.
    #[error(
        "{}",
        crate::i18n::format_message("Could not format the drive: {error}", &[("error", .0.as_str())])
    )]
    Failed(String),
}

impl FormatError {
    /// The error UDisks' answer `error` means.
    fn from_call(error: glib::Error) -> Self {
        let name = gio::DBusError::remote_error(&error).map(|name| name.to_string());
        let Some(name) = name else {
            return Self::Unavailable(error);
        };
        match name.strip_prefix(UDISKS_ERROR) {
            Some("NotAuthorizedDismissed" | "Cancelled") => Self::Cancelled,
            Some("NotAuthorized" | "NotAuthorizedCanObtain") => Self::NotAuthorized,
            Some("DeviceBusy") => Self::Busy,
            Some(_) => {
                let mut error = error;
                gio::DBusError::strip_remote_error(&mut error);
                Self::Failed(error.message().to_owned())
            }
            None if name.starts_with("org.freedesktop.DBus.Error.") => Self::Unavailable(error),
            None => Self::Failed(error.message().to_owned()),
        }
    }
}

/// Reads what UDisks `service` on `connection` knows about the block
/// device `device`, such as `/dev/sdb1`.
///
/// # Errors
///
/// [`FormatError::NotFound`] when UDisks does not know it, and the error
/// of an unanswered call otherwise.
pub async fn drive_facts(
    connection: &gio::DBusConnection,
    service: &str,
    device: &str,
) -> Result<DriveFacts, FormatError> {
    let object = resolve_device(connection, service, device).await?;
    facts_of(connection, service, object).await
}

/// Reads what UDisks `service` on `connection` knows about the block
/// device `object`.
async fn facts_of(
    connection: &gio::DBusConnection,
    service: &str,
    object: String,
) -> Result<DriveFacts, FormatError> {
    let block = properties(connection, service, &object, BLOCK_INTERFACE)
        .await?
        .ok_or(FormatError::NotFound)?;
    let has_file_system = properties(connection, service, &object, FILESYSTEM_INTERFACE)
        .await?
        .is_some();
    // `Drive` is an object path, which `lookup::<String>` refuses.
    let text = |name: &str| {
        block
            .lookup_value(name, None)
            .and_then(|value| value.str().map(str::to_owned))
            .unwrap_or_default()
    };
    let id_type = text("IdType");
    Ok(DriveFacts {
        size: block.lookup::<u64>("Size").ok().flatten().unwrap_or_default(),
        label: text("IdLabel"),
        drive: text("Drive"),
        uuid: text("IdUUID"),
        file_system: FileSystem::from_udisks_type(&id_type),
        read_only: block
            .lookup::<bool>("ReadOnly")
            .ok()
            .flatten()
            .unwrap_or_default(),
        has_file_system,
        object,
    })
}

/// The file systems UDisks `service` can make on a drive of `size` bytes:
/// those that fit it whose formatting program is installed. A UDisks too
/// old to say offers them all, and `Format` reports one it cannot make.
pub async fn available_file_systems(
    connection: &gio::DBusConnection,
    service: &str,
    size: u64,
) -> Vec<FileSystem> {
    let mut available = Vec::new();
    for system in FileSystem::ALL.into_iter().filter(|system| system.fits(size)) {
        let answer = connection
            .call_future(
                Some(service),
                MANAGER_PATH,
                MANAGER_INTERFACE,
                "CanFormat",
                Some(&(system.udisks_type(),).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                QUERY_TIMEOUT_MS,
            )
            .await;
        let can_format = match answer {
            Ok(reply) => reply.get::<((bool, String),)>().is_some_and(|((can, _),)| can),
            Err(_) => true,
        };
        if can_format {
            available.push(system);
        }
    }
    available
}

/// Formats the drive `facts` describes as `request` asks, through UDisks
/// `service` on `connection`: unmounts it, makes the file system, and
/// mounts it again. Returns where it is mounted, if mounting succeeded.
///
/// The drive is read again first: one removed while the dialog was open,
/// or another plugged in at its device since, is not touched.
///
/// # Errors
///
/// [`FormatError::NotFound`] for a drive that is gone or was replaced,
/// [`FormatError::ReadOnly`] for a write-protected drive, and what
/// unmounting or formatting answered ([`FormatError::from_call`]).
pub async fn format_drive(
    connection: &gio::DBusConnection,
    service: &str,
    facts: &DriveFacts,
    request: &FormatRequest,
) -> Result<Option<String>, FormatError> {
    if facts.read_only {
        return Err(FormatError::ReadOnly);
    }
    let current = facts_of(connection, service, facts.object.clone()).await?;
    if !current.is_same_drive(facts) {
        return Err(FormatError::NotFound);
    }
    if current.read_only {
        return Err(FormatError::ReadOnly);
    }
    if facts.has_file_system {
        let unmounted = call(
            connection,
            service,
            &facts.object,
            FILESYSTEM_INTERFACE,
            "Unmount",
            &(glib::VariantDict::new(None),).to_variant(),
        )
        .await;
        match unmounted {
            Ok(_) => {}
            Err(error) if is_udisks_error(&error, "NotMounted") => {}
            Err(error) => return Err(FormatError::from_call(error)),
        }
    }
    let arguments = (request.file_system.udisks_type(), format_options(request)).to_variant();
    call(
        connection,
        service,
        &facts.object,
        BLOCK_INTERFACE,
        "Format",
        &arguments,
    )
    .await
    .map_err(FormatError::from_call)?;
    let mounted = call(
        connection,
        service,
        &facts.object,
        FILESYSTEM_INTERFACE,
        "Mount",
        &(glib::VariantDict::new(None),).to_variant(),
    )
    .await;
    Ok(mounted
        .ok()
        .and_then(|reply| reply.get::<(String,)>())
        .map(|(path,)| path))
}

/// The options of `Block.Format` (`a{sv}`): the label, fitted again so
/// UDisks never refuses it after the drive was unmounted, zeros first for
/// a full format, the partition's type to match the file system, FAT32
/// itself, as `mkfs.fat` otherwise makes FAT16 on drives up to 512 MB,
/// and for ext4 the new file system's top folder owned by the user, as
/// FAT and NTFS mounts are.
fn format_options(request: &FormatRequest) -> glib::VariantDict {
    let options = glib::VariantDict::new(None);
    options.insert("label", request.file_system.fitted_label(&request.label).as_str());
    options.insert("update-partition-type", true);
    if !request.quick {
        options.insert("erase", "zero");
    }
    match request.file_system {
        FileSystem::Fat32 => options.insert_value("mkfs-args", &vec!["-F", "32"].to_variant()),
        FileSystem::Ext4 => options.insert("take-ownership", true),
        FileSystem::ExFat | FileSystem::Ntfs => {}
    }
    options
}

/// The UDisks object of the block device `device`.
async fn resolve_device(
    connection: &gio::DBusConnection,
    service: &str,
    device: &str,
) -> Result<String, FormatError> {
    let spec = glib::VariantDict::new(None);
    spec.insert("path", device);
    let reply = connection
        .call_future(
            Some(service),
            MANAGER_PATH,
            MANAGER_INTERFACE,
            "ResolveDevice",
            Some(&(spec, glib::VariantDict::new(None)).to_variant()),
            Some(glib::VariantTy::new("(ao)").expect("a valid type")),
            gio::DBusCallFlags::NONE,
            QUERY_TIMEOUT_MS,
        )
        .await
        .map_err(FormatError::Unavailable)?;
    let objects = reply.child_value(0);
    let first = objects.iter().next().ok_or(FormatError::NotFound)?;
    Ok(first.str().unwrap_or_default().to_owned())
}

/// The properties of `interface` on `object`, or `None` when the object
/// does not have it.
async fn properties(
    connection: &gio::DBusConnection,
    service: &str,
    object: &str,
    interface: &str,
) -> Result<Option<glib::VariantDict>, FormatError> {
    let reply = connection
        .call_future(
            Some(service),
            object,
            PROPERTIES_INTERFACE,
            "GetAll",
            Some(&(interface,).to_variant()),
            Some(glib::VariantTy::new("(a{sv})").expect("a valid type")),
            gio::DBusCallFlags::NONE,
            QUERY_TIMEOUT_MS,
        )
        .await;
    match reply {
        Ok(reply) => Ok(reply
            .get::<(glib::VariantDict,)>()
            .map(|(properties,)| properties)),
        Err(error) if is_missing(&error) => Ok(None),
        Err(error) => Err(FormatError::Unavailable(error)),
    }
}

/// Calls `method` of `interface` on `object`, letting polkit ask for a
/// password and waiting as long as the work takes.
async fn call(
    connection: &gio::DBusConnection,
    service: &str,
    object: &str,
    interface: &str,
    method: &str,
    arguments: &glib::Variant,
) -> Result<glib::Variant, glib::Error> {
    connection
        .call_future(
            Some(service),
            object,
            interface,
            method,
            Some(arguments),
            None,
            gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
            NO_TIMEOUT_MS,
        )
        .await
}

/// Whether `error` is UDisks' error `name`.
fn is_udisks_error(error: &glib::Error, name: &str) -> bool {
    gio::DBusError::remote_error(error).is_some_and(|remote| remote.strip_prefix(UDISKS_ERROR) == Some(name))
}

/// Whether `error` says the object or interface does not exist: `GDBus`
/// answers `InvalidArgs` for an interface an object lacks, and the bus
/// `UnknownObject` or `UnknownMethod` for an object it lacks.
fn is_missing(error: &glib::Error) -> bool {
    error.matches(gio::DBusError::InvalidArgs)
        || error.matches(gio::DBusError::UnknownObject)
        || error.matches(gio::DBusError::UnknownInterface)
        || error.matches(gio::DBusError::UnknownMethod)
}

#[cfg(test)]
mod tests;
