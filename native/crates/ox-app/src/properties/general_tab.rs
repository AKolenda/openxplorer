// SPDX-License-Identifier: AGPL-3.0-only
//! The General tab of one item, laid out as Windows 11's (PROP-003,
//! PROP-004, PROP-005).
//!
//! The item's picture and name, then groups of rows between divider
//! lines: Type of file and Opens with (Change...), Location, Size, Size on
//! disk and, for a folder, Contains; Created, Modified and Accessed (a
//! folder shows Created only); then the Read-only and Hidden attributes.
//! Sizes give every byte (`314 KB (322,003 bytes)`) and dates the long
//! style (`Tuesday, October 20, 2026, 14:24:52`).
//!
//! A folder's Size, Size on disk and Contains count up by themselves
//! while the folder is measured off the main thread
//! ([`super::folder_count`]); closing the window stops the count.
//!
//! A folder a file system is mounted at, when it is not shown as a drive,
//! is a Mounted Volume as Windows shows a drive mounted in a folder: Type
//! says so, and in place of the sizes Target names the volume, with
//! Properties beside it opening the drive's own window; nothing is
//! counted.
//!
//! The name, Read-only and Hidden are held until OK or Apply
//! ([`GeneralEdits`]). Hidden is a leading dot in the name, so ticking it
//! renames `notes.txt` to `.notes.txt`, with the normal refusal when that
//! name is taken. Read-only takes every write permission from a file, or
//! gives its owner write again; a folder's box starts half ticked and
//! changes only the files inside it, as Windows' "Only applies to files in
//! folder" says.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gtk::glib;
use gtk::prelude::*;
use ox_core::entry::Entry;
use ox_core::format;
use ox_core::location::LocationContext;
use ox_core::permissions::{apply_outside_in_background, ModeChange, PermissionRequest};
use ox_core::transfer::Cancellation;
use ox_core::versions::PreviousVersions;

use super::folder_count::FolderCountRows;
use super::metadata::{ItemProperties, MountFacts};
use super::window::{divider, push_button, ApplyFuture, Pending, PropertiesWindow, Rows};
use crate::icons::{Art, ArtImage};

/// The size of the item's picture beside its name.
pub(super) const HEADER_ART_SIZE: i32 = 32;

/// A file's Opens with row without a default application.
const NO_DEFAULT_APP: &str = crate::i18n::message_id("No default application");

/// What the General tab may change.
#[derive(Debug, Clone, Copy)]
pub(super) struct GeneralRights {
    /// The name, and so Hidden, can be changed (PROP-005): not a standard
    /// folder, a share, a page or a previous version.
    pub can_rename: bool,
    /// The permissions, and so Read-only, can be changed: the user owns
    /// the item, outside previous versions and shares (PROP-007).
    pub can_change_permissions: bool,
    /// Opens with can be changed: a file outside previous versions.
    pub can_change_app: bool,
}

/// The General tab's page and the rows a folder count updates.
pub(super) struct GeneralTab {
    pub page: gtk::Box,
    pub folder_rows: Option<FolderCountRows>,
    /// The Opens with value, read again when the window comes back to
    /// the front after Change....
    pub opens_with: Option<gtk::Label>,
    /// The name, Read-only and Hidden, which follow the item when another
    /// tab changes it.
    pub edits: Rc<GeneralEdits>,
}

