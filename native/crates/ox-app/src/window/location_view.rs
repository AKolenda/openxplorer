// SPDX-License-Identifier: AGPL-3.0-only
//! What the frame shows for the active tab's location: the window title,
//! the history buttons, the address bar, the tabs, the search box and the
//! sidebar highlight.
//!
//! Ports `renderNavigation` and `renderTabs` in `v2.0.0:desktop/ui/app.js`, and
//! `editAddress` and `finishAddress`. Titles, addresses and crumbs come
//! from the window's [`LocationContext`], so a phone is called by its mount
//! name everywhere.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::{
    self, is_archive_location, is_device_location, is_server_location, parent_location, LocationContext,
};
use ox_core::ops::JournalDirection;
use ox_core::places::{NetworkLocation, Place};

use crate::folder_view::shown_name::shown_name;
use crate::icons::{Art, Icon};
use crate::locations::Page;

use super::address_bar::CrumbButton;
use super::command_bar::{FolderPin, MoreMenuState, MorePlace, NetworkDrive};
use super::session::{Session, Tab};
use super::tab_strip::TabView;
use super::window_action::WindowAction;
use super::BrowserWindow;

/// Where the active tab is and where its history can go from there.
#[derive(Debug)]
struct ActiveLocation {
    uri: String,
    can_go_back: bool,
    can_go_forward: bool,
}

/// The address-bar icon for a location (`address-icon` in
/// `renderNavigation`): the page's glyph, the house for the home folder
/// at `home_uri` (the Home of app.js), the network glyph for SMB and the
/// other network protocols, as their tabs show, a phone for devices, else
/// the colour folder.
pub(super) fn address_icon(uri: &str, home_uri: &str) -> Icon {
    if let Some(page) = Page::from_uri(uri) {
        return page.icon();
    }
    if location::same_location(uri, home_uri) {
        Icon::Home
    } else if is_archive_location(uri) {
        Icon::FolderZip
    } else if is_server_location(uri) {
        Icon::Organization
    } else if is_device_location(uri) {
        Icon::Phone
    } else {
        Icon::FileFolder
    }
}

/// The title of a history step at `uri`, as its tab, the window and the
/// Back and Forward menus name it: the location's title, or for a search
/// there "Search Results in Documents", as Windows 11 File Explorer names
/// it.
pub(super) fn step_title(locations: &LocationContext, uri: &str, is_search: bool) -> String {
    let place = locations.title_for(uri);
    if is_search {
        ox_core::i18n::format_message("Search Results in {place}", &[("place", &place)])
    } else {
        place
    }
}

/// The address-bar icon of a history step at `uri`: the magnifier for a
/// search, as Windows 11 File Explorer shows it, else [`address_icon`].
pub(super) fn step_icon(uri: &str, home_uri: &str, is_search: bool) -> Icon {
    if is_search {
        Icon::Search
    } else {
        address_icon(uri, home_uri)
    }
}

/// The crumbs the address bar shows for `uri`, divided as
/// `renderNavigation` divides them. Inside the home folder they start at
/// it, as "Home › Documents" in Dolphin and Explorer, unless `full_path`
/// asks for the path from `/` (NAV-024).
pub(super) fn crumb_buttons(locations: &LocationContext, uri: &str, full_path: bool) -> Vec<CrumbButton> {
    let mut breadcrumbs = locations.breadcrumbs(uri);
    let home = locations.home_uri();
    let home_at = breadcrumbs
        .iter()
        .position(|crumb| location::same_location(&crumb.uri, &home))
        .filter(|&index| !full_path && index > 0);
    let skipped = home_at.unwrap_or(0);
    if let Some(index) = home_at {
        breadcrumbs.drain(..index);
        breadcrumbs[0].label = locations.title_for(&home);
    }
    breadcrumbs
        .into_iter()
        .enumerate()
        .map(|(index, crumb)| CrumbButton {
            address: locations.display_location(&crumb.uri),
            divider_before: (index > 0)
                .then(|| location::crumb_divider(uri, index + skipped))
                .flatten(),
            crumb,
            refreshes: false,
        })
        .collect()
}

/// The crumb a search of `uri` adds after the folder's, as Windows 11
/// File Explorer shows "Documents › Search Results in Documents";
/// clicking it runs the search again, as Refresh does.
fn search_crumb(locations: &LocationContext, uri: &str) -> CrumbButton {
    CrumbButton {
        crumb: ox_core::location::Crumb::new(step_title(locations, uri, true), uri),
        address: locations.display_location(uri),
        // Past the first crumbs, which a local path draws without one.
        divider_before: location::crumb_divider(uri, 2),
        refreshes: true,
    }
}

