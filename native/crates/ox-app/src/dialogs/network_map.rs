// SPDX-License-Identifier: AGPL-3.0-only
//! Map network drive, as Windows 11 asks it: a drive letter, a folder and
//! whether to reconnect, and the link to Add a network location, which
//! connects a share with a Type list and a display name of its own.
//!
//! Ports `connectDialog` in `v2.0.0:desktop/ui/app.js` (NET-001), with the other
//! protocols of Dolphin and Files (NET-002) in Add a network location. The
//! dialogs only ask; the window connects ([`crate::window`]'s Map network
//! drive), showing "Connecting…" meanwhile and any error inside the dialog.

use gtk::prelude::*;
use ox_core::location::MAX_LABEL_CHARS;
use ox_core::network::CredentialChoice;

use super::network_protocol::Protocol;
use crate::window::{ButtonStyle, Dialog, WindowAction};

/// Map network drive's question, under its heading.
const MAP_QUESTION: &str = crate::i18n::message_id("What network folder would you like to map?");

/// What Map network drive asks for, under its question.
const MAP_MESSAGE: &str = crate::i18n::message_id(
    "Specify the drive letter for the connection and the folder that you want to connect to:",
);

/// The link that opens Add a network location, in Windows' words.
const WEB_SITE_LINK: &str =
    crate::i18n::message_id("Connect to a Web site that you can use to store your documents and pictures.");

/// The letters of the Drive list, Z: first and chosen, as Windows offers
/// them; C: is the system drive. On Linux a letter is only the end of the
/// share's label, such as `Projects (Z:)`.
const DRIVE_LETTERS: [&str; 25] = [
    "Z:", "Y:", "X:", "W:", "V:", "U:", "T:", "S:", "R:", "Q:", "P:", "O:", "N:", "M:", "L:", "K:", "J:",
    "I:", "H:", "G:", "F:", "E:", "D:", "B:", "A:",
];

/// What Add a network location says under its heading.
const MESSAGE: &str =
    crate::i18n::message_id("Add a network folder to your sidebar: a Windows share, or a folder on an SSH, FTP, WebDAV or NFS server.");

/// The note under Add a network location's fields: what connecting asks
/// and saves.
const NOTE: &str = crate::i18n::message_id("OpenXplorer will ask for your username and password if needed. Remember my credentials is \
selected by default. No passwords are saved in OpenXplorer settings. A label such as “Z:” is only a label, not a \
system-wide drive letter.");

/// Whether a mapped share is kept in the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShareKeeping {
    /// Saved under Network and connected again when opened: Map network
    /// drive's "Reconnect at sign-in", and Add a network location's "Save
    /// in the sidebar · reconnect when opened", both the default.
    SaveInSidebar,
    /// Listed under Network for this session only.
    ThisSessionOnly,
}

/// What the user asked Map network drive or Add a network location to
/// connect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MapRequest {
    /// The share, as typed: `\\nas\Projects` or `smb://nas/Projects`.
    pub address: String,
    /// The sidebar label, or empty for the folder's name.
    pub label: String,
    /// The drive letter Map network drive gives the label, such as `Z:`.
    pub drive: Option<&'static str>,
    /// Whether the share is saved in the sidebar.
    pub keeping: ShareKeeping,
    /// Which account signs in: "Connect using different credentials"
    /// always asks.
    pub credentials: CredentialChoice,
}

impl MapRequest {
    /// The share's label once connected as `name`: the drive letter
    /// after it, as `Projects (Z:)`, which This PC shows as Windows names
    /// a mapped drive. A name too long to take the letter within a
    /// sidebar label's limit is shortened, since this dialog has no field
    /// to correct it.
    pub(crate) fn label_for(&self, name: &str) -> String {
        let Some(letter) = self.drive else {
            return name.to_owned();
        };
        let suffix = format!(" ({letter})");
        let room = MAX_LABEL_CHARS.saturating_sub(suffix.chars().count());
        let name: String = name.chars().take(room).collect();
        format!("{}{suffix}", name.trim_end())
    }
}

