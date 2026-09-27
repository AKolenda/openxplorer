// SPDX-License-Identifier: AGPL-3.0-only
//! A simulated MTP device behind real `mtp://` URIs: the counterpart of
//! `FakeGFile` in `desktop/tests/test_device_staging.py`.
//!
//! GIO lets a process add its own handler for a URI scheme. Every
//! `mtp://fake-device-N/...` URI of a [`FakeDevice`] resolves to a
//! [`DeviceFile`], a `gio::File` answered from the device's item list;
//! every other `mtp://` URI still reaches the real `GVfs` backend. The
//! production `GioNode` therefore runs unchanged, and each test sees which
//! device calls it made.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use gio::prelude::*;
use gio::subclass::prelude::*;
use ox_core::gio_node::GioNode;
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};

/// The folder every test works in, below the device root.
const DOWNLOAD_FOLDER: &str = "Internal%20shared%20storage/Download";

/// Characters escaped in one path segment of a URI, as GIO escapes them.
const SEGMENT_ESCAPES: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}');

/// `name` escaped for use as one path segment of a URI.
fn escape_segment(name: &str) -> String {
    utf8_percent_encode(name, SEGMENT_ESCAPES).to_string()
}

/// One call that changes the device, in the order it was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceCall {
    /// `set_display_name` (MTP `SetObjectPropValue`) of `from` to `name`.
    Rename { from: String, name: String },
    /// `g_file_move` (MTP `MoveObject`) with its flags.
    Move {
        from: String,
        to: String,
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

/// The items of one simulated device and the calls it received.
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
        devices()
            .lock()
            .expect("device registry")
            .insert(authority, Arc::clone(&device));
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

    fn add(&self, relative: &str, file_type: gio::FileType) {
        let mut items = self.items.lock().expect("items");
        let mut uri = self.root.trim_end_matches('/').to_owned();
        let path = format!("{DOWNLOAD_FOLDER}/{relative}");
        let mut parts = path.split('/').filter(|part| !part.is_empty()).peekable();
        while let Some(part) = parts.next() {
            uri = format!("{uri}/{part}");
            let is_last = parts.peek().is_none();
            let part_type = if is_last {
                file_type
            } else {
                gio::FileType::Directory
            };
            items.entry(uri.clone()).or_insert(part_type);
        }
    }

    fn file_type(&self, uri: &str) -> Option<gio::FileType> {
        self.items.lock().expect("items").get(uri).copied()
    }

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
    let device = devices()
        .lock()
        .expect("device registry")
        .get(authority)
        .cloned()?;
    Some(DeviceFile::new(uri.trim_end_matches('/'), device).upcast())
}

glib::wrapper! {
    /// One item on a [`FakeDevice`], addressed by its URI.
    pub struct DeviceFile(ObjectSubclass<imp::DeviceFile>) @implements gio::File;
}

impl DeviceFile {
    fn new(uri: &str, device: Arc<FakeDevice>) -> Self {
        let file: Self = glib::Object::new();
        file.imp()
            .uri
            .set(uri.to_owned())
            .expect("a new file has no URI yet");
        file.imp()
            .device
            .set(device)
            .expect("a new file has no device yet");
        file
    }
}

/// The URI of the folder that holds `uri`, or `None` at the device root.
fn parent_uri(uri: &str) -> Option<&str> {
    let after_scheme = uri.strip_prefix("mtp://")?;
    let (_, path) = after_scheme.split_once('/')?;
    if path.is_empty() {
        return None;
    }
    let (parent, _) = uri.rsplit_once('/')?;
    Some(parent)
}

/// A `GIO` error with `code`.
fn device_error(code: gio::IOErrorEnum, message: &str) -> glib::Error {
    glib::Error::new(code, message)
}

mod imp {
    use super::*;

    /// The state behind a [`super::DeviceFile`].
    #[derive(Debug, Default)]
    pub struct DeviceFile {
        pub(super) uri: OnceLock<String>,
        pub(super) device: OnceLock<Arc<FakeDevice>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DeviceFile {
        const NAME: &'static str = "OxTestDeviceFile";
        type Type = super::DeviceFile;
        type Interfaces = (gio::File,);
    }

