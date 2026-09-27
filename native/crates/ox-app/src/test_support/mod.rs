// SPDX-License-Identifier: AGPL-3.0-only
//! What the crate's tests share: listed entries built through ox-core's own
//! conversion, so they carry every field a real listing does, and the GTK
//! [`harness`] for tests that open windows.

pub(crate) mod harness;

use gtk::gio;
use ox_core::entry::{entry_from_info, Entry};

fn entry(name: &str, file_type: gio::FileType) -> Entry {
    let info = gio::FileInfo::new();
    info.set_file_type(file_type);
    info.set_display_name(name);
    let file = gio::File::for_path(format!("/tmp/ox-test/{name}"));
    entry_from_info(&file, &info)
}

/// A regular file named `name` in `/tmp/ox-test`.
pub(crate) fn file_entry(name: &str) -> Entry {
    entry(name, gio::FileType::Regular)
}

/// A folder named `name` in `/tmp/ox-test`.
pub(crate) fn folder_entry(name: &str) -> Entry {
    entry(name, gio::FileType::Directory)
}