/// Windows 11's Map network drive over `parent`: the Drive list, the
/// Folder, "Reconnect at sign-in", "Connect using different credentials"
/// and the link to Add a network location. Its Finish button calls
/// `on_connect` with the dialog, to run the connection in, and what was
/// chosen.
pub(crate) fn map_network_dialog(
    parent: &impl IsA<gtk::Window>,
    on_connect: impl Fn(&Dialog, MapRequest) + 'static,
) -> Dialog {
    let message = format!(
        "{}\n{}",
        ox_core::i18n::gettext_static(MAP_QUESTION),
        ox_core::i18n::gettext_static(MAP_MESSAGE)
    );
    let dialog = Dialog::with_title_bar(parent, &ox_core::i18n::gettext("Map network drive"), &message);
    let drive = gtk::DropDown::from_strings(&DRIVE_LETTERS);
    drive.set_halign(gtk::Align::Start);
    dialog.add_labelled(ox_core::i18n::gettext_static("Drive:"), &drive);
    let folder = address_field(&dialog, ox_core::i18n::gettext_static("Folder:"), "");
    dialog.add_hint(&ox_core::i18n::format_message(
        "Example: {address}",
        &[("address", "\\\\server\\share")],
    ));
    let reconnect = dialog.add_check_button(&ox_core::i18n::gettext("Reconnect at sign-in"), true);
    let different = dialog.add_check_button(
        &ox_core::i18n::gettext("Connect using different credentials"),
        false,
    );
    dialog.add_field(&web_site_link(&dialog));
    dialog.add_cancel_button();
    dialog.add_button(&ox_core::i18n::gettext("Finish"), ButtonStyle::Accent);
    dialog.connect_confirmed(move |dialog| {
        let keeping = if reconnect.is_active() {
            ShareKeeping::SaveInSidebar
        } else {
            ShareKeeping::ThisSessionOnly
        };
        let credentials = if different.is_active() {
            CredentialChoice::Different
        } else {
            CredentialChoice::Known
        };
        let letter = usize::try_from(drive.selected()).ok();
        let request = MapRequest {
            address: folder.text().trim().to_owned(),
            label: String::new(),
            drive: letter.and_then(|letter| DRIVE_LETTERS.get(letter).copied()),
            keeping,
            credentials,
        };
        on_connect(dialog, request);
    });
    dialog
}

/// "Connect to a Web site…": cancels Map network drive and opens Add a
/// network location, as Windows' link starts the Add Network Location
/// wizard. SFTP, FTP, WebDAV and NFS folders are mapped there; their
/// addresses typed in Folder work too.
fn web_site_link(dialog: &Dialog) -> gtk::Button {
    let label = gtk::Label::builder()
        .label(ox_core::i18n::gettext_static(WEB_SITE_LINK))
        .wrap(true)
        .xalign(0.0)
        .build();
    let link = gtk::Button::builder()
        .child(&label)
        .halign(gtk::Align::Start)
        .css_classes(["link-button", "dialog-link"])
        .build();
    let dialog = dialog.downgrade();
    link.connect_clicked(move |_| {
        let Some(dialog) = dialog.upgrade() else {
            return;
        };
        let parent = dialog.transient_for();
        // Closing cancels, which also drops a connection still running.
        dialog.close();
        if let Some(parent) = parent {
            let action = WindowAction::AddNetworkLocation.detailed_name();
            let _ = parent.activate_action(&action, None);
        }
    });
    link
}

