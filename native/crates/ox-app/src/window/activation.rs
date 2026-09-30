// SPDX-License-Identifier: AGPL-3.0-only
//! Opening items and typed addresses: folders open in the tab, files in
//! their default application.
//!
//! Ports `openEntry`, `submitAddress` and `openIncoming` in
//! `desktop/ui/app.js` and `activation_kind` in `desktop/activation.py`.
//! An address or command-line argument is looked up first, so a file is
//! opened without moving the tab or adding a history entry, and a typed
//! page title ("Network") names a folder of that name when one exists.
//! Only an explicit request (Enter, double-click, Open, a typed address or
//! a command-line argument) ever launches an application.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::{self, Entry, EntryError, EntryKind};
use ox_core::integration;

use crate::locations::{self, Page};

use super::desktop_link::{link_target_of_file, may_be_link, LinkTarget};
use super::dialog::{ButtonStyle, Dialog};
use super::session::TabId;
use super::software_search::{self, FIND_IN_SOFTWARE};
use super::BrowserWindow;

/// Why an item cannot be opened (`activation_kind` in activation.py).
const NOT_OPENABLE: &str = "This item is not a regular file or a readable folder.";

/// The title of the dialog that says why an item did not open.
const OPEN_FAILED: &str = "Could not open the item";

/// Where an activation started: its tab, and how often that tab had moved
/// to another location by then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActivationOrigin {
    tab: TabId,
    moves: u64,
}

/// What is left to do once an activated item was read again.
#[derive(Debug)]
enum Resolved {
    /// Open this folder in the tab.
    Folder(String),
    /// Browse this ZIP archive.
    Archive(Entry),
    /// The file opened in its application.
    Opened,
}

/// What activating an item does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Activation {
    /// Open this folder in the tab.
    Folder(String),
    /// Open the file in its default application.
    File,
    /// Browse the ZIP archive in the archive browser (ARC-002).
    Archive,
    /// Refuse, with this message.
    Refused(&'static str),
}

/// Where a folder from the command line or another app opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IncomingTab {
    /// The active tab moves to it: the first location, as `openIncoming`.
    Active,
    /// A new tab in front: every later location.
    New,
}

/// What activating `entry` does, from freshly queried metadata.
pub(super) fn activation_for(entry: &Entry) -> Activation {
    let not_a_file = !matches!(
        entry.kind,
        EntryKind::File | EntryKind::Special | EntryKind::Symlink
    );
    if entry.kind == EntryKind::Directory || (not_a_file && entry.is_dir) {
        return Activation::Folder(entry.navigation_uri().to_owned());
    }
    if matches!(
        entry.kind,
        EntryKind::Special | EntryKind::Unknown | EntryKind::Symlink
    ) {
        return Activation::Refused(NOT_OPENABLE);
    }
    if integration::Activation::for_entry(entry) == Ok(integration::Activation::BrowseArchive) {
        return Activation::Archive;
    }
    Activation::File
}

/// Where the local `.desktop` link file `entry` points, if it is one.
fn desktop_link(entry: &Entry) -> Option<Result<LinkTarget, String>> {
    if !may_be_link(entry.content_type.as_deref(), &entry.name) {
        return None;
    }
    let path = gio::File::for_uri(&entry.uri).path()?;
    let target = link_target_of_file(&path)?;
    Some(target.map_err(str::to_owned))
}

