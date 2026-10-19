// SPDX-License-Identifier: AGPL-3.0-only
//! Tests of formatting through UDisks, against a fake UDisks exported on
//! the test's private session bus, so no real drive is ever touched.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::Rc;

use super::*;

/// The object of the fake drive's block device.
const SDB1: &str = "/org/freedesktop/UDisks2/block_devices/sdb1";

/// The object of the drive the fake block device is on.
const STICK: &str = "/org/freedesktop/UDisks2/drives/SanDisk_Cruzer_4C530001";

/// The parts of UDisks' manager the client calls.
const MANAGER_XML: &str = r#"<node>
  <interface name="org.freedesktop.UDisks2.Manager">
    <method name="ResolveDevice">
      <arg type="a{sv}" name="devspec" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="ao" name="devices" direction="out"/>
    </method>
    <method name="CanFormat">
      <arg type="s" name="type" direction="in"/>
      <arg type="(bs)" name="available" direction="out"/>
    </method>
  </interface>
</node>"#;

/// The parts of a block device holding a file system the client uses.
const DRIVE_XML: &str = r#"<node>
  <interface name="org.freedesktop.UDisks2.Block">
    <method name="Format">
      <arg type="s" name="type" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
    </method>
    <property type="t" name="Size" access="read"/>
    <property type="s" name="IdLabel" access="read"/>
    <property type="s" name="IdType" access="read"/>
    <property type="b" name="ReadOnly" access="read"/>
    <property type="o" name="Drive" access="read"/>
    <property type="s" name="IdUUID" access="read"/>
  </interface>
  <interface name="org.freedesktop.UDisks2.Filesystem">
    <method name="Unmount">
      <arg type="a{sv}" name="options" direction="in"/>
    </method>
    <method name="Mount">
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="s" name="mount_path" direction="out"/>
    </method>
  </interface>
</node>"#;

/// What the fake answers.
#[derive(Default)]
struct Answers {
    /// The device `ResolveDevice` knows, `/dev/sdb1` by default.
    unknown_device: bool,
    /// The file systems `CanFormat` refuses.
    cannot_format: Vec<&'static str>,
    /// The file systems `CanFormat` answers an error for, as UDisks does
    /// for one it does not know.
    unknown_file_systems: Vec<&'static str>,
    /// `CanFormat` is missing, as in an old UDisks.
    no_can_format: bool,
    /// The UDisks error `Unmount` answers.
    unmount_error: Option<&'static str>,
    /// The UDisks error `Format` answers.
    format_error: Option<&'static str>,
    /// Another drive is at `/dev/sdb1` once the first was read.
    replaced: bool,
}

/// A fake UDisks on its own connection; it records each call as
/// `Interface.Method arguments`.
struct FakeUdisks {
    connection: gio::DBusConnection,
    calls: Rc<RefCell<Vec<String>>>,
}