/// A tab's icon, as `renderTabs` picks it: the network glyph on
/// Network, the gear on Settings, a phone for devices, the colour
/// folder everywhere else, This PC included, and for a network location
/// (SMB, or a folder under a kernel CIFS/SMB3 mount, NET-006) the location
/// on the network bar as its row of `network` shows it in the sidebar.
fn tab_icon(uri: &str, locations: &LocationContext, network: &[NetworkLocation]) -> Art {
    match Page::from_uri(uri) {
        Some(page @ (Page::Network | Page::Settings)) => return Art::Glyph(page.icon()),
        Some(Page::ThisPc) | None => {}
    }
    if is_device_location(uri) {
        Art::Glyph(Icon::Phone)
    } else if is_archive_location(uri) {
        Art::ZipFolder
    } else if locations.is_network_location(uri) {
        Art::for_smb_location(uri, network)
    } else {
        Art::Folder
    }
}

/// How the strip shows `tab`: its title, its address (with "Network
/// location" for a network location) and its icon, which for a network
/// location comes from `network`.
fn tab_view(
    tab: &Tab,
    session: &Session,
    locations: &LocationContext,
    network: &[NetworkLocation],
) -> TabView {
    let uri = tab.uri();
    let is_search = tab.history.current_search().is_some();
    let mut tooltip = locations.display_location(uri);
    if locations.is_network_location(uri) {
        tooltip.push_str(" · Network location");
    }
    TabView {
        id: tab.id,
        uri: uri.to_owned(),
        title: step_title(locations, uri, is_search),
        tooltip,
        icon: if is_search {
            Art::Glyph(Icon::Search)
        } else {
            tab_icon(uri, locations, network)
        },
        active: session.is_active(tab.id),
        previous_version: None,
    }
}

/// What the "…" menu keeps of the places the sidebar was last drawn from.
#[derive(Debug, Default)]
pub(super) struct MoreMenuPlaces {
    /// Quick access, for Pin or Unpin.
    quick_access: Vec<Place>,
    /// The connected network shares, for Disconnect network drive.
    network_drives: Vec<NetworkDrive>,
}

impl BrowserWindow {
    /// Updates the frame after the active tab moved: the address bar
    /// returns to breadcrumbs.
    pub(super) fn render_navigation(&self) {
        self.render_location();
        if let Some(uri) = self.current_uri() {
            let address = self.imp().locations.borrow().display_location(&uri);
            self.address_bar().show_crumbs(&address);
        }
    }

    /// Updates the window title, history buttons, breadcrumbs, tabs,
    /// sidebar highlight and landing page for the active tab's location,
    /// and shows the Settings page on the Settings tab.
    pub(super) fn render_location(&self) {
        let Some(location) = self.active_location() else {
            return;
        };
        let uri = location.uri.as_str();
        let on_page = Page::from_uri(uri).is_some();
        let title = self.imp().locations.borrow().title_for(uri);
        self.render_title();
        self.set_action_enabled(WindowAction::Back, location.can_go_back);
        self.set_action_enabled(WindowAction::Forward, location.can_go_forward);
        self.set_action_enabled(WindowAction::Up, parent_location(uri).is_some());
        self.show_navigation_tooltips();
        let in_zip = is_archive_location(uri);
        // A ZIP opened like a folder is neither pinned nor searched: its
        // location is the app's own, and search reads real folders.
        self.set_action_enabled(WindowAction::PinFolder, !on_page && !in_zip);
        self.render_address(uri);
        let search = self.search_box();
        search.set_folder_title(&title);
        search.set_enabled(!on_page && !in_zip && !is_device_location(uri));
        self.show_extract_button();
        self.update_more_menu();
        self.update_cache_folder_action();
        self.render_tabs();
        self.show_pane_captions();
        self.show_snapshot_banner();
        self.show_network_info_bar();
        self.update_properties_actions();
        self.sidebar().select(uri);
        self.follow_with_folder_tree();
        self.render_landing();
        self.show_surface_for(uri);
        self.picker_selection_changed();
    }

