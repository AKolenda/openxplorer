// SPDX-License-Identifier: AGPL-3.0-only
//! The Properties window of one item (PROP-001, PROP-003, PROP-006).
//!
//! A file has General, Security, Details and Previous Versions; a folder
//! on this computer adds Sharing (where Samba user shares can be used)
//! and Customize, and a standard folder Location, in Windows 11's order:
//! General, Sharing, Security, Location, Details, Previous Versions,
//! Customize. A drive, a folder the kernel mounts a file system at that
//! This PC lists or the root, has Windows' drive General tab instead of a
//! folder's, and a Hardware tab after it ([`super::drive_tab`]); any other
//! mount point is a Mounted Volume, a folder whose General tab names the
//! volume, as Windows shows a drive mounted in a folder. The item's properties are read once,
//! off the main thread, when the window opens; the versions and a file's
//! media details are read the first time their tab is shown, and a
//! folder's size is counted from the start, never a drive's.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gtk::glib;
use gtk::prelude::*;
use ox_core::entry::EntryError;
use ox_core::folder_locations::FolderRelocation;
use ox_core::integration::BraveIntegration;
use ox_core::location::{is_smb_location, is_smb_server, is_smb_share_root, ItemKind, LocationContext};
use ox_core::network::Usershares;
use ox_core::transfer::Cancellation;
use ox_core::versions::{is_conventional_snapshot, PreviousVersions};

use super::details_tab::DetailsPanel;
use super::drive_tab::{drive_general_tab, hardware_rows, DRIVE_ART};
use super::folder_count::{count_folders, Totals};
use super::general_tab::{general_tab, GeneralEdits, GeneralRights};
use super::location_panel::LocationPanel;
use super::metadata::{is_mount_point, read_properties, ItemProperties};
use super::security_tab::{fill_security, SecurityContext, SecurityFacts};
use super::sharing_panel::sharing_panel;
use super::versions_panel::VersionsPanel;
use super::window::{page, PropertiesWindow};
use super::{PropertiesTab, PropertiesTarget, READING};
use crate::dialog::{note, quiet_text};
use crate::icons::Art;
use crate::window::BrowserWindow;

/// What a Properties window needs from the window that opens it.
#[derive(Debug, Clone)]
pub(crate) struct PropertiesContext {
    /// The previous-versions service every window shares.
    pub versions: Arc<PreviousVersions>,
    /// Display names for the home folder and devices.
    pub locations: LocationContext,
    /// Samba's user shares, for the Sharing tab; `None` for no tab.
    pub usershares: Option<Usershares>,
    /// Moves a standard folder, for the Location tab.
    pub relocation: Arc<FolderRelocation>,
    /// Brave's download-folder integration, for the Location tab's
    /// follow-up.
    pub brave: BraveIntegration,
}

/// The pages filled once the item is read.
struct ItemPages {
    window: glib::WeakRef<PropertiesWindow>,
    target: PropertiesTarget,
    context: PropertiesContext,
    /// The item's location; a rename on the General tab changes it.
    uri: Rc<RefCell<String>>,
    /// The location every tab works on, until a rename reaches them.
    shown_uri: RefCell<String>,
    /// The Previous Versions tab.
    versions: VersionsPanel,
    /// A file's Details tab.
    details: Option<DetailsPanel>,
    /// Points the Sharing tab at a renamed folder.
    share_folder: Option<Rc<dyn Fn(std::path::PathBuf)>>,
    general: gtk::Box,
    /// A drive's Hardware tab.
    hardware: Option<gtk::Box>,
    /// The General tab's edits, once it is filled; a drive's has none.
    general_edits: RefCell<Option<Rc<GeneralEdits>>>,
    security: gtk::Box,
    customize: Option<gtk::Box>,
    /// Stops the folder count.
    count: Cancellation,
}