impl FakeUdisks {
    /// Exports the fake answering as `answers` says.
    fn export(answers: Answers) -> Self {
        let connection = session_connection();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let answers = Rc::new(answers);
        let manager = gio::DBusNodeInfo::for_xml(MANAGER_XML).expect("valid XML");
        let heard = Rc::clone(&calls);
        let answering = Rc::clone(&answers);
        connection
            .register_object(
                MANAGER_PATH,
                &manager.lookup_interface(MANAGER_INTERFACE).expect("manager"),
            )
            .method_call(move |_, _, _, _, method, parameters, invocation| {
                heard.borrow_mut().push(format!("Manager.{method} {parameters}"));
                if method == "ResolveDevice" {
                    let device = parameters
                        .child_value(0)
                        .get::<glib::VariantDict>()
                        .and_then(|spec| spec.lookup::<String>("path").ok().flatten());
                    let found = device.as_deref() == Some("/dev/sdb1") && !answering.unknown_device;
                    let objects: Vec<glib::variant::ObjectPath> = found
                        .then(|| glib::variant::ObjectPath::try_from(SDB1.to_owned()).expect("a path"))
                        .into_iter()
                        .collect();
                    invocation.return_value(Some(&(objects,).to_variant()));
                } else {
                    let system = parameters.get::<(String,)>().unwrap_or_default().0;
                    if answering.no_can_format {
                        invocation.return_dbus_error(UNKNOWN_METHOD_ERROR, "no CanFormat");
                    } else if answering.unknown_file_systems.contains(&system.as_str()) {
                        invocation.return_dbus_error(&format!("{UDISKS_ERROR}NotSupported"), "unknown");
                    } else {
                        let can = !answering.cannot_format.contains(&system.as_str());
                        invocation.return_value(Some(&((can, String::new()),).to_variant()));
                    }
                }
            })
            .build()
            .expect("the manager is exported");
        let drive = gio::DBusNodeInfo::for_xml(DRIVE_XML).expect("valid XML");
        let reads = Rc::new(Cell::new(0));
        for interface in [BLOCK_INTERFACE, FILESYSTEM_INTERFACE] {
            let (reads, swapping) = (Rc::clone(&reads), Rc::clone(&answers));
            let heard = Rc::clone(&calls);
            let answering = Rc::clone(&answers);
            connection
                .register_object(SDB1, &drive.lookup_interface(interface).expect("the interface"))
                .method_call(move |_, _, _, interface, method, parameters, invocation| {
                    let short = interface
                        .unwrap_or_default()
                        .rsplit('.')
                        .next()
                        .unwrap_or_default();
                    heard.borrow_mut().push(format!("{short}.{method} {parameters}"));
                    let error = match method {
                        "Unmount" => answering.unmount_error,
                        "Format" => answering.format_error,
                        _ => None,
                    };
                    match (error, method) {
                        (Some(error), _) => {
                            invocation.return_dbus_error(&format!("{UDISKS_ERROR}{error}"), "refused");
                        }
                        (None, "Mount") => {
                            invocation.return_value(Some(&("/media/demo/USB",).to_variant()));
                        }
                        _ => invocation.return_value(None),
                    }
                })
                .property(move |_, _, _, _, property| {
                    let swapped = swapping.replaced && reads.get() > 0;
                    match property {
                        "Size" => 16_000_000_000_u64.to_variant(),
                        "IdLabel" => "OLD".to_variant(),
                        "IdType" => "vfat".to_variant(),
                        "Drive" => glib::variant::ObjectPath::try_from(STICK.to_owned())
                            .expect("a path")
                            .to_variant(),
                        "IdUUID" => {
                            reads.set(reads.get() + 1);
                            if swapped { "9C21-77F0" } else { "4E2A-1B3C" }.to_variant()
                        }
                        _ => false.to_variant(),
                    }
                })
                .build()
                .expect("the drive is exported");
        }
        Self { connection, calls }
    }

    /// The fake's bus name.
    fn name(&self) -> String {
        self.connection.unique_name().expect("a unique name").to_string()
    }

    /// The calls made of the drive itself, without the manager's.
    fn drive_calls(&self) -> Vec<String> {
        let calls = self.calls.borrow();
        calls
            .iter()
            .filter(|call| !call.starts_with("Manager."))
            .cloned()
            .collect()
    }
}

impl Drop for FakeUdisks {
    fn drop(&mut self) {
        let _ = self.connection.close_sync(gio::Cancellable::NONE);
    }
}

/// A new connection to the test's private session bus.
fn session_connection() -> gio::DBusConnection {
    let address = gio::dbus_address_get_for_bus_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .expect("the tests run on a private session bus");
    let flags =
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION;
    gio::DBusConnection::for_address_sync(&address, flags, None, gio::Cancellable::NONE)
        .expect("connect to the session bus")
}

/// Runs `test` with a fake UDisks answering as `answers` says, on a main
/// context of its own that also serves the fake.
fn with_udisks<F, T>(answers: Answers, test: impl FnOnce(gio::DBusConnection, FakeUdisks) -> F) -> T
where
    F: Future<Output = T>,
{
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let fake = FakeUdisks::export(answers);
            context.block_on(test(session_connection(), fake))
        })
        .expect("the context is free")
}

fn request(file_system: FileSystem, quick: bool) -> FormatRequest {
    FormatRequest {
        file_system,
        label: "PHOTOS".to_owned(),
        quick,
    }
}

