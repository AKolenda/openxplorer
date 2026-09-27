// SPDX-License-Identifier: AGPL-3.0-only
//! A simulated MTP device behind real `mtp://` URIs: the counterpart of
//! `FakeGFile` in `desktop/tests/test_device_staging.py`.
//!
//! GIO lets a process add its own handler for a URI scheme. Every
//! `mtp://fake-device-N/...` URI of a [`FakeDevice`] resolves to a
//! `gio::File` answered by that device (see `file.rs`); every other `mtp://`
//! URI still reaches the real `GVfs` backend. The production `GioNode`
//! therefore runs unchanged, and each test sees which device calls it made.

mod file;

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use gio::prelude::*;
use ox_core::gio_node::GioNode;

use file::DeviceFile;

/// The folder every test works in, below the device root.
const DOWNLOAD_FOLDER: &str = "Internal%20shared%20storage/Download";

/// One call that changes the device, in the order it was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceCall {
    /// `set_display_name` (MTP `SetObjectPropValue`) of `from` to `name`.
    Rename {
        /// The escaped URI of the renamed item.
        from: String,
        /// The new name, as text.
        name: String,
    },
    /// `g_file_move` (MTP `MoveObject`) with its flags.
    Move {
        /// The escaped URI of the moved item.
        from: String,
        /// The escaped URI it was moved to.
        to: String,
        /// The flags of the move, which must never allow a copy fallback.
        flags: gio::FileCopyFlags,
    },
    /// `g_file_delete` (MTP `DeleteObject`).
    Delete(String),
}

/// How the device answers a rename.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RenameAnswer {
    /// The rename works.
    #[default]
    Done,
    /// The device renames, then reports an error, as when the client's
    /// wait timed out or was cancelled.
    DoneButReportsError,
    /// Another program takes the new name first, so the device refuses.
    NameTakenMeanwhile,
    /// The device refuses and changes nothing, as when it disconnects.
    Refused,
}

/// The items of one simulated device and the calls it received. Items are
/// keyed by their escaped URI.
#[derive(Debug)]
pub struct FakeDevice {
    root: String,
    items: Mutex<BTreeMap<String, gio::FileType>>,
    calls: Mutex<Vec<DeviceCall>>,
    rename_answer: Mutex<RenameAnswer>,
}

impl FakeDevice {
    /// A new device holding only the folder `Internal shared storage/Download`.
    pub fn new() -> Arc<Self> {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
        register_scheme_handler();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let authority = format!("fake-device-{id}");
        let device = Arc::new(Self {
            root: format!("mtp://{authority}/"),
            items: Mutex::default(),
            calls: Mutex::default(),
            rename_answer: Mutex::default(),
        });
        device.add_folder("");
        let mut registry = devices().lock().expect("device registry");
        registry.insert(authority, Arc::clone(&device));
        device
    }

    /// The URI of `relative` below the Download folder (`""` for the folder).
    pub fn uri(&self, relative: &str) -> String {
        let folder = format!("{}{DOWNLOAD_FOLDER}", self.root);
        if relative.is_empty() {
            folder
        } else {
            format!("{folder}/{relative}")
        }
    }

    /// The production adapter for `relative` below the Download folder.
    pub fn node(&self, relative: &str) -> GioNode {
        GioNode::new(&self.uri(relative))
    }

    /// Adds a file (and any missing parent folders).
    pub fn add_file(&self, relative: &str) {
        self.add(relative, gio::FileType::Regular);
    }

    /// Adds a folder (and any missing parent folders).
    pub fn add_folder(&self, relative: &str) {
        self.add(relative, gio::FileType::Directory);
    }

    /// Sets how the device answers every later rename.
    pub fn answer_renames(&self, answer: RenameAnswer) {
        *self.rename_answer.lock().expect("rename answer") = answer;
    }

    /// The calls that changed the device, in order.
    pub fn calls(&self) -> Vec<DeviceCall> {
        self.calls.lock().expect("call log").clone()
    }

    /// The items below the Download folder, relative to it, sorted.
    pub fn items(&self) -> Vec<String> {
        let prefix = format!("{}/", self.uri(""));
        let items = self.items.lock().expect("items");
        items
            .keys()
            .filter_map(|uri| uri.strip_prefix(&prefix))
            .map(str::to_owned)
            .collect()
    }

    /// Adds the item `relative` of `file_type`, and any missing parent
    /// folders.
    fn add(&self, relative: &str, file_type: gio::FileType) {
        let mut items = self.items.lock().expect("items");
        let mut uri = self.root.trim_end_matches('/').to_owned();
        let path = format!("{DOWNLOAD_FOLDER}/{relative}");
        let mut parts = path.split('/').filter(|part| !part.is_empty()).peekable();
        while let Some(part) = parts.next() {
            uri = format!("{uri}/{part}");
            let part_type = if parts.peek().is_none() {
                file_type
            } else {
                gio::FileType::Directory
            };
            items.entry(uri.clone()).or_insert(part_type);
        }
    }

