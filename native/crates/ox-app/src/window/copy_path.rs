// SPDX-License-Identifier: AGPL-3.0-only
//! Copy as path: puts the address of each selected item, one per line, or
//! of the folder, on the clipboard as text, each in double quotes as
//! Windows writes them, with or without spaces (CLIP-012, CLIP-014). The
//! context menus name it "Copy as path", as Windows does; the Python app
//! called it Copy path and had it in the command bar too, where Share is
//! now (`super::share`).
//!
//! Ports `copyPath` in `v2.0.0:desktop/ui/app.js` and the text it copies,
//! `displayUri`: a plain path for local items, `\\server\share\…` for SMB
//! and `<device> / path` for phones and cameras, each quoted whole. An SMB
//! server or share root loses its trailing backslash, which would escape
//! the closing quote in a shell. Only the quotes and that trim are made
//! here: other users of the display address, such as the Properties copy
//! button, keep it plain. Only text is copied, so the command changes no
//! file and no sharing permission, and it needs
//! none of the file-operation workflows. Ctrl+Shift+C and Ctrl+Alt+C run
//! it too (CLIP-013, in `file_ops::shortcuts`).
//!
//! The Python app shows the text in a "Location" dialog when its bridge
//! cannot write the clipboard. GTK's display clipboard always exists and
//! setting its text cannot fail, so that dialog has no counterpart here.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::{is_smb_location, LocationContext};

use crate::locations::Page;

use super::BrowserWindow;

/// What Copy path copies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CopiedPath {
    /// The quoted display address of a folder or item, or of several
    /// items, one per line.
    Address(String),
    /// A landing page has no path.
    NoFolder,
}

/// What Copy path copies when `selected` are the URIs of the selected
/// items in the folder `folder_uri`: the quoted address of every selected
/// item, one per line, else the folder's. A landing page has none.
fn copied_path(selected: &[String], folder_uri: Option<&str>, locations: &LocationContext) -> CopiedPath {
    let uris: Vec<&str> = if selected.is_empty() {
        folder_uri.into_iter().collect()
    } else {
        selected.iter().map(String::as_str).collect()
    };
    let addresses: Vec<String> = uris
        .into_iter()
        .filter(|uri| Page::from_uri(uri).is_none())
        .map(|uri| format!("\"{}\"", quotable_address(uri, locations)))
        .collect();
    if addresses.is_empty() {
        return CopiedPath::NoFolder;
    }
    CopiedPath::Address(addresses.join("\n"))
}

/// The address of `uri` to put between quotes. An SMB server or share root
/// (`\\server\`) drops its trailing backslash, which a shell would read
/// as escaping the closing quote; the backslash is not doubled.
fn quotable_address(uri: &str, locations: &LocationContext) -> String {
    let address = locations.copied_location(uri);
    if !is_smb_location(uri) {
        return address;
    }
    match address.strip_suffix('\\') {
        Some(trimmed) => trimmed.to_owned(),
        None => address,
    }
}

impl BrowserWindow {
    /// What Copy path would copy now: the selected items, else the folder
    /// the tab shows.
    pub(super) fn path_to_copy(&self) -> CopiedPath {
        let selected = self.folder_pane().model().selected_uris();
        let folder_uri = self.current_uri();
        let locations = self.imp().locations.borrow();
        copied_path(&selected, folder_uri.as_deref(), &locations)
    }

