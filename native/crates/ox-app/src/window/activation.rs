// SPDX-License-Identifier: AGPL-3.0-only
//! Opening items and typed addresses: folders open in the tab, files in
//! their default application.
//!
//! Ports `openEntry`, `submitAddress` and `openIncoming` in
//! `desktop/ui/app.js` and `activation_kind` in `desktop/activation.py`.
//! An address or command-line argument is looked up first, so a file is
//! opened without moving the tab or adding a history entry, and a typed
//! page title ("Network") names a folder of that name when one exists.
//! A typed `http:` or `https:` address opens in the web browser.
//! Only an explicit request (Enter, double-click, Open, a typed address or
//! a command-line argument) ever launches an application.

use std::collections::HashMap;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::{self, Entry, EntryError, EntryKind};
use ox_core::integration;

use crate::locations::{self, Page};

use super::session::TabId;
use super::BrowserWindow;

/// Why an item cannot be opened (`activation_kind` in activation.py).
const NOT_OPENABLE: &str = "This item is not a regular file or a readable folder.";

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

/// A lookup started for one tab: a typed address or a location the tab
/// found to be a file. Its answer counts only while it is that tab's
/// newest lookup, the tab has not navigated and the user has not
/// switched away from it (the activation token of `openEntry`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PendingActivation {
    tab: TabId,
    generation: u64,
}

/// The newest lookup of each tab that has one running.
#[derive(Debug, Default)]
pub(super) struct Activations {
    started: u64,
    newest: HashMap<TabId, u64>,
    /// How many lookups have answered, current or not.
    #[cfg(test)]
    answered: u64,
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

/// Whether `typed` is an `http:` or `https:` address with a host, which
/// the address bar hands to the web browser.
fn is_web_address(typed: &str) -> bool {
    let Some((scheme, rest)) = typed.split_once("://") else {
        return false;
    };
    let is_web = scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https");
    let has_host = rest
        .chars()
        .next()
        .is_some_and(|first| !matches!(first, '/' | '?' | '#'));
    is_web && has_host && !typed.chars().any(char::is_control)
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

/// Where an address given to the address bar came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddressSource {
    /// Typed and applied with Enter, or chosen from the list below.
    Typed,
    /// Pasted: Paste and go, or the primary selection.
    Pasted,
}

/// An address being opened: where it resolved to, as given, the place
/// of that name if any, and where it came from.
struct AddressRequest<'a> {
    uri: &'a str,
    typed: &'a str,
    place: Option<&'a str>,
    source: AddressSource,
}

impl BrowserWindow {
    /// Drops the lookup tab `id` has running: it navigated.
    pub(super) fn supersede_activations(&self, id: TabId) {
        self.imp().activations.borrow_mut().newest.remove(&id);
    }

    /// Drops the lookups of every tab but `id`, which the user switched
    /// to.
    pub(super) fn keep_activations_of(&self, id: TabId) {
        let mut activations = self.imp().activations.borrow_mut();
        activations.newest.retain(|tab, _| *tab == id);
    }

    /// Starts a lookup for tab `id`, superseding its earlier one.
    fn begin_activation(&self, id: TabId) -> PendingActivation {
        let mut activations = self.imp().activations.borrow_mut();
        activations.started += 1;
        let generation = activations.started;
        activations.newest.insert(id, generation);
        PendingActivation { tab: id, generation }
    }

    /// Whether `pending` still speaks for its tab, and ends it.
    fn finish_activation(&self, pending: PendingActivation) -> bool {
        let imp = self.imp();
        let mut activations = imp.activations.borrow_mut();
        #[cfg(test)]
        {
            activations.answered += 1;
        }
        let is_current = activations.newest.get(&pending.tab) == Some(&pending.generation);
        if is_current {
            activations.newest.remove(&pending.tab);
        }
        is_current && imp.session.borrow().tab(pending.tab).is_some()
    }

    /// How many lookups have answered so far, including dropped ones.
    #[cfg(test)]
    pub(super) fn answered_activations(&self) -> u64 {
        self.imp().activations.borrow().answered
    }

    /// Opens the item at a display position (Enter, double-click, Open).
    pub(super) fn activate_item(&self, position: u32) {
        if let Some(item) = self.folder_pane().model().item(position) {
            self.activate_entry(item.entry());
        }
    }

    fn activate_entry(&self, entry: &Entry) {
        match activation_for(entry) {
            Activation::Folder(uri) => self.navigate_or_report(&uri),
            Activation::File => {
                let file = entry.clone();
                self.after_mounting(&entry.uri, move |window| window.open_file(&file));
            }
            Activation::Archive => {
                let archive = entry.clone();
                self.after_mounting(&entry.uri, move |window| window.open_archive(&archive));
            }
            Activation::Refused(message) => self.show_message(message),
        }
    }