/// Opens the Properties window of `target` on `initial` (General when the
/// item has no such tab), from `opener`.
pub(crate) fn open_item_properties(
    opener: &BrowserWindow,
    target: PropertiesTarget,
    context: PropertiesContext,
    initial: PropertiesTab,
) -> PropertiesWindow {
    let local_folder = local_folder(&target);
    let is_drive = target.as_drive && local_folder.is_some() && is_mount_point(&target.uri);
    let art = if is_drive {
        Some(DRIVE_ART)
    } else {
        (target.kind == ItemKind::Folder).then_some(Art::Folder)
    };
    let window = PropertiesWindow::new(&target.dialog_title(), art, Some(opener));
    window.set_widget_name(&target.uri);
    window.set_shows_drive(target.as_drive);
    let general = page();
    general.append(&quiet_text(ox_core::i18n::gettext_static(READING)));
    window.add_tab(
        PropertiesTab::General.page_name(),
        PropertiesTab::General.label(),
        &general,
    );
    let hardware = is_drive.then(|| add_hardware_tab(&window));
    let mut share_folder = None;
    if let Some((folder, usershares)) = local_folder.clone().zip(context.usershares.as_ref()) {
        let (sharing, follow) = sharing_panel(folder, usershares.clone());
        window.add_tab(
            PropertiesTab::Sharing.page_name(),
            PropertiesTab::Sharing.label(),
            &sharing,
        );
        share_folder = Some(follow);
    }
    let security = page();
    window.add_tab(
        PropertiesTab::Security.page_name(),
        PropertiesTab::Security.label(),
        &security,
    );
    if let Some(folder) = target.known_folder {
        let location = LocationPanel::new(folder, Arc::clone(&context.relocation), context.brave.clone());
        window.add_pending_last(location.pending());
        window.add_tab(
            PropertiesTab::Location.page_name(),
            PropertiesTab::Location.label(),
            &location,
        );
    }
    let details = add_details_tab(&window, &target, &context);
    let versions = VersionsPanel::new(&target, Arc::clone(&context.versions), context.locations.clone());
    window.add_tab(
        PropertiesTab::PreviousVersions.page_name(),
        PropertiesTab::PreviousVersions.label(),
        &versions,
    );
    let is_read_only =
        context.locations.is_snapshot_location(&target.uri) || is_conventional_snapshot(&target.uri);
    let customize = (local_folder.is_some() && !is_read_only).then(|| {
        let customize = page();
        window.add_tab(
            PropertiesTab::Customize.page_name(),
            PropertiesTab::Customize.label(),
            &customize,
        );
        customize
    });
    follow_shown_tabs(&window, &versions, details.as_ref(), initial);
    let closing = versions.clone();
    window.connect_closed_once(move || closing.cancel());
    let count = Cancellation::new();
    window.track_work(count.clone());
    let pages = Rc::new(ItemPages {
        window: window.downgrade(),
        uri: Rc::new(RefCell::new(target.uri.clone())),
        shown_uri: RefCell::new(target.uri.clone()),
        versions,
        details,
        share_folder,
        target,
        context,
        general,
        hardware,
        general_edits: RefCell::new(None),
        security,
        customize,
        count,
    });
    pages.read(true);
    let applied = Rc::downgrade(&pages);
    window.connect_applied(move || {
        if let Some(pages) = applied.upgrade() {
            pages.applied();
        }
    });
    // The window keeps its pages, which the Security tab's dialogs read
    // the item again into, until it closes.
    let kept = RefCell::new(Some(pages));
    window.connect_closed_once(move || {
        kept.take();
    });
    window
}

/// A drive's Hardware tab, in Windows' order: General, Tools, Hardware,
/// Sharing, Security.
fn add_hardware_tab(window: &PropertiesWindow) -> gtk::Box {
    let hardware = page();
    window.add_tab(
        PropertiesTab::Hardware.page_name(),
        PropertiesTab::Hardware.label(),
        &hardware,
    );
    hardware
}

/// A file's Details tab, as Windows gives files and never folders.
fn add_details_tab(
    window: &PropertiesWindow,
    target: &PropertiesTarget,
    context: &PropertiesContext,
) -> Option<DetailsPanel> {
    if target.kind == ItemKind::Folder {
        return None;
    }
    let details = DetailsPanel::new(context.locations.clone());
    window.add_tab(
        PropertiesTab::Details.page_name(),
        PropertiesTab::Details.label(),
        details.page(),
    );
    details.follow_window(window);
    Some(details)
}