    impl ObjectImpl for DeviceFile {}

    impl DeviceFile {
        fn location(&self) -> &str {
            self.uri.get().expect("set at construction")
        }

        fn device(&self) -> &Arc<FakeDevice> {
            self.device.get().expect("set at construction")
        }

        fn sibling(&self, name: &str) -> String {
            let parent = parent_uri(self.location()).expect("items below the root have a folder");
            let escaped = escape_segment(name);
            format!("{parent}/{escaped}")
        }

        fn info(&self, file_type: gio::FileType) -> gio::FileInfo {
            let info = gio::FileInfo::new();
            info.set_name(self.basename().unwrap_or_default());
            info.set_file_type(file_type);
            info.set_size(0);
            info
        }
    }

    impl FileImpl for DeviceFile {
        fn dup(&self) -> gio::File {
            super::DeviceFile::new(self.location(), Arc::clone(self.device())).upcast()
        }

        fn hash(&self) -> u32 {
            // The djb2 string hash that `g_str_hash` uses.
            self.location().bytes().fold(5381_u32, |hash, byte| {
                hash.wrapping_mul(33).wrapping_add(u32::from(byte))
            })
        }

        fn equal(&self, other: &gio::File) -> bool {
            other.uri() == self.location()
        }

        fn is_native(&self) -> bool {
            false
        }

        fn has_uri_scheme(&self, scheme: &str) -> bool {
            scheme.eq_ignore_ascii_case("mtp")
        }

        fn uri_scheme(&self) -> Option<String> {
            Some("mtp".to_owned())
        }

        fn basename(&self) -> Option<PathBuf> {
            let (_, escaped) = self.location().rsplit_once('/')?;
            let bytes: Vec<u8> = percent_decode_str(escaped).collect();
            Some(PathBuf::from(OsStr::from_bytes(&bytes)))
        }

        fn path(&self) -> Option<PathBuf> {
            None
        }

        fn uri(&self) -> String {
            self.location().to_owned()
        }

        fn parse_name(&self) -> String {
            self.location().to_owned()
        }

        fn parent(&self) -> Option<gio::File> {
            let parent = parent_uri(self.location())?;
            Some(super::DeviceFile::new(parent, Arc::clone(self.device())).upcast())
        }

        fn resolve_relative_path(&self, relative_path: impl AsRef<std::path::Path>) -> gio::File {
            let name = relative_path.as_ref().to_string_lossy();
            let escaped = escape_segment(&name);
            let child = format!("{}/{escaped}", self.location());
            super::DeviceFile::new(&child, Arc::clone(self.device())).upcast()
        }

        fn query_info(
            &self,
            _attributes: &str,
            _flags: gio::FileQueryInfoFlags,
            _cancellable: Option<&gio::Cancellable>,
        ) -> Result<gio::FileInfo, glib::Error> {
            match self.device().file_type(self.location()) {
                Some(file_type) => Ok(self.info(file_type)),
                None => Err(device_error(
                    gio::IOErrorEnum::NotFound,
                    "No such file or directory",
                )),
            }
        }

        fn enumerate_children(
            &self,
            _attributes: &str,
            _flags: gio::FileQueryInfoFlags,
            _cancellable: Option<&gio::Cancellable>,
        ) -> Result<gio::FileEnumerator, glib::Error> {
            if self.device().file_type(self.location()) != Some(gio::FileType::Directory) {
                return Err(device_error(gio::IOErrorEnum::NotDirectory, "Not a directory"));
            }
            let infos = self
                .device()
                .children_of(self.location())
                .into_iter()
                .map(|name| {
                    let info = gio::FileInfo::new();
                    info.set_name(percent_decode_str(&name).decode_utf8_lossy().as_ref());
                    info
                })
                .collect();
            Ok(super::DeviceListing::new(&self.obj(), infos).upcast())
        }

