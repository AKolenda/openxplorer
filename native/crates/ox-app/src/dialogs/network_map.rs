// SPDX-License-Identifier: AGPL-3.0-only
//! Map network location: a share's address, its sidebar label and whether
//! to keep it.
//!
//! Ports `connectDialog` in `v2.0.0:desktop/ui/app.js` (NET-001), with the other
//! protocols of Dolphin and Files (NET-002). The dialog only
//! asks; the window connects ([`crate::window`]'s Map network location),
//! showing "Connecting…" meanwhile and any error inside the dialog.

use gtk::prelude::*;

use super::network_protocol::Protocol;
use crate::window::{ButtonStyle, Dialog};

/// What the dialog says under its heading.
const MESSAGE: &str =
    crate::i18n::message_id("Add a network folder to your sidebar: a Windows share, or a folder on an SSH, FTP, WebDAV or NFS server.");

/// The note under the fields: what connecting asks and saves.
const NOTE: &str = crate::i18n::message_id("OpenXplorer will ask for your username and password if needed. Remember my credentials is \
selected by default. No passwords are saved in OpenXplorer settings. A label such as “Z:” is only a label, not a \
system-wide drive letter.");

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
    on_connect: impl Fn(&Dialog, MapRequest) + 'static,
) -> Dialog {
    let dialog = Dialog::new(
        parent,
        &ox_core::i18n::gettext("Map network location"),
        ox_core::i18n::gettext_static(MESSAGE),
    );
    let fields = ServerFields::add_to(&dialog);
    let label = address_field(&dialog, "Display name (optional)", "Projects (Z:)");
    let save = dialog.add_check_button(
        &ox_core::i18n::gettext("Save in the sidebar · reconnect when opened"),
        true,
    );
    dialog.add_note(ox_core::i18n::gettext_static(NOTE));
    dialog.add_cancel_button();
    dialog.add_button(&ox_core::i18n::gettext("Connect"), ButtonStyle::Accent);
    dialog.connect_confirmed(move |dialog| {
        let keeping = if save.is_active() {
            ShareKeeping::SaveInSidebar
        } else {
            ShareKeeping::ThisSessionOnly
        };
        let request = MapRequest {
            address: fields.address(),
            label: label.text().into(),
            keeping,
        };
        on_connect(dialog, request);
    });
    dialog
}

/// Adds a text field labelled `label` showing `placeholder` while empty.
/// Addresses and labels are never spell-checked or completed
/// (`spellcheck=false` and `autocomplete=off` in app.js).
fn address_field(dialog: &Dialog, label: &str, placeholder: &str) -> gtk::Entry {
    let entry = dialog.add_text_field(label, "");
    entry.set_placeholder_text(Some(placeholder));
    entry.set_input_hints(gtk::InputHints::NO_SPELLCHECK);
    entry
}

/// The fields that name the folder: the protocol, the folder, and for
/// protocols other than SMB the port and user name (NET-002).
struct ServerFields {
    protocol: gtk::DropDown,
    folder: gtk::Entry,
    port: gtk::Entry,
    user: gtk::Entry,
}

impl ServerFields {
    /// Adds the fields to `dialog`, showing those SMB uses.
    fn add_to(dialog: &Dialog) -> Self {
        let labels = Protocol::ALL.map(Protocol::label);
        let protocol = gtk::DropDown::from_strings(&labels);
        dialog.add_labelled("Type", &protocol);
        let folder = address_field(dialog, "Folder", Protocol::Smb.placeholder());
        let port = address_field(dialog, "Port (optional)", "");
        port.set_input_purpose(gtk::InputPurpose::Digits);
        let user = address_field(dialog, "User name (optional)", "");
        let fields = Self {
            protocol,
            folder,
            port,
            user,
        };
        fields.show_fields_of(Protocol::Smb);
        let (folder, port, user) = (fields.folder.clone(), fields.port.clone(), fields.user.clone());
        fields.protocol.connect_selected_notify(move |protocol| {
            let chosen = Protocol::at(protocol.selected());
            folder.set_placeholder_text(Some(chosen.placeholder()));
            Dialog::set_field_visible(&port, chosen.asks_port());
            Dialog::set_field_visible(&user, chosen.asks_user());
        });
        fields
    }

