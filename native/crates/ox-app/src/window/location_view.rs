// SPDX-License-Identifier: AGPL-3.0-only
//! What the frame shows for the active tab's location: the window title,
//! the history buttons, the address bar, the tabs, the search box and the
//! sidebar highlight.
//!
//! Ports `renderNavigation` and `renderTabs` in `desktop/ui/app.js`, and
//! `editAddress` and `finishAddress`. Titles, addresses and crumbs come
//! from the window's [`LocationContext`], so a phone is called by its mount
//! name everywhere.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::{self, is_device_location, is_smb_location, parent_location, LocationContext};
use ox_core::places::NetworkLocation;

use crate::icons::{Art, Icon};
use crate::locations::Page;

use super::address_bar::CrumbButton;
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
/// `renderNavigation`): the page's glyph, the network glyph for SMB, a
/// phone for devices, else the colour folder.
fn address_icon(uri: &str) -> Icon {
    if let Some(page) = Page::from_uri(uri) {
        return page.icon();
    }
    if is_smb_location(uri) {
        Icon::Organization
    } else if is_device_location(uri) {
        Icon::Phone
    } else {
        Icon::FileFolder
    }
}

/// A tab's icon, as `renderTabs` picks it: the network glyph on the
/// Network page, the gear on Settings, a phone for devices, the colour
/// folder everywhere else, This PC included, and for SMB the location on
/// the network bar as its row of `network` shows it in the sidebar.
fn tab_icon(uri: &str, network: &[NetworkLocation]) -> Art {
    match Page::from_uri(uri) {
        Some(page @ (Page::Network | Page::Settings)) => return Art::Glyph(page.icon()),
        Some(Page::ThisPc) | None => {}
    }
    if is_device_location(uri) {
        Art::Glyph(Icon::Phone)
    } else if is_smb_location(uri) {
        Art::for_smb_location(uri, network)
    } else {
        Art::Folder
    }
}

/// How the strip shows `tab`: its title, its address (with "Network
/// location" for SMB) and its icon, which for SMB comes from `network`.
fn tab_view(
    tab: &Tab,
    session: &Session,
    locations: &LocationContext,
    network: &[NetworkLocation],
) -> TabView {
    let uri = tab.uri();
    let mut tooltip = locations.display_location(uri);
    if is_smb_location(uri) {
        tooltip.push_str(" · Network location");
    }
    TabView {
        id: tab.id,
        uri: uri.to_owned(),
        title: locations.title_for(uri),
        tooltip,
        icon: tab_icon(uri, network),
        active: session.is_active(tab.id),
        previous_version: None,
    }
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
        self.set_title(Some(&format!("{title} — OpenXplorer")));
        self.set_action_enabled(WindowAction::Back, location.can_go_back);
        self.set_action_enabled(WindowAction::Forward, location.can_go_forward);
        self.set_action_enabled(WindowAction::Up, parent_location(uri).is_some());
        self.set_action_enabled(WindowAction::PinFolder, !on_page);
        self.render_address(uri);
        let search = self.search_box();
        search.set_folder_title(&title);
        search.set_enabled(!on_page && !is_device_location(uri));
        self.update_cache_folder_action();
        self.render_tabs();
        self.show_snapshot_banner();
        self.update_properties_actions();
        self.sidebar().select(uri);
        self.render_landing();
        self.show_surface_for(uri);
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
        let locations = self.imp().locations.borrow();
        let breadcrumbs = locations.breadcrumbs(uri);
        let crumbs: Vec<CrumbButton> = breadcrumbs
            .iter()
            .enumerate()
            .map(|(index, crumb)| CrumbButton {
                address: locations.display_location(&crumb.uri),
                divider_before: location::crumb_divider(uri, index),
                crumb: crumb.clone(),
            })
            .collect();
        let address = locations.display_location(uri);
        self.address_bar()
            .show_location(&crumbs, &address, address_icon(uri));
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
        self.mark_tabs_with_dialogs_and_snapshots(&mut views);
        self.tab_strip().set_tabs(&views);
    }

    /// Replaces the breadcrumbs with the editable address (Ctrl+L).
    pub(super) fn edit_address(&self) {
        let Some(uri) = self.current_uri() else { return };
        let address = self.imp().locations.borrow().display_location(&uri);
        self.address_bar().edit(&address);
    }

    /// Ends editing with Enter or Escape: back to the breadcrumbs, with
    /// keyboard focus in the folder view.
    pub(super) fn finish_address(&self) {
        if let Some(uri) = self.current_uri() {
            let address = self.imp().locations.borrow().display_location(&uri);
            self.address_bar().show_crumbs(&address);
        }
        self.folder_pane().focus_view();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{studio_nas_mapped_drive, studio_nas_server};

    /// A location and the tab and address-bar icons it shows.
    struct IconCase {
        uri: &'static str,
        tab: Art,
        address: Icon,
    }

    /// parity: TAB-010, DEV-004
    #[test]
    fn tabs_and_the_address_bar_show_the_current_apps_icons() {
        let cases = [
            IconCase {
                uri: "file:///tmp/work",
                tab: Art::Folder,
                address: Icon::FileFolder,
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
        for case in cases {
            assert_eq!(tab_icon(case.uri, &[]), case.tab, "{}", case.uri);
            assert_eq!(address_icon(case.uri), case.address, "{}", case.uri);
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
        let tab_on = |uri: &str| tab_icon(uri, &network);
        assert_eq!(tab_on(&server.uri), Art::for_network_row(&server));
        assert_eq!(tab_on(&mapped_drive.uri), Art::for_network_row(&mapped_drive));
        assert_eq!(tab_on("smb://studio-nas/projects/2024"), Art::SHARE);
    }
}