/// Queries `uri` without blocking the interface.
pub(super) async fn query_entry(uri: &str) -> Result<Entry, EntryError> {
    let file = gio::File::for_uri(uri);
    let info = file
        .query_info_future(
            entry::ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await?;
    Ok(entry::entry_from_info(&file, &info))
}

impl BrowserWindow {
    /// Opens the item at a display position (Enter, double-click, Open),
    /// as `openEntry` does: its metadata is read again rather than
    /// trusted, one item of a tab opens at a time, and the result belongs
    /// to the tab that asked. It is dropped when that tab moved elsewhere
    /// or closed meanwhile; a folder opens in that tab even when another
    /// one is in front by then (OPEN-001, OPEN-004).
    pub(super) fn activate_item(&self, position: u32) {
        let Some(item) = self.folder_pane().model().item(position) else {
            return;
        };
        let entry = item.entry().clone();
        let Some(origin) = self.begin_activation() else {
            return;
        };
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let outcome = window.resolve_activation(&entry).await;
                if window.end_activation(origin) {
                    window.finish_activation(origin, &entry, outcome);
                }
            }
        ));
    }

    /// Marks the active tab as opening an item; `None` while it already
    /// is, or before the window has a tab.
    fn begin_activation(&self) -> Option<ActivationOrigin> {
        let mut session = self.imp().session.borrow_mut();
        let tab = session.active_mut()?;
        if tab.is_activating {
            return None;
        }
        tab.is_activating = true;
        Some(ActivationOrigin {
            tab: tab.id,
            moves: tab.history.moves(),
        })
    }

    /// Ends the activation `origin` started; true when its tab is still
    /// open and still at the location it was activated in.
    fn end_activation(&self, origin: ActivationOrigin) -> bool {
        let mut session = self.imp().session.borrow_mut();
        let Some(tab) = session.tab_mut(origin.tab) else {
            return false;
        };
        tab.is_activating = false;
        tab.history.moves() == origin.moves
    }

    /// Reads `entry` again and opens a file at once; says what else to do.
    async fn resolve_activation(&self, entry: &Entry) -> Result<Resolved, String> {
        let fresh = query_entry(entry.navigation_uri())
            .await
            .map_err(|error| error.to_string())?;
        match activation_for(&fresh) {
            Activation::Folder(uri) => Ok(Resolved::Folder(uri)),
            Activation::Archive => Ok(Resolved::Archive(fresh)),
            Activation::Refused(message) => Err(message.to_owned()),
            Activation::File => {
                if let Some(target) = desktop_link(&fresh) {
                    return self.follow_link(target?).await;
                }
                let window = self.upcast_ref::<gtk::Window>();
                self.context().open_file(&fresh, window).await?;
                Ok(Resolved::Opened)
            }
        }
    }

    /// Goes where a `.desktop` link points: a folder in the tab, a web
    /// page or mail address in the desktop's handler (OPEN-009).
    async fn follow_link(&self, target: LinkTarget) -> Result<Resolved, String> {
        match target {
            LinkTarget::Location(uri) => Ok(Resolved::Folder(uri)),
            LinkTarget::Web(url) => {
                gtk::UriLauncher::new(&url)
                    .launch_future(Some(self.upcast_ref::<gtk::Window>()))
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(Resolved::Opened)
            }
        }
    }

    /// Shows the result of an activation in the tab `origin` names.
    fn finish_activation(&self, origin: ActivationOrigin, entry: &Entry, outcome: Result<Resolved, String>) {
        let is_active = self.imp().session.borrow().is_active(origin.tab);
        match outcome {
            Ok(Resolved::Folder(uri)) if is_active => self.navigate_or_report(&uri),
            Ok(Resolved::Folder(uri)) => self.navigate_background_tab(origin.tab, &uri),
            Ok(Resolved::Archive(archive)) if is_active => self.open_archive(&archive),
            Ok(Resolved::Archive(_) | Resolved::Opened) => {}
            Err(reason) if is_active => self.report_open_failure(&reason, entry),
            Err(reason) => self.show_message(&format!("Could not open {}: {reason}", entry.name)),
        }
    }

    /// Moves the background tab `id` to the folder `uri`; it is listed
    /// when it is next shown.
    fn navigate_background_tab(&self, id: TabId, uri: &str) {
        let Ok(uri) = self.resolve_address(uri) else {
            return;
        };
        let stale = {
            let mut session = self.imp().session.borrow_mut();
            let Some(tab) = session.tab_mut(id) else {
                return;
            };
            tab.history.push(&uri);
            tab.forget_location_state();
            tab.mark_stale()
        };
        stale.remove_all();
        self.render_tabs();
    }

    /// Says why `entry` could not be opened in a dialog, as
    /// `showMessage('Could not open the item', …)` does. When no
    /// application opens its type, the dialog offers to find one in
    /// Software (OPEN-010).
    pub(super) fn report_open_failure(&self, reason: &str, entry: &Entry) {
        let reason = reason.to_owned();
        let unhandled = software_search::unhandled_type(&reason, entry.content_type.as_deref())
            .filter(|_| software_search::is_available());
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let dialog = Dialog::new(&window, OPEN_FAILED, &reason);
                let find = unhandled
                    .as_ref()
                    .map(|_| dialog.add_button(FIND_IN_SOFTWARE, ButtonStyle::Standard));
                dialog.add_button("OK", ButtonStyle::Primary);
                dialog.open();
                let answer = dialog.next_response().await;
                dialog.finish();
                let (Some(content_type), true) = (unhandled, answer.is_some() && answer == find) else {
                    return;
                };
                if let Err(error) = software_search::search_software(&content_type).await {
                    window.show_message(&error.to_string());
                }
            }
        ));
    }

    /// Opens an entry of a typed address or another app's request.
    fn activate_entry(&self, entry: &Entry) {
        match activation_for(entry) {
            Activation::Folder(uri) => self.navigate_or_report(&uri),
            Activation::File => self.open_file(entry),
            Activation::Archive => self.open_archive(entry),
            Activation::Refused(message) => self.show_message(message),
        }
    }

    /// Opens a file in its default application and records it among the
    /// recent files; a failure is shown in a dialog.
    fn open_file(&self, entry: &Entry) {
        let entry = entry.clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let opened = window.context().open_file(&entry, window.upcast_ref()).await;
                if let Err(reason) = opened {
                    window.report_open_failure(&reason, &entry);
                }
            }
        ));
    }

    /// Opens the file at `uri`, which the tab tried to list as a folder.
    pub(super) fn open_file_location(&self, uri: &str) {
        let uri = uri.to_owned();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                match query_entry(&uri).await {
                    Ok(entry) if activation_for(&entry) == Activation::File => window.open_file(&entry),
                    Ok(entry) if activation_for(&entry) == Activation::Archive => window.open_archive(&entry),
                    Ok(_) => {}
                    Err(error) => window.show_message(&error.to_string()),
                }
            }
        ));
    }

    /// Opens what was typed into the address bar and pressed Enter on.
    pub(super) fn submit_address(&self, text: &str) {
        let typed = text.trim();
        let current = self.current_uri();
        let place = self.place_titled(typed);
        let unchanged_page = place.is_some() && place == current;
        let is_page_uri = locations::is_home_alias(typed) || Page::from_uri(typed).is_some();
        if unchanged_page || is_page_uri {
            self.finish_address();
            self.navigate_or_report(typed);
            return;
        }
        let folder = match self.resolve_relative(text) {
            Ok(folder) => folder,
            Err(error) => {
                match place {
                    Some(place) => self.navigate_or_report(&place),
                    None => self.show_message(&error.to_string()),
                }
                return;
            }
        };
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let result = query_entry(&folder).await;
                window.open_typed_location(&folder, place.as_deref(), result);
            }
        ));
    }

    fn open_typed_location(&self, uri: &str, place: Option<&str>, result: Result<Entry, EntryError>) {
        let entry = match (result, place) {
            (Ok(entry), _) => entry,
            (Err(EntryError::NotFound(_)), Some(place)) => {
                self.finish_address();
                self.navigate_or_report(place);
                return;
            }
            // An unmounted share opens in the tab, which says why it is
            // unavailable and offers Try again.
            (Err(EntryError::NotMounted(_)), _) => {
                self.finish_address();
                self.navigate_or_report(uri);
                return;
            }
            (Err(error), _) => {
                self.show_message(&error.to_string());
                return;
            }
        };
        if !matches!(activation_for(&entry), Activation::Refused(_)) {
            self.finish_address();
        }
        self.activate_entry(&entry);
    }

    /// Opens command-line or desktop locations, as `openIncoming`: the
    /// first folder in the active tab, the others in new tabs, and files
    /// in their applications.
    pub(crate) fn open_locations(&self, uris: Vec<String>) {
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                for (index, uri) in uris.iter().enumerate() {
                    let tab = if index == 0 {
                        IncomingTab::Active
                    } else {
                        IncomingTab::New
                    };
                    let result = query_entry(uri).await;
                    window.open_incoming(uri, tab, result);
                }
            }
        ));
    }

    /// Opens one incoming location, whose metadata query gave `result`.
    pub(super) fn open_incoming(&self, uri: &str, tab: IncomingTab, result: Result<Entry, EntryError>) {
        let Ok(entry) = result else {
            // A missing or unreadable location opens as a tab that says so.
            self.open_incoming_folder(uri, tab);
            return;
        };
        match activation_for(&entry) {
            Activation::Folder(folder) => self.open_incoming_folder(&folder, tab),
            Activation::File => self.open_file(&entry),
            Activation::Archive => self.open_archive(&entry),
            Activation::Refused(message) => self.show_message(message),
        }
    }

    /// Opens the folder `uri` where `tab` says, showing an address the app
    /// cannot open in the message line.
    fn open_incoming_folder(&self, uri: &str, tab: IncomingTab) {
        let opened = match tab {
            IncomingTab::Active => self.navigate(uri),
            IncomingTab::New => self.add_tab(uri),
        };
        if let Err(error) = opened {
            self.show_message(&error.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{file_entry, folder_entry};

    /// parity: ARC-002
    #[test]
    fn folders_open_in_the_tab_files_in_an_application_and_zips_in_the_browser() {
        let folder = folder_entry("Projects");
        assert_eq!(activation_for(&folder), Activation::Folder(folder.uri.clone()));
        assert_eq!(activation_for(&file_entry("notes.txt")), Activation::File);
        assert_eq!(activation_for(&file_entry("photos.zip")), Activation::Archive);
        assert_eq!(activation_for(&file_entry("PHOTOS.ZIP")), Activation::Archive);
        assert_eq!(
            activation_for(&folder_entry("Archive.zip")),
            Activation::Folder(folder_entry("Archive.zip").uri)
        );
    }

    #[test]
    fn special_items_and_dangling_links_are_refused() {
        for kind in [EntryKind::Special, EntryKind::Symlink, EntryKind::Unknown] {
            let mut entry = file_entry("pipe");
            entry.kind = kind;
            assert_eq!(
                activation_for(&entry),
                Activation::Refused(NOT_OPENABLE),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn shares_and_shortcuts_open_their_target() {
        let mut share = folder_entry("media");
        share.kind = EntryKind::Mountable;
        share.target_uri = Some("smb://nas/media".into());
        assert_eq!(
            activation_for(&share),
            Activation::Folder("smb://nas/media".into())
        );
    }
}