/// The same dialog as Windows 11's "Add a network location" (the "…"
/// menu of This PC and Network, and Map network drive's link): it starts
/// at a web address, as Windows' wizard adds websites and FTP sites, and
/// saves what it connects under Network locations like any mapped
/// folder.
pub(crate) fn network_location_dialog(
    parent: &impl IsA<gtk::Window>,
    on_connect: impl Fn(&Dialog, MapRequest) + 'static,
) -> Dialog {
    let title = ox_core::i18n::gettext("Add a network location");
    let dialog = Dialog::with_title_bar(parent, &title, ox_core::i18n::gettext_static(MESSAGE));
    let fields = ServerFields::add_to(&dialog);
    fields.choose(Protocol::WebDavs);
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
            drive: None,
            keeping,
            credentials: CredentialChoice::Known,
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
        dialog.add_labelled(ox_core::i18n::gettext_static("Type"), &protocol);
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

    /// Chooses `protocol` in the Type list, which shows its fields.
    fn choose(&self, protocol: Protocol) {
        let position = Protocol::ALL.iter().position(|each| *each == protocol);
        let position = position.and_then(|position| u32::try_from(position).ok());
        self.protocol.set_selected(position.unwrap_or_default());
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

    /// The requests a dialog's `on_connect` heard.
    type Heard = Rc<RefCell<Vec<MapRequest>>>;

    /// A handler that records each request in `heard`.
    fn recorder(heard: &Heard) -> impl Fn(&Dialog, MapRequest) + 'static {
        let heard = Rc::clone(heard);
        move |_, request| heard.borrow_mut().push(request)
    }

    /// Map network drive asks as Windows 11 does: a drive letter, Z:
    /// first, the folder with Windows' example, "Reconnect at sign-in"
    /// checked and "Connect using different credentials" not, then Finish
    /// and Cancel. The letter ends the share's label.
    ///
    /// parity: NET-001
    #[gtk::test]
    fn map_network_drive_asks_for_a_drive_and_a_folder_as_windows_does() {
        let parent = gtk::Window::new();
        let requests = Heard::default();
        let dialog = map_network_dialog(&parent, recorder(&requests));
        dialog.open();
        settle();

        assert_eq!(dialog.title_text(), "Map network drive");
        assert_eq!(dialog.message_text(), format!("{MAP_QUESTION}\n{MAP_MESSAGE}"));
        let texts = dialog.texts();
        for expected in [
            "Drive:",
            "Folder:",
            "Example: \\\\server\\share",
            "Reconnect at sign-in",
            "Connect using different credentials",
            WEB_SITE_LINK,
        ] {
            assert!(
                texts.iter().any(|text| text == expected),
                "{expected} in {texts:?}"
            );
        }
        assert_eq!(dialog.button_labels(), ["Cancel", "Finish"]);
        let drive = descendants::<gtk::DropDown>(&dialog);
        assert_eq!(drive[0].selected(), 0, "Z: is chosen");
        let checks: Vec<bool> = descendants::<gtk::CheckButton>(&dialog)
            .iter()
            .map(CheckButtonExt::is_active)
            .collect();
        assert_eq!(checks, [true, false]);
        let entries = descendants::<gtk::Entry>(&dialog);
        assert_eq!(entries.len(), 1, "only the folder is typed");
        let focus = GtkWindowExt::focus(&dialog);
        assert!(
            focus.is_some_and(|focus| focus.is_ancestor(&entries[0])),
            "the folder field has focus"
        );

        entries[0].set_text("\\\\nas\\Projects");
        dialog.press("Finish");
        settle();
        let expected = MapRequest {
            address: "\\\\nas\\Projects".into(),
            label: String::new(),
            drive: Some("Z:"),
            keeping: ShareKeeping::SaveInSidebar,
            credentials: CredentialChoice::Known,
        };
        assert_eq!(*requests.borrow(), std::slice::from_ref(&expected));
        assert_eq!(expected.label_for("Projects"), "Projects (Z:)");
        let long = expected.label_for(&"é".repeat(MAX_LABEL_CHARS));
        assert_eq!(
            long.chars().count(),
            MAX_LABEL_CHARS,
            "the letter fits the label's limit"
        );
        assert!(long.ends_with("é (Z:)"));
        dialog.close();
        parent.close();
    }

    /// Another letter names the share, an unchecked "Reconnect at
    /// sign-in" maps it for this session only, and "Connect using
    /// different credentials" asks for an account even where one is
    /// known.
    ///
    /// parity: NET-001
    #[gtk::test]
    fn the_letter_reconnecting_and_different_credentials_are_kept() {
        let parent = gtk::Window::new();
        let requests = Heard::default();
        let dialog = map_network_dialog(&parent, recorder(&requests));
        descendants::<gtk::DropDown>(&dialog)[0].set_selected(1);
        let checks = descendants::<gtk::CheckButton>(&dialog);
        checks[0].set_active(false);
        checks[1].set_active(true);
        descendants::<gtk::Entry>(&dialog)[0].set_text("  sftp://anna@build/home/anna ");

        dialog.press("Finish");
        settle();

        let requests = requests.borrow();
        let [request] = requests.as_slice() else {
            panic!("one request: {requests:?}");
        };
        assert_eq!(request.address, "sftp://anna@build/home/anna", "a URL works too");
        assert_eq!(request.drive, Some("Y:"));
        assert_eq!(request.keeping, ShareKeeping::ThisSessionOnly);
        assert_eq!(request.credentials, CredentialChoice::Different);
        dialog.close();
        parent.close();
    }

    /// "Connect to a Web site…" cancels Map network drive, dropping a
    /// connection still running, and opens Add a network location, as
    /// Windows' link starts its wizard.
    ///
    /// parity: NET-001, NET-002
    #[gtk::test]
    fn the_web_site_link_opens_add_a_network_location() {
        let parent = gtk::Window::new();
        let opened = Rc::new(Cell::new(0));
        let actions = gtk::gio::SimpleActionGroup::new();
        let add = gtk::gio::SimpleAction::new("add-network-location", None);
        let count = Rc::clone(&opened);
        add.connect_activate(move |_, _| count.set(count.get() + 1));
        actions.add_action(&add);
        parent.insert_action_group("win", Some(&actions));
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
        descendants::<gtk::Entry>(&dialog)[0].set_text("\\\\nas\\Projects");
        dialog.press("Finish");
        settle();

        let link = descendants::<gtk::Button>(&dialog)
            .into_iter()
            .find(|button| button.has_css_class("dialog-link"))
            .expect("the link");
        link.emit_clicked();
        settle();

        assert_eq!(opened.get(), 1, "Add a network location opens");
        assert!(!dialog.is_visible(), "Map network drive closes");
        assert!(dropped.get(), "the running connection is dropped");
        parent.close();
    }

    /// Add a network location keeps the Type list, a display name and
    /// whether to save the folder; another protocol asks for a port and
    /// user and maps its folder.
    ///
    /// parity: NET-001, NET-002
    #[gtk::test]
    fn add_a_network_location_maps_an_sftp_folder_under_its_display_name() {
        let parent = gtk::Window::new();
        let requests = Heard::default();
        let dialog = network_location_dialog(&parent, recorder(&requests));
        let protocol = descendants::<gtk::DropDown>(&dialog);
        dialog.open();
        settle();
        let texts = dialog.texts();
        for expected in [
            "Display name (optional)",
            "Save in the sidebar · reconnect when opened",
            NOTE,
        ] {
            assert!(
                texts.iter().any(|text| text == expected),
                "{expected} in {texts:?}"
            );
        }
        protocol[0].set_selected(1);
        let entries = descendants::<gtk::Entry>(&dialog);
        assert!(
            WidgetExt::is_visible(&entries[1]) && WidgetExt::is_visible(&entries[2]),
            "port and user"
        );
        assert_eq!(entries[0].placeholder_text().as_deref(), Some("server/home/anna"));
        assert_eq!(entries[3].placeholder_text().as_deref(), Some("Projects (Z:)"));

        entries[0].set_text("build/home/anna");
        entries[1].set_text("2222");
        entries[2].set_text("anna");
        entries[3].set_text("Build");
        descendants::<gtk::CheckButton>(&dialog)[0].set_active(false);
        dialog.press("Connect");
        settle();

        let expected = MapRequest {
            address: "sftp://anna@build:2222/home/anna".into(),
            label: "Build".into(),
            drive: None,
            keeping: ShareKeeping::ThisSessionOnly,
            credentials: CredentialChoice::Known,
        };
        assert_eq!(*requests.borrow(), [expected]);
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

        dialog.press("Finish");
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

        dialog.press("Finish");
        settle();
        assert!(started.get(), "Connect started the connection");
        dialog.press("Cancel");
        wait_for(Duration::from_millis(300));

        assert!(!answered.get(), "the late connection was dropped");
        parent.close();
    }
}
