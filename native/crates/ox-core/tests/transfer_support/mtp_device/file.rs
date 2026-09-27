// SPDX-License-Identifier: AGPL-3.0-only
//! The `gio::File` and `gio::FileEnumerator` of a [`FakeDevice`]: thin
//! `GObject` types that answer GIO's calls from the device model.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use gio::prelude::*;
use gio::subclass::prelude::*;
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};

use super::{device_error, FakeDevice};

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

glib::wrapper! {
    /// One item on a [`FakeDevice`], addressed by its URI.
    pub struct DeviceFile(ObjectSubclass<imp::DeviceFile>) @implements gio::File;
}

impl DeviceFile {
    /// The item at `uri` on `device`.
    pub(super) fn new(uri: &str, device: Arc<FakeDevice>) -> Self {
        let file: Self = glib::Object::new();
        let state = file.imp();
        state.uri.set(uri.to_owned()).expect("a new file has no URI yet");
        state.device.set(device).expect("a new file has no device yet");
        file
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

/// `name` escaped for use as one path segment of a URI.
fn escape_segment(name: &str) -> String {
    utf8_percent_encode(name, SEGMENT_ESCAPES).to_string()
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

/// A listing entry named `escaped_name` (decoded for GIO).
fn listing_entry(escaped_name: &str) -> gio::FileInfo {
    let info = gio::FileInfo::new();
    let name = percent_decode_str(escaped_name).decode_utf8_lossy();
    info.set_name(name.as_ref());
    info
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

        fn file_at(&self, uri: &str) -> gio::File {
            super::DeviceFile::new(uri, Arc::clone(self.device())).upcast()
        }

        /// The info GIO's required getters need: name, type and size.
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
            self.file_at(self.location())
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
            Some(self.file_at(parent))
        }

        fn resolve_relative_path(&self, relative_path: impl AsRef<Path>) -> gio::File {
            let name = relative_path.as_ref().to_string_lossy();
            let child = format!("{}/{}", self.location(), escape_segment(&name));
            self.file_at(&child)
        }

        fn query_info(
            &self,
            _attributes: &str,
            _flags: gio::FileQueryInfoFlags,
            _cancellable: Option<&gio::Cancellable>,
        ) -> Result<gio::FileInfo, glib::Error> {
            let Some(file_type) = self.device().file_type(self.location()) else {
                return Err(device_error(
                    gio::IOErrorEnum::NotFound,
                    "No such file or directory",
                ));
            };
            Ok(self.info(file_type))
        }

        fn enumerate_children(
            &self,
            _attributes: &str,
            _flags: gio::FileQueryInfoFlags,
            _cancellable: Option<&gio::Cancellable>,
        ) -> Result<gio::FileEnumerator, glib::Error> {
            let device = self.device();
            if device.file_type(self.location()) != Some(gio::FileType::Directory) {
                return Err(device_error(gio::IOErrorEnum::NotDirectory, "Not a directory"));
            }
            let names = device.children_of(self.location());
            let infos = names.iter().map(|name| listing_entry(name)).collect();
            Ok(super::DeviceListing::new(&self.obj(), infos).upcast())
        }

        fn set_display_name(
            &self,
            name: &str,
            _cancellable: Option<&gio::Cancellable>,
        ) -> Result<gio::File, glib::Error> {
            let parent = parent_uri(self.location()).expect("items below the root have a folder");
            let renamed = format!("{parent}/{}", escape_segment(name));
            self.device().rename(self.location(), &renamed, name)?;
            Ok(self.file_at(&renamed))
        }

        fn delete(&self, _cancellable: Option<&gio::Cancellable>) -> Result<(), glib::Error> {
            self.device().delete(self.location())
        }

        fn move_(
            source: &gio::File,
            destination: &gio::File,
            flags: gio::FileCopyFlags,
            _cancellable: Option<&gio::Cancellable>,
            _progress_callback: Option<&mut dyn FnMut(i64, i64)>,
        ) -> Result<(), glib::Error> {
            let source = source
                .downcast_ref::<super::DeviceFile>()
                .expect("only device files are moved on a device");
            let device = source.imp().device();
            device.move_item(&source.uri(), &destination.uri(), flags)
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
            Ok(self.remaining.lock().expect("listing").pop())
        }

        fn close(&self, _cancellable: Option<&gio::Cancellable>) -> (bool, Option<glib::Error>) {
            (true, None)
        }
    }
}