/// Fills the General tab of `properties`, registering its changes with
/// `window`.
pub(super) fn general_tab(
    window: &PropertiesWindow,
    properties: &ItemProperties,
    locations: &LocationContext,
    rights: GeneralRights,
    uri: &Rc<RefCell<String>>,
    versions: &Arc<PreviousVersions>,
) -> GeneralTab {
    let entry = &properties.entry;
    let is_folder = entry.is_dir;
    let page = super::window::page();
    let edits = GeneralEdits::new(window, properties, rights, Rc::clone(uri), Arc::clone(versions));
    page.append(&header(properties, &edits));
    page.append(&divider());

    let kind = Rows::new();
    let mut opens_with = None;
    if properties.mount.is_some() {
        kind.add(
            &ox_core::i18n::gettext("Type"),
            &ox_core::i18n::gettext("Mounted Volume"),
        );
    } else if is_folder {
        kind.add(&ox_core::i18n::gettext("Type"), &entry.type_label);
    } else {
        kind.add(&ox_core::i18n::gettext("Type of file"), &type_of_file(entry));
        let (row, value) = opens_with_row(properties, rights.can_change_app, uri);
        kind.add_control(&ox_core::i18n::gettext("Opens with"), &row);
        opens_with = Some(value);
    }
    page.append(kind.widget());
    page.append(&divider());

    let place = Rows::new();
    let container = properties.parent_uri.as_deref().unwrap_or(&entry.uri);
    place.add_line(
        &ox_core::i18n::gettext("Location"),
        &locations.display_location(container),
    );
    if let Some(target) = &properties.link_target {
        place.add_line(&ox_core::i18n::gettext("Target"), target);
    }
    let folder_rows = if let Some(mount) = &properties.mount {
        place.add_control(&ox_core::i18n::gettext("Target"), &volume_row(mount, uri));
        None
    } else if is_folder {
        let size = place.add(&ox_core::i18n::gettext("Size"), "");
        let on_disk = place.add(&ox_core::i18n::gettext("Size on disk"), "");
        let contains = place.add(&ox_core::i18n::gettext("Contains"), "");
        Some(FolderCountRows {
            size,
            on_disk,
            contains,
        })
    } else {
        let size = entry.size.map(format::exact_size_text).unwrap_or_default();
        place.add(&ox_core::i18n::gettext("Size"), &size);
        let on_disk = properties
            .size_on_disk
            .map(format::exact_size_text)
            .unwrap_or_default();
        place.add(&ox_core::i18n::gettext("Size on disk"), &on_disk);
        None
    };
    page.append(place.widget());
    page.append(&divider());

    let dates = Rows::new();
    dates.add(
        &ox_core::i18n::gettext("Created"),
        &format::long_date_time_text(properties.created),
    );
    if !is_folder {
        dates.add(
            &ox_core::i18n::gettext("Modified"),
            &format::long_date_time_text(entry.modified),
        );
        dates.add(
            &ox_core::i18n::gettext("Accessed"),
            &format::long_date_time_text(properties.accessed),
        );
    }
    page.append(dates.widget());
    page.append(&divider());

    let attributes = Rows::new();
    attributes.add_control(&ox_core::i18n::gettext("Attributes"), &edits.attribute_boxes());
    page.append(attributes.widget());
    window.add_pending(Rc::<GeneralEdits>::clone(&edits));
    GeneralTab {
        page,
        folder_rows,
        opens_with,
        edits,
    }
}

/// `PNG File (.png)`: the type and, when the name has one, its extension.
/// A shortcut is just "Shortcut": its name has no extension, and the
/// `.lnk` Windows shows there does not exist here ([`Entry::is_shortcut`]).
fn type_of_file(entry: &Entry) -> String {
    if entry.is_shortcut() {
        return entry.type_label.clone();
    }
    let extension = std::path::Path::new(&entry.name)
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|_| !entry.name.starts_with('.') || entry.name.matches('.').count() > 1);
    match extension {
        Some(extension) => ox_core::i18n::format_message(
            "{type} (.{extension})",
            &[("type", &entry.type_label), ("extension", extension)],
        ),
        None => entry.type_label.clone(),
    }
}

/// The application's name and Change..., which opens Open with for the
/// file as a dialog of the Properties window.
fn opens_with_row(
    properties: &ItemProperties,
    can_change: bool,
    uri: &Rc<RefCell<String>>,
) -> (gtk::Box, gtk::Label) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let app = properties
        .default_app
        .as_deref()
        .unwrap_or(ox_core::i18n::gettext_static(NO_DEFAULT_APP));
    let value = gtk::Label::builder()
        .label(app)
        .xalign(0.0)
        .hexpand(true)
        .selectable(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["property-value"])
        .build();
    row.append(&value);
    let change = push_button(&ox_core::i18n::gettext("Change..."));
    change.set_valign(gtk::Align::Center);
    change.set_sensitive(can_change);
    change.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext(
        "Change the app this file opens with",
    ))]);
    // Open with is a dialog of this Properties window, not of the browser
    // window, which may be behind other windows.
    // The file's location now: a rename on this tab changes it.
    let uri = Rc::clone(uri);
    change.connect_clicked(move |button| {
        let Some(window) = PropertiesWindow::of(button) else {
            return;
        };
        if let Some(opener) = window.opener() {
            opener.change_app_over(window.upcast_ref(), &uri.borrow());
        }
    });
    row.append(&change);
    (row, value)
}