    /// Copies the path without a message, as Windows Explorer's Copy as
    /// path.
    pub(super) fn copy_path(&self) {
        match self.path_to_copy() {
            CopiedPath::NoFolder => self.show_message(&ox_core::i18n::gettext("Open a folder first.")),
            CopiedPath::Address(address) => {
                self.clipboard().set_text(&address);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ox_core::location::DeviceLabel;

    use super::*;

    /// What Copy path copies for the one item at `uri`.
    fn copied_item(uri: &str, locations: &LocationContext) -> CopiedPath {
        copied_path(&[uri.to_owned()], Some("file:///home/sam"), locations)
    }

    fn address(text: &str) -> CopiedPath {
        CopiedPath::Address(text.to_owned())
    }

    /// parity: CLIP-012
    #[test]
    fn a_local_path_with_a_space_is_copied_in_quotes() {
        let locations = LocationContext::default();

        let local = copied_item("file:///home/sam/Plan%20Q3.txt", &locations);

        assert_eq!(local, address(r#""/home/sam/Plan Q3.txt""#));
    }

    /// parity: CLIP-012
    #[test]
    fn an_smb_item_copies_its_unc_path_in_quotes() {
        let locations = LocationContext::default();

        let smb = copied_item("smb://studio-nas/projects/Plan%20Q3.txt", &locations);

        assert_eq!(smb, address(r#""\\studio-nas\projects\Plan Q3.txt""#));
    }

    /// A server or share root drops its trailing backslash before the
    /// closing quote, which a shell would otherwise read as escaping it.
    ///
    /// parity: CLIP-012
    #[test]
    fn an_smb_server_or_share_root_ends_without_a_backslash() {
        let locations = LocationContext::default();

        let server = copied_path(&[], Some("smb://studio-nas/"), &locations);
        let share = copied_path(&[], Some("smb://studio-nas/projects/"), &locations);
        let share_without_slash = copied_path(&[], Some("smb://studio-nas/projects"), &locations);

        assert_eq!(server, address(r#""\\studio-nas""#));
        assert_eq!(share, address(r#""\\studio-nas\projects""#));
        assert_eq!(share_without_slash, address(r#""\\studio-nas\projects""#));
    }

    /// parity: CLIP-012
    #[test]
    fn the_local_root_stays_a_slash() {
        let locations = LocationContext::default();

        let root = copied_path(&[], Some("file:///"), &locations);

        assert_eq!(root, address(r#""/""#));
    }

    /// The device name and the path inside it are quoted once, together.
    ///
    /// parity: CLIP-012
    #[test]
    fn a_phone_item_copies_its_device_path_in_quotes() {
        let locations = LocationContext {
            devices: vec![DeviceLabel {
                uri: "mtp://Pixel_7/".to_owned(),
                label: "Pixel 7".to_owned(),
            }],
            ..LocationContext::default()
        };

        let device = copied_item("mtp://Pixel_7/Internal%20storage/DCIM", &locations);

        assert_eq!(device, address(r#""Pixel 7 / Internal storage/DCIM""#));
    }

    /// Windows quotes every path, with or without spaces.
    ///
    /// parity: CLIP-012
    #[test]
    fn a_path_without_spaces_is_quoted_too() {
        let locations = LocationContext::default();

        let plain = copied_item("file:///home/sam/notes.txt", &locations);

        assert_eq!(plain, address(r#""/home/sam/notes.txt""#));
    }

    /// The quotes belong to Copy as path only: the address bar and the
    /// Properties copy button keep the plain address.
    #[test]
    fn copied_location_itself_stays_unquoted() {
        let locations = LocationContext::default();

        let plain = locations.copied_location("file:///home/sam/Plan%20Q3.txt");

        assert_eq!(plain, "/home/sam/Plan Q3.txt");
        assert_eq!(locations.copied_location("smb://studio-nas/"), r"\\studio-nas\");
    }

    /// parity: CLIP-012
    #[test]
    fn without_a_selection_the_folder_is_copied_and_a_page_has_none() {
        let locations = LocationContext::default();
        let nothing_selected = copied_path(&[], Some("file:///srv/media"), &locations);
        let on_this_pc = copied_path(&[], Some(Page::ThisPc.uri()), &locations);

        assert_eq!(nothing_selected, address(r#""/srv/media""#));
        assert_eq!(on_this_pc, CopiedPath::NoFolder);
    }

    /// parity: CLIP-014
    #[test]
    fn several_selected_items_copy_one_quoted_path_per_line() {
        let locations = LocationContext::default();
        let two = ["file:///srv/a.txt".to_owned(), "file:///srv/b%20c.txt".to_owned()];

        let several_selected = copied_path(&two, Some("file:///srv"), &locations);

        assert_eq!(several_selected, address("\"/srv/a.txt\"\n\"/srv/b c.txt\""));
    }
}
