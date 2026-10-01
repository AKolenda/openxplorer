// SPDX-License-Identifier: AGPL-3.0-only
//! Copy path: puts the address of each selected item, one per line, or of
//! the folder, on the clipboard as text (CLIP-012, CLIP-014).
//!
//! Ports `copyPath` in `v2.0.0:desktop/ui/app.js` and the text it copies,
//! `displayUri`: a plain path for local items, `\\server\share\…` for SMB
//! and `<device> / path` for phones and cameras. Only text is copied, so
//! the command changes no file and no sharing permission, and it needs
//! none of the file-operation workflows. Ctrl+Shift+C and Ctrl+Alt+C run
//! it too (CLIP-013, in `file_ops::shortcuts`).
//!
//! The Python app shows the text in a "Location" dialog when its bridge
//! cannot write the clipboard. GTK's display clipboard always exists and
//! setting its text cannot fail, so that dialog has no counterpart here.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::LocationContext;

use crate::locations::Page;

use super::BrowserWindow;

/// What Copy path copies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CopiedPath {
    /// The display address of a folder or item, or of several items, one
    /// per line.
    Address(String),
    /// A landing page has no path.
    NoFolder,
}

/// What Copy path copies when `selected` are the URIs of the selected
/// items in the folder `folder_uri`: the address of every selected item,
/// one per line, else the folder's. A landing page has none.
fn copied_path(selected: &[String], folder_uri: Option<&str>, locations: &LocationContext) -> CopiedPath {
    let uris: Vec<&str> = if selected.is_empty() {
        folder_uri.into_iter().collect()
    } else {
        selected.iter().map(String::as_str).collect()
    };
    let addresses: Vec<String> = uris
        .into_iter()
        .filter(|uri| Page::from_uri(uri).is_none())
        .map(|uri| locations.display_location(uri))
        .collect();
    if addresses.is_empty() {
        return CopiedPath::NoFolder;
    }
    CopiedPath::Address(addresses.join("\n"))
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

    /// Copies the path and says so, as app.js does.
    pub(super) fn copy_path(&self) {
        match self.path_to_copy() {
            CopiedPath::NoFolder => self.show_message("Open a folder first."),
            CopiedPath::Address(address) => {
                self.clipboard().set_text(&address);
                self.show_message("Path copied. Sharing permissions are unchanged.");
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
    fn local_smb_and_device_items_copy_their_display_path() {
        let locations = LocationContext {
            devices: vec![DeviceLabel {
                uri: "mtp://Pixel_7/".to_owned(),
                label: "Pixel 7".to_owned(),
            }],
            ..LocationContext::default()
        };

        let local = copied_item("file:///home/sam/Plan%20Q3.txt", &locations);
        let smb = copied_item("smb://studio-nas/projects/Plan%20Q3.txt", &locations);
        let device = copied_item("mtp://Pixel_7/Internal%20storage/DCIM", &locations);

        assert_eq!(local, address("/home/sam/Plan Q3.txt"));
        assert_eq!(smb, address(r"\\studio-nas\projects\Plan Q3.txt"));
        assert_eq!(device, address("Pixel 7 / Internal storage/DCIM"));
    }

    /// parity: CLIP-012
    #[test]
    fn without_a_selection_the_folder_is_copied_and_a_page_has_none() {
        let locations = LocationContext::default();
        let nothing_selected = copied_path(&[], Some("file:///srv/media"), &locations);
        let on_this_pc = copied_path(&[], Some(Page::ThisPc.uri()), &locations);

        assert_eq!(nothing_selected, address("/srv/media"));
        assert_eq!(on_this_pc, CopiedPath::NoFolder);
    }

    /// parity: CLIP-014
    #[test]
    fn several_selected_items_copy_one_path_per_line() {
        let locations = LocationContext::default();
        let two = ["file:///srv/a.txt".to_owned(), "file:///srv/b%20c.txt".to_owned()];

        let several_selected = copied_path(&two, Some("file:///srv"), &locations);

        assert_eq!(several_selected, address("/srv/a.txt\n/srv/b c.txt"));
    }
}