/// A Mounted Volume's Target: the volume's label, else its type, and
/// Properties, which opens the drive's own Properties window, titled
/// with that name.
fn volume_row(mount: &MountFacts, uri: &Rc<RefCell<String>>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = if mount.label.is_empty() {
        mount.drive_type.label()
    } else {
        mount.label.clone()
    };
    row.append(
        &gtk::Label::builder()
            .label(&name)
            .xalign(0.0)
            .hexpand(true)
            .selectable(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["property-value"])
            .build(),
    );
    let properties = push_button(&ox_core::i18n::gettext("Properties"));
    properties.set_valign(gtk::Align::Center);
    properties.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext(
        "Properties of the mounted volume",
    ))]);
    // The folder's location now: a rename on this tab changes it.
    let uri = Rc::clone(uri);
    properties.connect_clicked(move |button| {
        let opener = PropertiesWindow::of(button).and_then(|window| window.opener());
        if let Some(opener) = opener {
            opener.open_drive_properties(&uri.borrow(), &name);
        }
    });
    row.append(&properties);
    row
}

/// The item's picture and its name: a field when it can be renamed,
/// where Enter presses OK.
fn header(properties: &ItemProperties, edits: &GeneralEdits) -> gtk::Box {
    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["properties-header"])
        .build();
    header.append(&ArtImage::new(Art::for_entry(&properties.entry), HEADER_ART_SIZE));
    header.append(&edits.name);
    header
}

/// A folder's Read-only box cycles through three states, as Windows'
/// does: unchanged (half ticked), read-only, and writable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadOnly {
    /// The files inside keep what they have.
    Unchanged,
    Set,
    Cleared,
}

/// The name, Read-only and Hidden as the user left them, until OK or
/// Apply.
pub(super) struct GeneralEdits {
    window: glib::WeakRef<PropertiesWindow>,
    /// The item's location, which a rename changes; the window shares it.
    uri: Rc<RefCell<String>>,
    /// The name the item has now.
    original_name: RefCell<String>,
    is_folder: bool,
    /// Renaming asks about a changed extension, except for a link to a
    /// folder, whose name has none (OPS-049).
    links_to_folder: bool,
    /// A symbolic link: while its name has no known type's extension it
    /// is a shortcut, whose name has none either, as Windows keeps a
    /// shortcut's hidden `.lnk` and never asks ([`Entry::is_shortcut`]).
    is_link: bool,
    name: gtk::Entry,
    read_only: gtk::CheckButton,
    /// Whether the item is read-only now; `None` for a folder, whose box
    /// starts half ticked.
    read_only_now: Cell<Option<bool>>,
    folder_read_only: Cell<ReadOnly>,
    hidden: gtk::CheckButton,
    /// Set while the tab changes a box itself.
    is_updating: Cell<bool>,
    /// Which locations are previous versions, which are never changed.
    versions: Arc<PreviousVersions>,
}