    /// Keeps what "…" needs of `places`, which the sidebar has just been
    /// drawn from: Quick access, for Pin or Unpin, and, for Disconnect
    /// network drive, the network mounts themselves, one per mount, so
    /// a saved share inside a mount is not listed as a drive of its own.
    pub(super) fn remember_more_menu_places(&self, places: &crate::places::Places) {
        let network_drives = crate::places::network_drive_mounts(&self.imp().volumes.borrow())
            .into_iter()
            .map(|mount| NetworkDrive {
                label: mount.label,
                uri: mount.uri,
            })
            .collect();
        self.imp().more_menu_places.replace(Some(MoreMenuPlaces {
            quick_access: places.quick_access.clone(),
            network_drives,
        }));
    }

    /// The connected network shares, one per mount, for the navigation
    /// pane's Disconnect network drive, as "…" lists them.
    pub(super) fn network_drives(&self) -> Vec<NetworkDrive> {
        if self.imp().more_menu_places.borrow().is_none() {
            let places = self.places();
            self.remember_more_menu_places(&places);
        }
        self.imp()
            .more_menu_places
            .borrow()
            .as_ref()
            .map(|sources| sources.network_drives.clone())
            .unwrap_or_default()
    }

    /// Shows in "…" what the location shown allows, as Windows 11's "…"
    /// does: in This PC and Network the network drive commands; in a
    /// folder Undo and Redo while each can run, named for the step, and
    /// Unpin from Quick access while the folder is pinned, else Pin to
    /// Quick access. It follows the location, every change to Quick access
    /// or the network shares, and the undo journal.
    pub(super) fn update_more_menu(&self) {
        let Some(uri) = self.current_uri() else {
            return;
        };
        if self.imp().more_menu_places.borrow().is_none() {
            let places = self.places();
            self.remember_more_menu_places(&places);
        }
        let sources = self.imp().more_menu_places.borrow();
        let Some(sources) = sources.as_ref() else {
            return;
        };
        let page = Page::from_uri(&uri);
        let place = match page {
            Some(Page::ThisPc) => MorePlace::ThisPc,
            Some(Page::Network) => MorePlace::Network,
            _ => MorePlace::Folder,
        };
        // Only This PC asks GIO, which lists every application.
        let system_information =
            place == MorePlace::ThisPc && crate::integration::system_information().is_some();
        let pinned = sources
            .quick_access
            .iter()
            .find(|pin| page.is_none() && location::same_location(&pin.uri, &uri));
        let pin = pinned.map_or(FolderPin::Unpinned, |pin| FolderPin::Pinned(pin.uri.clone()));
        let context = self.context();
        let state = MoreMenuState {
            place,
            pin,
            can_undo: context.journal_label(JournalDirection::Undo).is_some(),
            can_redo: context.journal_label(JournalDirection::Redo).is_some(),
            network_drives: sources.network_drives.clone(),
            system_information,
        };
        self.command_bar().show_more_menu(state);
    }

    /// Titles the window after the active tab's location: its name, or
    /// its full path when the settings ask for it (Dolphin's "Show full
    /// path in title bar", SET-011); a page keeps its title.
    pub(super) fn render_title(&self) {
        // A file dialog keeps the caller's title (INT-032).
        if let Some(title) = self.picker_title() {
            self.set_title(Some(&title));
            return;
        }
        let Some(uri) = self.current_uri() else {
            return;
        };
        let full_path = self.context().settings_data().preferences.full_path_in_title;
        let is_search = self.shows_search_step();
        let locations = self.imp().locations.borrow();
        let place = if is_search {
            step_title(&locations, &uri, true)
        } else if full_path && Page::from_uri(&uri).is_none() {
            locations.display_location(&uri)
        } else {
            locations.title_for(&uri)
        };
        self.set_title(Some(&ox_core::i18n::format_message(
            "{place} — OpenXplorer",
            &[("place", &place)],
        )));
    }

    fn active_location(&self) -> Option<ActiveLocation> {
        let session = self.imp().session.borrow();
        let tab = session.active()?;
        Some(ActiveLocation {
            uri: tab.uri().to_owned(),
            can_go_back: tab.history.can_go_back(),
            can_go_forward: tab.history.can_go_forward(),
        })
    }

    /// Shows `uri` in the address bar: its icon, and its crumbs divided as
    /// `renderNavigation` divides them.
    fn render_address(&self, uri: &str) {
        self.render_address_of(uri);
    }