    /// Shows the fields `protocol` asks for.
    fn show_fields_of(&self, protocol: Protocol) {
        Dialog::set_field_visible(&self.port, protocol.asks_port());
        Dialog::set_field_visible(&self.user, protocol.asks_user());
    }

    /// The address the fields describe.
    fn address(&self) -> String {
        let protocol = Protocol::at(self.protocol.selected());
        protocol.address(&self.folder.text(), &self.port.text(), &self.user.text())
    }
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
        dialog.open();
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
        assert_eq!(placeholders, ["\\\\nas\\Projects", "", "", "Projects (Z:)"]);
        let visible: Vec<bool> = entries.iter().map(WidgetExt::is_visible).collect();
        assert_eq!(
            visible,
            [true, false, false, true],
            "SMB asks for no port or user"
        );
        let focus = GtkWindowExt::focus(&dialog);
        assert!(
            focus.is_some_and(|focus| focus.is_ancestor(&entries[0])),
            "the folder field has focus"
        );

        entries[0].set_text("\\\\nas\\Projects");
        entries[3].set_text("Projects (Z:)");
        dialog.press("Connect");
        settle();
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

        dialog.press("Connect");
        settle();

        let keeping: Vec<ShareKeeping> = requests.borrow().iter().map(|request| request.keeping).collect();
        assert_eq!(keeping, [ShareKeeping::ThisSessionOnly]);
        dialog.close();
        parent.close();
    }

    /// parity: NET-002
    #[gtk::test]
    fn choosing_sftp_asks_for_a_port_and_user_and_maps_an_sftp_folder() {
        let parent = gtk::Window::new();
        let requests = Rc::new(RefCell::new(Vec::new()));
        let heard = Rc::clone(&requests);
        let dialog = map_network_dialog(&parent, move |_, request| heard.borrow_mut().push(request));
        let protocol = descendants::<gtk::DropDown>(&dialog);
        dialog.open();
        settle();
        protocol[0].set_selected(1);
        let entries = descendants::<gtk::Entry>(&dialog);
        assert!(
            WidgetExt::is_visible(&entries[1]) && WidgetExt::is_visible(&entries[2]),
            "port and user"
        );
        assert_eq!(entries[0].placeholder_text().as_deref(), Some("server/home/anna"));

        entries[0].set_text("build/home/anna");
        entries[1].set_text("2222");
        entries[2].set_text("anna");
        dialog.press("Connect");
        settle();

        let addresses: Vec<String> = requests
            .borrow()
            .iter()
            .map(|request| request.address.clone())
            .collect();
        assert_eq!(addresses, ["sftp://anna@build:2222/home/anna"]);
        protocol[0].set_selected(6);
        assert!(!WidgetExt::is_visible(&entries[2]), "NFS asks for no user name");
        dialog.close();
        parent.close();
    }

    /// Sets its flag when the work holding it is dropped.
    struct DropFlag(Rc<Cell<bool>>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    /// While a connection runs, Connect reads "Connecting…" and waits,
    /// Cancel stays usable, and Cancel drops the connection so a late
    /// success is ignored.
    ///
    /// parity: NET-001
    #[gtk::test]
    fn cancel_drops_the_running_connection() {
        let parent = gtk::Window::new();
        let dropped = Rc::new(Cell::new(false));
        let flag = Rc::clone(&dropped);
        let dialog = map_network_dialog(&parent, move |dialog, _| {
            let guard = DropFlag(Rc::clone(&flag));
            dialog.run("Connecting…", async move {
                let _guard = guard;
                std::future::pending::<()>().await;
            });
        });
        dialog.open();
        settle();

        dialog.press("Connect");
        settle();
        let connecting = !dialog.can_press("Connecting…") && dialog.can_press("Cancel");
        dialog.press("Cancel");
        settle();

        assert!(connecting, "Connect waits, Cancel does not");
        assert!(dropped.get(), "the connection is dropped");
        assert!(!dialog.is_visible());
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
        dialog.open();
        settle();
        descendants::<gtk::Entry>(&dialog)[0].set_text("\\\\nas\\Projects");

        dialog.press("Connect");
        settle();
        assert!(started.get(), "Connect started the connection");
        dialog.press("Cancel");
        wait_for(Duration::from_millis(300));

        assert!(!answered.get(), "the late connection was dropped");
        parent.close();
    }
}