impl GeneralEdits {
    fn new(
        window: &PropertiesWindow,
        properties: &ItemProperties,
        rights: GeneralRights,
        uri: Rc<RefCell<String>>,
        versions: Arc<PreviousVersions>,
    ) -> Rc<Self> {
        let entry = &properties.entry;
        let name = gtk::Entry::builder()
            .text(&entry.name)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .activates_default(true)
            .editable(rights.can_rename)
            .can_focus(true)
            .css_classes(["properties-name"])
            .build();
        name.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext("Name"))]);
        let read_only_label = if entry.is_dir {
            ox_core::i18n::gettext("Read-only (Only applies to files in folder)")
        } else {
            ox_core::i18n::gettext("Read-only")
        };
        let read_only = gtk::CheckButton::with_mnemonic(&read_only_label);
        let read_only_now = (!entry.is_dir).then(|| properties.mode.is_some_and(|mode| mode & 0o200 == 0));
        read_only.set_active(read_only_now == Some(true));
        read_only.set_inconsistent(entry.is_dir);
        read_only.set_sensitive(rights.can_change_permissions);
        let hidden = gtk::CheckButton::with_mnemonic(&ox_core::i18n::gettext("Hidden"));
        hidden.set_active(entry.name.starts_with('.'));
        hidden.set_sensitive(rights.can_rename);
        let edits = Rc::new(Self {
            window: window.downgrade(),
            uri,
            original_name: RefCell::new(entry.name.clone()),
            is_folder: entry.is_dir,
            links_to_folder: properties.links_to_folder,
            is_link: entry.is_symlink,
            name,
            read_only,
            read_only_now: Cell::new(read_only_now),
            folder_read_only: Cell::new(ReadOnly::Unchanged),
            hidden,
            is_updating: Cell::new(false),
            versions,
        });
        edits.connect_controls();
        edits
    }

    fn connect_controls(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.name.connect_changed(move |_| {
            if let Some(edits) = weak.upgrade() {
                edits.notify_changed();
            }
        });
        let weak = Rc::downgrade(self);
        self.hidden.connect_toggled(move |_| {
            if let Some(edits) = weak.upgrade() {
                edits.notify_changed();
            }
        });
        let weak = Rc::downgrade(self);
        self.read_only.connect_toggled(move |_| {
            if let Some(edits) = weak.upgrade() {
                edits.read_only_toggled();
            }
        });
    }

    /// A folder's box moves on to its next state; a file's is ticked or
    /// cleared.
    fn read_only_toggled(&self) {
        if self.is_updating.get() {
            return;
        }
        if self.is_folder {
            let next = match self.folder_read_only.get() {
                ReadOnly::Unchanged => ReadOnly::Set,
                ReadOnly::Set => ReadOnly::Cleared,
                ReadOnly::Cleared => ReadOnly::Unchanged,
            };
            self.show_folder_read_only(next);
        }
        self.notify_changed();
    }

    fn show_folder_read_only(&self, state: ReadOnly) {
        self.folder_read_only.set(state);
        self.is_updating.set(true);
        self.read_only.set_inconsistent(state == ReadOnly::Unchanged);
        self.read_only.set_active(state == ReadOnly::Set);
        self.is_updating.set(false);
    }

    /// After Security changed the item's permissions, a file's Read-only
    /// compares with its mode `mode`, and shows it unless the box holds a
    /// change not applied, which stays.
    pub(super) fn follow_mode(&self, mode: Option<u32>) {
        if self.is_folder {
            return;
        }
        let has_choice = self.read_only_change().is_some();
        let read_only = mode.is_some_and(|mode| mode & 0o200 == 0);
        self.read_only_now.set(Some(read_only));
        if !has_choice {
            self.is_updating.set(true);
            self.read_only.set_active(read_only);
            self.is_updating.set(false);
        }
        self.notify_changed();
    }

    fn notify_changed(&self) {
        if let Some(window) = self.window.upgrade() {
            window.changed();
        }
    }

    /// Read-only and Hidden: side by side for a file, one above the other
    /// for a folder, whose Read-only label is long.
    fn attribute_boxes(&self) -> gtk::Box {
        let orientation = if self.is_folder {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        };
        let boxes = gtk::Box::builder()
            .orientation(orientation)
            .css_classes(["properties-attributes"])
            .build();
        boxes.append(&self.read_only);
        boxes.append(&self.hidden);
        boxes
    }

    /// The name the item should have: the field's, with a leading dot for
    /// Hidden and none without it.
    fn wanted_name(&self) -> String {
        let typed = self.name.text().to_string();
        let was_hidden = self.original_name.borrow().starts_with('.');
        let hide = self.hidden.is_active();
        if hide == was_hidden {
            return typed;
        }
        if hide {
            format!(".{typed}")
        } else {
            typed.trim_start_matches('.').to_owned()
        }
    }

    /// The read-only change to make, if any.
    fn read_only_change(&self) -> Option<bool> {
        if self.is_folder {
            return match self.folder_read_only.get() {
                ReadOnly::Unchanged => None,
                ReadOnly::Set => Some(true),
                ReadOnly::Cleared => Some(false),
            };
        }
        let wanted = self.read_only.is_active();
        (Some(wanted) != self.read_only_now.get()).then_some(wanted)
    }

    async fn apply_now(self: Rc<Self>) -> Result<(), String> {
        let window = self.window.upgrade();
        let opener = window.as_ref().and_then(PropertiesWindow::opener);
        if let Some(read_only) = self.read_only_change() {
            let uri = self.uri.borrow().clone();
            // A location registered as a previous version since the window
            // opened is never changed (PROP-024).
            self.versions
                .check_writable(&uri)
                .map_err(|refusal| refusal.to_string())?;
            // No drive is formatted while the item changes (DEV-012).
            let _writing = match &opener {
                Some(opener) => Some(opener.hold_write()?),
                None => None,
            };
            let request = PermissionRequest {
                mode: ModeChange::ReadOnlyFiles(read_only),
                owner: None,
                group: None,
                recursive: self.is_folder,
            };
            let submitted = self.folder_read_only.get();
            // Backups inside a folder are left alone too.
            let protected = self.versions.protected_locations();
            apply_outside_in_background(uri, request, Cancellation::new(), protected)
                .await
                .map_err(|error| error.to_string())?;
            if self.is_folder {
                // A box changed while the change was made stays to apply.
                if self.folder_read_only.get() == submitted {
                    self.show_folder_read_only(ReadOnly::Unchanged);
                }
            } else {
                self.read_only_now.set(Some(read_only));
            }
        }
        let wanted = self.wanted_name();
        let original = self.original_name.borrow().clone();
        if wanted != original {
            let Some(opener) = opener else {
                return Err(ox_core::i18n::gettext(
                    "The window that opened these properties is closed.",
                ));
            };
            let uri = self.uri.borrow().clone();
            // Asked of the current name: a rename can give a link a known
            // type's extension, or take it away.
            let has_no_extension = self.is_folder
                || self.links_to_folder
                || (self.is_link && ox_core::entry::known_extension(&original).is_none());
            let parent = window.as_ref().map_or_else(
                || opener.clone().upcast::<gtk::Window>(),
                |window| window.clone().upcast(),
            );
            // The name and Hidden wait while the rename runs, so they show
            // what it made rather than losing an edit made meanwhile.
            let (editable, can_hide) = (self.name.is_editable(), self.hidden.is_sensitive());
            self.name.set_editable(false);
            self.hidden.set_sensitive(false);
            let renaming = opener
                .rename_item_at(&parent, &uri, &original, &wanted, has_no_extension)
                .await;
            self.name.set_editable(editable);
            self.hidden.set_sensitive(can_hide);
            renaming?;
            let renamed = gtk::gio::File::for_uri(&uri)
                .parent()
                .map(|parent| parent.child(&wanted).uri().to_string());
            if let Some(renamed) = renamed {
                self.uri.replace(renamed);
            }
            self.original_name.replace(wanted.clone());
            self.is_updating.set(true);
            self.name.set_text(&wanted);
            self.hidden.set_active(wanted.starts_with('.'));
            self.is_updating.set(false);
            if let Some(window) = &window {
                window.set_title_text(&ox_core::i18n::format_message(
                    "{title} Properties",
                    &[("title", &wanted)],
                ));
            }
        }
        Ok(())
    }
}

impl Pending for GeneralEdits {
    fn has_changes(&self) -> bool {
        self.wanted_name() != *self.original_name.borrow() || self.read_only_change().is_some()
    }

    fn apply(self: Rc<Self>) -> ApplyFuture {
        Box::pin(self.apply_now())
    }
}