    /// Draws the crumbs and the address of `uri`. The crumb of an opened
    /// archive is a file's name, so it hides a known extension while
    /// names do, as Windows' does for a ZIP folder; folders keep theirs.
    /// Search results add their own crumb and icon.
    pub(super) fn render_address_of(&self, uri: &str) {
        let full_path = self.shows_full_path();
        let extensions = self.file_name_extensions();
        let is_search = self.current_uri().as_deref() == Some(uri) && self.shows_search_step();
        let locations = self.imp().locations.borrow();
        let mut crumbs = crumb_buttons(&locations, uri, full_path);
        if is_search {
            crumbs.push(search_crumb(&locations, uri));
        }
        if let Some(archive) = crumbs
            .iter_mut()
            .find(|crumb| is_archive_location(&crumb.crumb.uri))
        {
            let shown = shown_name(&archive.crumb.label, false, extensions).to_owned();
            archive.crumb.label = shown;
        }
        let address = locations.display_location(uri);
        let icon = step_icon(uri, &locations.home_uri(), is_search);
        self.address_bar().show_location(&crumbs, &address, icon);
    }

    /// Redraws the tab strip.
    pub(super) fn render_tabs(&self) {
        let network = self.network_locations();
        let mut views: Vec<TabView> = {
            let session = self.imp().session.borrow();
            let locations = self.imp().locations.borrow();
            let tab_views = session
                .tabs()
                .iter()
                .map(|tab| tab_view(tab, &session, &locations, &network));
            tab_views.collect()
        };
        self.mark_snapshot_tabs(&mut views);
        self.tab_strip().set_tabs(&views);
    }

    /// Replaces the breadcrumbs with the editable address (Ctrl+L). The
    /// Settings tab has no address to edit (`editAddress` in app.js). A
    /// second Ctrl+L, before anything was typed, returns to the crumbs.
    pub(super) fn edit_address(&self) {
        let Some(uri) = self.current_uri() else { return };
        if Page::from_uri(&uri) == Some(Page::Settings) {
            return;
        }
        if self.address_bar().edits_whole_address() {
            self.finish_address();
            return;
        }
        self.forget_address_completions();
        let address = self.imp().locations.borrow().display_location(&uri);
        self.address_bar().edit(&address);
    }

