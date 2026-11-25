// SPDX-License-Identifier: AGPL-3.0-only
//! A network share's items on the clipboard, for applications that open
//! local files only (CLIP-004, NET-026): a browser's upload, an email
//! attachment, a terminal. Windows Explorer gives them the `\\server\share`
//! path; here a mounted share's item is its local file in the standard URI
//! list, as a drag gives it (DND-004).

use std::fs;
use std::path::PathBuf;

use gtk::prelude::*;
use gtk::{gdk, glib};
use ox_core::clipboard::{ClipboardFiles, ClipboardMode, GNOME, KDE_URI_LIST, URI_LIST};
use ox_core::location::LocationContext;

use crate::test_support::harness::{Fixture, TestWindow};

/// A share item whose share the test mounts.
const SHARE_ITEM: &str = "smb://ledger-nas/finance/Budget%202027.ods";

/// `GVfs`'s FUSE export of `ledger-nas`'s `finance` share in the test's
/// private runtime folder, removed when dropped. Nothing is mounted: the
/// app only reads the export's folder names (`local_path`).
struct FuseExport {
    share: PathBuf,
}

impl FuseExport {
    fn new() -> Self {
        let share = glib::user_runtime_dir()
            .join("gvfs")
            .join("smb-share:server=ledger-nas,share=finance");
        fs::create_dir_all(&share).expect("the test's runtime folder is writable");
        Self { share }
    }
}

impl Drop for FuseExport {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.share);
    }
}

/// The text `mime_type` holds on `clipboard`.
fn clipboard_text(clipboard: &gdk::Clipboard, mime_type: &str) -> String {
    glib::MainContext::default().block_on(async {
        let (stream, _) = clipboard
            .read_future(&[mime_type], glib::Priority::DEFAULT)
            .await
            .expect("the clipboard offers the format");
        let bytes = stream
            .read_bytes_future(4096, glib::Priority::DEFAULT)
            .await
            .expect("the payload reads");
        String::from_utf8(bytes.to_vec()).expect("an ASCII list")
    })
}

/// parity: CLIP-004, NET-026
#[gtk::test]
fn a_mounted_shares_item_is_copied_as_its_local_file_for_other_apps() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let export = FuseExport::new();
    let files = ClipboardFiles::new(
        ClipboardMode::Copy,
        &[SHARE_ITEM.to_owned()],
        &LocationContext::default(),
    )
    .expect("a share item");

    test.window.put_files_on_clipboard(files);

    let clipboard = test.window.clipboard();
    let local = ox_core::location::file_uri(&export.share.join("Budget 2027.ods"));
    assert_eq!(clipboard_text(&clipboard, URI_LIST), format!("{local}\r\n"));
    assert_eq!(
        clipboard_text(&clipboard, KDE_URI_LIST),
        format!("{SHARE_ITEM}\r\n")
    );
    assert_eq!(clipboard_text(&clipboard, GNOME), format!("copy\n{SHARE_ITEM}"));
}