/// Shows `initial`, and reads the versions and a file's media details the
/// first time their tab is shown; F5 looks for versions again.
fn follow_shown_tabs(
    window: &PropertiesWindow,
    versions: &VersionsPanel,
    details: Option<&DetailsPanel>,
    initial: PropertiesTab,
) {
    let refreshed = versions.clone();
    window.connect_refresh(move |name| {
        if name == PropertiesTab::PreviousVersions.page_name() {
            refreshed.load();
        }
    });
    let shown_versions = versions.clone();
    let shown_details = details.cloned();
    let shown = move |name: &str| {
        if name == PropertiesTab::PreviousVersions.page_name() {
            shown_versions.load_once();
        } else if name == PropertiesTab::Details.page_name() {
            if let Some(details) = &shown_details {
                details.shown();
            }
        }
    };
    window.select_tab(initial.page_name());
    if let Some(selected) = window.selected_tab() {
        shown(&selected);
    }
    window.connect_tab_shown(shown);
}

/// The path of a folder on this computer, which can be shared and
/// customized; `None` for files and for folders elsewhere (NET-035).
fn local_folder(target: &PropertiesTarget) -> Option<std::path::PathBuf> {
    if target.kind != ItemKind::Folder || !target.uri.starts_with("file:") {
        return None;
    }
    gtk::gio::File::for_uri(&target.uri).path()
}

impl ItemPages {
    /// After OK or Apply: a rename reaches every tab, and the Security tab
    /// reads the item again, as General may have changed its permissions.
    fn applied(self: &Rc<Self>) {
        let current = self.uri.borrow().clone();
        if *self.shown_uri.borrow() != current {
            self.shown_uri.replace(current.clone());
            if let Some(window) = self.window.upgrade() {
                // Properties of the renamed item come back to this window.
                window.set_widget_name(&current);
            }
            self.versions.set_item_uri(&current);
            if let Some((follow, path)) = self
                .share_folder
                .as_ref()
                .zip(gtk::gio::File::for_uri(&current).path())
            {
                follow(path);
            }
        }
        self.read(false);
    }

    /// Reads the item off the main thread, then fills every page the
    /// first time, or only the Security tab after one of its dialogs
    /// changed the item.
    fn read(self: &Rc<Self>, first: bool) {
        let uri = self.uri.borrow().clone();
        let pages = Rc::clone(self);
        glib::spawn_future_local(async move {
            let read = read_properties(uri).await;
            let Some(window) = pages.window.upgrade().filter(|window| !window.is_closed()) else {
                return;
            };
            pages.show(&window, read, first);
        });
    }

    fn show(
        self: &Rc<Self>,
        window: &PropertiesWindow,
        read: Result<ItemProperties, EntryError>,
        first: bool,
    ) {
        let properties = match read {
            Ok(properties) => properties,
            Err(error) => {
                if first {
                    clear(&self.general);
                    self.general.append(&note(&error.to_string()));
                    clear(&self.security);
                    self.security.append(&quiet_text(&ox_core::i18n::gettext(
                        "Metadata could not be read.",
                    )));
                }
                return;
            }
        };
        let can_edit = can_edit_permissions(&properties, &self.context);
        let pages = Rc::downgrade(self);
        let security_context = SecurityContext {
            versions: Arc::clone(&self.context.versions),
            reread: Rc::new(move || {
                if let Some(pages) = pages.upgrade() {
                    pages.read(false);
                }
            }),
        };
        let facts = SecurityFacts {
            uri: properties.entry.uri.clone(),
            name: properties.entry.name.clone(),
            object_name: self.context.locations.display_location(&properties.entry.uri),
            owner: properties.owner_account(),
            group: properties.group_account(),
            mode: properties.mode.map(|mode| mode & 0o7777),
            mode_text: properties.mode_text(),
            is_folder: properties.entry.is_dir,
            can_edit,
        };
        fill_security(&self.security, &facts, &security_context);
        // A rename or a permission change shows in the File rows.
        if let Some(details) = &self.details {
            details.show_properties(&properties);
        }
        if !first {
            if let Some(edits) = self.general_edits.borrow().as_ref() {
                edits.follow_mode(properties.mode);
            }
            return;
        }
        if let Some(mount) = properties.mount.as_ref().filter(|_| self.target.as_drive) {
            window.set_art(DRIVE_ART);
            clear(&self.general);
            let general = drive_general_tab(&properties.entry.uri, mount, &self.context.locations);
            self.general.append(&general);
            if let Some(hardware) = &self.hardware {
                hardware.append(&hardware_rows(mount));
            }
            self.fill_customize(window, &properties);
            return;
        }
        window.set_art(Art::for_entry(&properties.entry));
        let is_read_only = self.context.locations.is_snapshot_location(&properties.entry.uri)
            || is_conventional_snapshot(&properties.entry.uri);
        let rights = GeneralRights {
            can_rename: self.can_rename(&properties),
            can_change_permissions: can_edit,
            can_change_app: !properties.entry.is_dir && !is_read_only,
        };
        clear(&self.general);
        let general = general_tab(
            window,
            &properties,
            &self.context.locations,
            rights,
            &self.uri,
            &self.context.versions,
        );
        self.general.append(&general.page);
        self.general_edits.replace(Some(Rc::clone(&general.edits)));
        if let Some(rows) = &general.folder_rows {
            if is_smb_server(&properties.entry.uri) {
                rows.show(Totals::zero());
            } else {
                count_folders(
                    vec![properties.entry.uri.clone()],
                    Totals::zero(),
                    rows,
                    &self.count,
                );
            }
        }
        if let Some(opens_with) = general.opens_with {
            follow_default_app(window, &opens_with, properties.entry.content_type.clone());
        }
        self.fill_customize(window, &properties);
    }

