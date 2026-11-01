// SPDX-License-Identifier: AGPL-3.0-only
//! The Properties window of one item (PROP-001, PROP-003, PROP-006).
//!
//! A file has General, Security and Previous Versions; a folder on this
//! computer adds Sharing (where Samba user shares can be used) and
//! Customize, and a standard folder Location, in Windows 11's order:
//! General, Sharing, Security, Location, Previous Versions, Customize.
//! The item's properties are read once, off the main thread, when the
//! window opens; the versions are looked up the first time their tab is
//! shown, and a folder's size is counted from the start.

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

use super::folder_count::{count_folders, Totals};
use super::general_tab::{general_tab, GeneralRights};
use super::location_panel::LocationPanel;
use super::metadata::{read_properties, ItemProperties};
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
    general: gtk::Box,
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
    let art = (target.kind == ItemKind::Folder).then_some(Art::Folder);
    let window = PropertiesWindow::new(&target.dialog_title(), art, Some(opener));
    window.set_widget_name(&target.uri);
    let general = page();
    general.append(&quiet_text(ox_core::i18n::gettext_static(READING)));
    window.add_tab(
        PropertiesTab::General.page_name(),
        PropertiesTab::General.label(),
        &general,
    );
    let local_folder = local_folder(&target);
    if let Some((folder, usershares)) = local_folder.clone().zip(context.usershares.as_ref()) {
        let sharing = sharing_panel(folder, usershares.clone());
        window.add_tab(
            PropertiesTab::Sharing.page_name(),
            PropertiesTab::Sharing.label(),
            &sharing,
        );
    }
    let security = page();
    window.add_tab(
        PropertiesTab::Security.page_name(),
        PropertiesTab::Security.label(),
        &security,
    );
    if let Some(folder) = target.known_folder {
        let location = LocationPanel::new(folder, Arc::clone(&context.relocation), context.brave.clone());
        window.add_tab(
            PropertiesTab::Location.page_name(),
            PropertiesTab::Location.label(),
            &location,
        );
    }
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
    let shown_versions = versions.clone();
    window.connect_tab_shown(move |name| {
        if name == PropertiesTab::PreviousVersions.page_name() {
            shown_versions.load_once();
        }
    });
    window.select_tab(initial.page_name());
    if window.selected_tab().as_deref() == Some(PropertiesTab::PreviousVersions.page_name()) {
        versions.load_once();
    }
    let closing = versions.clone();
    window.connect_closed_once(move || closing.cancel());
    let count = Cancellation::new();
    window.track_work(count.clone());
    let pages = Rc::new(ItemPages {
        window: window.downgrade(),
        uri: Rc::new(RefCell::new(target.uri.clone())),
        target,
        context,
        general,
        security,
        customize,
        count,
    });
    pages.read(true);
    // The window keeps its pages, which the Security tab's dialogs read
    // the item again into, until it closes.
    let kept = RefCell::new(Some(pages));
    window.connect_closed_once(move || {
        kept.take();
    });
    window
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
        if !first {
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
            Rc::clone(&self.uri),
        );
        self.general.append(&general.page);
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
        if let Some(customize) = &self.customize {
            let tab = super::custom_icon::customize_tab(
                window,
                &properties.entry.uri,
                properties.custom_icon.as_deref(),
            );
            customize.append(&tab);
        }
    }

    /// Whether the name can be edited: an item in a local or shared folder,
    /// not a standard folder, a share or server, or inside a previous version.
    fn can_rename(&self, properties: &ItemProperties) -> bool {
        let uri = &properties.entry.uri;
        let is_share = is_smb_server(uri) || is_smb_share_root(uri);
        let is_read_only = self.context.locations.is_snapshot_location(uri) || is_conventional_snapshot(uri);
        let is_renamable_place = uri.starts_with("file:") || is_smb_location(uri);
        self.target.known_folder.is_none()
            && properties.parent_uri.is_some()
            && !is_share
            && !is_read_only
            && is_renamable_place
    }
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