/// parity: DEV-012
#[test]
fn labels_fit_each_file_system() {
    assert_eq!(
        FileSystem::Fat32.fitted_label("  My Holiday Photos "),
        "MY HOLIDAY"
    );
    assert_eq!(FileSystem::ExFat.fitted_label("My Holiday Photos"), "My Holiday");
    assert_eq!(FileSystem::ExFat.fitted_label("ABCDEFGHIJK"), "ABCDEFGHIJK");
    assert_eq!(FileSystem::ExFat.fitted_label("ABCDEFGHIJKL"), "ABCDEFGHIJK");
    // Each takes two UTF-16 code units, so 5 fit in exFAT's 11.
    assert_eq!(FileSystem::ExFat.fitted_label("😀😀😀😀😀😀"), "😀😀😀😀😀");
    assert_eq!(FileSystem::Fat32.fitted_label("ääääää"), "ÄÄÄÄÄ");
    // FAT refuses these; Windows drops them the same way.
    assert_eq!(FileSystem::Fat32.fitted_label("Backup:2026"), "BACKUP2026");
    assert_eq!(FileSystem::Fat32.fitted_label("a*?.,;:/\\|+=<>[]\"b"), "AB");
    assert_eq!(FileSystem::ExFat.fitted_label("Backup:2026"), "Backup:2026");
    assert_eq!(
        FileSystem::Ntfs.fitted_label("My Holiday Photos"),
        "My Holiday Photos"
    );
    assert_eq!(FileSystem::Ext4.fitted_label("ÄÄÄÄÄÄÄÄÄÄ"), "ÄÄÄÄÄÄÄÄ");
    assert_eq!(FileSystem::Ext4.fitted_label("ÄÄÄÄÄÄÄÄÄÄ").len(), 16);
    for system in FileSystem::ALL {
        assert_eq!(FileSystem::from_udisks_type(system.udisks_type()), Some(system));
    }
    assert_eq!(FileSystem::from_udisks_type("btrfs"), None);
}

/// The drive's own file system comes first, as in Windows; otherwise
/// exFAT above 32 GB and FAT32 below.
///
/// parity: DEV-012
#[test]
fn the_default_is_the_drives_own_file_system_or_one_for_its_size() {
    let all = FileSystem::ALL;
    let gigabytes = |count: u64| count * 1024 * 1024 * 1024;

    assert_eq!(
        default_file_system(Some(FileSystem::Ntfs), gigabytes(8), &all),
        Some(FileSystem::Ntfs)
    );
    assert_eq!(
        default_file_system(None, gigabytes(8), &all),
        Some(FileSystem::Fat32)
    );
    assert_eq!(
        default_file_system(None, gigabytes(64), &all),
        Some(FileSystem::ExFat)
    );
    assert_eq!(
        default_file_system(Some(FileSystem::Ntfs), gigabytes(64), &[FileSystem::Fat32]),
        Some(FileSystem::Fat32)
    );
    assert_eq!(default_file_system(None, gigabytes(64), &[]), None);
}

/// Quick format only makes the file system; a full format overwrites the
/// drive with zeros first. ext4's top folder belongs to the user.
///
/// parity: DEV-012
#[test]
fn the_options_carry_the_label_and_how_thoroughly_to_format() {
    let quick = format_options(&request(FileSystem::ExFat, true));
    let full = format_options(&request(FileSystem::Ext4, false));

    assert_eq!(
        quick.lookup::<String>("label").ok().flatten().as_deref(),
        Some("PHOTOS")
    );
    assert_eq!(
        quick.lookup::<bool>("update-partition-type").ok().flatten(),
        Some(true)
    );
    assert!(!quick.contains("erase"));
    assert!(!quick.contains("take-ownership"));
    assert_eq!(
        full.lookup::<String>("erase").ok().flatten().as_deref(),
        Some("zero")
    );
    assert_eq!(full.lookup::<bool>("take-ownership").ok().flatten(), Some(true));
    let arguments = ("exfat", quick).to_variant();
    assert_eq!(arguments.type_().as_str(), "(sa{sv})");
}