    /// What the item at `uri` is, or `None` when it does not exist.
    fn file_type(&self, uri: &str) -> Option<gio::FileType> {
        self.items.lock().expect("items").get(uri).copied()
    }

    /// The escaped names of the items directly inside `folder`, sorted.
    fn children_of(&self, folder: &str) -> Vec<String> {
        let prefix = format!("{folder}/");
        let items = self.items.lock().expect("items");
        items
            .keys()
            .filter_map(|uri| uri.strip_prefix(&prefix))
            .filter(|rest| !rest.contains('/'))
            .map(str::to_owned)
            .collect()
    }

    /// MTP `SetObjectPropValue`: renames `from` to the sibling URI
    /// `renamed`, answering as [`FakeDevice::answer_renames`] set. A taken
    /// name is refused, as devices do.
    fn rename(&self, from: &str, renamed: &str, name: &str) -> Result<(), glib::Error> {
        self.record(DeviceCall::Rename {
            from: from.to_owned(),
            name: name.to_owned(),
        });
        let answer = *self.rename_answer.lock().expect("rename answer");
        if answer == RenameAnswer::NameTakenMeanwhile {
            let mut items = self.items.lock().expect("items");
            items.insert(renamed.to_owned(), gio::FileType::Regular);
        }
        if answer == RenameAnswer::Refused || self.file_type(renamed).is_some() {
            return Err(device_error(
                gio::IOErrorEnum::Failed,
                "libmtp error: could not rename",
            ));
        }
        self.relocate(from, renamed);
        if answer == RenameAnswer::DoneButReportsError {
            return Err(device_error(
                gio::IOErrorEnum::Cancelled,
                "Operation was cancelled",
            ));
        }
        Ok(())
    }

    /// MTP `MoveObject`: never overwrites.
    fn move_item(&self, from: &str, to: &str, flags: gio::FileCopyFlags) -> Result<(), glib::Error> {
        self.record(DeviceCall::Move {
            from: from.to_owned(),
            to: to.to_owned(),
            flags,
        });
        if self.file_type(to).is_some() {
            return Err(device_error(gio::IOErrorEnum::Exists, "Target file exists"));
        }
        self.relocate(from, to);
        Ok(())
    }

    /// MTP `DeleteObject`: removes a file or an empty folder.
    fn delete(&self, uri: &str) -> Result<(), glib::Error> {
        self.record(DeviceCall::Delete(uri.to_owned()));
        if self.file_type(uri).is_none() {
            return Err(device_error(
                gio::IOErrorEnum::NotFound,
                "No such file or directory",
            ));
        }
        if !self.children_of(uri).is_empty() {
            return Err(device_error(gio::IOErrorEnum::NotEmpty, "Directory not empty"));
        }
        self.items.lock().expect("items").remove(uri);
        Ok(())
    }

    /// Adds `call` to the log.
    fn record(&self, call: DeviceCall) {
        self.calls.lock().expect("call log").push(call);
    }

    /// Renames `from` and everything below it to `to`.
    fn relocate(&self, from: &str, to: &str) {
        let mut items = self.items.lock().expect("items");
        let inside = format!("{from}/");
        let moved: Vec<String> = items
            .keys()
            .filter(|uri| *uri == from || uri.starts_with(&inside))
            .cloned()
            .collect();
        for old in moved {
            let file_type = items.remove(&old).expect("listed above");
            let new = format!("{to}{}", &old[from.len()..]);
            items.insert(new, file_type);
        }
    }
}

/// A GIO error the device reports.
fn device_error(code: gio::IOErrorEnum, message: &str) -> glib::Error {
    glib::Error::new(code, message)
}

/// The devices by URI authority, for the scheme handler.
fn devices() -> &'static Mutex<HashMap<String, Arc<FakeDevice>>> {
    static DEVICES: OnceLock<Mutex<HashMap<String, Arc<FakeDevice>>>> = OnceLock::new();
    DEVICES.get_or_init(Mutex::default)
}

/// Routes the `mtp://` URIs of fake devices to [`DeviceFile`], once per
/// test process.
fn register_scheme_handler() {
    static REGISTERED: OnceLock<()> = OnceLock::new();
    REGISTERED.get_or_init(|| {
        let registered = gio::Vfs::default().register_uri_scheme(
            "mtp",
            Some(Box::new(|_vfs, uri| device_file_for(uri))),
            Some(Box::new(|_vfs, parse_name| device_file_for(parse_name))),
        );
        assert!(registered, "the mtp scheme handler could not be registered");
    });
}

/// The fake file for `uri`, or `None` to let the real backend handle it.
fn device_file_for(uri: &str) -> Option<gio::File> {
    let authority = uri.strip_prefix("mtp://")?.split('/').next()?;
    let registry = devices().lock().expect("device registry");
    let device = registry.get(authority)?;
    Some(DeviceFile::new(uri.trim_end_matches('/'), Arc::clone(device)).upcast())
}