    fn fill_customize(&self, window: &PropertiesWindow, properties: &ItemProperties) {
        if let Some(customize) = &self.customize {
            let tab = super::custom_icon::customize_tab(window, &self.uri, properties.custom_icon.as_deref());
            customize.append(&tab);
        }
    }

    /// Whether the name can be edited: an item in a local or shared folder,
    /// not a standard folder, a share or server, or inside a previous version.
    fn can_rename(&self, properties: &ItemProperties) -> bool {
        can_rename(properties, &self.context, self.target.known_folder.is_some())
    }
}

/// Whether the item `properties` describes can be renamed, and so hidden
/// (PROP-005): not a standard folder, a share, a page or anything inside a
/// previous version.
pub(super) fn can_rename(
    properties: &ItemProperties,
    context: &PropertiesContext,
    is_standard_folder: bool,
) -> bool {
    let uri = &properties.entry.uri;
    let is_share = is_smb_server(uri) || is_smb_share_root(uri);
    let is_read_only = context.locations.is_snapshot_location(uri) || is_conventional_snapshot(uri);
    let is_renamable_place = uri.starts_with("file:") || is_smb_location(uri);
    !is_standard_folder && properties.parent_uri.is_some() && !is_share && !is_read_only && is_renamable_place
}

/// Reads the file type's default application again each time the window
/// comes back to the front, as after Change... chose another.
fn follow_default_app(window: &PropertiesWindow, opens_with: &gtk::Label, content_type: Option<String>) {
    let Some(content_type) = content_type else {
        return;
    };
    let label = opens_with.downgrade();
    window.connect_is_active_notify(move |window| {
        let Some(label) = label.upgrade().filter(|_| window.is_active()) else {
            return;
        };
        if let Some(app) = gtk::gio::AppInfo::default_for_type(&content_type, false) {
            label.set_text(&app.display_name());
        }
    });
}

/// Whether the permissions can be changed here: the user owns the
/// item, which has permission bits and is not a link, a share root or
/// inside a previous version (PROP-007).
pub(super) fn can_edit_permissions(properties: &ItemProperties, context: &PropertiesContext) -> bool {
    let uri = &properties.entry.uri;
    let is_owner = properties.owner.as_deref() == glib::user_name().to_str();
    let is_link = properties.link_target.is_some();
    let is_read_only = context.locations.is_snapshot_location(uri) || is_conventional_snapshot(uri);
    let is_place = uri.starts_with("file:") || (is_smb_location(uri) && !is_smb_share_root(uri));
    is_owner && properties.mode.is_some() && !is_link && !is_read_only && is_place && !is_smb_server(uri)
}

/// Removes every child of `panel`.
fn clear(panel: &gtk::Box) {
    while let Some(child) = panel.first_child() {
        panel.remove(&child);
    }
}