        fn set_display_name(
            &self,
            name: &str,
            _cancellable: Option<&gio::Cancellable>,
        ) -> Result<gio::File, glib::Error> {
            let device = self.device();
            device.record(DeviceCall::Rename {
                from: self.location().to_owned(),
                name: name.to_owned(),
            });
            let renamed = self.sibling(name);
            let answer = *device.rename_answer.lock().expect("rename answer");
            if answer == RenameAnswer::NameTakenMeanwhile {
                device
                    .items
                    .lock()
                    .expect("items")
                    .insert(renamed.clone(), gio::FileType::Regular);
            }
            if answer == RenameAnswer::Refused || device.file_type(&renamed).is_some() {
                return Err(device_error(
                    gio::IOErrorEnum::Failed,
                    "libmtp error: could not rename",
                ));
            }
            device.relocate(self.location(), &renamed);
            if answer == RenameAnswer::DoneButReportsError {
                return Err(device_error(
                    gio::IOErrorEnum::Cancelled,
                    "Operation was cancelled",
                ));
            }
            Ok(super::DeviceFile::new(&renamed, Arc::clone(device)).upcast())
        }

        fn delete(&self, _cancellable: Option<&gio::Cancellable>) -> Result<(), glib::Error> {
            let device = self.device();
            device.record(DeviceCall::Delete(self.location().to_owned()));
            if device.file_type(self.location()).is_none() {
                return Err(device_error(
                    gio::IOErrorEnum::NotFound,
                    "No such file or directory",
                ));
            }
            if !device.children_of(self.location()).is_empty() {
                return Err(device_error(gio::IOErrorEnum::NotEmpty, "Directory not empty"));
            }
            device.items.lock().expect("items").remove(self.location());
            Ok(())
        }

        fn move_(
            source: &gio::File,
            destination: &gio::File,
            flags: gio::FileCopyFlags,
            _cancellable: Option<&gio::Cancellable>,
            _progress_callback: Option<&mut dyn FnMut(i64, i64)>,
        ) -> Result<(), glib::Error> {
            let source = source.downcast_ref::<super::DeviceFile>().expect("a device file");
            let device = source.imp().device();
            let (from, to) = (source.uri().to_string(), destination.uri().to_string());
            device.record(DeviceCall::Move {
                from: from.clone(),
                to: to.clone(),
                flags,
            });
            if device.file_type(&to).is_some() {
                return Err(device_error(gio::IOErrorEnum::Exists, "Target file exists"));
            }
            device.relocate(&from, &to);
            Ok(())
        }
    }

    /// The state behind a [`super::DeviceListing`].
    #[derive(Debug, Default)]
    pub struct DeviceListing {
        pub(super) remaining: Mutex<Vec<gio::FileInfo>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DeviceListing {
        const NAME: &'static str = "OxTestDeviceListing";
        type Type = super::DeviceListing;
        type ParentType = gio::FileEnumerator;
    }

    impl ObjectImpl for DeviceListing {}

    impl FileEnumeratorImpl for DeviceListing {
        fn next_file(
            &self,
            _cancellable: Option<&gio::Cancellable>,
        ) -> Result<Option<gio::FileInfo>, glib::Error> {
            let mut remaining = self.remaining.lock().expect("listing");
            Ok(remaining.pop())
        }

        fn close(&self, _cancellable: Option<&gio::Cancellable>) -> (bool, Option<glib::Error>) {
            (true, None)
        }
    }
}

glib::wrapper! {
    /// The listing of one folder on a [`FakeDevice`].
    pub struct DeviceListing(ObjectSubclass<imp::DeviceListing>) @extends gio::FileEnumerator;
}

impl DeviceListing {
    fn new(folder: &DeviceFile, mut infos: Vec<gio::FileInfo>) -> Self {
        // Items are handed out from the end; reverse to list in name order.
        infos.reverse();
        let listing: Self = glib::Object::builder()
            .property("container", folder.upcast_ref::<gio::File>())
            .build();
        *listing.imp().remaining.lock().expect("listing") = infos;
        listing
    }
}