/// parity: DEV-012
#[test]
fn a_drive_is_read_unmounted_formatted_and_mounted_again() {
    let (facts, mounted, calls) = with_udisks(Answers::default(), |connection, fake| async move {
        let name = fake.name();
        let facts = drive_facts(&connection, &name, "/dev/sdb1")
            .await
            .expect("the drive");
        let mounted = format_drive(&connection, &name, &facts, &request(FileSystem::ExFat, false))
            .await
            .expect("formatted");
        (facts, mounted, fake.drive_calls())
    });

    assert_eq!(
        facts,
        DriveFacts {
            object: SDB1.to_owned(),
            drive: STICK.to_owned(),
            uuid: "4E2A-1B3C".to_owned(),
            size: 16_000_000_000,
            label: "OLD".to_owned(),
            file_system: Some(FileSystem::Fat32),
            read_only: false,
            has_file_system: true,
        }
    );
    assert_eq!(mounted.as_deref(), Some("/media/demo/USB"));
    let methods: Vec<&str> = calls
        .iter()
        .map(|call| call.split(' ').next().unwrap_or_default())
        .collect();
    assert_eq!(
        methods,
        ["Filesystem.Unmount", "Block.Format", "Filesystem.Mount"]
    );
    assert!(calls[1].starts_with("Block.Format ('exfat',"), "{calls:?}");
    assert!(calls[1].contains("'erase': <'zero'>"), "{calls:?}");
    assert!(calls[1].contains("'label': <'PHOTOS'>"), "{calls:?}");
}

/// A label too long for the file system is cut before UDisks sees it,
/// so UDisks never refuses it once the drive is unmounted.
///
/// parity: DEV-012
#[test]
fn a_label_too_long_is_cut_before_formatting() {
    let long = FormatRequest {
        label: "Holiday photos".to_owned(),
        ..request(FileSystem::ExFat, true)
    };

    let options = format_options(&long);

    assert_eq!(
        options.lookup::<String>("label").ok().flatten().as_deref(),
        Some("Holiday pho")
    );
}

/// Another drive plugged in at the same device while the dialog was open
/// is not touched: the format stops before unmounting anything.
///
/// parity: DEV-012
#[test]
fn a_drive_swapped_in_at_the_same_device_is_not_formatted() {
    let answers = Answers {
        replaced: true,
        ..Answers::default()
    };
    let (formatted, calls) = with_udisks(answers, |connection, fake| async move {
        let name = fake.name();
        let facts = drive_facts(&connection, &name, "/dev/sdb1")
            .await
            .expect("the drive");
        let formatted = format_drive(&connection, &name, &facts, &request(FileSystem::ExFat, true)).await;
        (formatted, fake.drive_calls())
    });

    assert!(matches!(formatted, Err(FormatError::NotFound)), "{formatted:?}");
    assert!(calls.is_empty(), "nothing unmounted or formatted: {calls:?}");
}

/// parity: DEV-012
#[test]
fn only_the_file_systems_udisks_can_make_are_offered() {
    let answers = Answers {
        cannot_format: vec!["ntfs", "exfat"],
        ..Answers::default()
    };
    let available = with_udisks(answers, |connection, fake| async move {
        available_file_systems(&connection, &fake.name(), 16_000_000_000).await
    });

    assert_eq!(available, [FileSystem::Fat32, FileSystem::Ext4]);
}

/// A file system UDisks answers an error for is not offered, so it is not
/// found out only after the drive was unmounted; a UDisks without
/// `CanFormat` offers them all.
///
/// parity: DEV-012
#[test]
fn a_file_system_udisks_does_not_know_is_not_offered() {
    let unknown = Answers {
        unknown_file_systems: vec!["exfat"],
        ..Answers::default()
    };
    let available = with_udisks(unknown, |connection, fake| async move {
        available_file_systems(&connection, &fake.name(), 16_000_000_000).await
    });
    let old = Answers {
        no_can_format: true,
        ..Answers::default()
    };
    let all = with_udisks(old, |connection, fake| async move {
        available_file_systems(&connection, &fake.name(), 16_000_000_000).await
    });

    assert_eq!(available, [FileSystem::Fat32, FileSystem::Ntfs, FileSystem::Ext4]);
    assert_eq!(all, FileSystem::ALL);
}

