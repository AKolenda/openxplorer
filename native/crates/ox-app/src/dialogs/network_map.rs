// SPDX-License-Identifier: AGPL-3.0-only
//! Map network location: a share's address, its sidebar label and whether
//! to keep it.
//!
//! Ports `connectDialog` in `desktop/ui/app.js` (NET-001). The dialog only
//! asks; the window connects ([`crate::window`]'s Map network location),
//! showing "Connecting…" meanwhile and any error inside the dialog.

use gtk::prelude::*;

use super::network_form::{CheckState, NetworkFormDialog};

/// What the dialog says under its heading.
const MESSAGE: &str =
    "Add a shared folder to your sidebar. Connect using a Windows-style address or an SMB URL.";

/// The note under the fields: what connecting asks and saves.
const NOTE: &str = "OpenXplorer will ask for your username and password if needed. Remember my credentials is \
selected by default. No passwords are saved in OpenXplorer settings. A label such as “Z:” is only a label, not a \
system-wide drive letter.";

/// Whether a mapped share is kept in the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShareKeeping {
    /// "Save in the sidebar · reconnect when opened", the default.
    SaveInSidebar,
    /// Listed under Network for this session only.
    ThisSessionOnly,
}

/// What the user asked Map network location to connect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MapRequest {
    /// The share, as typed: `\\nas\Projects` or `smb://nas/Projects`.
    pub address: String,
    /// The sidebar label, or empty for the folder's name.
    pub label: String,
    /// Whether the share is saved in the sidebar.
    pub keeping: ShareKeeping,
}

/// The Map network location dialog over `parent`. Its Connect button calls
/// `on_connect` with the dialog, to run the connection in, and what was
/// typed.
pub(crate) fn map_network_dialog(
    parent: &impl IsA<gtk::Window>,
    on_connect: impl Fn(&NetworkFormDialog, MapRequest) + 'static,
) -> NetworkFormDialog {
    let dialog = NetworkFormDialog::new(parent, "Map network location", MESSAGE, "Connect");
    let folder = dialog.add_text_field("Folder", "\\\\nas\\Projects");
    let label = dialog.add_text_field("Display name (optional)", "Projects (Z:)");
    let save = dialog.add_check_box("Save in the sidebar · reconnect when opened", CheckState::Checked);
    dialog.add_note(NOTE);
    dialog.connect_confirmed(move |dialog| {
        let keeping = if save.is_active() {
            ShareKeeping::SaveInSidebar
        } else {
            ShareKeeping::ThisSessionOnly
        };
        let request = MapRequest {
            address: folder.text().into(),
            label: label.text().into(),
            keeping,
        };
        on_connect(dialog, request);
    });
    dialog
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::Duration;

    use gtk::glib;

    use super::*;
    use crate::test_support::harness::{descendants, settle, wait_for};

    /// parity: NET-001
    #[gtk::test]
    fn the_dialog_asks_for_a_folder_a_label_and_whether_to_save_it() {
        let parent = gtk::Window::new();
        let requests = Rc::new(RefCell::new(Vec::new()));
        let heard = Rc::clone(&requests);
        let dialog = map_network_dialog(&parent, move |_, request| heard.borrow_mut().push(request));
        dialog.present();
        settle();

        let texts = dialog.texts();
        for expected in [
            "Map network location",
            MESSAGE,
            "Folder",
            "Display name (optional)",
            "Save in the sidebar · reconnect when opened",
            NOTE,
        ] {
            assert!(
                texts.iter().any(|text| text == expected),
                "{expected} in {texts:?}"
            );
        }
        let entries = descendants::<gtk::Entry>(&dialog);
        let placeholders: Vec<String> = entries
            .iter()
            .filter_map(|entry| entry.placeholder_text().map(|text| text.to_string()))
            .collect();
        assert_eq!(placeholders, ["\\\\nas\\Projects", "Projects (Z:)"]);
        let focus = GtkWindowExt::focus(&dialog);
        assert!(
            focus.is_some_and(|focus| focus.is_ancestor(&entries[0])),
            "the folder field has focus"
        );

        entries[0].set_text("\\\\nas\\Projects");
        entries[1].set_text("Projects (Z:)");
        dialog.press_confirm();
        let expected = MapRequest {
            address: "\\\\nas\\Projects".into(),
            label: "Projects (Z:)".into(),
            keeping: ShareKeeping::SaveInSidebar,
        };
        assert_eq!(*requests.borrow(), [expected]);
        dialog.close();
        parent.close();
    }

    /// parity: NET-001
    #[gtk::test]
    fn an_unchecked_box_maps_the_share_for_this_session_only() {
        let parent = gtk::Window::new();
        let requests = Rc::new(RefCell::new(Vec::new()));
        let heard = Rc::clone(&requests);
        let dialog = map_network_dialog(&parent, move |_, request| heard.borrow_mut().push(request));
        let save = descendants::<gtk::CheckButton>(&dialog);
        save[0].set_active(false);

        dialog.press_confirm();

        let keeping: Vec<ShareKeeping> = requests.borrow().iter().map(|request| request.keeping).collect();
        assert_eq!(keeping, [ShareKeeping::ThisSessionOnly]);
        dialog.close();
        parent.close();
    }

    /// Cancel while "Connecting…" drops the connection: a success that
    /// arrives afterwards is never acted on.
    ///
    /// parity: SAFE-013
    #[gtk::test]
    fn a_connection_that_answers_after_cancel_is_ignored() {
        let parent = gtk::Window::new();
        let started = Rc::new(Cell::new(false));
        let answered = Rc::new(Cell::new(false));
        let (begun, heard) = (Rc::clone(&started), Rc::clone(&answered));
        let dialog = map_network_dialog(&parent, move |dialog, _| {
            begun.set(true);
            let heard = Rc::clone(&heard);
            dialog.run("Connecting…", async move {
                glib::timeout_future(Duration::from_millis(100)).await;
                heard.set(true);
            });
        });
        dialog.present();
        descendants::<gtk::Entry>(&dialog)[0].set_text("\\\\nas\\Projects");

        dialog.press_confirm();
        assert!(started.get(), "Connect started the connection");
        dialog.press_cancel();
        wait_for(Duration::from_millis(300));

        assert!(!answered.get(), "the late connection was dropped");
        parent.close();
    }
}