    /// Opens a file in its default application and records it among the
    /// recent files.
    fn open_file(&self, entry: &Entry) {
        let on_error = glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |error: glib::Error| window.show_message(&error.to_string())
        );
        self.context().open_file(entry, self.upcast_ref(), on_error);
    }

    /// Opens the file at `uri`, which tab `id` tried to list as a folder.
    pub(super) fn open_file_location(&self, id: TabId, uri: &str) {
        let uri = uri.to_owned();
        let pending = self.begin_activation(id);
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let result = query_entry(&uri).await;
                if !window.finish_activation(pending) {
                    return;
                }
                match result {
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
        self.open_address(text, AddressSource::Typed);
    }

    /// Goes to a pasted address: Paste and go, or the text selected
    /// elsewhere and middle-clicked onto the crumbs (NAV-032). A file
    /// named there is shown in its folder rather than launched, and the
    /// address is not added to the typed history.
    pub(super) fn go_to_pasted_address(&self, text: &str) {
        self.open_address(text, AddressSource::Pasted);
    }

    /// Opens the address `text`, given as `source` says.
    fn open_address(&self, text: &str, source: AddressSource) {
        let typed = text.trim();
        if is_web_address(typed) {
            self.accept_address(typed, source);
            self.open_web_address(typed);
            return;
        }
        let current = self.current_uri();
        let place = self.place_titled(typed);
        let unchanged_page = place.is_some() && place == current;
        let is_page_uri = locations::is_home_alias(typed) || Page::from_uri(typed).is_some();
        if unchanged_page || is_page_uri {
            self.accept_address(typed, source);
            self.navigate_or_report(typed);
            return;
        }
        let folder = match self.resolve_relative(text) {
            Ok(folder) => folder,
            Err(error) => {
                match place {
                    Some(place) => {
                        self.accept_address(typed, source);
                        self.navigate_or_report(&place);
                    }
                    None => self.show_message(&error.to_string()),
                }
                return;
            }
        };
        let typed = typed.to_owned();
        let Some(tab) = self.imp().session.borrow().active_id() else {
            return;
        };
        let pending = self.begin_activation(tab);
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let result = query_entry(&folder).await;
                if window.finish_activation(pending) {
                    let request = AddressRequest {
                        uri: &folder,
                        typed: &typed,
                        place: place.as_deref(),
                        source,
                    };
                    window.open_typed_location(&request, result);
                }
            }
        ));
    }

    /// The address `typed` is going to be opened: the crumbs show again,
    /// and a typed one goes to the top of the typed history (NAV-043).
    fn accept_address(&self, typed: &str, source: AddressSource) {
        self.finish_address();
        if source == AddressSource::Typed {
            self.address_bar().remember_typed(typed);
        }
    }

    /// Hands a typed web address to the web browser and says so, as
    /// Explorer and Dolphin do, where app.js refused it (NAV-036).
    fn open_web_address(&self, address: &str) {
        let on_error = glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |error: glib::Error| window.show_message(&error.to_string())
        );
        self.context().open_uri(address, self.upcast_ref(), on_error);
        self.show_message(&format!("Opening {address} in your web browser."));
    }

    /// Opens the location an address resolved to, whose metadata query
    /// gave `result`.
    fn open_typed_location(&self, request: &AddressRequest<'_>, result: Result<Entry, EntryError>) {
        let entry = match (result, request.place) {
            (Ok(entry), _) => entry,
            (Err(EntryError::NotFound(_)), Some(place)) => {
                self.accept_address(request.typed, request.source);
                self.navigate_or_report(place);
                return;
            }
            // An unmounted share opens in the tab, which says why it is
            // unavailable and offers Try again.
            (Err(EntryError::NotMounted(_)), _) => {
                self.accept_address(request.typed, request.source);
                self.navigate_or_report(request.uri);
                return;
            }
            (Err(error), _) => {
                self.show_message(&error.to_string());
                return;
            }
        };
        match activation_for(&entry) {
            Activation::Refused(message) => self.show_message(message),
            Activation::File if request.source == AddressSource::Pasted => {
                self.accept_address(request.typed, request.source);
                self.show_in_its_folder(&entry.uri);
            }
            _ => {
                self.accept_address(request.typed, request.source);
                self.activate_entry(&entry);
            }
        }
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

    /// A folder's name never makes it a file, and a stale folder flag, such
    /// as a search row's, never makes a file a folder (`activation_kind`).
    ///
    /// parity: NAV-040
    #[test]
    fn the_kind_decides_what_opens_not_the_name_or_a_stale_flag() {
        let video_folder = folder_entry("clip.mp4");
        let mut stale = file_entry("report.txt");
        stale.is_dir = true;

        assert_eq!(
            activation_for(&video_folder),
            Activation::Folder(video_folder.uri.clone())
        );
        assert_eq!(activation_for(&stale), Activation::File);
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