/// A drive too small for a FAT32 within its specification is not offered
/// it.
///
/// parity: DEV-012
#[test]
fn fat32_is_not_offered_for_a_tiny_drive() {
    let available = with_udisks(Answers::default(), |connection, fake| async move {
        available_file_systems(&connection, &fake.name(), 32 * 1024 * 1024).await
    });

    assert_eq!(available, [FileSystem::ExFat, FileSystem::Ntfs, FileSystem::Ext4]);
    assert!(FileSystem::Fat32.fits(64 * 1024 * 1024));
}

/// FAT32 is asked for by name, so `mkfs.fat` does not make FAT16 on a
/// drive up to 512 MB; the other file systems get no extra arguments.
///
/// parity: DEV-012
#[test]
fn fat32_is_made_as_fat32_on_any_drive() {
    let arguments = |file_system| {
        format_options(&request(file_system, true))
            .lookup::<Vec<String>>("mkfs-args")
            .ok()
            .flatten()
    };

    assert_eq!(
        arguments(FileSystem::Fat32),
        Some(vec!["-F".to_owned(), "32".to_owned()])
    );
    assert_eq!(arguments(FileSystem::ExFat), None);
    assert_eq!(arguments(FileSystem::Ext4), None);
}

/// A drive in use is not formatted, and a dismissed password prompt is
/// a cancellation rather than a failure.
///
/// parity: DEV-012
#[test]
fn a_busy_drive_or_a_dismissed_prompt_stops_the_format() {
    let busy = Answers {
        unmount_error: Some("DeviceBusy"),
        ..Answers::default()
    };
    let (busy, busy_calls) = with_udisks(busy, |connection, fake| async move {
        let name = fake.name();
        let facts = drive_facts(&connection, &name, "/dev/sdb1")
            .await
            .expect("the drive");
        let formatted = format_drive(&connection, &name, &facts, &request(FileSystem::Fat32, true)).await;
        (formatted, fake.drive_calls())
    });
    let dismissed = Answers {
        format_error: Some("NotAuthorizedDismissed"),
        ..Answers::default()
    };
    let dismissed = with_udisks(dismissed, |connection, fake| async move {
        let name = fake.name();
        let facts = drive_facts(&connection, &name, "/dev/sdb1")
            .await
            .expect("the drive");
        format_drive(&connection, &name, &facts, &request(FileSystem::Fat32, true)).await
    });
    let refused = Answers {
        format_error: Some("Failed"),
        ..Answers::default()
    };
    let refused = with_udisks(refused, |connection, fake| async move {
        let name = fake.name();
        let facts = drive_facts(&connection, &name, "/dev/sdb1")
            .await
            .expect("the drive");
        format_drive(&connection, &name, &facts, &request(FileSystem::Fat32, true)).await
    });

    assert!(matches!(busy, Err(FormatError::Busy)), "{busy:?}");
    assert_eq!(
        busy_calls.len(),
        1,
        "nothing after the refused unmount: {busy_calls:?}"
    );
    assert!(matches!(dismissed, Err(FormatError::Cancelled)), "{dismissed:?}");
    assert_eq!(
        refused.map_err(|error| error.to_string()),
        Err("Could not format the drive: refused".to_owned())
    );
}

/// parity: DEV-012
#[test]
fn a_removed_or_write_protected_drive_is_not_formatted() {
    let gone = Answers {
        unknown_device: true,
        ..Answers::default()
    };
    let gone = with_udisks(gone, |connection, fake| async move {
        drive_facts(&connection, &fake.name(), "/dev/sdb1").await
    });
    let (protected, calls) = with_udisks(Answers::default(), |connection, fake| async move {
        let name = fake.name();
        let mut facts = drive_facts(&connection, &name, "/dev/sdb1")
            .await
            .expect("the drive");
        facts.read_only = true;
        let formatted = format_drive(&connection, &name, &facts, &request(FileSystem::Fat32, true)).await;
        (formatted, fake.drive_calls())
    });

    assert!(matches!(gone, Err(FormatError::NotFound)), "{gone:?}");
    assert!(matches!(protected, Err(FormatError::ReadOnly)), "{protected:?}");
    assert!(calls.is_empty(), "{calls:?}");
}