    /// Ends editing with Enter or Escape: back to the breadcrumbs, with
    /// keyboard focus in the folder view.
    pub(super) fn finish_address(&self) {
        self.forget_address_completions();
        // The keyboard leaves the entry before the entry is hidden: hiding a
        // focused widget makes GTK hand the keyboard to whatever control comes
        // next in the focus chain, such as a crumb or the Details pane's close
        // button, and typing then goes nowhere near the folder.
        self.folder_pane().focus_view();
        if let Some(uri) = self.current_uri() {
            let address = self.imp().locations.borrow().display_location(&uri);
            self.address_bar().show_crumbs(&address);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{studio_nas_mapped_drive, studio_nas_server};

    /// parity: NAV-024
    #[test]
    fn crumbs_start_at_home_unless_the_full_path_is_asked_for() {
        let locations = crate::locations::location_context("/home/demo".into(), &[]);
        let shown = |uri: &str, full_path: bool| -> Vec<(String, Option<&str>)> {
            let crumbs = crumb_buttons(&locations, uri, full_path);
            crumbs
                .into_iter()
                .map(|button| (button.crumb.label, button.divider_before))
                .collect()
        };
        let crumb = |label: &str, divider: Option<&'static str>| (label.to_owned(), divider);

        let relative = shown("file:///home/demo/Documents/Letters", false);
        let full = shown("file:///home/demo/Documents/Letters", true);

        let home = crumb("Home", None);
        assert_eq!(
            relative,
            [home, crumb("Documents", Some("/")), crumb("Letters", Some("/"))]
        );
        assert_eq!(full.len(), 5);
        assert_eq!(full[..2], [crumb("/", None), crumb("home", None)]);
        let outside = shown("file:///srv/media", false);
        assert_eq!(
            outside,
            [crumb("/", None), crumb("srv", None), crumb("media", Some("/"))]
        );
    }

    /// A location and the tab and address-bar icons it shows.
    struct IconCase {
        uri: &'static str,
        tab: Art,
        address: Icon,
    }

    /// parity: TAB-010, DEV-004, NAV-025, LOOK-015, LOOK-016
    #[test]
    fn tabs_and_the_address_bar_show_the_current_apps_icons() {
        let cases = [
            IconCase {
                uri: "file:///tmp/work",
                tab: Art::Folder,
                address: Icon::FileFolder,
            },
            IconCase {
                uri: "file:///home/demo/",
                tab: Art::Folder,
                address: Icon::Home,
            },
            IconCase {
                uri: "smb://nas/media",
                tab: Art::SHARE,
                address: Icon::Organization,
            },
            IconCase {
                uri: "mtp://%5Busb%3A001%2C010%5D/",
                tab: Art::Glyph(Icon::Phone),
                address: Icon::Phone,
            },
            IconCase {
                uri: Page::ThisPc.uri(),
                tab: Art::Folder,
                address: Icon::Laptop,
            },
            IconCase {
                uri: Page::Network.uri(),
                tab: Art::Glyph(Icon::Organization),
                address: Icon::Organization,
            },
            // The gear of `icon('settings')` in renderTabs.
            IconCase {
                uri: Page::Settings.uri(),
                tab: Art::Glyph(Icon::Settings),
                address: Icon::Settings,
            },
        ];
        let locations = LocationContext::default();
        for case in cases {
            assert_eq!(tab_icon(case.uri, &locations, &[]), case.tab, "{}", case.uri);
            let address = address_icon(case.uri, "file:///home/demo");
            assert_eq!(address, case.address, "{}", case.uri);
        }
    }

    /// A location and the title and tooltip of its tab.
    struct TabTextCase {
        uri: &'static str,
        title: &'static str,
        tooltip: &'static str,
    }

    /// Tab titles and tooltips as `renderTabs` writes them: the page
    /// names, the folder's decoded name, "Local Disk" for `/`, and "·
    /// Network location" after an SMB address.
    ///
    /// parity: TAB-010
    #[gtk::test]
    fn tabs_are_titled_and_described_as_render_tabs_does() {
        let locations = crate::locations::location_context("/home/demo".into(), &[]);
        let cases = [
            TabTextCase {
                uri: "file:///home/demo",
                title: "Home",
                tooltip: "/home/demo",
            },
            TabTextCase {
                uri: "file:///",
                title: "Local Disk",
                tooltip: "/",
            },
            TabTextCase {
                uri: "file:///srv/Brand%20assets",
                title: "Brand assets",
                tooltip: "/srv/Brand assets",
            },
            TabTextCase {
                uri: "smb://nas/media",
                title: "media",
                tooltip: "\\\\nas\\media · Network location",
            },
            TabTextCase {
                uri: Page::Settings.uri(),
                title: "Settings",
                tooltip: "Settings",
            },
        ];
        for case in cases {
            let mut session = Session::default();
            let id = session.add(case.uri, super::super::session::TabPlacement::Foreground);
            let tab = session.tab(id).expect("the tab was added");
            let view = tab_view(tab, &session, &locations, &[]);
            assert_eq!(view.title, case.title, "{}", case.uri);
            assert_eq!(view.tooltip, case.tooltip, "{}", case.uri);
            assert!(view.active);
        }
    }

    /// The owner's icon mapping (2026-09-28) shows a network location the
    /// same way everywhere, so a tab on a server or a mapped drive shows
    /// what its sidebar row shows.
    ///
    /// parity: LOOK-016
    #[test]
    fn a_tab_on_a_server_or_a_mapped_drive_shows_its_sidebar_art() {
        let server = studio_nas_server();
        let mapped_drive = studio_nas_mapped_drive();
        let network = [server.clone(), mapped_drive.clone()];
        let locations = LocationContext::default();
        let tab_on = |uri: &str| tab_icon(uri, &locations, &network);
        assert_eq!(tab_on(&server.uri), Art::for_network_row(&server));
        assert_eq!(tab_on(&mapped_drive.uri), Art::for_network_row(&mapped_drive));
        assert_eq!(tab_on("smb://studio-nas/projects/2024"), Art::SHARE);
    }

    /// A tab at or below a kernel CIFS/SMB3 mount point stands on the
    /// network bar like an SMB tab; a folder whose name only starts like
    /// the mount point does not.
    ///
    /// Ported from `v2.0.0:desktop/tests/ui_v06.py::Mounted CIFS paths are recognized`
    ///
    /// parity: NET-006
    #[test]
    fn a_tab_under_a_cifs_mount_is_a_network_location() {
        let locations = LocationContext {
            network_mounts: vec!["/mnt/nas".into()],
            ..LocationContext::default()
        };
        let tab_on = |uri: &str| tab_icon(uri, &locations, &[]);
        assert_eq!(tab_on("file:///mnt/nas"), Art::SHARE);
        assert_eq!(tab_on("file:///mnt/nas/Projects"), Art::SHARE);
        assert_eq!(tab_on("file:///mnt/nas-other"), Art::Folder);
        assert_eq!(tab_on("file:///home/demo"), Art::Folder);
    }
}
