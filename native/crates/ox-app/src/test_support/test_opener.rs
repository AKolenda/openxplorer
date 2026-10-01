// SPDX-License-Identifier: AGPL-3.0-only
//! An application for the file types window tests open, so opening a file
//! finds one whatever the machine has installed.
//!
//! Tests record the file the default application would get (see
//! `AppContext::record_launches`) only once ox-core's opener has found
//! that application. The distribution containers of CI install no desktop
//! applications, so without this entry every open would end in "no
//! application". A host's own defaults still come first: the entry is not
//! made the default, it only answers where nothing else does.

use std::fs;

use gtk::glib;

use crate::integration::installed_application;

/// The desktop entry's file name.
const DESKTOP_ID: &str = "ox-test-opener.desktop";

/// The types the window tests open: their text files, scripts, documents,
/// pictures and archives.
const CONTENT_TYPES: [&str; 8] = [
    "text/plain",
    "application/x-shellscript",
    "text/markdown",
    "application/pdf",
    "image/png",
    "application/zip",
    "application/x-compressed-tar",
    "application/gzip",
];

/// Adds the test application to the private data folder the tests run with
/// (native/tools/check.py) and waits until GIO lists it.
///
/// # Panics
///
/// When the data folder is not a private temporary one, or cannot be
/// written.
pub(crate) fn install() {
    let data = glib::user_data_dir();
    assert!(
        data.starts_with(std::env::temp_dir()),
        "tests add applications only to a private data folder"
    );
    let folder = data.join("applications");
    fs::create_dir_all(&folder).expect("the data folder is writable");
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Test opener\nExec=true %U\nNoDisplay=true\nMimeType={};\n",
        CONTENT_TYPES.join(";")
    );
    fs::write(folder.join(DESKTOP_ID), entry).expect("the data folder is writable");
    super::harness::wait_until("GIO to list the test opener", || {
        installed_application(DESKTOP_ID).is_some()
    });
}
